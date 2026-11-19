//! Системные вызовы: питание/батарея, буфер обмена, оформление, обои, шрифты и автозапуск.

mod appearance;
mod clipboard;
mod energy_saver;
mod night_light;
mod notif_banners;
mod paths;
mod registry;
mod seal;
mod shell;
mod uia;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub use appearance::{
    animations_enabled, current_wallpaper, dark_mode, is_theme_setting_str, light_mode,
    set_dark_mode, set_wallpaper, system_colors, taskbar_light, text_scale_factor,
    transparency_enabled, ColorRetry,
};
pub use clipboard::{
    clipboard_excluded, clipboard_files, clipboard_has_files, clipboard_has_image,
    clipboard_has_text, clipboard_image, clipboard_owned_by, clipboard_text, set_clipboard_dib,
    set_clipboard_dib_with_file, set_clipboard_files, set_clipboard_text,
};
pub use energy_saver::{energy_saver, set_energy_saver};
pub use night_light::{night_light, set_night_light};
pub use notif_banners::{
    init_notif_banners, notif_banners_active, notif_banners_need_reboot,
    set_notif_banners_suppressed, suppress_notif_banner,
};
pub use paths::{pictures_dir, windows_dir};
pub use registry::{reg_str, windows_build_number};
pub use seal::{is_sealed, seal, unseal};
pub use shell::{
    autostart_enabled, foreground_lang, ime_set_position, lang_code, make_icon, monitor_count,
    open, open_notification_app, pick_folder, process_running, set_autostart, tray_add,
    tray_remove, tray_set_icon, trim_working_set,
};
pub(crate) use uia::{handle_get_object, report_focus};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    pub ac_online: bool,
    pub charging: bool,
    pub saver_on: bool,
    pub percent: u8,
}

pub fn battery_status() -> Option<BatteryStatus> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/pmset")
            .args(["-g", "batt"])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        let ac_online = s.contains("AC Power");
        let mut percent = None;
        let mut charging = false;
        for line in s.lines() {
            if line.contains("InternalBattery") {
                if let Some(pct_i) = line.find('%') {
                    let prefix = &line[..pct_i];
                    if let Some(num) = prefix.split(|c: char| !c.is_ascii_digit()).rfind(|p| !p.is_empty()) {
                        percent = num.parse::<u8>().ok().map(|v| v.min(100));
                    }
                }
                charging = line.contains("; charging;") || line.contains("; finishing charge;");
                break;
            }
        }
        (percent.is_some() || ac_online).then_some(BatteryStatus {
            ac_online,
            charging,
            saver_on: energy_saver().unwrap_or(false),
            percent: percent.unwrap_or(100),
        })
    }
    #[cfg(not(target_os = "macos"))]
    None
}

pub fn sha256(data: &[u8]) -> Option<[u8; 32]> {
    Some(seal::sha256_bytes(data))
}

pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Kite Island");
    }
    std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Kite Island")
}

pub fn wz(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn with_utf16<R>(s: &str, f: impl FnOnce(&[u16]) -> R) -> R {
    let mut stack = [0u16; 160];
    let mut len = 0usize;
    for u in s.encode_utf16() {
        if len >= stack.len() {
            let v: Vec<u16> = s.encode_utf16().collect();
            return f(&v);
        }
        stack[len] = u;
        len += 1;
    }
    f(&stack[..len])
}

pub fn acquire_single_instance() -> bool {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let lock_path = dir.join("instance.lock");
        if let Ok(file) = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(lock_path) {
            let rc = unsafe { flock(file.as_raw_fd(), 2 | 4) }; // LOCK_EX | LOCK_NB
            if rc != 0 {
                return false;
            }
            // Keep fd alive for process lifetime so kernel drops lock on termination
            std::mem::forget(file);
            return true;
        }
    }
    true
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::with_utf16;

    #[test]
    fn matches_encode_utf16() {
        for s in ["", "abc", "Привет, мир", "😀 emoji", &"я".repeat(160), &"я".repeat(161), &"😀".repeat(200)] {
            let expected: Vec<u16> = s.encode_utf16().collect();
            with_utf16(s, |got| assert_eq!(got, expected.as_slice()));
        }
    }
}
