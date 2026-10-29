//! Вкладка Wallpaper: карусель обоев из папки (`wallpaper_dir` в конфиге, по умолчанию
//! «Изображения»), миниатюры декодируются в фоне (`thumbs`), смена обоев и список недавних.
//! Миниатюры живут в кэше битмапов видеопамяти; обои меняются через SystemParametersInfo.
//! Раздел «WE» — обои Wallpaper Engine (см. `we`): в карусели их превью, включает их сам WE.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED;

use crate::anim::Spring;
use crate::config::Config;
use crate::ui::{icon, Canvas, Rect, Ui, bare, card, ink};
use crate::{sys, thumbs, we};

use super::{id, read_lines, write_lines, PAD, TOP, WALLPAPER};

const FILE: &str = "wallpapers.txt";
const MAX_FILES: usize = 100;
const MAX_RECENT: usize = 12;

/// Разделы вкладки (`Wall::filter`).
const ALL: usize = 0;
const RECENT: usize = 1;
const WE: usize = 2;

/// Ширина центральной карточки и наклон её граней (логические пиксели).
const CENTER_W: f32 = 240.0;
const SLANT: f32 = 22.0;

/// Размер миниатюры для масштаба отрисовки `scale` (физ. пикселей на логический): центральная
/// карточка должна получать картинку не меньше своего размера на экране, иначе она размыта.
/// Ширина кратна 64 (меньше разных размеров в кэше на диске), пропорции 16:9.
fn thumb_size(scale: f32) -> (u32, u32) {
    let need = ((CENTER_W + SLANT) * scale.max(1.0)).ceil() as u32;
    let w = need.div_ceil(64).clamp(6, 20) * 64;
    (w, w * 9 / 16)
}

/// Папка, выбранная в системном диалоге (диалог работает в своём потоке), ждёт `sync`.
static PICKED: Mutex<Option<String>> = Mutex::new(None);
/// Диалог выбора папки уже открыт: второй не запускаем.
static PICKING: AtomicBool = AtomicBool::new(false);

pub struct Wall {
    /// Значение `wallpaper_dir`, которое видели в конфиге в последний раз: чтобы устаревшая копия
    /// конфига не отменила выбор, сделанный в диалоге.
    cfg_dir: Option<String>,
    /// Папка, из которой берутся обои (`None` — «Изображения»).
    dir: Option<String>,
    /// Картинки карусели: сами обои или (в разделе WE) превью проектов из `we_items`.
    files: Vec<PathBuf>,
    scanned: bool,
    /// `files` собраны из Wallpaper Engine, а не из папки.
    loaded_we: bool,
    /// Установка Wallpaper Engine (ищется один раз, при первом открытии вкладки).
    we: Option<we::Install>,
    we_checked: bool,
    /// Проекты WE, по индексу совпадают с `files`, пока `loaded_we`.
    we_items: Vec<we::Item>,
    /// Проекты, включённые сейчас в WE (нормализованные пути их папок, см. `we::project_dir`).
    we_current: Vec<String>,
    gen: u32,
    /// Пиксели миниатюр, ещё не загруженные в видеопамять.
    pending: HashMap<usize, Vec<u8>>,
    /// Миниатюры, которые уже готовы: вытесненный из кэша видеопамяти битмап перечитывается
    /// из кэша на диске (копии всех пикселей в памяти не держим).
    decoded: HashSet<usize>,
    /// Размер миниатюр текущего поколения.
    thumb: (u32, u32),
    /// Положение карусели в карточках.
    offset: Spring,
    /// Путь текущих обоев (в нижнем регистре, с `/`).
    current: String,
    recent: Vec<String>,
    /// Раздел: `ALL`, `RECENT` или `WE`.
    filter: usize,
    dir_label: String,
}

impl Default for Wall {
    fn default() -> Self {
        Wall {
            cfg_dir: None,
            dir: None,
            files: Vec::new(),
            scanned: false,
            loaded_we: false,
            we: None,
            we_checked: false,
            we_items: Vec::new(),
            we_current: Vec::new(),
            gen: 0,
            pending: HashMap::new(),
            decoded: HashSet::new(),
            thumb: thumb_size(1.0),
            offset: Spring::unit(0.0, 170.0, 24.0),
            current: String::new(),
            recent: Vec::new(),
            filter: 0,
            dir_label: String::new(),
        }
    }
}

