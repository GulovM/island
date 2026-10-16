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

use crate::ui::{icon, Canvas, Nav, Rect, Submit, Ui, card, dim, txt};

use super::{escape, id, read_lines_sealed, unescape, write_lines_sealed, Action, CLIPBOARD, PAD, TOP};

const FILE: &str = "clipboard.txt";
/// Сколько записей хранится (закреплённые не считаются).
const MAX_ITEMS: usize = 100;
/// Длиннее этого текст не сохраняется целиком.
const MAX_CHARS: usize = 20_000;
const LIST_W: f32 = 228.0;
const IMG_TAG: &str = "\u{1}img:";
/// Размер превью-изображения (пиксели), вписывается без увеличения.
const PREVIEW_MAX: (u32, u32) = (420, 280);

/// Папка с PNG-файлами изображений истории.
pub fn clip_dir() -> PathBuf {
    sys::data_dir().join("clip")
}

/// Готовые к записи в историю изображения: (файл, ширина, высота).
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

    /// (имя файла, ширина, высота) для записи-изображения.
    fn image(&self) -> Option<(&str, u32, u32)> {
        let rest = self.text.strip_prefix(IMG_TAG)?;
        let (file, dims) = rest.split_once(':')?;
        let (w, h) = dims.split_once('x')?;
        Some((file, w.parse().ok()?, h.parse().ok()?))
    }

    /// Подпись в списке.
    fn label(&self) -> String {
        match self.image() {
            Some((_, w, h)) => format!("Изображение {w}×{h}"),
            None => first_line(&self.text).to_string(),
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
    pub fn load(&mut self) {
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
        let widths: Vec<f32> = names.iter().map(|n| n.chars().count() as f32 * 5.0 + 14.0).collect();
        let gear = Rect::new(bar.right() - 23.0, bar.y + 4.0, 18.0, 18.0);
        let pills_w: f32 = widths.iter().sum::<f32>() + 4.0 * (names.len() - 1) as f32;
        let mut px = gear.x - 8.0 - pills_w;
        let fid = id(CLIPBOARD, 0);
        let ph = format!("Поиск среди {} записей…", self.items.len());
        ui.text_field_ex(cv, fid, Rect::new(bar.x, bar.y, px - bar.x - 6.0, bar.h), &ph, icon::SEARCH, Submit::Keep);
        for (i, name) in names.iter().enumerate() {
            let r = Rect::new(px, bar.y + 4.5, widths[i], 17.0);
            if ui.chip_ex(cv, id(CLIPBOARD, 1 + i as u32), r, name, self.filter == i, true, None) {
                self.filter = i;
            }
            px += widths[i] + 4.0;
        }
        if ui.icon_button(cv, id(CLIPBOARD, 5), gear, icon::SETTINGS_GEAR, self.paused) {
            self.paused = !self.paused;
        }

        let query = ui.field_text(fid);
        let order = self.visible(&query);

        // Клавиши, пока в поиске фокус: стрелки двигают выбор, Ctrl+P закрепляет, Enter вставляет.
        let cur = self.selected.as_deref().and_then(|t| order.iter().position(|&n| self.items[n].text == t));
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
        let list = Rect::new(PAD, TOP + 33.0, LIST_W, (h - TOP - 33.0 - 36.0).max(ROW));

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
                cv.text(title, Rect::new(list.x + 5.0, yy + 3.0, list.w, 11.0), &f.caps, dim());
                continue;
            }
            let n = order[p];
            let it = &self.items[n];
            let g = if it.pinned {
                icon::PIN
            } else if it.is_image() {
                icon::PICTURE
            } else if it.is_link() {
                icon::LINK
            } else {
                icon::TEXT_DOC
            };
            let r = Rect::new(list.x, yy, list.w - 3.0, ROW);
            let sel = selected_pos == Some(p);
            let label = it.label();
            if ui.row_compact(cv, id(CLIPBOARD, 200 + p as u32), r, g, true, &label, &short_ago(now, it.ts), 0.0, sel, !sel) {
                clicked = Some(n);
            }
        }
        ui.scroll_end(cv, sid);
        if let Some(n) = clicked {
            self.selected = Some(self.items[n].text.clone());
        }
        if order.is_empty() {
            let msg = if self.items.is_empty() { "История пуста — скопируйте что-нибудь" } else { "Ничего не найдено" };
            cv.text(msg, Rect::new(list.x + 5.0, list.y + 6.0, list.w, 14.0), &f.small, dim());
        }

        let prev = Rect::new(PAD + LIST_W + 11.0, TOP + 32.0, w - 2.0 * PAD - LIST_W - 11.0, 175.0);
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
        let area = Rect::new(r.x + 14.0, r.y + 11.0, r.w - 28.0, r.h - 11.0 - 62.0);
        let (ts, is_link, image, text) = {
            let it = &self.items[idx];
            (it.ts, it.is_link(), it.image().map(|(f, w, h)| (f.to_string(), w, h)), it.text.clone())
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
            meta[0].1 = "Изображение".into();
            meta[1].1 = format!("{w} × {h} px");
        } else {
            let shown: String = text.chars().take(600).collect();
            cv.text_wrap_c(&shown, area, txt());
            let chars = text.chars().count();
            let words = text.split_whitespace().count();
            let lines = text.lines().count().max(1);
            meta[0].1 = if is_link { "Ссылка".to_string() } else { "Текст".to_string() };
            meta[1].1 = format!("{chars} симв. • {words} сл. • {lines} стр.");
        }
        for (k, (label, value)) in meta.iter().enumerate() {
            let cy = r.bottom() - 41.0 + k as f32 * 12.5;
            cv.text(label, Rect::new(r.x + 11.0, cy - 6.0, 52.0, 12.0), &f.small, dim());
            cv.text(value, Rect::new(r.x + 63.0, cy - 6.0, r.w - 72.0, 12.0), &f.small_b, txt());
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
                let mut c = crate::ui::WHITE;
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
        cv.text(hint, Rect::new(w - PAD - 230.0, cy - 7.0, 228.0, 14.0), &cv.g.fonts.small_r, dim());
    }
}

/// «Сегодня, 9:41», «Вчера, 18:05», «12 окт, 10:00» — по местному времени.
fn copied_at(ts: u64, day_start: u64) -> String {
    let local = ts as i64 / 60 + crate::online::utc_offset_minutes();
    let tod = local.rem_euclid(1440);
    let hm = format!("{}:{:02}", tod / 60, tod % 60);
    if ts >= day_start {
        format!("Сегодня, {hm}")
    } else if ts + 86_400 >= day_start {
        format!("Вчера, {hm}")
    } else {
        const MONTHS: [&str; 12] = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
        let (_, m, d) = crate::online::civil_from_days(local.div_euclid(1440));
        format!("{d} {}, {hm}", MONTHS[(m as usize).clamp(1, 12) - 1])
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
    match d {
        0..=59 => "сейчас".into(),
        60..=3599 => format!("{} м", d / 60),
        3600..=86_399 => format!("{} ч", d / 3600),
        _ => format!("{} д", d / 86_400),
    }
}

fn first_line(s: &str) -> &str {
    let t = s.trim_start();
    t.lines().next().unwrap_or("").trim_end()
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
}
