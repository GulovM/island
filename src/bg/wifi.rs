//! Управление беспроводной сетью Wi-Fi на macOS через `networksetup`, `ipconfig` и `system_profiler`.

use std::time::Instant;

use super::devices::{set_status, store_list, DevItem, ListKind};
use super::{send, Cmd};
use crate::log;

#[cfg(test)]
#[allow(non_camel_case_types)]
pub(crate) type DOT11_RADIO_STATE = i32;
#[cfg(test)]
#[allow(non_upper_case_globals)]
pub(crate) const dot11_radio_state_unknown: DOT11_RADIO_STATE = 0;
#[cfg(test)]
#[allow(non_upper_case_globals)]
pub(crate) const dot11_radio_state_on: DOT11_RADIO_STATE = 1;
#[cfg(test)]
#[allow(non_upper_case_globals)]
pub(crate) const dot11_radio_state_off: DOT11_RADIO_STATE = 2;

#[cfg(test)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
#[allow(non_snake_case)]
pub(crate) struct WLAN_PHY_RADIO_STATE {
    pub dwPhyIndex: u32,
    pub dot11SoftwareRadioState: DOT11_RADIO_STATE,
    pub dot11HardwareRadioState: DOT11_RADIO_STATE,
}

#[cfg(test)]
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_snake_case)]
pub(crate) struct WLAN_RADIO_STATE {
    pub dwNumberOfPhys: u32,
    pub PhyRadioState: [WLAN_PHY_RADIO_STATE; 64],
}

#[cfg(test)]
impl Default for WLAN_RADIO_STATE {
    fn default() -> Self {
        Self { dwNumberOfPhys: 0, PhyRadioState: [WLAN_PHY_RADIO_STATE::default(); 64] }
    }
}

/// Снимок состояния Wi-Fi: включено ли радио и к какой сети подключено.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct WifiStatus {
    pub(super) radio_on: bool,
    pub(super) radio_known: bool,
    pub(super) ssid: Option<String>,
}

#[cfg(target_os = "macos")]
fn run_cmd(prog: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = std::process::Command::new(prog).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().is_empty() && !out.stderr.is_empty() {
        text = String::from_utf8_lossy(&out.stderr).into_owned();
    }
    Some((out.status.success(), text))
}

#[cfg(not(target_os = "macos"))]
fn run_cmd(_prog: &str, _args: &[&str]) -> Option<(bool, String)> {
    None
}

/// Выделяет имя BSD-интерфейса Wi-Fi (например, `en0`) из вывода `networksetup -listallhardwareports`.
fn parse_wifi_device(ports: &str) -> Option<String> {
    let mut is_wifi = false;
    for line in ports.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Hardware Port:") {
            let port = rest.trim().to_ascii_lowercase();
            is_wifi = port.contains("wi-fi") || port.contains("airport");
        } else if is_wifi {
            if let Some(dev) = trimmed.strip_prefix("Device:") {
                let dev = dev.trim();
                if !dev.is_empty() {
                    return Some(dev.to_string());
                }
            }
        }
    }
    None
}

fn wifi_iface() -> Option<String> {
    let (ok, out) = run_cmd("networksetup", &["-listallhardwareports"])?;
    if !ok {
        return None;
    }
    parse_wifi_device(&out)
}

fn parse_current_ssid(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Current Wi-Fi Network:") {
            let ssid = rest.trim();
            if !ssid.is_empty() && ssid != "<redacted>" {
                return Some(ssid.to_string());
            }
        } else if let Some(rest) = trimmed.strip_prefix("Current AirPort Network:") {
            let ssid = rest.trim();
            if !ssid.is_empty() && ssid != "<redacted>" {
                return Some(ssid.to_string());
            }
        } else if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim() == "SSID" {
                let ssid = v.trim();
                if !ssid.is_empty() && ssid != "<redacted>" {
                    return Some(ssid.to_string());
                }
            }
        }
    }
    None
}

