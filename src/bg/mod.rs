//! Фоновый поток для системных вызовов macOS: радио Wi-Fi/Bluetooth, громкость,
//! медиаплееры и уведомления. Главный поток шлёт команды и читает снимок состояния;
//! по готовности результата окну уходит `WM_BG`. Поток спит на `recv`.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
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
    /// Подключено ли хотя бы одно Bluetooth-устройство (классическое или BLE).
    pub bt_connected: bool,
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
    /// Системные уведомления Windows (новые сверху).
    pub notifications: Vec<notif::SysNotif>,
    /// Счётчик обновлений списка уведомлений и доступа к ним.
    pub notif_rev: u64,
    /// Статус доступа к центру уведомлений Windows.
    pub notif_access: notif::Access,
    /// Включён ли в Windows режим «Не беспокоить».
    pub dnd_active: bool,
    /// Сколько раз снимок обновлялся (чтобы вкладка понимала, что есть новое).
    pub rev: u64,
}

mod audio_policy;
mod bluetooth;
mod devices;
mod meter;
mod media;
mod net;
pub mod notif;
mod volume;
mod wifi;

pub use devices::{list, status, DevItem, ListKind};
pub use meter::AudioMeter;
pub use media::{Media, Timeline, ART};
pub use net::{net_status, NetStatus};
pub use notif::{Access as NotifAccess, SysNotif, ICON_PX as NOTIF_ICON_PX};

use audio_policy::audio_set_default;
use bluetooth::{bluetooth_devices, bt_any_connected, bt_power_on, bt_spawn, ensure_bt_watch, set_bt_power};
use devices::{audio_devices, store_list};
use media::{media_command, read_media_pair, refresh_media};
use volume::watch_volume;
use wifi::{set_wifi_power, watch_wlan, wifi_disconnect, wifi_networks, wifi_spawn, wifi_status};

#[cfg(target_os = "macos")]
fn osascript(script: &str) -> Option<String> {
    let out = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn osascript(_script: &str) -> Option<String> {
    None
}

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Извлекает пары `(ключ, значение)` из XML-представления Apple plist.
pub(super) fn parse_plist_xml_dict(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur_key: Option<String> = None;
    let mut rest = xml;

    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];

        let tag_name = tag.split_whitespace().next().unwrap_or("");
        if tag_name.starts_with('/') || tag_name.starts_with('?') || tag_name.starts_with('!') {
            continue;
        }
        if tag_name == "true/" || tag_name == "false/" {
            if let Some(k) = cur_key.take() {
                out.push((k, tag_name.trim_end_matches('/').to_string()));
            }
            continue;
        }
        if matches!(tag_name, "key" | "string" | "integer" | "real") {
            let end_tag = format!("</{tag_name}>");
            if let Some(end) = rest.find(&end_tag) {
                let text = unescape_xml(rest[..end].trim());
                rest = &rest[end + end_tag.len()..];
                if tag_name == "key" {
                    cur_key = Some(text);
                } else if let Some(k) = cur_key.take() {
                    out.push((k, text));
                }
            }
        }
    }
    out
}

fn parse_volume_settings(out: &str) -> (Option<f32>, bool) {
    let mut vol: Option<f32> = None;
    let mut muted = false;
    for part in out.split(',') {
        let Some((k, v)) = part.trim().split_once(':') else { continue };
        let key = k.trim();
        let val = v.trim();
        if key == "output volume" {
            if let Ok(n) = val.parse::<f32>() {
                vol = Some((n / 100.0).clamp(0.0, 1.0));
            }
        } else if key == "output muted" {
            muted = val.eq_ignore_ascii_case("true");
        }
    }
    (vol, muted)
}

fn read_volume_settings() -> (Option<f32>, bool) {
    osascript("get volume settings").map(|s| parse_volume_settings(&s)).unwrap_or((None, false))
}

fn set_volume_scalar(v: f32) {
    let pct = (v.clamp(0.0, 1.0) * 100.0).round() as u32;
    let _ = osascript(&format!("set volume output volume {pct}"));
}

