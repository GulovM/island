//! Фоновый поток для медленных системных вызовов (WinRT, COM): радио Wi-Fi/Bluetooth,
//! громкость, медиасессия. Главный поток шлёт команды и читает снимок состояния; по готовности
//! результата окну уходит `WM_APP + 3`. Потока в простое нет: он спит на `recv`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::log;

/// Сообщение главному окну: снимок состояния обновился.
pub const WM_BG: u32 = WM_APP + 3;
/// Громкость или «без звука» изменились (из любого источника): wparam — громкость в ‰, lparam — mute.
pub const WM_VOLUME: u32 = WM_APP + 4;

pub(super) static HWND_MAIN: AtomicIsize = AtomicIsize::new(0);

/// Пробуждает главное окно из любого потока.
pub fn notify() {
    let h = HWND_MAIN.load(Ordering::Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_BG, WPARAM(0), LPARAM(0));
        }
    }
}

#[derive(Clone, Debug)]
pub struct Media {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Позиция и длительность в секундах на момент `at`.
    pub pos: f32,
    pub dur: f32,
    pub at: Instant,
    /// Обложка: пиксели BGRA premultiplied `ART`×`ART` (если у приложения есть).
    pub art: Option<Arc<Vec<u8>>>,
    /// Ключ трека (хэш названия и исполнителя), по нему кэшируется битмап и сверяется смена трека.
    pub key: u64,
}

/// Сторона квадрата обложки, пиксели.
pub const ART: u32 = 80;

impl Media {
    /// Позиция с поправкой на прошедшее с момента опроса время, пока играет.
    pub fn position_now(&self) -> f32 {
        let p = if self.playing { self.pos + self.at.elapsed().as_secs_f32() } else { self.pos };
        if self.dur > 0.0 {
            p.min(self.dur)
        } else {
            p
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
    pub volume: Option<f32>,
    /// Яркость экрана, 0..=1; `None`, если устройство её не поддерживает.
    pub brightness: Option<f32>,
    pub muted: bool,
    pub media: Option<Media>,
    /// Сеть Wi-Fi, к которой подключены.
    pub ssid: Option<String>,
    /// Короткое имя устройства вывода звука («Динамики»).
    pub audio: Option<String>,
    /// Сколько раз снимок обновлялся (чтобы вкладка понимала, что есть новое).
    pub rev: u64,
}

mod audio_policy;
mod bluetooth;
mod devices;
mod net;
mod volume;
mod wifi;
mod winrt;

pub use devices::{list, status, DevItem, ListKind};
pub use net::{net_status, NetStatus};

use audio_policy::audio_set_default;
use bluetooth::{bluetooth_devices, bt_spawn};
use devices::{audio_devices, device, device_id, device_name, endpoint, set_status, store_list};
use volume::watch_volume;
use wifi::{wifi_disconnect, wifi_networks, wifi_spawn, wifi_ssid};
use winrt::wait;

pub enum Cmd {
    /// Обновить выпадающий список устройств.
    List(ListKind),
    /// Подключиться к Wi-Fi: SSID и имя сохранённого профиля.
    WifiConnect(String, String),
    /// Подключиться к новой сети: SSID, «аутентификация|шифрование», пароль (пусто у открытой).
    WifiJoin(String, String, String),
    /// Отключиться от текущей сети Wi-Fi.
    WifiDisconnect,
    /// Подключить (`true`) или отключить Bluetooth-аудиоустройство по ContainerId.
    BtAudio(String, bool),
    /// Сделать устройство вывода звука основным (id конечной точки).
    AudioDefault(String),
    /// Полное обновление: радио, громкость, медиа.
    Refresh,
    /// Лёгкое обновление раз в секунду: только громкость и медиа (радио не опрашиваются).
    Tick,
    /// Обновить только медиа: приходит по событиям медиасессии (индикатор в свёрнутой форме).
    Media,
    /// Перечитать состояние сети (значок Wi-Fi на свёрнутом острове).
    Net,
    Wifi(bool),
    Bluetooth(bool),
    Volume(f32),
    Brightness(f32),
    Mute(bool),
    PlayPause,
    Next,
    Prev,
}

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);
static TX: OnceLock<Sender<Cmd>> = OnceLock::new();

pub fn snapshot() -> Snapshot {
    SNAPSHOT.lock().ok().and_then(|s| s.clone()).unwrap_or_default()
}

pub fn send(cmd: Cmd) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(cmd);
    }
}

