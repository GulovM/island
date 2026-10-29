//! Конфиг `%APPDATA%\island\config.toml`. Своя мини-реализация TOML (ключи, секции,
//! строки, числа, массивы строк) — отдельная зависимость ради десятка строк не нужна.

use std::path::PathBuf;

use crate::log;
use crate::theme;

pub use crate::ios::Material;
pub use crate::theme::PaletteStyle;

/// Скругление нижних углов по умолчанию и допустимые пределы, логические пиксели.
pub const DEFAULT_RADIUS: f32 = 14.0;
pub const MAX_RADIUS: f32 = 30.0;
/// Прозрачность стекла по умолчанию: 0 — почти прозрачное, 100 — сплошное тело.
pub const DEFAULT_GLASS_OPACITY: f32 = 70.0;
/// Масштаб интерфейса (шрифты и панель вместе): по умолчанию и пределы.
pub const DEFAULT_SCALE: f32 = 1.2;
pub const MIN_SCALE: f32 = 0.9;
pub const MAX_SCALE: f32 = 2.0;

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

# 1 — эффект стекла у раскрытого острова (включённое стекло iOS 26): размытый фон просвечивает сквозь тело,
# на кромке блик, сверху мягкое осветление, у выбранной вкладки подвижная «капля». 0 — прежний плоский вид.
# Под телом размытый фон; если в Windows выключены «Эффекты прозрачности»,
# размытия нет и тело плотнее. Меняется и в Настройках.
glass = 1

# Прозрачность стекла: 0 — самое прозрачное из допустимого (чёрная подкладка 41% поверх размытия),
# 100 — сплошное тело (0–100). Меняется и в Настройках.
glass_opacity = 70

# Материал стеклянного тела острова: "ultrathin", "thin", "regular", "thick", "chrome".
# Меняется и в Настройках.
material = "regular"

# Стиль палитры: "windows" (цвета системы/пользовательские) или "macos" (системные цвета iOS 26). Прежнее "ios" тоже принимается.
# Меняется и в Настройках.
palette_style = "windows"

# 1 — капля выбора вкладки залита акцентом (как до стекла); 0 — нейтральная плашка. Меняется и в Настройках.
accent_pill = 0

# Масштаб интерфейса: шрифты и панель увеличиваются вместе (0.9–2.0). Меняется и в Настройках.
ui_scale = 1.2

# Язык интерфейса: "auto" (как в Windows), "ru" или "en". Меняется и в Настройках.
language = "auto"

# 1 — над развёрнутыми на весь экран окнами остров сжимается в тонкую полоску
# (раскрывается при наведении на верхний край), чтобы не закрывать вкладки браузера
strip_over_maximized = 1

# Режим свёрнутого острова: "auto" — обычная чёлка, полоска только над окнами на весь экран
# (если включено выше); "strip" — всегда тонкая полоска, без мини-плеера и времени.
# Меняется и в Настройках.
collapsed_mode = "auto"

# Скрытые вкладки (список имён из tabs ниже)
hidden_tabs = []

# 1 — в свёрнутом острове без музыки показывать время, раскладку клавиатуры и Wi-Fi
idle_face = 1

# 1 — выполненные задачи в списке дел убираются сами с наступлением нового дня; 0 — не убираются.
# Меняется и в Настройках.
todo_clean_done = 0

# 1 — найденные обновления скачиваются и ставятся сами (проверка — раз в сутки);
# 0 — остров только сообщает об обновлении. Меняется и в Настройках.
auto_update = 0

# 1 — по PrtSc и Win+Shift+S остров прячется на время съёмки, чтобы не попасть в кадр;
# 0 — остров остаётся на экране и виден на скриншотах. Меняется и в Настройках.
hide_on_screenshot = 1

# Через сколько мс после ухода курсора остров сворачивается
leave_delay_ms = 220

# Как часто (мс) проверять, не развернули ли окно на весь экран
env_poll_ms = 400

# Размер свёрнутого острова, логические пиксели
collapsed_width = 180
collapsed_height = 24

# Порядок вкладок. Пропущенные добавляются в конец.
tabs = ["Controls", "Today", "Stats", "Clipboard", "Apps", "Wallpaper", "Power"]

# Собственные вкладки: "номер|название|раскладка". Их удобнее создавать и править в настройках.
# В порядке и в hidden_tabs такая вкладка называется custom:<номер>.
custom_tabs = []

# Глобальная горячая клавиша: раскрывает остров на вкладке Apps с фокусом в поиске.
# Модификаторы ctrl/alt/shift/win и клавиша (буква, цифра, space, enter, f1…f12); "" — отключить
hotkey = "ctrl+alt+space"

# Глобальная горячая клавиша раздела «Быстрый доступ» на вкладке «Буфер». Обычно её назначают
# прямо в разделе (кнопка «Назначить горячую клавишу»); "" — не назначена
quick_hotkey = ""

# Папка с обоями для вкладки Wallpaper (по умолчанию «Изображения» пользователя)
# wallpaper_dir = "D:/Pictures/Wallpapers"

# Город для погоды на вкладке Today (Open-Meteo, ключ не нужен), например "Москва".
# Пусто — город определяется по IP. Меняется и прямо в острове: клик по карточке погоды.
weather_city = ""

# Циферблат часов на вкладке Today, 0–4 (переключается стрелками на карточке часов).
clock_style = 0

# Календарь: «секретный адрес в формате iCal» из настроек Google Calendar (или любой .ics).
# Поддерживается одна ссылка: calendar_ics = "https://..."
# или список ссылок: calendar_ics = ["https://...", "https://..."]
calendar_ics = []

# Раскладка виджетов вкладок. Меняется прямо в острове: кнопка с карандашом справа в ряду вкладок
# (рядом с шестерёнкой, или долгое нажатие на пустом месте вкладки). Пусто — раскладка по умолчанию.
# Формат: метка версии «v2», затем виджеты через пробел, «-» — скрыт, «:N» — номер стандартного размера,
# «@col,row» — крупная ячейка сетки, например "v2 wifi:0@0,0 -night". Строки старого формата (без «v2»,
# мелкая сетка) приводятся к ближайшим стандартным размерам, а совсем старые (@col) сбрасываются.
# Остров автоматически подгоняется под видимые виджеты по «Готово»; в режиме правки ручки по краям позволяют растянуть сетку обратно.
layout_controls = ""
layout_today = ""
layout_stats = ""

