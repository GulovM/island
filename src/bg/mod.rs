//! Фоновый поток для медленных системных вызовов (WinRT, COM): радио Wi-Fi/Bluetooth,
//! громкость, медиасессия. Главный поток шлёт команды и читает снимок состояния; по готовности
//! результата окну уходит `WM_APP + 3`. Потока в простое нет: он спит на `recv`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::log;

/// Сообщение главному окну: снимок состояния обновился.
pub const WM_BG: u32 = WM_APP + 3;
/// Громкость или «без звука» изменились (из любого источника): wparam — громкость в ‰, lparam — mute.
pub const WM_VOLUME: u32 = WM_APP + 4;

static HWND_MAIN: AtomicIsize = AtomicIsize::new(0);

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
mod net;
mod winrt;

pub use net::{net_status, NetStatus};
use audio_policy::audio_set_default;
use winrt::wait;

/// Какой список устройств запрошен (выпадающие списки вкладки Controls).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListKind {
    Wifi,
    Bluetooth,
    Audio,
}

/// Строка выпадающего списка.
#[derive(Clone, Debug, Default)]
pub struct DevItem {
    /// Wi-Fi: имя сохранённого профиля (пусто, если сеть не сохранена); аудио: id конечной точки;
    /// Bluetooth: id устройства WinRT.
    pub id: String,
    pub name: String,
    pub sub: String,
    pub active: bool,
    /// Wi-Fi без профиля: «аутентификация|шифрование» для нового профиля («open|none» — без пароля;
    /// пусто — корпоративная сеть, подключаться через Параметры). Bluetooth и аудио: ContainerId
    /// устройства, если его можно подключить из острова (Bluetooth-аудио), иначе пусто.
    pub extra: String,
}

/// Строка состояния последнего действия для каждого списка («Подключение…», «Неверный пароль?»).
static STATUS: Mutex<[Option<(String, Instant)>; 3]> = Mutex::new([None, None, None]);
/// Сколько держится сообщение о результате (сообщение «…» держится, пока действие идёт).
const STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(6);

/// Сообщение для списка `kind`, если оно свежее.
pub fn status(kind: ListKind) -> Option<String> {
    let s = STATUS.lock().ok()?;
    let (text, at) = s[kind as usize].as_ref()?;
    (text.ends_with('…') || at.elapsed() < STATUS_TTL).then(|| text.clone())
}

/// Ставит сообщение для списков `kinds` и будит вкладку (новая ревизия снимка).
fn set_status(kinds: &[ListKind], text: &str) {
    if let Ok(mut s) = STATUS.lock() {
        for k in kinds {
            s[*k as usize] = Some((text.to_string(), Instant::now()));
        }
    }
    bump();
}

/// Новая ревизия снимка без перечитывания состояния: вкладка заберёт списки и статус.
fn bump() {
    if let Ok(mut s) = SNAPSHOT.lock() {
        let mut snap = s.clone().unwrap_or_default();
        snap.rev += 1;
        *s = Some(snap);
    }
    notify();
}

static LISTS: Mutex<[Vec<DevItem>; 3]> = Mutex::new([Vec::new(), Vec::new(), Vec::new()]);

pub fn list(kind: ListKind) -> Vec<DevItem> {
    LISTS.lock().map(|l| l[kind as usize].clone()).unwrap_or_default()
}

fn store_list(kind: ListKind, items: Vec<DevItem>) {
    if let Ok(mut l) = LISTS.lock() {
        l[kind as usize] = items;
    }
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

unsafe fn device() -> Option<IMMDevice> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
    en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()
}

unsafe fn endpoint() -> Option<IAudioEndpointVolume> {
    device()?.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
}

/// «Динамики (Realtek Audio)» → «Динамики».
unsafe fn device_name(dev: &IMMDevice) -> Option<String> {
    let name = device_full_name(dev)?;
    let short = name.split(" (").next().unwrap_or(&name).trim().to_string();
    let short = if short.is_empty() { name } else { short };
    Some(label_for(&short, device_form_factor(dev)))
}

const PKEY_FORM_FACTOR: windows::Win32::Foundation::PROPERTYKEY = windows::Win32::Foundation::PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e),
    pid: 0,
};

