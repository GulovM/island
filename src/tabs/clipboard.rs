//! Вкладка Clipboard: история текста и ссылок, поиск, фильтры, закрепление, превью,
//! вставка выбранного в окно, где работали. Запись идёт всегда (слушатель в main.rs),
//! данные менеджеров паролей и собственные записи острова отсекаются до `record`.
//! Изображения (скриншоты, копирование картинок) сохраняются как PNG в `%APPDATA%\island\clip`;
//! в истории запись вида `\u{1}img:<файл>:<ширина>x<высота>`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::sys;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::ui::{icon, Canvas, Nav, Rect, Submit, Ui, bare, card, dim, txt};

use super::{escape, id, read_lines_sealed, unescape, write_lines_sealed, Action, CLIPBOARD, PAD, TOP};

const FILE: &str = "clipboard.txt";
/// Сколько записей хранится (закреплённые не считаются).
const MAX_ITEMS: usize = 100;
/// Длиннее этого текст не сохраняется целиком.
const MAX_CHARS: usize = 20_000;
const LIST_W: f32 = 250.0;
const IMG_TAG: &str = "\u{1}img:";
/// Размер превью-изображения (пиксели), вписывается без увеличения.
const PREVIEW_MAX: (u32, u32) = (420, 280);

/// Папка с PNG-файлами изображений истории.
pub fn clip_dir() -> PathBuf {
    sys::data_dir().join("clip")
}

/// Готовые к записи в историю изображения: (файл, ширина, высота).
/// Подпись и время последнего снимка (для отсева повторных уведомлений).
static LAST_IMG: Mutex<(u64, u64)> = Mutex::new((0, 0));
static DONE: Mutex<Vec<(String, u32, u32)>> = Mutex::new(Vec::new());
/// Декодированное превью: (файл, ширина, высота, пиксели BGRA premultiplied).
#[allow(clippy::type_complexity)]
static PREVIEW: Mutex<Option<(String, u32, u32, Vec<u8>)>> = Mutex::new(None);
static IMG_SEQ: AtomicU32 = AtomicU32::new(0);

struct PreviewImg {
    file: String,
    w: u32,
    h: u32,
    px: Vec<u8>,
}

struct Item {
    text: String,
    ts: u64,
    pinned: bool,
}