/// Запускает поток. `hwnd` — окно, которому слать `WM_BG`.
pub fn start(hwnd: HWND) {
    HWND_MAIN.store(hwnd.0 as isize, Ordering::Relaxed);
    let (tx, rx) = channel::<Cmd>();
    if TX.set(tx).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new().name("island-bg".into()).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            crate::bright::probe();
            watch_volume();
            net::watch_network();
            net::refresh_net();
        }
        let mut radio_access = false;
        while let Ok(first) = rx.recv() {
            let mut batch = vec![first];
            while let Ok(c) = rx.try_recv() {
                batch.push(c);
            }
            // Слайдер громкости шлёт десятки значений — исполняем только последнее.
            let batch_full = batch.iter().any(|c| matches!(c, Cmd::Refresh | Cmd::Wifi(_) | Cmd::Bluetooth(_)));
            let only_media = batch.iter().all(|c| matches!(c, Cmd::Media | Cmd::Net));
            let wants_net = batch.iter().any(|c| matches!(c, Cmd::Net));
            let only_net = batch.iter().all(|c| matches!(c, Cmd::Net));
            let last_volume = batch.iter().rposition(|c| matches!(c, Cmd::Volume(_)));
            let last_bright = batch.iter().rposition(|c| matches!(c, Cmd::Brightness(_)));
            for (i, cmd) in batch.into_iter().enumerate() {
                match cmd {
                    Cmd::Volume(_) if Some(i) != last_volume => {}
                    Cmd::Brightness(_) if Some(i) != last_bright => {}
                    cmd => unsafe { run(cmd, &mut radio_access) },
                }
            }
            // Радио опрашиваем только по запросу и после переключения: WinRT-вызов дорогой.
            let full = batch_full;
            if only_net {
                // Только сеть: снимок громкости и медиа не трогаем.
            } else if only_media {
                unsafe { refresh_media() };
            } else {
                unsafe { refresh(full) };
            }
            if wants_net || full {
                unsafe { net::refresh_net() };
            }
            if full {
                // Загрузка WinRT раздувает рабочий набор; в простое он не нужен.
                crate::sys::trim_working_set();
            }
            notify();
        }
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить фоновый поток: {e}"));
    }
}



unsafe fn run(cmd: Cmd, radio_access: &mut bool) {
    match cmd {
        Cmd::Tick | Cmd::Media | Cmd::Net => {}
        Cmd::Refresh => watch_volume(),
        Cmd::List(ListKind::Wifi) => store_list(ListKind::Wifi, wifi_networks(true)),
        Cmd::List(ListKind::Bluetooth) => store_list(ListKind::Bluetooth, bluetooth_devices()),
        Cmd::List(ListKind::Audio) => store_list(ListKind::Audio, audio_devices()),
        Cmd::WifiConnect(ssid, profile) => wifi_spawn(ssid, Some(profile), None),
        Cmd::WifiJoin(ssid, params, key) => wifi_spawn(ssid, None, Some((params, key))),
        Cmd::WifiDisconnect => wifi_disconnect(),
        Cmd::BtAudio(container, connect) => bt_spawn(container, connect),
        Cmd::AudioDefault(id) => {
            audio_set_default(&id);
            watch_volume();
            store_list(ListKind::Audio, audio_devices());
        }
        Cmd::Wifi(on) => set_radio(windows::Devices::Radios::RadioKind::WiFi, on, radio_access),
        Cmd::Bluetooth(on) => set_radio(windows::Devices::Radios::RadioKind::Bluetooth, on, radio_access),
        Cmd::Volume(v) => {
            if let Some(ep) = endpoint() {
                let _ = ep.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
            }
        }
        Cmd::Brightness(v) => {
            crate::bright::set(v);
            // Снимок обновляем сами, иначе после удержания ползунок вернётся к старому значению.
            if let Ok(mut s) = SNAPSHOT.lock() {
                if let Some(s) = s.as_mut() {
                    s.brightness = Some(v.clamp(0.0, 1.0));
                }
            }
        }
        Cmd::Mute(m) => {
            if let Some(ep) = endpoint() {
                let _ = ep.SetMute(m, std::ptr::null());
            }
        }
        Cmd::PlayPause | Cmd::Next | Cmd::Prev => media_command(cmd),
    }
}

pub(super) unsafe fn set_radio(kind: windows::Devices::Radios::RadioKind, on: bool, access: &mut bool) {
    use windows::Devices::Radios::{Radio, RadioState};
    if !*access {
        let _ = wait(Radio::RequestAccessAsync());
        *access = true;
    }
    let Ok(radios) = wait(Radio::GetRadiosAsync()) else { return };
    for r in radios {
        if r.Kind().ok() == Some(kind) {
            let state = if on { RadioState::On } else { RadioState::Off };
            if let Err(e) = wait(r.SetStateAsync(state)) {
                log::error(&format!("не удалось переключить радио: {e}"));
            }
        }
    }
}

