//! Сетевые данные вкладок (WinHTTP, без сторонних библиотек): погода Open-Meteo, ближайшее
//! событие из iCal-ссылки (например, «секретный адрес» Google Calendar), замер скорости через
//! speed.cloudflare.com. Каждый запрос — в коротком потоке, результат в общем снимке, окно
//! будится `bg::notify`. В простое потоков нет.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{bg, log, sys};

mod datetime;
mod http;

pub use datetime::{civil_from_days, days_from_civil, now_local_minutes, utc_offset_minutes, weekday, SYSTEM_TZ};
use datetime::offset_at;
pub(crate) use http::{get_bytes, get_text, json_str};
use http::{http, json_num, urlenc};

const WEATHER_TTL: Duration = Duration::from_secs(20 * 60);
const CALENDAR_TTL: Duration = Duration::from_secs(5 * 60);
/// После ошибки повторяем раньше, чем через полный срок.
const RETRY_AFTER: Duration = Duration::from_secs(2 * 60);
const DOWN_URL: &str = "https://speed.cloudflare.com/__down?bytes=25000000";
const UP_URL: &str = "https://speed.cloudflare.com/__up";
const UP_BYTES: usize = 8_000_000;

#[derive(Clone, Debug)]
pub struct Weather {
    pub temp: f32,
    pub code: i32,
    pub city: String,
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
    rrule: Option<String>,
    /// Исключённые повторы (минуты; для событий на весь день важен только день).
    exdates: Vec<i64>,
}

/// Событие в списке выбранного дня.
#[derive(Clone, Debug, PartialEq)]
pub struct DayEvent {
    /// Минуты от полуночи; `None` — весь день (или продолжение многодневного события).
    pub minute: Option<i64>,
    pub title: String,
}

#[derive(Clone, Debug, Default)]
pub struct Speed {
    /// Мбит/с.
    pub down: f32,
    pub up: f32,
    /// Код узла Cloudflare (город), через который шёл замер.
    pub place: String,
    /// Когда замерили (unix-секунды); 0 — ещё не замеряли.
    pub at: u64,
    pub running: bool,
    pub failed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Online {
    pub weather: Option<Weather>,
    /// Почему погоды нет (не задан город, нет сети).
    pub weather_note: Option<String>,
    /// Все события загруженного календаря.
    pub cal: Arc<Vec<RawEvent>>,
    /// Растёт при каждой смене `cal` (ключ кэша месячного вида).
    pub cal_ver: u64,
    pub calendar_set: bool,
    /// Последняя загрузка календаря не удалась.
    pub calendar_failed: bool,
    pub speed: Speed,
}

#[derive(Default)]
struct State {
    snap: Online,
    weather_at: Option<Instant>,
    weather_busy: bool,
    weather_key: String,
    cal_at: Option<Instant>,
    cal_busy: bool,
    cal_key: String,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn snapshot() -> Online {
    with(|s| s.snap.clone())
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        log::error(&format!("не удалось запустить поток {name}: {e}"));
    }
}

fn stale(at: Option<Instant>, ttl: Duration) -> bool {
    at.is_none_or(|t| t.elapsed() >= ttl)
}

// ---------- Погода ----------

/// Обновляет погоду, если она устарела или сменился город. Пустой город — место определяется по IP.
pub fn refresh_weather(city: &str) {
    let city = city.trim().to_string();
    let go = with(|s| {
        if s.weather_busy {
            return false;
        }
        if s.weather_key != city {
            // Город сменили: старую погоду не показываем, пока грузится новая.
            s.snap.weather = None;
            s.snap.weather_note = None;
            s.weather_at = None;
        } else if !stale(s.weather_at, WEATHER_TTL) {
            return false;
        }
        s.weather_busy = true;
        true
    });
    if !go {
        return;
    }
    spawn("island-weather", move || {
        let r = fetch_weather(&city);
        with(|s| {
            s.weather_busy = false;
            s.weather_key = city.clone();
            match r {
                Ok(w) => {
                    s.snap.weather = Some(w);
                    s.snap.weather_note = None;
                    s.weather_at = Some(Instant::now());
                }
                Err(e) => {
                    log::error(&format!("погода: {e}"));
                    if s.snap.weather.is_none() {
                        let note = if e == NOT_FOUND { "Город не найден" } else { "Нет данных" };
                        s.snap.weather_note = Some(note.into());
                    }
                    s.weather_at = Instant::now().checked_sub(WEATHER_TTL - RETRY_AFTER).or_else(|| Some(Instant::now())); // при малом аптайме checked_sub = None: без запасного значения повторы шли бы без паузы
                }
            }
        });
        bg::notify();
    });
}

const NOT_FOUND: &str = "город не найден";

fn fetch_weather(city: &str) -> Result<Weather, String> {
    if city.is_empty() {
        let (name, lat, lon) = locate_by_ip()?;
        return forecast(lat, lon, if name.is_empty() { "Рядом".into() } else { name });
    }
    let geo = get_text(&format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=ru&format=json",
        urlenc(city)
    ))?;
    let lat = json_num(&geo, "latitude").ok_or(NOT_FOUND)?;
    let lon = json_num(&geo, "longitude").ok_or(NOT_FOUND)?;
    let name = json_str(&geo, "name").unwrap_or_else(|| city.to_string());
    forecast(lat, lon, name)
}

