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
    /// Сессия карточки плеера: выбранная стрелками, иначе та, что играет.
    pub media: Option<Media>,
    /// Сессия, которая реально звучит, — для свёрнутого острова. Листание источников в карточке
    /// её не меняет.
    pub live_media: Option<Media>,
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
use media::{media_command, read_media_pair, refresh_media};
use volume::watch_volume;
use wifi::{watch_wlan, wifi_disconnect, wifi_networks, wifi_spawn, wifi_status};
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
    /// Лёгкое обновление раз в секунду: громкость, медиа, Wi-Fi (и периодически радио).
    Tick,
    /// Обновить состояние радиомодулей и Wi-Fi (по событиям или после переключения).
    Radios,
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
    /// Показать в карточке плеера предыдущий / следующий источник звука (без смены воспроизведения).
    PrevSession,
    NextSession,
    /// Вернуть карточку плеера к сессии, которая реально играет (снять выбор стрелками).
    ResetSession,
}

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);
static TX: OnceLock<Sender<Cmd>> = OnceLock::new();

pub fn snapshot() -> Snapshot {
    SNAPSHOT.lock().ok().and_then(|s| s.clone()).unwrap_or_default()
}

/// Возвращает карточку плеера к играющей сессии, если её переключали стрелками. Без выбора
/// ничего не отправляет: вызывается на каждую смену вкладки.
pub fn reset_card_session() {
    if media::pin_set() {
        send(Cmd::ResetSession);
    }
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
        let mut radio_access = false;
        let mut pending = PendingRadios::default();
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            crate::bright::probe();
            watch_volume();
            net::watch_network();
            watch_wlan();
            refresh(true, &mut radio_access, &mut pending);
            net::refresh_net();
            crate::sys::trim_working_set();
            notify();
        }
        let mut last_radios = std::time::Instant::now();
        while let Ok(first) = rx.recv() {
            let mut batch = vec![first];
            while let Ok(c) = rx.try_recv() {
                batch.push(c);
            }
            // Слайдер громкости шлёт десятки значений — исполняем только последнее.
            let full = batch.iter().any(|c| matches!(c, Cmd::Refresh | Cmd::Wifi(_) | Cmd::Bluetooth(_) | Cmd::Airplane(_)));
            let has_radios = batch.iter().any(|c| matches!(c, Cmd::Radios));
            let tick_radios = batch.iter().any(|c| matches!(c, Cmd::Tick)) && last_radios.elapsed() >= std::time::Duration::from_secs(2);
            let wants_radios = full || has_radios || tick_radios;
            let has_fast = batch.iter().any(|c| matches!(c, Cmd::Volume(_) | Cmd::Brightness(_) | Cmd::Mute(_)));
            // Листание источников меняет только медиа: полный опрос (радио, громкость) не нужен.
            let needs_refresh = batch.iter().any(|c| {
                !matches!(
                    c,
                    Cmd::Volume(_)
                        | Cmd::Brightness(_)
                        | Cmd::Mute(_)
                        | Cmd::Net
                        | Cmd::Radios
                        | Cmd::Media
                        | Cmd::PrevSession
                        | Cmd::NextSession
                        | Cmd::ResetSession
                )
            });
            let wants_media = batch.iter().any(|c| matches!(c, Cmd::Media | Cmd::PrevSession | Cmd::NextSession | Cmd::ResetSession));
            let wants_net = batch.iter().any(|c| matches!(c, Cmd::Net | Cmd::Radios));
            let last_volume = batch.iter().rposition(|c| matches!(c, Cmd::Volume(_)));
            let last_bright = batch.iter().rposition(|c| matches!(c, Cmd::Brightness(_)));
            let last_seek = batch.iter().rposition(|c| matches!(c, Cmd::Seek(_)));
            for (i, cmd) in batch.into_iter().enumerate() {
                match cmd {
                    Cmd::Volume(_) if Some(i) != last_volume => {}
                    Cmd::Brightness(_) if Some(i) != last_bright => {}
                    Cmd::Seek(_) if Some(i) != last_seek => {}
                    cmd => unsafe { run(cmd, &mut radio_access, &mut pending) },
                }
            }
            if has_fast {
                notify();
            }
            if needs_refresh {
                unsafe { refresh(wants_radios, &mut radio_access, &mut pending) };
            } else if wants_radios {
                unsafe { refresh_radios(&mut radio_access, &mut pending) };
            } else if wants_media {
                unsafe { refresh_media() };
            }
            if wants_radios {
                last_radios = std::time::Instant::now();
            }
            if wants_net || full {
                unsafe { net::refresh_net() };
            }
            if full {
                // Загрузка WinRT раздувает рабочий набор; в простое он не нужен.
                crate::sys::trim_working_set();
            }
            if needs_refresh || wants_radios || wants_media || wants_net || full {
                notify();
            }
        }
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить фоновый поток: {e}"));
    }
}

