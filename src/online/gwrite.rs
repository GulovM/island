//! Запись событий острова в основной календарь Google и в календарь iCloud. Событие уходит в
//! подключённые аккаунты, только если в форме включён переключатель: остров создаёт его (Google —
//! `events.update`/`events.insert`, iCloud — `PUT` ресурса `.ics`) и удаляет при удалении события
//! или выключении переключателя. Очередь операций лежит в `events_sync.txt` и повторяется при
//! следующем опросе календаря, если сети не было.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::calendar::{
    accounts_connected, apple_auth, apple_first_calendar, google_access_token, ics_text, report_google_write_error,
    sanitize_google_error,
};
use super::caldav;
use super::datetime::{civil_from_days, offset_at, utc_offset_minutes, SYSTEM_TZ};
use super::http;
use super::local::{LocalEvent, Repeat};
use crate::log;
use crate::tabs::{read_lines, write_lines};

const SYNC_FILE: &str = "events_sync.txt";
const GOOGLE_EVENTS: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
/// За один запуск отправляется не больше стольких операций: остальные уйдут при следующем опросе.
const MAX_OPS_PER_FLUSH: usize = 50;

/// Что нужно сделать с событием в календаре.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// Создать или обновить.
    Upsert,
    /// Уже отправлено, менять нечего.
    Synced,
    /// Удалить из календаря (событие удалено в острове или переключатель выключен).
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

/// В какой аккаунт пишем.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Google,
    Apple,
}

impl Target {
    const fn code(self) -> char {
        match self {
            Self::Google => 'g',
            Self::Apple => 'a',
        }
    }

    fn from_code(s: &str) -> Option<Self> {
        match s {
            "g" => Some(Self::Google),
            "a" => Some(Self::Apple),
            _ => None,
        }
    }
}

/// Связь события острова с событием во внешнем календаре.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    local_id: u64,
    target: Target,
    /// Id события в Google или UID в iCloud: задаётся при первой отправке и не меняется.
    gid: String,
    op: Op,
    /// iCloud: адрес календаря, куда событие записано (у Google пусто).
    url: String,
}

fn parse_entries(lines: &[String]) -> Vec<Entry> {
    lines
        .iter()
        .filter_map(|l| {
            let mut p = l.trim().splitn(5, '|');
            let local_id = p.next()?.parse().ok()?;
            let gid = p.next()?.to_string();
            let op = Op::from_code(p.next()?)?;
            // Строки старого вида (три поля) относились к Google.
            let target = p.next().and_then(Target::from_code).unwrap_or(Target::Google);
            let url = p.next().unwrap_or("").to_string();
            (!gid.is_empty()).then_some(Entry { local_id, target, gid, op, url })
        })
        .collect()
}