thread_local! {
    /// Менеджер медиасессий создаётся один раз: его запрос — самая дорогая часть опроса.
    static MEDIA_MGR: std::cell::RefCell<Option<windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager>> =
        const { std::cell::RefCell::new(None) };
}

/// Менеджер сообщил, что сессии поменялись: подписку на события сессии нужно пересоздать,
/// даже если у новой сессии тот же AUMID (вкладка браузера, следующий плеер того же приложения).
static RESUBSCRIBE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn media_manager() -> Option<windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager> {
    use windows::Foundation::TypedEventHandler;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Mgr;
    MEDIA_MGR.with(|m| {
        let mut m = m.borrow_mut();
        if m.is_none() {
            let mgr = wait(Mgr::RequestAsync()).ok()?;
            // События самого менеджера: без них запущенная при свёрнутом острове музыка (сессии не
            // было, подписываться не на что) и смена трека другим плеером оставались незамеченными.
            let wake = || {
                log::info("медиасессии изменились (событие менеджера)");
                RESUBSCRIBE.store(true, Ordering::SeqCst);
                send(Cmd::Media);
            };
            let _ = mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                wake();
                Ok(())
            }));
            let _ = mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| {
                wake();
                Ok(())
            }));
            *m = Some(mgr);
        }
        m.clone()
    })
}

thread_local! {
    /// Подписка на события текущей медиасессии: (сессия, токены двух событий, AUMID приложения).
    static SESSION_WATCH: RefCell<Option<(windows::Media::Control::GlobalSystemMediaTransportControlsSession, i64, i64, String)>> =
        const { RefCell::new(None) };
}

/// Подписывается на изменения текущей медиасессии: смена трека и пауза/воспроизведение будят
/// фоновый поток, поэтому опрашивать медиа по таймеру не нужно.
unsafe fn watch_media() {
    use windows::Foundation::TypedEventHandler;
    let Some(mgr) = media_manager() else { return };
    let cur = mgr.GetCurrentSession().ok();
    let id = cur
        .as_ref()
        .and_then(|s| s.SourceAppUserModelId().ok())
        .map(|h| h.to_string())
        .unwrap_or_default();
    let resub = RESUBSCRIBE.swap(false, Ordering::SeqCst);
    SESSION_WATCH.with(|w| {
        let mut w = w.borrow_mut();
        match (w.as_ref(), &cur) {
            (Some((_, _, _, old)), Some(_)) if *old == id && !resub => return,
            (None, None) => return,
            _ => {}
        }
        if let Some((old, t1, t2, _)) = w.take() {
            let _ = old.RemoveMediaPropertiesChanged(t1);
            let _ = old.RemovePlaybackInfoChanged(t2);
        }
        let Some(s) = cur else { return };
        let t1 = s.MediaPropertiesChanged(&TypedEventHandler::new(|_, _| {
            send(Cmd::Media);
            Ok(())
        }));
        let t2 = s.PlaybackInfoChanged(&TypedEventHandler::new(|_, _| {
            send(Cmd::Media);
            Ok(())
        }));
        if let (Ok(t1), Ok(t2)) = (t1, t2) {
            *w = Some((s, t1, t2, id));
        }
    });
}

/// Обновляет в снимке только медиа (события сессии), остальное оставляя как есть.
unsafe fn refresh_media() {
    let media = read_media();
    if let Ok(mut s) = SNAPSHOT.lock() {
        let mut snap = s.clone().unwrap_or_default();
        snap.media = media;
        snap.rev += 1;
        *s = Some(snap);
    }
}

unsafe fn media_command(cmd: Cmd) {
    let Some(mgr) = media_manager() else { return };
    let Ok(s) = mgr.GetCurrentSession() else { return };
    let _ = match cmd {
        Cmd::PlayPause => wait(s.TryTogglePlayPauseAsync()),
        Cmd::Next => wait(s.TrySkipNextAsync()),
        Cmd::Prev => wait(s.TrySkipPreviousAsync()),
        _ => return,
    };
}