/// Тип устройства по `PKEY_AudioEndpoint_FormFactor` (3 — наушники, 5 — гарнитура).
unsafe fn device_form_factor(dev: &IMMDevice) -> Option<u32> {
    use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::System::Variant::VT_UI4;
    let store = dev.OpenPropertyStore(STGM_READ).ok()?;
    let mut pv = store.GetValue(&PKEY_FORM_FACTOR).ok()?;
    let v = (pv.Anonymous.Anonymous.vt == VT_UI4).then(|| pv.Anonymous.Anonymous.Anonymous.ulVal);
    let _ = PropVariantClear(&mut pv);
    v
}

/// Windows называет конечную точку наушников общим «Динамики»: по типу устройства подставляем «Наушники».
fn label_for(name: &str, form: Option<u32>) -> String {
    let lower = name.to_lowercase();
    let already = ["науш", "headphone", "headset", "гарнитур"].iter().any(|w| lower.contains(w));
    if matches!(form, Some(3 | 5)) && !already {
        crate::i18n::tr("Наушники").to_string()
    } else {
        name.to_string()
    }
}

/// Полное дружественное имя устройства: «Динамики (Realtek Audio)».
unsafe fn device_full_name(dev: &IMMDevice) -> Option<String> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::System::Variant::VT_LPWSTR;
    let store = dev.OpenPropertyStore(STGM_READ).ok()?;
    let mut pv = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
    let name = if pv.Anonymous.Anonymous.vt == VT_LPWSTR {
        pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().ok()
    } else {
        None
    };
    let _ = PropVariantClear(&mut pv);
    name
}

unsafe fn device_id(dev: &IMMDevice) -> Option<String> {
    let p = dev.GetId().ok()?;
    let s = p.to_string().ok();
    windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const std::ffi::c_void));
    s
}

/// Активные устройства вывода звука; текущее по умолчанию помечено `active`.
unsafe fn audio_devices() -> Vec<DevItem> {
    use windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE;
    let mut out = Vec::new();
    let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return out };
    let default = en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok().and_then(|d| device_id(&d));
    let Ok(coll) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
    for i in 0..coll.GetCount().unwrap_or(0) {
        let Ok(dev) = coll.Item(i) else { continue };
        let (Some(id), Some(full)) = (device_id(&dev), device_full_name(&dev)) else { continue };
        let (name, sub) = match full.split_once(" (") {
            Some((n, rest)) => (n.trim().to_string(), rest.trim_end_matches(')').to_string()),
            None => (full.clone(), String::new()),
        };
        let name = label_for(&name, device_form_factor(&dev));
        let active = default.as_deref() == Some(id.as_str());
        out.push(DevItem { id, name, sub, active, extra: String::new() });
    }
    // Отключённые Bluetooth-наушники и колонки: клик подключает и делает основными.
    let eps = audio_endpoints();
    for (_, name, container) in paired_cached() {
        let mine: Vec<&AudioEp> = eps.iter().filter(|e| e.container == container).collect();
        if container.is_empty() || mine.is_empty() || mine.iter().any(|e| e.active) {
            continue;
        }
        let id = mine[0].endpoint.clone();
        out.push(DevItem { id, name, sub: "Bluetooth • не подключено".into(), active: false, extra: container });
    }
    out
}


/// Открывает первый Wi-Fi-адаптер и вызывает `f(handle, guid)`.
unsafe fn with_wlan<T>(f: impl FnOnce(windows::Win32::Foundation::HANDLE, &windows::core::GUID) -> T) -> Option<T> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::*;
    let (mut ver, mut h) = (0u32, HANDLE::default());
    if WlanOpenHandle(2, None, &mut ver, &mut h) != 0 {
        return None;
    }
    let mut out = None;
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    if WlanEnumInterfaces(h, None, &mut list) == 0 && !list.is_null() {
        if (*list).dwNumberOfItems > 0 {
            let guid = (*list).InterfaceInfo[0].InterfaceGuid;
            out = Some(f(h, &guid));
        }
        WlanFreeMemory(list as *const std::ffi::c_void);
    }
    let _ = WlanCloseHandle(h, None);
    out
}

