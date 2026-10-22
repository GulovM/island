//! Интерфейс на двух языках: русский (исходные строки в коде) и английский.
//! Русская строка служит ключом: `tr` ищет её в таблице и возвращает английскую; в русском режиме
//! строка не меняется. Перевод применяется при раскладке текста (`Fonts::layout`), поэтому вызовы
//! рисования остаются как есть. Строки со вставленными числами собираются функциями этого модуля.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ru,
    En,
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::Ru
    }
}

pub fn en() -> bool {
    get() == Lang::En
}

/// Русский или английский вариант по текущему языку.
pub fn pick<'a>(ru: &'a str, en_text: &'a str) -> &'a str {
    if en() {
        en_text
    } else {
        ru
    }
}

/// Язык из значения конфига: `ru`, `en`, иначе («auto») — по языку Windows.
pub fn from_config(v: &str) -> Lang {
    match v.trim().to_lowercase().as_str() {
        "ru" => Lang::Ru,
        "en" => Lang::En,
        _ => system_lang(),
    }
}

fn system_lang() -> Lang {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if n > 0 {
        let name = String::from_utf16_lossy(&buf[..(n as usize).saturating_sub(1)]).to_lowercase();
        if name.starts_with("ru") || name.starts_with("uk") || name.starts_with("be") || name.starts_with("kk") || name.starts_with("tg") {
            return Lang::Ru;
        }
    }
    Lang::En
}

/// Перевод статической строки; незнакомая строка (в том числе пользовательские данные) не меняется.
pub fn tr(s: &str) -> Cow<'_, str> {
    if !en() {
        return Cow::Borrowed(s);
    }
    match table().get(s) {
        Some(e) => Cow::Borrowed(e),
        None => Cow::Borrowed(s),
    }
}

/// `tr` для `&'static str` с результатом без `Cow`.
pub fn t(s: &'static str) -> &'static str {
    if !en() {
        return s;
    }
    table().get(s).copied().unwrap_or(s)
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| TABLE.iter().copied().collect())
}

/// Название вкладки для показа (идентификаторы в конфиге остаются английскими).
pub fn tab_name(i: usize) -> &'static str {
    const RU: [&str; 7] = ["Управление", "Сегодня", "Статистика", "Буфер", "Приложения", "Обои", "Питание"];
    const EN: [&str; 7] = ["Controls", "Today", "Stats", "Clipboard", "Apps", "Wallpaper", "Power"];
    let names = if en() { &EN } else { &RU };
    names.get(i).copied().unwrap_or("")
}

pub fn weekdays() -> [&'static str; 7] {
    if en() {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    } else {
        ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"]
    }
}

pub fn months_short() -> [&'static str; 12] {
    if en() {
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
    } else {
        ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"]
    }
}

pub fn months_full() -> [&'static str; 12] {
    if en() {
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
    } else {
        ["Январь", "Февраль", "Март", "Апрель", "Май", "Июнь", "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь"]
    }
}

/// Полные названия дней недели с понедельника.
pub fn weekdays_full() -> [&'static str; 7] {
    if en() {
        ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
    } else {
        ["Понедельник", "Вторник", "Среда", "Четверг", "Пятница", "Суббота", "Воскресенье"]
    }
}

/// Месяц в дате: «22 октября» (родительный падеж) / «October 22».
pub fn day_month(day: u32, month: u32) -> String {
    let m = (month as usize).clamp(1, 12) - 1;
    if en() {
        format!("{} {day}", months_full()[m])
    } else {
        const GEN: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
        format!("{day} {}", GEN[m])
    }
}

/// «5 мин назад» / «5 min ago».
pub fn ago_long(d: u64) -> String {
    if en() {
        match d {
            0..=9 => "just now".into(),
            10..=59 => format!("{d} s ago"),
            60..=3599 => format!("{} min ago", d / 60),
            3600..=86_399 => format!("{} h ago", d / 3600),
            _ => format!("{} d ago", d / 86_400),
        }
    } else {
        match d {
            0..=9 => "только что".into(),
            10..=59 => format!("{d} с назад"),
            60..=3599 => format!("{} мин назад", d / 60),
            3600..=86_399 => format!("{} ч назад", d / 3600),
            _ => format!("{} дн назад", d / 86_400),
        }
    }
}