# 0 — виджеты сами уплотняются, пустот в раскладке нет (как в Пункте управления iOS);
# 1 — свободная раскладка: пустые места между виджетами допустимы. Меняется и в Настройках.
free_layout = 0

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

/// Собственная вкладка пользователя: стабильный номер, название и раскладка (формат `tabs::layout`).
#[derive(Clone, Debug, PartialEq)]
pub struct CustomTab {
    pub id: u32,
    pub name: String,
    pub layout: String,
}

/// Самая длинная подпись собственной вкладки, символов.
pub const MAX_TAB_NAME: usize = 16;

impl CustomTab {
    /// Строка для `custom_tabs`: `номер|название|раскладка`.
    fn encode(&self) -> String {
        format!("{}|{}|{}", self.id, self.name, self.layout)
    }

    fn decode(s: &str) -> Option<CustomTab> {
        let mut it = s.splitn(3, '|');
        let id = it.next()?.trim().parse().ok()?;
        let name = clean_tab_name(it.next()?);
        if name.is_empty() {
            return None;
        }
        Some(CustomTab { id, name, layout: it.next().unwrap_or("").trim().to_string() })
    }
}

/// Название вкладки без служебных символов (`|`, кавычки, управляющие) и не длиннее `MAX_TAB_NAME`.
pub fn clean_tab_name(s: &str) -> String {
    let cleaned: String = s.chars().map(|c| if c.is_control() || matches!(c, '|' | '"' | '\\') { ' ' } else { c }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_TAB_NAME).collect::<String>().trim().to_string()
}

/// Сколько вкладок может быть включено одновременно (встроенные и собственные вместе), чтобы ряд поместился.
pub const MAX_ENABLED_TABS: usize = 8;

/// Номер вкладки и её ключ в `tabs` / `hidden_tabs`: имя встроенной вкладки или `custom:<id>`.
pub type TabKey = (usize, String);

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub accent: [f32; 3],
    /// Цвет фона острова.
    pub bg: [f32; 3],
    /// Цвета берутся из Windows (акцент и палитра системы), `accent` и `bg` не используются.
    pub system_colors: bool,
    /// Скругление нижних углов раскрытого острова, логические пиксели.
    pub corner_radius: f32,
    /// Эффект стекла: блик на кромке, осветление сверху, подвижная капля вкладки.
    pub glass: bool,
    /// Прозрачность стекла, 0–100: 0 — почти прозрачное, 100 — сплошное тело.
    pub glass_opacity: f32,
    /// Материал стеклянного тела острова (iOS 26 Liquid Glass).
    pub material: Material,
    /// Стиль палитры: Windows (из системы/пользовательские) или iOS (системные цвета iOS 26).
    pub palette_style: PaletteStyle,
    /// 1 — капля выбора вкладки залита акцентом, 0 — нейтральная плашка.
    pub accent_pill: bool,
    /// Масштаб интерфейса (множитель к DPI монитора).
    pub ui_scale: f32,
    /// Над развёрнутыми окнами остров сжимается в тонкую полоску.
    pub strip_over_maximized: bool,
    /// Свёрнутый остров всегда тонкая полоска (`collapsed_mode = "strip"`).
    pub always_strip: bool,
    /// `auto`, `ru` или `en`.
    pub language: String,
    /// Имена скрытых вкладок.
    pub hidden_tabs: Vec<String>,
    /// Время, раскладка и Wi-Fi на свёрнутом острове, когда музыка не играет.
    pub idle_face: bool,
    /// Убирать выполненные задачи при смене дня.
    pub todo_clean_done: bool,
    /// Устанавливать найденные обновления без вопросов.
    pub auto_update: bool,
    /// Прятать остров на время скриншота (PrtSc, Win+Shift+S).
    pub hide_on_screenshot: bool,
    pub leave_delay_ms: u32,
    pub env_poll_ms: u32,
    pub collapsed: (f32, f32),
    pub tabs: Vec<String>,
    /// Собственные вкладки; номер вкладки на острове — `tabs::CUSTOM_BASE` плюс позиция в списке.
    pub custom: Vec<CustomTab>,
    pub keys: Vec<(String, String)>,
    /// Папка с обоями; `None` — «Изображения» пользователя.
    pub wallpaper_dir: Option<String>,
    /// Быстрые ссылки для вкладки Apps: (имя, адрес).
    pub snippets: Vec<(String, String)>,
    /// Город для погоды; пустой — погода не запрашивается.
    pub weather_city: String,
    /// Номер циферблата часов на вкладке Today.
    pub clock_style: usize,
    /// Ссылки на календари в формате iCal (до 8 штук).
    pub calendar_ics: Vec<String>,
    /// Раскладка виджетов вкладок Controls, Today, Stats (пустая строка — по умолчанию).
    pub layouts: [String; 3],
    /// Свободная раскладка виджетов: пустоты допустимы. По умолчанию (`false`) раскладка уплотняется.
    pub free_layout: bool,
    /// Глобальная горячая клавиша («ctrl+alt+space»); пустая — отключена.
    pub hotkey: String,
    /// Горячая клавиша раздела «Быстрый доступ»; пустая — не назначена.
    pub quick_hotkey: String,
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
            glass: true,
            glass_opacity: DEFAULT_GLASS_OPACITY,
            material: Material::Regular,
            palette_style: PaletteStyle::Windows,
            accent_pill: false,
            ui_scale: DEFAULT_SCALE,
            strip_over_maximized: true,
            always_strip: false,
            language: "auto".to_string(),
            hidden_tabs: Vec::new(),
            idle_face: true,
            todo_clean_done: false,
            auto_update: false,
            hide_on_screenshot: true,
            leave_delay_ms: 220,
            env_poll_ms: 400,
            collapsed: (180.0, 24.0),
            tabs: Vec::new(),
            custom: Vec::new(),
            keys: Vec::new(),
            wallpaper_dir: None,
            snippets: Vec::new(),
            weather_city: String::new(),
            clock_style: 0,
            calendar_ics: Vec::new(),
            layouts: Default::default(),
            free_layout: false,
            hotkey: "ctrl+alt+space".into(),
            quick_hotkey: String::new(),
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

    /// Номер для новой собственной вкладки: больше всех существующих.
    pub fn next_custom_id(&self) -> u32 {
        self.custom.iter().map(|c| c.id).max().map_or(1, |m| m + 1)
    }

    /// Ключи всех вкладок: сначала встроенные, затем собственные в порядке списка.
    pub fn tab_keys(&self) -> Vec<TabKey> {
        let builtin = crate::TABS.iter().enumerate().map(|(i, n)| (i, n.to_string()));
        let custom = self.custom.iter().enumerate().map(|(i, c)| (crate::tabs::CUSTOM_BASE + i, format!("custom:{}", c.id)));
        builtin.chain(custom).collect()
    }

    /// Ключ вкладки в конфиге: имя встроенной или `custom:<id>`; пусто у страницы настроек и чужих номеров.
    pub fn tab_key(&self, tab: usize) -> String {
        self.tab_keys().into_iter().find(|(t, _)| *t == tab).map(|(_, k)| k).unwrap_or_default()
    }

    /// Название вкладки для показа в ряду и в настройках.
    pub fn tab_label(&self, tab: usize) -> String {
        match tab.checked_sub(crate::tabs::CUSTOM_BASE) {
            Some(i) => self.custom.get(i).map(|c| c.name.clone()).unwrap_or_default(),
            None => crate::i18n::tab_name(tab).to_string(),
        }
    }

    /// Все вкладки в порядке показа, включая выключенные.
    pub fn ordered_tabs(&self) -> Vec<usize> {
        self.resolve_order(&self.tab_keys())
    }

    /// Включённые вкладки в порядке показа.
    pub fn visible_tabs(&self) -> Vec<usize> {
        self.resolve_visible(&self.tab_keys())
    }

    /// Порядок вкладок: номера вкладок по позициям. Неизвестные ключи игнорируются,
    /// повторы и пропущенные вкладки дописываются в исходном порядке.
    pub fn resolve_order(&self, keys: &[TabKey]) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::with_capacity(keys.len());
        for want in &self.tabs {
            if let Some((tab, _)) = keys.iter().find(|(_, k)| k.eq_ignore_ascii_case(want)) {
                if !out.contains(tab) {
                    out.push(*tab);
                }
            }
        }
        let rest: Vec<usize> = keys.iter().map(|(t, _)| *t).filter(|t| !out.contains(t)).collect();
        out.extend(rest);
        out
    }

    /// Вкладки, которые видны на острове, в порядке показа: полный порядок без скрытых, но не больше
    /// `MAX_ENABLED_TABS` (лишние в конце порядка считаются выключенными).
    /// Хотя бы одна вкладка остаётся всегда (если скрыты все — первая по порядку).
    pub fn resolve_visible(&self, keys: &[TabKey]) -> Vec<usize> {
        let order = self.resolve_order(keys);
        let hidden: Vec<usize> =
            self.hidden_tabs.iter().filter_map(|h| keys.iter().find(|(_, k)| k.eq_ignore_ascii_case(h)).map(|(t, _)| *t)).collect();
        let mut out: Vec<usize> = order.iter().copied().filter(|t| !hidden.contains(t)).take(MAX_ENABLED_TABS).collect();
        if out.is_empty() {
            out.push(order[0]);
        }
        out
    }
}