/// Видимые Wi-Fi сети: подключённая первой, дальше по уровню сигнала.
unsafe fn wifi_networks(scan: bool) -> Vec<DevItem> {
    use windows::Win32::NetworkManagement::WiFi::*;
    with_wlan(|h, guid| {
        if scan {
            let _ = WlanScan(h, guid, None, None, None);
        }
        let mut out: Vec<(u32, DevItem)> = Vec::new();
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        if WlanGetAvailableNetworkList(h, guid, 0, None, &mut list) == 0 && !list.is_null() {
            let items = std::slice::from_raw_parts((*list).Network.as_ptr(), (*list).dwNumberOfItems as usize);
            for n in items {
                let len = (n.dot11Ssid.uSSIDLength as usize).min(32);
                if len == 0 {
                    continue;
                }
                let ssid = String::from_utf8_lossy(&n.dot11Ssid.ucSSID[..len]).into_owned();
                let connected = n.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0;
                let saved = n.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0;
                let profile = {
                    let end = n.strProfileName.iter().position(|&c| c == 0).unwrap_or(n.strProfileName.len());
                    String::from_utf16_lossy(&n.strProfileName[..end])
                };
                let sub = if connected {
                    "Подключено".to_string()
                } else if saved {
                    format!("{} • {}%", crate::i18n::pick("Сохранена", "Saved"), n.wlanSignalQuality)
                } else if n.bSecurityEnabled.as_bool() {
                    format!("{} • {}%", crate::i18n::pick("Защищена", "Secured"), n.wlanSignalQuality)
                } else {
                    format!("{} • {}%", crate::i18n::pick("Открытая", "Open"), n.wlanSignalQuality)
                };
                let extra = if saved {
                    String::new()
                } else {
                    join_params(n.dot11DefaultAuthAlgorithm.0, n.dot11DefaultCipherAlgorithm.0, n.bSecurityEnabled.as_bool())
                };
                let item = DevItem { id: if saved { profile } else { String::new() }, name: ssid, sub, active: connected, extra };
                if let Some(old) = out.iter_mut().find(|(_, d)| d.name == item.name) {
                    if n.wlanSignalQuality > old.0 || connected {
                        *old = (n.wlanSignalQuality, item);
                    }
                } else {
                    out.push((n.wlanSignalQuality, item));
                }
            }
            WlanFreeMemory(list as *const std::ffi::c_void);
        }
        out.sort_by_key(|(q, d)| (std::cmp::Reverse(d.active), std::cmp::Reverse(*q)));
        out.into_iter().map(|(_, d)| d).collect::<Vec<_>>()
    })
    .unwrap_or_default()
}

