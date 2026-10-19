//! Списки устройств (Wi-Fi, Bluetooth, аудио) и функции работы с аудио-устройствами.

use std::sync::Mutex;
use std::time::Instant;

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

use super::bluetooth;
use super::{notify, SNAPSHOT};

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
pub(super) fn set_status(kinds: &[ListKind], text: &str) {
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

pub(super) fn store_list(kind: ListKind, items: Vec<DevItem>) {
    if let Ok(mut l) = LISTS.lock() {
        l[kind as usize] = items;
    }
}

pub(super) unsafe fn device() -> Option<IMMDevice> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
    en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()
}

pub(super) unsafe fn endpoint() -> Option<IAudioEndpointVolume> {
    device()?.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
}

/// «Динамики (Realtek Audio)» → «Динамики».
pub(super) unsafe fn device_name(dev: &IMMDevice) -> Option<String> {
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

pub(super) unsafe fn device_id(dev: &IMMDevice) -> Option<String> {
    let p = dev.GetId().ok()?;
    let s = p.to_string().ok();
    windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const std::ffi::c_void));
    s
}

/// Активные устройства вывода звука; текущее по умолчанию помечено `active`.
pub(super) unsafe fn audio_devices() -> Vec<DevItem> {
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
    let eps = bluetooth::audio_endpoints();
    for (_, name, container) in bluetooth::paired_cached() {
        let mine: Vec<&bluetooth::AudioEp> = eps.iter().filter(|e| e.container == container).collect();
        if container.is_empty() || mine.is_empty() || mine.iter().any(|e| e.active) {
            continue;
        }
        let id = mine[0].endpoint.clone();
        out.push(DevItem { id, name, sub: "Bluetooth • не подключено".into(), active: false, extra: container });
    }
    out
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use super::super::bluetooth::bluetooth_devices;
    use super::super::wifi::wifi_networks;
    use super::*;

    #[test]
    fn headphones_label() {
        assert_eq!(label_for("Динамики", Some(3)), "Наушники");
        assert_eq!(label_for("Динамики", Some(5)), "Наушники");
        assert_eq!(label_for("Динамики", Some(1)), "Динамики");
        assert_eq!(label_for("Динамики", None), "Динамики");
        assert_eq!(label_for("Headset Earphone", Some(5)), "Headset Earphone");
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