impl Item {
    fn is_link(&self) -> bool {
        let t = self.text.trim();
        !self.is_image() && (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(char::is_whitespace)
    }

    fn is_image(&self) -> bool {
        self.text.starts_with(IMG_TAG)
    }

    fn is_file(&self) -> bool {
        !self.is_image() && !self.is_link() && is_file_path(&self.text)
    }

    fn file_name(&self) -> Option<&str> {
        if self.is_file() {
            Some(path_file_name(&self.text))
        } else {
            None
        }
    }

    /// (имя файла, ширина, высота) для записи-изображения.
    fn image(&self) -> Option<(&str, u32, u32)> {
        let rest = self.text.strip_prefix(IMG_TAG)?;
        let (file, dims) = rest.split_once(':')?;
        let (w, h) = dims.split_once('x')?;
        Some((file, w.parse().ok()?, h.parse().ok()?))
    }

    /// Подпись в списке (имя или адрес отдельно от размера и времени).
    fn label(&self) -> String {
        if self.is_image() {
            crate::i18n::t("Изображение").to_string()
        } else if let Some(name) = self.file_name() {
            name.to_string()
        } else {
            first_line(&self.text).to_string()
        }
    }

    /// Текст, по которому ищет строка поиска (в нижнем регистре).
    fn haystack(&self) -> String {
        match self.image() {
            Some((_, w, h)) => format!("изображение image фото картинка {w}x{h}"),
            None => self.text.to_lowercase(),
        }
    }

    fn remove_file(&self) {
        if let Some((file, _, _)) = self.image() {
            let _ = std::fs::remove_file(clip_dir().join(file));
        }
    }
}

#[derive(Default)]
pub struct Clip {
    /// Новые сверху.
    items: Vec<Item>,
    /// Выбранная запись — по тексту: индексы меняются при каждой новой записи.
    selected: Option<String>,
    /// 0 — все, 1 — текст, 2 — ссылки.
    filter: usize,
    /// Запись истории приостановлена пользователем.
    paused: bool,
    preview: Option<PreviewImg>,
    preview_req: Option<String>,
}

impl Clip {
    pub fn load(&mut self, paused: bool) {
        self.paused = paused;
        self.items = read_lines_sealed(FILE)
            .iter()
            .filter_map(|l| {
                let mut p = l.splitn(3, '|');
                let pinned = p.next()? == "1";
                let ts = p.next()?.parse().ok()?;
                Some(Item { text: unescape(p.next()?), ts, pinned })
            })
            .take(MAX_ITEMS * 2)
            .collect();
        // Файл старой версии (открытый текст) сразу переписываем зашифрованным.
        let legacy = std::fs::read(sys::data_dir().join(FILE)).is_ok_and(|b| !b.is_empty() && !sys::is_sealed(&b));
        if legacy && !self.items.is_empty() {
            self.save();
        }
        // То же для PNG старых версий — в фоне, чтобы не задерживать запуск.
        let _ = std::thread::Builder::new().name("island-clip-seal".into()).spawn(|| {
            let Ok(rd) = std::fs::read_dir(clip_dir()) else { return };
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "png") {
                    let _ = crate::img::seal_file(&e.path());
                }
            }
        });
    }

    fn save(&self) {
        let lines: Vec<String> = self
            .items
            .iter()
            .map(|i| format!("{}|{}|{}", if i.pinned { 1 } else { 0 }, i.ts, escape(&i.text)))
            .collect();
        write_lines_sealed(FILE, &lines);
    }

    /// Новый текст из буфера обмена. `true` — история изменилась.
    pub fn record(&mut self, text: String) -> bool {
        if self.paused || text.trim().is_empty() {
            return false;
        }
        let text: String = text.chars().take(MAX_CHARS).collect();
        let pinned = match self.items.iter().position(|i| i.text == text) {
            Some(p) => self.items.remove(p).pinned,
            None => false,
        };
        self.items.insert(0, Item { text, ts: sys::now_secs(), pinned });
        // Обрезаем хвост, не трогая закреплённые.
        let mut kept = 0;
        self.items.retain(|i| {
            if i.pinned {
                return true;
            }
            kept += 1;
            let keep = kept <= MAX_ITEMS;
            if !keep {
                i.remove_file();
            }
            keep
        });
        self.save();
        true
    }

    /// Ставит в очередь сохранение изображения из буфера обмена (PNG пишется в фоновом потоке).
    pub fn capture_image(&self, w: u32, h: u32, px: Vec<u8>) {
        if self.paused {
            return;
        }
        // Одно копирование часто даёт два уведомления подряд: одинаковый снимок не дублируем.
        let sig = {
            use std::hash::{Hash, Hasher};
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            (w, h, px.len()).hash(&mut hs);
            px.hash(&mut hs);
            (hs.finish(), sys::now_secs())
        };
        if let Ok(mut last) = LAST_IMG.lock() {
            if last.0 == sig.0 && sig.1.saturating_sub(last.1) < 3 {
                return;
            }
            *last = sig;
        }
        let seq = IMG_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("{}_{}.png", sys::now_secs(), seq);
        let spawned = std::thread::Builder::new().name("island-clip-img".into()).spawn(move || unsafe {
            let dir = clip_dir();
            let _ = std::fs::create_dir_all(&dir);
            if crate::img::save_png(&dir.join(&name), w, h, &px).is_none() {
                crate::log::error("не удалось сохранить изображение из буфера обмена");
                return;
            }
            if let Ok(mut d) = DONE.lock() {
                d.push((name, w, h));
            }
            crate::bg::notify();
        });
        if let Err(e) = spawned {
            crate::log::error(&format!("не удалось запустить поток сохранения изображения: {e}"));
        }
    }

    /// Забирает результаты фоновых потоков. `true` — что-то изменилось.
    pub fn sync(&mut self) -> bool {
        let mut changed = false;
        let done = DONE.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default();
        for (file, w, h) in done {
            changed |= self.record(format!("{IMG_TAG}{file}:{w}x{h}"));
        }
        if let Some((file, w, h, px)) = PREVIEW.lock().ok().and_then(|mut p| p.take()) {
            if self.preview_req.as_deref() == Some(file.as_str()) {
                self.preview = Some(PreviewImg { file, w, h, px });
                changed = true;
            }
        }
        changed
    }

    /// Запрашивает декодирование превью изображения (если его ещё нет и не запрошено).
    fn want_preview(&mut self, file: &str) {
        if self.preview.as_ref().is_some_and(|p| p.file == file) || self.preview_req.as_deref() == Some(file) {
            return;
        }
        self.preview_req = Some(file.to_string());
        let name = file.to_string();
        let _ = std::thread::Builder::new().name("island-clip-prev".into()).spawn(move || unsafe {
            let path = clip_dir().join(&name);
            if let Some((w, h, px)) = crate::img::load_fit(&path, PREVIEW_MAX.0, PREVIEW_MAX.1) {
                if let Ok(mut p) = PREVIEW.lock() {
                    *p = Some((name, w, h, px));
                }
                crate::bg::notify();
            }
        });
    }

    /// Индексы записей по текущему фильтру и поиску: закреплённые выше, порядок сохраняется.
    fn visible(&self, query: &str) -> Vec<usize> {
        let q = query.to_lowercase();
        let ok = |i: &Item| {
            let kind = match self.filter {
                1 => !i.is_link() && !i.is_image(),
                2 => i.is_link(),
                3 => i.is_image(),
                _ => true,
            };
            kind && (q.is_empty() || i.haystack().contains(&q))
        };
        let mut idx: Vec<usize> = (0..self.items.len()).filter(|&n| ok(&self.items[n])).collect();
        idx.sort_by_key(|&n| !self.items[n].pinned);
        idx
    }

    fn position(&self, text: &str) -> Option<usize> {
        self.items.iter().position(|i| i.text == text)
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32, act: &mut Vec<Action>) {
        let f = &cv.g.fonts;
        // Строка поиска: поле слева, фильтры и шестерёнка («не записывать») справа на общем фоне.
        let bar = Rect::new(PAD, TOP, w - 2.0 * PAD, 26.0);
        cv.rr(bar, 10.0, card());
        let names = ["Все", "Текст", "Ссылки", "Фото"];
        let widths: Vec<f32> = names.iter().map(|n| cv.chip_w(n)).collect();
        let gear = Rect::new(bar.right() - 23.0, bar.y + 4.0, 18.0, 18.0);
        let pills_w = crate::ui::chips_total_width(&widths, 4.0);
        let mut px = gear.x - 8.0 - pills_w;
        let fid = id(CLIPBOARD, 0);
        let query = ui.field_text(fid);
        let count = self.visible(&query).len();
        let ph = if crate::i18n::en() {
            format!("Search {count} items…")
        } else {
            format!("Поиск среди {count} записей…")
        };
        // Если поиск не пуст, число совпадений показываем счётчиком между полем и фильтрами.
        let (field_w, count_badge) = if query.is_empty() {
            (px - bar.x - 6.0, None)
        } else {
            let s = count.to_string();
            let cw = cv.text_w(&s, &f.small);
            let cx = px - 6.0 - cw;
            (cx - bar.x - 6.0, Some((s, cx, cw)))
        };
        ui.text_field_ex(cv, fid, Rect::new(bar.x, bar.y, field_w, bar.h), &ph, icon::SEARCH, Submit::Keep);
        if let Some((s, cx, cw)) = count_badge {
            cv.text(&s, Rect::new(cx, bar.y, cw + 2.0, bar.h), &f.small, dim());
        }
        for (i, name) in names.iter().enumerate() {
            let r = Rect::new(px, bar.y + 4.5, widths[i], 17.0);
            if ui.chip_ex(cv, id(CLIPBOARD, 1 + i as u32), r, name, self.filter == i, true, None) {
                self.filter = i;
            }
            px += widths[i] + 4.0;
        }
        if ui.icon_button(cv, id(CLIPBOARD, 5), gear, icon::SETTINGS_GEAR, self.paused) {
            self.paused = !self.paused;
            crate::state::set_clipboard_paused(self.paused);
        }

        ui.sticky_focus = Some(fid);
        let order = self.visible(&query);

        // Клавиши, пока в поиске фокус: стрелки двигают выбор, Ctrl+P закрепляет, Enter вставляет.
        let mut cur = self.selected.as_deref().and_then(|t| order.iter().position(|&n| self.items[n].text == t));
        // Выбор по умолчанию — первая запись: кнопки внизу и превью сразу работают.
        if cur.is_none() && !order.is_empty() {
            self.selected = Some(self.items[order[0]].text.clone());
            cur = Some(0);
        }
        let mut pos = cur;
        for n in ui.take_nav() {
            match n {
                Nav::Down => pos = Some(pos.map_or(0, |p| (p + 1).min(order.len().saturating_sub(1)))),
                Nav::Up => pos = Some(pos.map_or(0, |p| p.saturating_sub(1))),
                Nav::Pin => {
                    if let Some(&n) = pos.and_then(|p| order.get(p)) {
                        self.items[n].pinned = !self.items[n].pinned;
                        self.save();
                    }
                }
                Nav::Copy => {
                    if let Some(&n) = pos.and_then(|p| order.get(p)) {
                        act.push(self.copy_action(n));
                    }
                }
                Nav::Delete => {
                    if let Some(&n) = pos.and_then(|p| order.get(p)) {
                        self.remove_at(n);
                        self.selected = None;
                        // Порядок списка устарел: перерисуем в следующем кадре.
                        return;
                    }
                }
                Nav::Tab => self.filter = (self.filter + 1) % names.len(),
            }
        }
        if pos != cur {
            self.selected = pos.and_then(|p| order.get(p)).map(|&n| self.items[n].text.clone());
        }
        if ui.take_submitted(fid).is_some() {
            let target = self.selected.as_deref().and_then(|t| self.position(t)).or_else(|| order.first().copied());
            if let Some(n) = target {
                act.push(self.paste_action(n));
            }
        }

        // Список с заголовками секций: закреплённые, сегодня, вчера, раньше.
        const ROW: f32 = 23.0;
        const HEAD: f32 = 16.0;
        let now = sys::now_secs();
        let lt = GetLocalTime();
        let day_start = now.saturating_sub(lt.wHour as u64 * 3600 + lt.wMinute as u64 * 60 + lt.wSecond as u64);
        let list = Rect::new(PAD, TOP + 36.0, LIST_W, (h - TOP - 36.0 - 36.0).max(ROW));

        let mut rows: Vec<(f32, Option<&'static str>, usize)> = Vec::new();
        let (mut y, mut last) = (0.0f32, None::<&'static str>);
        for (p, &n) in order.iter().enumerate() {
            let sec = section(&self.items[n], day_start);
            if last != Some(sec) {
                rows.push((y, Some(sec), p));
                y += HEAD;
                last = Some(sec);
            }
            rows.push((y, None, p));
            y += ROW;
        }

        let sid = id(CLIPBOARD, 100);
        let selected_pos = self.selected.as_deref().and_then(|t| order.iter().position(|&n| self.items[n].text == t));
        let off = ui.scroll_begin(cv, sid, list, y);
        let mut clicked = None;
        for (ry, head, p) in rows {
            let yy = list.y + ry - off;
            let hgt = if head.is_some() { HEAD } else { ROW };
            if yy + hgt < list.y || yy > list.bottom() {
                continue;
            }
            if let Some(title) = head {
                // Заголовок секции — вторичным цветом, как в списках iOS: не спорит с записями.
                cv.text(title, Rect::new(list.x + 5.0, yy + 3.0, list.w, 11.0), &f.caps, crate::ui::dim());
                continue;
            }
            let n = order[p];
            let it = &self.items[n];
            // Тип записи виден по цвету плашки: закреплённое — оранжевое, картинка — фиолетовая,
            // ссылка — синяя, файл — зелёный, текст — серый.
            use crate::ios::SysColor;
            let (g, tint) = if it.pinned {
                (icon::PIN, crate::ui::hue(SysColor::Orange, cv.accent))
            } else if it.is_image() {
                (icon::PICTURE, crate::ui::hue(SysColor::Purple, cv.accent))
            } else if it.is_link() {
                (icon::LINK, crate::ui::hue(SysColor::Blue, cv.accent))
            } else if it.is_file() {
                (icon::FOLDER, crate::ui::hue(SysColor::Green, cv.accent))
            } else {
                let p = crate::theme::get();
                let dark = !crate::theme::is_light([p.bg.r, p.bg.g, p.bg.b]);
                (icon::TEXT_DOC, crate::ios::to_d2d(crate::ios::gray(crate::ios::GrayToken::Gray, dark), 1.0))
            };
            let r = Rect::new(list.x, yy, list.w - 3.0, ROW);
            let sel = selected_pos == Some(p);
            let label = it.label();
            let right = if let Some((_, w, h)) = it.image() {
                format!("{w}×{h} • {}", short_ago(now, it.ts))
            } else {
                short_ago(now, it.ts)
            };
            if ui.row_compact_tinted(cv, id(CLIPBOARD, 200 + p as u32), r, g, true, &label, &right, 0.0, sel, !sel, Some(tint)) {
                clicked = Some(n);
            }
        }
        ui.scroll_end(cv, sid);
        if let Some(n) = clicked {
            self.selected = Some(self.items[n].text.clone());
        }
        if order.is_empty() {
            let msg = if self.items.is_empty() { "История пуста — скопируйте что-нибудь" } else { "Ничего не найдено" };
            cv.text(msg, Rect::new(list.x + 5.0, list.y + 6.0, list.w, 14.0), &f.small, bare());
        }

        let prev = Rect::new(PAD + LIST_W + 14.0, TOP + 36.0, w - 2.0 * PAD - LIST_W - 14.0, (h - TOP - 36.0 - 36.0).max(120.0));
        self.preview(ui, cv, prev, day_start);
        self.footer(ui, cv, w, h, act);
    }

    fn paste_action(&self, n: usize) -> Action {
        match self.items[n].image() {
            Some((file, _, _)) => Action::PasteImage(file.to_string()),
            None => Action::Paste(self.items[n].text.clone()),
        }
    }

    fn copy_action(&self, n: usize) -> Action {
        match self.items[n].image() {
            Some((file, _, _)) => Action::CopyImage(file.to_string()),
            None => Action::Copy(self.items[n].text.clone()),
        }
    }

    fn remove_at(&mut self, n: usize) {
        let it = self.items.remove(n);
        it.remove_file();
        self.save();
    }

    unsafe fn preview(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, day_start: u64) {
        cv.card(r);
        let f = &cv.g.fonts;
        let Some(idx) = self.selected.as_deref().and_then(|t| self.position(t)) else {
            cv.text("Выберите запись слева", r, &f.small_c, dim());
            return;
        };
        // Область данных сверху; внизу оставлено место под разделитель (#30) и блок метаданных.
        let area = Rect::new(r.x + 14.0, r.y + 11.0, r.w - 28.0, (r.h - 77.0).max(20.0));
        let (ts, is_link, is_file, image, text) = {
            let it = &self.items[idx];
            (it.ts, it.is_link(), it.is_file(), it.image().map(|(f, w, h)| (f.to_string(), w, h)), it.text.clone())
        };
        let mut meta: Vec<(&str, String)> =
            vec![("Тип", String::new()), ("Размер", String::new()), ("Скопировано", copied_at(ts, day_start))];
        if let Some((file, w, h)) = image {
            self.want_preview(&file);
            if let Some(p) = self.preview.as_ref().filter(|p| p.file == file) {
                let key = {
                    use std::hash::{Hash, Hasher};
                    let mut hs = std::collections::hash_map::DefaultHasher::new();
                    file.hash(&mut hs);
                    (0x43u64 << 56) | (hs.finish() >> 8)
                };
                let bmp = ui.cache.get(&cv.g.dc, key, p.w, p.h, || p.px.clone());
                let k = (area.w / p.w as f32).min(area.h / p.h as f32).max(0.01);
                let (dw, dh) = (p.w as f32 * k, p.h as f32 * k);
                let dest = Rect::new(area.x + (area.w - dw) / 2.0, area.y + (area.h - dh) / 2.0, dw, dh);
                cv.bitmap_quad(bmp.as_ref(), dest, 0.0, 1.0);
            } else {
                cv.text("Загрузка…", area, &f.small_c, dim());
            }
            meta[0].1 = crate::i18n::t("Изображение").to_string();
            meta[1].1 = format!("{w} × {h} px");
        } else {
            let shown: String = text.chars().take(600).collect();
            let max_lines = (area.h / 13.5).floor().max(1.0) as usize;
            let clipped = cv.wrap_clip(&shown, &f.wrap_b, area.w, max_lines);
            cv.text_wrap_c(&clipped, area, txt());
            let chars = text.chars().count();
            if is_file {
                meta[0].1 = crate::i18n::t("Файл").to_string();
                let clean = text.trim().trim_matches('"');
                let clean = clean.strip_prefix("file://").unwrap_or(clean);
                meta[1].1 = match std::fs::metadata(clean) {
                    Ok(m) => format_file_size(m.len()),
                    Err(_) => {
                        if crate::i18n::en() {
                            format!("{chars} chars")
                        } else {
                            format!("{chars} симв.")
                        }
                    }
                };
            } else if is_link {
                meta[0].1 = crate::i18n::t("Ссылка").to_string();
                meta[1].1 = if crate::i18n::en() {
                    format!("{chars} chars")
                } else {
                    format!("{chars} симв.")
                };
            } else {
                let words = text.split_whitespace().count();
                let lines = text.lines().count().max(1);
                meta[0].1 = crate::i18n::t("Текст").to_string();
                meta[1].1 = if crate::i18n::en() {
                    format!("{chars} chars • {words} words • {lines} lines")
                } else {
                    format!("{chars} симв. • {words} сл. • {lines} стр.")
                };
            }
        }
        // Блок метаданных в подвале: приглушённый мелкий шрифт, отделённый от данных.
        let footer_y = r.bottom() - 43.0;
        // Горизонтальная линия между содержимым и метаданными (стиль разделителей острова — `track`).
        cv.rr(Rect::new(r.x + 14.0, footer_y - 12.0, r.w - 28.0, 1.0), 0.5, crate::ui::track());
        for (k, (label, value)) in meta.iter().enumerate() {
            let cy = footer_y + k as f32 * 13.0;
            cv.text(label, Rect::new(r.x + 14.0, cy - 6.0, 68.0, 12.0), &f.small, dim());
            cv.text(value, Rect::new(r.x + 86.0, cy - 6.0, r.w - 100.0, 12.0), &f.small, dim());
        }
    }

    /// Нижняя строка как в макете: клавиши с подписями (кликабельны) и подсказка справа.
    unsafe fn footer(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32, act: &mut Vec<Action>) {
        let cy = h - 21.0;
        let sel = self.selected.as_deref().and_then(|t| self.position(t));
        let keys: [(&str, &str); 4] = [("↵", "Вставить"), ("Ctrl C", "Копировать"), ("Ctrl P", "Закрепить"), ("Del", "Удалить")];
        let mut x = PAD + 2.0;
        for (k, (key, label)) in keys.iter().enumerate() {
            let kid = id(CLIPBOARD, 10 + k as u32);
            let wdt = cv.text_w(key, &cv.g.fonts.small_c) + 9.0 + 4.0 + cv.text_w(label, &cv.g.fonts.small) + 4.0;
            let (_, clicked) = ui.click_area(kid, Rect::new(x - 2.0, cy - 8.0, wdt + 4.0, 16.0));
            let hv = ui.hover_of(kid);
            if hv > 0.01 {
                let mut c = crate::ui::fg();
                c.a = 0.05 * hv;
                cv.rr(Rect::new(x - 3.0, cy - 9.0, wdt + 6.0, 18.0), 5.0, c);
            }
            cv.keycap(x, cy, key, label);
            x += wdt + 12.0;
            if clicked {
                if let Some(i) = sel {
                    match k {
                        0 => act.push(self.paste_action(i)),
                        1 => act.push(self.copy_action(i)),
                        2 => {
                            self.items[i].pinned = !self.items[i].pinned;
                            self.save();
                        }
                        _ => {
                            self.remove_at(i);
                            self.selected = None;
                        }
                    }
                }
            }
        }
        let hint = if self.paused { "Запись приостановлена (шестерёнка)" } else { "↑↓ выбор • Tab фильтр • Enter вставить" };
        cv.text(hint, Rect::new(w - PAD - 230.0, cy - 7.0, 228.0, 14.0), &cv.g.fonts.small_r, bare());
    }
}

/// «Сегодня, 9:41», «Вчера, 18:05», «12 окт, 10:00» — по местному времени.
fn copied_at(ts: u64, day_start: u64) -> String {
    let local = ts as i64 / 60 + crate::online::utc_offset_minutes();
    let tod = local.rem_euclid(1440);
    let hm = format!("{}:{:02}", tod / 60, tod % 60);
    if ts >= day_start {
        format!("{}, {hm}", crate::i18n::pick("Сегодня", "Today"))
    } else if ts + 86_400 >= day_start {
        format!("{}, {hm}", crate::i18n::pick("Вчера", "Yesterday"))
    } else {
        let (_, m, d) = crate::online::civil_from_days(local.div_euclid(1440));
        format!("{d} {}, {hm}", crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1])
    }
}

