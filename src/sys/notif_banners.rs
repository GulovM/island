//! Подавление системных баннеров уведомлений на macOS (через Ntfy/Notification Center).

#[cfg(test)]
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static REBOOT_PENDING: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) fn is_valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains('\\')
        && !id.contains('/')
        && !id.contains('\t')
        && !id.contains('\0')
        && id != "windows.unknown"
        && id != "island.calendar"
}

#[cfg(test)]
fn serialize_backup(entries: &[(String, Option<u32>)]) -> String {
    let mut out = String::new();
    for (id, prev) in entries {
        if !is_valid_app_id(id) {
            continue;
        }
        match prev {
            Some(v) => out.push_str(&format!("{id}\t{v}\n")),
            None => out.push_str(&format!("{id}\tnone\n")),
        }
    }
    out
}

#[cfg(test)]
fn parse_backup(text: &str) -> Vec<(String, Option<u32>)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((id, val)) = line.split_once('\t') else { continue };
        if !is_valid_app_id(id) || !seen.insert(id.to_lowercase()) {
            continue;
        }
        let parsed = if val == "none" {
            Some(None)
        } else {
            val.parse::<u32>().ok().map(Some)
        };
        if let Some(p) = parsed {
            out.push((id.to_string(), p));
        }
    }
    out
}

#[cfg(test)]
fn needs_reboot_for(boot_state: bool, current_state: bool) -> bool {
    boot_state != current_state
}

pub fn init_notif_banners(effective_enabled: bool) {
    ACTIVE.store(effective_enabled, Ordering::Relaxed);
    REBOOT_PENDING.store(false, Ordering::Relaxed);
}

pub fn set_notif_banners_suppressed(enable: bool) {
    ACTIVE.store(enable, Ordering::Relaxed);
}

pub fn suppress_notif_banner(_app_id: &str) {}

pub fn notif_banners_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn notif_banners_need_reboot() -> bool {
    REBOOT_PENDING.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_roundtrip_and_dedup() {
        let entries = vec![
            ("Chrome".to_string(), Some(1)),
            ("Telegram.TelegramDesktop".to_string(), None),
            ("Weird\\App\tName".to_string(), Some(0)),
        ];
        let text = serialize_backup(&entries);
        let parsed = parse_backup(&text);
        assert_eq!(
            parsed,
            vec![
                ("Chrome".to_string(), Some(1)),
                ("Telegram.TelegramDesktop".to_string(), None),
            ]
        );

        let dup = "Chrome\t1\nchrome\t0\nBadLine\nAppA\tnone\nAppB\tbad\n";
        assert_eq!(
            parse_backup(dup),
            vec![("Chrome".to_string(), Some(1)), ("AppA".to_string(), None)]
        );
    }

    #[test]
    fn valid_app_id_guards_against_paths_and_sentinel() {
        assert!(is_valid_app_id("Chrome"));
        assert!(is_valid_app_id("Microsoft.ScreenSketch_8wekyb3d8bbwe!App"));
        assert!(!is_valid_app_id(""));
        assert!(!is_valid_app_id("."));
        assert!(!is_valid_app_id(".."));
        assert!(!is_valid_app_id("windows.unknown"));
        assert!(!is_valid_app_id("island.calendar"));
        assert!(!is_valid_app_id("C:\\Program Files\\App.exe"));
        assert!(!is_valid_app_id("foo/bar"));
        assert!(!is_valid_app_id("foo\0bar"));
    }

    #[test]
    fn reboot_required_only_when_differs_from_boot_state() {
        assert!(!needs_reboot_for(false, false));
        assert!(!needs_reboot_for(true, true));
        assert!(needs_reboot_for(false, true));
        assert!(needs_reboot_for(true, false));
    }
}
