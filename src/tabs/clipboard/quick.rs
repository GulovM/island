//! Раздел «Быстрый доступ» вкладки «Буфер»: постоянное хранилище закреплённых файлов, ссылок,
//! текстов и картинок (в отличие от истории буфера, записи не удаляются сами).
//!
//! Файл `quick.txt` зашифрован DPAPI, как и история: строка `unix_секунды|текст`, новые сверху.
//! Текст записи кодируется так же, как в истории: путь файла (хранится только путь — нужна
//! актуальная версия, а место не тратится), адрес, текст или `\u{1}img:<файл>:<ш>x<в>`.
//! Картинки лежат копиями PNG в `%APPDATA%\island\quick`: исходного файла у снимков нет.
//! Здесь только данные и чистая логика; рисование — в родительском модуле.

use std::path::PathBuf;
use std::sync::Mutex;

use super::{clean_path, clip_dir, is_file_path, Item, MAX_CHARS};
use crate::sys;
use crate::tabs::{escape, read_lines_sealed, unescape, write_lines_sealed};

const FILE: &str = "quick.txt";
/// Сколько записей помещается в быстром доступе.
pub const MAX_ITEMS: usize = 30;

/// Папка с копиями картинок быстрого доступа.
pub fn dir() -> PathBuf {
    sys::data_dir().join("quick")
}

pub fn load() -> Vec<Item> {
    read_lines_sealed(FILE)
        .iter()
        .filter_map(|l| {
            let (ts, text) = l.split_once('|')?;
            Some(Item { text: unescape(text), ts: ts.parse().ok()? })
        })
        .take(MAX_ITEMS)
        .collect()
}

pub fn save(items: &[Item]) {
    let lines: Vec<String> = items.iter().map(|i| format!("{}|{}", i.ts, escape(&i.text))).collect();
    write_lines_sealed(FILE, &lines);
}

/// Что получилось при добавлении.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Added {
    /// Новые записи.
    pub new: usize,
    /// Уже были: поднялись наверх.
    pub existing: usize,
    /// Место кончилось: остальное не добавлено.
    pub full: bool,
}

/// Приводит брошенную строку к записи: пути файлов — без кавычек и `file://`, текст — обрезан.
fn normalize_entry(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let t: String = t.chars().take(MAX_CHARS).collect();
    Some(if is_file_path(&t) {
        let p = clean_path(&t);
        if t.trim().trim_matches('"').starts_with("file://") { percent_decode(p) } else { p.to_string() }
    } else {
        t
    })
}

/// `%20` → пробел и т. п. в пути из адреса `file://`; неверные последовательности остаются как есть.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (b[i], b.get(i + 1).and_then(|&c| hex(c)), b.get(i + 2).and_then(|&c| hex(c))) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Добавляет записи наверх списка в том порядке, в каком их бросили. Повтор не дублируется, а
/// поднимается наверх. Когда место кончилось, остальное не добавляется: закреплённое не вытесняется.
pub fn add_entries(items: &mut Vec<Item>, entries: &[String], now: u64) -> Added {
    let mut rep = Added::default();
    let mut at = 0;
    for raw in entries {
        let Some(text) = normalize_entry(raw) else { continue };
        if let Some(p) = items.iter().position(|i| i.text == text) {
            items.remove(p);
            items.insert(at.min(items.len()), Item { text, ts: now });
            rep.existing += 1;
            at += 1;
            continue;
        }
        if items.len() >= MAX_ITEMS {
            rep.full = true;
            break;
        }
        items.insert(at, Item { text, ts: now });
        rep.new += 1;
        at += 1;
    }
    rep
}

/// Запись из истории можно закрепить: есть место или она уже закреплена.
pub fn can_pin(quick: &[Item], it: &Item) -> bool {
    quick.len() < MAX_ITEMS || quick.iter().any(|q| q.text == it.text)
}

/// Кладёт запись наверх быстрого доступа (повтор заменяется).
pub fn insert_front(quick: &mut Vec<Item>, mut it: Item, now: u64) {
    quick.retain(|q| q.text != it.text);
    it.ts = now;
    quick.insert(0, it);
}