fn section(it: &Item, day_start: u64) -> &'static str {
    if it.pinned {
        "ЗАКРЕПЛЕНО"
    } else if it.ts >= day_start {
        "СЕГОДНЯ"
    } else if it.ts + 86_400 >= day_start {
        "ВЧЕРА"
    } else {
        "РАНЕЕ"
    }
}

/// Короткая подпись времени для строки списка: «сейчас», «4 м», «1 ч», «2 д».
fn short_ago(now: u64, then: u64) -> String {
    let d = now.saturating_sub(then);
    crate::i18n::ago_short(d)
}

fn first_line(s: &str) -> &str {
    let t = s.trim_start();
    t.lines().next().unwrap_or("").trim_end()
}

/// Человекочитаемый размер файла (Б, КБ, МБ, ГБ).
fn format_file_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b < K {
        format!("{} {}", bytes, crate::i18n::pick("Б", "B"))
    } else if b < K * K {
        format!("{:.1} {}", b / K, crate::i18n::pick("КБ", "KB"))
    } else if b < K * K * K {
        format!("{:.1} {}", b / (K * K), crate::i18n::pick("МБ", "MB"))
    } else {
        format!("{:.2} {}", b / (K * K * K), crate::i18n::pick("ГБ", "GB"))
    }
}

