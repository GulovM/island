use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{bg, log};

use super::datetime::{civil_from_days, days_from_civil, offset_at, weekday, SYSTEM_TZ};
use super::http::get_text;
use super::{spawn, stale, with, RETRY_AFTER};

const CALENDAR_TTL: Duration = Duration::from_secs(5 * 60);
const CALENDAR_CACHE_PREFIX: &str = "calendar_cache_";
const LEGACY_CALENDAR_CACHE_FILE: &str = "calendar_cache.ics";

fn hash_url(url: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    url.trim().hash(&mut h);
    format!("{:016x}", h.finish())
}

fn cache_filename(url: &str) -> String {
    format!("{CALENDAR_CACHE_PREFIX}{}.ics", hash_url(url))
}

/// Упаковывает кэш календаря: заголовок с хэшем ссылки и шифрование через DPAPI (`sys::seal`).
fn pack_cached_calendar(url: &str, ics: &str) -> Option<Vec<u8>> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let header = format!("URL_HASH:{}\n", hash_url(url));
    let mut payload = Vec::with_capacity(header.len() + ics.len());
    payload.extend_from_slice(header.as_bytes());
    payload.extend_from_slice(ics.as_bytes());
    crate::sys::seal(&payload)
}

/// Расшифровывает кэш календаря и проверяет соответствие хэша ссылки.
/// Возвращает `None` при битом кэше, несовпадении ссылки или пустом URL (без паник).
fn unpack_cached_calendar(url: &str, bytes: &[u8]) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || bytes.is_empty() {
        return None;
    }
    let unsealed = crate::sys::unseal(bytes)?;
    let text = String::from_utf8_lossy(&unsealed);
    let (header, ics) = text.split_once('\n')?;
    let hash = header.trim().strip_prefix("URL_HASH:")?;
    if hash != hash_url(url) {
        return None;
    }
    Some(ics.to_string())
}

fn save_cached_calendar(url: &str, ics: &str) {
    let Some(sealed) = pack_cached_calendar(url, ics) else {
        log::error("DPAPI недоступен или пустая ссылка, кэш календаря не сохранён");
        return;
    };
    let file = cache_filename(url);
    crate::tabs::write_bytes(&file, &sealed);
}

fn load_cached_calendar(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let data_dir = crate::sys::data_dir();
    let path = data_dir.join(cache_filename(url));
    if let Ok(bytes) = std::fs::read(&path) {
        if let Some(ics) = unpack_cached_calendar(url, &bytes) {
            return Some(ics);
        }
    }
    // Обратная совместимость с единым файлом calendar_cache.ics из T8
    let legacy_path = data_dir.join(LEGACY_CALENDAR_CACHE_FILE);
    if let Ok(bytes) = std::fs::read(&legacy_path) {
        if let Some(ics) = unpack_cached_calendar(url, &bytes) {
            save_cached_calendar(url, &ics);
            return Some(ics);
        }
    }
    None
}

