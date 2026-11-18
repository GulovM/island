//! Вкладка «Буфер» из двух разделов.
//!
//! «Быстрый доступ» (`quick.rs`) — постоянное хранилище закреплённых файлов, ссылок, текстов и
//! картинок: открыть, вставить, перетащить наружу; до 30 записей, сами не удаляются.
//!
//! «Буфер» — история недавно скопированного: поиск, фильтры, превью, вставка выбранного в окно,
//! где работали. Запись идёт всегда (слушатель в main.rs), данные менеджеров паролей и собственные
//! записи острова отсекаются до `record`. Запись живёт сутки и удаляется сама; закрепление
//! переносит её в «Быстрый доступ». Изображения (скриншоты, копирование картинок) сохраняются как
//! PNG в `%APPDATA%\island\clip`; в списке запись вида `\u{1}img:<файл>:<ширина>x<высота>`.

mod quick;

pub use quick::{combo as hotkey_combo, Added, MAX_ITEMS as QUICK_MAX};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::dnd::Out;
use crate::sys;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};

use crate::ui::{chips_total_width, icon, Canvas, Nav, Rect, Submit, Ui, bare, card, dim, status_orange, txt};

use super::{escape, id, read_lines_sealed, unescape, write_lines_sealed, Action, CLIPBOARD, PAD, TOP};

const FILE: &str = "clipboard.txt";
/// Сколько записей хранит история.
const MAX_ITEMS: usize = 100;
/// Сколько живёт запись истории: старше — удаляется сама.
const RETENTION_SECS: u64 = 24 * 3600;
/// Длиннее этого текст не сохраняется целиком.
const MAX_CHARS: usize = 20_000;
const LIST_W: f32 = 250.0;
const IMG_TAG: &str = "\u{1}img:";
/// Размер превью-изображения (пиксели), вписывается без увеличения.
const PREVIEW_MAX: (u32, u32) = (420, 280);
/// Строка разделов над поиском.
const SECTION_H: f32 = 28.0;
/// На сколько сдвинуть мышь с нажатой кнопкой на строке, чтобы это стало перетаскиванием.
const DRAG_START: f32 = 6.0;
const FILTERS: [&str; 5] = ["Все", "Текст", "Ссылки", "Фото", "Файлы"];

/// Элемент интерфейса, который принимает нажатие сочетания (запись горячей клавиши раздела).
const HOTKEY_ID: u32 = 22;

/// Папка с PNG-файлами изображений истории.
pub fn clip_dir() -> PathBuf {
    sys::data_dir().join("clip")
}

/// Готовые к записи в историю изображения: (файл, ширина, высота).
/// Подпись и время последнего снимка (для отсева повторных уведомлений).
static LAST_IMG: Mutex<(u64, u64)> = Mutex::new((0, 0));
static DONE: Mutex<Vec<(String, u32, u32)>> = Mutex::new(Vec::new());
/// Декодированное превью: (путь к файлу, ширина, высота, пиксели BGRA premultiplied).
#[allow(clippy::type_complexity)]
static PREVIEW: Mutex<Option<(String, u32, u32, Vec<u8>)>> = Mutex::new(None);
static IMG_SEQ: AtomicU32 = AtomicU32::new(0);

struct PreviewImg {
    /// Путь к PNG: ключ превью.
    key: String,
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// Раздел вкладки.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Buffer,
    Quick,
}

