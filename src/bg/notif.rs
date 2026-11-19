//! Чтение системных уведомлений macOS (Notification Center `db2/db`) и статуса «Не беспокоить» (Focus).
//! При отсутствии Full Disk Access просит доступ в Настройках macOS, а локально хранит список
//! отмеченных как прочитанные уведомлений.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::parse_plist_xml_dict;

/// Размер кэшированной иконки приложения (в пикселях, premultiplied BGRA).
pub const ICON_PX: u32 = crate::icons::SIZE_PX;
/// Разница между эпохой Apple CoreFoundation (`2001-01-01`) и Unix (`1970-01-01`).
const APPLE_EPOCH_OFFSET: i64 = 978_307_200;
const MAX_NOTIFS: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Access {
    #[default]
    Unspecified,
    Allowed,
    Denied,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SysNotif {
    pub id: u32,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    /// Время получения (Unix-секунды).
    pub time_secs: u64,
    /// Иконка приложения (`ICON_PX` × `ICON_PX`, premultiplied BGRA).
    pub icon: Option<Arc<Vec<u8>>>,
}

static DISMISSED: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
static NC_DB_PATH: Mutex<Option<String>> = Mutex::new(None);

#[cfg(target_os = "macos")]
fn run_cmd(prog: &str, args: &[&str]) -> Option<(bool, String, String)> {
    let out = std::process::Command::new(prog).args(args).output().ok()?;
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn run_cmd(_prog: &str, _args: &[&str]) -> Option<(bool, String, String)> {
    None
}

fn nc_db_path() -> Option<String> {
    if let Ok(guard) = NC_DB_PATH.lock() {
        if let Some(p) = guard.as_ref() {
            return Some(p.clone());
        }
    }
    let (ok, out, _) = run_cmd("getconf", &["DARWIN_USER_DIR"])?;
    if !ok {
        return None;
    }
    let dir = out.trim();
    if dir.is_empty() {
        return None;
    }
    let path = format!("{}/com.apple.notificationcenter/db2/db", dir.trim_end_matches('/'));
    if let Ok(mut guard) = NC_DB_PATH.lock() {
        *guard = Some(path.clone());
    }
    Some(path)
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    let s = hex.trim();
    if s.is_empty() || s.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for pair in s.as_bytes().chunks_exact(2) {
        let hi = std::str::from_utf8(pair).ok()?;
        bytes.push(u8::from_str_radix(hi, 16).ok()?);
    }
    Some(bytes)
}

#[cfg(target_os = "macos")]
fn plist_blob_to_xml(blob: &[u8]) -> Option<String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = std::process::Command::new("plutil")
        .args(["-convert", "xml1", "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(blob).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(not(target_os = "macos"))]
fn plist_blob_to_xml(_blob: &[u8]) -> Option<String> {
    None
}

/// Подготовка наблюдателя уведомлений (на macOS чтение идёт фоновым опросом `db2/db`).
pub unsafe fn init() {}

/// Открывает настройки Full Disk Access, чтобы пользователь разрешил чтение уведомлений macOS.
pub unsafe fn request_access() -> Access {
    let (access, _) = poll_notifications();
    if access != Access::Allowed {
        let _ = run_cmd("open", &["x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"]);
    }
    access
}

/// Читает доставленные уведомления macOS из базы Центра уведомлений.
pub unsafe fn poll_notifications() -> (Access, Vec<SysNotif>) {
    let Some(db) = nc_db_path() else {
        return (Access::Allowed, Vec::new());
    };
    if !std::path::Path::new(&db).exists() {
        return (Access::Allowed, Vec::new());
    }
    let sql = "SELECT r.rec_id, a.identifier, CAST(r.delivered_date AS INTEGER), hex(r.data) \
               FROM record r JOIN app a ON r.app_id = a.app_id \
               ORDER BY r.delivered_date DESC LIMIT 100;";
    let Some((ok, stdout, stderr)) = run_cmd("sqlite3", &["-readonly", "-separator", "\t", &db, sql]) else {
        return (Access::Allowed, Vec::new());
    };
    if !ok {
        let low = stderr.to_ascii_lowercase();
        if low.contains("authorization denied") || low.contains("operation not permitted") || low.contains("permission denied") {
            return (Access::Denied, Vec::new());
        }
        return (Access::Allowed, Vec::new());
    }

    let dismissed = DISMISSED.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
    let mut out = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.splitn(4, '\t');
        let (Some(id_s), Some(app_id_s), Some(ts_s), Some(hex_s)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Ok(id) = id_s.trim().parse::<u32>() else { continue };
        if dismissed.contains(&id) {
            continue;
        }
        let app_id = if app_id_s.trim().is_empty() {
            "com.apple.unknown".to_string()
        } else {
            app_id_s.trim().to_string()
        };
        let cf_secs = ts_s.trim().parse::<i64>().unwrap_or(0);
        let time_secs = (cf_secs + APPLE_EPOCH_OFFSET).max(0) as u64;
        let Some(blob) = hex_decode(hex_s) else { continue };
        let Some(xml) = plist_blob_to_xml(&blob) else { continue };
        let fields = parse_plist_xml_dict(&xml);
        let app_name = crate::icons::reg_app_name(&app_id).unwrap_or_else(|| fallback_app_name(&app_id));

        let mut lines = Vec::new();
        for key in ["titl", "subt", "body"] {
            if let Some(val) = fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim()).filter(|v| !v.is_empty()) {
                lines.push(val.to_string());
            }
        }
        let (title, body) = parse_notif_texts(&app_id, &app_name, &lines, None);
        if title.is_empty() && body.is_empty() {
            continue;
        }
        let icon = crate::icons::resolve_notif_icon(&app_id, &app_name, || None);
        out.push(SysNotif { id, app_id, app_name, title, body, time_secs, icon });
    }
    out.sort_by(|a, b| b.time_secs.cmp(&a.time_secs).then_with(|| b.id.cmp(&a.id)));
    out.truncate(MAX_NOTIFS);
    (Access::Allowed, out)
}

/// Помечает уведомление скрытым в ленте острова.
pub unsafe fn remove_notification(id: u32) {
    if let Ok(mut guard) = DISMISSED.lock() {
        guard.get_or_insert_with(HashSet::new).insert(id);
    }
}

/// Проверяет, включён ли в macOS режим Фокусирования / «Не беспокоить».
pub unsafe fn poll_dnd() -> Option<bool> {
    if let Some(home) = std::env::var_os("HOME") {
        let assertions = std::path::PathBuf::from(home).join("Library/DoNotDisturb/DB/Assertions.json");
        if let Ok(text) = std::fs::read_to_string(assertions) {
            return Some(text.contains("assertionDetails"));
        }
    }
    let (ok, out, _) = run_cmd("defaults", &["read", "com.apple.controlcenter", "NSStatusItem Visible FocusModes"])?;
    if !ok {
        return Some(false);
    }
    Some(out.trim() == "1")
}

/// Переключает системный режим Focus через macOS Shortcuts (если настроена быстрая команда Focus/DND).
pub unsafe fn set_dnd(on: bool) -> bool {
    let (_, list, _) = match run_cmd("shortcuts", &["list"]) {
        Some(r) => r,
        None => return false,
    };
    let candidates: &[&str] = if on {
        &["Turn Do Not Disturb On", "Do Not Disturb On", "Focus On", "Включить Не беспокоить"]
    } else {
        &["Turn Do Not Disturb Off", "Do Not Disturb Off", "Focus Off", "Выключить Не беспокоить"]
    };
    for name in candidates {
        if list.lines().any(|l| l.trim().eq_ignore_ascii_case(name))
            && run_cmd("shortcuts", &["run", name]).is_some_and(|(ok, _, _)| ok)
        {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Messenger {
    Telegram,
    Discord,
    WhatsApp,
    Vk,
    Instagram,
}

fn has_word(haystack: &str, word: &str) -> bool {
    let w = word.to_lowercase();
    haystack
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|part| part == w)
}

fn detect_messenger(app_id: &str, app_name: &str, attribution: Option<&str>) -> Option<Messenger> {
    let id = app_id.to_lowercase();
    let name = app_name.to_lowercase();
    let attr = attribution.unwrap_or("").to_lowercase();
    let any_contains = |needle: &str| id.contains(needle) || name.contains(needle) || attr.contains(needle);

    if any_contains("telegram")
        || any_contains("unigram")
        || any_contains("kotatogram")
        || any_contains("ayugram")
        || any_contains("64gram")
    {
        return Some(Messenger::Telegram);
    }
    if any_contains("discord") || any_contains("vesktop") || any_contains("armcord") {
        return Some(Messenger::Discord);
    }
    if any_contains("whatsapp") {
        return Some(Messenger::WhatsApp);
    }
    if any_contains("instagram") {
        return Some(Messenger::Instagram);
    }
    if any_contains("vkontakte")
        || any_contains("вконтакте")
        || any_contains("vk messenger")
        || any_contains("vk.com")
        || any_contains("vkme")
        || any_contains("com.vk")
        || has_word(&id, "vk")
        || has_word(&name, "vk")
        || has_word(&name, "вк")
        || has_word(&attr, "vk")
    {
        return Some(Messenger::Vk);
    }
    None
}

fn is_redundant_app_header(line: &str, app_name: &str, m: Messenger) -> bool {
    let l = line.trim().to_lowercase();
    if l == app_name.trim().to_lowercase() {
        return true;
    }
    match m {
        Messenger::Telegram => l == "telegram" || l == "telegram desktop",
        Messenger::Discord => l == "discord",
        Messenger::WhatsApp => l == "whatsapp",
        Messenger::Vk => {
            matches!(
                l.as_str(),
                "vk" | "vk messenger" | "vkontakte" | "вконтакте" | "вк" | "новое сообщение"
            )
        }
        Messenger::Instagram => l == "instagram",
    }
}

/// Разделяет префикс вида `"Отправитель: сообщение"` в тексте группового чата.
fn split_sender_message(s: &str) -> Option<(&str, &str)> {
    let (sender, msg) = s.split_once(": ")?;
    let sender = sender.trim();
    let msg = msg.trim();
    if sender.is_empty() || msg.is_empty() || sender.chars().count() > 48 {
        return None;
    }
    if sender.contains('\n') || sender.contains('\r') || sender.contains("://") {
        return None;
    }
    if sender.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if sender.split_whitespace().count() > 4 {
        return None;
    }
    let lower = sender.to_lowercase();
    if matches!(
        lower.as_str(),
        "http" | "https" | "re" | "fw" | "fwd" | "note" | "error" | "warning" | "ошибка" | "важно" | "тема" | "ссылка"
    ) {
        return None;
    }
    Some((sender, msg))
}

/// Разбирает заголовок Discord вида `"Пользователь (#канал, Сервер)"` или `"Пользователь (#канал)"`.
fn split_discord_channel_title(s: &str) -> Option<(String, String)> {
    let trimmed = s.trim();
    let stripped = trimmed.strip_suffix(')')?;
    let (user, inside) = stripped.split_once(" (#")?;
    let user = user.trim();
    let inside = inside.trim();
    if user.is_empty() || inside.is_empty() {
        return None;
    }
    let group = if let Some((chan, srv)) = inside.split_once(", ") {
        let chan = chan.trim();
        let srv = srv.trim();
        if srv.is_empty() {
            format!("#{chan}")
        } else {
            format!("{srv} (#{chan})")
        }
    } else {
        format!("#{inside}")
    };
    Some((user.to_string(), group))
}

fn format_group_user(group: &str, user: &str) -> String {
    let g = group.trim();
    let u = user.trim();
    if g.is_empty() {
        u.to_string()
    } else if u.is_empty() || g.eq_ignore_ascii_case(u) {
        g.to_string()
    } else {
        format!("{g}: {u}")
    }
}

/// Разбирает текстовые элементы системного тоста в `(контекст_шапки, сообщение)`.
/// Для поддерживаемых мессенджеров и соцсетей выделяет `<группа>: <пользователь>` или `<пользователь>`.
fn parse_notif_texts(app_id: &str, app_name: &str, lines: &[String], attribution: Option<&str>) -> (String, String) {
    if lines.is_empty() {
        return (String::new(), String::new());
    }
    let Some(m) = detect_messenger(app_id, app_name, attribution) else {
        let title = lines[0].clone();
        let body = lines[1..].join(" · ");
        return (title, body);
    };

    let work = if lines.len() >= 2 && is_redundant_app_header(&lines[0], app_name, m) {
        &lines[1..]
    } else {
        lines
    };

    if work.len() >= 3 {
        let group = &work[0];
        let user = &work[1];
        let msg = work[2..].join(" · ");
        return (format_group_user(group, user), msg);
    }

    if work.len() == 1 {
        if let Some((user, msg)) = split_sender_message(&work[0]) {
            return (user.to_string(), msg.to_string());
        }
        return (work[0].clone(), String::new());
    }

    let head = work[0].trim();
    let tail = work[1].trim();

    match m {
        Messenger::Discord => {
            if let Some((user, group)) = split_discord_channel_title(head) {
                let msg = tail
                    .strip_prefix(&format!("{user}: "))
                    .unwrap_or(tail)
                    .trim()
                    .to_string();
                return (format_group_user(&group, &user), msg);
            }
            if let Some((user, msg)) = split_sender_message(tail) {
                return (format_group_user(head, user), msg.to_string());
            }
        }
        Messenger::WhatsApp => {
            if let Some((user, group)) = head.split_once(" @ ") {
                let (user, group) = (user.trim(), group.trim());
                if !user.is_empty() && !group.is_empty() {
                    return (format_group_user(group, user), tail.to_string());
                }
            }
            if let Some((user, msg)) = split_sender_message(tail) {
                return (format_group_user(head, user), msg.to_string());
            }
        }
        Messenger::Telegram | Messenger::Vk | Messenger::Instagram => {
            if let Some((user, msg)) = split_sender_message(tail) {
                return (format_group_user(head, user), msg.to_string());
            }
        }
    }

    (head.to_string(), tail.to_string())
}

fn fallback_app_name(bundle_id: &str) -> String {
    let base = bundle_id.split('!').next().unwrap_or(bundle_id);
    let base = base.rsplit(['\\', '/']).next().unwrap_or(base);
    let base = base.strip_suffix(".app").or_else(|| base.strip_suffix(".exe")).unwrap_or(base);
    let last = base.rsplit('.').next().unwrap_or(base).trim();
    if last.is_empty() {
        "macOS".to_string()
    } else {
        last.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn parses_telegram_dm_and_group() {
        assert_eq!(
            parse_notif_texts("Telegram.TelegramDesktop", "Telegram Desktop", &s(&["Алексей", "Погнали в 18:00"]), None),
            ("Алексей".into(), "Погнали в 18:00".into())
        );
        assert_eq!(
            parse_notif_texts("Telegram.TelegramDesktop", "Telegram Desktop", &s(&["Frontend Team", "Мария: Залила фикс"]), None),
            ("Frontend Team: Мария".into(), "Залила фикс".into())
        );
        assert_eq!(
            parse_notif_texts("Telegram.Unigram", "Unigram", &s(&["Дизайн", "Олег", "Новый макет готов"]), None),
            ("Дизайн: Олег".into(), "Новый макет готов".into())
        );
    }

    #[test]
    fn parses_discord_dm_and_channels() {
        assert_eq!(
            parse_notif_texts("com.squirrel.Discord.Discord", "Discord", &s(&["alice", "Глянь PR"]), None),
            ("alice".into(), "Глянь PR".into())
        );
        assert_eq!(
            parse_notif_texts("com.squirrel.Discord.Discord", "Discord", &s(&["alice (#dev, Island Server)", "Сборка прошла!"]), None),
            ("Island Server (#dev): alice".into(), "Сборка прошла!".into())
        );
        assert_eq!(
            parse_notif_texts("com.squirrel.Discord.Discord", "Discord", &s(&["alice (#general)", "Привет"]), None),
            ("#general: alice".into(), "Привет".into())
        );
    }

    #[test]
    fn parses_whatsapp_vk_and_instagram() {
        assert_eq!(
            parse_notif_texts("5319275A.WhatsAppDesktop", "WhatsApp", &s(&["Иван @ Семья", "Купил хлеб"]), None),
            ("Семья: Иван".into(), "Купил хлеб".into())
        );
        assert_eq!(
            parse_notif_texts("5319275A.WhatsAppDesktop", "WhatsApp", &s(&["Семья", "Иван: Купил хлеб"]), None),
            ("Семья: Иван".into(), "Купил хлеб".into())
        );
        assert_eq!(
            parse_notif_texts("com.vk.messenger", "VK Messenger", &s(&["Универ 2026", "Дмитрий: Пара перенесена"]), None),
            ("Универ 2026: Дмитрий".into(), "Пара перенесена".into())
        );
        assert_eq!(
            parse_notif_texts("Chrome", "Google Chrome", &s(&["ВКонтакте", "Анна Иванова: Привет"]), Some("vk.com")),
            ("Анна Иванова".into(), "Привет".into())
        );
        assert_eq!(
            parse_notif_texts("Chrome", "Google Chrome", &s(&["Instagram", "ivan_dev: Посмотри рилс"]), Some("instagram.com")),
            ("ivan_dev".into(), "Посмотри рилс".into())
        );
        assert_eq!(
            parse_notif_texts("Instagram", "Instagram", &s(&["Друзья", "ivan_dev: Хаха"]), None),
            ("Друзья: ivan_dev".into(), "Хаха".into())
        );
    }

    #[test]
    fn non_messenger_keeps_plain_title_and_body() {
        assert_eq!(
            parse_notif_texts("com.vulkan.VulkanApp", "Vulkan Monitor", &s(&["GPU Alert", "Temp: 82C"]), None),
            ("GPU Alert".into(), "Temp: 82C".into())
        );
        assert_eq!(
            parse_notif_texts("Telegram.TelegramDesktop", "Telegram Desktop", &s(&["Иван", "https://example.com"]), None),
            ("Иван".into(), "https://example.com".into())
        );
    }
}