fn norm(p: &str) -> String {
    p.replace('\\', "/").to_lowercase()
}

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["jpg", "jpeg", "png", "bmp"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Собирает обои из `dir` и из его прямых подпапок (глубже не заходит). Сначала файлы самой
/// папки, затем подпапки по порядку имён; не-изображения пропускаются.
fn scan(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    for p in rd.flatten().map(|e| e.path()) {
        if p.is_dir() {
            subdirs.push(p);
        } else if is_image(&p) && p.is_file() {
            files.push(p);
        }
    }
    files.sort();
    subdirs.sort();
    for p in files {
        if out.len() >= MAX_FILES {
            return;
        }
        out.push(p);
    }
    for sub in subdirs {
        let Ok(rd) = std::fs::read_dir(&sub) else { continue };
        let mut inner: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| is_image(p) && p.is_file()).collect();
        inner.sort();
        for p in inner {
            if out.len() >= MAX_FILES {
                return;
            }
            out.push(p);
        }
    }
}

impl Wall {
    pub fn load(&mut self, filter: usize) {
        self.filter = filter.min(WE);
    }

    pub fn configure(&mut self, cfg: &Config) {
        if self.cfg_dir != cfg.wallpaper_dir {
            self.cfg_dir = cfg.wallpaper_dir.clone();
            self.dir = cfg.wallpaper_dir.clone();
            self.scanned = false;
        }
    }

    /// Открывает системный диалог выбора папки (в отдельном потоке: диалог блокирующий).
    fn choose_folder(&self) {
        if PICKING.swap(true, Ordering::SeqCst) {
            return;
        }
        let start = self.dir.clone().or_else(|| sys::pictures_dir().map(|p| p.display().to_string()));
        let spawned = std::thread::Builder::new().name("island-folder".into()).spawn(move || {
            let picked = sys::pick_folder(start.as_deref());
            if let (Some(p), Ok(mut slot)) = (picked, PICKED.lock()) {
                *slot = Some(p);
            }
            PICKING.store(false, Ordering::SeqCst);
            crate::bg::notify();
        });
        if let Err(e) = spawned {
            PICKING.store(false, Ordering::SeqCst);
            crate::log::error(&format!("не удалось запустить диалог выбора папки: {e}"));
        }
    }

    /// Применяет выбранную папку: запоминает в `config.toml` и пересобирает список.
    fn apply_picked(&mut self, dir: String) {
        crate::config::save_str("wallpaper_dir", &dir.replace('\\', "/"));
        self.dir = Some(dir);
        self.filter = ALL;
        self.scanned = false;
        self.show();
    }

    pub fn show(&mut self) -> bool {
        self.current = sys::current_wallpaper().map(|p| norm(&p)).unwrap_or_default();
        if !self.we_checked {
            self.we_checked = true;
            self.we = we::find();
        }
        if self.filter == WE && self.we.is_none() {
            self.filter = ALL;
        }
        let want_we = self.filter == WE;
        if let (true, Some(inst)) = (want_we, &self.we) {
            // Обои могли сменить в самом WE, пока остров был свёрнут.
            self.we_current = we::current(inst);
        }
        if self.scanned && self.loaded_we == want_we {
            self.center_on_current();
            return true;
        }
        self.scanned = true;
        self.loaded_we = want_we;
        self.recent = read_lines(FILE).into_iter().filter(|l| !l.is_empty()).take(MAX_RECENT).collect();

        self.files.clear();
        self.we_items.clear();
        self.dir_label.clear();
        match (want_we, &self.we) {
            (true, Some(inst)) => {
                self.we_items = we::scan(inst, MAX_FILES);
                self.files = self.we_items.iter().map(|i| i.preview.clone()).collect();
                self.dir_label = inst.roots.first().unwrap_or(&inst.dir).display().to_string();
            }
            _ => self.scan_folder(),
        }
        self.offset.value = 0.0;
        self.offset.target = 0.0;
        self.request_thumbs();
        self.center_on_current();
        true
    }

    /// Заново запрашивает миниатюры списка (новое поколение: старые битмапы больше не берутся).
    fn request_thumbs(&mut self) {
        self.pending.clear();
        self.decoded.clear();
        self.gen = thumbs::request(self.files.clone(), self.thumb.0, self.thumb.1);
    }

