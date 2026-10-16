//! Конфиг `%APPDATA%\island\config.toml`. Своя мини-реализация TOML (ключи, секции,
//! строки, числа, массивы строк) — отдельная зависимость ради десятка строк не нужна.

use std::path::PathBuf;

use crate::log;
use crate::theme;

/// Скругление нижних углов по умолчанию и допустимые пределы, логические пиксели.
pub const DEFAULT_RADIUS: f32 = 14.0;
pub const MAX_RADIUS: f32 = 30.0;

pub const DEFAULT_TEMPLATE: &str = r##"# Island — настройки. Изменения применяются из меню в трее: «Перечитать конфиг».

# Акцентный цвет (активная вкладка), #RRGGBB
accent = "#D4AF5E"

# Цвет фона острова, #RRGGBB (текст и карточки подстраиваются под яркость фона)
bg = "#0D0D0D"

# 1 — брать цвета из Windows (в том числе подстроенные Wallpaper Engine под обои);
# тогда accent и bg выше не используются
system_colors = 0

# Скругление нижних углов острова, пиксели (0–30)
corner_radius = 14

# Скрытые вкладки (список имён из tabs ниже)
hidden_tabs = []

# 1 — в свёрнутом острове без музыки показывать время, раскладку клавиатуры и Wi-Fi
idle_face = 1

# Через сколько мс после ухода курсора остров сворачивается
leave_delay_ms = 220

# Как часто (мс) проверять, не развернули ли окно на весь экран
env_poll_ms = 400

# Размер свёрнутого острова, логические пиксели
collapsed_width = 180
collapsed_height = 24

# Порядок вкладок. Пропущенные добавляются в конец.
tabs = ["Controls", "Today", "Stats", "Clipboard", "Apps", "Wallpaper", "Power"]

# Глобальная горячая клавиша: раскрывает остров на вкладке Apps с фокусом в поиске.
# Модификаторы ctrl/alt/shift/win и клавиша (буква, цифра, space, enter, f1…f12); "" — отключить
hotkey = "ctrl+alt+space"

# Папка с обоями для вкладки Wallpaper (по умолчанию «Изображения» пользователя)
# wallpaper_dir = "D:/Pictures/Wallpapers"

# Город для погоды на вкладке Today (Open-Meteo, ключ не нужен), например "Москва".
# Пусто — город определяется по IP. Меняется и прямо в острове: клик по карточке погоды.
weather_city = ""

# Календарь: «секретный адрес в формате iCal» из настроек Google Calendar (или любой .ics)
calendar_ics = ""

# Индикаторы в свёрнутой форме (1 — включено, 0 — выключено):
# мини-плеер при воспроизведении музыки и короткие уведомления (таймер, громкость, зарядка)
live_media = 1
live_toasts = 1

# API-ключи для вкладок (пока не используются)
[keys]
weather = ""