/// Параметры профиля для сети без профиля по алгоритмам из списка сетей (DOT11_AUTH_ALGORITHM,
/// DOT11_CIPHER_ALGORITHM). Пусто — сеть, которую из острова не подключить (корпоративная, WEP).
pub fn join_params(auth: i32, cipher: i32, secured: bool) -> String {
    let auth = match auth {
        1 if !secured => return "open|none".into(),
        4 => "WPAPSK",
        7 => "WPA2PSK",
        9 => "WPA3SAE",
        _ => return String::new(),
    };
    // 2 — TKIP; остальное у WPA — AES (CCMP/GCMP).
    let enc = if cipher == 2 { "TKIP" } else { "AES" };
    format!("{auth}|{enc}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// XML профиля WLAN для новой сети. `params` — из `join_params`.
pub fn profile_xml(ssid: &str, params: &str, key: &str) -> String {
    let (auth, enc) = params.split_once('|').unwrap_or(("open", "none"));
    let hex: String = ssid.bytes().map(|b| format!("{b:02X}")).collect();
    let name = xml_escape(ssid);
    let shared = if auth == "open" {
        String::new()
    } else {
        format!(
            "<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>",
            xml_escape(key)
        )
    };
    format!(
        "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
<name>{name}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID></SSIDConfig>\
<connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM><security><authEncryption>\
<authentication>{auth}</authentication><encryption>{enc}</encryption><useOneX>false</useOneX></authEncryption>\
{shared}</security></MSM></WLANProfile>"
    )
}

/// Подключение по профилю. `false` — WlanConnect отказал сразу.
unsafe fn wifi_connect(profile: &str) -> bool {
    use windows::Win32::NetworkManagement::WiFi::*;
    let wide = crate::sys::wz(profile);
    with_wlan(|h, guid| {
        let params = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(wide.as_ptr()),
            pDot11Ssid: std::ptr::null_mut(),
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_any,
            dwFlags: 0,
        };
        let r = WlanConnect(h, guid, &params, None);
        if r != 0 {
            log::error(&format!("WlanConnect вернул {r}"));
        }
        r == 0
    })
    .unwrap_or(false)
}

/// Ждёт подключения к сети `ssid` (до 20 с). Сдаётся раньше, если после попытки подключения
/// адаптер вернулся в «отключено» (чаще всего — неверный пароль).
unsafe fn wifi_wait(ssid: &str) -> bool {
    use windows::Win32::NetworkManagement::WiFi::*;
    let start = Instant::now();
    let mut tried = false;
    while start.elapsed() < std::time::Duration::from_secs(20) {
        std::thread::sleep(std::time::Duration::from_millis(400));
        if wifi_ssid().as_deref() == Some(ssid) {
            return true;
        }
        let state = with_wlan(|h, guid| {
            let (mut size, mut data) = (0u32, std::ptr::null_mut::<std::ffi::c_void>());
            let mut st = None;
            if WlanQueryInterface(h, guid, wlan_intf_opcode_interface_state, None, &mut size, &mut data, None) == 0 && !data.is_null() {
                st = Some(*(data as *const WLAN_INTERFACE_STATE));
                WlanFreeMemory(data);
            }
            st
        })
        .flatten();
        match state {
            Some(s) if s == wlan_interface_state_disconnected || s == wlan_interface_state_connected => {
                if tried || start.elapsed() > std::time::Duration::from_secs(8) {
                    return false;
                }
            }
            Some(_) => tried = true,
            None => return false,
        }
    }
    false
}

/// Подключение к сети в отдельном потоке: ожидание результата не задерживает громкость и медиа.
/// `profile` — сохранённый профиль; `join` — («аутентификация|шифрование», пароль) для новой сети.
fn wifi_spawn(ssid: String, profile: Option<String>, join: Option<(String, String)>) {
    set_status(&[ListKind::Wifi], "Подключение…");
    let spawned = std::thread::Builder::new().name("island-wifi".into()).spawn(move || unsafe {
        use windows::Win32::NetworkManagement::WiFi::*;
        let mut created = false;
        let name = match (profile, join) {
            (Some(p), _) => p,
            (None, Some((params, key))) => {
                let xml = crate::sys::wz(&profile_xml(&ssid, &params, &key));
                let mut reason = 0u32;
                let r = with_wlan(|h, guid| WlanSetProfile(h, guid, 0, PCWSTR(xml.as_ptr()), PCWSTR::null(), true, None, &mut reason));
                if r != Some(0) {
                    log::error(&format!("WlanSetProfile вернул {r:?}, причина {reason}"));
                    set_status(&[ListKind::Wifi], "Не удалось создать профиль сети");
                    return;
                }
                created = true;
                ssid.clone()
            }
            (None, None) => return,
        };
        let ok = wifi_connect(&name) && wifi_wait(&ssid);
        if !ok {
            log::info(&format!("Wi-Fi: не удалось подключиться к «{ssid}»"));
            // Новый профиль с неверным паролем не оставляем, иначе Windows будет пробовать его снова.
            if created {
                let wide = crate::sys::wz(&name);
                let _ = with_wlan(|h, guid| WlanDeleteProfile(h, guid, PCWSTR(wide.as_ptr()), None));
            }
        }
        store_list(ListKind::Wifi, wifi_networks(false));
        let msg = match (ok, created) {
            (true, _) => "Подключено",
            (false, true) => "Не подключилось — неверный пароль?",
            (false, false) => "Не удалось подключиться",
        };
        set_status(&[ListKind::Wifi], msg);
        send(Cmd::Net);
        send(Cmd::Refresh);
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить поток Wi-Fi: {e}"));
    }
}

/// Отключение идёт в отдельном потоке: пауза ниже не должна задерживать громкость и медиа-команды
/// на общем командном потоке.
fn wifi_disconnect() {
    use windows::Win32::NetworkManagement::WiFi::WlanDisconnect;
    let spawned = std::thread::Builder::new().name("island-wifi-off".into()).spawn(|| unsafe {
        let r = with_wlan(|h, guid| WlanDisconnect(h, guid, None));
        if r != Some(0) {
            log::error(&format!("WlanDisconnect вернул {r:?}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(600));
        store_list(ListKind::Wifi, wifi_networks(false));
        send(Cmd::Net);
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить поток отключения Wi-Fi: {e}"));
    }
}

// ---------- Bluetooth-аудио: подключение через драйвер (IKsControl) ----------
//
// Публичного API «подключить сопряжённое устройство» нет, но у аудиофильтров Bluetooth есть
// свойство KSPROPSETID_BtAudio: ONESHOT_RECONNECT (0) / ONESHOT_DISCONNECT (1) — им пользуется
// кнопка «Подключить» в Windows и утилиты вроде ToothTray. Работает для наушников и колонок;
// мыши, клавиатуры и прочее по-прежнему подключаются из Параметров.

const KSPROPSETID_BT_AUDIO: windows::core::GUID = windows::core::GUID::from_u128(0x7fa06c40_b8f6_4c7e_8556_e8c33a12e54d);
const PKEY_CONTAINER_ID: windows::Win32::Foundation::PROPERTYKEY = windows::Win32::Foundation::PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x8c7ed206_3f8a_4827_b3ab_ae9e1faefc6c),
    pid: 2,
};

/// Конечная точка вывода звука (любого состояния) с ContainerId устройства.
struct AudioEp {
    container: String,
    endpoint: String,
    active: bool,
    /// id KS-фильтра Bluetooth-драйвера (у него вызывается свойство подключения). Есть, только
    /// пока драйвер загружен: при выключенном Bluetooth точки «отсутствуют» и фильтра нет.
    bt_filter: Option<String>,
}

fn guid_str(g: &windows::core::GUID) -> String {
    format!("{g:?}").to_uppercase()
}

unsafe fn container_id(dev: &IMMDevice) -> Option<String> {
    use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::System::Variant::VT_CLSID;
    let store = dev.OpenPropertyStore(STGM_READ).ok()?;
    let mut pv = store.GetValue(&PKEY_CONTAINER_ID).ok()?;
    let v = &pv.Anonymous.Anonymous;
    let out = if v.vt == VT_CLSID && !v.Anonymous.puuid.is_null() { Some(guid_str(&*v.Anonymous.puuid)) } else { None };
    let _ = PropVariantClear(&mut pv);
    out
}

/// id устройства-фильтра, к которому подключена конечная точка (через топологию).
unsafe fn filter_of(dev: &IMMDevice) -> Option<String> {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{IDeviceTopology, IPart};
    let topo: IDeviceTopology = dev.Activate(CLSCTX_ALL, None).ok()?;
    let conn = topo.GetConnector(0).ok()?;
    let other = conn.GetConnectedTo().ok()?;
    let part: IPart = other.cast().ok()?;
    let ftopo = part.GetTopologyObject().ok()?;
    let p = ftopo.GetDeviceId().ok()?;
    let id = p.to_string().ok();
    windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const std::ffi::c_void));
    id
}

/// Все конечные точки вывода, включая отключённые и отсутствующие.
unsafe fn audio_endpoints() -> Vec<AudioEp> {
    use windows::Win32::Media::Audio::{DEVICE_STATE, DEVICE_STATEMASK_ALL, DEVICE_STATE_ACTIVE, DEVICE_STATE_UNPLUGGED};
    let mut out = Vec::new();
    let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return out };
    let Ok(coll) = en.EnumAudioEndpoints(eRender, DEVICE_STATE(DEVICE_STATEMASK_ALL)) else { return out };
    for i in 0..coll.GetCount().unwrap_or(0) {
        let Ok(dev) = coll.Item(i) else { continue };
        let state = dev.GetState().map(|s| s.0).unwrap_or(0);
        let (Some(container), Some(endpoint)) = (container_id(&dev), device_id(&dev)) else { continue };
        let bt_filter = if state & (DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_UNPLUGGED.0) != 0 {
            filter_of(&dev).filter(|f| {
                let up = f.to_uppercase();
                up.contains("BTHENUM") || up.contains("BTHHFENUM")
            })
        } else {
            None
        };
        out.push(AudioEp { container, endpoint, active: state & DEVICE_STATE_ACTIVE.0 != 0, bt_filter });
    }
    out
}

/// Сопряжённое Bluetooth-устройство: (id WinRT, имя, ContainerId).
type Paired = (String, String, String);

/// Последний список сопряжённых устройств: список звука берёт его отсюда, чтобы не перечислять
/// Bluetooth каждую секунду.
static PAIRED: Mutex<Option<(Vec<Paired>, Instant)>> = Mutex::new(None);

unsafe fn paired_bt() -> Vec<Paired> {
    use windows::core::{Interface, HSTRING};
    use windows::Devices::Bluetooth::BluetoothDevice;
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Foundation::IReference;
    let mut out = Vec::new();
    let Ok(sel) = BluetoothDevice::GetDeviceSelectorFromPairingState(true) else { return out };
    const CONTAINER: &str = "System.Devices.Aep.ContainerId";
    let props = windows_collections::IIterable::<HSTRING>::from(vec![HSTRING::from(CONTAINER)]);
    let Ok(coll) = wait(DeviceInformation::FindAllAsyncAqsFilterAndAdditionalProperties(&sel, &props)) else {
        return out;
    };
    for info in coll {
        let container = info
            .Properties()
            .and_then(|p| p.Lookup(&HSTRING::from(CONTAINER)))
            .and_then(|v| v.cast::<IReference<windows::core::GUID>>())
            .and_then(|r| r.Value())
            .map(|g| guid_str(&g))
            .unwrap_or_default();
        let (Ok(name), Ok(id)) = (info.Name(), info.Id()) else { continue };
        if !name.is_empty() {
            out.push((id.to_string(), name.to_string(), container));
        }
    }
    if let Ok(mut p) = PAIRED.lock() {
        *p = Some((out.clone(), Instant::now()));
    }
    out
}

/// Сопряжённые устройства из кэша (не старше минуты).
unsafe fn paired_cached() -> Vec<Paired> {
    let cached = PAIRED.lock().ok().and_then(|p| p.as_ref().filter(|(_, t)| t.elapsed().as_secs() < 60).map(|(v, _)| v.clone()));
    cached.unwrap_or_else(|| paired_bt())
}

/// Вызывает у всех аудиофильтров устройства `container` подключение или отключение.
/// `false` — фильтров нет (драйвер не загружен) или все вызовы отказали.
unsafe fn bt_oneshot(container: &str, connect: bool) -> bool {
    use windows::Win32::Media::KernelStreaming::{IKsControl, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0};
    let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return false };
    let mut done: Vec<String> = Vec::new();
    let mut ok = false;
    for ep in audio_endpoints().into_iter().filter(|e| e.container == container) {
        let Some(filter) = ep.bt_filter else { continue };
        if done.contains(&filter) {
            continue;
        }
        done.push(filter.clone());
        let wide = crate::sys::wz(&filter);
        let Ok(dev) = en.GetDevice(PCWSTR(wide.as_ptr())) else { continue };
        let ks: IKsControl = match dev.Activate(CLSCTX_ALL, None) {
            Ok(k) => k,
            Err(e) => {
                log::error(&format!("IKsControl для {filter}: {e}"));
                continue;
            }
        };
        let prop = KSIDENTIFIER {
            Anonymous: KSIDENTIFIER_0 {
                // Flags = KSPROPERTY_TYPE_GET.
                Anonymous: KSIDENTIFIER_0_0 { Set: KSPROPSETID_BT_AUDIO, Id: if connect { 0 } else { 1 }, Flags: 1 },
            },
        };
        let mut ret = 0u32;
        match ks.KsProperty(&prop, std::mem::size_of::<KSIDENTIFIER>() as u32, std::ptr::null_mut(), 0, &mut ret) {
            Ok(()) => ok = true,
            Err(e) => log::error(&format!("Bluetooth {} {filter}: {e}", if connect { "подключение" } else { "отключение" })),
        }
    }
    ok
}

