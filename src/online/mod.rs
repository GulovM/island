//! Сетевые данные вкладок (WinHTTP, без сторонних библиотек): погода Open-Meteo, ближайшее
//! событие из iCal-ссылки (например, «секретный адрес» Google Calendar), замер скорости через
//! speed.cloudflare.com. Каждый запрос — в коротком потоке, результат в общем снимке, окно
//! будится `bg::notify`. В простое потоков нет.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::log;

mod calendar;
mod datetime;
mod http;
mod local;
mod speedtest;
mod weather;

pub use calendar::{events_by_day, init_cached_calendar, next_of, refresh_calendar, refresh_calendar_age, DayEvent, Event, RawEvent};
pub use local::{due_reminders, merge_local, Due, DueKind, LocalEvent, Repeat, MAX_LOCAL, MISSED_WINDOW, REMIND_CHOICES};
pub use datetime::{civil_from_days, days_from_civil, now_local_minutes, utc_offset_minutes, weekday};
pub(crate) use http::{get_bytes, get_text, json_str};
pub use speedtest::{run_speedtest, Speed};
pub use weather::{init_cached_weather, refresh_weather, weather_text, Weather};

/// После ошибки повторяем раньше, чем через полный срок.
pub(super) const RETRY_AFTER: Duration = Duration::from_secs(2 * 60);

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
