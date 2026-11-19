//! Управление Bluetooth-устройствами на macOS через `system_profiler`, `defaults` и `blueutil`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use super::audio_policy::audio_set_default;
use super::devices::{audio_devices, set_status, store_list, DevItem, ListKind};
use super::{send, Cmd};
use crate::log;

/// Конечная точка вывода звука с идентификатором Bluetooth-устройства.
#[allow(dead_code)]
pub(super) struct AudioEp {
    pub(super) container: String,
    pub(super) endpoint: String,
    pub(super) active: bool,
    pub(super) bt_filter: Option<String>,
}

/// Сопряжённое Bluetooth-устройство: `(адрес/id, имя, ключ аудио)`.
pub(super) type Paired = (String, String, String);

#[derive(Clone, Debug, PartialEq, Eq)]
struct BtRecord {
    id: String,
    name: String,
    connected: bool,
    is_audio: bool,
}

static PAIRED: Mutex<Option<(Vec<Paired>, Instant)>> = Mutex::new(None);
static BT_ANY_CONNECTED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
fn run_cmd(prog: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = std::process::Command::new(prog).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    Some((out.status.success(), text))
}

#[cfg(not(target_os = "macos"))]
fn run_cmd(_prog: &str, _args: &[&str]) -> Option<(bool, String)> {
    None
}

fn blueutil_bin() -> Option<&'static str> {
    for candidate in ["/opt/homebrew/bin/blueutil", "/usr/local/bin/blueutil", "blueutil"] {
        if candidate == "blueutil" {
            if run_cmd(candidate, &["--version"]).is_some_and(|(ok, _)| ok) {
                return Some(candidate);
            }
        } else if std::path::Path::new(candidate).exists() {
            return Some(candidate);
        }
    }
    None
}

fn is_audio_minor_type(minor: &str, name: &str) -> bool {
    let m = minor.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    m.contains("headphone")
        || m.contains("headset")
        || m.contains("speaker")
        || m.contains("audio")
        || n.contains("airpods")
        || n.contains("beats")
        || n.contains("buds")
        || n.contains("headphone")
}

/// Разбирает вывод `system_profiler SPBluetoothDataType` на `(power_on, список_устройств)`.
fn parse_bluetooth_profiler(text: &str) -> (Option<bool>, Vec<BtRecord>) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Section {
        None,
        Controller,
        Connected,
        NotConnected,
        Legacy,
    }

    let mut power: Option<bool> = None;
    let mut devices: Vec<BtRecord> = Vec::new();
    let mut section = Section::None;
    let mut dev_indent: Option<usize> = None;

    let mut cur_name: Option<String> = None;
    let mut cur_addr = String::new();
    let mut cur_minor = String::new();
    let mut cur_conn: Option<bool> = None;

    let flush = |devices: &mut Vec<BtRecord>,
                 sec: Section,
                 name: &mut Option<String>,
                 addr: &mut String,
                 minor: &mut String,
                 conn: &mut Option<bool>| {
        let Some(dev_name) = name.take() else {
            addr.clear();
            minor.clear();
            *conn = None;
            return;
        };
        let connected = conn.unwrap_or(sec == Section::Connected);
        let id = if addr.is_empty() { dev_name.clone() } else { addr.clone() };
        let is_audio = is_audio_minor_type(minor, &dev_name);
        devices.push(BtRecord { id, name: dev_name, connected, is_audio });
        addr.clear();
        minor.clear();
        *conn = None;
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();

        if trimmed == "Bluetooth Controller:" {
            flush(&mut devices, section, &mut cur_name, &mut cur_addr, &mut cur_minor, &mut cur_conn);
            section = Section::Controller;
            dev_indent = None;
            continue;
        }
        if trimmed == "Connected:" && indent <= 8 {
            flush(&mut devices, section, &mut cur_name, &mut cur_addr, &mut cur_minor, &mut cur_conn);
            section = Section::Connected;
            dev_indent = None;
            continue;
        }
        if trimmed == "Not Connected:" && indent <= 8 {
            flush(&mut devices, section, &mut cur_name, &mut cur_addr, &mut cur_minor, &mut cur_conn);
            section = Section::NotConnected;
            dev_indent = None;
            continue;
        }
        if trimmed.starts_with("Devices (") && trimmed.ends_with(':') {
            flush(&mut devices, section, &mut cur_name, &mut cur_addr, &mut cur_minor, &mut cur_conn);
            section = Section::Legacy;
            dev_indent = None;
            continue;
        }

        if section == Section::Controller {
            if let Some((k, v)) = trimmed.split_once(':') {
                if k.trim() == "State" {
                    let val = v.trim();
                    if val.eq_ignore_ascii_case("On") {
                        power = Some(true);
                    } else if val.eq_ignore_ascii_case("Off") {
                        power = Some(false);
                    }
                }
            }
            continue;
        }

        if matches!(section, Section::Connected | Section::NotConnected | Section::Legacy) {
            if trimmed.ends_with(':') && !trimmed.contains(": ") {
                let target_indent = *dev_indent.get_or_insert(indent);
                if indent <= target_indent {
                    flush(&mut devices, section, &mut cur_name, &mut cur_addr, &mut cur_minor, &mut cur_conn);
                    cur_name = Some(trimmed.trim_end_matches(':').to_string());
                    continue;
                }
            }
            if let Some((k, v)) = trimmed.split_once(':') {
                let k = k.trim();
                let v = v.trim();
                match k {
                    "Address" => cur_addr = v.to_string(),
                    "Minor Type" => cur_minor = v.to_string(),
                    "Connected" => {
                        cur_conn = Some(v.eq_ignore_ascii_case("Yes") || v.eq_ignore_ascii_case("True"));
                    }
                    _ => {}
                }
            }
        }
    }
    flush(&mut devices, section, &mut cur_name, &mut cur_addr, &mut cur_minor, &mut cur_conn);
    (power, devices)
}

