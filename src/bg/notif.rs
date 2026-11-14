//! Чтение системных уведомлений Windows и статуса «Не беспокоить» (Focus Assist / DND)
//! через WinRT (`UserNotificationListener` и `ToastNotificationManagerForUser`).
//! Работает событийно на фоновом потоке: в простое 0% CPU.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::core::IInspectable;

use super::winrt::wait;
use super::{send, Cmd};

/// Размер кэшированной иконки приложения (в пикселях, premultiplied BGRA).
pub const ICON_PX: u32 = 40;
/// 100-наносекундных тиков между 1601-01-01 (WinRT DateTime) и 1970-01-01 (Unix epoch).
const UNIX_EPOCH_100NS: i64 = 116_444_736_000_000_000;
const ICON_CACHE_CAP: usize = 64;
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

type IconCache = HashMap<String, Option<Arc<Vec<u8>>>>;

static WATCH_INIT: Mutex<Option<WatchState>> = Mutex::new(None);
static ICON_CACHE: Mutex<Option<IconCache>> = Mutex::new(None);

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
    if app_name.trim().is_empty() {
        app_name = fallback_app_name(&app_id);
    }

    let (title, body) = extract_texts(un);
    if title.is_empty() && body.is_empty() {
        return None;
    }

    let icon = app_info.as_ref().and_then(|info| cached_logo(&app_id, info));

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

unsafe fn cached_logo(app_id: &str, info: &AppInfo) -> Option<Arc<Vec<u8>>> {
    if let Ok(guard) = ICON_CACHE.lock() {
        if let Some(map) = guard.as_ref() {
            if let Some(cached) = map.get(app_id) {
                return cached.clone();
            }
        }
    }
    let decoded = read_logo(info).map(Arc::new);
    if let Ok(mut guard) = ICON_CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        if map.len() >= ICON_CACHE_CAP {
            map.clear();
        }
        map.insert(app_id.to_string(), decoded.clone());
    }
    decoded
}

unsafe fn read_logo(info: &AppInfo) -> Option<Vec<u8>> {
    let disp = info.DisplayInfo().ok()?;
    let logo_ref = disp
        .GetLogo(Size { Width: ICON_PX as f32, Height: ICON_PX as f32 })
        .ok()?;
    let stream: IRandomAccessStreamWithContentType = wait(logo_ref.OpenReadAsync()).ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 4 << 20 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    wait(reader.LoadAsync(size)).ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;

    let factory: IWICImagingFactory =
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let ws = factory.CreateStream().ok()?;
    ws.InitializeFromMemory(&bytes).ok()?;
    let dec = factory
        .CreateDecoderFromStream(&ws, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
        .ok()?;
    let frame = dec.GetFrame(0).ok()?;
    let (mut w, mut h) = (0u32, 0u32);
    frame.GetSize(&mut w, &mut h).ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    let scaler = factory.CreateBitmapScaler().ok()?;
    scaler
        .Initialize(&frame, ICON_PX, ICON_PX, WICBitmapInterpolationModeFant)
        .ok()?;
    let conv = factory.CreateFormatConverter().ok()?;
    conv.Initialize(
        &scaler,
        &GUID_WICPixelFormat32bppPBGRA,
        WICBitmapDitherTypeNone,
        None,
        0.0,
        WICBitmapPaletteTypeCustom,
    )
    .ok()?;
    let mut buf = vec![0u8; (ICON_PX * ICON_PX * 4) as usize];
    conv.CopyPixels(std::ptr::null(), ICON_PX * 4, &mut buf).ok()?;
    Some(buf)
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