/// Ждёт до `secs` секунд, пока `f()` не станет истинным (проверка раз в полсекунды).
fn wait_for(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(secs) {
        if f() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    f()
}

/// Подключение/отключение Bluetooth-аудио в отдельном потоке. Если Bluetooth выключен, сначала
/// включает его и ждёт, пока загрузится драйвер устройства. После подключения делает устройство
/// основным устройством вывода, как Windows.
fn bt_spawn(container: String, connect: bool) {
    let lists = [ListKind::Bluetooth, ListKind::Audio];
    set_status(&lists, if connect { "Подключение…" } else { "Отключение…" });
    let spawned = std::thread::Builder::new().name("island-bt".into()).spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let has_filter = || audio_endpoints().iter().any(|e| e.container == container && e.bt_filter.is_some());
        let is_active = || audio_endpoints().iter().any(|e| e.container == container && e.active);
        if connect && !has_filter() {
            let mut access = false;
            set_radio(windows::Devices::Radios::RadioKind::Bluetooth, true, &mut access);
            send(Cmd::Refresh);
            // Включённый Bluetooth часто сам подключает последние наушники.
            wait_for(10, || is_active() || has_filter());
        }
        let ok = if connect && is_active() {
            true
        } else {
            bt_oneshot(&container, connect) && wait_for(12, || is_active() == connect)
        };
        if ok && connect {
            if let Some(ep) = audio_endpoints().into_iter().find(|e| e.container == container && e.active) {
                audio_set_default(&ep.endpoint);
            }
        }
        store_list(ListKind::Bluetooth, bluetooth_devices());
        store_list(ListKind::Audio, audio_devices());
        let msg = match (ok, connect) {
            (true, true) => "Подключено",
            (true, false) => "Отключено",
            (false, true) => "Не удалось подключить (устройство рядом и включено?)",
            (false, false) => "Не удалось отключить",
        };
        set_status(&lists, msg);
        send(Cmd::Refresh);
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить поток Bluetooth: {e}"));
    }
}