# Быстрые ссылки во вкладке Apps: имя = "адрес или путь"
[snippets]
# gmail = "https://mail.google.com"
"##;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub accent: [f32; 3],
    /// Цвет фона острова.
    pub bg: [f32; 3],
    /// Цвета берутся из Windows (акцент и палитра системы), `accent` и `bg` не используются.
    pub system_colors: bool,
    /// Скругление нижних углов раскрытого острова, логические пиксели.
    pub corner_radius: f32,
    /// Имена скрытых вкладок.
    pub hidden_tabs: Vec<String>,
    /// Время, раскладка и Wi-Fi на свёрнутом острове, когда музыка не играет.
    pub idle_face: bool,
    pub leave_delay_ms: u32,
    pub env_poll_ms: u32,
    pub collapsed: (f32, f32),
    pub tabs: Vec<String>,
    pub keys: Vec<(String, String)>,
    /// Папка с обоями; `None` — «Изображения» пользователя.
    pub wallpaper_dir: Option<String>,
    /// Быстрые ссылки для вкладки Apps: (имя, адрес).
    pub snippets: Vec<(String, String)>,
    /// Город для погоды; пустой — погода не запрашивается.
    pub weather_city: String,
    /// Ссылка на календарь в формате iCal.
    pub calendar_ics: String,
    /// Глобальная горячая клавиша («ctrl+alt+space»); пустая — отключена.
    pub hotkey: String,
    /// Мини-плеер в свёрнутой форме, пока играет музыка.
    pub live_media: bool,
    /// Короткие уведомления в свёрнутой форме (таймер, громкость, зарядка).
    pub live_toasts: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            accent: theme::DEFAULT_ACCENT,
            bg: theme::DEFAULT_BG,
            system_colors: false,
            corner_radius: DEFAULT_RADIUS,
            hidden_tabs: Vec::new(),
            idle_face: true,
            leave_delay_ms: 220,
            env_poll_ms: 400,
            collapsed: (180.0, 24.0),
            tabs: Vec::new(),
            keys: Vec::new(),
            wallpaper_dir: None,
            snippets: Vec::new(),
            weather_city: String::new(),
            calendar_ics: String::new(),
            hotkey: "ctrl+alt+space".into(),
            live_media: true,
            live_toasts: true,
        }
    }
}

impl Config {
    /// Ключ из секции `[keys]`; пустое значение считается отсутствующим.
    #[allow(dead_code)] // понадобится вкладке Today (погода)
    pub fn key(&self, name: &str) -> Option<&str> {
        self.keys.iter().find(|(k, v)| k == name && !v.is_empty()).map(|(_, v)| v.as_str())
    }

    /// Порядок вкладок: `out[позиция] = индекс в names`. Неизвестные имена игнорируются,
    /// повторы и пропущенные вкладки дописываются в исходном порядке.
    pub fn resolve_order<const N: usize>(&self, names: &[&str; N]) -> [usize; N] {
        let mut out = [0usize; N];
        let mut used = [false; N];
        let mut n = 0;
        for want in &self.tabs {
            if let Some(i) = names.iter().position(|x| x.eq_ignore_ascii_case(want)) {
                if !used[i] {
                    used[i] = true;
                    out[n] = i;
                    n += 1;
                }
            }
        }
        for i in 0..N {
            if !used[i] {
                out[n] = i;
                n += 1;
            }
        }
        out
    }

    /// Вкладки, которые видны на острове, в порядке показа: полный порядок без скрытых.
    /// Хотя бы одна вкладка остаётся всегда (если скрыты все — первая по порядку).
    pub fn resolve_visible<const N: usize>(&self, names: &[&str; N]) -> Vec<usize> {
        let order = self.resolve_order(names);
        let hidden: Vec<usize> =
            self.hidden_tabs.iter().filter_map(|h| names.iter().position(|x| x.eq_ignore_ascii_case(h))).collect();
        let mut out: Vec<usize> = order.iter().copied().filter(|i| !hidden.contains(i)).collect();
        if out.is_empty() {
            out.push(order[0]);
        }
        out
    }
}

/// Записывает настройки внешнего вида обратно в `config.toml`, не трогая остальные строки и комментарии.
pub fn save(cfg: &Config, names: &[&str]) {
    let order: Vec<String> = cfg.tabs.iter().map(|t| t.to_string()).collect();
    let order = if order.is_empty() { names.iter().map(|n| n.to_string()).collect() } else { order };
    let updates = [
        ("accent", fmt_str(&theme::to_hex(cfg.accent))),
        ("bg", fmt_str(&theme::to_hex(cfg.bg))),
        ("system_colors", (cfg.system_colors as u8).to_string()),
        ("corner_radius", format!("{}", cfg.corner_radius.round() as i32)),
        ("tabs", fmt_list(&order)),
        ("hidden_tabs", fmt_list(&cfg.hidden_tabs)),
        ("idle_face", (cfg.idle_face as u8).to_string()),
    ];
    write_updates(&updates);
}

