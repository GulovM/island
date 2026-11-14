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

    let (title, body) = extract_texts(un);
    if title.is_empty() && body.is_empty() {
        return None;
    }

    let icon = crate::icons::resolve_notif_icon(&app_id, &app_name, || {
        read_logo_bytes(&app_id, app_info.as_ref())
    });

    Some(SysNotif { id, app_id, app_name, title, body, time_secs, icon })
}

unsafe fn extract_texts(un: &UserNotification) -> (String, String) {
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
    for el in elems {
        if let Ok(t) = el.Text() {
            let clean = t.to_string().trim().to_string();
            if !clean.is_empty() {
                lines.push(clean);
            }
        }
    }
    let mut it = lines.into_iter();
    let title = it.next().unwrap_or_default();
    let body = it.collect::<Vec<_>>().join(" · ");
    (title, body)
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