/// Копирует картинку из истории в быстрый доступ. `false` — не вышло.
pub fn copy_image(file: &str) -> bool {
    let to = dir();
    std::fs::create_dir_all(&to).is_ok() && std::fs::copy(clip_dir().join(file), to.join(file)).is_ok()
}

// ---------- Наличие файлов ----------

/// Что известно о файле на диске.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub size: u64,
    pub dir: bool,
}

type Checked = Vec<(String, Option<Info>)>;

static CHECKED: Mutex<Checked> = Mutex::new(Vec::new());

/// Проверяет пути в фоне: сетевой диск может отвечать секундами, а рисовать нельзя ждать.
/// Результаты забирает `take_checked`, главный поток будится через `bg::notify`.
pub fn spawn_check(paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let spawned = std::thread::Builder::new().name("island-quick-check".into()).spawn(move || {
        for p in paths {
            let info = std::fs::metadata(&p).ok().map(|m| Info { size: m.len(), dir: m.is_dir() });
            if let Ok(mut c) = CHECKED.lock() {
                c.push((p, info));
            }
            crate::bg::notify();
        }
    });
    if let Err(e) = spawned {
        crate::log::error(&format!("не удалось запустить проверку файлов: {e}"));
    }
}

pub fn take_checked() -> Checked {
    CHECKED.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default()
}

// ---------- Горячая клавиша раздела ----------