/// Записывает одну строку верхнего уровня (`key = "value"`) в `config.toml` (город погоды из острова).
pub fn save_str(key: &str, value: &str) {
    write_updates(&[(key, fmt_str(value))]);
}

fn write_updates(updates: &[(&str, String)]) {
    let path = path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_TEMPLATE.to_string());
    let out = set_keys(&text, updates);
    if out == text {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, out) {
        log::error(&format!("не удалось записать {}: {e}", path.display()));
    }
}

fn fmt_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn fmt_list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| fmt_str(s)).collect::<Vec<_>>().join(", "))
}

/// Меняет значения верхнего уровня (до первой секции) или добавляет недостающие ключи;
/// комментарии, секции и остальные строки остаются как были. Формат строк (LF/CRLF) и BOM сохраняются.
pub fn set_keys(text: &str, updates: &[(&str, String)]) -> String {
    let bom = text.starts_with('\u{feff}');
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let nl = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    for (key, value) in updates {
        let new_line = format!("{key} = {value}");
        let top = lines.iter().position(|l| l.trim_start().starts_with('[')).unwrap_or(lines.len());
        let found = lines[..top].iter().position(|l| {
            let t = l.trim_start();
            !t.starts_with('#') && t.split_once('=').is_some_and(|(k, _)| k.trim() == *key)
        });
        match found {
            Some(i) => lines[i] = new_line,
            None => {
                // Комментарии и пустые строки перед секцией относятся к ней — вставляем выше них.
                let mut at = top;
                while at > 0 && (lines[at - 1].trim().is_empty() || lines[at - 1].trim_start().starts_with('#')) {
                    at -= 1;
                }
                // Если секций нет, ключ идёт в самый конец файла.
                if top == lines.len() {
                    at = lines.len();
                }
                lines.insert(at, new_line);
            }
        }
    }
    let mut out = String::new();
    if bom {
        out.push('\u{feff}');
    }
    out.push_str(&lines.join(nl));
    out.push_str(nl);
    out
}

pub fn path() -> PathBuf {
    crate::sys::data_dir().join("config.toml")
}

/// Читает конфиг; если файла нет — создаёт с шаблоном. Любые ошибки — в лог и значения по умолчанию.
pub fn load() -> Config {
    let path = path();
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, DEFAULT_TEMPLATE) {
                log::error(&format!("не удалось создать {}: {e}", path.display()));
            }
            parse(DEFAULT_TEMPLATE)
        }
        Err(e) => {
            log::error(&format!("не удалось прочитать {}: {e}", path.display()));
            parse(DEFAULT_TEMPLATE)
        }
    }
}

enum Value {
    Str(String),
    Num(f64),
    List(Vec<String>),
}

