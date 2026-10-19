//! Сетевые данные вкладок (WinHTTP, без сторонних библиотек): погода Open-Meteo, ближайшее
//! событие из iCal-ссылки (например, «секретный адрес» Google Calendar), замер скорости через
//! speed.cloudflare.com. Каждый запрос — в коротком потоке, результат в общем снимке, окно
//! будится `bg::notify`. В простое потоков нет.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{bg, log, sys};

mod calendar;
mod datetime;
mod http;

pub use calendar::{events_by_day, next_of, refresh_calendar, refresh_calendar_age, DayEvent, Event, RawEvent};
pub use datetime::{civil_from_days, days_from_civil, now_local_minutes, utc_offset_minutes, weekday};
pub(crate) use http::{get_bytes, get_text, json_str};
use http::{http, json_num, urlenc};

const WEATHER_TTL: Duration = Duration::from_secs(20 * 60);
/// После ошибки повторяем раньше, чем через полный срок.
pub(super) const RETRY_AFTER: Duration = Duration::from_secs(2 * 60);
const DOWN_URL: &str = "https://speed.cloudflare.com/__down?bytes=25000000";
const UP_URL: &str = "https://speed.cloudflare.com/__up";
const UP_BYTES: usize = 8_000_000;

#[derive(Clone, Debug)]
pub struct Weather {
    pub temp: f32,
    pub code: i32,
    pub city: String,
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
pub(super) struct State {
    pub(super) snap: Online,
    pub(super) weather_at: Option<Instant>,
    pub(super) weather_busy: bool,
    pub(super) weather_key: String,
    pub(super) cal_at: Option<Instant>,
    pub(super) cal_busy: bool,
    pub(super) cal_key: String,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub(super) fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn snapshot() -> Online {
    with(|s| s.snap.clone())
}

pub(super) fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        log::error(&format!("не удалось запустить поток {name}: {e}"));
    }
}

pub(super) fn stale(at: Option<Instant>, ttl: Duration) -> bool {
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




}