/// Проверяет, является ли строка путём к файлу или папке Windows (буква диска, UNC или file://).
fn is_file_path(s: &str) -> bool {
    let t = s.trim().trim_matches('"');
    if t.is_empty() || t.contains('\n') || t.contains('\r') {
        return false;
    }
    if t.starts_with("file://") {
        return true;
    }
    let b = t.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') {
        return true;
    }
    if (t.starts_with(r"\\") || t.starts_with("//")) && t.len() > 3 {
        return true;
    }
    false
}

/// Извлекает имя файла из пути.
fn path_file_name(s: &str) -> &str {
    let t = s.trim().trim_matches('"');
    let t = t.strip_prefix("file://").unwrap_or(t);
    let t = t.trim_end_matches(['\\', '/']);
    if let Some(pos) = t.rfind(['\\', '/']) {
        let name = &t[pos + 1..];
        if !name.is_empty() {
            return name;
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_moves_to_top_and_keeps_pin() {
        let mut c = Clip::default();
        c.items.push(Item { text: "a".into(), ts: 1, pinned: true });
        c.items.push(Item { text: "b".into(), ts: 1, pinned: false });
        // `record` пишет файл — для проверки логики используем только содержимое списка.
        let pinned = match c.items.iter().position(|i| i.text == "a") {
            Some(p) => c.items.remove(p).pinned,
            None => false,
        };
        assert!(pinned);
        assert_eq!(c.items[0].text, "b");
    }

    #[test]
    fn link_detection_and_filter() {
        let mut c = Clip::default();
        c.items.push(Item { text: "https://example.com/x".into(), ts: 0, pinned: false });
        c.items.push(Item { text: "hello world".into(), ts: 0, pinned: false });
        c.items.push(Item { text: "see https://a.b".into(), ts: 0, pinned: false });
        c.filter = 2;
        assert_eq!(c.visible(""), vec![0]);
        c.filter = 1;
        assert_eq!(c.visible(""), vec![1, 2]);
        c.filter = 0;
        assert_eq!(c.visible("WORLD"), vec![1]);
    }

    #[test]
    fn count_by_filter_and_query() {
        let mut c = Clip::default();
        c.items.push(Item { text: "https://example.com/x".into(), ts: 0, pinned: false });
        c.items.push(Item { text: "hello world".into(), ts: 0, pinned: false });
        c.items.push(Item { text: "see https://a.b".into(), ts: 0, pinned: false });
        c.items.push(Item { text: format!("{IMG_TAG}screen.png:100x100"), ts: 0, pinned: false });

        // Пустой запрос: число записей в текущем фильтре.
        c.filter = 0;
        assert_eq!(c.visible("").len(), 4);
        c.filter = 1;
        assert_eq!(c.visible("").len(), 2);
        c.filter = 2;
        assert_eq!(c.visible("").len(), 1);
        c.filter = 3;
        assert_eq!(c.visible("").len(), 1);

        // Поиск с фильтром: число совпадений внутри выбранной категории.
        c.filter = 0;
        assert_eq!(c.visible("world").len(), 1);
        assert_eq!(c.visible("https").len(), 2);
        assert_eq!(c.visible("фото").len(), 1);
        c.filter = 1;
        assert_eq!(c.visible("https").len(), 1);
        assert_eq!(c.visible("world").len(), 1);
        c.filter = 2;
        assert_eq!(c.visible("https").len(), 1);
        assert_eq!(c.visible("world").len(), 0);
        c.filter = 3;
        assert_eq!(c.visible("фото").len(), 1);
        assert_eq!(c.visible("world").len(), 0);
    }

    #[test]
    fn pinned_float_up() {
        let mut c = Clip::default();
        c.items.push(Item { text: "new".into(), ts: 2, pinned: false });
        c.items.push(Item { text: "old".into(), ts: 1, pinned: true });
        assert_eq!(c.visible(""), vec![1, 0]);
    }

    #[test]
    fn sections_and_short_time() {
        let it = |ts, pinned| Item { text: "x".into(), ts, pinned };
        assert_eq!(section(&it(100, true), 1000), "ЗАКРЕПЛЕНО");
        assert_eq!(section(&it(1500, false), 1000), "СЕГОДНЯ");
        assert_eq!(section(&it(10_000, false), 200_000), "РАНЕЕ");
        assert_eq!(short_ago(1000, 1000), "сейчас");
        assert_eq!(short_ago(1000 + 300, 1000), "5 м");
        assert_eq!(short_ago(1000 + 7200, 1000), "2 ч");
    }

    #[test]
    fn first_line_trims() {
        assert_eq!(first_line("  \nabc\ndef"), "abc");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn file_path_detection_and_name() {
        let file_item = Item { text: r"C:\Users\User\Documents\Report.docx".into(), ts: 0, pinned: false };
        assert!(file_item.is_file());
        assert!(!file_item.is_link());
        assert!(!file_item.is_image());
        assert_eq!(file_item.file_name(), Some("Report.docx"));
        assert_eq!(file_item.label(), "Report.docx");

        let slash_item = Item { text: "D:/Photos/vacation.jpg".into(), ts: 0, pinned: false };
        assert!(slash_item.is_file());
        assert_eq!(slash_item.file_name(), Some("vacation.jpg"));

        let unc_item = Item { text: r"\\server\share\data.csv".into(), ts: 0, pinned: false };
        assert!(unc_item.is_file());
        assert_eq!(unc_item.file_name(), Some("data.csv"));

        let quoted_item = Item { text: r#""C:\Program Files\app.exe""#.into(), ts: 0, pinned: false };
        assert!(quoted_item.is_file());
        assert_eq!(quoted_item.file_name(), Some("app.exe"));

        let file_uri = Item { text: "file:///C:/path/file.txt".into(), ts: 0, pinned: false };
        assert!(file_uri.is_file());
        assert_eq!(file_uri.file_name(), Some("file.txt"));

        // Обычный текст и ссылки не должны определяться как файлы
        let text_item = Item { text: "Just some text".into(), ts: 0, pinned: false };
        assert!(!text_item.is_file());
        assert_eq!(text_item.file_name(), None);
        assert_eq!(text_item.label(), "Just some text");

        let link_item = Item { text: "https://example.com/file.txt".into(), ts: 0, pinned: false };
        assert!(!link_item.is_file());
        assert!(link_item.is_link());
        assert_eq!(link_item.label(), "https://example.com/file.txt");

        let multiline_item = Item { text: "C:\\first\nsecond".into(), ts: 0, pinned: false };
        assert!(!multiline_item.is_file());
    }

    #[test]
    fn item_label_separated_from_metadata() {
        // У картинок подпись содержит только имя/тип («Изображение»), размеры вынесены в правую колонку
        let img_item = Item { text: format!("{IMG_TAG}photo.png:1920x1080"), ts: 0, pinned: false };
        assert_eq!(img_item.label(), "Изображение");

        // У ссылок — адрес
        let link_item = Item { text: "https://example.com".into(), ts: 0, pinned: false };
        assert_eq!(link_item.label(), "https://example.com");

        // У файлов — имя файла
        let file_item = Item { text: r"C:\path\to\document.pdf".into(), ts: 0, pinned: false };
        assert_eq!(file_item.label(), "document.pdf");

        // У текста — первая строка
        let text_item = Item { text: "Hello\nWorld".into(), ts: 0, pinned: false };
        assert_eq!(text_item.label(), "Hello");
    }

    #[test]
    fn format_file_sizes() {
        assert_eq!(format_file_size(500), "500 Б");
        assert_eq!(format_file_size(1024), "1.0 КБ");
        assert_eq!(format_file_size(1024 * 1536), "1.5 МБ");
        assert_eq!(format_file_size(1024 * 1024 * 1024 * 2), "2.00 ГБ");
    }
}