pub fn parse(text: &str) -> Config {
    // Некоторые редакторы сохраняют UTF-8 с BOM — без этого первый ключ файла терялся.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut cfg = Config::default();
    let mut section = String::new();
    for (n, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            section = rest.trim_end_matches(']').trim().to_string();
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            log::error(&format!("config.toml:{}: ожидалось «ключ = значение»", n + 1));
            continue;
        };
        let (key, val) = (key.trim(), val.trim());
        let Some(value) = parse_value(val) else {
            log::error(&format!("config.toml:{}: не разобрано значение «{val}»", n + 1));
            continue;
        };
        match (section.as_str(), key, value) {
            ("", "accent", Value::Str(s)) => match theme::parse_hex(&s) {
                Some(c) => cfg.accent = c,
                None => log::error(&format!("config.toml:{}: неверный цвет «{s}»", n + 1)),
            },
            ("", "bg", Value::Str(s)) => match theme::parse_hex(&s) {
                Some(c) => cfg.bg = c,
                None => log::error(&format!("config.toml:{}: неверный цвет «{s}»", n + 1)),
            },
            ("", "system_colors", Value::Num(v)) => cfg.system_colors = v != 0.0,
            ("", "idle_face", Value::Num(v)) => cfg.idle_face = v != 0.0,
            ("", "corner_radius", Value::Num(v)) => cfg.corner_radius = (v as f32).clamp(0.0, MAX_RADIUS),
            ("", "hidden_tabs", Value::List(l)) => cfg.hidden_tabs = l,
            ("", "leave_delay_ms", Value::Num(v)) => cfg.leave_delay_ms = v.clamp(0.0, 5000.0) as u32,
            ("", "env_poll_ms", Value::Num(v)) => cfg.env_poll_ms = v.clamp(100.0, 10_000.0) as u32,
            ("", "collapsed_width", Value::Num(v)) => cfg.collapsed.0 = v.clamp(60.0, 400.0) as f32,
            ("", "collapsed_height", Value::Num(v)) => cfg.collapsed.1 = v.clamp(8.0, 60.0) as f32,
            ("", "live_media", Value::Num(v)) => cfg.live_media = v != 0.0,
            ("", "live_toasts", Value::Num(v)) => cfg.live_toasts = v != 0.0,
            ("", "tabs", Value::List(l)) => cfg.tabs = l,
            ("", "weather_city", Value::Str(s)) => cfg.weather_city = s,
            ("", "calendar_ics", Value::Str(s)) => cfg.calendar_ics = s,
            ("", "hotkey", Value::Str(s)) => {
                if !s.is_empty() && parse_hotkey(&s).is_none() {
                    log::error(&format!("config.toml:{}: неверная горячая клавиша «{s}»", n + 1));
                }
                cfg.hotkey = s;
            }
            ("", "wallpaper_dir", Value::Str(s)) => cfg.wallpaper_dir = Some(s).filter(|d| !d.is_empty()),
            ("keys", k, Value::Str(s)) => cfg.keys.push((k.to_string(), s)),
            ("snippets", k, Value::Str(s)) => cfg.snippets.push((k.to_string(), s)),
            _ => log::error(&format!("config.toml:{}: неизвестный параметр «{key}»", n + 1)),
        }
    }
    cfg
}

