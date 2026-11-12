//! Запись событий острова в основной календарь Google. Событие уходит в Google, только если в форме
//! включена отметка: остров создаёт его через `events.update`/`events.insert` и удаляет при удалении
//! события или снятии отметки. Очередь операций лежит в `events_sync.txt` и повторяется при следующем
//! опросе календаря, если сети не было.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::calendar::{google_access_token, report_google_write_error, sanitize_google_error};
use super::datetime::{civil_from_days, offset_at, utc_offset_minutes, SYSTEM_TZ};
use super::http;
use super::local::{LocalEvent, Repeat};
use crate::log;
use crate::tabs::{read_lines, write_lines};

const SYNC_FILE: &str = "events_sync.txt";
const GOOGLE_EVENTS: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
/// За один запуск отправляется не больше стольких операций: остальные уйдут при следующем опросе.
const MAX_OPS_PER_FLUSH: usize = 50;

/// Что нужно сделать с событием в Google.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// Создать или обновить.
    Upsert,
    /// Уже отправлено, менять нечего.
    Synced,
    /// Удалить из Google (событие удалено в острове или отметка снята).
    Delete,
}

impl Op {
    const fn code(self) -> char {
        match self {
            Self::Upsert => 'U',
            Self::Synced => 'S',
            Self::Delete => 'D',
        }
    }

    fn from_code(s: &str) -> Option<Self> {
        match s {
            "U" => Some(Self::Upsert),
            "S" => Some(Self::Synced),
            "D" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// Связь события острова с событием Google.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    local_id: u64,
    /// Id события в Google: задаётся при первой отправке и не меняется.
    gid: String,
    op: Op,
}

fn parse_entries(lines: &[String]) -> Vec<Entry> {
    lines
        .iter()
        .filter_map(|l| {
            let mut p = l.trim().splitn(3, '|');
            let local_id = p.next()?.parse().ok()?;
            let gid = p.next()?.to_string();
            let op = Op::from_code(p.next()?)?;
            (!gid.is_empty()).then_some(Entry { local_id, gid, op })
        })
        .collect()
}

fn serialize_entries(entries: &[Entry]) -> Vec<String> {
    entries.iter().map(|e| format!("{}|{}|{}", e.local_id, e.gid, e.op.code())).collect()
}

static STORE: Mutex<Option<Vec<Entry>>> = Mutex::new(None);
/// Последний список событий острова: опрос календаря отправляет их без обращения к интерфейсу.
static LOCAL: Mutex<Vec<LocalEvent>> = Mutex::new(Vec::new());
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Последняя записанная в журнал ошибка: одинаковые подряд не повторяются.
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

fn with_store<R>(f: impl FnOnce(&mut Vec<Entry>) -> R) -> R {
    let mut guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(|| parse_entries(&read_lines(SYNC_FILE))))
}

fn persist(entries: &[Entry]) {
    write_lines(SYNC_FILE, &serialize_entries(entries));
}

/// Запоминает события острова для отправки.
pub fn set_local_snapshot(events: &[LocalEvent]) {
    *LOCAL.lock().unwrap_or_else(|e| e.into_inner()) = events.to_vec();
}

/// Событие острова отправляется в Google (или ждёт отправки).
pub fn local_sync_enabled(local_id: u64) -> bool {
    with_store(|e| e.iter().any(|x| x.local_id == local_id && x.op != Op::Delete))
}

/// Id всех событий острова в Google: копии, пришедшие обратно из календаря, при слиянии скрываются.
pub fn synced_gids() -> Vec<String> {
    with_store(|e| e.iter().map(|x| x.gid.clone()).collect())
}

/// Id события в Google: время и номер события, только символы `0-9a-f`, которые Google принимает.
fn new_gid(local_id: u64) -> String {
    format!("isl{:x}{:x}", crate::sys::now_secs(), local_id)
}

/// Включает или выключает отправку события. Включение отправит текущие данные, выключение удалит
/// событие из Google.
pub fn set_synced(local_id: u64, on: bool) {
    with_store(|entries| {
        let pos = entries.iter().position(|e| e.local_id == local_id);
        match (pos, on) {
            (Some(i), true) => entries[i].op = Op::Upsert,
            (None, true) => entries.push(Entry { local_id, gid: new_gid(local_id), op: Op::Upsert }),
            (Some(i), false) => entries[i].op = Op::Delete,
            (None, false) => return,
        }
        persist(entries);
    });
}

/// Забывает все связи: аккаунт отключён, события в нём остаются как есть.
pub fn forget_all() {
    with_store(|entries| {
        entries.clear();
        persist(entries);
    });
}

fn pending() -> bool {
    with_store(|e| e.iter().any(|x| x.op != Op::Synced))
}

/// Отправляет ожидающие операции в фоновом потоке; если поток уже идёт, он подхватит новые сам.
pub fn flush_async() {
    if !pending() || RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let started = std::thread::Builder::new().name("island-google-push".into()).spawn(|| {
        flush();
        RUNNING.store(false, Ordering::SeqCst);
    });
    if let Err(e) = started {
        RUNNING.store(false, Ordering::SeqCst);
        log::error(&format!("не удалось запустить поток island-google-push: {e}"));
    }
}

fn note_error(msg: &str) {
    let mut last = LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner());
    if *last != msg {
        *last = msg.to_string();
        log::error(msg);
    }
}