impl Section {
    pub fn from_index(i: usize) -> Self {
        if i == 1 {
            Section::Quick
        } else {
            Section::Buffer
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// Папка с PNG записей этого раздела.
    fn dir(self) -> PathBuf {
        match self {
            Section::Buffer => clip_dir(),
            Section::Quick => quick::dir(),
        }
    }
}

#[derive(Clone)]
struct Item {
    text: String,
    ts: u64,
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

    /// Путь файла без кавычек и `file://`.
    fn path(&self) -> Option<&str> {
        self.is_file().then(|| clean_path(&self.text))
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

    fn remove_file(&self, dir: &std::path::Path) {
        if let Some((file, _, _)) = self.image() {
            let _ = std::fs::remove_file(dir.join(file));
        }
    }
}

/// Список раздела: записи (новые сверху), выбранная по тексту (индексы меняются) и фильтр.
#[derive(Default)]
struct List {
    items: Vec<Item>,
    selected: Option<String>,
    /// Дополнительные выбранные записи при Ctrl+клике / Shift+клике.
    multi: HashSet<String>,
    /// Опорная запись для диапазонного выделения (Shift+клик).
    anchor: Option<String>,
    /// 0 — все, 1 — текст, 2 — ссылки, 3 — фото, 4 — файлы.
    filter: usize,
}

impl List {
    /// Индексы записей по текущему фильтру и поиску, порядок сохраняется.
    fn visible(&self, query: &str) -> Vec<usize> {
        let q = query.to_lowercase();
        let ok = |i: &Item| {
            let kind = match self.filter {
                1 => !i.is_link() && !i.is_image() && !i.is_file(),
                2 => i.is_link(),
                3 => i.is_image(),
                4 => i.is_file(),
                _ => true,
            };
            kind && (q.is_empty() || i.haystack().contains(&q))
        };
        (0..self.items.len()).filter(|&n| ok(&self.items[n])).collect()
    }

    fn position(&self, text: &str) -> Option<usize> {
        self.items.iter().position(|i| i.text == text)
    }

    fn is_selected(&self, text: &str) -> bool {
        self.selected.as_deref() == Some(text) || self.multi.contains(text)
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.multi.clear();
        self.anchor = None;
    }

    fn select_only(&mut self, text: String) {
        self.multi.clear();
        self.anchor = Some(text.clone());
        self.selected = Some(text);
    }

    fn toggle_select(&mut self, text: String) {
        if self.is_selected(&text) {
            self.multi.remove(&text);
            if self.selected.as_deref() == Some(text.as_str()) {
                self.selected = self.multi.iter().next().cloned();
                if let Some(new_sel) = &self.selected {
                    self.multi.remove(new_sel);
                }
            }
        } else {
            if let Some(prev) = self.selected.take() {
                self.multi.insert(prev);
            }
            self.anchor = Some(text.clone());
            self.selected = Some(text);
        }
    }

    fn select_range(&mut self, order: &[usize], target_vis_pos: usize, keep_existing: bool) {
        let Some(&target_idx) = order.get(target_vis_pos) else { return };
        let anchor_pos = self
            .anchor
            .as_deref()
            .or(self.selected.as_deref())
            .and_then(|a| order.iter().position(|&idx| self.items[idx].text == a));
        if let Some(a_pos) = anchor_pos {
            if !keep_existing {
                self.multi.clear();
            } else if let Some(prev) = self.selected.take() {
                self.multi.insert(prev);
            }
            let (lo, hi) = (a_pos.min(target_vis_pos), a_pos.max(target_vis_pos));
            for &idx in &order[lo..=hi] {
                self.multi.insert(self.items[idx].text.clone());
            }
            let target_text = self.items[target_idx].text.clone();
            self.multi.remove(&target_text);
            self.selected = Some(target_text);
        } else {
            self.select_only(self.items[target_idx].text.clone());
        }
    }

    fn selected_indices(&self, order: &[usize]) -> Vec<usize> {
        let out: Vec<usize> = order.iter().copied().filter(|&n| self.is_selected(&self.items[n].text)).collect();
        if out.is_empty() {
            self.selected.as_deref().and_then(|t| self.position(t)).into_iter().collect()
        } else {
            out
        }
    }

    fn prune_selection(&mut self) {
        self.multi.retain(|t| self.items.iter().any(|i| &i.text == t));
        if let Some(sel) = &self.selected {
            if !self.items.iter().any(|i| &i.text == sel) {
                self.selected = self.multi.iter().next().cloned();
                if let Some(new_sel) = &self.selected {
                    self.multi.remove(new_sel);
                }
            }
        }
        if let Some(anc) = &self.anchor {
            if !self.items.iter().any(|i| &i.text == anc) {
                self.anchor = self.selected.clone();
            }
        }
    }
}

#[derive(Default)]
pub struct Clip {
    /// История буфера обмена.
    buf: List,
    /// Быстрый доступ.
    quick: List,
    section: Section,
    /// Запись истории приостановлена пользователем.
    paused: bool,
    preview: Option<PreviewImg>,
    preview_req: Option<String>,
    /// Что известно о файлах на диске: `None` — файла нет на месте.
    files: HashMap<String, Option<quick::Info>>,
    /// Пути, проверка которых уже запрошена.
    requested: HashSet<String>,
    /// Строка, на которой нажата кнопка, и место нажатия: отсюда может начаться перетаскивание наружу.
    drag_from: Option<(u32, (f32, f32))>,
    /// Перетаскивание уже началось: пока кнопка не отпущена, второе не стартует.
    drag_fired: bool,
    /// Горячая клавиша раздела («ctrl+alt+q») и ожидание нового сочетания.
    hotkey: String,
    recording: bool,
    /// Пояснение вместо обычной подсказки в нижней строке (лимит, отказ).
    note: Option<&'static str>,
}

/// Запись истории старше срока хранения.
fn expired(it: &Item, now: u64) -> bool {
    now.saturating_sub(it.ts) >= RETENTION_SECS
}

fn save_buffer(items: &[Item]) {
    // Закрепления в файле больше нет: колонка осталась ради совместимости формата.
    let lines: Vec<String> = items.iter().map(|i| format!("0|{}|{}", i.ts, escape(&i.text))).collect();
    write_lines_sealed(FILE, &lines);
}

impl Clip {
    pub fn load(&mut self, paused: bool, section: usize) {
        self.paused = paused;
        self.section = Section::from_index(section);
        let mut pinned = Vec::new();
        for l in read_lines_sealed(FILE) {
            let mut p = l.splitn(3, '|');
            let (Some(pin), Some(ts), Some(text)) = (p.next(), p.next(), p.next()) else { continue };
            let Ok(ts) = ts.parse() else { continue };
            let it = Item { text: unescape(text), ts };
            if pin == "1" {
                pinned.push(it);
            } else {
                self.buf.items.push(it);
            }
        }
        self.buf.items.truncate(MAX_ITEMS * 2);
        self.quick.items = quick::load();
        // Закреплённое прежних версий переезжает в «Быстрый доступ»; что не влезло или не скопировалось
        // (картинка), остаётся обычной записью истории.
        let mut moved = false;
        for it in pinned {
            let copied = it.image().is_none_or(|(file, _, _)| quick::copy_image(file));
            if copied && self.quick.items.len() < quick::MAX_ITEMS && !self.quick.items.iter().any(|q| q.text == it.text) {
                self.quick.items.push(it);
                moved = true;
            } else {
                self.buf.items.push(it);
            }
        }
        if moved {
            quick::save(&self.quick.items);
        }
        let pruned = self.prune_buffer(sys::now_secs());
        // Файл старой версии (открытый текст) сразу переписываем зашифрованным.
        let legacy = std::fs::read(sys::data_dir().join(FILE)).is_ok_and(|b| !b.is_empty() && !sys::is_sealed(&b));
        if (legacy || moved || pruned) && !self.buf.items.is_empty() {
            save_buffer(&self.buf.items);
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

    /// Убирает из истории записи старше суток (вместе с файлами картинок). `true` — что-то убрано.
    fn prune_buffer(&mut self, now: u64) -> bool {
        let before = self.buf.items.len();
        let dir = clip_dir();
        self.buf.items.retain(|i| {
            let keep = !expired(i, now);
            if !keep {
                i.remove_file(&dir);
            }
            keep
        });
        self.buf.items.len() != before
    }

    /// Срок хранения истории: вызывается при показе вкладки и раз в час. `true` — список изменился.
    pub fn prune(&mut self) -> bool {
        let changed = self.prune_buffer(sys::now_secs());
        if changed {
            save_buffer(&self.buf.items);
        }
        changed
    }

    /// Вкладка стала видимой. `true` — нужен кадр.
    pub fn show(&mut self) -> bool {
        let changed = self.prune();
        if self.section == Section::Quick {
            self.recheck_quick();
        }
        changed
    }

    /// Вкладка скрыта: незавершённая запись сочетания и начатое перетаскивание отменяются.
    pub fn hide(&mut self) {
        self.recording = false;
        self.drag_from = None;
        self.drag_fired = false;
        self.note = None;
    }

    pub fn set_section(&mut self, s: Section) {
        if s == self.section {
            return;
        }
        self.section = s;
        self.recording = false;
        self.note = None;
        crate::state::set_clipboard_section(s.index());
        if s == Section::Quick {
            self.recheck_quick();
        }
    }

    /// Новый текст из буфера обмена. `true` — история изменилась.
    pub fn record(&mut self, text: String) -> bool {
        if self.paused || text.trim().is_empty() {
            return false;
        }
        let text: String = text.chars().take(MAX_CHARS).collect();
        if let Some(p) = self.buf.position(&text) {
            self.buf.items.remove(p);
        }
        let now = sys::now_secs();
        self.buf.items.insert(0, Item { text, ts: now });
        // Обрезаем хвост и всё, что старше суток.
        let dir = clip_dir();
        while self.buf.items.len() > MAX_ITEMS {
            if let Some(old) = self.buf.items.pop() {
                old.remove_file(&dir);
            }
        }
        self.prune_buffer(now);
        save_buffer(&self.buf.items);
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
        if let Some((key, w, h, px)) = PREVIEW.lock().ok().and_then(|mut p| p.take()) {
            if self.preview_req.as_deref() == Some(key.as_str()) {
                self.preview = Some(PreviewImg { key, w, h, px });
                changed = true;
            }
        }
        for (path, info) in quick::take_checked() {
            self.files.insert(path, info);
            changed = true;
        }
        changed
    }

    /// Запрашивает декодирование превью изображения (если его ещё нет и не запрошено).
    fn want_preview(&mut self, path: PathBuf) {
        let key = path.to_string_lossy().into_owned();
        if self.preview.as_ref().is_some_and(|p| p.key == key) || self.preview_req.as_deref() == Some(key.as_str()) {
            return;
        }
        self.preview_req = Some(key.clone());
        let _ = std::thread::Builder::new().name("island-clip-prev".into()).spawn(move || unsafe {
            if let Some((w, h, px)) = crate::img::load_fit(&path, PREVIEW_MAX.0, PREVIEW_MAX.1) {
                if let Ok(mut p) = PREVIEW.lock() {
                    *p = Some((key, w, h, px));
                }
                crate::bg::notify();
            }
        });
    }

    /// Просит проверить файлы в фоне (по разу на путь).
    fn want_files(&mut self, paths: Vec<String>) {
        let fresh: Vec<String> = paths.into_iter().filter(|p| self.requested.insert(p.clone())).collect();
        quick::spawn_check(fresh);
    }

    /// Заново проверяет, на месте ли файлы быстрого доступа.
    fn recheck_quick(&mut self) {
        let paths: Vec<String> = self.quick.items.iter().filter_map(|i| i.path().map(str::to_string)).collect();
        for p in &paths {
            self.requested.remove(p);
        }
        self.want_files(paths);
    }

    /// Файла нет на месте (проверка успела это выяснить).
    fn missing(&self, it: &Item) -> bool {
        it.path().is_some_and(|p| matches!(self.files.get(p), Some(None)))
    }

    /// Файл не открылся: помечаем недоступным, не дожидаясь следующей проверки.
    pub fn mark_missing(&mut self, path: &str) {
        self.files.insert(path.to_string(), None);
    }

    // ---------- Быстрый доступ ----------

    /// Добавляет брошенное в «Быстрый доступ» (файлы, адреса, текст).
    pub fn add_quick(&mut self, entries: &[String]) -> Added {
        let rep = quick::add_entries(&mut self.quick.items, entries, sys::now_secs());
        if rep.new + rep.existing > 0 {
            quick::save(&self.quick.items);
            if let Some(first) = self.quick.items.first() {
                self.quick.select_only(first.text.clone());
            }
            self.recheck_quick();
        }
        rep
    }

    pub fn quick_full(&self) -> bool {
        self.quick.items.len() >= quick::MAX_ITEMS
    }

    /// Переносит запись истории в «Быстрый доступ». `false` — не вышло (причина в `note`).
    #[cfg(test)]
    fn pin(&mut self, l: &mut List, n: usize) -> bool {
        self.pin_many(l, &[n])
    }

    /// Переносит выбранные записи истории в «Быстрый доступ». `false` — не вышло (причина в `note`).
    fn pin_many(&mut self, l: &mut List, indices: &[usize]) -> bool {
        let mut unique: Vec<usize> = Vec::new();
        for &n in indices {
            if n < l.items.len() && !unique.contains(&n) {
                unique.push(n);
            }
        }
        if unique.is_empty() {
            return false;
        }
        let now = sys::now_secs();
        let mut pinned: Vec<usize> = Vec::new();
        let mut extra = 0usize;
        let mut failed_note = None;
        for &n in &unique {
            let it = &l.items[n];
            let in_batch = pinned.iter().any(|&p| l.items[p].text == it.text);
            let in_quick = self.quick.items.iter().any(|q| q.text == it.text);
            let fits = if in_batch {
                true
            } else if extra == 0 {
                quick::can_pin(&self.quick.items, it)
            } else {
                in_quick || self.quick.items.len() + extra < quick::MAX_ITEMS
            };
            if !fits {
                failed_note = Some("Быстрый доступ заполнен — освободите место");
                break;
            }
            if let Some((file, _, _)) = it.image() {
                if !quick::copy_image(file) {
                    failed_note = Some("Не удалось сохранить картинку");
                    break;
                }
            }
            if !in_batch && !in_quick {
                extra += 1;
            }
            pinned.push(n);
        }
        if pinned.is_empty() {
            self.note = failed_note;
            return false;
        }
        // Вставляем в обратном порядке, чтобы верхние элементы списка остались наверху быстрого доступа.
        for &n in pinned.iter().rev() {
            quick::insert_front(&mut self.quick.items, l.items[n].clone(), now);
        }
        pinned.sort_unstable_by(|a, b| b.cmp(a));
        let dir = clip_dir();
        for n in pinned {
            let it = l.items.remove(n);
            it.remove_file(&dir);
        }
        quick::save(&self.quick.items);
        save_buffer(&l.items);
        l.prune_selection();
        self.recheck_quick();
        self.note = failed_note.or(Some("Закреплено в «Быстром доступе»"));
        failed_note.is_none()
    }

    fn remove_at(&mut self, sec: Section, l: &mut List, n: usize) {
        self.remove_many(sec, l, &[n]);
        l.clear_selection();
    }

    fn remove_many(&mut self, sec: Section, l: &mut List, indices: &[usize]) {
        let mut to_remove: Vec<usize> = indices.iter().copied().filter(|&n| n < l.items.len()).collect();
        to_remove.sort_unstable_by(|a, b| b.cmp(a));
        to_remove.dedup();
        if to_remove.is_empty() {
            return;
        }
        let dir = sec.dir();
        for n in to_remove {
            let it = l.items.remove(n);
            it.remove_file(&dir);
        }
        match sec {
            Section::Buffer => save_buffer(&l.items),
            Section::Quick => quick::save(&l.items),
        }
        l.prune_selection();
    }

    // ---------- Горячая клавиша раздела ----------

    pub fn set_hotkey(&mut self, s: &str) {
        self.hotkey = s.to_string();
    }

    /// Ждёт нажатия сочетания для горячей клавиши раздела.
    pub fn recording(&self) -> bool {
        self.recording
    }

    pub fn stop_recording(&mut self) {
        self.recording = false;
    }

    pub fn set_note(&mut self, note: Option<&'static str>) {
        self.note = note;
    }

    // ---------- Рисование ----------

    fn list_mut(&mut self, sec: Section) -> &mut List {
        match sec {
            Section::Buffer => &mut self.buf,
            Section::Quick => &mut self.quick,
        }
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32, act: &mut Vec<Action>) {
        self.section_bar(ui, cv, w, act);
        // Список вынимается на кадр: рисованию нужны и он, и остальные поля, и методы `self`.
        let sec = self.section;
        let mut l = std::mem::take(self.list_mut(sec));
        self.draw_list(ui, cv, w, h, act, sec, &mut l);
        *self.list_mut(sec) = l;
        ui.paint_tip(cv, Rect::new(0.0, 0.0, w, h));
    }

    /// Верхняя строка: переключатель разделов, справа — горячая клавиша (в быстром доступе)
    /// или напоминание о сроке хранения (в буфере).
    unsafe fn section_bar(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, act: &mut Vec<Action>) {
        let f = &cv.g.fonts;
        let mut x = PAD + 2.0;
        let mut pick = None;
        for (i, (sec, name)) in [(Section::Quick, "Быстрый доступ"), (Section::Buffer, "Буфер")].into_iter().enumerate() {
            let cw = cv.chip_w(name);
            let r = Rect::new(x, TOP + 1.0, cw, 20.0);
            if ui.chip_ex(cv, id(CLIPBOARD, 20 + i as u32), r, name, self.section == sec, false, None) {
                pick = Some(sec);
            }
            x += cw + 6.0;
        }
        if let Some(s) = pick {
            self.set_section(s);
            ui.focus = Some(id(CLIPBOARD, 0));
        }
        let right = w - PAD - 2.0;
        if self.section == Section::Buffer {
            let tip = crate::i18n::pick("Записи хранятся сутки", "Items are kept for a day");
            let tw = cv.text_w(tip, &f.small_r);
            cv.text(tip, Rect::new(right - tw, TOP + 1.0, tw + 2.0, 20.0), &f.small_r, dim());
            return;
        }
        let title = crate::i18n::pick("Горячая клавиша", "Hotkey");
        let label = if self.recording {
            crate::i18n::pick("Нажмите сочетание… (Esc — отмена)", "Press a shortcut… (Esc to cancel)").to_string()
        } else if self.hotkey.is_empty() {
            crate::i18n::pick("Назначить горячую клавишу", "Set a hotkey").to_string()
        } else {
            format!("{title}: {}", quick::pretty_hotkey(&self.hotkey))
        };
        let clear = !self.recording && !self.hotkey.is_empty();
        let mut end = right;
        if clear {
            let r = Rect::new(end - 18.0, TOP + 2.0, 18.0, 18.0);
            if ui.icon_button(cv, id(CLIPBOARD, 23), r, icon::CLOSE, false, crate::i18n::pick("Сбросить горячую клавишу", "Clear hotkey")) {
                self.hotkey.clear();
                self.note = None;
                act.push(Action::SetQuickHotkey(String::new()));
            }
            end -= 22.0;
        }
        let cw = cv.text_w(&label, &f.small_c) + 18.0;
        let r = Rect::new(end - cw, TOP + 1.0, cw, 20.0);
        if ui.chip_ex(cv, id(CLIPBOARD, HOTKEY_ID), r, &label, self.recording, true, None) {
            self.recording = !self.recording;
            self.note = None;
            ui.focus = Some(if self.recording { id(CLIPBOARD, HOTKEY_ID) } else { id(CLIPBOARD, 0) });
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_list(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32, act: &mut Vec<Action>, sec: Section, l: &mut List) {
        let f = &cv.g.fonts;
        let quick_sec = sec == Section::Quick;
        let top = TOP + SECTION_H;
        // Строка поиска: поле слева, фильтры и шестерёнка («не записывать», только в буфере) справа на общем фоне.
        let bar = Rect::new(PAD, top, w - 2.0 * PAD, 26.0);
        cv.rr(bar, 10.0, card());
        let widths: Vec<f32> = FILTERS.iter().map(|n| cv.chip_w(n)).collect();
        let gear = Rect::new(bar.right() - 23.0, bar.y + 4.0, 18.0, 18.0);
        let tail = if quick_sec { bar.right() - 8.0 } else { gear.x - 8.0 };
        let pills_w = chips_total_width(&widths, 4.0);
        let mut px = tail - pills_w;
        let fid = id(CLIPBOARD, 0);
        let query = ui.field_text(fid);
        let count = l.visible(&query).len();
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
        for (i, name) in FILTERS.iter().enumerate() {
            let r = Rect::new(px, bar.y + 4.5, widths[i], 17.0);
            if ui.chip_ex(cv, id(CLIPBOARD, 1 + i as u32), r, name, l.filter == i, true, None) {
                l.filter = i;
            }
            px += widths[i] + 4.0;
        }
        let gear_tip = if self.paused {
            crate::i18n::pick("Возобновить запись буфера", "Resume clipboard recording")
        } else {
            crate::i18n::pick("Приостановить запись буфера", "Pause clipboard recording")
        };
        if !quick_sec && ui.icon_button(cv, id(CLIPBOARD, 6), gear, icon::SETTINGS_GEAR, self.paused, gear_tip) {
            self.paused = !self.paused;
            crate::state::set_clipboard_paused(self.paused);
        }

        // Пока ждём сочетание, клавиатура у кнопки горячей клавиши, а не у поиска.
        ui.sticky_focus = Some(if self.recording { id(CLIPBOARD, HOTKEY_ID) } else { fid });
        l.prune_selection();
        let order = l.visible(&query);

        // Клавиши, пока в поиске фокус: стрелки двигают выбор, Ctrl+P закрепляет.
        let mut cur = l.selected.as_deref().and_then(|t| order.iter().position(|&n| l.items[n].text == t));
        // Выбор по умолчанию — первая запись: кнопки внизу и превью сразу работают.
        if cur.is_none() && !order.is_empty() {
            l.select_only(l.items[order[0]].text.clone());
            cur = Some(0);
        }
        let mut pos = cur;
        for n in ui.take_nav() {
            self.note = None;
            match n {
                Nav::Down => pos = Some(pos.map_or(0, |p| (p + 1).min(order.len().saturating_sub(1)))),
                Nav::Up => pos = Some(pos.map_or(0, |p| p.saturating_sub(1))),
                Nav::Pin if !quick_sec => {
                    let sel = l.selected_indices(&order);
                    if !sel.is_empty() {
                        self.pin_many(l, &sel);
                        // Порядок списка устарел: перерисуем в следующем кадре.
                        return;
                    }
                }
                Nav::Pin => {}
                Nav::Copy => {
                    let sel = l.selected_indices(&order);
                    let items: Vec<&Item> = sel.iter().map(|&n| &l.items[n]).collect();
                    if let Some(a) = copy_many(sec, &items) {
                        act.push(a);
                    }
                }
                Nav::Delete => {
                    let sel = l.selected_indices(&order);
                    if !sel.is_empty() {
                        self.remove_many(sec, l, &sel);
                        return;
                    }
                }
                Nav::Tab => l.filter = (l.filter + 1) % FILTERS.len(),
            }
        }
        if pos != cur {
            if let Some(text) = pos.and_then(|p| order.get(p)).map(|&n| l.items[n].text.clone()) {
                l.select_only(text);
            }
        }
        if ui.take_submitted(fid).is_some() {
            let target = l.selected.as_deref().and_then(|t| l.position(t)).or_else(|| order.first().copied());
            if let Some(a) = target.and_then(|n| self.enter_action(sec, &l.items[n])) {
                act.push(a);
            }
        }

        // Список; в буфере с заголовками секций: сегодня, вчера, раньше.
        const ROW: f32 = 23.0;
        const HEAD: f32 = 16.0;
        let now = sys::now_secs();
        let lt = GetLocalTime();
        let day_start = now.saturating_sub(lt.wHour as u64 * 3600 + lt.wMinute as u64 * 60 + lt.wSecond as u64);
        let list_top = top + 36.0;
        let list = Rect::new(PAD, list_top, LIST_W, (h - list_top - 36.0).max(ROW));

        let mut rows: Vec<(f32, Option<&'static str>, usize)> = Vec::new();
        let (mut y, mut last) = (0.0f32, None::<&'static str>);
        for (p, &n) in order.iter().enumerate() {
            if !quick_sec {
                let head = section(&l.items[n], day_start);
                if last != Some(head) {
                    rows.push((y, Some(head), p));
                    y += HEAD;
                    last = Some(head);
                }
            }
            rows.push((y, None, p));
            y += ROW;
        }

        let sid = id(CLIPBOARD, 100 + sec.index() as u32);
        let row_base = 200 + 300 * sec.index() as u32;
        let off = ui.scroll_begin(cv, sid, list, y);
        let mut clicked = None;
        let mut drag_item = None;
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
            let it = &l.items[n];
            let missing = self.missing(it);
            // Тип записи виден по цвету плашки: картинка — фиолетовая, ссылка — синяя, файл — зелёный
            // (красный, если его нет на месте), текст — серый.
            use crate::ios::SysColor;
            let (g, tint) = if it.is_image() {
                (icon::PICTURE, crate::ui::hue(SysColor::Purple, cv.accent))
            } else if it.is_link() {
                (icon::LINK, crate::ui::hue(SysColor::Blue, cv.accent))
            } else if it.is_file() {
                (icon::FOLDER, crate::ui::hue(if missing { SysColor::Red } else { SysColor::Green }, cv.accent))
            } else {
                let p = crate::theme::get();
                let dark = !crate::theme::is_light([p.bg.r, p.bg.g, p.bg.b]);
                (icon::TEXT_DOC, crate::ios::to_d2d(crate::ios::gray(crate::ios::GrayToken::Gray, dark), 1.0))
            };
            let r = Rect::new(list.x, yy, list.w - 3.0, ROW);
            let sel = l.is_selected(&it.text);
            let label = it.label();
            let right = if missing {
                crate::i18n::t("не найден").to_string()
            } else if let Some((_, w, h)) = it.image() {
                format!("{w}×{h} • {}", short_ago(now, it.ts))
            } else {
                short_ago(now, it.ts)
            };
            let rid = id(CLIPBOARD, row_base + p as u32);
            if ui.row_compact_tinted(cv, rid, r, g, true, &label, &right, 0.0, sel, !sel, Some(tint)) {
                clicked = Some((p, n));
            }
            // Нажатие на строке и сдвиг мыши — начало перетаскивания наружу.
            if !missing && !self.drag_fired && ui.is_dragging(rid) {
                let m = ui.pointer();
                match self.drag_from {
                    Some((i, o)) if i == rid => {
                        if (m.0 - o.0).hypot(m.1 - o.1) > DRAG_START {
                            drag_item = Some(n);
                        }
                    }
                    _ => self.drag_from = Some((rid, m)),
                }
            }
        }
        ui.scroll_end(cv, sid);
        if !ui.down {
            self.drag_from = None;
            self.drag_fired = false;
        }
        if let Some(n) = drag_item {
            self.drag_from = None;
            self.drag_fired = true;
            if !l.is_selected(&l.items[n].text) {
                l.select_only(l.items[n].text.clone());
            }
            let sel = l.selected_indices(&order);
            let items: Vec<&Item> = sel
                .into_iter()
                .filter(|&idx| !self.missing(&l.items[idx]))
                .map(|idx| &l.items[idx])
                .collect();
            let payload = if items.is_empty() {
                drag_payload(sec, &l.items[n])
            } else {
                drag_payload_many(sec, &items)
            };
            act.push(Action::DragOut(payload));
        }
        if let Some((p, n)) = clicked {
            self.note = None;
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            if shift {
                l.select_range(&order, p, ctrl);
            } else if ctrl {
                l.toggle_select(l.items[n].text.clone());
            } else {
                l.select_only(l.items[n].text.clone());
            }
        }
        if order.is_empty() {
            let msg = match (l.items.is_empty(), quick_sec) {
                (true, true) => "Пусто — перетащите файл, ссылку или текст на остров",
                (true, false) => "История пуста — скопируйте что-нибудь",
                (false, _) => "Ничего не найдено",
            };
            cv.text(msg, Rect::new(list.x + 5.0, list.y + 6.0, list.w, 14.0), &f.small, bare());
        }

        let prev = Rect::new(PAD + LIST_W + 14.0, list_top, w - 2.0 * PAD - LIST_W - 14.0, (h - list_top - 36.0).max(120.0));
        if let Some(n) = self.preview(ui, cv, prev, day_start, sec, l) {
            self.remove_at(sec, l, n);
            return;
        }
        self.footer(ui, cv, w, h, act, sec, l, &order);
    }

    /// Что делает Enter на записи: в быстром доступе открывает файлы и ссылки.
    /// Недоступный файл не делает ничего.
    fn enter_action(&self, sec: Section, it: &Item) -> Option<Action> {
        if sec == Section::Quick {
            if let Some(p) = it.path() {
                return (!self.missing(it)).then(|| Action::Open(p.to_string()));
            }
            if it.is_link() {
                return Some(Action::Open(it.text.trim().to_string()));
            }
        }
        None
    }

    /// Превью выбранной записи. `Some(n)` — пользователь убрал недоступный файл (индекс записи).
    unsafe fn preview(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, day_start: u64, sec: Section, l: &List) -> Option<usize> {
        cv.card(r);
        let f = &cv.g.fonts;
        let Some(idx) = l.selected.as_deref().and_then(|t| l.position(t)) else {
            cv.text("Выберите запись слева", r, &f.small_c, dim());
            return None;
        };
        // Область данных сверху; внизу оставлено место под разделитель (#30) и блок метаданных.
        let area = Rect::new(r.x + 14.0, r.y + 11.0, r.w - 28.0, (r.h - 77.0).max(20.0));
        let it = &l.items[idx];
        let when = if sec == Section::Quick { "Добавлено" } else { "Скопировано" };
        let mut meta: Vec<(&str, String)> = vec![("Тип", String::new()), ("Размер", String::new()), (when, copied_at(it.ts, day_start))];
        let mut remove = None;
        if let Some((file, w, h)) = it.image() {
            let path = sec.dir().join(file);
            self.want_preview(path.clone());
            let key = path.to_string_lossy().into_owned();
            if let Some(p) = self.preview.as_ref().filter(|p| p.key == key) {
                let ck = {
                    use std::hash::{Hash, Hasher};
                    let mut hs = std::collections::hash_map::DefaultHasher::new();
                    key.hash(&mut hs);
                    (0x43u64 << 56) | (hs.finish() >> 8)
                };
                let bmp = ui.cache.get(&cv.g.dc, ck, p.w, p.h, || p.px.clone());
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
            let shown: String = it.text.chars().take(600).collect();
            let max_lines = (area.h / 13.5).floor().max(1.0) as usize;
            let clipped = cv.wrap_clip(&shown, &f.wrap_b, area.w, max_lines);
            cv.text_wrap_c(&clipped, area, txt());
            let chars = it.text.chars().count();
            if let Some(path) = it.path() {
                meta[0].1 = crate::i18n::t("Файл").to_string();
                // Диск спрашивается в фоне: сетевой путь не должен вешать рисование.
                if sec == Section::Buffer {
                    self.want_files(vec![path.to_string()]);
                }
                match self.files.get(path).copied() {
                    Some(Some(info)) => {
                        meta[1].1 = if info.dir { crate::i18n::pick("Папка", "Folder").to_string() } else { format_file_size(info.size) };
                    }
                    Some(None) => {
                        meta[1].1 = "—".to_string();
                        let msg = Rect::new(area.x, area.bottom() - 52.0, area.w, 16.0);
                        cv.text("Файл не найден", msg, &f.small_c, status_orange());
                        let btn = Rect::new(area.x + area.w / 2.0 - 70.0, area.bottom() - 30.0, 140.0, 24.0);
                        if sec == Section::Quick
                            && ui.solid_button(cv, id(CLIPBOARD, 24), btn, "Убрать из списка", crate::ui::chip_bg(), txt(), 0.0)
                        {
                            remove = Some(idx);
                        }
                    }
                    None => meta[1].1 = "…".to_string(),
                }
            } else if it.is_link() {
                meta[0].1 = crate::i18n::t("Ссылка").to_string();
                meta[1].1 = chars_label(chars);
            } else {
                let words = it.text.split_whitespace().count();
                let lines = it.text.lines().count().max(1);
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
        remove
    }

    /// Нижняя строка как в макете: клавиши с подписями (кликабельны) и подсказка справа.
    #[allow(clippy::too_many_arguments)]
    unsafe fn footer(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32, act: &mut Vec<Action>, sec: Section, l: &mut List, order: &[usize]) {
        let cy = h - 21.0;
        let sel = l.selected.as_deref().and_then(|t| l.position(t));
        #[derive(Clone, Copy, PartialEq)]
        enum K {
            Enter,
            Copy,
            Pin,
            Delete,
        }
        let can_open = sec == Section::Quick && sel.is_some_and(|i| l.items[i].is_file() || l.items[i].is_link());
        let mut keys: Vec<(K, &str, &str)> = if sec == Section::Quick {
            vec![(K::Copy, "Ctrl C", "Копировать"), (K::Delete, "Del", "Убрать")]
        } else {
            vec![(K::Copy, "Ctrl C", "Копировать"), (K::Pin, "Ctrl P", "Закрепить"), (K::Delete, "Del", "Удалить")]
        };
        if can_open {
            keys.insert(0, (K::Enter, "↵", "Открыть"));
        }
        let mut x = PAD + 2.0;
        let mut after = None;
        for (k, (key, cap, label)) in keys.iter().enumerate() {
            let kid = id(CLIPBOARD, 10 + k as u32);
            let wdt = cv.text_w(cap, &cv.g.fonts.small_c) + 9.0 + 4.0 + cv.text_w(label, &cv.g.fonts.small) + 4.0;
            let (_, clicked) = ui.click_area(kid, Rect::new(x - 2.0, cy - 8.0, wdt + 4.0, 16.0));
            let hv = ui.hover_of(kid);
            if hv > 0.01 {
                let mut c = crate::ui::fg();
                c.a = 0.05 * hv;
                cv.rr(Rect::new(x - 3.0, cy - 9.0, wdt + 6.0, 18.0), 5.0, c);
            }
            cv.keycap(x, cy, cap, label);
            x += wdt + 12.0;
            if clicked {
                let sel_indices = l.selected_indices(order);
                if !sel_indices.is_empty() {
                    self.note = None;
                    after = Some((*key, sel_indices));
                }
            }
        }
        if let Some((key, indices)) = after {
            match key {
                K::Enter => {
                    if let Some(a) = sel.and_then(|i| self.enter_action(sec, &l.items[i])) {
                        act.push(a);
                    }
                }
                K::Copy => {
                    let items: Vec<&Item> = indices.iter().map(|&i| &l.items[i]).collect();
                    if let Some(a) = copy_many(sec, &items) {
                        act.push(a);
                    }
                }
                K::Pin => {
                    self.pin_many(l, &indices);
                }
                K::Delete => self.remove_many(sec, l, &indices),
            }
        }
        let hint = match (self.note, sec) {
            (Some(n), _) => n.to_string(),
            (None, Section::Quick) => {
                if crate::i18n::en() {
                    format!("{} / {} • drag onto the island to add", l.items.len(), quick::MAX_ITEMS)
                } else {
                    format!("{} / {} • перетащите на остров, чтобы добавить", l.items.len(), quick::MAX_ITEMS)
                }
            }
            (None, Section::Buffer) if self.paused => "Запись приостановлена (шестерёнка)".to_string(),
            (None, Section::Buffer) => "↑↓ выбор • Tab фильтр • перетащите в окно".to_string(),
        };
        cv.text(&hint, Rect::new(w - PAD - 270.0, cy - 7.0, 268.0, 14.0), &cv.g.fonts.small_r, bare());
    }
}

fn copy_action(sec: Section, it: &Item) -> Action {
    match it.image() {
        Some((file, _, _)) => Action::CopyImage(sec.dir().join(file)),
        None => Action::Copy(it.path().map_or_else(|| it.text.clone(), str::to_string)),
    }
}

fn copy_many(sec: Section, items: &[&Item]) -> Option<Action> {
    match items {
        [] => None,
        [single] => Some(copy_action(sec, single)),
        many => {
            let texts: Vec<String> = many
                .iter()
                .filter(|it| !it.is_image())
                .map(|it| it.path().map_or_else(|| it.text.clone(), str::to_string))
                .collect();
            if !texts.is_empty() {
                Some(Action::Copy(texts.join("\n")))
            } else {
                many.first().map(|it| copy_action(sec, it))
            }
        }
    }
}

/// Что отдать, когда запись тянут наружу: файл — файлом, ссылка — ссылкой, картинка — картинкой.
fn drag_payload(sec: Section, it: &Item) -> Out {
    if let Some((file, _, _)) = it.image() {
        Out::Image(sec.dir().join(file))
    } else if let Some(p) = it.path() {
        Out::Files(vec![p.to_string()])
    } else if it.is_link() {
        Out::Url(it.text.trim().to_string())
    } else {
        Out::Text(it.text.clone())
    }
}

/// Что отдать при перетаскивании одной или нескольких выбранных записей наружу.
fn drag_payload_many(sec: Section, items: &[&Item]) -> Out {
    if items.len() == 1 {
        return drag_payload(sec, items[0]);
    }
    let mut files = Vec::new();
    let mut images = Vec::new();
    let mut texts = Vec::new();
    let dir = sec.dir();
    for &it in items {
        if let Some((file, _, _)) = it.image() {
            images.push(dir.join(file));
        } else if let Some(p) = it.path() {
            files.push(p.to_string());
        } else {
            texts.push(it.text.clone());
        }
    }
    let text = (!texts.is_empty()).then(|| texts.join("\n"));
    Out::Multi { files, images, text }
}

fn chars_label(chars: usize) -> String {
    if crate::i18n::en() {
        format!("{chars} chars")
    } else {
        format!("{chars} симв.")
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
    if it.ts >= day_start {
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

/// Путь без кавычек и `file://` (у `file:///C:/x` остаётся `C:/x`, а не `/C:/x`).
fn clean_path(s: &str) -> &str {
    let t = s.trim().trim_matches('"');
    if let Some(rest) = t.strip_prefix("file:///") {
        if rest.as_bytes().get(1) == Some(&b':') {
            return rest;
        }
    }
    t.strip_prefix("file://").unwrap_or(t)
}

/// Извлекает имя файла из пути.
fn path_file_name(s: &str) -> &str {
    let t = clean_path(s).trim_end_matches(['\\', '/']);
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

    fn it(text: &str, ts: u64) -> Item {
        Item { text: text.into(), ts }
    }

    #[test]
    fn link_detection_and_filter() {
        let mut l = List::default();
        l.items.push(it("https://example.com/x", 0));
        l.items.push(it("hello world", 0));
        l.items.push(it("see https://a.b", 0));
        l.filter = 2;
        assert_eq!(l.visible(""), vec![0]);
        l.filter = 1;
        assert_eq!(l.visible(""), vec![1, 2]);
        l.filter = 0;
        assert_eq!(l.visible("WORLD"), vec![1]);
    }

    #[test]
    fn count_by_filter_and_query() {
        let mut l = List::default();
        l.items.push(it("https://example.com/x", 0));
        l.items.push(it("hello world", 0));
        l.items.push(it("see https://a.b", 0));
        l.items.push(it(&format!("{IMG_TAG}screen.png:100x100"), 0));
        l.items.push(it(r"C:\Docs\report.pdf", 0));

        // Пустой запрос: число записей в текущем фильтре.
        l.filter = 0;
        assert_eq!(l.visible("").len(), 5);
        l.filter = 1;
        assert_eq!(l.visible("").len(), 2, "путь к файлу — не «текст»");
        l.filter = 2;
        assert_eq!(l.visible("").len(), 1);
        l.filter = 3;
        assert_eq!(l.visible("").len(), 1);
        l.filter = 4;
        assert_eq!(l.visible(""), vec![4]);

        // Поиск с фильтром: число совпадений внутри выбранной категории.
        l.filter = 0;
        assert_eq!(l.visible("world").len(), 1);
        assert_eq!(l.visible("https").len(), 2);
        assert_eq!(l.visible("фото").len(), 1);
        assert_eq!(l.visible("report").len(), 1);
        l.filter = 1;
        assert_eq!(l.visible("https").len(), 1);
        assert_eq!(l.visible("world").len(), 1);
        l.filter = 2;
        assert_eq!(l.visible("https").len(), 1);
        assert_eq!(l.visible("world").len(), 0);
        l.filter = 3;
        assert_eq!(l.visible("фото").len(), 1);
        assert_eq!(l.visible("world").len(), 0);
        l.filter = 4;
        assert_eq!(l.visible("report").len(), 1);
        assert_eq!(l.visible("world").len(), 0);
    }

    #[test]
    fn list_keeps_insertion_order() {
        let mut l = List::default();
        l.items.push(it("new", 2));
        l.items.push(it("old", 1));
        assert_eq!(l.visible(""), vec![0, 1]);
    }

    #[test]
    fn history_expires_after_one_day() {
        let day = RETENTION_SECS;
        assert!(!expired(&it("x", 1000), 1000 + day - 1));
        assert!(expired(&it("x", 1000), 1000 + day));
        assert!(!expired(&it("x", 5000), 1000), "запись «из будущего» не удаляется");
    }

    #[test]
    fn prune_drops_only_old_entries() {
        let mut c = Clip::default();
        let now = 10 * RETENTION_SECS;
        c.buf.items.push(it("fresh", now - 60));
        c.buf.items.push(it("edge", now - RETENTION_SECS + 1));
        c.buf.items.push(it("stale", now - RETENTION_SECS));
        c.buf.items.push(it("ancient", 5));
        assert!(c.prune_buffer(now));
        let left: Vec<&str> = c.buf.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(left, ["fresh", "edge"]);
        assert!(!c.prune_buffer(now), "второй проход ничего не меняет");
    }

    #[test]
    fn pin_is_refused_when_quick_is_full() {
        let mut c = Clip::default();
        c.quick.items = (0..quick::MAX_ITEMS).map(|i| it(&format!("q{i}"), 1)).collect();
        let mut buf = List::default();
        buf.items.push(it("новая", 1));
        assert!(!c.pin(&mut buf, 0));
        assert_eq!(buf.items.len(), 1, "запись остаётся в истории");
        assert_eq!(c.quick.items.len(), quick::MAX_ITEMS);
        assert!(c.note.is_some());
        assert!(c.quick_full());
    }

    #[test]
    fn missing_only_for_files_checked_as_absent() {
        let mut c = Clip::default();
        let file = it(r"C:\gone\a.txt", 0);
        let text = it("hello", 0);
        assert!(!c.missing(&file), "пока проверка не вернулась, файл считается доступным");
        c.files.insert(r"C:\gone\a.txt".into(), None);
        assert!(c.missing(&file));
        assert!(!c.missing(&text));
        c.files.insert(r"C:\gone\a.txt".into(), Some(quick::Info { size: 3, dir: false }));
        assert!(!c.missing(&file));
        c.mark_missing(r"C:\gone\a.txt");
        assert!(c.missing(&file));
    }

    #[test]
    fn enter_opens_files_and_links_in_quick_only() {
        let mut c = Clip::default();
        let file = it(r"C:\Docs\a.pdf", 0);
        let link = it("https://example.com", 0);
        let text = it("заметка", 0);
        assert!(matches!(c.enter_action(Section::Quick, &file), Some(Action::Open(p)) if p == r"C:\Docs\a.pdf"));
        assert!(matches!(c.enter_action(Section::Quick, &link), Some(Action::Open(u)) if u == "https://example.com"));
        assert!(c.enter_action(Section::Quick, &text).is_none());
        assert!(c.enter_action(Section::Buffer, &file).is_none(), "в буфере вставка по Enter убрана");
        c.mark_missing(r"C:\Docs\a.pdf");
        assert!(c.enter_action(Section::Quick, &file).is_none(), "недоступный файл не открывается");
    }

    #[test]
    fn quick_drag_payload_by_kind() {
        assert!(matches!(drag_payload(Section::Quick, &it(r"C:\a.txt", 0)), Out::Files(f) if f == [r"C:\a.txt"]));
        assert!(matches!(drag_payload(Section::Quick, &it("https://a.b", 0)), Out::Url(_)));
        assert!(matches!(drag_payload(Section::Quick, &it("просто текст", 0)), Out::Text(_)));
        assert!(matches!(drag_payload(Section::Quick, &it(&format!("{IMG_TAG}x.png:1x1"), 0)), Out::Image(p) if p.ends_with("x.png")));
        assert!(matches!(drag_payload(Section::Buffer, &it(&format!("{IMG_TAG}b.png:2x2"), 0)), Out::Image(p) if p == clip_dir().join("b.png")));
    }

    #[test]
    fn multi_selection_ctrl_and_shift_click() {
        let mut l = List::default();
        for name in ["a", "b", "c", "d", "e"] {
            l.items.push(it(name, 1));
        }
        let order = l.visible("");

        // Обычный клик выбирает одну запись и ставит её опорой.
        l.select_only("b".into());
        assert_eq!(l.selected_indices(&order), vec![1]);

        // Shift+клик вниз выделяет диапазон [1..=3].
        l.select_range(&order, 3, false);
        assert_eq!(l.selected_indices(&order), vec![1, 2, 3]);

        // Повторный Shift+клик вверх от той же опоры ("b") выделяет [0..=1].
        l.select_range(&order, 0, false);
        assert_eq!(l.selected_indices(&order), vec![0, 1]);

        // Ctrl+клик добавляет "d" к текущему выбору и снимает "a".
        l.toggle_select("d".into());
        assert_eq!(l.selected_indices(&order), vec![0, 1, 3]);
        l.toggle_select("a".into());
        assert_eq!(l.selected_indices(&order), vec![1, 3]);
        // Снятие последней выбранной не оставляет пустоты, пока есть остальные.
        l.toggle_select("d".into());
        assert_eq!(l.selected_indices(&order), vec![1]);
        l.toggle_select("b".into());
        assert!(l.selected_indices(&order).is_empty());
    }

    #[test]
    fn multi_drag_and_batch_remove_and_pin() {
        let mut c = Clip::default();
        let mut l = List::default();
        l.items.push(it(r"C:\one.txt", 1));
        l.items.push(it("hello", 2));
        l.items.push(it("world", 3));
        l.items.push(it(&format!("{IMG_TAG}snap.png:10x10"), 4));
        let order = l.visible("");

        l.select_only(r"C:\one.txt".into());
        l.toggle_select("hello".into());
        l.toggle_select("world".into());
        let sel = l.selected_indices(&order);
        let refs: Vec<&Item> = sel.iter().map(|&i| &l.items[i]).collect();
        match drag_payload_many(Section::Buffer, &refs) {
            Out::Multi { files, images, text } => {
                assert_eq!(files, vec![r"C:\one.txt"]);
                assert!(images.is_empty());
                assert_eq!(text.as_deref(), Some("hello\nworld"));
            }
            _ => panic!("expected Out::Multi"),
        }

        // Закрепление нескольких записей сразу переносит их в быстрый доступ, удаляя из истории.
        assert!(c.pin_many(&mut l, &[1, 2]));
        assert_eq!(l.items.len(), 2);
        assert_eq!(c.quick.items.len(), 2);
        assert_eq!(c.quick.items[0].text, "hello");
        assert_eq!(c.quick.items[1].text, "world");

        // Пакетное удаление оставшихся записей.
        c.remove_many(Section::Buffer, &mut l, &[0, 1]);
        assert!(l.items.is_empty());
    }

    #[test]
    fn sections_and_short_time() {
        assert_eq!(section(&it("x", 1500), 1000), "СЕГОДНЯ");
        assert_eq!(section(&it("x", 10_000), 200_000), "РАНЕЕ");
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
        let file_item = it(r"C:\Users\User\Documents\Report.docx", 0);
        assert!(file_item.is_file());
        assert!(!file_item.is_link());
        assert!(!file_item.is_image());
        assert_eq!(file_item.file_name(), Some("Report.docx"));
        assert_eq!(file_item.label(), "Report.docx");

        let slash_item = it("D:/Photos/vacation.jpg", 0);
        assert!(slash_item.is_file());
        assert_eq!(slash_item.file_name(), Some("vacation.jpg"));

        let unc_item = it(r"\\server\share\data.csv", 0);
        assert!(unc_item.is_file());
        assert_eq!(unc_item.file_name(), Some("data.csv"));

        let quoted_item = it(r#""C:\Program Files\app.exe""#, 0);
        assert!(quoted_item.is_file());
        assert_eq!(quoted_item.file_name(), Some("app.exe"));
        assert_eq!(quoted_item.path(), Some(r"C:\Program Files\app.exe"));

        let file_uri = it("file:///C:/path/file.txt", 0);
        assert!(file_uri.is_file());
        assert_eq!(file_uri.file_name(), Some("file.txt"));

        // Обычный текст и ссылки не должны определяться как файлы
        let text_item = it("Just some text", 0);
        assert!(!text_item.is_file());
        assert_eq!(text_item.file_name(), None);
        assert_eq!(text_item.path(), None);
        assert_eq!(text_item.label(), "Just some text");

        let link_item = it("https://example.com/file.txt", 0);
        assert!(!link_item.is_file());
        assert!(link_item.is_link());
        assert_eq!(link_item.label(), "https://example.com/file.txt");

        let multiline_item = it("C:\\first\nsecond", 0);
        assert!(!multiline_item.is_file());
    }

    #[test]
    fn item_label_separated_from_metadata() {
        // У картинок подпись содержит только имя/тип («Изображение»), размеры вынесены в правую колонку
        assert_eq!(it(&format!("{IMG_TAG}photo.png:1920x1080"), 0).label(), "Изображение");
        // У ссылок — адрес
        assert_eq!(it("https://example.com", 0).label(), "https://example.com");
        // У файлов — имя файла
        assert_eq!(it(r"C:\path\to\document.pdf", 0).label(), "document.pdf");
        // У текста — первая строка
        assert_eq!(it("Hello\nWorld", 0).label(), "Hello");
    }

    #[test]
    fn format_file_sizes() {
        assert_eq!(format_file_size(500), "500 Б");
        assert_eq!(format_file_size(1024), "1.0 КБ");
        assert_eq!(format_file_size(1024 * 1536), "1.5 МБ");
        assert_eq!(format_file_size(1024 * 1024 * 1024 * 2), "2.00 ГБ");
    }
}