/// Записывает настройки внешнего вида обратно в `config.toml`, не трогая остальные строки и комментарии.
pub fn save(cfg: &Config) {
    write_updates(&save_updates(cfg));
}

fn save_updates(cfg: &Config) -> Vec<(&'static str, String)> {
    let order: Vec<String> = cfg.tabs.iter().map(|t| t.to_string()).collect();
    let order = if order.is_empty() { cfg.tab_keys().into_iter().map(|(_, k)| k).collect() } else { order };
    let updates = [
        ("accent", fmt_str(&theme::to_hex(cfg.accent))),
        ("bg", fmt_str(&theme::to_hex(cfg.bg))),
        ("system_colors", (cfg.system_colors as u8).to_string()),
        ("corner_radius", format!("{}", cfg.corner_radius.round() as i32)),
        ("glass", (cfg.glass as u8).to_string()),
        ("glass_opacity", format!("{}", cfg.glass_opacity.round() as i32)),
        ("material", fmt_str(cfg.material.as_str())),
        ("palette_style", fmt_str(cfg.palette_style.as_str())),
        ("accent_pill", (cfg.accent_pill as u8).to_string()),
        ("ui_scale", format!("{:.3}", cfg.ui_scale)),
        ("strip_over_maximized", (cfg.strip_over_maximized as u8).to_string()),
        ("collapsed_mode", fmt_str(if cfg.always_strip { "strip" } else { "auto" })),
        ("language", fmt_str(&cfg.language)),
        ("tabs", fmt_list(&order)),
        ("hidden_tabs", fmt_list(&cfg.hidden_tabs)),
        ("custom_tabs", fmt_list(&cfg.custom.iter().map(CustomTab::encode).collect::<Vec<_>>())),
        ("idle_face", (cfg.idle_face as u8).to_string()),
        ("todo_clean_done", (cfg.todo_clean_done as u8).to_string()),
        ("auto_update", (cfg.auto_update as u8).to_string()),
        ("free_layout", (cfg.free_layout as u8).to_string()),
        ("hide_on_screenshot", (cfg.hide_on_screenshot as u8).to_string()),
    ];
    updates.to_vec()
}

/// Ключи раскладки виджетов в `config.toml` по номеру вкладки (Controls, Today, Stats).
pub const LAYOUT_KEYS: [&str; 3] = ["layout_controls", "layout_today", "layout_stats"];

/// Записывает одну строку верхнего уровня (`key = "value"`) в `config.toml` (город погоды из острова).
pub fn save_str(key: &str, value: &str) {
    write_updates(&[(key, fmt_str(value))]);
}

/// Записывает одно число верхнего уровня (`key = 3`) в `config.toml`.
pub fn save_num(key: &str, value: usize) {
    write_updates(&[(key, value.to_string())]);
}

