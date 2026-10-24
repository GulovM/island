//! Фоновый поток для медленных системных вызовов (WinRT, COM): радио Wi-Fi/Bluetooth,
//! громкость, медиасессия. Главный поток шлёт команды и читает снимок состояния; по готовности
//! результата окну уходит `WM_APP + 3`. Потока в простое нет: он спит на `recv`.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::{CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
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

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
    /// Режим полёта: все радио выключены; `None` — радио в системе нет.
    pub airplane: Option<bool>,
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
mod media;
mod net;
mod volume;
mod wifi;
mod winrt;

pub use devices::{list, status, DevItem, ListKind};
pub use media::{Media, Timeline, ART};
pub use net::{net_status, NetStatus};

use audio_policy::audio_set_default;
use bluetooth::{bluetooth_devices, bt_spawn};
use devices::{audio_devices, device, device_name, endpoint, store_list};
use media::{media_command, read_media, refresh_media};
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
    /// Режим полёта: выключить все радио или вернуть те, что были включены.
    Airplane(bool),
    Volume(f32),
    Brightness(f32),
    Mute(bool),
    PlayPause,
    Next,
    Prev,
    /// Перемотка трека на позицию в секундах от начала.
    Seek(f32),
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
            let batch_full = batch.iter().any(|c| matches!(c, Cmd::Refresh | Cmd::Wifi(_) | Cmd::Bluetooth(_) | Cmd::Airplane(_)));
            let only_media = batch.iter().all(|c| matches!(c, Cmd::Media | Cmd::Net));
            let wants_net = batch.iter().any(|c| matches!(c, Cmd::Net));
            let only_net = batch.iter().all(|c| matches!(c, Cmd::Net));
            let last_volume = batch.iter().rposition(|c| matches!(c, Cmd::Volume(_)));
            let last_bright = batch.iter().rposition(|c| matches!(c, Cmd::Brightness(_)));
            let last_seek = batch.iter().rposition(|c| matches!(c, Cmd::Seek(_)));
            for (i, cmd) in batch.into_iter().enumerate() {
                match cmd {
                    Cmd::Volume(_) if Some(i) != last_volume => {}
                    Cmd::Brightness(_) if Some(i) != last_bright => {}
                    Cmd::Seek(_) if Some(i) != last_seek => {}
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
        Cmd::Airplane(on) => set_airplane(on, radio_access),
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
        Cmd::PlayPause | Cmd::Next | Cmd::Prev | Cmd::Seek(_) => media_command(cmd),
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

/// Радио, которые были включены до режима полёта (`RadioKind` как число), чтобы вернуть их при выходе.
static BEFORE_AIRPLANE: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// Режим полёта: выключает все радио, при выходе включает те, что работали до него (если запомненных
/// нет, например после перезапуска, — Wi-Fi и Bluetooth).
pub(super) unsafe fn set_airplane(on: bool, access: &mut bool) {
    use windows::Devices::Radios::{Radio, RadioKind, RadioState};
    if !*access {
        let _ = wait(Radio::RequestAccessAsync());
        *access = true;
    }
    let Ok(radios) = wait(Radio::GetRadiosAsync()) else { return };
    let mut before = BEFORE_AIRPLANE.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        before.clear();
        before.extend(radios.into_iter().filter(|r| r.State().ok() == Some(RadioState::On)).filter_map(|r| r.Kind().ok()).map(|k| k.0));
    }
    let restore = if before.is_empty() { vec![RadioKind::WiFi.0, RadioKind::Bluetooth.0] } else { before.clone() };
    let Ok(radios) = wait(Radio::GetRadiosAsync()) else { return };
    for r in radios {
        let Ok(kind) = r.Kind() else { continue };
        let state = if on {
            RadioState::Off
        } else if restore.contains(&kind.0) {
            RadioState::On
        } else {
            continue;
        };
        if let Err(e) = wait(r.SetStateAsync(state)) {
            log::error(&format!("не удалось переключить радио: {e}"));
        }
    }
    if !on {
        before.clear();
    }
}

unsafe fn refresh(full: bool) {
    use windows::Devices::Radios::{Radio, RadioKind, RadioState};
    let mut snap = Snapshot::default();

    if full {
        if let Ok(radios) = wait(Radio::GetRadiosAsync()) {
            for r in radios {
                let on = r.State().ok() == Some(RadioState::On);
                snap.airplane = Some(snap.airplane.unwrap_or(true) & !on);
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
        snap.airplane = prev.airplane;
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
