//! Системная интеграция: единственный экземпляр, автозапуск, трей, обрезка working set.

use std::path::PathBuf;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

mod appearance;
mod clipboard;
mod night_light;
mod paths;
mod registry;
mod seal;
mod shell;

pub use appearance::{
    current_wallpaper, dark_mode, light_mode, set_dark_mode, set_wallpaper, system_colors, transparency_enabled,
};
pub use clipboard::{
    clipboard_excluded, clipboard_has_image, clipboard_has_text, clipboard_image, clipboard_owned_by,
    clipboard_text, send_paste, set_clipboard_dib, set_clipboard_text,
};
pub use night_light::{night_light, set_night_light};
pub use paths::{pictures_dir, system_drive_root, windows_dir};
pub use registry::reg_str;
pub(in crate::sys) use registry::{read_binary, read_dword};
pub use seal::{is_sealed, seal, unseal};
pub use shell::{
    autostart_enabled, foreground_lang, ime_set_position, lang_code, make_icon, monitor_count, open,
    pick_folder, process_running, set_autostart, tray_add, tray_remove, trim_working_set,
};

/// `%APPDATA%\island` — конфиг, лог, позже кэши.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("island")
}

/// Строка в UTF-16 с завершающим нулём.
pub fn wz(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 без аллокации для коротких строк: отрисовка зовёт это сотни раз за кадр.
/// Строка длиннее буфера уходит в `Vec`.
pub fn with_utf16<R>(s: &str, f: impl FnOnce(&[u16]) -> R) -> R {
    let mut buf = [0u16; 160];
    let mut n = 0;
    for u in s.encode_utf16() {
        if n == buf.len() {
            let v: Vec<u16> = s.encode_utf16().collect();
            return f(&v);
        }
        buf[n] = u;
        n += 1;
    }
    f(&buf[..n])
}

/// `false` — такой экземпляр уже запущен.
pub fn acquire_single_instance() -> bool {
    unsafe {
        match CreateMutexW(None, true, w!("Local\\Island.SingleInstance")) {
            // Хэндл намеренно не закрываем: HANDLE — просто число, мьютекс живёт до конца процесса.
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            // Не смогли проверить — лучше запуститься, чем молча не запуститься.
            Err(_) => true,
        }
    }
}

/// Секунды с 1970 года.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::with_utf16;

    #[test]
    fn matches_encode_utf16() {
        for s in ["", "abc", "Привет, мир", "😀 emoji", &"я".repeat(160), &"я".repeat(161), &"😀".repeat(200)] {
            let want: Vec<u16> = s.encode_utf16().collect();
            assert_eq!(with_utf16(s, |w| w.to_vec()), want, "{s:?}");
        }
    }
}