/// Сопряжённые Bluetooth-устройства (классические); у подключённых `active`. Наушникам и
/// колонкам (у их ContainerId есть конечная точка звука) `extra` = ContainerId: их можно
/// подключать и отключать из острова.
unsafe fn bluetooth_devices() -> Vec<DevItem> {
    use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice};
    let audio: Vec<String> = audio_endpoints().into_iter().map(|e| e.container).collect();
    let mut out = Vec::new();
    for (id, name, container) in paired_bt() {
        let container = if audio.contains(&container) { container } else { String::new() };
        let connected = wait(BluetoothDevice::FromIdAsync(&windows::core::HSTRING::from(id.as_str())))
            .and_then(|d| d.ConnectionStatus())
            .map(|s| s == BluetoothConnectionStatus::Connected)
            .unwrap_or(false);
        let sub = match (connected, container.is_empty()) {
            (true, _) => "Подключено",
            (false, false) => "Сопряжено • нажмите, чтобы подключить",
            (false, true) => "Сопряжено",
        }
        .to_string();
        out.push(DevItem { id, name, sub, active: connected, extra: container });
    }
    out.sort_by_key(|d| (std::cmp::Reverse(d.active), d.name.to_lowercase()));
    out
}

/// Имя сети Wi-Fi, к которой подключён первый беспроводной адаптер.
unsafe fn wifi_ssid() -> Option<String> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::*;
    let (mut ver, mut h) = (0u32, HANDLE::default());
    if WlanOpenHandle(2, None, &mut ver, &mut h) != 0 {
        return None;
    }
    let mut out = None;
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    if WlanEnumInterfaces(h, None, &mut list) == 0 && !list.is_null() {
        let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), (*list).dwNumberOfItems as usize);
        for it in items {
            if it.isState != wlan_interface_state_connected {
                continue;
            }
            let (mut size, mut data) = (0u32, std::ptr::null_mut::<c_void>());
            if WlanQueryInterface(h, &it.InterfaceGuid, wlan_intf_opcode_current_connection, None, &mut size, &mut data, None) == 0
                && !data.is_null()
            {
                let attr = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
                let ssid = &attr.wlanAssociationAttributes.dot11Ssid;
                let n = (ssid.uSSIDLength as usize).min(32);
                out = Some(String::from_utf8_lossy(&ssid.ucSSID[..n]).into_owned());
                WlanFreeMemory(data);
            }
            break;
        }
        WlanFreeMemory(list as *const c_void);
    }
    let _ = WlanCloseHandle(h, None);
    out
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