/// Удаляет файлы кэша, хэши которых не соответствуют ни одной из настроенных ссылок.
pub fn clean_orphan_caches(urls: &[String]) {
    let valid_hashes: std::collections::HashSet<String> = urls.iter().map(|u| hash_url(u)).collect();
    let data_dir = crate::sys::data_dir();
    let Ok(entries) = std::fs::read_dir(&data_dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(hash) = name_str.strip_prefix(CALENDAR_CACHE_PREFIX).and_then(|s| s.strip_suffix(".ics")) {
            if !valid_hashes.contains(hash) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    if urls.is_empty() {
        let _ = std::fs::remove_file(data_dir.join(LEGACY_CALENDAR_CACHE_FILE));
    }
}

/// Инициализирует снимок календаря сохранённым зашифрованным кэшем с диска (до первого сетевого ответа).
/// Загружает кэш только для тех ссылок, которые сейчас настроены; кэши удалённых ссылок удаляются.
pub fn init_cached_calendar(urls: &[String]) {
    let active_urls: Vec<String> = urls.iter().map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).collect();
    clean_orphan_caches(&active_urls);
    if active_urls.is_empty() {
        return;
    }
    let mut cached_lists = Vec::new();
    for url in &active_urls {
        if let Some(ics) = load_cached_calendar(url) {
            let raws = parse_ics(&ics, SYSTEM_TZ);
            if !raws.is_empty() {
                cached_lists.push(raws);
            }
        }
    }
    let merged = merge_raw_events(cached_lists);
    if !merged.is_empty() {
        with(|s| {
            if s.snap.cal.is_empty() {
                s.snap.cal = Arc::new(merged);
                s.snap.cal_ver = 1;
                s.snap.calendar_set = true;
                s.cal_key = active_urls.join("\n");
            }
        });
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Начало в местном времени: минуты от 1970-01-01.
    pub start: i64,
    pub all_day: bool,
    pub title: String,
}

/// Событие календаря как в файле: одно вхождение или правило повтора.
#[derive(Clone, Debug, PartialEq)]
pub struct RawEvent {
    /// Начало в местном времени: минуты от 1970-01-01.
    pub start: i64,
    /// Конец; у событий на весь день — начало следующего дня. 0 — не задан.
    pub end: i64,
    pub all_day: bool,
    pub title: String,
    pub rrule: Option<String>,
    /// Исключённые повторы (минуты; для событий на весь день важен только день).
    pub exdates: Vec<i64>,
    /// Идентификатор события (UID) для дедупликации между календарями.
    pub uid: String,
}

/// Объединяет списки событий из нескольких календарей с дедупликацией:
/// события с одинаковым непустым UID и одинаковым временем начала считаются дубликатами.
/// Если UID пустой, дедупликация производится по названию и времени начала.
pub fn merge_raw_events(lists: impl IntoIterator<Item = Vec<RawEvent>>) -> Vec<RawEvent> {
    let mut out = Vec::new();
    let mut seen_uid = std::collections::HashSet::new();
    let mut seen_title = std::collections::HashSet::new();
    for list in lists {
        for ev in list {
            if !ev.uid.is_empty() {
                if !seen_uid.insert((ev.uid.clone(), ev.start)) {
                    continue;
                }
            } else if !seen_title.insert((ev.title.clone(), ev.start)) {
                continue;
            }
            out.push(ev);
        }
    }
    out
}

/// Событие в списке выбранного дня.
#[derive(Clone, Debug, PartialEq)]
pub struct DayEvent {
    /// Минуты от полуночи; `None` — весь день (или продолжение многодневного события).
    pub minute: Option<i64>,
    pub title: String,
}

/// Обновляет события из настроенных iCal-ссылок (раз в 5 минут).
pub fn refresh_calendar(urls: &[String]) {
    refresh_calendar_age(urls, CALENDAR_TTL);
}

/// То же, но обновляет, если данным больше `max_age` (при открытии вкладки — чаще обычного).
pub fn refresh_calendar_age(urls: &[String], max_age: Duration) {
    let clean_urls: Vec<String> = urls
        .iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .collect();
    let key = clean_urls.join("\n");
    let go = with(|s| {
        s.snap.calendar_set = !clean_urls.is_empty();
        if s.cal_busy || (s.cal_key == key && !stale(s.cal_at, max_age)) {
            return false;
        }
        if s.cal_key != key && !s.snap.cal.is_empty() {
            // Список ссылок сменился: сбрасываем старые события
            s.snap.cal = Arc::default();
            s.snap.cal_ver += 1;
        }
        s.snap.calendar_failed = false;
        if clean_urls.is_empty() {
            s.cal_key.clear();
            s.cal_at = Some(Instant::now());
            clean_orphan_caches(&[]);
            return false;
        }
        s.cal_busy = true;
        true
    });
    if !go {
        return;
    }
    spawn("island-calendar", move || {
        let mut fetched = Vec::with_capacity(clean_urls.len());
        if clean_urls.len() <= 1 {
            for url in &clean_urls {
                fetched.push((url.clone(), get_text(url)));
            }
        } else {
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(clean_urls.len());
                for url in &clean_urls {
                    handles.push(scope.spawn(move || (url.clone(), get_text(url))));
                }
                for h in handles {
                    if let Ok(res) = h.join() {
                        fetched.push(res);
                    }
                }
            });
        }

        let mut all_events = Vec::new();
        let mut any_success = false;
        let mut any_failed = false;

        for (url, res) in fetched {
            match res {
                Ok(ics) => {
                    save_cached_calendar(&url, &ics);
                    let raws = parse_ics(&ics, SYSTEM_TZ);
                    all_events.push(raws);
                    any_success = true;
                }
                Err(e) => {
                    log::error(&format!("календарь {url}: {e}"));
                    any_failed = true;
                    if let Some(cached_ics) = load_cached_calendar(&url) {
                        let raws = parse_ics(&cached_ics, SYSTEM_TZ);
                        if !raws.is_empty() {
                            all_events.push(raws);
                        }
                    }
                }
            }
        }

        let merged = merge_raw_events(all_events);
        with(|s| {
            s.cal_busy = false;
            s.cal_key = key.clone();
            if any_success || !merged.is_empty() {
                s.snap.cal = Arc::new(merged);
                s.snap.cal_ver += 1;
            }
            s.snap.calendar_failed = any_failed;
            if any_failed {
                s.cal_at = Instant::now().checked_sub(CALENDAR_TTL - RETRY_AFTER).or_else(|| Some(Instant::now()));
            } else {
                s.cal_at = Some(Instant::now());
            }
        });
        bg::notify();
    });
}

