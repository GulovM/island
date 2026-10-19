//! Тема оформления Windows (светлая/тёмная), акцентные цвета и обои рабочего стола.

use std::ffi::c_void;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegSetKeyValueW, REG_DWORD};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SystemParametersInfoW, HWND_BROADCAST, SMTO_ABORTIFHUNG, SPIF_SENDCHANGE,
    SPIF_UPDATEINIFILE, SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    WM_SETTINGCHANGE,
};

use crate::theme::{parse_accent_palette, SystemColors};
use super::{read_binary, read_dword, wz};

const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

/// Тёмная тема приложений включена.
pub fn dark_mode() -> bool {
    read_dword(PERSONALIZE, w!("AppsUseLightTheme")) == Some(0)
}

/// В Windows включена светлая тема приложений (если значения в реестре нет — считаем тёмной).
pub fn light_mode() -> bool {
    read_dword(PERSONALIZE, w!("AppsUseLightTheme")) == Some(1)
}

/// В Windows включены «Эффекты прозрачности» (без значения в реестре — включены).
pub fn transparency_enabled() -> bool {
    read_dword(PERSONALIZE, w!("EnableTransparency")) != Some(0)
}

/// Переключает тему приложений и системы и сообщает всем окнам (`WM_SETTINGCHANGE`).
pub fn set_dark_mode(dark: bool) {
    unsafe {
        let v: u32 = if dark { 0 } else { 1 };
        for name in [w!("AppsUseLightTheme"), w!("SystemUsesLightTheme")] {
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PERSONALIZE,
                name,
                REG_DWORD.0,
                Some(&v as *const u32 as *const c_void),
                4,
            );
        }
        let area = wz("ImmersiveColorSet");
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(area.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            300,
            None,
        );
    }
}

/// Ставит обои рабочего стола.
pub fn set_wallpaper(path: &str) -> bool {
    unsafe {
        let wide = wz(path);
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_ptr() as *mut c_void),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
        .is_ok()
    }
}

/// Путь текущих обоев.
pub fn current_wallpaper() -> Option<String> {
    unsafe {
        let mut buf = [0u16; 520];
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
    }
}

/// Акцентные цвета Windows. Их же подстраивает под обои Wallpaper Engine (и сама Windows при
/// «автоматическом выборе цвета фона»): читаем палитру из реестра — это мгновенно и без WinRT.
pub fn system_colors() -> Option<SystemColors> {
    if let Some(p) = read_binary(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"), w!("AccentPalette"))
        .and_then(|b| parse_accent_palette(&b))
    {
        return Some(p);
    }
    // Запасной путь: цвет акцента окон (ABGR).
    let v = read_dword(w!("Software\\Microsoft\\Windows\\DWM"), w!("AccentColor"))?;
    let ch = |shift: u32| ((v >> shift) & 0xFF) as f32 / 255.0;
    Some(SystemColors::from_accent([ch(0), ch(8), ch(16)]))
}