unsafe fn set_radio(kind: windows::Devices::Radios::RadioKind, on: bool, access: &mut bool) {
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

// ---------- События громкости и медиасессии (индикатор в свёрнутой форме) ----------

/// Приёмник уведомлений об изменении громкости: вызывается системой в своём потоке, только
/// пересылает значения главному окну — тяжёлого здесь ничего нет.
#[windows::core::implement(IAudioEndpointVolumeCallback)]
struct VolumeWatch;

#[allow(non_snake_case)]
impl IAudioEndpointVolumeCallback_Impl for VolumeWatch_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        let h = HWND_MAIN.load(Ordering::Relaxed);
        if let (Some(n), true) = (unsafe { pnotify.as_ref() }, h != 0) {
            let permille = (n.fMasterVolume.clamp(0.0, 1.0) * 1000.0).round() as usize;
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_VOLUME, WPARAM(permille), LPARAM(n.bMuted.as_bool() as isize));
            }
        }
        Ok(())
    }
}

thread_local! {
    /// Подписка на громкость устройства по умолчанию: (конечная точка, приёмник, id устройства).
    static VOL_WATCH: RefCell<Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback, String)>> =
        const { RefCell::new(None) };
    /// Подписка на события текущей медиасессии: (сессия, токены двух событий, AUMID приложения).
    static SESSION_WATCH: RefCell<Option<(windows::Media::Control::GlobalSystemMediaTransportControlsSession, i64, i64, String)>> =
        const { RefCell::new(None) };
}

