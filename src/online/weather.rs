use std::time::{Duration, Instant};

use crate::{bg, log};

use super::http::{get_text, json_num, json_str, urlenc};
use super::{spawn, stale, with, RETRY_AFTER};

const WEATHER_TTL: Duration = Duration::from_secs(20 * 60);
const NOT_FOUND: &str = "город не найден";

#[derive(Clone, Debug)]
pub struct Weather {
    pub temp: f32,
    pub code: i32,
    pub city: String,
}

/// Инициализирует снимок погоды сохранённым кэшем (до первого сетевого ответа).
pub fn init_cached_weather(temp: f32, code: i32, city: String) {
    if city.is_empty() {
        return;
    }
    with(|s| {
        if s.snap.weather.is_none() {
            s.snap.weather = Some(Weather { temp, code, city: city.clone() });
            s.weather_key = city;
            s.snap.weather_note = None;
        }
    });
}

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
                    crate::state::save_weather(w.temp, w.code, &w.city);
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
