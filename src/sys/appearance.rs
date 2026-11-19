//! Системная тема (macOS Dark Mode), обои рабочего стола, цвета и настройки прозрачности/анимаций.

pub use crate::theme::SystemColors;

pub fn dark_mode() -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
        {
            return String::from_utf8_lossy(&out.stdout)
                .trim()
                .eq_ignore_ascii_case("Dark");
        }
    }
    true
}

pub fn light_mode() -> bool {
    !dark_mode()
}

pub fn taskbar_light() -> bool {
    light_mode()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) const fn combine_transparency(reg_enabled: bool, advanced_effects: bool) -> bool {
    reg_enabled && advanced_effects
}

pub fn transparency_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        let reduce = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "com.apple.universalaccess", "reduceTransparency"])
            .output()
            .ok()
            .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1");
        combine_transparency(!reduce, true)
    }
    #[cfg(not(target_os = "macos"))]
    true
}

#[allow(dead_code)]
fn clamp_text_scale(percent: u32) -> f32 {
    (percent as f32 / 100.0).clamp(1.0, 1.15)
}

pub fn text_scale_factor() -> f32 {
    clamp_text_scale(100)
}

pub fn animations_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        let reduce = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "com.apple.universalaccess", "reduceMotion"])
            .output()
            .ok()
            .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1");
        !reduce
    }
    #[cfg(not(target_os = "macos"))]
    true
}

pub fn set_dark_mode(dark: bool) {
    #[cfg(target_os = "macos")]
    {
        let val = if dark { "true" } else { "false" };
        let script = format!(
            "tell application \"System Events\" to tell appearance preferences to set dark mode to {val}"
        );
        let _ = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .status();
    }
    let _ = dark;
}

pub fn set_wallpaper(path: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        let esc = path.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "tell application \"System Events\" to tell every desktop to set picture to POSIX file \"{esc}\""
        );
        std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

pub fn current_wallpaper() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "tell application \"System Events\" to get picture of current desktop",
            ])
            .output()
            .ok()?;
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    None
}

pub fn system_colors() -> Option<SystemColors> {
    #[cfg(target_os = "macos")]
    {
        let code = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleAccentColor"])
            .output()
            .ok()
            .and_then(|o| {
                o.status
                    .success()
                    .then(|| String::from_utf8_lossy(&o.stdout).trim().parse::<i32>().ok())
                    .flatten()
            });
        let accent = match code {
            Some(-1) => [0.55, 0.55, 0.58], // Graphite
            Some(0) => [1.00, 0.27, 0.23],  // Red
            Some(1) => [1.00, 0.58, 0.00],  // Orange
            Some(2) => [1.00, 0.80, 0.00],  // Yellow
            Some(3) => [0.20, 0.78, 0.35],  // Green
            Some(5) => [0.69, 0.32, 0.87],  // Purple
            Some(6) => [1.00, 0.22, 0.48],  // Pink
            _ => [0.04, 0.52, 1.00],        // Apple Blue default
        };
        Some(SystemColors::from_accent(accent))
    }
    #[cfg(not(target_os = "macos"))]
    Some(SystemColors::from_accent([0.04, 0.52, 1.00]))
}

pub struct ColorRetry {
    retries: u8,
    pending: bool,
}

impl Default for ColorRetry {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorRetry {
    pub fn new() -> Self {
        Self { retries: 0, pending: false }
    }

    pub fn trigger(&mut self) {
        self.pending = true;
        self.retries = 2;
    }

    pub fn arm(&mut self) -> u32 {
        self.trigger();
        250
    }

    pub fn on_tick(&mut self) -> Option<u32> {
        if self.pending && self.retries > 0 {
            self.retries -= 1;
            if self.retries > 0 {
                return Some(250);
            }
        }
        self.pending = false;
        self.retries = 0;
        None
    }

    #[cfg(test)]
    pub fn try_complete<F: FnMut() -> bool>(&mut self, mut check: F) -> bool {
        if !self.pending {
            return false;
        }
        if check() {
            self.pending = false;
            self.retries = 0;
            return false;
        }
        if self.retries > 0 {
            self.retries -= 1;
            true
        } else {
            self.pending = false;
            false
        }
    }
}

pub fn is_theme_setting_str(s: &str) -> bool {
    matches!(
        s,
        "ImmersiveColorSet" | "WindowsThemeElement" | "AppleInterfaceThemeChangedNotification"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_clamps_to_safe_ceiling() {
        assert_eq!(clamp_text_scale(100), 1.0);
        assert_eq!(clamp_text_scale(110), 1.10);
        assert_eq!(clamp_text_scale(125), 1.15);
        assert_eq!(clamp_text_scale(225), 1.15);
        assert_eq!(clamp_text_scale(80), 1.0);
    }

    #[test]
    fn color_retry_lifecycle() {
        let mut cr = ColorRetry::new();
        assert!(!cr.try_complete(|| false));

        cr.trigger();
        assert!(cr.try_complete(|| false));
        assert!(cr.try_complete(|| false));
        assert!(!cr.try_complete(|| false));
    }

    #[test]
    fn color_retry_rearm() {
        let mut cr = ColorRetry::new();
        cr.trigger();
        assert!(!cr.try_complete(|| true));
        assert!(!cr.try_complete(|| false));

        cr.trigger();
        assert!(cr.try_complete(|| false));
    }

    #[test]
    fn theme_setting_strings_match_expected() {
        assert!(is_theme_setting_str("ImmersiveColorSet"));
        assert!(is_theme_setting_str("WindowsThemeElement"));
        assert!(!is_theme_setting_str("UserPreferencesMask"));
        assert!(!is_theme_setting_str(""));
    }

    #[test]
    fn transparency_combines_registry_and_advanced_effects() {
        assert!(combine_transparency(true, true));
        assert!(!combine_transparency(false, true));
        assert!(!combine_transparency(true, false));
        assert!(!combine_transparency(false, false));
    }
}
