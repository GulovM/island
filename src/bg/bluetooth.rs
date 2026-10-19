//! Управление Bluetooth-устройствами и сопряжением через WinRT и IKsControl.

use std::sync::Mutex;
use std::time::Instant;

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{
    eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE, DEVICE_STATEMASK_ALL,
    DEVICE_STATE_ACTIVE, DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

use super::audio_policy::audio_set_default;
use super::winrt::wait;
use super::{audio_devices, send, set_radio, set_status, store_list, Cmd, DevItem, ListKind};
use crate::log;

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
pub(super) struct AudioEp {
    pub(super) container: String,
    pub(super) endpoint: String,
    pub(super) active: bool,
    /// id KS-фильтра Bluetooth-драйвера (у него вызывается свойство подключения). Есть, только
    /// пока драйвер загружен: при выключенном Bluetooth точки «отсутствуют» и фильтра нет.
    pub(super) bt_filter: Option<String>,
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
pub(super) unsafe fn audio_endpoints() -> Vec<AudioEp> {
    let mut out = Vec::new();
    let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return out };
    let Ok(coll) = en.EnumAudioEndpoints(eRender, DEVICE_STATE(DEVICE_STATEMASK_ALL)) else { return out };
    for i in 0..coll.GetCount().unwrap_or(0) {
        let Ok(dev) = coll.Item(i) else { continue };
        let state = dev.GetState().map(|s| s.0).unwrap_or(0);
        let (Some(container), Some(endpoint)) = (container_id(&dev), super::device_id(&dev)) else { continue };
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
pub(super) type Paired = (String, String, String);

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
pub(super) unsafe fn paired_cached() -> Vec<Paired> {
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
pub(super) fn bt_spawn(container: String, connect: bool) {
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
pub(super) unsafe fn bluetooth_devices() -> Vec<DevItem> {
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
