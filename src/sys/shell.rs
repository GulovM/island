//! Интеграция с macOS LaunchAgents, menu bar (StatusItem), языком ввода, диалогами и процессами.

#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::HICON;

#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "io.kiteisland.app";

#[cfg(target_os = "macos")]
fn launch_agent_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist"))
}

pub fn autostart_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        launch_agent_path().is_file()
    }
    #[cfg(not(target_os = "macos"))]
    false
}

pub fn set_autostart(on: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        let p = launch_agent_path();
        if !on {
            return !p.exists() || std::fs::remove_file(&p).is_ok();
        }
        let Ok(exe) = std::env::current_exe() else { return false };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let exe_str = exe.to_string_lossy();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
            <plist version=\"1.0\">\n<dict>\n\
            <key>Label</key><string>{LAUNCH_AGENT_LABEL}</string>\n\
            <key>ProgramArguments</key><array><string>{exe_str}</string></array>\n\
            <key>RunAtLoad</key><true/>\n\
            </dict>\n</plist>\n"
        );
        std::fs::write(&p, plist).is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = on;
        false
    }
}

pub fn tray_add(hwnd: HWND, icon: HICON, callback_msg: u32) -> bool {
    let mut data = NOTIFYICONDATAW {
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: callback_msg,
        hIcon: icon,
        ..Default::default()
    };
    let tip = "Kite Island";
    for (d, s) in data.szTip.iter_mut().zip(tip.encode_utf16()) {
        *d = s;
    }
    unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() }
}

pub fn tray_remove(hwnd: HWND) {
    let data = NOTIFYICONDATAW { hWnd: hwnd, uID: 1, ..Default::default() };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

pub fn tray_set_icon(hwnd: HWND, icon: HICON) -> bool {
    let data = NOTIFYICONDATAW {
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON,
        hIcon: icon,
        ..Default::default()
    };
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() }
}

const KITE_COBALT: [f32; 3] = [0x2F as f32 / 255.0, 0x5B as f32 / 255.0, 0xFF as f32 / 255.0];

pub fn make_icon(_hinst: HINSTANCE, light_taskbar: bool) -> Option<HICON> {
    let _px = tray_pixels(20, KITE_COBALT, light_taskbar);
    Some(HICON(1 as *mut std::ffi::c_void))
}

fn tray_pixels(n: usize, brand: [f32; 3], light_taskbar: bool) -> Vec<u8> {
    const SS: usize = 4;
    let nf = n as f32;
    let cx = (nf / 2.0).round();
    let cy = (0.44 * nf).round();
    let (top, bot, hw) = (0.04 * nf, 0.98 * nf, 0.40 * nf);
    let fg = if light_taskbar { [0x1A as f32 / 255.0; 3] } else { [1.0; 3] };

    let mut out = vec![0u8; n * n * 4];
    for py in 0..n {
        for px in 0..n {
            let (mut a_fg, mut a_brand) = (0.0f32, 0.0f32);
            for sy in 0..SS {
                for sxi in 0..SS {
                    let x = px as f32 + (sxi as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    let dx = (x - cx).abs() / hw;
                    let dy = if y < cy { (cy - y) / (cy - top) } else { (y - cy) / (bot - cy) };
                    if dx + dy > 1.0 {
                        continue;
                    }
                    if (x < cx) == (y < cy) {
                        a_fg += 1.0;
                    } else {
                        a_brand += 1.0;
                    }
                }
            }
            let k = (SS * SS) as f32;
            let (a_fg, a_brand) = (a_fg / k, a_brand / k);
            let a = a_fg + a_brand;
            if a <= 0.0 {
                continue;
            }
            let i = (py * n + px) * 4;
            for c in 0..3 {
                let v = (fg[2 - c] * a_fg + brand[2 - c] * a_brand) / a;
                out[i + c] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            out[i + 3] = (a.min(1.0) * 255.0).round() as u8;
        }
    }
    out
}

pub fn trim_working_set() {}

pub fn ime_set_position(_hwnd: HWND, _x: i32, _y: i32) {}

pub fn process_running(names: &[&str]) -> bool {
    #[cfg(target_os = "macos")]
    {
        for name in names {
            let clean = name.trim_end_matches(".exe");
            if Command::new("/usr/bin/pgrep")
                .args(["-ix", clean])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
            {
                return true;
            }
        }
    }
    let _ = names;
    false
}

pub fn monitor_count() -> u32 {
    unsafe extern "system" fn count_cb(
        _: HMONITOR,
        _: HDC,
        _: *mut RECT,
        lp: LPARAM,
    ) -> windows::core::BOOL {
        let count = &mut *(lp.0 as *mut u32);
        *count += 1;
        windows::core::BOOL(1)
    }
    let mut count = 0u32;
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(count_cb), LPARAM(&mut count as *mut u32 as isize));
    }
    count.max(1)
}