/// Максимальное число ссылок на календари.
pub const MAX_CALENDAR_URLS: usize = 8;

/// Проверяет, является ли строка корректным адресом календаря (http://, https:// или webcal://).
pub fn valid_calendar_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() || u.contains(char::is_whitespace) {
        return false;
    }
    ["https://", "http://", "webcal://"].iter().any(|p| u.starts_with(p))
}

/// Нормализует список адресов: убирает недопустимые, удаляет дубликаты, ограничивает лимитом `MAX_CALENDAR_URLS`.
pub fn normalize_calendar_urls(raw_urls: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for u in raw_urls {
        let trimmed = u.trim();
        if !valid_calendar_url(trimmed) {
            continue;
        }
        if !out.iter().any(|existing: &String| existing == trimmed) {
            out.push(trimmed.to_string());
            if out.len() >= MAX_CALENDAR_URLS {
                break;
            }
        }
    }
    out
}

/// Записывает список ссылок на календари в `config.toml`.
pub fn save_calendar_ics(urls: &[String]) {
    write_updates(&[("calendar_ics", fmt_list(urls))]);
}

fn write_updates(updates: &[(&str, String)]) {
    if LOAD_FAILED.load(std::sync::atomic::Ordering::Relaxed) {
        // Файл не читался при запуске: пробуем ещё раз, и только если прочитался — пишем.
        match read_with_retry(&path()) {
            Ok(_) => LOAD_FAILED.store(false, std::sync::atomic::Ordering::Relaxed),
            Err(_) => return,
        }
    }
    let path = path();
    // Шаблон подставляем только если файла нет; любая другая ошибка чтения (файл открыт в редакторе,
    // антивирус) не должна приводить к перезаписи пользовательского конфига шаблоном.
    let text = match read_with_retry(&path) {
        Ok(b) => decode(&b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_TEMPLATE.to_string(),
        Err(e) => {
            log::error(&format!("не удалось прочитать {} для сохранения: {e}", path.display()));
            return;
        }
    };
    let out = set_keys(&text, updates);
    write_text(&path, &out, &text);
}

/// Атомарная запись: через временный файл; если переименование не вышло — прямая запись поверх.
fn write_text(path: &std::path::Path, out: &str, old: &str) {
    if out == old {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("toml.tmp");
    let atomic = std::fs::write(&tmp, out).and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = atomic {
        let _ = std::fs::remove_file(&tmp);
        log::error(&format!("атомарная запись {} не удалась ({e}), пишу напрямую", path.display()));
        if let Err(e) = std::fs::write(path, out) {
            log::error(&format!("не удалось записать {}: {e}", path.display()));
        }
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

/// Конфиг не удалось прочитать (не «нет файла»): пока так, настройки не пишем — иначе значения по умолчанию
/// затёрли бы пользовательский файл. Сбрасывается при успешном чтении.
static LOAD_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Читает файл целиком; UTF-16 (Блокнот) приводит к UTF-8, остальное читает с заменой битых байтов.
fn decode(b: &[u8]) -> String {
    let utf16 = |b: &[u8], le: bool| {
        let u: Vec<u16> = b.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&u)
    };
    match b {
        [0xff, 0xfe, rest @ ..] => utf16(rest, true),
        [0xfe, 0xff, rest @ ..] => utf16(rest, false),
        _ => String::from_utf8_lossy(b).into_owned(),
    }
}

/// Чтение с повторами: при автозапуске файл может быть на секунды занят антивирусом или ещё не доступен.
fn read_with_retry(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let mut last = None;
    for attempt in 0..6 {
        match std::fs::read(path) {
            Ok(b) => return Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(e),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(150 * (attempt + 1)));
            }
        }
    }
    Err(last.unwrap())
}

fn backup_path() -> PathBuf {
    path().with_extension("toml.bak")
}

/// Читает конфиг; если файла нет — создаёт с шаблоном. Любые ошибки — в лог; нечитаемый файл не перезаписываем.
/// Недостающие ключи (после обновления) дописываются в файл, так что `config.toml` всегда полный.
pub fn load() -> Config {
    use std::sync::atomic::Ordering::Relaxed;
    let path = path();
    match read_with_retry(&path) {
        Ok(b) => {
            LOAD_FAILED.store(false, Relaxed);
            let text = decode(&b);
            if !text.trim().is_empty() {
                let _ = std::fs::write(backup_path(), &b);
            }
            let cfg = parse(&text);
            migrate(&path, &text, &cfg);
            cfg
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            LOAD_FAILED.store(false, Relaxed);
            // Файл пропал — пробуем последнюю копию, иначе шаблон.
            let text = std::fs::read(backup_path()).map(|b| decode(&b)).unwrap_or_else(|_| DEFAULT_TEMPLATE.to_string());
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, &text) {
                log::error(&format!("не удалось создать {}: {e}", path.display()));
            }
            parse(&text)
        }
        Err(e) => {
            log::error(&format!("не удалось прочитать {}: {e}", path.display()));
            LOAD_FAILED.store(true, Relaxed);
            // Читаем копию, но файл не трогаем.
            std::fs::read(backup_path()).map(|b| parse(&decode(&b))).unwrap_or_default()
        }
    }
}