/// Удерживает целевое состояние радио на короткое время после переключения:
/// Windows завершает SetStateAsync раньше, чем драйвер обновляет Radio::State / WLAN_RADIO_STATE.
#[derive(Default)]
struct PendingRadios {
    wifi: Option<(bool, std::time::Instant)>,
    bluetooth: Option<(bool, std::time::Instant)>,
    airplane: Option<(bool, std::time::Instant)>,
}

impl PendingRadios {
    fn resolve(slot: &mut Option<(bool, std::time::Instant)>, actual: Option<bool>) -> Option<bool> {
        if let Some((expected, until)) = *slot {
            if actual == Some(expected) || std::time::Instant::now() >= until {
                *slot = None;
                return actual;
            }
            return Some(expected);
        }
        actual
    }
}

fn schedule_radio_recheck() {
    let _ = std::thread::Builder::new().name("island-radio-recheck".into()).spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(600));
        send(Cmd::Radios);
        std::thread::sleep(std::time::Duration::from_millis(1000));
        send(Cmd::Radios);
    });
}

unsafe fn run(cmd: Cmd, radio_access: &mut bool, pending: &mut PendingRadios) {
    match cmd {
        Cmd::Tick | Cmd::Media | Cmd::Net | Cmd::Radios => {}
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
        Cmd::Wifi(on) => {
            if set_radio(windows::Devices::Radios::RadioKind::WiFi, on, radio_access) {
                let until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
                pending.wifi = Some((on, until));
                if on {
                    pending.airplane = None;
                }
                schedule_radio_recheck();
            } else {
                pending.wifi = None;
            }
        }
        Cmd::Bluetooth(on) => {
            if set_radio(windows::Devices::Radios::RadioKind::Bluetooth, on, radio_access) {
                let until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
                pending.bluetooth = Some((on, until));
                if on {
                    pending.airplane = None;
                }
                schedule_radio_recheck();
            } else {
                pending.bluetooth = None;
            }
        }
        Cmd::Airplane(on) => {
            if set_airplane(on, radio_access) {
                let until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
                pending.airplane = Some((on, until));
                if on {
                    pending.wifi = Some((false, until));
                    pending.bluetooth = Some((false, until));
                } else {
                    pending.wifi = None;
                    pending.bluetooth = None;
                }
                schedule_radio_recheck();
            } else {
                pending.airplane = None;
            }
        }
        Cmd::Volume(v) => {
            let clamped = v.clamp(0.0, 1.0);
            if let Some(ep) = endpoint() {
                let _ = ep.SetMasterVolumeLevelScalar(clamped, std::ptr::null());
            }
            if let Ok(mut s) = SNAPSHOT.lock() {
                let mut snap = s.clone().unwrap_or_default();
                snap.volume = Some(clamped);
                snap.rev += 1;
                *s = Some(snap);
            }
        }
        Cmd::Brightness(v) => {
            let clamped = v.clamp(0.0, 1.0);
            crate::bright::set(clamped);
            // Снимок обновляем сами, иначе после удержания ползунок вернётся к старому значению.
            if let Ok(mut s) = SNAPSHOT.lock() {
                let mut snap = s.clone().unwrap_or_default();
                snap.brightness = Some(clamped);
                snap.rev += 1;
                *s = Some(snap);
            }
        }
        Cmd::Mute(m) => {
            if let Some(ep) = endpoint() {
                let _ = ep.SetMute(m, std::ptr::null());
            }
            if let Ok(mut s) = SNAPSHOT.lock() {
                let mut snap = s.clone().unwrap_or_default();
                snap.muted = m;
                snap.rev += 1;
                *s = Some(snap);
            }
        }
        Cmd::PlayPause | Cmd::Next | Cmd::Prev | Cmd::Seek(_) => media_command(cmd),
        Cmd::PrevSession => media::cycle_session(-1),
        Cmd::NextSession => media::cycle_session(1),
        Cmd::ResetSession => media::reset_pin(),
    }
}

unsafe fn ensure_radio_access(access: &mut bool) {
    use windows::Devices::Radios::Radio;
    if !*access {
        let _ = wait(Radio::RequestAccessAsync());
        *access = true;
    }
}