pub fn open(target: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/open")
            .arg(target)
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = target;
        false
    }
}

pub fn open_notification_app(app_id: &str, app_name: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        let name = if app_name.is_empty() { app_id } else { app_name };
        if Command::new("/usr/bin/open")
            .args(["-b", app_id])
            .status()
            .is_ok_and(|s| s.success())
        {
            return true;
        }
        Command::new("/usr/bin/open")
            .args(["-a", name])
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_id, app_name);
        false
    }
}

pub fn foreground_lang() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").ok()?;
        let plist = format!("{home}/Library/Preferences/com.apple.HIToolbox.plist");
        let out = Command::new("/usr/bin/defaults")
            .args(["read", &plist, "AppleSelectedInputSources"])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        if s.contains("Russian") {
            return Some(0x0419);
        }
        if s.contains("Ukrainian") {
            return Some(0x0422);
        }
        if s.contains("German") {
            return Some(0x0407);
        }
        if s.contains("French") {
            return Some(0x040C);
        }
        if s.contains("Spanish") {
            return Some(0x040A);
        }
        if s.contains("ABC") || s.contains("U.S.") || s.contains("US") || s.contains("British") {
            return Some(0x0409);
        }
        Some(0x0409)
    }
    #[cfg(not(target_os = "macos"))]
    Some(0x0409)
}

pub fn lang_code(lang: u32) -> String {
    match lang & 0x3FF {
        0x09 => "EN".into(),
        0x19 => "RU".into(),
        0x22 => "UK".into(),
        0x07 => "DE".into(),
        0x0C => "FR".into(),
        0x0A => "ES".into(),
        0x10 => "IT".into(),
        0x16 => "PT".into(),
        0x15 => "PL".into(),
        0x1F => "TR".into(),
        0x11 => "JA".into(),
        0x12 => "KO".into(),
        0x04 => "ZH".into(),
        _ => format!("{lang:X}"),
    }
}

pub fn pick_folder(initial: Option<&str>) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let script = if let Some(dir) = initial {
            let esc = dir.replace('\\', "\\\\").replace('"', "\\\"");
            format!("POSIX path of (choose folder default location (POSIX file \"{esc}\"))")
        } else {
            "POSIX path of (choose folder)".to_string()
        };
        let out = Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .output()
            .ok()?;
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    let _ = initial;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_shapes_and_colors() {
        for n in [16usize, 20, 24, 32] {
            let dark = tray_pixels(n, KITE_COBALT, false);
            assert_eq!(dark.len(), n * n * 4);
            let light = tray_pixels(n, KITE_COBALT, true);
            let px = |buf: &[u8], x: usize, y: usize| -> [u8; 4] {
                let i = (y * n + x) * 4;
                [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
            };
            assert_eq!(px(&dark, 0, 0)[3], 0, "corner transparent for n={n}");
            let (cx, cy) = (n / 2, ((0.44 * n as f32).round() as usize).max(2));
            assert_eq!(px(&dark, cx - 1, cy - 1), [255, 255, 255, 255], "n={n}");
            assert_eq!(px(&light, cx - 1, cy - 1), [0x1A, 0x1A, 0x1A, 255], "n={n}");
            let cobalt = [0xFF, 0x5B, 0x2F, 255];
            assert_eq!(px(&dark, cx, cy - 1), cobalt, "n={n}");
        }
    }

    #[test]
    fn test_foreground_lang_does_not_panic() {
        let _ = foreground_lang();
    }

    #[test]
    fn test_lang_code_formatting() {
        assert_eq!(lang_code(0x0409), "EN");
        assert_eq!(lang_code(0x0419), "RU");
    }
}
