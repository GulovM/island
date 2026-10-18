//! Вкладка Wallpaper: карусель обоев из папки (`wallpaper_dir` в конфиге, по умолчанию
//! «Изображения»), миниатюры декодируются в фоне (`thumbs`), смена обоев и список недавних.
//! Миниатюры живут в кэше битмапов видеопамяти; обои меняются через SystemParametersInfo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED;

use crate::anim::Spring;
use crate::config::Config;
use crate::ui::{icon, Canvas, Rect, Ui, card, dim, ink};
use crate::{sys, thumbs};

use super::{id, read_lines, write_lines, PAD, TOP, WALLPAPER};

const FILE: &str = "wallpapers.txt";
const MAX_FILES: usize = 40;
const MAX_RECENT: usize = 12;

pub struct Wall {
    cfg_dir: Option<String>,
    files: Vec<PathBuf>,
    scanned: bool,
    gen: u32,
    /// Пиксели миниатюр, ещё не загруженные в видеопамять.
    pending: HashMap<usize, Vec<u8>>,
    /// Положение карусели в карточках.
    offset: Spring,
    /// Путь текущих обоев (в нижнем регистре, с `/`).
    current: String,
    recent: Vec<String>,
    /// 0 — все, 1 — недавние.
    filter: usize,
    dir_label: String,
}