/// «ctrl+alt+q» → «Ctrl + Alt + Q».
pub fn pretty_hotkey(s: &str) -> String {
    s.split('+')
        .map(|p| match p.trim().to_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "win" | "super" => "Win".to_string(),
            "space" => "Space".to_string(),
            "enter" | "return" => "Enter".to_string(),
            "tab" => "Tab".to_string(),
            other => other.to_uppercase(),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Имя клавиши для `hotkey` в конфиге по коду виртуальной клавиши; `None` — клавиша не годится.
fn key_name(vk: u32) -> Option<String> {
    match vk {
        0x20 => Some("space".into()),
        0x0D => Some("enter".into()),
        0x09 => Some("tab".into()),
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).map(|c| c.to_ascii_lowercase().to_string()),
        0x70..=0x7B => Some(format!("f{}", vk - 0x70 + 1)),
        _ => None,
    }
}

/// Сочетание из нажатых модификаторов и клавиши. Нужен хотя бы один модификатор: одиночную
/// клавишу глобально занимать нельзя.
pub fn combo(ctrl: bool, alt: bool, shift: bool, vk: u32) -> Option<String> {
    if !(ctrl || alt || shift) {
        return None;
    }
    let mut parts = Vec::new();
    if ctrl {
        parts.push("ctrl".to_string());
    }
    if alt {
        parts.push("alt".to_string());
    }
    if shift {
        parts.push("shift".to_string());
    }
    parts.push(key_name(vk)?);
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str) -> Item {
        Item { text: text.into(), ts: 1 }
    }

    fn texts(v: &[Item]) -> Vec<&str> {
        v.iter().map(|i| i.text.as_str()).collect()
    }

    #[test]
    fn dropped_entries_keep_their_order_on_top() {
        let mut v = vec![item("old")];
        let rep = add_entries(&mut v, &["C:\\a.txt".into(), "https://x.y".into(), "заметка".into()], 5);
        assert_eq!(rep, Added { new: 3, existing: 0, full: false });
        assert_eq!(texts(&v), ["C:\\a.txt", "https://x.y", "заметка", "old"]);
        assert!(v[..3].iter().all(|i| i.ts == 5));
    }

    #[test]
    fn repeat_moves_up_instead_of_duplicating() {
        let mut v = vec![item("a"), item("b"), item("c")];
        let rep = add_entries(&mut v, &["c".into()], 9);
        assert_eq!(rep, Added { new: 0, existing: 1, full: false });
        assert_eq!(texts(&v), ["c", "a", "b"]);
    }

    #[test]
    fn full_list_refuses_new_but_still_lifts_existing() {
        let mut v: Vec<Item> = (0..MAX_ITEMS).map(|i| item(&format!("t{i}"))).collect();
        let rep = add_entries(&mut v, &["новая".into()], 2);
        assert_eq!(rep, Added { new: 0, existing: 0, full: true });
        assert_eq!(v.len(), MAX_ITEMS);
        assert!(!texts(&v).contains(&"новая"), "закреплённое не вытесняется");
        let rep = add_entries(&mut v, &["t7".into()], 3);
        assert_eq!(rep.existing, 1);
        assert_eq!(v[0].text, "t7");
        assert_eq!(v.len(), MAX_ITEMS);
    }

    #[test]
    fn limit_stops_a_batch_midway() {
        let mut v: Vec<Item> = (0..MAX_ITEMS - 1).map(|i| item(&format!("t{i}"))).collect();
        let rep = add_entries(&mut v, &["x".into(), "y".into()], 2);
        assert_eq!(rep, Added { new: 1, existing: 0, full: true });
        assert_eq!(v.len(), MAX_ITEMS);
    }

    #[test]
    fn paths_are_cleaned_and_blank_entries_skipped() {
        let mut v = Vec::new();
        let rep = add_entries(&mut v, &["  \"C:\\Program Files\\app.exe\" ".into(), "   ".into(), "file:///D:/a b.txt".into()], 1);
        assert_eq!(rep.new, 2);
        assert_eq!(texts(&v), ["C:\\Program Files\\app.exe", "D:/a b.txt"]);
    }

    #[test]
    fn file_uri_becomes_a_plain_path() {
        let mut v = Vec::new();
        add_entries(&mut v, &["file:///D:/My%20Docs/%D0%BE%D1%82%D1%87%D1%91%D1%82.txt".into()], 1);
        assert_eq!(v[0].text, "D:/My Docs/отчёт.txt");
        assert_eq!(percent_decode("100%"), "100%", "обрывок без цифр не трогаем");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn long_text_is_cut() {
        let mut v = Vec::new();
        add_entries(&mut v, &["я".repeat(MAX_CHARS + 50)], 1);
        assert_eq!(v[0].text.chars().count(), MAX_CHARS);
    }

    #[test]
    fn pin_needs_room_unless_already_there() {
        let full: Vec<Item> = (0..MAX_ITEMS).map(|i| item(&format!("t{i}"))).collect();
        assert!(!can_pin(&full, &item("новая")));
        assert!(can_pin(&full, &item("t3")));
        assert!(can_pin(&full[..5], &item("новая")));
    }

    #[test]
    fn insert_front_replaces_duplicate() {
        let mut q = vec![item("a"), item("b")];
        insert_front(&mut q, Item { text: "b".into(), ts: 1 }, 77);
        assert_eq!(texts(&q), ["b", "a"]);
        assert_eq!(q[0].ts, 77);
    }

    #[test]
    fn hotkey_combo_and_display() {
        assert_eq!(combo(true, true, false, 0x51).as_deref(), Some("ctrl+alt+q"));
        assert_eq!(combo(false, true, true, 0x70).as_deref(), Some("alt+shift+f1"));
        assert_eq!(combo(true, false, false, 0x20).as_deref(), Some("ctrl+space"));
        assert_eq!(combo(false, false, false, 0x51), None, "без модификатора нельзя");
        assert_eq!(combo(true, false, false, 0x1B), None, "Esc не клавиша сочетания");
        assert_eq!(pretty_hotkey("ctrl+alt+q"), "Ctrl + Alt + Q");
        assert_eq!(pretty_hotkey("ctrl+shift+f5"), "Ctrl + Shift + F5");
    }

    #[test]
    fn recorded_combo_is_accepted_by_config() {
        for vk in [0x41u32, 0x35, 0x20, 0x0D, 0x09, 0x70, 0x7B] {
            let s = combo(true, true, false, vk).unwrap();
            assert!(crate::config::parse_hotkey(&s).is_some(), "{s}");
        }
    }
}
