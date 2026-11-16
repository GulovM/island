//! Чтение системных уведомлений Windows и статуса «Не беспокоить» (Focus Assist / DND)
//! через WinRT (`UserNotificationListener` и `ToastNotificationManagerForUser`).
//! Работает событийно на фоновом потоке: в простое 0% CPU.

use std::sync::{Arc, Mutex};

use windows::core::{IInspectable, HSTRING};
use windows::ApplicationModel::AppInfo;
use windows::Foundation::{Size, TypedEventHandler};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamWithContentType};
use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{
    KnownNotificationBindings, NotificationKinds, ToastNotificationManager,
    ToastNotificationManagerForUser, ToastNotificationMode, UserNotification,
};

use super::winrt::wait;
use super::{send, Cmd};

/// Размер кэшированной иконки приложения (в пикселях, premultiplied BGRA).
pub const ICON_PX: u32 = crate::icons::SIZE_PX;
/// 100-наносекундных тиков между 1601-01-01 (WinRT DateTime) и 1970-01-01 (Unix epoch).
const UNIX_EPOCH_100NS: i64 = 116_444_736_000_000_000;
const MAX_NOTIFS: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Access {
    #[default]
    Unspecified,
    Allowed,
    Denied,
}

impl Access {
    fn from_winrt(s: UserNotificationListenerAccessStatus) -> Self {
        match s {
            UserNotificationListenerAccessStatus::Allowed => Self::Allowed,
            UserNotificationListenerAccessStatus::Denied => Self::Denied,
            _ => Self::Unspecified,
        }
    }
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

static WATCH_INIT: Mutex<Option<WatchState>> = Mutex::new(None);

struct WatchState {
    _listener: Option<UserNotificationListener>,
    _dnd_mgr: Option<ToastNotificationManagerForUser>,
}

// SAFETY: WinRT-объекты хранятся только ради удержания подписок событий до конца жизни процесса
// (внутри MTA-потока фонового воркера и процесса).
unsafe impl Send for WatchState {}
unsafe impl Sync for WatchState {}

/// Подписывается на `NotificationChanged` и `NotificationModeChanged`. Безопасно вызывать повторно.
pub unsafe fn init() {
    let Ok(mut guard) = WATCH_INIT.lock() else { return };
    if guard.is_some() {
        return;
    }

    let listener = UserNotificationListener::Current().ok().inspect(|l| {
        let handler = TypedEventHandler::new(|_, _| {
            send(Cmd::NotifRefresh);
            Ok(())
        });
        let _ = l.NotificationChanged(&handler);
    });

    let dnd_mgr = ToastNotificationManager::GetDefault().ok().inspect(|m| {
        let handler: TypedEventHandler<ToastNotificationManagerForUser, IInspectable> =
            TypedEventHandler::new(|_, _| {
                send(Cmd::DndRefresh);
                Ok(())
            });
        let _ = m.NotificationModeChanged(&handler);
    });

    *guard = Some(WatchState { _listener: listener, _dnd_mgr: dnd_mgr });
}

/// Запрашивает у пользователя доступ к уведомлениям Windows.
pub unsafe fn request_access() -> Access {
    let Ok(l) = UserNotificationListener::Current() else {
        return Access::Denied;
    };
    match wait(l.RequestAccessAsync()) {
        Ok(s) => Access::from_winrt(s),
        Err(_) => Access::from_winrt(l.GetAccessStatus().unwrap_or(UserNotificationListenerAccessStatus::Unspecified)),
    }
}

/// Читает текущий список тост-уведомлений из Windows.
pub unsafe fn poll_notifications() -> (Access, Vec<SysNotif>) {
    let Ok(l) = UserNotificationListener::Current() else {
        return (Access::Denied, Vec::new());
    };
    let access = l
        .GetAccessStatus()
        .map(Access::from_winrt)
        .unwrap_or(Access::Unspecified);
    if access != Access::Allowed {
        return (access, Vec::new());
    }

    let Ok(list) = wait(l.GetNotificationsAsync(NotificationKinds::Toast)) else {
        return (access, Vec::new());
    };

    let mut out = Vec::new();
    for un in list {
        if let Some(item) = extract_notif(&un) {
            out.push(item);
        }
    }
    // Свежие сверху.
    out.sort_by(|a, b| b.time_secs.cmp(&a.time_secs).then_with(|| b.id.cmp(&a.id)));
    out.truncate(MAX_NOTIFS);
    (access, out)
}

/// Удаляет одно уведомление из центра уведомлений Windows.
pub unsafe fn remove_notification(id: u32) {
    if let Ok(l) = UserNotificationListener::Current() {
        let _ = l.RemoveNotification(id);
    }
}

/// Очищает все уведомления в центре уведомлений Windows.
pub unsafe fn clear_notifications() {
    if let Ok(l) = UserNotificationListener::Current() {
        let _ = l.ClearNotifications();
    }
}

/// Проверяет, включён ли в Windows режим «Не беспокоить» (`PriorityOnly` или `AlarmsOnly`).
pub unsafe fn poll_dnd() -> Option<bool> {
    let mgr = ToastNotificationManager::GetDefault().ok()?;
    let mode = mgr.NotificationMode().ok()?;
    Some(mode != ToastNotificationMode::Unrestricted)
}

#[allow(non_snake_case, dead_code)]
mod qh {
    use windows::core::{HRESULT, PCWSTR, PWSTR};