impl Default for Wall {
    fn default() -> Self {
        Wall {
            cfg_dir: None,
            files: Vec::new(),
            scanned: false,
            gen: 0,
            pending: HashMap::new(),
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

fn scan(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if out.len() >= MAX_FILES {
            return;
        }
        if p.is_dir() {
            if depth < 2 {
                scan(&p, depth + 1, out);
            }
        } else if is_image(&p) {
            out.push(p);
        }
    }
}

impl Wall {
    pub fn configure(&mut self, cfg: &Config) {
        if self.cfg_dir != cfg.wallpaper_dir {
            self.cfg_dir = cfg.wallpaper_dir.clone();
            self.scanned = false;
        }
    }

    pub fn show(&mut self) -> bool {
        self.current = sys::current_wallpaper().map(|p| norm(&p)).unwrap_or_default();
        if self.scanned {
            self.center_on_current();
            return true;
        }
        self.scanned = true;
        self.recent = read_lines(FILE).into_iter().filter(|l| !l.is_empty()).take(MAX_RECENT).collect();

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(d) = &self.cfg_dir {
            candidates.push(PathBuf::from(d));
        } else if let Some(p) = sys::pictures_dir() {
            candidates.push(p);
        }
        candidates.push(sys::windows_dir().join("Web").join("Wallpaper"));

        self.files.clear();
        for dir in candidates {
            scan(&dir, 0, &mut self.files);
            if !self.files.is_empty() {
                self.dir_label = dir.display().to_string();
                break;
            }
            if self.dir_label.is_empty() {
                self.dir_label = dir.display().to_string();
            }
        }
        self.pending.clear();
        self.offset.value = 0.0;
        self.offset.target = 0.0;
        self.gen = thumbs::request(self.files.clone());
        self.center_on_current();
        true
    }

    /// Карусель открывается на текущих обоях (иначе — на середине списка), чтобы карточки
    /// были с обеих сторон, как в макете.
    fn center_on_current(&mut self) {
        let items = self.shown();
        if items.is_empty() {
            return;
        }
        let pos = items
            .iter()
            .position(|&i| norm(&self.files[i].display().to_string()) == self.current)
            .unwrap_or(items.len() / 2);
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
        let ready = thumbs::drain(self.gen);
        let any = !ready.is_empty();
        for (i, px) in ready {
            self.pending.insert(i, px);
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
        if self.filter == 0 {
            return (0..self.files.len()).collect();
        }
        self.recent
            .iter()
            .filter_map(|r| self.files.iter().position(|f| norm(&f.display().to_string()) == norm(r)))
            .collect()
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32) {
        let items = self.shown();
        let n = items.len();
        let max_off = n.saturating_sub(1) as f32;
        let f = &cv.g.fonts;

        // Верхний ряд: «Недавние» / «Папка», счётчик, кнопка открытия папки.
        let y = TOP + 2.0;
        let mut x = PAD;
        for (i, (name, filt)) in [("Недавние", 1usize), ("Папка", 0usize)].iter().enumerate() {
            let bw = name.chars().count() as f32 * 5.0 + 16.0;
            if ui.chip(cv, id(WALLPAPER, 1 + i as u32), Rect::new(x, y, bw, 17.0), name, self.filter == *filt) {
                self.filter = *filt;
                self.offset.target = 0.0;
            }
            x += bw + 5.0;
        }
        cv.text(&format!("{n} {}", crate::i18n::pick("изобр.", "images")), Rect::new(x + 4.0, y, 100.0, 17.0), &f.small, dim());
        let fb = Rect::new(w - PAD - 19.0, y - 1.0, 19.0, 19.0);
        cv.circle(fb.cx(), fb.cy(), 9.5, card());
        if ui.icon_button(cv, id(WALLPAPER, 7), fb, icon::FOLDER, false) && !self.dir_label.is_empty() {
            sys::open(&self.dir_label);
        }

        // Карусель как в макете: центральная карточка широкая, соседние узкие; все наклонены.
        let wheel = ui.wheel_in(Rect::new(0.0, TOP, w, h - TOP));
        if wheel != 0.0 {
            self.offset.target = (self.offset.target - wheel).round();
        }
        self.offset.target = self.offset.target.clamp(0.0, max_off);

        const CENTER_W: f32 = 240.0;
        const SIDE_W: f32 = 46.0;
        const SLANT: f32 = 22.0;
        const FIRST: f32 = 150.0;
        const STEP: f32 = 54.0;
        let (cx, cy) = (w / 2.0, TOP + 108.0);
        let mut vis: Vec<(usize, f32)> = (0..n)
            .map(|p| (p, p as f32 - self.offset.value))
            .filter(|(_, r)| r.abs() < 4.6)
            .collect();
        vis.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));

        cv.g.dc.PushAxisAlignedClip(
            &D2D_RECT_F { left: PAD, top: TOP + 24.0, right: w - PAD, bottom: h - 4.0 },
            D2D1_ANTIALIAS_MODE_ALIASED,
        );
        let mut chosen = None;
        for (pos, r) in vis {
            let a = r.abs().min(1.0);
            let cw = SIDE_W + (CENTER_W - SIDE_W) * (1.0 - a);
            let ch = 126.0 + 10.0 * (1.0 - a);
            let d = if r.abs() <= 1.0 { r.abs() * FIRST } else { FIRST + (r.abs() - 1.0) * STEP };
            let xc = cx + if r < 0.0 { -d } else { d };
            // Центр параллелограмма по середине высоты, нижняя грань левее верхней на SLANT.
            let rect = Rect::new(xc - cw / 2.0 - SLANT / 2.0, cy - ch / 2.0, cw, ch);
            let fade = (1.0 - (r.abs() - 3.6).max(0.0)).clamp(0.0, 1.0);

            let fi = items[pos];
            let path = self.files[fi].clone();
            let key = (0x57u64 << 56) | ((self.gen as u64) << 32) | fi as u64;
            // Пиксели остаются в `pending` до смены папки: вытесненный из кэша битмап грузится заново.
            let pending = &self.pending;
            let bmp = ui.cache.get(&cv.g.dc, key, thumbs::W, thumbs::H, || pending.get(&fi).cloned().unwrap_or_default());
            let cid = id(WALLPAPER, 100 + pos as u32);
            let hit = Rect::new(rect.x, rect.y, rect.w + SLANT, rect.h);
            let (_, clicked) = ui.click_area(cid, hit);
            let hv = ui.hover_of(cid);
            cv.bitmap_quad(bmp.as_ref(), rect, SLANT, fade * (0.88 + 0.12 * hv.max(1.0 - a)));

            if norm(&path.display().to_string()) == self.current && fade > 0.5 {
                let b = Rect::new(rect.right() + SLANT - 22.0, rect.y + 6.0, 15.0, 15.0);
                cv.circle(b.cx(), b.cy(), 7.5, cv.accent);
                cv.text(icon::CHECK, b, &f.icon_small, ink());
            }
            if clicked {
                if r.abs() < 0.5 {
                    chosen = Some(path);
                } else {
                    self.offset.target = pos as f32;
                }
            }
        }
        cv.g.dc.PopAxisAlignedClip();

        if n == 0 {
            let msg = if self.filter == 1 && !self.files.is_empty() {
                "Недавних обоев пока нет".to_string()
            } else {
                format!("{}: {}", crate::i18n::pick("В папке нет изображений", "No images in the folder"), self.dir_label)
            };
            cv.text(&msg, Rect::new(PAD, cy - 8.0, w - 2.0 * PAD, 16.0), &f.small_c, dim());
        }

        if let Some(path) = chosen {
            let s = path.display().to_string();
            self.current = norm(&s);
            self.remember(&s);
            // SPI_SETDESKWALLPAPER рассылает сообщения всем окнам — не блокируем кадр.
            std::thread::spawn(move || {
                if !sys::set_wallpaper(&s) {
                    crate::log::error(&format!("не удалось сменить обои: {s}"));
                }
            });
        }
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
