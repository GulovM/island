//! Списки устройств (Wi-Fi / Bluetooth / Audio) и их кэш для выпадающих меню острова.

use std::sync::Mutex;

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::IMMDevice;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Wifi,
    Bluetooth,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevItem {
    pub id: String,
    pub name: String,
    pub sub: String,
    pub active: bool,
    pub extra: String,
}

static LISTS: Mutex<[Vec<DevItem>; 3]> = Mutex::new([Vec::new(), Vec::new(), Vec::new()]);
static STATUS: Mutex<[Option<String>; 3]> = Mutex::new([None, None, None]);

fn idx(k: ListKind) -> usize {
    match k {
        ListKind::Wifi => 0,
        ListKind::Bluetooth => 1,
        ListKind::Audio => 2,
    }
}

pub fn status(kind: ListKind) -> Option<String> {
    STATUS.lock().ok().and_then(|s| s[idx(kind)].clone())
}

pub(super) fn set_status(kinds: &[ListKind], text: &str) {
    if let Ok(mut s) = STATUS.lock() {
        for &k in kinds {
            s[idx(k)] = if text.is_empty() { None } else { Some(text.to_string()) };
        }
    }
    super::notify();
}

pub fn list(kind: ListKind) -> Vec<DevItem> {
    LISTS.lock().ok().map(|l| l[idx(kind)].clone()).unwrap_or_default()
}

pub(super) fn store_list(kind: ListKind, items: Vec<DevItem>) {
    if let Ok(mut l) = LISTS.lock() {
        l[idx(kind)] = items;
    }
    super::notify();
}

#[allow(dead_code)]
pub(super) unsafe fn device() -> Option<IMMDevice> {
    None
}

#[allow(dead_code)]
pub(super) unsafe fn endpoint() -> Option<IAudioEndpointVolume> {
    None
}

#[cfg(test)]
pub(super) unsafe fn device_name(_dev: &IMMDevice) -> Option<String> {
    None
}

#[allow(dead_code)]
pub(super) unsafe fn device_id(_dev: &IMMDevice) -> Option<String> {
    None
}

pub(super) unsafe fn audio_devices() -> Vec<DevItem> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("/usr/sbin/system_profiler")
            .args(["SPAudioDataType"])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            let mut items = Vec::new();
            let mut cur_name: Option<String> = None;
            let mut is_out = false;
            let mut is_default = false;
            let push_dev = |items: &mut Vec<DevItem>, name: String, active: bool| {
                let sub = if active {
                    crate::i18n::pick("По умолчанию", "Default").to_string()
                } else {
                    String::new()
                };
                items.push(DevItem { id: name.clone(), name, sub, active, extra: String::new() });
            };
            for line in s.lines() {
                let trim = line.trim();
                let indent = line.len() - line.trim_start().len();
                if indent == 8 && trim.ends_with(':') {
                    if let (Some(name), true) = (cur_name.take(), is_out) {
                        push_dev(&mut items, name, is_default);
                    }
                    cur_name = Some(trim.trim_end_matches(':').to_string());
                    is_out = false;
                    is_default = false;
                } else if trim.starts_with("Output Channels:") {
                    is_out = true;
                } else if trim == "Default Output Device: Yes" {
                    is_default = true;
                }
            }
            if let (Some(name), true) = (cur_name, is_out) {
                push_dev(&mut items, name, is_default);
            }
            if !items.is_empty() {
                return items;
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_no_panic() {
        unsafe {
            assert!(device_name(&IMMDevice).is_none());
        }
    }
}