fn serialize_entries(entries: &[Entry]) -> Vec<String> {
    entries.iter().map(|e| format!("{}|{}|{}|{}|{}", e.local_id, e.gid, e.op.code(), e.target.code(), e.url)).collect()
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

/// Событие острова отправляется во внешний календарь (или ждёт отправки).
pub fn local_sync_enabled(local_id: u64) -> bool {
    with_store(|e| e.iter().any(|x| x.local_id == local_id && x.op != Op::Delete))
}

/// Id всех событий острова во внешних календарях: копии, пришедшие обратно при чтении, скрываются.
pub fn synced_gids() -> Vec<String> {
    with_store(|e| e.iter().map(|x| x.gid.clone()).collect())
}

/// Id события: время и номер события, только символы `0-9a-f`, которые принимают и Google, и iCloud.
fn new_gid(local_id: u64) -> String {
    format!("isl{:x}{:x}", crate::sys::now_secs(), local_id)
}

/// Включает или выключает отправку события во все подключённые аккаунты. Включение отправит текущие
/// данные, выключение удалит событие из календарей.
pub fn set_synced(local_id: u64, on: bool) {
    let (google, apple) = accounts_connected();
    with_store(|entries| {
        let mut changed = false;
        for (target, connected) in [(Target::Google, google), (Target::Apple, apple)] {
            let pos = entries.iter().position(|e| e.local_id == local_id && e.target == target);
            match (pos, on) {
                (Some(i), true) => entries[i].op = Op::Upsert,
                (None, true) if connected => {
                    entries.push(Entry { local_id, target, gid: new_gid(local_id), op: Op::Upsert, url: String::new() })
                }
                (Some(i), false) => entries[i].op = Op::Delete,
                _ => continue,
            }
            changed = true;
        }
        if changed {
            persist(entries);
        }
    });
}

/// Забывает связи с отключённым аккаунтом: события в самом календаре остаются как есть.
pub fn forget_google() {
    forget(Target::Google);
}

pub fn forget_apple() {
    forget(Target::Apple);
}

fn forget(target: Target) {
    with_store(|entries| {
        entries.retain(|e| e.target != target);
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
    let started = std::thread::Builder::new().name("island-calendar-push".into()).spawn(|| {
        flush();
        RUNNING.store(false, Ordering::SeqCst);
    });
    if let Err(e) = started {
        RUNNING.store(false, Ordering::SeqCst);
        log::error(&format!("не удалось запустить поток island-calendar-push: {e}"));
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
    /// Отправлено; для iCloud — адрес календаря, куда записано.
    Synced(String),
    Removed,
}

/// Учётные данные, взятые по требованию: токен Google и заголовок Basic для iCloud.
#[derive(Default)]
struct Creds {
    google: Option<String>,
    apple: Option<String>,
}

fn flush() {
    let mut creds = Creds::default();
    // Аккаунты, к которым не удалось подключиться в этом запуске: их записи ждут следующего опроса.
    let mut blocked: Vec<Target> = Vec::new();
    // Запись без события острова не отправляем и не удаляем: список мог ещё не загрузиться.
    let mut skipped: Vec<(u64, Target)> = Vec::new();
    for _ in 0..MAX_OPS_PER_FLUSH {
        let Some(entry) = with_store(|e| {
            e.iter().find(|x| x.op != Op::Synced && !blocked.contains(&x.target) && !skipped.contains(&(x.local_id, x.target))).cloned()
        }) else {
            break;
        };
        let result = match run_entry(&entry, &mut creds) {
            Some(r) => r,
            None => {
                skipped.push((entry.local_id, entry.target));
                continue;
            }
        };
        match result {
            Ok(done) => {
                clear_error();
                with_store(|entries| {
                    // Пока шёл запрос, событие могли изменить: тогда операция остаётся в очереди.
                    let same = |e: &Entry| e.local_id == entry.local_id && e.target == entry.target && e.op == entry.op;
                    if let Some(i) = entries.iter().position(same) {
                        match done {
                            Done::Synced(url) => {
                                entries[i].op = Op::Synced;
                                entries[i].url = url;
                            }
                            Done::Removed => {
                                entries.remove(i);
                            }
                        }
                        persist(entries);
                    }
                });
            }
            Err(e) => {
                let (name, msg) = match entry.target {
                    Target::Google => ("Google Календарь", sanitize_google_error(&e)),
                    Target::Apple => ("iCloud Календарь", e.clone()),
                };
                note_error(&format!("запись в {name}: {msg}"));
                if entry.target == Target::Google && e.contains("HTTP 403") {
                    report_google_write_error("Для записи в Google переподключите аккаунт: нужны новые права доступа");
                }
                blocked.push(entry.target);
            }
        }
    }
}

/// Выполняет операцию записи; `None` — событие острова не найдено, запись пропускается.
fn run_entry(entry: &Entry, creds: &mut Creds) -> Option<Result<Done, String>> {
    let ev = if entry.op == Op::Delete {
        None
    } else {
        Some(LOCAL.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|e| e.id == entry.local_id).cloned()?)
    };
    Some(match entry.target {
        Target::Google => google_run(entry, ev.as_ref(), creds),
        Target::Apple => apple_run(entry, ev.as_ref(), creds),
    })
}

fn google_run(entry: &Entry, ev: Option<&LocalEvent>, creds: &mut Creds) -> Result<Done, String> {
    if creds.google.is_none() {
        creds.google = Some(google_access_token()?);
    }
    let token = creds.google.as_deref().unwrap_or_default();
    match ev {
        Some(ev) => upsert_remote(token, ev, &entry.gid).map(|()| Done::Synced(String::new())),
        None => delete_remote(token, &entry.gid).map(|()| Done::Removed),
    }
}

fn apple_run(entry: &Entry, ev: Option<&LocalEvent>, creds: &mut Creds) -> Result<Done, String> {
    if creds.apple.is_none() {
        creds.apple = Some(apple_auth().ok_or_else(|| "Apple Календарь не подключён".to_string())?);
    }
    let auth = creds.apple.as_deref().unwrap_or_default();
    match ev {
        Some(ev) => {
            // Календарь выбирается при первой отправке и дальше не меняется: правки и удаление идут туда же.
            let cal = if entry.url.is_empty() { apple_first_calendar(auth)? } else { entry.url.clone() };
            let ics = event_ics(ev, &entry.gid, crate::sys::now_secs(), &local_offset)?;
            caldav::put_resource(&resource_url(&cal, &entry.gid), auth, &ics)?;
            Ok(Done::Synced(cal))
        }
        None => {
            // Без адреса календаря событие в iCloud не создавалось.
            if !entry.url.is_empty() {
                caldav::delete_resource(&resource_url(&entry.url, &entry.gid), auth)?;
            }
            Ok(Done::Removed)
        }
    }
}

/// Адрес ресурса события внутри календаря iCloud.
fn resource_url(calendar: &str, uid: &str) -> String {
    format!("{}{}{uid}.ics", calendar, if calendar.ends_with('/') { "" } else { "/" })
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

/// `YYYYMMDDTHHMMSS` по минутам от 1970-01-01.
fn ics_time(min: i64) -> String {
    let (y, m, d) = civil_from_days(min.div_euclid(1440));
    let r = min.rem_euclid(1440);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}00", r / 60, r % 60)
}

fn ics_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}{m:02}{d:02}")
}

/// Событие iCal для iCloud. Разовое событие уходит в UTC (`Z`), повторяющееся — «плавающим» местным
/// временем без зоны: так клиент сам держит нужный час после перехода на летнее время, а смещение
/// не обязано быть в целых часах.
fn event_ics(ev: &LocalEvent, uid: &str, now_secs: u64, off: &dyn Fn(i64) -> i64) -> Result<String, String> {
    let mut s = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Island//Calendar//EN\r\nBEGIN:VEVENT\r\n");
    s.push_str(&format!("UID:{}\r\n", ics_text(uid)));
    s.push_str(&format!("DTSTAMP:{}Z\r\n", ics_time((now_secs / 60) as i64)));
    s.push_str(&format!("SUMMARY:{}\r\n", ics_text(&ev.title)));
    if ev.all_day {
        let start_day = ev.start.div_euclid(1440);
        let end_day = if ev.end > ev.start { ev.end.div_euclid(1440).max(start_day + 1) } else { start_day + 1 };
        s.push_str(&format!("DTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\n", ics_date(start_day), ics_date(end_day)));
    } else {
        let end = if ev.end > ev.start { ev.end } else { ev.start + 60 };
        if ev.repeat == Repeat::None {
            s.push_str(&format!("DTSTART:{}Z\r\nDTEND:{}Z\r\n", ics_time(ev.start - off(ev.start)), ics_time(end - off(end))));
        } else {
            s.push_str(&format!("DTSTART:{}\r\nDTEND:{}\r\n", ics_time(ev.start), ics_time(end)));
        }
    }
    if let Some(rule) = ev.rrule() {
        s.push_str(&format!("RRULE:{rule}\r\n"));
    }
    if ev.remind > 0 {
        s.push_str(&format!(
            "BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:{}\r\nTRIGGER:-PT{}M\r\nEND:VALARM\r\n",
            ics_text(&ev.title),
            ev.remind
        ));
    }
    s.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    Ok(s)
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
    fn icloud_event_is_valid_ics_and_reads_back() {
        let now = (days_from_civil(2026, 11, 1) * 86_400) as u64;
        let once = event_ics(&ev("Звонок; важно", Repeat::None, false), "isl1a", now, &|_| 180).unwrap();
        assert!(once.contains("UID:isl1a\r\n") && once.contains("SUMMARY:Звонок\\; важно\r\n"), "{once}");
        // 10:00 местного при UTC+3 — 07:00 UTC.
        assert!(once.contains("DTSTART:20261112T070000Z\r\n") && once.contains("DTEND:20261112T080000Z\r\n"), "{once}");
        assert!(once.contains("TRIGGER:-PT10M"));
        // Остров читает собственный ics обратно с тем же началом.
        let raws = super::super::calendar::parse_ics(&once, 180);
        assert_eq!(raws.len(), 1);
        assert_eq!(raws[0].start, days_from_civil(2026, 11, 12) * 1440 + 600);
        // Повтор — плавающее местное время и RRULE.
        let weekly = event_ics(&ev("w", Repeat::Weekly, false), "isl1a", now, &|_| 180).unwrap();
        assert!(weekly.contains("DTSTART:20261112T100000\r\n") && weekly.contains("RRULE:FREQ=WEEKLY\r\n"), "{weekly}");
        let all_day = event_ics(&ev("d", Repeat::None, true), "isl1a", now, &|_| 180).unwrap();
        assert!(all_day.contains("DTSTART;VALUE=DATE:20261112\r\n"), "{all_day}");
    }

    #[test]
    fn resource_url_joins_with_single_slash() {
        assert_eq!(resource_url("https://h/cal/", "u"), "https://h/cal/u.ics");
        assert_eq!(resource_url("https://h/cal", "u"), "https://h/cal/u.ics");
    }

    #[test]
    fn json_escape_covers_control_chars() {
        assert_eq!(json_escape("a\"b\\c\nd\te\u{1}"), "a\\\"b\\\\c\\nd\\te\\u0001");
    }

    #[test]
    fn entries_roundtrip_and_read_old_lines_as_google() {
        let entries = vec![
            Entry { local_id: 3, target: Target::Google, gid: "isl1a3".into(), op: Op::Upsert, url: String::new() },
            Entry { local_id: 4, target: Target::Apple, gid: "isl1a4".into(), op: Op::Synced, url: "https://h/cal/".into() },
            Entry { local_id: 5, target: Target::Google, gid: "isl1a5".into(), op: Op::Delete, url: String::new() },
        ];
        let mut lines = serialize_entries(&entries);
        assert_eq!(parse_entries(&lines), entries);
        lines.push("мусор".into());
        lines.push("9||U".into());
        lines.push("9|g|X".into());
        lines.push("8|isl8|S".into());
        let parsed = parse_entries(&lines);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[3], Entry { local_id: 8, target: Target::Google, gid: "isl8".into(), op: Op::Synced, url: String::new() });
    }

    #[test]
    fn gid_is_valid_google_id() {
        let g = new_gid(12);
        assert!(g.len() >= 5 && g.chars().all(|c| c.is_ascii_digit() || ('a'..='v').contains(&c)), "{g}");
    }
}