/// Читает состояние радио Bluetooth на macOS (`blueutil` -> `defaults` -> `system_profiler`).
pub(super) fn bt_power_on() -> Option<bool> {
    if let Some(bin) = blueutil_bin() {
        if let Some((ok, out)) = run_cmd(bin, &["--power"]) {
            if ok {
                let t = out.trim();
                if t == "1" {
                    return Some(true);
                }
                if t == "0" {
                    return Some(false);
                }
            }
        }
    }
    if let Some((ok, out)) = run_cmd("defaults", &["read", "/Library/Preferences/com.apple.Bluetooth", "ControllerPowerState"]) {
        if ok {
            let t = out.trim();
            if t == "1" {
                return Some(true);
            }
            if t == "0" {
                return Some(false);
            }
        }
    }
    let (ok, out) = run_cmd("system_profiler", &["SPBluetoothDataType"])?;
    if !ok {
        return None;
    }
    let (power, devs) = parse_bluetooth_profiler(&out);
    BT_ANY_CONNECTED.store(devs.iter().any(|d| d.connected), Ordering::Relaxed);
    power
}

/// Включает или выключает радио Bluetooth на macOS (через `blueutil`, если установлен).
pub(super) fn set_bt_power(on: bool) -> bool {
    if let Some(bin) = blueutil_bin() {
        let arg = if on { "1" } else { "0" };
        if run_cmd(bin, &["--power", arg]).is_some_and(|(ok, _)| ok) {
            if !on {
                BT_ANY_CONNECTED.store(false, Ordering::Relaxed);
            }
            return true;
        }
    }
    false
}

/// Все известные конечные точки звука, сопоставленные с аудиоустройствами macOS.
pub(super) unsafe fn audio_endpoints() -> Vec<AudioEp> {
    audio_devices()
        .into_iter()
        .map(|d| AudioEp {
            container: d.id.clone(),
            endpoint: d.id,
            active: d.active,
            bt_filter: Some(d.name),
        })
        .collect()
}

unsafe fn paired_bt() -> (Vec<BtRecord>, Vec<AudioEp>) {
    let eps = audio_endpoints();
    let devs = run_cmd("system_profiler", &["SPBluetoothDataType"])
        .filter(|(ok, _)| *ok)
        .map(|(_, text)| parse_bluetooth_profiler(&text).1)
        .unwrap_or_default();

    BT_ANY_CONNECTED.store(devs.iter().any(|d| d.connected), Ordering::Relaxed);
    let paired: Vec<Paired> = devs
        .iter()
        .map(|d| (d.id.clone(), d.name.clone(), d.id.clone()))
        .collect();
    if let Ok(mut p) = PAIRED.lock() {
        *p = Some((paired, Instant::now()));
    }
    (devs, eps)
}

/// Сопряжённые устройства из кэша (не старше минуты).
#[allow(dead_code)]
pub(super) unsafe fn paired_cached() -> Vec<Paired> {
    let cached = PAIRED
        .lock()
        .ok()
        .and_then(|p| p.as_ref().filter(|(_, t)| t.elapsed().as_secs() < 60).map(|(v, _)| v.clone()));
    cached.unwrap_or_else(|| paired_bt().0.into_iter().map(|d| (d.id.clone(), d.name, d.id)).collect())
}