    /// Внутренний COM-интерфейс Focus Assist / Quiet Hours Windows 10/11 (`UserSelectedProfile`).
    #[windows::core::interface("6bff4732-81ec-4ffb-ae67-b6c1bc29631f")]
    pub unsafe trait IQuietHoursSettings: windows::core::IUnknown {
        pub unsafe fn get_UserSelectedProfile(&self, profile_id: *mut PWSTR) -> HRESULT;
        pub unsafe fn put_UserSelectedProfile(&self, profile_id: PCWSTR) -> HRESULT;
    }
}

/// Включает (`PriorityOnly`) или выключает (`Unrestricted`) системный режим «Не беспокоить».
pub unsafe fn set_dnd(on: bool) -> bool {
    use windows::core::{w, GUID};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_LOCAL_SERVER};
    use qh::IQuietHoursSettings;

    const CLSID_QUIET_HOURS_SETTINGS: GUID = GUID::from_u128(0xf53321fa_34f8_4b7f_b9a3_361877cb94cf);
    let settings = match CoCreateInstance::<_, IQuietHoursSettings>(&CLSID_QUIET_HOURS_SETTINGS, None, CLSCTX_LOCAL_SERVER) {
        Ok(s) => s,
        Err(e) => {
            crate::log::error(&format!("не удалось получить IQuietHoursSettings: {e}"));
            return false;
        }
    };
    let profile = if on {
        w!("Microsoft.QuietHoursProfile.PriorityOnly")
    } else {
        w!("Microsoft.QuietHoursProfile.Unrestricted")
    };
    let hr = settings.put_UserSelectedProfile(profile);
    if hr.is_err() {
        crate::log::error(&format!("не удалось переключить «Не беспокоить»: 0x{:08X}", hr.0 as u32));
        return false;
    }
    true
}

unsafe fn extract_notif(un: &UserNotification) -> Option<SysNotif> {
    let id = un.Id().ok()?;
    let time_secs = un
        .CreationTime()
        .ok()
        .map(|dt| ((dt.UniversalTime - UNIX_EPOCH_100NS).max(0) / 10_000_000) as u64)
        .unwrap_or(0);

    let app_info = un.AppInfo().ok();
    let app_id = app_info
        .as_ref()
        .and_then(|a| a.AppUserModelId().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "windows.unknown".to_string());

    let mut app_name = app_info
        .as_ref()
        .and_then(|a| a.DisplayInfo().ok())
        .and_then(|d| d.DisplayName().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    if app_name.starts_with('@') {
        if let Some(resolved) = crate::icons::resolve_indirect(&app_name) {
            app_name = resolved;
        }
    }
    if app_name.trim().is_empty() {
        app_name = crate::icons::reg_app_name(&app_id).unwrap_or_else(|| fallback_app_name(&app_id));
    }

    let (title, body) = extract_texts(un, &app_id, &app_name);
    if title.is_empty() && body.is_empty() {
        return None;
    }

    let icon = crate::icons::resolve_notif_icon(&app_id, &app_name, || {
        read_logo_bytes(&app_id, app_info.as_ref())
    });

    Some(SysNotif { id, app_id, app_name, title, body, time_secs, icon })
}

unsafe fn extract_texts(un: &UserNotification, app_id: &str, app_name: &str) -> (String, String) {
    let Ok(notif) = un.Notification() else { return (String::new(), String::new()) };
    let Ok(visual) = notif.Visual() else { return (String::new(), String::new()) };

    let binding = KnownNotificationBindings::ToastGeneric()
        .ok()
        .and_then(|name| visual.GetBinding(&name).ok())
        .or_else(|| {
            visual
                .Bindings()
                .ok()
                .and_then(|v| v.GetAt(0).ok())
        });
    let Some(binding) = binding else { return (String::new(), String::new()) };
    let Ok(elems) = binding.GetTextElements() else { return (String::new(), String::new()) };

    let mut lines: Vec<String> = Vec::new();
    let mut attribution: Option<String> = None;
    let placement_key = HSTRING::from("placement");
    for el in elems {
        if let Ok(t) = el.Text() {
            let clean = t.to_string().trim().to_string();
            if !clean.is_empty() {
                let is_attr = el
                    .Hints()
                    .ok()
                    .and_then(|h| h.Lookup(&placement_key).ok())
                    .is_some_and(|v| v.to_string().eq_ignore_ascii_case("attribution"));
                if is_attr {
                    attribution = Some(clean);
                } else {
                    lines.push(clean);
                }
            }
        }
    }
    parse_notif_texts(app_id, app_name, &lines, attribution.as_deref())
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

unsafe fn read_logo_bytes(app_id: &str, info: Option<&AppInfo>) -> Option<Vec<u8>> {
    let owned = if info.is_none() && app_id.contains('!') {
        AppInfo::GetFromAppUserModelId(&HSTRING::from(app_id)).ok()
    } else {
        None
    };
    let info = info.or(owned.as_ref())?;
    let disp = info.DisplayInfo().ok()?;
    let logo_ref = disp
        .GetLogo(Size { Width: ICON_PX as f32, Height: ICON_PX as f32 })
        .ok()?;
    let stream: IRandomAccessStreamWithContentType = wait(logo_ref.OpenReadAsync()).ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > (4 << 20) {
        return None;
    }
    let size = size as u32;
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let mut loaded = 0u32;
    while loaded < size {
        let chunk = wait(reader.LoadAsync(size - loaded)).ok()?;
        if chunk == 0 {
            break;
        }
        loaded = loaded.saturating_add(chunk);
    }
    let avail = reader.UnconsumedBufferLength().ok()? as usize;
    if avail == 0 {
        return None;
    }
    let mut bytes = vec![0u8; avail];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

fn fallback_app_name(aumid: &str) -> String {
    let base = aumid.split('!').next().unwrap_or(aumid);
    let base = base.rsplit(['\\', '/']).next().unwrap_or(base);
    let base = base.split('_').next().unwrap_or(base);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let last = base.rsplit('.').next().unwrap_or(base).trim();
    if last.is_empty() {
        "Windows".to_string()
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