/// Подписывается на изменения громкости устройства вывода по умолчанию (повторно — только если
/// устройство сменилось). Событийная подписка: в простое потоку делать нечего.
unsafe fn watch_volume() {
    let Some(dev) = device() else { return };
    let id = device_id(&dev).unwrap_or_default();
    VOL_WATCH.with(|w| {
        let mut w = w.borrow_mut();
        if w.as_ref().is_some_and(|(_, _, old)| *old == id) {
            return;
        }
        if let Some((ep, cb, _)) = w.take() {
            let _ = ep.UnregisterControlChangeNotify(&cb);
        }
        let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) else { return };
        let cb: IAudioEndpointVolumeCallback = VolumeWatch.into();
        match ep.RegisterControlChangeNotify(&cb) {
            Ok(()) => *w = Some((ep, cb, id)),
            Err(e) => log::error(&format!("не удалось подписаться на громкость: {e}")),
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headphones_label() {
        assert_eq!(label_for("Динамики", Some(3)), "Наушники");
        assert_eq!(label_for("Динамики", Some(5)), "Наушники");
        assert_eq!(label_for("Динамики", Some(1)), "Динамики");
        assert_eq!(label_for("Динамики", None), "Динамики");
        assert_eq!(label_for("Headset Earphone", Some(5)), "Headset Earphone");
    }

    #[test]
    fn wifi_join_params() {
        assert_eq!(join_params(7, 4, true), "WPA2PSK|AES");
        assert_eq!(join_params(4, 2, true), "WPAPSK|TKIP");
        assert_eq!(join_params(9, 4, true), "WPA3SAE|AES");
        assert_eq!(join_params(1, 0, false), "open|none");
        // WEP и корпоративные сети — только через Параметры.
        assert_eq!(join_params(1, 1, true), "");
        assert_eq!(join_params(6, 4, true), "");
    }

    #[test]
    fn wifi_profile_xml_escapes_and_hexes() {
        let x = profile_xml("Кафе & <Wi-Fi>", "WPA2PSK|AES", "p\"ss'<>&");
        assert!(x.contains("<name>Кафе &amp; &lt;Wi-Fi&gt;</name>"));
        assert!(x.contains("<keyMaterial>p&quot;ss&apos;&lt;&gt;&amp;</keyMaterial>"));
        assert!(x.contains("<authentication>WPA2PSK</authentication><encryption>AES</encryption>"));
        // SSID в hex — байты UTF-8.
        assert!(x.contains("<hex>D09AD0B0"));
        let open = profile_xml("Free", "open|none", "");
        assert!(open.contains("<authentication>open</authentication><encryption>none</encryption>"));
        assert!(!open.contains("sharedKey"));
    }

    /// Живая проверка списков устройств (только чтение).
    #[test]
    #[ignore]
    fn live_lists() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            for (n, l) in [("wifi", wifi_networks(false)), ("bt", bluetooth_devices()), ("audio", audio_devices())] {
                println!("{n}: {}", l.len());
                for d in l.iter().take(8) {
                    println!("  {} | {} | active={} | extra={} | id={}", d.name, d.sub, d.active, d.extra, d.id.chars().take(24).collect::<String>());
                }
            }
        }
    }
}