fn parse_preferred_ssids(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn rssi_to_percent(rssi: i32) -> u32 {
    ((rssi + 100).clamp(0, 60) * 100 / 60) as u32
}

/// Разбирает `system_profiler SPAirPortDataType` на пары `(качество%, DevItem)`.
fn parse_airport_profiler(text: &str, saved: &[String], current: Option<&str>) -> Vec<(u32, DevItem)> {
    let mut out: Vec<(u32, DevItem)> = Vec::new();
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Section {
        None,
        Current,
        Other,
    }
    let mut section = Section::None;
    let mut cur_name: Option<String> = None;
    let mut cur_security = String::new();
    let mut cur_signal: u32 = 60;

    let flush = |out: &mut Vec<(u32, DevItem)>,
                 sec: Section,
                 name: &mut Option<String>,
                 security: &mut String,
                 signal: &mut u32| {
        let Some(ssid) = name.take() else { return };
        if ssid.is_empty() || ssid == "<redacted>" {
            security.clear();
            *signal = 60;
            return;
        }
        let active = sec == Section::Current || current == Some(ssid.as_str());
        let is_saved = active || saved.iter().any(|s| s == &ssid);
        let secured = !security.is_empty()
            && !security.eq_ignore_ascii_case("none")
            && !security.eq_ignore_ascii_case("open");
        let sub = if active {
            "Подключено".to_string()
        } else if is_saved {
            format!("{} • {}%", crate::i18n::pick("Сохранена", "Saved"), *signal)
        } else if secured {
            format!("{} • {}%", crate::i18n::pick("Защищена", "Secured"), *signal)
        } else {
            format!("{} • {}%", crate::i18n::pick("Открытая", "Open"), *signal)
        };
        let sec_lower = security.to_ascii_lowercase();
        let extra = if is_saved {
            String::new()
        } else if !secured {
            "open|none".into()
        } else if sec_lower.contains("wpa3") {
            "WPA3SAE|AES".into()
        } else if sec_lower.contains("wpa2") {
            "WPA2PSK|AES".into()
        } else if sec_lower.contains("wpa") {
            "WPAPSK|TKIP".into()
        } else {
            String::new()
        };
        let item = DevItem {
            id: if is_saved { ssid.clone() } else { String::new() },
            name: ssid,
            sub,
            active,
            extra,
        };
        if let Some(old) = out.iter_mut().find(|(_, d)| d.name == item.name) {
            if *signal > old.0 || active {
                *old = (*signal, item);
            }
        } else {
            out.push((*signal, item));
        }
        security.clear();
        *signal = 60;
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Current Network Information:") {
            flush(&mut out, section, &mut cur_name, &mut cur_security, &mut cur_signal);
            section = Section::Current;
            continue;
        }
        if trimmed.starts_with("Other Local Wi-Fi Networks:") {
            flush(&mut out, section, &mut cur_name, &mut cur_security, &mut cur_signal);
            section = Section::Other;
            continue;
        }
        if section == Section::None {
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent <= 10 && trimmed.ends_with(':') {
            flush(&mut out, section, &mut cur_name, &mut cur_security, &mut cur_signal);
            section = Section::None;
            continue;
        }
        // Заголовок сети внутри секции (`<SSID>:` с отступом глубже заголовка секции).
        if indent <= 14 && trimmed.ends_with(':') && !trimmed.contains(": ") {
            flush(&mut out, section, &mut cur_name, &mut cur_security, &mut cur_signal);
            cur_name = Some(trimmed.trim_end_matches(':').to_string());
            continue;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let k = k.trim();
            let v = v.trim();
            if k == "Security" {
                cur_security = v.to_string();
            } else if k == "Signal / Noise" {
                if let Some(dbm) = v.split_whitespace().next().and_then(|n| n.parse::<i32>().ok()) {
                    cur_signal = rssi_to_percent(dbm);
                }
            }
        }
    }
    flush(&mut out, section, &mut cur_name, &mut cur_security, &mut cur_signal);
    out
}

/// Включает или выключает радио Wi-Fi на macOS через `networksetup -setairportpower`.
pub(super) fn set_wifi_power(on: bool) -> bool {
    let Some(iface) = wifi_iface() else {
        return false;
    };
    let arg = if on { "on" } else { "off" };
    run_cmd("networksetup", &["-setairportpower", &iface, arg]).is_some_and(|(ok, _)| ok)
}

/// Видимые Wi-Fi сети: подключённая первой, дальше по уровню сигнала и сохранённые профили.
pub(super) unsafe fn wifi_networks(_scan: bool) -> Vec<DevItem> {
    let Some(iface) = wifi_iface() else {
        return Vec::new();
    };
    let status = wifi_status();
    let current = status.as_ref().and_then(|s| s.ssid.as_deref());
    let saved = run_cmd("networksetup", &["-listpreferredwirelessnetworks", &iface])
        .filter(|(ok, _)| *ok)
        .map(|(_, t)| parse_preferred_ssids(&t))
        .unwrap_or_default();

    let mut out = run_cmd("system_profiler", &["SPAirPortDataType"])
        .filter(|(ok, _)| *ok)
        .map(|(_, t)| parse_airport_profiler(&t, &saved, current))
        .unwrap_or_default();

    // Добавляем текущую сеть и сохранённые профили, если сканер macOS их скрыл без геолокации.
    if let Some(cur) = current {
        if !out.iter().any(|(_, d)| d.name == cur) {
            out.push((
                100,
                DevItem {
                    id: cur.to_string(),
                    name: cur.to_string(),
                    sub: "Подключено".to_string(),
                    active: true,
                    extra: String::new(),
                },
            ));
        }
    }
    for ssid in saved {
        if !out.iter().any(|(_, d)| d.name == ssid) {
            out.push((
                10,
                DevItem {
                    id: ssid.clone(),
                    name: ssid,
                    sub: crate::i18n::pick("Сохранена", "Saved").to_string(),
                    active: false,
                    extra: String::new(),
                },
            ));
        }
    }

    out.sort_by_key(|(q, d)| (std::cmp::Reverse(d.active), std::cmp::Reverse(*q)));
    out.into_iter().map(|(_, d)| d).collect()
}

/// Параметры профиля для сети без профиля по алгоритмам из списка сетей (DOT11_AUTH_ALGORITHM,
/// DOT11_CIPHER_ALGORITHM). Пусто — сеть, которую из острова не подключить (корпоративная, WEP).
#[cfg(test)]
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

#[cfg(test)]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// XML профиля WLAN для новой сети. `params` — из `join_params`.
#[cfg(test)]
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

/// Ждёт подключения к сети `ssid` (до 20 с).
unsafe fn wifi_wait(ssid: &str) -> bool {
    let start = Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(20) {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if wifi_ssid().as_deref() == Some(ssid) {
            return true;
        }
        if let Some(st) = wifi_status() {
            if st.radio_known && !st.radio_on {
                return false;
            }
        }
    }
    false
}

/// Подключение к сети в отдельном потоке: ожидание результата не задерживает громкость и медиа.
/// `profile` — сохранённое имя сети; `join` — («аутентификация|шифрование», пароль) для новой сети.
pub(super) fn wifi_spawn(ssid: String, profile: Option<String>, join: Option<(String, String)>) {
    set_status(&[ListKind::Wifi], "Подключение…");
    let spawned = std::thread::Builder::new().name("island-wifi".into()).spawn(move || unsafe {
        let has_key = join.as_ref().is_some_and(|(_, k)| !k.is_empty());
        let target = profile.unwrap_or_else(|| ssid.clone());
        let Some(iface) = wifi_iface() else {
            set_status(&[ListKind::Wifi], "Wi-Fi не найден");
            return;
        };
        let res = match join.as_ref() {
            Some((_, key)) if !key.is_empty() => {
                run_cmd("networksetup", &["-setairportnetwork", &iface, &target, key])
            }
            _ => run_cmd("networksetup", &["-setairportnetwork", &iface, &target]),
        };
        let cmd_ok = res.as_ref().is_some_and(|(ok, out)| {
            let low = out.to_ascii_lowercase();
            *ok && !low.contains("failed") && !low.contains("could not find")
        });
        let ok = cmd_ok && (wifi_ssid().as_deref() == Some(ssid.as_str()) || wifi_wait(&ssid));
        if !ok {
            log::info(&format!("Wi-Fi: не удалось подключиться к «{ssid}»"));
        }
        store_list(ListKind::Wifi, wifi_networks(false));
        let msg = match (ok, has_key) {
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

/// Отключение идёт в отдельном потоке, чтобы не задерживать громкость и медиа-команды.
pub(super) fn wifi_disconnect() {
    let spawned = std::thread::Builder::new().name("island-wifi-off".into()).spawn(|| unsafe {
        let airport = "/System/Library/PrivateFrameworks/Apple80211.framework/Versions/Current/Resources/airport";
        if std::path::Path::new(airport).exists() {
            let _ = run_cmd(airport, &["-z"]);
        } else {
            let _ = set_wifi_power(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(600));
        store_list(ListKind::Wifi, wifi_networks(false));
        send(Cmd::Net);
        send(Cmd::Refresh);
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить поток отключения Wi-Fi: {e}"));
    }
}

/// На macOS события Wi-Fi обновляются по запросу и фоновым тиком в `bg::mod`.
pub(super) unsafe fn watch_wlan() {}

/// Проверяет, включён ли хотя бы один PHY в `WLAN_RADIO_STATE` (программно и аппаратно).
#[cfg(test)]
pub(crate) fn phy_radio_on(rs: &WLAN_RADIO_STATE) -> (bool, bool) {
    let n = (rs.dwNumberOfPhys as usize).min(rs.PhyRadioState.len());
    let mut known = false;
    let mut on = false;
    for phy in &rs.PhyRadioState[..n] {
        if phy.dot11SoftwareRadioState != dot11_radio_state_unknown || phy.dot11HardwareRadioState != dot11_radio_state_unknown {
            known = true;
        }
        if phy.dot11SoftwareRadioState == dot11_radio_state_on && phy.dot11HardwareRadioState == dot11_radio_state_on {
            on = true;
        }
    }
    (known, on)
}

/// Синхронно читает состояние Wi-Fi через `networksetup` / `ipconfig`.
pub(super) unsafe fn wifi_status() -> Option<WifiStatus> {
    let iface = wifi_iface()?;
    let mut st = WifiStatus::default();
    if let Some((ok, out)) = run_cmd("networksetup", &["-getairportpower", &iface]) {
        if ok {
            let low = out.to_ascii_lowercase();
            if low.contains(": on") || low.trim_end().ends_with(" on") {
                st.radio_known = true;
                st.radio_on = true;
            } else if low.contains(": off") || low.trim_end().ends_with(" off") {
                st.radio_known = true;
                st.radio_on = false;
            }
        }
    }
    if st.radio_on {
        st.ssid = run_cmd("networksetup", &["-getairportnetwork", &iface])
            .and_then(|(ok, out)| if ok { parse_current_ssid(&out) } else { None })
            .or_else(|| {
                run_cmd("ipconfig", &["getsummary", &iface])
                    .and_then(|(ok, out)| if ok { parse_current_ssid(&out) } else { None })
            });
    }
    Some(st)
}

/// Имя сети Wi-Fi, к которой подключён беспроводной адаптер.
pub(super) unsafe fn wifi_ssid() -> Option<String> {
    wifi_status().and_then(|s| s.ssid)
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

    #[test]
    fn phy_radio_on_requires_both_software_and_hardware() {
        let mut rs = WLAN_RADIO_STATE::default();
        assert_eq!(phy_radio_on(&rs), (false, false));

        rs.dwNumberOfPhys = 1;
        rs.PhyRadioState[0] = WLAN_PHY_RADIO_STATE {
            dwPhyIndex: 0,
            dot11SoftwareRadioState: dot11_radio_state_unknown,
            dot11HardwareRadioState: dot11_radio_state_unknown,
        };
        assert_eq!(phy_radio_on(&rs), (false, false));

        rs.PhyRadioState[0].dot11SoftwareRadioState = dot11_radio_state_on;
        rs.PhyRadioState[0].dot11HardwareRadioState = dot11_radio_state_off;
        assert_eq!(phy_radio_on(&rs), (true, false));

        rs.PhyRadioState[0].dot11HardwareRadioState = dot11_radio_state_on;
        assert_eq!(phy_radio_on(&rs), (true, true));
    }

    #[test]
    fn macos_wifi_parsers_extract_iface_ssid_and_networks() {
        let ports = "\
Hardware Port: Thunderbolt Bridge
Device: bridge0
Hardware Port: Wi-Fi
Device: en0
Ethernet Address: aa:bb:cc:dd:ee:ff
";
        assert_eq!(parse_wifi_device(ports).as_deref(), Some("en0"));
        assert_eq!(
            parse_current_ssid("Current Wi-Fi Network: HomeNet\n").as_deref(),
            Some("HomeNet")
        );
        assert_eq!(
            parse_current_ssid("You are not associated with an AirPort network."),
            None
        );

        let prof = "\
Wi-Fi:

      Interfaces:
        en0:
          Current Network Information:
            HomeNet:
              Security: WPA2 Personal
              Signal / Noise: -52 dBm / -90 dBm
          Other Local Wi-Fi Networks:
            CafeGuest:
              Security: None
              Signal / Noise: -70 dBm / -92 dBm
";
        let list = parse_airport_profiler(prof, &[], Some("HomeNet"));
        assert_eq!(list.len(), 2);
        assert!(list[0].1.active);
        assert_eq!(list[0].1.name, "HomeNet");
        assert_eq!(list[1].1.name, "CafeGuest");
        assert_eq!(list[1].1.extra, "open|none");
    }
}