/// Отрезает `# комментарий`, если `#` не внутри строки.
fn strip_comment(line: &str) -> &str {
    let (mut in_str, mut esc) = (false, false);
    for (i, c) in line.char_indices() {
        match c {
            _ if esc => esc = false,
            '\\' if in_str => esc = true,
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_value(s: &str) -> Option<Value> {
    if s.starts_with('"') {
        let (v, rest) = parse_string(s)?;
        return rest.trim().is_empty().then_some(Value::Str(v));
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        let mut items = Vec::new();
        let mut rest = inner.trim();
        while !rest.is_empty() {
            let (v, tail) = parse_string(rest)?;
            items.push(v);
            rest = tail.trim_start();
            if let Some(t) = rest.strip_prefix(',') {
                rest = t.trim_start();
            } else if !rest.is_empty() {
                return None;
            }
        }
        return Some(Value::List(items));
    }
    match s {
        "true" => return Some(Value::Num(1.0)),
        "false" => return Some(Value::Num(0.0)),
        _ => {}
    }
    s.parse::<f64>().ok().map(Value::Num)
}

/// Разбирает `"строку"` в начале `s`, возвращает её и остаток.
fn parse_string(s: &str) -> Option<(String, &str)> {
    let body = s.strip_prefix('"')?;
    let mut chars = body.char_indices();
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &body[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// «ctrl+alt+space» → (модификаторы `MOD_*`, код виртуальной клавиши). Нужен хотя бы один
/// модификатор: одиночную клавишу глобально занимать нельзя.
pub fn parse_hotkey(s: &str) -> Option<(u32, u32)> {
    let (mut mods, mut vk) = (0u32, None);
    for part in s.split('+') {
        let p = part.trim().to_lowercase();
        match p.as_str() {
            "alt" => mods |= 0x1,
            "ctrl" | "control" => mods |= 0x2,
            "shift" => mods |= 0x4,
            "win" | "super" => mods |= 0x8,
            "space" => vk = Some(0x20),
            "enter" | "return" => vk = Some(0x0D),
            "tab" => vk = Some(0x09),
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => vk = Some(k.as_bytes()[0].to_ascii_uppercase() as u32),
            k if k.starts_with('f') && k[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n)) => {
                vk = Some(0x70 + k[1..].parse::<u32>().unwrap() - 1)
            }
            _ => return None,
        }
    }
    (mods != 0).then_some(()).and(vk).map(|v| (mods, v))
}

#[cfg(test)]
mod tests {
    #[test]
    fn hotkey_parsing() {
        assert_eq!(super::parse_hotkey("ctrl+alt+space"), Some((0x3, 0x20)));
        assert_eq!(super::parse_hotkey("Win + Shift + K"), Some((0xC, 0x4B)));
        assert_eq!(super::parse_hotkey("ctrl+f5"), Some((0x2, 0x74)));
        assert_eq!(super::parse_hotkey("space"), None);
        assert_eq!(super::parse_hotkey("ctrl+"), None);
        assert_eq!(super::parse_hotkey("ctrl+f13"), None);
    }

    use super::*;

    const NAMES: [&str; 4] = ["A", "B", "C", "D"];

    #[test]
    fn template_matches_defaults() {
        let c = parse(DEFAULT_TEMPLATE);
        assert!((c.accent[0] - 0.83).abs() < 0.01 && (c.accent[2] - 0.37).abs() < 0.01);
        assert_eq!((c.leave_delay_ms, c.env_poll_ms), (220, 400));
        assert_eq!(c.collapsed, (180.0, 24.0));
        assert_eq!(c.tabs.len(), 7);
        assert_eq!(c.key("weather"), None);
    }

    #[test]
    fn overrides_and_clamps() {
        let c = parse("leave_delay_ms = 99999 # много\ncollapsed_width = 1\n[keys]\nweather = \"abc#1\"\n");
        assert_eq!(c.leave_delay_ms, 5000);
        assert_eq!(c.collapsed.0, 60.0);
        assert_eq!(c.key("weather"), Some("abc#1"));
    }

    #[test]
    fn bom_does_not_eat_first_key() {
        let c = parse("\u{feff}leave_delay_ms = 500\r\nenv_poll_ms = 900\r\n");
        assert_eq!((c.leave_delay_ms, c.env_poll_ms), (500, 900));
    }

    #[test]
    fn snippets_and_wallpaper_dir() {
        let c = parse("wallpaper_dir = \"D:/Pics\"
[snippets]
gmail = \"https://mail.google.com\"
");
        assert_eq!(c.wallpaper_dir.as_deref(), Some("D:/Pics"));
        assert_eq!(c.snippets, vec![("gmail".to_string(), "https://mail.google.com".to_string())]);
    }

    #[test]
    fn live_flags() {
        assert!(Config::default().live_media && Config::default().live_toasts);
        let c = parse("live_media = 0\nlive_toasts = false\n");
        assert!(!c.live_media && !c.live_toasts);
        let t = parse(DEFAULT_TEMPLATE);
        assert!(t.live_media && t.live_toasts);
    }

    #[test]
    fn bad_lines_are_ignored() {
        let c = parse("accent = \"zzz\"\nnonsense\nleave_delay_ms = \"x\"\ntabs = [\"A\" \"B\"]\n");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn appearance_keys_parse() {
        let c = parse("bg = \"#102030\"\nsystem_colors = 1\ncorner_radius = 99\nhidden_tabs = [\"Power\", \"stats\"]\nidle_face = 0\n");
        assert!((c.bg[2] - 0x30 as f32 / 255.0).abs() < 0.001);
        assert!(c.system_colors && !c.idle_face);
        assert_eq!(c.corner_radius, MAX_RADIUS);
        assert_eq!(c.hidden_tabs, vec!["Power".to_string(), "stats".to_string()]);
        let d = parse(DEFAULT_TEMPLATE);
        assert_eq!((d.corner_radius, d.system_colors, d.idle_face), (DEFAULT_RADIUS, false, true));
        assert!(d.hidden_tabs.is_empty());
    }

    #[test]
    fn visible_tabs_skip_hidden_but_keep_one() {
        let mut c = Config::default();
        c.tabs = vec!["C".into(), "A".into()];
        c.hidden_tabs = vec!["a".into()];
        assert_eq!(c.resolve_visible(&NAMES), vec![2, 1, 3]);
        c.hidden_tabs = NAMES.iter().map(|s| s.to_string()).collect();
        assert_eq!(c.resolve_visible(&NAMES), vec![2], "скрыть все вкладки нельзя");
    }

    #[test]
    fn set_keys_replaces_and_inserts_without_touching_the_rest() {
        let src = "# заголовок\naccent = \"#000000\" # старый\n\n# live\nlive_toasts = 1\n\n# ключи\n[keys]\nweather = \"\"\n";
        let out = set_keys(src, &[("accent", "\"#FFFFFF\"".into()), ("bg", "\"#101010\"".into())]);
        assert!(out.contains("accent = \"#FFFFFF\"\n"));
        assert!(!out.contains("старый"), "строка заменяется целиком");
        // Новый ключ — в верхнем уровне, до секции и её комментария.
        let bg = out.find("bg = ").unwrap();
        assert!(bg > out.find("live_toasts").unwrap() && bg < out.find("# ключи").unwrap(), "{out}");
        assert!(out.contains("# заголовок") && out.contains("[keys]\nweather = \"\""));
        // Значение с тем же именем в секции не трогается.
        let out = set_keys("[keys]\nbg = \"x\"\n", &[("bg", "\"#111111\"".into())]);
        assert!(out.starts_with("bg = \"#111111\"\n[keys]") || out.contains("bg = \"x\""), "{out}");
        assert!(out.contains("bg = \"x\""));
    }

    #[test]
    fn set_keys_keeps_crlf_and_bom_and_roundtrips() {
        let out = set_keys("\u{feff}accent = \"#000000\"\r\ntabs = [\"A\"]\r\n", &[("tabs", "[\"B\", \"A\"]".into())]);
        assert!(out.starts_with('\u{feff}') && out.contains("\r\n") && !out.replace("\r\n", "").contains('\n'));
        assert_eq!(parse(&out).tabs, vec!["B".to_string(), "A".to_string()]);
        // Ключи в конец файла, если секций нет.
        let out = set_keys("accent = \"#000000\"\n", &[("corner_radius", "20".into())]);
        assert_eq!(out, "accent = \"#000000\"\ncorner_radius = 20\n");
    }

    #[test]
    fn saved_appearance_roundtrips_through_the_template() {
        let mut c = parse(DEFAULT_TEMPLATE);
        c.accent = theme::parse_hex("#3A86FF").unwrap();
        c.bg = theme::parse_hex("#101820").unwrap();
        c.system_colors = true;
        c.corner_radius = 22.0;
        c.hidden_tabs = vec!["Power".into(), "Wallpaper".into()];
        c.tabs = vec!["Apps".into(), "Controls".into()];
        c.idle_face = false;
        let updates = [
            ("accent", fmt_str(&theme::to_hex(c.accent))),
            ("bg", fmt_str(&theme::to_hex(c.bg))),
            ("system_colors", "1".to_string()),
            ("corner_radius", "22".to_string()),
            ("tabs", fmt_list(&c.tabs)),
            ("hidden_tabs", fmt_list(&c.hidden_tabs)),
            ("idle_face", "0".to_string()),
        ];
        let back = parse(&set_keys(DEFAULT_TEMPLATE, &updates));
        assert_eq!(back, c);
    }

    #[test]
    fn tab_order() {
        let mut c = Config::default();
        c.tabs = vec!["c".into(), "zzz".into(), "A".into(), "c".into()];
        assert_eq!(c.resolve_order(&NAMES), [2, 0, 1, 3]);
        c.tabs.clear();
        assert_eq!(c.resolve_order(&NAMES), [0, 1, 2, 3]);
    }
}