fn set_volume_muted(muted: bool) {
    let _ = osascript(&format!("set volume output muted {muted}"));
}

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
    /// Изменился список подключённых Bluetooth-устройств (из `DeviceWatcher`).
    #[allow(dead_code)]
    BtConn,
    /// Обновить только медиа: приходит по событиям медиасессии (индикатор в свёрнутой форме).
    Media,
    /// Перечитать состояние сети (значок Wi-Fi на свёрнутом острове).
    Net,
    Wifi(bool),
    Bluetooth(bool),
    /// Обновить список системных уведомлений (по событию WinRT).
    NotifRefresh,
    /// Запросить разрешение на чтение уведомлений Windows.
    NotifRequestAccess,
    /// Удалить уведомление по его id в Windows.
    NotifRemove(u32),
    /// Удалить несколько уведомлений по их id в Windows.
    NotifRemoveMany(Vec<u32>),
    /// Обновить статус режима «Не беспокоить» (по событию WinRT).
    DndRefresh,
    /// Переключить системный режим «Не беспокоить».
    Dnd(bool),
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
        let mut pending = PendingRadios::default();
        unsafe {
            crate::bright::probe();
            watch_volume();
            net::watch_network();
            watch_wlan();
            notif::init();
            refresh(true, &mut pending);
            refresh_notifs();
            refresh_dnd(&mut pending.dnd);
            net::refresh_net();
            crate::sys::trim_working_set();
            notify();
        }
        let mut last_radios = std::time::Instant::now();
        let mut last_notif_poll = std::time::Instant::now();
        loop {
            let batch = match rx.recv_timeout(std::time::Duration::from_millis(900)) {
                Ok(first) => {
                    let mut b = vec![first];
                    while let Ok(c) = rx.try_recv() {
                        b.push(c);
                    }
                    b
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // WinRT NotificationChanged в unpackaged-приложениях может не срабатывать при свёрнутом окне,
                    // поэтому раз в ~0.9 с тихо проверяем уведомления и DND; UI просыпается только при изменениях.
                    let mut changed = false;
                    unsafe {
                        changed |= refresh_notifs();
                        changed |= refresh_dnd(&mut pending.dnd);
                    }
                    if changed {
                        notify();
                    }
                    last_notif_poll = std::time::Instant::now();
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
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
                        | Cmd::BtConn
                        | Cmd::Media
                        | Cmd::PlayPause
                        | Cmd::Next
                        | Cmd::Prev
                        | Cmd::Seek(_)
                        | Cmd::PrevSession
                        | Cmd::NextSession
                        | Cmd::ResetSession
                        | Cmd::NotifRefresh
                        | Cmd::NotifRequestAccess
                        | Cmd::NotifRemove(_)
                        | Cmd::NotifRemoveMany(_)
                        | Cmd::DndRefresh
                        | Cmd::Dnd(_)
                )
            });
            let wants_media = batch.iter().any(|c| {
                matches!(
                    c,
                    Cmd::Media
                        | Cmd::PlayPause
                        | Cmd::Next
                        | Cmd::Prev
                        | Cmd::Seek(_)
                        | Cmd::PrevSession
                        | Cmd::NextSession
                        | Cmd::ResetSession
                )
            });
            let wants_net = batch.iter().any(|c| matches!(c, Cmd::Net | Cmd::Radios));
            let wants_bt_conn = batch.iter().any(|c| matches!(c, Cmd::BtConn));
            let explicit_notif = full
                || batch.iter().any(|c| {
                    matches!(
                        c,
                        Cmd::NotifRefresh
                            | Cmd::NotifRequestAccess
                            | Cmd::NotifRemove(_)
                            | Cmd::NotifRemoveMany(_)
                    )
                });
            let explicit_dnd = full || batch.iter().any(|c| matches!(c, Cmd::DndRefresh | Cmd::Dnd(_)));
            let periodic_notif = last_notif_poll.elapsed() >= std::time::Duration::from_millis(900);
            let last_volume = batch.iter().rposition(|c| matches!(c, Cmd::Volume(_)));
            let last_bright = batch.iter().rposition(|c| matches!(c, Cmd::Brightness(_)));
            let last_seek = batch.iter().rposition(|c| matches!(c, Cmd::Seek(_)));
            for (i, cmd) in batch.into_iter().enumerate() {
                match cmd {
                    Cmd::Volume(_) if Some(i) != last_volume => {}
                    Cmd::Brightness(_) if Some(i) != last_bright => {}
                    Cmd::Seek(_) if Some(i) != last_seek => {}
                    cmd => unsafe { run(cmd, &mut pending) },
                }
            }
            if has_fast {
                notify();
            }
            if needs_refresh {
                unsafe { refresh(wants_radios, &mut pending) };
            } else if wants_radios {
                unsafe { refresh_radios(&mut pending) };
            } else if wants_media {
                unsafe { refresh_media() };
            }
            if wants_radios {
                last_radios = std::time::Instant::now();
            }
            if wants_net || full {
                unsafe { net::refresh_net() };
            }
            let mut notif_changed = false;
            if explicit_notif || periodic_notif {
                notif_changed |= unsafe { refresh_notifs() };
                last_notif_poll = std::time::Instant::now();
            }
            let mut dnd_changed = false;
            if explicit_dnd || periodic_notif {
                dnd_changed |= unsafe { refresh_dnd(&mut pending.dnd) };
            }
            let mut bt_conn_changed = false;
            if wants_bt_conn && !needs_refresh && !wants_radios {
                if let Ok(mut s) = SNAPSHOT.lock() {
                    let mut snap = s.clone().unwrap_or_default();
                    let next = snap.bluetooth == Some(true) && bt_any_connected();
                    if snap.bt_connected != next {
                        snap.bt_connected = next;
                        snap.rev += 1;
                        *s = Some(snap);
                        bt_conn_changed = true;
                    }
                }
            }
            if full {
                // Загрузка WinRT раздувает рабочий набор; в простое он не нужен.
                crate::sys::trim_working_set();
            }
            if needs_refresh
                || wants_radios
                || wants_media
                || wants_net
                || (explicit_notif && notif_changed)
                || (explicit_dnd && dnd_changed)
                || (periodic_notif && (notif_changed || dnd_changed))
                || bt_conn_changed
                || full
            {
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
    dnd: Option<(bool, std::time::Instant)>,
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

fn schedule_dnd_recheck() {
    let _ = std::thread::Builder::new().name("island-dnd-recheck".into()).spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(250));
        send(Cmd::DndRefresh);
        std::thread::sleep(std::time::Duration::from_millis(600));
        send(Cmd::DndRefresh);
    });
}

