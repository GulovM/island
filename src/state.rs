//! Сохранение и восстановление рабочего состояния острова между запусками (`%APPDATA%\island\state.txt`).
//!
//! Текстовый формат (ключи и секции), свой разбор без сторонних зависимостей.
//! Атомарная запись через временный файл и rename, версия формата, устойчивость
//! к битым/старым файлам. Запись по изменению данных, при скрытии/выходе
//! и перед установкой обновления. 0% CPU в простое (активный таймер сохраняется один раз
//! по абсолютному времени окончания).

use std::path::PathBuf;
use std::sync::Mutex;

use crate::log;

pub const STATE_VERSION: u32 = 1;
pub const STATE_FILE: &str = "state.txt";

/// Состояние таймера на вкладке Today.
#[derive(Clone, Debug, PartialEq)]
pub struct TimerState {
    /// Общая установленная продолжительность таймера в секундах.
    pub duration: u64,
    /// Оставшееся время (на паузе или при сбросе).
    pub left: u64,
    /// Абсолютное Unix-время (в секундах) завершения активного таймера. 0 — таймер не запущен.
    pub finish: u64,
    /// Выбранный пресет (индекс 0..4) или `None`, если время задано вручную.
    pub preset: Option<usize>,
    /// Звуковой сигнал включён.
    pub sound: bool,
}

impl Default for TimerState {
    fn default() -> Self {
        Self {
            duration: 300,
            left: 300,
            finish: 0,
            preset: Some(1),
            sound: true,
        }
    }
}

/// Кэш последнего прогноза погоды на вкладке Today.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherState {
    pub temp: f32,
    pub code: i32,
    pub city: String,
    pub time: u64,
}

/// Полное сохраняемое состояние компонентов острова.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub version: u32,
    /// Название последней открытой вкладки (например, "Today").
    pub last_tab: String,
    /// Пауза записи буфера обмена («Не писать»).
    pub clipboard_paused: bool,
    /// Открытый раздел вкладки «Буфер»: 0 — «Буфер», 1 — «Быстрый доступ».
    pub clipboard_section: usize,
    /// Режим сортировки топа приложений в Stats (true — CPU, false — RAM).
    pub stats_top_by_cpu: bool,
    /// Выбранный раздел обоев в Wallpaper (0 — Все, 1 — Недавние, 2 — WE).
    pub wallpaper_filter: usize,
    /// Вкладка «Приложения»: сетка вместо списка.
    pub apps_grid: bool,
    /// Вкладка «Приложения»: сортировка по имени.
    pub apps_sort_name: bool,
    /// Вкладка «Приложения»: показан полный список (false — только избранные).
    pub apps_all: bool,
    pub timer: TimerState,
    pub weather: WeatherState,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            last_tab: String::new(),
            clipboard_paused: false,
            clipboard_section: 0,
            stats_top_by_cpu: false,
            wallpaper_filter: 0,
            apps_grid: false,
            apps_sort_name: false,
            apps_all: false,
            timer: TimerState::default(),
            weather: WeatherState::default(),
        }
    }
}

struct StateStore {
    state: State,
    dirty: bool,
}

static STORE: Mutex<Option<StateStore>> = Mutex::new(None);

pub fn path() -> PathBuf {
    crate::sys::data_dir().join(STATE_FILE)
}

/// Снимает кавычки со строкового значения и убирает экранирование.
fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        let inner = &s[1..s.len() - 1];
        inner.replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

/// Преобразует состояние в простой текстовый формат.
pub fn serialize(state: &State) -> String {
    let mut out = String::new();
    out.push_str("# Island state\n");
    out.push_str(&format!("version = {}\n", state.version));
    out.push_str(&format!("last_tab = \"{}\"\n", state.last_tab.replace('\\', "\\\\").replace('"', "\\\"")));
    out.push_str(&format!("clipboard_paused = {}\n", state.clipboard_paused as u8));
    out.push_str(&format!("clipboard_section = {}\n", state.clipboard_section));
    out.push_str(&format!("stats_top_by_cpu = {}\n", state.stats_top_by_cpu as u8));
    out.push_str(&format!("wallpaper_filter = {}\n", state.wallpaper_filter));
    out.push_str(&format!("apps_grid = {}\n", state.apps_grid as u8));
    out.push_str(&format!("apps_sort_name = {}\n", state.apps_sort_name as u8));
    out.push_str(&format!("apps_all = {}\n", state.apps_all as u8));

    out.push_str("\n[timer]\n");
    out.push_str(&format!("duration = {}\n", state.timer.duration));
    out.push_str(&format!("left = {}\n", state.timer.left));
    out.push_str(&format!("finish = {}\n", state.timer.finish));
    if let Some(p) = state.timer.preset {
        out.push_str(&format!("preset = {}\n", p));
    } else {
        out.push_str("preset = -1\n");
    }
    out.push_str(&format!("sound = {}\n", state.timer.sound as u8));

    out.push_str("\n[weather]\n");
    out.push_str(&format!("temp = {:.1}\n", state.weather.temp));
    out.push_str(&format!("code = {}\n", state.weather.code));
    out.push_str(&format!("city = \"{}\"\n", state.weather.city.replace('\\', "\\\\").replace('"', "\\\"")));
    out.push_str(&format!("time = {}\n", state.weather.time));

    out
}