fn clear_error() {
    LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Исход одной операции для очереди.
enum Done {
    Synced,
    Removed,
}

fn flush() {
    let token = match google_access_token() {
        Ok(t) => t,
        Err(e) => {
            note_error(&format!("запись в Google Календарь: {e}"));
            return;
        }
    };
    // Запись без события острова не отправляем и не удаляем: список мог ещё не загрузиться.
    let mut skipped: Vec<u64> = Vec::new();
    for _ in 0..MAX_OPS_PER_FLUSH {
        let Some(entry) = with_store(|e| e.iter().find(|x| x.op != Op::Synced && !skipped.contains(&x.local_id)).cloned())
        else {
            break;
        };
        let result = match entry.op {
            Op::Delete => delete_remote(&token, &entry.gid).map(|()| Done::Removed),
            _ => {
                let ev = LOCAL.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|e| e.id == entry.local_id).cloned();
                match ev {
                    Some(ev) => upsert_remote(&token, &ev, &entry.gid).map(|()| Done::Synced),
                    None => {
                        skipped.push(entry.local_id);
                        continue;
                    }
                }
            }
        };
        match result {
            Ok(done) => {
                clear_error();
                with_store(|entries| {
                    // Пока шёл запрос, событие могли изменить: тогда операция остаётся в очереди.
                    if let Some(i) = entries.iter().position(|e| e.local_id == entry.local_id && e.op == entry.op) {
                        match done {
                            Done::Synced => entries[i].op = Op::Synced,
                            Done::Removed => {
                                entries.remove(i);
                            }
                        }
                        persist(entries);
                    }
                });
            }
            Err(e) => {
                note_error(&format!("запись в Google Календарь: {}", sanitize_google_error(&e)));
                if e.contains("HTTP 403") {
                    report_google_write_error("Для записи в Google переподключите аккаунт: нужны новые права доступа");
                }
                return;
            }
        }
    }
}

fn http_error(status: u16, body: &str) -> String {
    let reason: String = http::json_str(body, "message").unwrap_or_default().chars().take(160).collect();
    format!("Google Calendar API: HTTP {status} {reason}").trim_end().to_string()
}

fn request(method: &str, url: &str, token: &str, body: Option<&str>) -> Result<(u16, String), String> {
    let auth = format!("Bearer {token}");
    let mut headers = vec![("Authorization", auth.as_str())];
    if body.is_some() {
        headers.push(("Content-Type", "application/json"));
    }
    let resp = unsafe { http::request(method, url, &headers, body.map(str::as_bytes), 64 * 1024)? };
    Ok((resp.status, String::from_utf8_lossy(&resp.body).into_owned()))
}