unsafe fn run(cmd: Cmd, pending: &mut PendingRadios) {
    match cmd {
        Cmd::Tick | Cmd::Media | Cmd::Net | Cmd::Radios | Cmd::BtConn | Cmd::NotifRefresh | Cmd::DndRefresh => {}
        Cmd::NotifRequestAccess => {
            let _ = notif::request_access();
        }
        Cmd::NotifRemove(id) => notif::remove_notification(id),
        Cmd::NotifRemoveMany(ids) => {
            for id in ids {
                notif::remove_notification(id);
            }
        }
        Cmd::Dnd(on) => {
            if notif::set_dnd(on) {
                pending.dnd = Some((on, std::time::Instant::now() + std::time::Duration::from_millis(1500)));
                schedule_dnd_recheck();
            } else {
                pending.dnd = None;
            }
        }
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
            if set_wifi_power(on) {
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
            if set_bt_power(on) {
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
            if set_airplane(on) {
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
            set_volume_scalar(clamped);
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
            set_volume_muted(m);
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

/// Какие радио работали до «режима полёта»: `(wifi, bluetooth)`.
static BEFORE_AIRPLANE: Mutex<(bool, bool)> = Mutex::new((true, true));

/// Выключает все доступные радио на macOS (`on = true`) или восстанавливает прежнее состояние.
pub(super) unsafe fn set_airplane(on: bool) -> bool {
    let cur_wifi = wifi_status().map(|s| s.radio_on).unwrap_or(true);
    let cur_bt = bt_power_on().unwrap_or(false);
    let mut before = BEFORE_AIRPLANE.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        *before = (cur_wifi, cur_bt);
        let w = set_wifi_power(false);
        let b = set_bt_power(false);
        w || b
    } else {
        let (want_w, want_b) = *before;
        let w = want_w && set_wifi_power(true);
        let b = want_b && set_bt_power(true);
        w || b
    }
}

/// Обновляет поля радиомодулей, SSID и Bluetooth в `snap`.
unsafe fn update_radio_fields(
    snap: &mut Snapshot,
    prev: &Snapshot,
    query_radios: bool,
    pending: &mut PendingRadios,
) {
    let bt_actual = if query_radios { bt_power_on().or(prev.bluetooth) } else { prev.bluetooth };

    let wlan = wifi_status();
    let (raw_wifi, raw_ssid) = match wlan {
        Some(ws) => {
            let ssid = ws.ssid;
            let wifi_on = if ws.radio_known {
                ws.radio_on || ssid.is_some()
            } else {
                ssid.is_some() || prev.wifi.unwrap_or(false)
            };
            (Some(wifi_on), ssid)
        }
        None => (prev.wifi, None),
    };

    snap.wifi = PendingRadios::resolve(&mut pending.wifi, raw_wifi);
    snap.ssid = if snap.wifi == Some(false) { None } else { raw_ssid };
    snap.bluetooth = PendingRadios::resolve(&mut pending.bluetooth, bt_actual);
    if snap.bluetooth == Some(true) {
        ensure_bt_watch();
    }
    snap.bt_connected = snap.bluetooth == Some(true) && bt_any_connected();

    let raw_air = match (snap.wifi, snap.bluetooth) {
        (Some(w), Some(b)) => Some(!w && !b),
        (Some(w), None) => Some(!w),
        (None, Some(b)) => Some(!b),
        (None, None) => prev.airplane,
    };
    let resolved_air = PendingRadios::resolve(&mut pending.airplane, raw_air);
    snap.airplane = match resolved_air {
        Some(true) if snap.wifi == Some(true) || snap.bluetooth == Some(true) => Some(false),
        other => other,
    };
}

unsafe fn refresh_radios(pending: &mut PendingRadios) {
    let prev = snapshot();
    let mut snap = prev.clone();
    update_radio_fields(&mut snap, &prev, true, pending);
    if let Ok(mut s) = SNAPSHOT.lock() {
        snap.rev = s.as_ref().map_or(0, |p| p.rev) + 1;
        *s = Some(snap);
    }
}

unsafe fn refresh_notifs() -> bool {
    let (access, list) = notif::poll_notifications();
    let changed = if let Ok(mut s) = SNAPSHOT.lock() {
        let mut snap = s.clone().unwrap_or_default();
        if snap.notif_access != access || snap.notifications != list {
            snap.notif_access = access;
            snap.notifications = list.clone();
            snap.notif_rev += 1;
            snap.rev += 1;
            *s = Some(snap);
            true
        } else {
            false
        }
    } else {
        false
    };
    if changed && crate::sys::notif_banners_active() {
        for n in &list {
            crate::sys::suppress_notif_banner(&n.app_id);
        }
    }
    changed
}

unsafe fn refresh_dnd(pending_dnd: &mut Option<(bool, std::time::Instant)>) -> bool {
    let actual = notif::poll_dnd();
    if let Some(dnd) = PendingRadios::resolve(pending_dnd, actual) {
        if let Ok(mut s) = SNAPSHOT.lock() {
            let mut snap = s.clone().unwrap_or_default();
            if snap.dnd_active != dnd {
                snap.dnd_active = dnd;
                snap.rev += 1;
                *s = Some(snap);
                return true;
            }
        }
    }
    false
}

unsafe fn refresh(full: bool, pending: &mut PendingRadios) {
    let prev = snapshot();
    let mut snap = Snapshot {
        notifications: prev.notifications.clone(),
        notif_rev: prev.notif_rev,
        notif_access: prev.notif_access,
        dnd_active: prev.dnd_active,
        ..Snapshot::default()
    };

    update_radio_fields(&mut snap, &prev, full, pending);
    snap.brightness = if full || prev.brightness.is_none() && crate::bright::available() {
        crate::bright::get()
    } else {
        prev.brightness
    };
    let devs = audio_devices();
    snap.audio = devs
        .iter()
        .find(|d| d.active)
        .or_else(|| devs.first())
        .map(|d| d.name.clone());
    let (vol, muted) = read_volume_settings();
    snap.volume = vol.or(prev.volume);
    snap.muted = if vol.is_some() { muted } else { prev.muted };
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

    #[test]
    fn parse_volume_settings_parses_slider_and_mute() {
        let (v, m) = parse_volume_settings("output volume:65, input volume:50, alert volume:100, output muted:true");
        assert!(m);
        assert!((v.unwrap() - 0.65).abs() < 0.001);
        let (v2, m2) = parse_volume_settings("output volume:0, output muted:false");
        assert!(!m2);
        assert_eq!(v2, Some(0.0));
    }

    #[test]
    fn parse_plist_xml_dict_extracts_notification_fields() {
        let xml = r#"<plist version="1.0"><dict><key>titl</key><string>Alice &amp; Bob</string><key>body</key><string>Hello &lt;Mac&gt;</string></dict></plist>"#;
        let pairs = parse_plist_xml_dict(xml);
        assert_eq!(
            pairs,
            vec![
                ("titl".into(), "Alice & Bob".into()),
                ("body".into(), "Hello <Mac>".into()),
            ]
        );
    }

    /// Живая диагностика состояния радио и Wi-Fi (только чтение).
    #[test]
    #[ignore]
    fn live_radio_snapshot() {
        unsafe {
            let mut snap = Snapshot::default();
            let prev = Snapshot::default();
            let mut pending = PendingRadios::default();
            update_radio_fields(&mut snap, &prev, true, &mut pending);
            println!(
                "wifi={:?} ssid={:?} bluetooth={:?} airplane={:?}",
                snap.wifi, snap.ssid, snap.bluetooth, snap.airplane
            );
        }
    }
}