/// Разбирает текстовый формат состояния. Устойчив к повреждениям, мусору и старым версиям.
pub fn parse(text: &str) -> State {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut state = State::default();
    let mut section = "";

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            section = rest.trim_end_matches(']').trim();
            continue;
        }

        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());

        match (section, key) {
            ("", "version") => {
                if let Ok(v) = val.parse::<u32>() {
                    state.version = v;
                }
            }
            ("", "last_tab") => {
                state.last_tab = unquote(val);
            }
            ("", "clipboard_paused") => {
                state.clipboard_paused = val == "1" || val.eq_ignore_ascii_case("true");
            }
            ("", "clipboard_section") => {
                if let Ok(s) = val.parse::<usize>() {
                    state.clipboard_section = s.min(1);
                }
            }
            ("", "stats_top_by_cpu") => {
                state.stats_top_by_cpu = val == "1" || val.eq_ignore_ascii_case("true");
            }
            ("", "wallpaper_filter") => {
                if let Ok(f) = val.parse::<usize>() {
                    state.wallpaper_filter = f;
                }
            }
            ("", "apps_grid") => {
                state.apps_grid = val == "1" || val.eq_ignore_ascii_case("true");
            }
            ("", "apps_sort_name") => {
                state.apps_sort_name = val == "1" || val.eq_ignore_ascii_case("true");
            }
            ("", "apps_all") => {
                state.apps_all = val == "1" || val.eq_ignore_ascii_case("true");
            }
            ("timer", "duration") => {
                if let Ok(d) = val.parse::<u64>() {
                    state.timer.duration = d;
                }
            }
            ("timer", "left") => {
                if let Ok(l) = val.parse::<u64>() {
                    state.timer.left = l;
                }
            }
            ("timer", "finish") => {
                if let Ok(f) = val.parse::<u64>() {
                    state.timer.finish = f;
                }
            }
            ("timer", "preset") => {
                if let Ok(p) = val.parse::<i32>() {
                    state.timer.preset = if p >= 0 { Some(p as usize) } else { None };
                }
            }
            ("timer", "sound") => {
                state.timer.sound = val == "1" || val.eq_ignore_ascii_case("true");
            }
            ("weather", "temp") => {
                if let Ok(t) = val.parse::<f32>() {
                    if t.is_finite() {
                        state.weather.temp = t;
                    }
                }
            }
            ("weather", "code") => {
                if let Ok(c) = val.parse::<i32>() {
                    state.weather.code = c;
                }
            }
            ("weather", "city") => {
                state.weather.city = unquote(val);
            }
            ("weather", "time") => {
                if let Ok(t) = val.parse::<u64>() {
                    state.weather.time = t;
                }
            }
            _ => {
                // Неизвестные секции и ключи молча пропускаем для обратной и прямой совместимости.
            }
        }
    }

    state
}

/// Декодирует байты в строку с поддержкой UTF-8 и UTF-16 (LE/BE).
fn decode(b: &[u8]) -> String {
    match b {
        [0xff, 0xfe, rest @ ..] => {
            let u: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&u)
        }
        [0xfe, 0xff, rest @ ..] => {
            let u: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&u)
        }
        _ => String::from_utf8_lossy(b).into_owned(),
    }
}

/// Читает файл состояния с диска.
fn read_state_from_disk() -> State {
    let p = path();
    if let Ok(bytes) = std::fs::read(&p) {
        let text = decode(&bytes);
        if !text.trim().is_empty() {
            return parse(&text);
        }
    }

    State::default()
}