/// Дописывает в файл ключи, которых в нём нет (новые версии добавляют настройки), значениями из `cfg`.
fn migrate(path: &std::path::Path, text: &str, cfg: &Config) {
    let tmpl = DEFAULT_TEMPLATE.lines().filter_map(|l| {
        let t = l.trim_start();
        if t.starts_with('#') || t.starts_with('[') { None } else { t.split_once('=').map(|(k, _)| k.trim().to_string()) }
    });
    let top: Vec<&str> = text.lines().take_while(|l| !l.trim_start().starts_with('[')).collect();
    let present = |k: &str| top.iter().any(|l| !l.trim_start().starts_with('#') && l.split_once('=').is_some_and(|(a, _)| a.trim() == k));
    let appearance = [
        "accent",
        "bg",
        "system_colors",
        "corner_radius",
        "glass",
        "glass_opacity",
        "material",
        "palette_style",
        "accent_pill",
        "ui_scale",
        "strip_over_maximized",
        "collapsed_mode",
        "language",
        "tabs",
        "hidden_tabs",
        "custom_tabs",
        "idle_face",
        "todo_clean_done",
        "auto_update",
        "free_layout",
        "hide_on_screenshot",
    ];
    if tmpl.filter(|k| appearance.contains(&k.as_str())).all(|k| present(&k)) {
        return;
    }
    // Недостающие ключи берём из разобранных настроек (они уже учитывают значения по умолчанию).
    let missing_only = save_updates(cfg).into_iter().filter(|(k, _)| !present(k)).collect::<Vec<_>>();
    if !missing_only.is_empty() {
        write_text(path, &set_keys(text, &missing_only), text);
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
        // Ключ верхнего уровня, дописанный под `[keys]`, принадлежит секции и молча терялся.
        let misplaced = section == "keys" && matches!(key, "calendar_ics" | "weather_city");
        if misplaced {
            log::error(&format!("config.toml:{}: «{key}» стоит в секции [keys], перенесите выше неё", n + 1));
        }
        let sect = if misplaced { "" } else { section.as_str() };
        match (sect, key, value) {
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
            ("", "todo_clean_done", Value::Num(v)) => cfg.todo_clean_done = v != 0.0,
            ("", "auto_update", Value::Num(v)) => cfg.auto_update = v != 0.0,
            ("", "free_layout", Value::Num(v)) => cfg.free_layout = v != 0.0,
            ("", "hide_on_screenshot", Value::Num(v)) => cfg.hide_on_screenshot = v != 0.0,
            ("", "corner_radius", Value::Num(v)) => cfg.corner_radius = (v as f32).clamp(0.0, MAX_RADIUS),
            ("", "glass", Value::Num(v)) => cfg.glass = v != 0.0,
            ("", "glass_opacity", Value::Num(v)) => cfg.glass_opacity = (v as f32).clamp(0.0, 100.0),
            ("", "material", Value::Str(s)) => cfg.material = Material::parse(&s),
            ("", "material", _) => cfg.material = Material::Regular,
            ("", "palette_style", Value::Str(s)) => cfg.palette_style = PaletteStyle::parse(&s),
            ("", "palette_style", _) => cfg.palette_style = PaletteStyle::Windows,
            // Ключ из 0.5.0: настройка капли убрана, строку в старых конфигах молча пропускаем.
            // Ключ тестовых сборок с подложкой размытия: больше ничего не значит, молча пропускаем.
            ("", "glass_hint_dismissed", _) => {}
            ("", "accent_pill", Value::Num(v)) => cfg.accent_pill = v != 0.0,
            ("", "ui_scale", Value::Num(v)) => cfg.ui_scale = (v as f32).clamp(MIN_SCALE, MAX_SCALE),
            ("", "strip_over_maximized", Value::Num(v)) => cfg.strip_over_maximized = v != 0.0,
            ("", "collapsed_mode", Value::Str(s)) => cfg.always_strip = s.trim().eq_ignore_ascii_case("strip"),
            ("", "language", Value::Str(s)) => {
                cfg.language = match s.trim().to_lowercase().as_str() {
                    l @ ("ru" | "en") => l.to_string(),
                    _ => "auto".to_string(),
                }
            }
            ("", "hidden_tabs", Value::List(l)) => cfg.hidden_tabs = l,
            ("", "leave_delay_ms", Value::Num(v)) => cfg.leave_delay_ms = v.clamp(0.0, 5000.0) as u32,
            ("", "env_poll_ms", Value::Num(v)) => cfg.env_poll_ms = v.clamp(100.0, 10_000.0) as u32,
            ("", "collapsed_width", Value::Num(v)) => cfg.collapsed.0 = v.clamp(60.0, 400.0) as f32,
            ("", "collapsed_height", Value::Num(v)) => cfg.collapsed.1 = v.clamp(8.0, 60.0) as f32,
            ("", "live_media", Value::Num(v)) => cfg.live_media = v != 0.0,
            ("", "live_toasts", Value::Num(v)) => cfg.live_toasts = v != 0.0,
            ("", "tabs", Value::List(l)) => cfg.tabs = l,
            ("", "custom_tabs", Value::List(l)) => {
                for c in l.iter().filter_map(|s| CustomTab::decode(s)) {
                    if !cfg.custom.iter().any(|o| o.id == c.id) {
                        cfg.custom.push(c);
                    }
                }
            }
            ("", "weather_city", Value::Str(s)) => cfg.weather_city = s,
            ("", "clock_style", Value::Num(v)) => cfg.clock_style = (v.max(0.0) as usize).min(4),
            ("", "calendar_ics", Value::Str(s)) => {
                let urls: Vec<String> = s.split_whitespace().map(|x| x.to_string()).collect();
                cfg.calendar_ics = normalize_calendar_urls(&urls);
            }
            ("", "calendar_ics", Value::List(l)) => {
                cfg.calendar_ics = normalize_calendar_urls(&l);
            }
            ("", "layout_controls", Value::Str(s)) => cfg.layouts[0] = s,
            ("", "layout_today", Value::Str(s)) => cfg.layouts[1] = s,
            ("", "layout_stats", Value::Str(s)) => cfg.layouts[2] = s,
            ("", "hotkey", Value::Str(s)) => {
                if !s.is_empty() && parse_hotkey(&s).is_none() {
                    log::error(&format!("config.toml:{}: неверная горячая клавиша «{s}»", n + 1));
                }
                cfg.hotkey = s;
            }
            ("", "quick_hotkey", Value::Str(s)) => {
                if !s.is_empty() && parse_hotkey(&s).is_none() {
                    log::error(&format!("config.toml:{}: неверная горячая клавиша «{s}»", n + 1));
                }
                cfg.quick_hotkey = s;
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
    s.parse::<f64>().ok().filter(|v| v.is_finite()).map(Value::Num)
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

    fn keys() -> Vec<TabKey> {
        NAMES.iter().enumerate().map(|(i, n)| (i, n.to_string())).collect()
    }

    #[test]
    fn template_matches_defaults() {
        let c = parse(DEFAULT_TEMPLATE);
        assert!((c.accent[0] - 0.83).abs() < 0.01 && (c.accent[2] - 0.37).abs() < 0.01);
        assert_eq!((c.leave_delay_ms, c.env_poll_ms), (220, 400));
        assert_eq!(c.collapsed, (180.0, 24.0));
        assert_eq!(c.tabs.len(), 7);
        assert_eq!(c.key("weather"), None);
        assert!(c.glass);
        assert!(!c.accent_pill);
        assert_eq!(c.palette_style, PaletteStyle::Windows);
        assert_eq!(c.material, Material::Regular);
        assert_eq!(
            (Config::default().glass, Config::default().glass_opacity, Config::default().accent_pill, Config::default().palette_style, Config::default().material),
            (c.glass, c.glass_opacity, c.accent_pill, c.palette_style, c.material)
        );
        assert_eq!(c.glass_opacity, DEFAULT_GLASS_OPACITY);
    }

    #[test]
    fn obsolete_glass_hint_key_is_ignored() {
        // Старый ключ из тестовых сборок не ломает разбор и не попадает обратно в файл.
        assert_eq!(parse("glass_hint_dismissed = 1\n"), Config::default());
        assert!(!DEFAULT_TEMPLATE.contains("glass_hint_dismissed"));
        assert!(save_updates(&Config::default()).iter().all(|(k, _)| *k != "glass_hint_dismissed"));
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
    fn top_level_keys_under_keys_section_still_apply() {
        let c = parse("[keys]\nweather = \"x\"\n\ncalendar_ics = \"https://a.b/c.ics\"\nweather_city = \"Душанбе\"\n");
        assert_eq!(c.calendar_ics, vec!["https://a.b/c.ics".to_string()]);
        assert_eq!(c.weather_city, "Душанбе");
        assert_eq!(c.key("weather"), Some("x"));
        assert_eq!(c.key("calendar_ics"), None);
    }

    #[test]
    fn calendar_url_validation() {
        assert!(valid_calendar_url("https://calendar.google.com/x.ics"));
        assert!(valid_calendar_url("http://example.com/cal.ics"));
        assert!(valid_calendar_url("webcal://example.com/feed.ics"));
        assert!(!valid_calendar_url(""));
        assert!(!valid_calendar_url("   "));
        assert!(!valid_calendar_url("ftp://example.com/cal.ics"));
        assert!(!valid_calendar_url("file:///C:/test.ics"));
        assert!(!valid_calendar_url("not-a-url"));
        assert!(!valid_calendar_url("https://cal.com/a.ics https://cal.com/b.ics"));
    }

    #[test]
    fn calendar_ics_normalization_and_limits() {
        let raw = vec![
            "https://a.com/1.ics".to_string(),
            "  https://a.com/1.ics  ".to_string(), // дубликат после trim
            "invalid-url".to_string(),
            "".to_string(),
            "webcal://b.com/2.ics".to_string(),
            "http://c.com/3.ics".to_string(),
            "https://d.com/4.ics".to_string(),
            "https://e.com/5.ics".to_string(),
            "https://f.com/6.ics".to_string(),
            "https://g.com/7.ics".to_string(),
            "https://h.com/8.ics".to_string(),
            "https://i.com/9.ics".to_string(), // превышение лимита MAX_CALENDAR_URLS (8)
        ];
        let norm = normalize_calendar_urls(&raw);
        assert_eq!(norm.len(), 8);
        assert_eq!(norm[0], "https://a.com/1.ics");
        assert_eq!(norm[1], "webcal://b.com/2.ics");
        assert_eq!(norm[2], "http://c.com/3.ics");
        assert_eq!(norm[7], "https://h.com/8.ics");
    }

    #[test]
    fn calendar_ics_parsing_legacy_and_new_formats() {
        // Старый формат: одна строка
        let c1 = parse("calendar_ics = \"https://google.com/cal.ics\"\n");
        assert_eq!(c1.calendar_ics, vec!["https://google.com/cal.ics".to_string()]);

        // Пустая строка
        let c2 = parse("calendar_ics = \"\"\n");
        assert!(c2.calendar_ics.is_empty());

        // Новый формат: массив строк
        let c3 = parse("calendar_ics = [\"https://a.com/1.ics\", \"webcal://b.com/2.ics\"]\n");
        assert_eq!(
            c3.calendar_ics,
            vec!["https://a.com/1.ics".to_string(), "webcal://b.com/2.ics".to_string()]
        );

        // Строка с несколькими ссылками через пробел/перевод строки
        let c4 = parse("calendar_ics = \"https://a.com/1.ics   webcal://b.com/2.ics\"\n");
        assert_eq!(
            c4.calendar_ics,
            vec!["https://a.com/1.ics".to_string(), "webcal://b.com/2.ics".to_string()]
        );

        // Пустой массив
        let c5 = parse("calendar_ics = []\n");
        assert!(c5.calendar_ics.is_empty());
    }

    #[test]
    fn calendar_ics_saving_and_set_keys_roundtrip() {
        let base = "accent = \"#D4AF5E\"\ncalendar_ics = \"https://old.com/old.ics\"\n\n[keys]\n";
        let urls = vec!["https://new.com/1.ics".to_string(), "webcal://new.com/2.ics".to_string()];
        let updated = set_keys(base, &[("calendar_ics", fmt_list(&urls))]);
        assert!(updated.contains("calendar_ics = [\"https://new.com/1.ics\", \"webcal://new.com/2.ics\"]"));
        let parsed = parse(&updated);
        assert_eq!(parsed.calendar_ics, urls);

        // Сохранение пустого списка
        let empty_updated = set_keys(base, &[("calendar_ics", fmt_list(&[]))]);
        assert!(empty_updated.contains("calendar_ics = []"));
        let parsed_empty = parse(&empty_updated);
        assert!(parsed_empty.calendar_ics.is_empty());
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
        assert_eq!(parse("ui_scale = 9").ui_scale, MAX_SCALE);
        assert_eq!(parse("ui_scale = 0.1").ui_scale, MIN_SCALE);
        assert_eq!(c.hidden_tabs, vec!["Power".to_string(), "stats".to_string()]);
        let d = parse(DEFAULT_TEMPLATE);
        assert_eq!((d.corner_radius, d.system_colors, d.idle_face), (DEFAULT_RADIUS, false, true));
        assert!(d.hidden_tabs.is_empty());
        assert!(!d.auto_update && !Config::default().auto_update, "автообновление по умолчанию выключено");
    }

    #[test]
    fn visible_tabs_skip_hidden_but_keep_one() {
        let mut c = Config::default();
        c.tabs = vec!["C".into(), "A".into()];
        c.hidden_tabs = vec!["a".into()];
        assert_eq!(c.resolve_visible(&keys()), vec![2, 1, 3]);
        c.hidden_tabs = NAMES.iter().map(|s| s.to_string()).collect();
        assert_eq!(c.resolve_visible(&keys()), vec![2], "скрыть все вкладки нельзя");
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
        c.glass = false;
        c.glass_opacity = 35.0;
        c.material = Material::Thin;
        c.palette_style = PaletteStyle::Ios;
        c.accent_pill = true;
        c.ui_scale = 1.5;
        c.strip_over_maximized = false;
        c.hidden_tabs = vec!["Power".into(), "Wallpaper".into()];
        c.tabs = vec!["Apps".into(), "Controls".into()];
        c.idle_face = false;
        c.auto_update = true;
        let updates = [
            ("accent", fmt_str(&theme::to_hex(c.accent))),
            ("bg", fmt_str(&theme::to_hex(c.bg))),
            ("system_colors", "1".to_string()),
            ("corner_radius", "22".to_string()),
            ("glass", "0".to_string()),
            ("glass_opacity", "35".to_string()),
            ("material", fmt_str(c.material.as_str())),
            ("palette_style", fmt_str(c.palette_style.as_str())),
            ("accent_pill", "1".to_string()),
            ("ui_scale", format!("{:.3}", c.ui_scale)),
            ("strip_over_maximized", "0".to_string()),
            ("tabs", fmt_list(&c.tabs)),
            ("hidden_tabs", fmt_list(&c.hidden_tabs)),
            ("idle_face", "0".to_string()),
            ("auto_update", "1".to_string()),
        ];
        let back = parse(&set_keys(DEFAULT_TEMPLATE, &updates));
        assert_eq!(back, c);
    }

    #[test]
    fn palette_style_and_material_parsing() {
        // Проверка парсинга всех материалов (строки в кавычках, регистронезависимо)
        for (input, expected) in [
            ("material = \"ultrathin\"\n", Material::Ultrathin),
            ("material = \"Thin\"\n", Material::Thin),
            ("material = \"REGULAR\"\n", Material::Regular),
            ("material = \"thick\"\n", Material::Thick),
            ("material = \"Chrome\"\n", Material::Chrome),
        ] {
            assert_eq!(parse(input).material, expected, "input: {input}");
        }

        // Проверка парсинга palette_style (строки в кавычках, регистронезависимо)
        for (input, expected) in [
            ("palette_style = \"windows\"\n", PaletteStyle::Windows),
            ("palette_style = \"Windows\"\n", PaletteStyle::Windows),
            ("palette_style = \"ios\"\n", PaletteStyle::Ios),
            ("palette_style = \"IOS\"\n", PaletteStyle::Ios),
            ("palette_style = \"macos\"\n", PaletteStyle::Ios),
            ("palette_style = \"macOS\"\n", PaletteStyle::Ios),
        ] {
            assert_eq!(parse(input).palette_style, expected, "input: {input}");
        }
    }

    #[test]
    fn palette_style_and_material_garbage_values_fallback_to_defaults() {
        // Мусорные значения (неизвестные строки, числа, списки, пустые строки) сбрасываются в умолчания
        let c = parse("material = \"superglass\"\npalette_style = \"linux\"\n");
        assert_eq!(c.material, Material::Regular);
        assert_eq!(c.palette_style, PaletteStyle::Windows);

        let c = parse("material = 42\npalette_style = 999\n");
        assert_eq!(c.material, Material::Regular);
        assert_eq!(c.palette_style, PaletteStyle::Windows);

        let c = parse("material = [\"regular\"]\npalette_style = [\"ios\"]\n");
        assert_eq!(c.material, Material::Regular);
        assert_eq!(c.palette_style, PaletteStyle::Windows);

        let c = parse("material = \"\"\npalette_style = \"\"\n");
        assert_eq!(c.material, Material::Regular);
        assert_eq!(c.palette_style, PaletteStyle::Windows);

        // Неквотированные слова игнорируются парсером и сбрасываются в умолчания
        let c = parse("material = ultrathin\npalette_style = ios\n");
        assert_eq!(c.material, Material::Regular);
        assert_eq!(c.palette_style, PaletteStyle::Windows);
    }

    #[test]
    fn layout_keys_parse_and_save() {
        assert_eq!(Config::default().layouts, [String::new(), String::new(), String::new()]);
        assert_eq!(parse(DEFAULT_TEMPLATE).layouts, Config::default().layouts);
        let c = parse("layout_controls = \"-wifi night:1@0,1\"\nlayout_today = \"clock\"\nlayout_stats = \"\"\n");
        assert_eq!(c.layouts[0], "-wifi night:1@0,1");
        assert_eq!(c.layouts[1], "clock");
        assert_eq!(c.layouts[2], "");
        // Запись в шаблон и обратное чтение.
        let out = set_keys(DEFAULT_TEMPLATE, &[("layout_today", fmt_str("-todo clock:1@0,0"))]);
        assert_eq!(parse(&out).layouts[1], "-todo clock:1@0,0");
        assert_eq!(out.matches("layout_today").count(), 1, "ключ заменён, а не продублирован");
    }

    #[test]
    fn custom_tabs_join_the_dynamic_tab_list() {
        let mut c = Config::default();
        c.custom = vec![
            CustomTab { id: 7, name: "Работа".into(), layout: String::new() },
            CustomTab { id: 9, name: "Дом".into(), layout: String::new() },
        ];
        let keys = c.tab_keys();
        assert_eq!(keys.len(), crate::TABS.len() + 2);
        assert_eq!(c.tab_key(crate::tabs::CUSTOM_BASE + 1), "custom:9");
        assert_eq!(c.tab_label(crate::tabs::CUSTOM_BASE), "Работа");
        assert_eq!(c.tab_key(crate::tabs::SETTINGS), "", "у страницы настроек ключа нет");
        // Собственные вкладки встают в порядок по ключу, а неизвестные ключи пропускаются.
        c.tabs = vec!["custom:9".into(), "custom:404".into(), "Today".into()];
        let order = c.ordered_tabs();
        assert_eq!(&order[..3], &[crate::tabs::CUSTOM_BASE + 1, 1, 0]);
        assert_eq!(order.len(), keys.len());
        c.hidden_tabs = vec!["CUSTOM:9".into()];
        assert!(!c.visible_tabs().contains(&(crate::tabs::CUSTOM_BASE + 1)));
    }

    #[test]
    fn custom_tabs_survive_save_and_load() {
        let mut c = Config::default();
        c.custom = vec![
            CustomTab { id: 3, name: "Работа".into(), layout: "v2 controls.wifi@0,0 today.clock:1@2,0".into() },
            CustomTab { id: 8, name: "Дом".into(), layout: String::new() },
        ];
        let back = parse(&set_keys(DEFAULT_TEMPLATE, &save_updates(&c)));
        assert_eq!(back.custom, c.custom);
        assert_eq!(back.next_custom_id(), 9);
        assert_eq!(parse(DEFAULT_TEMPLATE).custom, Vec::new(), "старый конфиг открывается без собственных вкладок");
    }

    #[test]
    fn broken_custom_tab_entries_are_dropped() {
        let c = parse("custom_tabs = [\"x|Нет номера|\", \"4||пустое имя\", \"5|Живая|v2\", \"5|Повтор|\", \"мусор\"]
");
        assert_eq!(c.custom.len(), 1);
        assert_eq!((c.custom[0].id, c.custom[0].name.as_str()), (5, "Живая"));
        assert_eq!(clean_tab_name("  a|b \"c\"
 d  "), "a b c d");
        assert_eq!(clean_tab_name(&"я".repeat(40)).chars().count(), MAX_TAB_NAME);
    }

    #[test]
    fn no_more_than_the_limit_of_tabs_is_enabled() {
        let mut c = Config::default();
        c.custom = (1..=4).map(|id| CustomTab { id, name: format!("T{id}"), layout: String::new() }).collect();
        let visible = c.visible_tabs();
        assert_eq!(visible.len(), MAX_ENABLED_TABS);
        // Лишние отбрасываются с конца порядка: остаются встроенные и первая собственная.
        assert_eq!(visible, [0, 1, 2, 3, 4, 5, 6, crate::tabs::CUSTOM_BASE]);
        // Выключив встроенную, освобождаем место следующей собственной.
        c.hidden_tabs = vec!["Power".into()];
        assert_eq!(c.visible_tabs().last(), Some(&(crate::tabs::CUSTOM_BASE + 1)));
    }

    #[test]
    fn tab_order() {
        let mut c = Config::default();
        c.tabs = vec!["c".into(), "zzz".into(), "A".into(), "c".into()];
        assert_eq!(c.resolve_order(&keys()), [2, 0, 1, 3]);
        c.tabs.clear();
        assert_eq!(c.resolve_order(&keys()), [0, 1, 2, 3]);
    }

    #[test]
    fn decode_reads_utf16_and_lossy_utf8() {
        let mut b = vec![0xff, 0xfe];
        for u in "glass = 0".encode_utf16() {
            b.extend(u.to_le_bytes());
        }
        assert_eq!(super::decode(&b), "glass = 0");
        assert!(!super::decode(&[b'a', b' ', 0xff, b'=']).is_empty());
        assert!(!parse(&super::decode(&b)).glass);
    }

    #[test]
    fn glass_key_parses() {
        assert!(parse("glass = 1\n").glass);
        assert!(!parse("glass = 0\n").glass);
        assert!(parse("glass = true\n").glass);
        assert!(!parse("glass = false\n").glass);
        // Неизвестное/нечисловое значение не перезаписывает значение по умолчанию
        assert!(parse("glass = \"yes\"\n").glass);
    }

    #[test]
    fn hide_on_screenshot_key_parses_and_defaults_to_hiding() {
        assert!(Config::default().hide_on_screenshot);
        assert!(!parse("hide_on_screenshot = 0\n").hide_on_screenshot);
        assert!(parse("hide_on_screenshot = 1\n").hide_on_screenshot);
        assert!(parse(DEFAULT_TEMPLATE).hide_on_screenshot);
    }

    #[test]
    fn free_layout_key_parses_and_defaults_to_compact() {
        assert!(!Config::default().free_layout, "по умолчанию раскладка уплотняется");
        assert!(parse("free_layout = 1\n").free_layout);
        assert!(!parse("free_layout = 0\n").free_layout);
        assert!(!parse(DEFAULT_TEMPLATE).free_layout);
        let mut c = Config::default();
        c.free_layout = true;
        assert!(save_updates(&c).contains(&("free_layout", "1".to_string())));
    }

    #[test]
    fn todo_clean_done_key_parses() {
        assert!(!Config::default().todo_clean_done);
        assert!(parse("todo_clean_done = 1
").todo_clean_done);
        assert!(!parse("todo_clean_done = 0
").todo_clean_done);
        assert!(!parse(DEFAULT_TEMPLATE).todo_clean_done);
    }

    #[test]
    fn collapsed_mode_key_parses() {
        assert!(!Config::default().always_strip);
        assert!(parse("collapsed_mode = \"strip\"
").always_strip);
        assert!(parse("collapsed_mode = \"STRIP\"
").always_strip);
        assert!(!parse("collapsed_mode = \"auto\"
").always_strip);
        assert!(!parse("collapsed_mode = \"мусор\"
").always_strip);
        assert!(!parse(DEFAULT_TEMPLATE).always_strip);
    }

    #[test]
    fn accent_pill_key_parses() {
        assert!(parse("accent_pill = 1\nglass_opacity = 250").accent_pill);
        assert!(!parse("accent_pill = 0\n").accent_pill);
        assert!(parse("accent_pill = true\n").accent_pill);
        assert!(!parse("accent_pill = false\n").accent_pill);
    }
}