unsafe fn refresh(full: bool) {
    use windows::Devices::Radios::{Radio, RadioKind, RadioState};
    let mut snap = Snapshot::default();

    if full {
        if let Ok(radios) = wait(Radio::GetRadiosAsync()) {
            for r in radios {
                let on = r.State().ok() == Some(RadioState::On);
                match r.Kind() {
                    Ok(RadioKind::WiFi) => snap.wifi = Some(snap.wifi.unwrap_or(false) | on),
                    Ok(RadioKind::Bluetooth) => snap.bluetooth = Some(snap.bluetooth.unwrap_or(false) | on),
                    _ => {}
                }
            }
        }
    } else {
        let prev = snapshot();
        snap.wifi = prev.wifi;
        snap.bluetooth = prev.bluetooth;
    }
    snap.brightness = if full || snapshot().brightness.is_none() && crate::bright::available() {
        crate::bright::get()
    } else {
        snapshot().brightness
    };
    if let Some(dev) = device() {
        snap.audio = device_name(&dev);
        if let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
            snap.volume = ep.GetMasterVolumeLevelScalar().ok();
            snap.muted = ep.GetMute().map(|b| b.as_bool()).unwrap_or(false);
        }
    }
    snap.ssid = wifi_ssid();
    snap.media = read_media();

    if let Ok(mut s) = SNAPSHOT.lock() {
        snap.rev = s.as_ref().map_or(0, |p| p.rev) + 1;
        *s = Some(snap);
    }
}

/// Обложка последнего трека: повторно не декодируем, пока трек тот же.
static COVER: Mutex<Option<(u64, Arc<Vec<u8>>)>> = Mutex::new(None);

unsafe fn read_media() -> Option<Media> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;
    use std::hash::{Hash, Hasher};
    watch_media();
    let mgr = media_manager()?;
    let s = mgr.GetCurrentSession().ok()?;
    let props = wait(s.TryGetMediaPropertiesAsync()).ok()?;
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
    if title.is_empty() && artist.is_empty() {
        return None;
    }
    let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).ok() == Some(Status::Playing);
    let (mut pos, mut dur) = (0.0f32, 0.0f32);
    if let Ok(tl) = s.GetTimelineProperties() {
        let secs = |t: windows::Foundation::TimeSpan| t.Duration as f64 / 1e7;
        let start = tl.StartTime().map(secs).unwrap_or(0.0);
        let mut p = tl.Position().map(secs).unwrap_or(0.0) - start;
        dur = (tl.EndTime().map(secs).unwrap_or(0.0) - start).max(0.0) as f32;
        // Позиция относится к моменту `LastUpdatedTime`, а не к «сейчас»: добавляем прошедшее.
        if playing {
            if let Ok(t) = tl.LastUpdatedTime() {
                // 100-нс интервалы с 1601 года (как FILETIME).
                const UNIX_OFFSET: f64 = 11_644_473_600.0;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0)
                    + UNIX_OFFSET;
                let behind = now - t.UniversalTime as f64 / 1e7;
                if (0.0..43_200.0).contains(&behind) {
                    p += behind;
                }
            }
        }
        pos = p as f32;
    }
    if dur > 0.0 {
        pos = pos.min(dur);
    }

    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&title, &artist).hash(&mut h);
    let key = h.finish();
    let cached = COVER.lock().ok().and_then(|c| c.as_ref().filter(|(k, _)| *k == key).map(|(_, a)| a.clone()));
    let art = cached.or_else(|| {
        let px = Arc::new(read_cover(&props)?);
        if let Ok(mut c) = COVER.lock() {
            *c = Some((key, px.clone()));
        }
        Some(px)
    });
    Some(Media { title, artist, playing, pos: pos.max(0.0), dur, at: Instant::now(), art, key })
}

/// Обложка из миниатюры медиасессии → квадрат `ART`×`ART` (обрезка по центру).
unsafe fn read_cover(props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<Vec<u8>> {
    use windows::Storage::Streams::DataReader;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::CLSCTX_INPROC_SERVER;

    let stream = wait(props.Thumbnail().ok()?.OpenReadAsync()).ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 16 << 20 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    wait(reader.LoadAsync(size)).ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;

    let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let ws = factory.CreateStream().ok()?;
    ws.InitializeFromMemory(&bytes).ok()?;
    let dec = factory.CreateDecoderFromStream(&ws, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
    let frame = dec.GetFrame(0).ok()?;
    let (mut w, mut h) = (0u32, 0u32);
    frame.GetSize(&mut w, &mut h).ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    let k = (ART as f32 / w as f32).max(ART as f32 / h as f32);
    let nw = ((w as f32 * k).ceil() as u32).max(ART);
    let nh = ((h as f32 * k).ceil() as u32).max(ART);
    let scaler = factory.CreateBitmapScaler().ok()?;
    scaler.Initialize(&frame, nw, nh, WICBitmapInterpolationModeFant).ok()?;
    let conv = factory.CreateFormatConverter().ok()?;
    conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
        .ok()?;
    let rect = WICRect { X: ((nw - ART) / 2) as i32, Y: ((nh - ART) / 2) as i32, Width: ART as i32, Height: ART as i32 };
    let mut buf = vec![0u8; (ART * ART * 4) as usize];
    conv.CopyPixels(&rect, ART * 4, &mut buf).ok()?;
    Some(buf)
}