/// Где мы, по IP-адресу: (город, широта, долгота). Бесплатные сервисы без ключа; второй — запасной.
fn locate_by_ip() -> Result<(String, f64, f64), String> {
    let services = [
        ("https://ipwho.is/?lang=ru", "city", "latitude", "longitude"),
        ("https://freeipapi.com/api/json", "cityName", "latitude", "longitude"),
    ];
    let mut last = String::from("нет ответа");
    for (url, city, lat, lon) in services {
        match get_text(url) {
            Ok(t) => {
                if let Some(r) = parse_location(&t, city, lat, lon) {
                    return Ok(r);
                }
                last = format!("в ответе {url} нет координат");
            }
            Err(e) => last = e,
        }
    }
    Err(format!("местоположение по IP: {last}"))
}

fn parse_location(text: &str, city: &str, lat: &str, lon: &str) -> Option<(String, f64, f64)> {
    let (la, lo) = (json_num(text, lat)?, json_num(text, lon)?);
    // (0, 0) — «неизвестно» у некоторых сервисов.
    if la == 0.0 && lo == 0.0 {
        return None;
    }
    Some((json_str(text, city).unwrap_or_default(), la, lo))
}

fn forecast(lat: f64, lon: f64, name: String) -> Result<Weather, String> {
    let fc = get_text(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code&timezone=auto"
    ))?;
    let cur = fc.find("\"current\":{").map(|i| &fc[i..]).ok_or("в ответе нет current")?;
    let temp = json_num(cur, "temperature_2m").ok_or("нет температуры")? as f32;
    let code = json_num(cur, "weather_code").unwrap_or(0.0) as i32;
    Ok(Weather { temp, code, city: name })
}

/// Описание погоды по коду WMO.
pub fn weather_text(code: i32) -> &'static str {
    match code {
        0 => "Ясно",
        1 => "Почти ясно",
        2 => "Переменная облачность",
        3 => "Пасмурно",
        45 | 48 => "Туман",
        51..=57 => "Морось",
        61..=67 => "Дождь",
        71..=77 => "Снег",
        80..=82 => "Ливень",
        85 | 86 => "Снегопад",
        95..=99 => "Гроза",
        _ => "—",
    }
}

// ---------- Календарь ----------

/// Обновляет события из iCal-ссылки (раз в 5 минут).
pub fn refresh_calendar(url: &str) {
    refresh_calendar_age(url, CALENDAR_TTL);
}