/// Короткий возраст записи буфера: «5 м» / «5 m».
pub fn ago_short(d: u64) -> String {
    let (now, m, h, dd) = if en() { ("now", "m", "h", "d") } else { ("сейчас", "м", "ч", "д") };
    match d {
        0..=59 => now.into(),
        60..=3599 => format!("{} {m}", d / 60),
        3600..=86_399 => format!("{} {h}", d / 3600),
        _ => format!("{} {dd}", d / 86_400),
    }
}

/// Английские значения для строк таблицы: (русский ключ, английский перевод).
const TABLE: &[(&str, &str)] = &[
    // Wi-Fi / Bluetooth / аудио (bg.rs)
    ("Bluetooth • не подключено", "Bluetooth • not connected"),
    ("Подключено", "Connected"),
    ("Подключение…", "Connecting…"),
    ("Не удалось создать профиль сети", "Couldn't create the network profile"),
    ("Не подключилось — неверный пароль?", "Couldn't connect — wrong password?"),
    ("Не удалось подключиться", "Couldn't connect"),
    ("Отключение…", "Disconnecting…"),
    ("Отключено", "Disconnected"),
    ("Не удалось подключить (устройство рядом и включено?)", "Couldn't connect (is the device nearby and on?)"),
    ("Не удалось отключить", "Couldn't disconnect"),
    ("Сопряжено • нажмите, чтобы подключить", "Paired • click to connect"),
    ("Сопряжено", "Paired"),
    // Живые уведомления
    ("Таймер завершён", "Timer finished"),
    ("Выкл", "Off"),
    ("Вкл", "On"),
    ("Зарядка", "Charging"),
    ("От батареи", "On battery"),
    ("Низкий заряд", "Low battery"),
    // Погода
    ("Город не найден", "City not found"),
    ("Нет данных", "No data"),
    ("Рядом", "Nearby"),
    ("Ясно", "Clear"),
    ("Почти ясно", "Mostly clear"),
    ("Переменная облачность", "Partly cloudy"),
    ("Пасмурно", "Overcast"),
    ("Туман", "Fog"),
    ("Морось", "Drizzle"),
    ("Дождь", "Rain"),
    ("Снег", "Snow"),
    ("Ливень", "Showers"),
    ("Снегопад", "Snow showers"),
    ("Гроза", "Thunderstorm"),
    // Трей
    ("Открыть config.toml", "Open config.toml"),
    ("Перечитать конфиг", "Reload config"),
    ("Автозапуск", "Start with Windows"),
    // Обновления
    ("Автообновление", "Auto-update"),
    ("Проверить", "Check"),
    ("Проверка…", "Checking…"),
    ("Установить", "Install"),
    ("Установка…", "Installing…"),
    ("Повторить", "Retry"),
    ("Нет связи с GitHub", "Can't reach GitHub"),
    ("Не удалось скачать", "Download failed"),
    ("Файл повреждён", "File is corrupted"),
    ("Не удалось запустить установщик", "Couldn't start the installer"),
    ("Выход", "Exit"),
    // Apps
    ("ИЗБРАННОЕ", "FAVORITES"),
    ("НЕДАВНИЕ", "RECENT"),
    ("ПРИЛОЖЕНИЯ", "APPS"),
    ("Поиск приложений, файлов и ссылок…", "Search apps, files and links…"),
    ("↵ Открыть", "↵ Open"),
    ("Приложение", "App"),
    ("Ссылка", "Link"),
    ("Ярлыки меню «Пуск» не найдены", "No Start menu shortcuts found"),
    ("Ничего не найдено", "Nothing found"),
    // Clipboard
    ("Все", "All"),
    ("Текст", "Text"),
    ("Ссылки", "Links"),
    ("Фото", "Photos"),
    ("История пуста — скопируйте что-нибудь", "History is empty — copy something"),
    ("Выберите запись слева", "Select an item on the left"),
    ("Тип", "Type"),
    ("Размер", "Size"),
    ("Скопировано", "Copied"),
    ("Загрузка…", "Loading…"),
    ("Изображение", "Image"),
    ("Вставить", "Paste"),
    ("Копировать", "Copy"),
    ("Закрепить", "Pin"),
    ("Удалить", "Delete"),
    ("Запись приостановлена (шестерёнка)", "Recording paused (gear icon)"),
    ("↑↓ выбор • Tab фильтр • Enter вставить", "↑↓ select • Tab filter • Enter paste"),
    ("ЗАКРЕПЛЕНО", "PINNED"),
    ("СЕГОДНЯ", "TODAY"),
    ("ВЧЕРА", "YESTERDAY"),
    ("РАНЕЕ", "EARLIER"),
    // Controls
    ("Динамики", "Speakers"),
    ("Наушники", "Headphones"),
    ("Без звука", "Muted"),
    ("Аудио", "Audio"),
    ("Параметры", "Settings"),
    ("Ночной свет", "Night light"),
    ("Тёмная тема", "Dark theme"),
    ("СЕТИ WI-FI", "WI-FI NETWORKS"),
    ("УСТРОЙСТВА ВЫВОДА", "OUTPUT DEVICES"),
    ("Сети не найдены (Wi-Fi выключен?)", "No networks found (is Wi-Fi off?)"),
    ("Нет сопряжённых устройств", "No paired devices"),
    ("Устройства не найдены", "No devices found"),
    ("Пароль сети", "Network password"),
    ("Подключить", "Connect"),
    ("Отмена", "Cancel"),
    ("Нужно не меньше 8 символов", "At least 8 characters"),
    ("Enter — подключить", "Enter — connect"),
    ("Ничего не играет", "Nothing playing"),
    // Power
    ("Блокировка", "Lock"),
    ("Сон", "Sleep"),
    ("Перезагрузка", "Restart"),
    ("Выключение", "Shut down"),
    ("Подтвердить", "Confirm"),
    // Settings
    ("Цвет системы", "System color"),
    ("Как в Windows и Wallpaper Engine", "Like Windows and Wallpaper Engine"),
    ("Акцент", "Accent"),
    ("Фон", "Background"),
    ("Время и Wi-Fi на свёрнутом острове", "Time and Wi-Fi on the collapsed island"),
    ("Над окнами на весь экран — полоска", "Thin strip over fullscreen windows"),
    ("Наведите на верхний край, чтобы раскрыть", "Hover the top edge to expand"),
    ("Радиус углов", "Corner radius"),
    ("Стекло", "Glass"),
    ("Нет размытия: в Windows выключены эффекты прозрачности", "No blur: Windows transparency effects are off"),
    ("Акцентная капля", "Accent pill"),
    ("Прозрачность", "Transparency"),
    ("Стиль цветов", "Color style"),
    ("Материал", "Material"),
    ("Масштаб интерфейса", "Interface scale"),
    ("Внешний вид", "Appearance"),
    ("Вкладки", "Tabs"),
    ("Система", "System"),
    ("ПОВЕДЕНИЕ", "BEHAVIOR"),
    ("ОБНОВЛЕНИЯ", "UPDATES"),
    ("ВКЛАДКИ — ВКЛЮЧЕНИЕ И ПОРЯДОК", "TABS — VISIBILITY AND ORDER"),
    ("Сбросить внешний вид", "Reset appearance"),
    ("Язык", "Language"),
    ("Авто", "Auto"),
    // Stats
    ("Диск C:", "Disk C:"),
    ("В работе", "Uptime"),
    ("Собираю данные…", "Collecting data…"),
    ("Скорость сети", "Network speed"),
    ("Мбит/с ↓", "Mbit/s ↓"),
    ("Мбит/с ↑", "Mbit/s ↑"),
    ("Идёт замер…", "Measuring…"),
    ("▶ Повторить замер", "▶ Measure again"),
    ("▶ Замерить скорость", "▶ Measure speed"),
    ("ещё не замеряли", "not measured yet"),
    // Today
    ("Город", "City"),
    ("по IP", "by IP"),
    ("Enter — сохранить", "Enter — save"),
    ("пусто — по IP", "empty — by IP"),
    ("Эта неделя", "This week"),
    ("Сегодня", "Today"),
    ("На сегодня событий нет", "No events for today"),
    ("ещё", "more"),
    ("Не удалось загрузить", "Couldn't load"),
    ("Проверьте ссылку calendar_ics", "Check the calendar_ics link"),
    ("Событий нет", "No events"),
    ("Календарь пуст на ближайшее время", "The calendar is empty for now"),
    ("Календарь не подключён", "Calendar not connected"),
    ("Нажмите, чтобы добавить ссылку", "Click to add a link"),
    ("Ссылка на календарь (iCal)", "Calendar link (iCal)"),
    ("https://… или webcal://…", "https://… or webcal://…"),
    ("Нужна ссылка вида https:// или webcal://", "A link like https:// or webcal:// is required"),
    ("Enter — сохранить, пусто — отключить, Esc — отмена", "Enter — save, empty — disable, Esc — cancel"),
    ("Google/Яндекс/Outlook: «секретный адрес в формате iCal»", "Google/Yandex/Outlook: “secret address in iCal format”"),
    ("Календарь не подключён — нажмите, чтобы добавить ссылку", "Calendar not connected — click to add a link"),
    ("Не удалось загрузить календарь", "Couldn't load the calendar"),
    ("весь день", "all day"),
    ("ч", "h"),
    ("мин", "min"),
    ("Пауза", "Pause"),
    ("Старт", "Start"),
    ("Сброс", "Reset"),
    ("Задачи", "Tasks"),
    ("Изменить задачу…", "Edit task…"),
    ("Добавить задачу…", "Add task…"),
    ("Список пуст", "No tasks"),
    // Wallpaper
    ("Недавние", "Recent"),
    ("Папка", "Folder"),
    ("Недавних обоев пока нет", "No recent wallpapers yet"),
    ("Выбрать папку…", "Choose folder…"),
    ("Обои Wallpaper Engine не найдены", "No Wallpaper Engine wallpapers found"),
    // Режим правки виджетов
    ("Готово", "Done"),
    ("Сбросить", "Reset layout"),
    ("Виджеты", "Widgets"),
    ("Нет виджетов", "No widgets"),
    ("Настроить", "Customize"),
    ("Верните их из списка внизу", "Bring them back from the list below"),
    ("Не хватает места", "Not enough room"),
    // Названия виджетов (лоток скрытых)
    ("Громкость", "Volume"),
    ("Яркость", "Brightness"),
    ("Плеер", "Player"),
    ("Часы", "Clock"),
    ("Погода", "Weather"),
    ("Таймер", "Timer"),
    ("Диск", "Disk"),
    ("Сеть", "Network"),
    ("Аптайм", "Uptime"),
    ("Топ приложений", "Top apps"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_unique_and_filled() {
        let mut seen = std::collections::HashSet::new();
        for (ru, en) in TABLE {
            assert!(!ru.is_empty() && !en.is_empty(), "пустая строка: {ru:?}");
            assert!(seen.insert(*ru), "дубликат ключа: {ru}");
            assert_ne!(ru, en, "перевод совпадает с ключом: {ru}");
        }
    }

    #[test]
    fn lookup_knows_static_strings_only() {
        // Глобальный язык в тестах не переключаем: тесты идут параллельно.
        assert_eq!(table().get("Задачи"), Some(&"Tasks"));
        assert_eq!(table().get("Мои дела"), None);
        assert_eq!(tr("Задачи"), "Задачи");
    }

    #[test]
    fn config_value_picks_language() {
        assert_eq!(from_config("ru"), Lang::Ru);
        assert_eq!(from_config(" EN "), Lang::En);
    }
}