/// Поддерживает подписку `Radio::StateChanged` на текущем списке радиомодулей, чтобы
/// переключение Wi-Fi / Bluetooth в Windows сразу обновляло снимок.
unsafe fn ensure_radio_watch(radios: &[windows::Devices::Radios::Radio]) {
    use windows::Devices::Radios::Radio;
    use windows::Foundation::TypedEventHandler;

    static WATCHED: Mutex<Vec<(Radio, i64, i32, String)>> = Mutex::new(Vec::new());

    let sig: Vec<(i32, String)> = radios
        .iter()
        .map(|r| (r.Kind().ok().map_or(-1, |k| k.0), r.Name().map(|n| n.to_string()).unwrap_or_default()))
        .collect();
    let Ok(mut guard) = WATCHED.lock() else { return };
    let unchanged = guard.len() == sig.len() && guard.iter().zip(&sig).all(|(a, b)| a.2 == b.0 && a.3 == b.1);
    if unchanged {
        return;
    }
    for (r, tok, _, _) in guard.drain(..) {
        let _ = r.RemoveStateChanged(tok);
    }
    for (r, (kind, name)) in radios.iter().zip(sig) {
        let handler = TypedEventHandler::new(|_, _| {
            send(Cmd::Radios);
            Ok(())
        });
        if let Ok(tok) = r.StateChanged(&handler) {
            guard.push((r.clone(), tok, kind, name));
        }
    }
}

pub(super) unsafe fn set_radio(kind: windows::Devices::Radios::RadioKind, on: bool, access: &mut bool) -> bool {
    use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioState};
    ensure_radio_access(access);
    let Ok(radios) = wait(Radio::GetRadiosAsync()) else { return false };
    let mut changed = false;
    for r in radios {
        if r.Kind().ok() == Some(kind) {
            let state = if on { RadioState::On } else { RadioState::Off };
            match wait(r.SetStateAsync(state)) {
                Ok(RadioAccessStatus::Allowed) => changed = true,
                Ok(status) => log::error(&format!("переключение радио отклонено: {status:?}")),
                Err(e) => log::error(&format!("не удалось переключить радио: {e}")),
            }
        }
    }
    changed
}

/// Радио, которые были включены до режима полёта (`RadioKind` как число), чтобы вернуть их при выходе.
static BEFORE_AIRPLANE: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// Режим полёта: выключает все радио, при выходе включает те, что работали до него (если запомненных
/// нет, например после перезапуска, — Wi-Fi и Bluetooth).
pub(super) unsafe fn set_airplane(on: bool, access: &mut bool) -> bool {
    use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
    ensure_radio_access(access);
    let Ok(radios) = wait(Radio::GetRadiosAsync()) else { return false };
    let mut before = BEFORE_AIRPLANE.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        before.clear();
        before.extend(radios.into_iter().filter(|r| r.State().ok() == Some(RadioState::On)).filter_map(|r| r.Kind().ok()).map(|k| k.0));
    }
    let restore = if before.is_empty() { vec![RadioKind::WiFi.0, RadioKind::Bluetooth.0] } else { before.clone() };
    let Ok(radios) = wait(Radio::GetRadiosAsync()) else { return false };
    let mut changed = false;
    for r in radios {
        let Ok(kind) = r.Kind() else { continue };
        let state = if on {
            RadioState::Off
        } else if restore.contains(&kind.0) {
            RadioState::On
        } else {
            continue;
        };
        match wait(r.SetStateAsync(state)) {
            Ok(RadioAccessStatus::Allowed) => changed = true,
            Ok(status) => log::error(&format!("режим полёта отклонён: {status:?}")),
            Err(e) => log::error(&format!("не удалось переключить радио: {e}")),
        }
    }
    if !on {
        before.clear();
    }
    changed
}