/// То же, но обновляет, если данным больше `max_age` (при открытии вкладки — чаще обычного).
pub fn refresh_calendar_age(url: &str, max_age: Duration) {
    let url = url.trim().to_string();
    let go = with(|s| {
        s.snap.calendar_set = !url.is_empty();
        if s.cal_busy || (s.cal_key == url && !stale(s.cal_at, max_age)) {
            return false;
        }
        if s.cal_key != url && !s.snap.cal.is_empty() {
            // Ссылку сменили: чужие события не показываем, пока грузятся новые.
            s.snap.cal = Arc::default();
            s.snap.cal_ver += 1;
        }
        s.snap.calendar_failed = false;
        if url.is_empty() {
            s.cal_key.clear();
            s.cal_at = Some(Instant::now());
            return false;
        }
        s.cal_busy = true;
        true
    });
    if !go {
        return;
    }
    spawn("island-calendar", move || {
        let r = get_text(&url).map(|ics| parse_ics(&ics, SYSTEM_TZ));
        with(|s| {
            s.cal_busy = false;
            s.cal_key = url.clone();
            match r {
                Ok(raws) => {
                    s.snap.cal = Arc::new(raws);
                    s.snap.cal_ver += 1;
                    s.snap.calendar_failed = false;
                    s.cal_at = Some(Instant::now());
                }
                Err(e) => {
                    log::error(&format!("календарь: {e}"));
                    s.snap.calendar_failed = true;
                    s.cal_at = Instant::now().checked_sub(CALENDAR_TTL - RETRY_AFTER).or_else(|| Some(Instant::now()));
                }
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
                    let ev = RawEvent { start, end, all_day, title: unescape_ics(&c.title), rrule: c.rrule, exdates: c.exdates };
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

// ---------- Замер скорости ----------

pub fn run_speedtest() {
    let go = with(|s| {
        if s.snap.speed.running {
            return false;
        }
        s.snap.speed.running = true;
        s.snap.speed.failed = false;
        true
    });
    if !go {
        return;
    }
    bg::notify();
    spawn("island-speed", || {
        let place = get_text("https://speed.cloudflare.com/cdn-cgi/trace")
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("colo=").map(str::to_string)))
            .unwrap_or_default();
        let down = measure(None);
        let up = measure(Some(UP_BYTES));
        with(|s| {
            let sp = &mut s.snap.speed;
            sp.running = false;
            match (down, up) {
                (Ok(d), Ok(u)) => {
                    sp.down = d;
                    sp.up = u;
                    sp.place = place;
                    sp.at = sys::now_secs();
                }
                (d, u) => {
                    log::error(&format!("замер скорости: {:?} / {:?}", d.err(), u.err()));
                    sp.failed = true;
                }
            }
        });
        bg::notify();
    });
}

fn measure(upload: Option<usize>) -> Result<f32, String> {
    let url = if upload.is_some() { UP_URL } else { DOWN_URL };
    let r = unsafe { http(url, upload, 0)? };
    if r.status >= 400 {
        return Err(format!("HTTP {}", r.status));
    }
    let bytes = upload.unwrap_or(r.total as usize) as f64;
    Ok((bytes * 8.0 / r.secs.max(0.001) / 1e6) as f32)
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_location_parsing() {
        let who = r#"{"ip":"1.2.3.4","success":true,"country":"Таджикистан","city":"Душанбе","latitude":38.5357473,"longitude":68.779047}"#;
        let (c, la, lo) = parse_location(who, "city", "latitude", "longitude").unwrap();
        assert_eq!(c, "Душанбе");
        assert!((la - 38.5357).abs() < 0.001 && (lo - 68.779).abs() < 0.001);
        let free = r#"{"ipVersion":4,"latitude":38.5598,"longitude":68.787,"cityName":"Dushanbe"}"#;
        assert_eq!(parse_location(free, "cityName", "latitude", "longitude").unwrap().0, "Dushanbe");
        assert!(parse_location(r#"{"latitude":0,"longitude":0}"#, "city", "latitude", "longitude").is_none());
        assert!(parse_location(r#"{"success":false}"#, "city", "latitude", "longitude").is_none());
    }

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


}