/// Подключение/отключение Bluetooth-устройства в отдельном потоке.
pub(super) fn bt_spawn(target: String, connect: bool) {
    let lists = [ListKind::Bluetooth, ListKind::Audio];
    set_status(&lists, if connect { "Подключение…" } else { "Отключение…" });
    let spawned = std::thread::Builder::new().name("island-bt".into()).spawn(move || unsafe {
        let (addr, name) = target
            .split_once('|')
            .map(|(a, n)| (a.to_string(), n.to_string()))
            .unwrap_or_else(|| (target.clone(), target.clone()));
        let mut ok = false;

        if let Some(bin) = blueutil_bin() {
            if connect && bt_power_on() == Some(false) {
                let _ = run_cmd(bin, &["--power", "1"]);
                std::thread::sleep(std::time::Duration::from_millis(800));
            }
            let flag = if connect { "--connect" } else { "--disconnect" };
            ok = run_cmd(bin, &[flag, &addr]).is_some_and(|(s, _)| s);
        }

        if ok && connect {
            std::thread::sleep(std::time::Duration::from_millis(700));
        }

        let audio = audio_devices();
        if connect {
            let nl = name.to_ascii_lowercase();
            if let Some(ep) = audio.into_iter().find(|d| d.name.to_ascii_lowercase().contains(&nl) || nl.contains(&d.name.to_ascii_lowercase())) {
                audio_set_default(&ep.id);
                ok = true;
            }
        }

        store_list(ListKind::Bluetooth, bluetooth_devices());
        store_list(ListKind::Audio, audio_devices());
        let msg = match (ok, connect) {
            (true, true) => "Подключено",
            (true, false) => "Отключено",
            (false, true) => "Подключите в Настройках Bluetooth (или через blueutil)",
            (false, false) => "Не удалось отключить",
        };
        set_status(&lists, msg);
        send(Cmd::Refresh);
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить поток Bluetooth: {e}"));
    }
}

/// Сопряжённые Bluetooth-устройства macOS; аудиоустройства имеют непустой `extra` для клика.
pub(super) unsafe fn bluetooth_devices() -> Vec<DevItem> {
    let (devs, eps) = paired_bt();
    let mut out = Vec::with_capacity(devs.len());
    for d in devs {
        let name_low = d.name.to_ascii_lowercase();
        let has_audio_ep = d.is_audio
            || eps
                .iter()
                .filter_map(|e| e.bt_filter.as_deref())
                .any(|n| n.to_ascii_lowercase().contains(&name_low) || name_low.contains(&n.to_ascii_lowercase()));
        let can_toggle = has_audio_ep || blueutil_bin().is_some();
        let extra = if can_toggle { format!("{}|{}", d.id, d.name) } else { String::new() };
        let sub = match (d.connected, !extra.is_empty()) {
            (true, _) => "Подключено",
            (false, true) => "Сопряжено • нажмите, чтобы подключить",
            (false, false) => "Сопряжено",
        }
        .to_string();
        out.push(DevItem { id: d.id, name: d.name, sub, active: d.connected, extra });
    }
    out.sort_by_key(|d| (std::cmp::Reverse(d.active), d.name.to_lowercase()));
    out
}

/// Есть ли сейчас хотя бы одно подключённое Bluetooth-устройство.
pub(super) fn bt_any_connected() -> bool {
    BT_ANY_CONNECTED.load(Ordering::Relaxed)
}

/// Обновляет снимок подключённых Bluetooth-устройств для индикатора в трее/островке.
pub(super) unsafe fn ensure_bt_watch() {
    let _ = paired_bt();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_macos_bluetooth_profiler() {
        let sample = "\
Bluetooth:

      Bluetooth Controller:
          Address: 11:22:33:44:55:66
          State: On
      Connected:
          AirPods Pro:
              Address: AA:BB:CC:DD:EE:01
              Minor Type: Headphones
      Not Connected:
          Magic Mouse:
              Address: AA:BB:CC:DD:EE:02
              Minor Type: Mouse
";
        let (power, devs) = parse_bluetooth_profiler(sample);
        assert_eq!(power, Some(true));
        assert_eq!(devs.len(), 2);
        assert_eq!(devs[0].name, "AirPods Pro");
        assert!(devs[0].connected);
        assert!(devs[0].is_audio);
        assert_eq!(devs[1].name, "Magic Mouse");
        assert!(!devs[1].connected);
        assert!(!devs[1].is_audio);
    }
}