/// Смещение местного времени от UTC для местной минуты `local_min`.
fn local_offset(local_min: i64) -> i64 {
    offset_at(local_min - utc_offset_minutes(), SYSTEM_TZ)
}

fn upsert_remote(token: &str, ev: &LocalEvent, gid: &str) -> Result<(), String> {
    let body = event_json(ev, gid, &local_offset)?;
    let url = format!("{GOOGLE_EVENTS}/{}", http::urlenc(gid));
    let (status, text) = request("PUT", &url, token, Some(&body))?;
    match status {
        200..=299 => Ok(()),
        // События с таким id в Google ещё нет — создаём.
        404 => {
            let (status, text) = request("POST", GOOGLE_EVENTS, token, Some(&body))?;
            if (200..300).contains(&status) {
                Ok(())
            } else {
                Err(http_error(status, &text))
            }
        }
        _ => Err(http_error(status, &text)),
    }
}

fn delete_remote(token: &str, gid: &str) -> Result<(), String> {
    let url = format!("{GOOGLE_EVENTS}/{}", http::urlenc(gid));
    let (status, text) = request("DELETE", &url, token, None)?;
    match status {
        // Уже удалено — то, что нужно.
        200..=299 | 404 | 410 => Ok(()),
        _ => Err(http_error(status, &text)),
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn date_str(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Местное время с явным смещением: `2026-11-12T10:00:00+03:00`.
fn date_time_str(local_min: i64, off: &dyn Fn(i64) -> i64) -> String {
    let o = off(local_min);
    let (sign, o) = if o < 0 { ('-', -o) } else { ('+', o) };
    let r = local_min.rem_euclid(1440);
    format!("{}T{:02}:{:02}:00{sign}{:02}:{:02}", date_str(local_min.div_euclid(1440)), r / 60, r % 60, o / 60, o % 60)
}

/// Название зоны для повторяющегося события: Google требует `timeZone`, а имени IANA у острова нет,
/// поэтому берётся зона с фиксированным смещением (`Etc/GMT-3` = UTC+3). Она не учитывает переход
/// на летнее время и не выражает смещения не в целых часах.
fn fixed_zone(offset_min: i64) -> Result<String, String> {
    if offset_min % 60 != 0 {
        return Err("повтор события не поддерживается для часового пояса со смещением не в целых часах".into());
    }
    let hours = offset_min / 60;
    Ok(if hours == 0 { "Etc/GMT".to_string() } else { format!("Etc/GMT{:+}", -hours) })
}

/// Тело события для Google Calendar API.
fn event_json(ev: &LocalEvent, gid: &str, off: &dyn Fn(i64) -> i64) -> Result<String, String> {
    let mut j = format!("{{\"id\":\"{}\",\"summary\":\"{}\"", json_escape(gid), json_escape(&ev.title));
    if ev.all_day {
        let start_day = ev.start.div_euclid(1440);
        let end_day = if ev.end > ev.start { ev.end.div_euclid(1440).max(start_day + 1) } else { start_day + 1 };
        j.push_str(&format!(",\"start\":{{\"date\":\"{}\"}},\"end\":{{\"date\":\"{}\"}}", date_str(start_day), date_str(end_day)));
    } else {
        let end = if ev.end > ev.start { ev.end } else { ev.start + 60 };
        let zone = if ev.repeat == Repeat::None { String::new() } else { format!(",\"timeZone\":\"{}\"", fixed_zone(off(ev.start))?) };
        j.push_str(&format!(
            ",\"start\":{{\"dateTime\":\"{}\"{zone}}},\"end\":{{\"dateTime\":\"{}\"{zone}}}",
            date_time_str(ev.start, off),
            date_time_str(end, off)
        ));
    }
    if let Some(rule) = ev.rrule() {
        j.push_str(&format!(",\"recurrence\":[\"RRULE:{rule}\"]"));
    }
    if ev.remind > 0 {
        j.push_str(&format!(",\"reminders\":{{\"useDefault\":false,\"overrides\":[{{\"method\":\"popup\",\"minutes\":{}}}]}}", ev.remind));
    } else {
        j.push_str(",\"reminders\":{\"useDefault\":false}");
    }
    j.push('}');
    Ok(j)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online::days_from_civil;

    fn ev(title: &str, repeat: Repeat, all_day: bool) -> LocalEvent {
        let day = days_from_civil(2026, 11, 12);
        LocalEvent { id: 7, title: title.into(), start: day * 1440 + 600, end: day * 1440 + 660, all_day, repeat, days: 0, remind: 10 }
    }

    #[test]
    fn timed_event_has_offset_and_reminder() {
        let j = event_json(&ev("Встреча \"A\"", Repeat::None, false), "isl1", &|_| 180).unwrap();
        assert!(j.contains("\"summary\":\"Встреча \\\"A\\\"\""), "{j}");
        assert!(j.contains("\"start\":{\"dateTime\":\"2026-11-12T10:00:00+03:00\"}"), "{j}");
        assert!(j.contains("\"end\":{\"dateTime\":\"2026-11-12T11:00:00+03:00\"}"), "{j}");
        assert!(j.contains("\"minutes\":10"));
        assert!(!j.contains("recurrence") && !j.contains("timeZone"));
    }

    #[test]
    fn recurring_event_gets_fixed_zone_and_rrule() {
        let j = event_json(&ev("w", Repeat::Weekly, false), "isl1", &|_| 180).unwrap();
        assert!(j.contains("\"timeZone\":\"Etc/GMT-3\""), "{j}");
        assert!(j.contains("\"recurrence\":[\"RRULE:FREQ=WEEKLY\"]"), "{j}");
        let west = event_json(&ev("w", Repeat::Daily, false), "isl1", &|_| -300).unwrap();
        assert!(west.contains("Etc/GMT+5") && west.contains("-05:00"), "{west}");
        assert!(event_json(&ev("w", Repeat::Daily, false), "isl1", &|_| 330).is_err(), "+5:30 не выразить зоной Etc/GMT");
        // Без повтора смещение не в целых часах не мешает: время уходит с явным смещением.
        assert!(event_json(&ev("w", Repeat::None, false), "isl1", &|_| 330).unwrap().contains("+05:30"));
    }

    #[test]
    fn all_day_event_uses_dates() {
        let mut e = ev("d", Repeat::None, true);
        e.start = days_from_civil(2026, 11, 12) * 1440;
        e.end = e.start + 1440;
        let j = event_json(&e, "isl1", &|_| 180).unwrap();
        assert!(j.contains("\"start\":{\"date\":\"2026-11-12\"},\"end\":{\"date\":\"2026-11-13\"}"), "{j}");
        e.end = 0;
        assert!(event_json(&e, "isl1", &|_| 180).unwrap().contains("\"end\":{\"date\":\"2026-11-13\"}"));
    }

    #[test]
    fn json_escape_covers_control_chars() {
        assert_eq!(json_escape("a\"b\\c\nd\te\u{1}"), "a\\\"b\\\\c\\nd\\te\\u0001");
    }

    #[test]
    fn entries_roundtrip_and_skip_garbage() {
        let entries = vec![
            Entry { local_id: 3, gid: "isl1a3".into(), op: Op::Upsert },
            Entry { local_id: 4, gid: "isl1a4".into(), op: Op::Synced },
            Entry { local_id: 5, gid: "isl1a5".into(), op: Op::Delete },
        ];
        let mut lines = serialize_entries(&entries);
        lines.push("мусор".into());
        lines.push("9||U".into());
        lines.push("9|g|X".into());
        assert_eq!(parse_entries(&lines), entries);
    }

    #[test]
    fn gid_is_valid_google_id() {
        let g = new_gid(12);
        assert!(g.len() >= 5 && g.chars().all(|c| c.is_ascii_digit() || ('a'..='v').contains(&c)), "{g}");
    }
}