    /// Обои из папки (или «Изображений» и системных, если папка не выбрана).
    fn scan_folder(&mut self) {
        // Папку, которую выбрал пользователь, не подменяем системной: пустая папка так и скажет «нет изображений».
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(d) = &self.dir {
            candidates.push(PathBuf::from(d));
        } else {
            if let Some(p) = sys::pictures_dir() {
                candidates.push(p);
            }
            candidates.push(sys::windows_dir().join("Web").join("Wallpaper"));
        }
        for dir in candidates {
            scan(&dir, &mut self.files);
            if !self.files.is_empty() {
                self.dir_label = dir.display().to_string();
                break;
            }
            if self.dir_label.is_empty() {
                self.dir_label = dir.display().to_string();
            }
        }
    }

    /// Карточка `fi` — обои, которые сейчас стоят (в WE — на любом из мониторов).
    fn is_current(&self, fi: usize) -> bool {
        if self.loaded_we {
            let p = we::project_dir(&self.we_items[fi].project.display().to_string());
            self.we_current.contains(&p)
        } else {
            norm(&self.files[fi].display().to_string()) == self.current
        }
    }

    /// Карусель открывается на текущих обоях (иначе — на середине списка), чтобы карточки
    /// были с обеих сторон, как в макете.
    fn center_on_current(&mut self) {
        let items = self.shown();
        if items.is_empty() {
            return;
        }
        let pos = items.iter().position(|&i| self.is_current(i)).unwrap_or(items.len() / 2);
        self.offset.target = pos as f32;
        self.offset.snap();
    }

    pub fn step(&mut self, dt: f32) {
        self.offset.step(dt);
    }

    pub fn animating(&self) -> bool {
        !self.offset.settled()
    }

    /// Забирает готовые миниатюры. `true` — есть что показать.
    pub fn sync(&mut self) -> bool {
        let picked = PICKED.lock().ok().and_then(|mut p| p.take());
        let mut any = picked.is_some();
        if let Some(dir) = picked {
            self.apply_picked(dir);
        }
        let ready = thumbs::drain(self.gen);
        any |= !ready.is_empty();
        for (i, px) in ready {
            self.pending.insert(i, px);
            self.decoded.insert(i);
        }
        any
    }

    fn remember(&mut self, path: &str) {
        self.recent.retain(|p| norm(p) != norm(path));
        self.recent.insert(0, path.to_string());
        self.recent.truncate(MAX_RECENT);
        write_lines(FILE, &self.recent);
    }