/// Разбирает `DTSTART`: (местные минуты, весь день). Время с `Z` переводится из UTC в местное,
/// время с `TZID` считается местным.
fn parse_dt(value: &str, offset: i64) -> Option<(i64, bool)> {
    let v = value.trim();
    let num = |a: usize, b: usize| v.get(a..b)?.parse::<u32>().ok();
    let day = days_from_civil(num(0, 4)? as i32, num(4, 6)?, num(6, 8)?);
    if v.len() < 13 || v.as_bytes().get(8) != Some(&b'T') {
        return Some((day * 1440, true));
    }
    let mut t = day * 1440 + num(9, 11)? as i64 * 60 + num(11, 13)? as i64;
    if v.ends_with('Z') {
        t += offset_at(t, offset);
    }
    Some((t, false))
}

fn rule_part<'a>(rule: &'a str, key: &str) -> Option<&'a str> {
    rule.split(';').find_map(|p| p.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

/// Ближайшее наступление события, не раньше `now` (для событий на весь день — не раньше
/// сегодняшнего дня). Повторы: DAILY, WEEKLY (с BYDAY), MONTHLY, YEARLY, INTERVAL, UNTIL, COUNT;
/// исключения EXDATE обрабатывает `occurrence_from`.
fn next_occurrence(start: i64, all_day: bool, rrule: Option<&str>, now: i64, offset: i64) -> Option<i64> {
    let threshold = if all_day { now - now.rem_euclid(1440) } else { now };
    if start >= threshold {
        return Some(start);
    }
    let rule = rrule?;
    let interval = rule_part(rule, "INTERVAL").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let until = rule_part(rule, "UNTIL").and_then(|v| parse_dt(v, offset)).map(|(t, day)| if day { t + 1439 } else { t });
    let count = rule_part(rule, "COUNT").and_then(|v| v.parse::<i64>().ok());
    let tod = start.rem_euclid(1440);
    let sday = start.div_euclid(1440);
    let tday = threshold.div_euclid(1440);
    let ok = |t: i64| until.is_none_or(|u| t <= u);
    let found = match rule_part(rule, "FREQ")? {
        "DAILY" => {
            let mut n = (tday - sday).max(0).div_euclid(interval);
            let mut t = (sday + n * interval) * 1440 + tod;
            if t < threshold {
                n += 1;
                t += interval * 1440;
            }
            if count.is_some_and(|c| n >= c) {
                return None;
            }
            Some(t)
        }
        "WEEKLY" => {
            let days: Vec<i64> = rule_part(rule, "BYDAY")
                .map(|v| {
                    v.split(',')
                        .filter_map(|d| {
                            let d = d.trim_start_matches(|c: char| c == '-' || c == '+' || c.is_ascii_digit());
                            ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|x| *x == d).map(|p| p as i64)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let days = if days.is_empty() { vec![weekday(sday)] } else { days };
            let start_monday = sday - weekday(sday);
            let hit = |d: i64| ((d - weekday(d) - start_monday) / 7) % interval == 0 && days.contains(&weekday(d));
            let first = (tday.max(sday)..tday.max(sday) + 7 * interval + 7).find(|&d| hit(d) && d * 1440 + tod >= threshold);
            // COUNT: номер найденного наступления среди всех с начала серии.
            first.filter(|&d| count.is_none_or(|c| (sday..=d).filter(|&x| hit(x)).count() as i64 <= c)).map(|d| d * 1440 + tod)
        }
        freq @ ("MONTHLY" | "YEARLY") => {
            let (y, m, d) = civil_from_days(sday);
            let step = if freq == "MONTHLY" { interval } else { 12 * interval };
            let mut seen = 0i64;
            let mut res = None;
            for k in 0..1200 {
                let months = (m as i64 - 1) + k * step;
                let (yy, mm) = (y as i64 + months.div_euclid(12), months.rem_euclid(12) as u32 + 1);
                let day = days_from_civil(yy as i32, mm, d);
                // Нет такого числа в месяце (31-е) — пропускаем.
                if civil_from_days(day).2 != d {
                    continue;
                }
                seen += 1;
                let t = day * 1440 + tod;
                if t >= threshold {
                    res = count.is_none_or(|c| seen <= c).then_some(t);
                    break;
                }
            }
            res
        }
        _ => None,
    };
    found.filter(|&t| ok(t))
}

fn unescape_ics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push(' '),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Default)]
struct Cur {
    start: Option<(i64, bool)>,
    end: Option<i64>,
    title: String,
    rrule: Option<String>,
    exdates: Vec<i64>,
    uid: String,
    /// `RECURRENCE-ID`: это изменённое вхождение повтора, а не отдельное событие.
    rid: Option<i64>,
    cancelled: bool,
}

/// Все события календаря в формате iCalendar.
pub fn parse_ics(ics: &str, offset: i64) -> Vec<RawEvent> {
    // Длинные строки iCal переносятся: продолжение начинается с пробела или табуляции.
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.split('\n') {
        let l = raw.trim_end_matches('\r');
        match lines.last_mut() {
            Some(last) if l.starts_with(' ') || l.starts_with('\t') => last.push_str(&l[1..]),
            _ => lines.push(l.to_string()),
        }
    }
    let mut found: Vec<(String, Option<i64>, RawEvent)> = Vec::new();
    let mut cur: Option<Cur> = None;
    for l in &lines {
        if l == "BEGIN:VEVENT" {
            cur = Some(Cur::default());
            continue;
        }
        if l == "END:VEVENT" {
            if let Some(c) = cur.take() {
                if let (Some((start, all_day)), false) = (c.start, c.cancelled) {
                    // Конец: у даты без времени `DTEND` — первый день после события.
                    let end = c.end.filter(|&e| e > start).unwrap_or(0);
                    let ev = RawEvent {
                        start,
                        end,
                        all_day,
                        title: unescape_ics(&c.title),
                        rrule: c.rrule,
                        exdates: c.exdates,
                        uid: c.uid.clone(),
                    };
                    found.push((c.uid, c.rid, ev));
                }
            }
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        let Some((name, value)) = l.split_once(':') else { continue };
        match name.split(';').next().unwrap_or("") {
            "DTSTART" => c.start = parse_dt(value, offset),
            "DTEND" => c.end = parse_dt(value, offset).map(|(t, _)| t),
            "SUMMARY" => c.title = value.to_string(),
            "RRULE" => c.rrule = Some(value.to_string()),
            "EXDATE" => c.exdates.extend(value.split(',').filter_map(|v| parse_dt(v, offset)).map(|(t, _)| t)),
            "UID" => c.uid = value.trim().to_string(),
            "RECURRENCE-ID" => c.rid = parse_dt(value, offset).map(|(t, _)| t),
            "STATUS" => c.cancelled = value.trim().eq_ignore_ascii_case("CANCELLED"),
            _ => {}
        }
    }
    // Изменённое вхождение заменяет исходное: исходное в этот день пропускаем.
    let moved: Vec<(String, i64)> = found.iter().filter_map(|(u, r, _)| r.map(|r| (u.clone(), r))).collect();
    for (uid, rid) in moved {
        if let Some((_, _, master)) = found.iter_mut().find(|(u, r, e)| *u == uid && r.is_none() && e.rrule.is_some()) {
            master.exdates.push(rid);
        }
    }
    found.into_iter().map(|(_, _, e)| e).collect()
}

/// Начало следующего вхождения `raw` не раньше `from` с учётом исключений.
fn occurrence_from(raw: &RawEvent, from: i64, offset: i64) -> Option<i64> {
    let mut from = from;
    for _ in 0..64 {
        let t = next_occurrence(raw.start, raw.all_day, raw.rrule.as_deref(), from, offset)?;
        let skipped = raw.exdates.iter().any(|&x| if raw.all_day { x.div_euclid(1440) == t.div_euclid(1440) } else { x == t });
        if !skipped {
            return Some(t);
        }
        from = if raw.all_day { (t.div_euclid(1440) + 1) * 1440 } else { t + 1 };
    }
    None
}

/// Ближайшее событие не раньше `now`.
pub fn next_of(raws: &[RawEvent], now: i64, offset: i64) -> Option<Event> {
    raws.iter()
        .filter_map(|r| occurrence_from(r, now, offset).map(|t| (t, r)))
        .min_by_key(|(t, _)| *t)
        .map(|(t, r)| Event { start: t, all_day: r.all_day, title: r.title.clone() })
}

/// Ближайшее событие календаря в формате iCalendar.
#[cfg(test)]
pub fn next_event(ics: &str, now: i64, offset: i64) -> Option<Event> {
    next_of(&parse_ics(ics, offset), now, offset)
}

/// События по дням: `days` дней подряд, начиная с `first_day` (дни от 1970-01-01).
/// В каждом дне сначала события на весь день, потом по времени.
pub fn events_by_day(raws: &[RawEvent], first_day: i64, days: usize, offset: i64) -> Vec<Vec<DayEvent>> {
    let (lo, hi) = (first_day, first_day + days as i64);
    let mut out: Vec<Vec<DayEvent>> = vec![Vec::new(); days];
    for raw in raws {
        let dur = if raw.end > raw.start { raw.end - raw.start } else { 0 };
        let mut add = |t: i64| {
            let first = t.div_euclid(1440);
            let last = if dur > 0 { (t + dur - 1).div_euclid(1440).max(first) } else { first };
            for d in first.max(lo)..=last.min(hi - 1) {
                let minute = (!raw.all_day && d == first).then_some(t.rem_euclid(1440));
                out[(d - lo) as usize].push(DayEvent { minute, title: raw.title.clone() });
            }
        };
        if raw.rrule.is_none() {
            add(raw.start);
            continue;
        }
        let mut from = lo * 1440;
        for _ in 0..400 {
            let Some(t) = occurrence_from(raw, from, offset) else { break };
            if t >= hi * 1440 {
                break;
            }
            add(t);
            from = if raw.all_day { (t.div_euclid(1440) + 1) * 1440 } else { t + 1 };
        }
    }
    for day in &mut out {
        day.sort_by(|a, b| a.minute.cmp(&b.minute).then_with(|| a.title.cmp(&b.title)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 1440 + h * 60 + mi
    }

    #[test]
    fn ics_next_single_and_utc() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20261015T080000Z\r\nSUMMARY:Встреча \r\n с командой\r\nEND:VEVENT\r\n\
                   BEGIN:VEVENT\r\nDTSTART;TZID=Europe/Moscow:20261016T090000\r\nSUMMARY:Позже\r\nEND:VEVENT\r\n\
                   BEGIN:VEVENT\r\nDTSTART:20261001T080000Z\r\nSUMMARY:Прошло\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let now = at(2026, 10, 15, 9, 0);
        let ev = next_event(ics, now, 180).unwrap();
        assert_eq!(ev.title, "Встреча с командой");
        assert_eq!(ev.start, at(2026, 10, 15, 11, 0)); // 08:00 UTC + 3 ч
    }

    #[test]
    fn events_are_laid_out_by_day() {
        let ics = "BEGIN:VEVENT\nDTSTART:20261016T090000\nSUMMARY:A\nEND:VEVENT\n\
                   BEGIN:VEVENT\nDTSTART;VALUE=DATE:20261016\nDTEND;VALUE=DATE:20261019\nSUMMARY:Trip\nEND:VEVENT\n\
                   BEGIN:VEVENT\nDTSTART:20261001T100000\nRRULE:FREQ=WEEKLY;BYDAY=FR\nEXDATE:20261023T100000\nSUMMARY:Fri\nEND:VEVENT\n\
                   BEGIN:VEVENT\nDTSTART:20261001T080000\nSUMMARY:Off\nSTATUS:CANCELLED\nEND:VEVENT\n";
        let raws = parse_ics(ics, 0);
        assert_eq!(raws.len(), 3, "отменённое событие пропускается");
        let first = days_from_civil(2026, 10, 12);
        let days = events_by_day(&raws, first, 28, 0);
        let on = |d: u32| &days[(days_from_civil(2026, 10, d) - first) as usize];
        // 16-го: поездка на весь день, затем 09:00 и пятничный повтор в 10:00.
        let t16: Vec<_> = on(16).iter().map(|e| (e.minute, e.title.as_str())).collect();
        assert_eq!(t16, vec![(None, "Trip"), (Some(540), "A"), (Some(600), "Fri")]);
        // Поездка на весь день занимает 16–18-е, 19-е свободно.
        assert!(on(17).iter().any(|e| e.title == "Trip") && on(18).iter().any(|e| e.title == "Trip"));
        assert!(on(19).is_empty());
        // 23-е исключено EXDATE, 30-е повторяется.
        assert!(on(23).is_empty());
        assert_eq!(on(30).len(), 1);
    }

    #[test]
    fn moved_instance_replaces_the_original() {
        let ics = "BEGIN:VEVENT\nUID:x\nDTSTART:20261001T100000\nRRULE:FREQ=DAILY\nSUMMARY:Daily\nEND:VEVENT\n\
                   BEGIN:VEVENT\nUID:x\nRECURRENCE-ID:20261015T100000\nDTSTART:20261015T150000\nSUMMARY:Daily moved\nEND:VEVENT\n";
        let raws = parse_ics(ics, 0);
        let first = days_from_civil(2026, 10, 14);
        let days = events_by_day(&raws, first, 3, 0);
        assert_eq!(days[0].len(), 1);
        let mid: Vec<_> = days[1].iter().map(|e| (e.minute, e.title.as_str())).collect();
        assert_eq!(mid, vec![(Some(900), "Daily moved")]);
    }

    #[test]
    fn ics_recurring() {
        let daily = "BEGIN:VEVENT\nDTSTART;TZID=X:20260901T110000\nRRULE:FREQ=DAILY\nSUMMARY:Daily\nEND:VEVENT\n";
        let now = at(2026, 10, 15, 12, 0);
        assert_eq!(next_event(daily, now, 0).unwrap().start, at(2026, 10, 16, 11, 0));

        let weekly = "BEGIN:VEVENT\nDTSTART:20260907T100000\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\nSUMMARY:W\nEND:VEVENT\n";
        // 15.10.2026 — четверг: следующая — понедельник 19.10.
        assert_eq!(next_event(weekly, now, 0).unwrap().start, at(2026, 10, 19, 10, 0));

        let ended = "BEGIN:VEVENT\nDTSTART:20260901T100000\nRRULE:FREQ=DAILY;UNTIL=20261001T000000Z\nSUMMARY:E\nEND:VEVENT\n";
        assert!(next_event(ended, now, 0).is_none());

        let monthly = "BEGIN:VEVENT\nDTSTART;VALUE=DATE:20260131\nRRULE:FREQ=MONTHLY\nSUMMARY:M\nEND:VEVENT\n";
        let ev = next_event(monthly, now, 0).unwrap();
        assert!(ev.all_day);
        assert_eq!(ev.start, at(2026, 10, 31, 0, 0));
    }

    #[test]
    fn ics_count_non_daily() {
        let now = at(2026, 10, 15, 12, 0);
        let weekly = |c: u32| format!("BEGIN:VEVENT
DTSTART:20260907T100000
RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT={c}
SUMMARY:W
END:VEVENT
");
        assert!(next_event(&weekly(2), now, 0).is_none());
        assert_eq!(next_event(&weekly(8), now, 0).unwrap().start, at(2026, 10, 19, 10, 0));
        let monthly = |c: u32| format!("BEGIN:VEVENT
DTSTART:20260901T100000
RRULE:FREQ=MONTHLY;COUNT={c}
SUMMARY:M
END:VEVENT
");
        assert!(next_event(&monthly(2), now, 0).is_none());
        assert_eq!(next_event(&monthly(3), now, 0).unwrap().start, at(2026, 11, 1, 10, 0));
    }

    #[test]
    fn calendar_cache_sealing_and_url_binding() {
        let url = "https://calendar.google.com/calendar/ical/test%40gmail.com/private-1234/basic.ics";
        let other_url = "https://calendar.google.com/calendar/ical/other%40gmail.com/private-5678/basic.ics";
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Приватная встреча\nEND:VEVENT\nEND:VCALENDAR";

        let sealed = pack_cached_calendar(url, ics).expect("seal");
        assert!(crate::sys::is_sealed(&sealed));

        // При совпадении ссылки кэш успешно расшифровывается и отдаёт оригинальный ICS.
        let unpacked = unpack_cached_calendar(url, &sealed);
        assert_eq!(unpacked.as_deref(), Some(ics));

        // При смене ссылки кэш игнорируется (защита от показа чужого кэша).
        assert_eq!(unpack_cached_calendar(other_url, &sealed), None);

        // При пустой ссылке (календарь отключён) кэш не используется.
        assert_eq!(unpack_cached_calendar("", &sealed), None);
        assert_eq!(unpack_cached_calendar("   ", &sealed), None);

        // Повреждённые байты и мусор не приводят к панике и игнорируются.
        assert_eq!(unpack_cached_calendar(url, b"ISL1corruptedgarbage"), None);
        assert_eq!(unpack_cached_calendar(url, b"random bytes"), None);
        assert_eq!(unpack_cached_calendar(url, b""), None);

        // Старый незашифрованный ICS без заголовка с хэшем ссылки игнорируется.
        assert_eq!(unpack_cached_calendar(url, ics.as_bytes()), None);
    }

    #[test]
    fn merge_raw_events_deduplication() {
        let ev = |uid: &str, title: &str, start: i64| RawEvent {
            start,
            end: start + 60,
            all_day: false,
            title: title.to_string(),
            rrule: None,
            exdates: Vec::new(),
            uid: uid.to_string(),
        };

        // Календарь 1: события A, B, C
        let cal1 = vec![
            ev("uid-1", "Meeting 1", 1000),
            ev("uid-2", "Recurring Master", 2000),
            ev("", "No UID Event", 3000),
        ];

        // Календарь 2:
        // - дубликат uid-1 в то же время (должен отсечься)
        // - то же uid-2, но другое время (изменённое вхождение — должно остаться)
        // - другое событие в то же время 1000 (должно остаться)
        // - дубликат события без UID (должен отсечься)
        // - другое событие без UID в то же время (должно остаться)
        let cal2 = vec![
            ev("uid-1", "Meeting 1 (copy)", 1000),
            ev("uid-2", "Recurring Moved", 2500),
            ev("uid-3", "Other Meeting", 1000),
            ev("", "No UID Event", 3000),
            ev("", "Another No UID", 3000),
        ];

        let merged = merge_raw_events(vec![cal1, cal2]);
        assert_eq!(merged.len(), 6);

        assert_eq!(merged[0].uid, "uid-1");
        assert_eq!(merged[0].title, "Meeting 1");

        assert_eq!(merged[1].uid, "uid-2");
        assert_eq!(merged[1].start, 2000);

        assert_eq!(merged[2].title, "No UID Event");

        assert_eq!(merged[3].uid, "uid-2");
        assert_eq!(merged[3].start, 2500);

        assert_eq!(merged[4].uid, "uid-3");
        assert_eq!(merged[5].title, "Another No UID");
    }

    #[test]
    fn parse_ics_preserves_uid() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:unique-event-id-999\nDTSTART:20261015T100000Z\nSUMMARY:Test UID\nEND:VEVENT\nEND:VCALENDAR\n";
        let raws = parse_ics(ics, 0);
        assert_eq!(raws.len(), 1);
        assert_eq!(raws[0].uid, "unique-event-id-999");
        assert_eq!(raws[0].title, "Test UID");
    }

    #[test]
    fn calendar_cache_filename_is_deterministic() {
        let u1 = "https://a.com/basic.ics";
        let u2 = "https://b.com/basic.ics";
        assert_eq!(cache_filename(u1), cache_filename(u1));
        assert_ne!(cache_filename(u1), cache_filename(u2));
        assert!(cache_filename(u1).starts_with(CALENDAR_CACHE_PREFIX));
        assert!(cache_filename(u1).ends_with(".ics"));
    }
}
