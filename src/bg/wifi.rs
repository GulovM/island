//! Управление беспроводной сетью Wi-Fi: поиск сетей, подключение по профилю и WPA.

use std::time::Instant;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::*;

use super::{set_status, store_list, DevItem, ListKind};
use super::{send, Cmd};
use crate::log;

/// Открывает первый Wi-Fi-адаптер и вызывает `f(handle, guid)`.
unsafe fn with_wlan<T>(f: impl FnOnce(HANDLE, &windows::core::GUID) -> T) -> Option<T> {
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
pub(super) unsafe fn wifi_networks(scan: bool) -> Vec<DevItem> {
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
pub(super) fn wifi_spawn(ssid: String, profile: Option<String>, join: Option<(String, String)>) {
    set_status(&[ListKind::Wifi], "Подключение…");
    let spawned = std::thread::Builder::new().name("island-wifi".into()).spawn(move || unsafe {
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
pub(super) fn wifi_disconnect() {
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

/// Имя сети Wi-Fi, к которой подключён первый беспроводной адаптер.
pub(super) unsafe fn wifi_ssid() -> Option<String> {
    use std::ffi::c_void;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