/// Обновляет поля радиомодулей и Wi-Fi в `snap`, опираясь на Win32 WLAN API и (при `query_winrt`)
/// WinRT `Radio::GetRadiosAsync()`. При сбое WinRT сохраняет прежние значения вместо сброса в `None`.
unsafe fn update_radio_fields(
    snap: &mut Snapshot,
    prev: &Snapshot,
    query_winrt: bool,
    access: &mut bool,
    pending: &mut PendingRadios,
) {
    use windows::Devices::Radios::{Radio, RadioKind, RadioState};

    let mut winrt_wifi = None;
    let mut winrt_bt = prev.bluetooth;
    let mut winrt_airplane = prev.airplane;

    if query_winrt {
        ensure_radio_access(access);
        if let Ok(coll) = wait(Radio::GetRadiosAsync()) {
            let radios: Vec<Radio> = coll.into_iter().collect();
            ensure_radio_watch(&radios);
            let mut bt = None;
            let mut air = None;
            for r in &radios {
                let on = r.State().ok() == Some(RadioState::On);
                air = Some(air.unwrap_or(true) & !on);
                match r.Kind() {
                    Ok(RadioKind::WiFi) => winrt_wifi = Some(winrt_wifi.unwrap_or(false) | on),
                    Ok(RadioKind::Bluetooth) => bt = Some(bt.unwrap_or(false) | on),
                    _ => {}
                }
            }
            winrt_bt = bt;
            winrt_airplane = air;
        }
    }

    let wlan = wifi_status();
    let (raw_wifi, raw_ssid) = match wlan {
        Some(ws) => {
            let ssid = ws.ssid;
            let wifi_on = if ws.radio_known {
                ws.radio_on || ssid.is_some()
            } else {
                ssid.is_some() || winrt_wifi.unwrap_or(prev.wifi.unwrap_or(false))
            };
            (Some(wifi_on), ssid)
        }
        None => (winrt_wifi.or(prev.wifi), None),
    };

    snap.wifi = PendingRadios::resolve(&mut pending.wifi, raw_wifi);
    snap.ssid = if snap.wifi == Some(false) { None } else { raw_ssid };
    snap.bluetooth = PendingRadios::resolve(&mut pending.bluetooth, winrt_bt);
    let resolved_air = PendingRadios::resolve(&mut pending.airplane, winrt_airplane);
    snap.airplane = match resolved_air {
        Some(true) if snap.wifi == Some(true) || snap.bluetooth == Some(true) => Some(false),
        other => other,
    };
}

unsafe fn refresh_radios(access: &mut bool, pending: &mut PendingRadios) {
    let prev = snapshot();
    let mut snap = prev.clone();
    update_radio_fields(&mut snap, &prev, true, access, pending);
    if let Ok(mut s) = SNAPSHOT.lock() {
        snap.rev = s.as_ref().map_or(0, |p| p.rev) + 1;
        *s = Some(snap);
    }
}

unsafe fn refresh(full: bool, access: &mut bool, pending: &mut PendingRadios) {
    let prev = snapshot();
    let mut snap = Snapshot::default();

    update_radio_fields(&mut snap, &prev, full, access, pending);
    snap.brightness = if full || prev.brightness.is_none() && crate::bright::available() {
        crate::bright::get()
    } else {
        prev.brightness
    };
    if let Some(dev) = device() {
        snap.audio = device_name(&dev);
        if let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
            snap.volume = ep.GetMasterVolumeLevelScalar().ok();
            snap.muted = ep.GetMute().map(|b| b.as_bool()).unwrap_or(false);
        }
    }
    (snap.media, snap.live_media) = read_media_pair();

    if let Ok(mut s) = SNAPSHOT.lock() {
        if !full {
            if let Some(cur_b) = s.as_ref().and_then(|p| p.brightness) {
                snap.brightness = Some(cur_b);
            }
        }
        snap.rev = s.as_ref().map_or(0, |p| p.rev) + 1;
        *s = Some(snap);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};
    use super::*;

    #[test]
    fn pending_radios_holds_expected_until_match_or_expiry() {
        let mut slot = Some((false, Instant::now() + Duration::from_secs(5)));
        // Пока драйвер ещё отдаёт старое состояние, удерживаем целевое.
        assert_eq!(PendingRadios::resolve(&mut slot, Some(true)), Some(false));
        assert!(slot.is_some());
        // Как только драйвер подтвердил целевое состояние, удержание снимается.
        assert_eq!(PendingRadios::resolve(&mut slot, Some(false)), Some(false));
        assert!(slot.is_none());

        // По истечении таймаута принимаем фактическое состояние.
        let mut expired = Some((false, Instant::now() - Duration::from_millis(1)));
        assert_eq!(PendingRadios::resolve(&mut expired, Some(true)), Some(true));
        assert!(expired.is_none());
    }

    /// Живая диагностика состояния радио и Wi-Fi (только чтение).
    #[test]
    #[ignore]
    fn live_radio_snapshot() {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let mut snap = Snapshot::default();
            let prev = Snapshot::default();
            let mut access = false;
            let mut pending = PendingRadios::default();
            update_radio_fields(&mut snap, &prev, true, &mut access, &mut pending);
            println!(
                "access={} wifi={:?} ssid={:?} bluetooth={:?} airplane={:?}",
                access, snap.wifi, snap.ssid, snap.bluetooth, snap.airplane
            );
        }
    }
}
