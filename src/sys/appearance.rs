//! Тема оформления Windows (светлая/тёмная), акцентные цвета и обои рабочего стола.

use std::ffi::c_void;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegSetKeyValueW, REG_DWORD};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SystemParametersInfoW, HWND_BROADCAST, SMTO_ABORTIFHUNG, SPIF_SENDCHANGE,
    SPIF_UPDATEINIFILE, SPI_GETCLIENTAREAANIMATION, SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WM_SETTINGCHANGE,
};

use crate::theme::{parse_accent_palette, SystemColors};
use super::{read_binary, read_dword, wz};

const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

/// Тёмная тема приложений включена.
pub fn dark_mode() -> bool {
    read_dword(PERSONALIZE, w!("AppsUseLightTheme"))
        .or_else(|| read_dword(PERSONALIZE, w!("SystemUsesLightTheme")))
        == Some(0)
}

/// В Windows включена светлая тема приложений (если значения в реестре нет — считаем тёмной).
pub fn light_mode() -> bool {
    read_dword(PERSONALIZE, w!("AppsUseLightTheme"))
        .or_else(|| read_dword(PERSONALIZE, w!("SystemUsesLightTheme")))
        == Some(1)
}

/// В Windows включены «Эффекты прозрачности» (без значения в реестре — включены).
pub fn transparency_enabled() -> bool {
    read_dword(PERSONALIZE, w!("EnableTransparency")) != Some(0)
}

/// Доля из `TextScaleFactor` (проценты) → множитель кегля, 1.0..=1.15. Потолок ниже системных
/// 2.25 намеренно: вёрстка острова плотная (хардкоженные смещения виджетов) и не проверена на
/// кегль крупнее — безопасный диапазон даёт что-то, не рискуя обрезкой/наездами.
fn clamp_text_scale(percent: u32) -> f32 {
    (percent as f32 / 100.0).clamp(1.0, 1.15)
}

/// Множитель кегля от системной настройки «Размер текста» (Специальные возможности).
pub fn text_scale_factor() -> f32 {
    let percent = read_dword(w!("Software\\Microsoft\\Accessibility"), w!("TextScaleFactor")).unwrap_or(100);
    clamp_text_scale(percent)
}

/// В Windows включена «Показывать анимацию» (Параметры → Специальные возможности → Эффекты
/// визуализации): без неё пружины острова должны сходиться почти мгновенно.
pub fn animations_enabled() -> bool {
    let mut on = windows::core::BOOL(1);
    unsafe {
        let ok = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut _ as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        ok.is_err() || on.as_bool()
    }
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

/// Планировщик повторных проверок системных цветов.
/// Защищает от гонки: Windows и сторонние программы (в т. ч. Wallpaper Engine) могут слать
/// уведомления до завершения записи новых значений в реестр или обновлять цвета обоев
/// асинхронно в течение нескольких сотен миллисекунд.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorRetry {
    retries_left: u8,
}

impl ColorRetry {
    /// Число повторных проверок после первого события.
    pub const DEFAULT_RETRIES: u8 = 2;
    /// Интервал между повторными проверками в миллисекундах (300–600 мс).
    pub const RETRY_DELAY_MS: u32 = 400;

    pub const fn new() -> Self {
        Self { retries_left: 0 }
    }

    /// Запускает или перезапускает цикл повторных проверок.
    /// Возвращает задержку первого тика в миллисекундах.
    pub fn arm(&mut self) -> u32 {
        self.retries_left = Self::DEFAULT_RETRIES;
        Self::RETRY_DELAY_MS
    }

    /// Обрабатывает очередной тик таймера.
    /// Если попытки ещё остались — возвращает `Some(next_delay_ms)`.
    /// Когда попытки исчерпаны — возвращает `None` (таймер пора снять через `KillTimer`).
    pub fn on_tick(&mut self) -> Option<u32> {
        if self.retries_left == 0 {
            return None;
        }
        self.retries_left -= 1;
        if self.retries_left > 0 {
            Some(Self::RETRY_DELAY_MS)
        } else {
            None
        }
    }
}

/// Проверяет, относится ли строка параметра `WM_SETTINGCHANGE` к оформлению (цвета, тема, обои).
pub fn is_theme_setting_str(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return true;
    }
    let lower = s.to_ascii_lowercase();
    lower.contains("color")
        || lower.contains("theme")
        || lower.contains("accent")
        || lower.contains("personalize")
        || lower.contains("wallpaper")
        || lower == "desktop"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_clamps_to_safe_ceiling() {
        assert_eq!(clamp_text_scale(100), 1.0);
        assert_eq!(clamp_text_scale(110), 1.10);
        // Системные 125%, 150%, 225% — всё упирается в безопасный потолок 1.15.
        assert_eq!(clamp_text_scale(125), 1.15);
        assert_eq!(clamp_text_scale(225), 1.15);
        // Значение меньше 100% (в реестре такого не бывает, но на всякий случай) не уменьшает текст.
        assert_eq!(clamp_text_scale(80), 1.0);
    }

    #[test]
    fn color_retry_lifecycle() {
        let mut cr = ColorRetry::new();
        assert_eq!(cr.on_tick(), None);

        let delay = cr.arm();
        assert_eq!(delay, ColorRetry::RETRY_DELAY_MS);

        // Первый тик — остаётся ещё одна попытка
        assert_eq!(cr.on_tick(), Some(ColorRetry::RETRY_DELAY_MS));

        // Второй тик — попытки исчерпаны
        assert_eq!(cr.on_tick(), None);

        // Последующие тики не уходят в минус и возвращают None
        assert_eq!(cr.on_tick(), None);
    }

    #[test]
    fn color_retry_rearm() {
        let mut cr = ColorRetry::new();
        cr.arm();
        assert_eq!(cr.on_tick(), Some(ColorRetry::RETRY_DELAY_MS));

        // Повторный arm перезапускает счётчик до начального максимума
        cr.arm();
        assert_eq!(cr.on_tick(), Some(ColorRetry::RETRY_DELAY_MS));
        assert_eq!(cr.on_tick(), None);
        assert_eq!(cr.on_tick(), None);
    }

    #[test]
    fn theme_setting_strings_match_expected() {
        assert!(is_theme_setting_str("ImmersiveColorSet"));
        assert!(is_theme_setting_str("WindowsThemeElement"));
        assert!(is_theme_setting_str("ColorPalette"));
        assert!(is_theme_setting_str("Personalize"));
        assert!(is_theme_setting_str("WindowColor"));
        assert!(is_theme_setting_str("Wallpaper"));
        assert!(is_theme_setting_str("desktop"));
        assert!(is_theme_setting_str(""));
        assert!(is_theme_setting_str("   "));

        assert!(!is_theme_setting_str("Environment"));
        assert!(!is_theme_setting_str("intl"));
        assert!(!is_theme_setting_str("Policy"));
        assert!(!is_theme_setting_str("FontSmoothing"));
    }
}