    fn shown(&self) -> Vec<usize> {
        if self.filter != RECENT || self.loaded_we {
            return (0..self.files.len()).collect();
        }
        self.recent
            .iter()
            .filter_map(|r| self.files.iter().position(|f| norm(&f.display().to_string()) == norm(r)))
            .collect()
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32) {
        let mut items = self.shown();
        let f = &cv.g.fonts;

        // Верхний ряд: «Недавние» / «Папка» / «WE», счётчик, кнопка открытия папки.
        let y = TOP + 2.0;
        let mut x = PAD;
        let mut sections = vec![("Недавние", RECENT), ("Папка", ALL)];
        if self.we.is_some() {
            sections.push(("WE", WE));
        }
        for (i, (name, filt)) in sections.into_iter().enumerate() {
            let bw = cv.chip_w(name);
            if ui.chip(cv, id(WALLPAPER, 1 + i as u32), Rect::new(x, y, bw, 17.0), name, self.filter == filt) && self.filter != filt {
                self.filter = filt;
                crate::state::set_wallpaper_filter(self.filter);
                self.offset.target = 0.0;
                // Переход между папкой и WE пересобирает список (миниатюры берутся из кэша на диске).
                self.show();
                // `show` подменил `files`: старые индексы карточек дальше в кадре брать нельзя.
                items = self.shown();
            }
            x += bw + 5.0;
        }
        let n = items.len();
        let max_off = n.saturating_sub(1) as f32;
        cv.text(&format!("{n} {}", crate::i18n::pick("изобр.", "images")), Rect::new(x + 4.0, y, 100.0, 17.0), &f.small, bare());
        let fb = Rect::new(w - PAD - 19.0, y - 1.0, 19.0, 19.0);
        let pick_w = cv.chip_w("Выбрать папку…");
        if ui.chip(cv, id(WALLPAPER, 8), Rect::new(fb.x - 6.0 - pick_w, y, pick_w, 17.0), "Выбрать папку…", false) {
            self.choose_folder();
        }
        cv.circle(fb.cx(), fb.cy(), 9.5, card());
        if ui.icon_button(cv, id(WALLPAPER, 7), fb, icon::FOLDER, false, crate::i18n::pick("Выбрать папку", "Choose folder")) && !self.dir_label.is_empty() {
            sys::open(&self.dir_label);
        }

        // Карусель как в макете: центральная карточка широкая, соседние узкие; все наклонены.
        let wheel = ui.wheel_in(Rect::new(0.0, TOP, w, h - TOP));
        if wheel != 0.0 {
            self.offset.target = (self.offset.target - wheel).round();
        }
        self.offset.target = self.offset.target.clamp(0.0, max_off);

        // Масштаб берём из текущего преобразования: он учитывает и DPI монитора, и масштаб интерфейса.
        let mut m = windows_numerics::Matrix3x2::default();
        cv.g.dc.GetTransform(&mut m);
        let want = thumb_size(m.M11);
        if want != self.thumb {
            self.thumb = want;
            self.request_thumbs();
        }

        const SIDE_W: f32 = 46.0;
        const FIRST: f32 = 150.0;
        const STEP: f32 = 54.0;
        let (cx, cy) = (w / 2.0, TOP + 108.0);
        let mut vis: Vec<(usize, f32)> = (0..n)
            .map(|p| (p, p as f32 - self.offset.value))
            .filter(|(_, r)| r.abs() < 3.4)
            .collect();
        vis.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));

        cv.g.dc.PushAxisAlignedClip(
            &D2D_RECT_F { left: PAD, top: TOP + 24.0, right: w - PAD, bottom: h - 4.0 },
            D2D1_ANTIALIAS_MODE_ALIASED,
        );
        let mut chosen = None;
        // Название проекта WE под центральной карточкой: (позиция, |r|).
        let mut center: Option<(usize, f32)> = None;
        for (pos, r) in vis {
            let a = r.abs().min(1.0);
            let cw = SIDE_W + (CENTER_W - SIDE_W) * (1.0 - a);
            let ch = 126.0 + 10.0 * (1.0 - a);
            let d = if r.abs() <= 1.0 { r.abs() * FIRST } else { FIRST + (r.abs() - 1.0) * STEP };
            let xc = cx + if r < 0.0 { -d } else { d };
            // Центр параллелограмма по середине высоты, нижняя грань левее верхней на SLANT.
            let rect = Rect::new(xc - cw / 2.0 - SLANT / 2.0, cy - ch / 2.0, cw, ch);
            let fade = (1.0 - (r.abs() - 2.4).max(0.0)).clamp(0.0, 1.0);

            let fi = items[pos];
            if r.abs() < 0.5 {
                center = Some((fi, r.abs()));
            }
            let key = (0x57u64 << 56) | ((self.gen as u64) << 32) | fi as u64;
            // Пиксели из `pending` уходят в видеопамять; вытесненный оттуда битмап читается с диска.
            let (tw, th) = self.thumb;
            let (pending, decoded, src) = (&mut self.pending, &self.decoded, &self.files[fi]);
            let bmp = ui.cache.get(&cv.g.dc, key, tw, th, || {
                pending
                    .remove(&fi)
                    .or_else(|| decoded.contains(&fi).then(|| thumbs::load(src, tw, th)).flatten())
                    .unwrap_or_default()
            });
            let cid = id(WALLPAPER, 100 + pos as u32);
            let hit = Rect::new(rect.x, rect.y, rect.w + SLANT, rect.h);
            let (_, clicked) = ui.click_area(cid, hit);
            let hv = ui.hover_of(cid);
            cv.bitmap_quad(bmp.as_ref(), rect, SLANT, fade * (0.88 + 0.12 * hv.max(1.0 - a)));

            if self.is_current(fi) && fade > 0.5 {
                let b = Rect::new(rect.right() + SLANT - 22.0, rect.y + 6.0, 15.0, 15.0);
                cv.circle(b.cx(), b.cy(), 7.5, cv.accent);
                cv.text(icon::CHECK, b, &f.icon_small, ink());
            }
            if clicked {
                if r.abs() < 0.5 {
                    chosen = Some(fi);
                } else {
                    self.offset.target = pos as f32;
                }
            }
        }
        cv.g.dc.PopAxisAlignedClip();

        // У проектов WE имена файлов ничего не говорят («preview.jpg»), поэтому подписываем название.
        if let (true, Some((fi, d))) = (self.loaded_we, center) {
            let c = bare();
            let col = windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F { a: c.a * (1.0 - d * 2.0).clamp(0.0, 1.0), ..c };
            let title = &self.we_items[fi].title;
            cv.text(title, Rect::new(PAD, cy + 71.0, w - 2.0 * PAD, 13.0), &f.small_c, col);
        }

        if n == 0 {
            let msg = if self.loaded_we {
                "Обои Wallpaper Engine не найдены".to_string()
            } else if self.filter == RECENT && !self.files.is_empty() {
                "Недавних обоев пока нет".to_string()
            } else {
                format!("{}: {}", crate::i18n::pick("В папке нет изображений", "No images in the folder"), self.dir_label)
            };
            cv.text(&msg, Rect::new(PAD, cy - 8.0, w - 2.0 * PAD, 16.0), &f.small_c, bare());
        }

        if let Some(fi) = chosen {
            if self.loaded_we {
                self.apply_we(fi);
            } else {
                self.apply_static(fi);
            }
        }
        ui.paint_tip(cv, Rect::new(0.0, 0.0, w, h));
    }

    /// Включает проект WE: его рендерит сам Wallpaper Engine (анимация, плагины, настройки проекта).
    fn apply_we(&mut self, fi: usize) {
        let Some(inst) = self.we.clone() else { return };
        let project = self.we_items[fi].project.clone();
        self.we_current = vec![we::project_dir(&project.display().to_string())];
        // Запуск WE и команды ему — внешние процессы, ждать их в кадре нельзя.
        std::thread::spawn(move || {
            if !we::apply(&inst, &project) {
                crate::log::error(&format!("Wallpaper Engine не включил обои: {}", project.display()));
            }
        });
    }

    /// Ставит картинку обоями Windows. Обои WE закрывают рабочий стол поверх неё, поэтому их убираем.
    fn apply_static(&mut self, fi: usize) {
        let s = self.files[fi].display().to_string();
        self.current = norm(&s);
        self.we_current.clear();
        self.remember(&s);
        let inst = self.we.clone();
        // SPI_SETDESKWALLPAPER рассылает сообщения всем окнам — не блокируем кадр.
        std::thread::spawn(move || {
            if let Some(inst) = inst {
                we::close_all(&inst);
            }
            if !sys::set_wallpaper(&s) {
                crate::log::error(&format!("не удалось сменить обои: {s}"));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_extensions() {
        assert!(is_image(Path::new("a/b.JPG")));
        assert!(is_image(Path::new("x.png")));
        assert!(!is_image(Path::new("x.gif")));
        assert!(!is_image(Path::new("noext")));
    }

    #[test]
    fn scan_takes_images_from_root_and_one_level_only() {
        let root = std::env::temp_dir().join(format!("island-wall-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let touch = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        touch("a.jpg");
        touch("notes.txt");
        touch("movie.mp4");
        touch("sub/b.PNG");
        touch("sub/readme.md");
        touch("sub/deep/c.jpg");
        touch("other/d.bmp");
        std::fs::create_dir_all(root.join("fake.png")).unwrap();

        let mut out = Vec::new();
        scan(&root, &mut out);
        let names: Vec<String> = out.iter().map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(names, ["a.jpg", "other/d.bmp", "sub/b.PNG"]);
    }

    #[test]
    fn thumbs_cover_the_center_card_on_screen() {
        assert_eq!(thumb_size(1.0), (384, 216));
        for s in [1.25, 1.5, 1.8, 2.0, 2.5, 3.0] {
            let (w, h) = thumb_size(s);
            assert!(w as f32 >= (CENTER_W + SLANT) * s, "{s}: {w}");
            assert_eq!(w % 64, 0);
            assert_eq!(w * 9, h * 16);
        }
        assert_eq!(thumb_size(10.0).0, 1280, "размер ограничен сверху");
    }

    #[test]
    fn path_normalization() {
        assert_eq!(norm("C:\\Pics\\A.JPG"), "c:/pics/a.jpg");
    }

    #[test]
    fn recent_is_unique_and_bounded() {
        let mut w = Wall::default();
        // `remember` пишет файл; проверяем только порядок через прямую логику.
        for i in 0..20 {
            w.recent.retain(|p| norm(p) != norm(&format!("p{i}")));
            w.recent.insert(0, format!("p{i}"));
            w.recent.truncate(MAX_RECENT);
        }
        assert_eq!(w.recent.len(), MAX_RECENT);
        assert_eq!(w.recent[0], "p19");
    }
}