/// Атомарно записывает состояние на диск через временный файл и rename.
fn write_state_to_disk(state: &State) {
    let p = path();
    let tmp = p.with_extension("txt.tmp");
    if let Some(dir) = p.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::error(&format!("не удалось создать папку состояния {}: {e}", dir.display()));
            return;
        }
    }
    let text = serialize(state);

    if let Err(e) = std::fs::write(&tmp, text.as_bytes()) {
        log::error(&format!("не удалось записать временный файл состояния {}: {e}", tmp.display()));
        let _ = std::fs::remove_file(&tmp);
        return;
    }

    if let Err(e) = std::fs::rename(&tmp, &p) {
        log::error(&format!("не удалось обновить файл состояния {}: {e}", p.display()));
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Загружает состояние из `%APPDATA%\island\state.txt`.
pub fn load() -> State {
    let state = read_state_from_disk();
    let clone = state.clone();
    if let Ok(mut g) = STORE.lock() {
        *g = Some(StateStore { state, dirty: false });
    }
    clone
}

/// Изменяет состояние в памяти и помечает его как требующее сохранения.
pub fn update<F: FnOnce(&mut State)>(f: F) {
    if let Ok(mut g) = STORE.lock() {
        if g.is_none() {
            let state = read_state_from_disk();
            *g = Some(StateStore { state, dirty: false });
        }
        if let Some(store) = g.as_mut() {
            f(&mut store.state);
            store.dirty = true;
        }
    }
}

/// Сбрасывает изменённое состояние на диск, если оно было помечено грязным.
pub fn flush() {
    let state_to_write = {
        let Ok(mut g) = STORE.lock() else { return };
        let Some(store) = g.as_mut() else { return };
        if !store.dirty {
            return;
        }
        store.dirty = false;
        store.state.clone()
    };
    write_state_to_disk(&state_to_write);
}

/// Запоминает последнюю активную вкладку.
pub fn set_last_tab(name: &str) {
    update(|s| s.last_tab = name.to_string());
}

/// Запоминает паузу записи буфера обмена.
pub fn set_clipboard_paused(paused: bool) {
    update(|s| s.clipboard_paused = paused);
    flush();
}

/// Запоминает открытый раздел вкладки «Буфер».
pub fn set_clipboard_section(section: usize) {
    update(|s| s.clipboard_section = section);
    flush();
}

/// Запоминает режим сортировки процессов в Stats.
pub fn set_stats_top_by_cpu(top_by_cpu: bool) {
    update(|s| s.stats_top_by_cpu = top_by_cpu);
    flush();
}

/// Запоминает выбранный раздел обоев.
pub fn set_wallpaper_filter(filter: usize) {
    update(|s| s.wallpaper_filter = filter);
    flush();
}

/// Запоминает вид, сортировку и режим («Избранные» / «Показать все») вкладки «Приложения».
pub fn set_apps_view(grid: bool, sort_name: bool, all: bool) {
    update(|s| {
        s.apps_grid = grid;
        s.apps_sort_name = sort_name;
        s.apps_all = all;
    });
    flush();
}

/// Запоминает состояние таймера.
pub fn set_timer(timer: TimerState) {
    update(|s| s.timer = timer);
    flush();
}

/// Запоминает последний прогноз погоды.
pub fn save_weather(temp: f32, code: i32, city: &str) {
    update(|s| {
        s.weather = WeatherState {
            temp,
            code,
            city: city.to_string(),
            time: crate::sys::now_secs(),
        };
    });
    flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_serialization_and_parsing() {
        let state = State {
            version: STATE_VERSION,
            last_tab: "Today".to_string(),
            clipboard_paused: true,
            clipboard_section: 1,
            stats_top_by_cpu: true,
            wallpaper_filter: 2,
            apps_grid: true,
            apps_sort_name: true,
            apps_all: true,
            timer: TimerState {
                duration: 600,
                left: 450,
                finish: 1718000600,
                preset: Some(2),
                sound: false,
            },
            weather: WeatherState {
                temp: 22.4,
                code: 1,
                city: "Санкт-Петербург".to_string(),
                time: 1718000000,
            },
        };

        let text = serialize(&state);
        let parsed = parse(&text);
        assert_eq!(parsed, state);
    }

    #[test]
    fn parse_survives_garbage_and_corrupted_input() {
        let garbage = r#"
            # Заголовок
            !@#$%^&*()_+{}:"<>?~`
            version = invalid_number
            last_tab = 
            clipboard_paused = maybe
            unknown_key = 12345
            [unknown_section]
            foo = bar
            [timer]
            duration = not_a_num
            left = -50
            finish = 999999999999999999999999999999999999999999999999999
            preset = bad
            sound = yes
            [weather]
            temp = NaN
            code = invalid
            city =
            time = string
        "#;

        let parsed = parse(garbage);
        // Не упало и дало осмысленные безопасные значения по умолчанию
        assert_eq!(parsed.version, STATE_VERSION);
    }

    #[test]
    fn parse_handles_bom_and_empty_text() {
        assert_eq!(parse(""), State::default());
        let with_bom = "\u{feff}version = 1\nlast_tab = \"Apps\"\n";
        let parsed = parse(with_bom);
        assert_eq!(parsed.last_tab, "Apps");
    }

    #[test]
    fn unquote_handles_quotes_and_escapes() {
        assert_eq!(unquote("\"test\""), "test");
        assert_eq!(unquote("'single'"), "single");
        assert_eq!(unquote("\"with \\\"quotes\\\" and \\\\ slash\""), "with \"quotes\" and \\ slash");
        assert_eq!(unquote("unquoted"), "unquoted");
    }

    #[test]
    fn parse_handles_crlf_and_whitespace() {
        let text = "\r\nversion = 1\r\n\r\nlast_tab = \"Stats\"\r\n\r\n[timer]\r\nleft = 120\r\n";
        let parsed = parse(text);
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.last_tab, "Stats");
        assert_eq!(parsed.timer.left, 120);
    }

    #[test]
    fn decode_handles_utf16_and_utf8() {
        let utf8_bytes = "version = 1\n".as_bytes();
        assert_eq!(decode(utf8_bytes), "version = 1\n");

        // UTF-16 LE с BOM
        let mut le_bytes = vec![0xff, 0xfe];
        for u in "version = 1".encode_utf16() {
            le_bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode(&le_bytes), "version = 1");
    }
}
