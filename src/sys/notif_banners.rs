//! Управление флагом `ShowBanner` в настройках уведомлений Windows.
//!
//! При активных всплывающих уведомлениях острова каждому приложению в
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\<AppID>` выставляется
//! `ShowBanner = 0`: системные баннеры Windows не появляются, но уведомления продолжают
//! поступать в Центр уведомлений (и, следовательно, в `UserNotificationListener`).
//! Исходные значения сохраняются в `%APPDATA%\island\win_banners_backup.txt` и возвращаются
//! при ручном выключении опции.
//!
//! Исходное состояние опции в текущем сеансе Windows хранится в RAM-ветке реестра
//! (`REG_OPTION_VOLATILE`) в `HKCU\Software\KiteIslandBannerPending\BootState`: ключ переживает
//! перезапуск острова в текущем сеансе, но автоматически пропадает после перезагрузки или
//! завершения работы ОС. Подсказка «Требуется перезагрузка» показывается только пока текущее
//! состояние отличается от исходного для этого сеанса.

use std::collections::HashSet;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, KEY_WRITE, REG_DWORD,
    REG_OPTION_VOLATILE, RRF_RT_REG_DWORD,
};

use super::{data_dir, wz};

const SETTINGS_ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings";
const REBOOT_MARKER_KEY: PCWSTR = w!("Software\\KiteIslandBannerPending");
const BOOT_STATE_VALUE: PCWSTR = w!("BootState");
const SHOW_BANNER: PCWSTR = w!("ShowBanner");
const ENABLED: PCWSTR = w!("Enabled");
const BACKUP_FILE: &str = "win_banners_backup.txt";

static SUPPRESS_ACTIVE: AtomicBool = AtomicBool::new(false);
static REBOOT_PENDING: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<BannerState>> = Mutex::new(None);

#[derive(Default)]
struct BannerState {
    active: bool,
    boot_enabled: Option<bool>,
    backup_loaded: bool,
    backup: Vec<(String, Option<u32>)>,
    processed: HashSet<String>,
}

struct KeyGuard(HKEY);

impl Drop for KeyGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: хэндл открыт Win32-вызовом в этом модуле и больше не используется.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

fn backup_path() -> PathBuf {
    data_dir().join(BACKUP_FILE)
}

/// Допустимое имя подключа приложения под `Notifications\Settings`.
pub(crate) fn is_valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.eq_ignore_ascii_case("windows.unknown")
        && !id.eq_ignore_ascii_case(crate::tabs::notifications::CALENDAR_APP_ID)
        && !id.contains(['\\', '/', '\0'])
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

fn parse_backup(text: &str) -> Vec<(String, Option<u32>)> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let Some((id_esc, val_str)) = line.split_once('\t') else {
            continue;
        };
        let app_id = unescape(id_esc);
        if !is_valid_app_id(&app_id) {
            continue;
        }
        let orig = if val_str.trim().eq_ignore_ascii_case("none") {
            None
        } else if let Ok(v) = val_str.trim().parse::<u32>() {
            Some(v)
        } else {
            continue;
        };
        if !out.iter().any(|(existing, _): &(String, Option<u32>)| existing.eq_ignore_ascii_case(&app_id)) {
            out.push((app_id, orig));
        }
    }
    out
}

fn serialize_backup(entries: &[(String, Option<u32>)]) -> String {
    let mut out = String::new();
    for (app_id, orig) in entries {
        out.push_str(&escape(app_id));
        out.push('\t');
        match orig {
            Some(v) => out.push_str(&v.to_string()),
            None => out.push_str("none"),
        }
        out.push('\n');
    }
    out
}

fn load_backup_disk() -> Vec<(String, Option<u32>)> {
    std::fs::read_to_string(backup_path())
        .map(|s| parse_backup(&s))
        .unwrap_or_default()
}

fn save_backup_disk(entries: &[(String, Option<u32>)]) {
    let p = backup_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("txt.tmp");
    let data = serialize_backup(entries);
    if std::fs::write(&tmp, data.as_bytes()).is_ok() && std::fs::rename(&tmp, &p).is_err() {
        let _ = std::fs::write(&p, data.as_bytes());
        let _ = std::fs::remove_file(&tmp);
    }
}

fn ensure_backup_loaded(st: &mut BannerState) {
    if !st.backup_loaded {
        st.backup = load_backup_disk();
        st.backup_loaded = true;
    }
}

fn subkey_path(app_id: &str) -> String {
    format!("{SETTINGS_ROOT}\\{app_id}")
}

fn read_subkey_dword(subkey: &str, name: PCWSTR) -> Option<u32> {
    let k = wz(subkey);
    unsafe {
        let mut v = 0u32;
        let mut size = 4u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut size),
        );
        (r == ERROR_SUCCESS).then_some(v)
    }
}

fn write_subkey_dword(subkey: &str, name: PCWSTR, value: u32) -> bool {
    let k = wz(subkey);
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(k.as_ptr()),
            name,
            REG_DWORD.0,
            Some(&value as *const u32 as *const c_void),
            4,
        ) == ERROR_SUCCESS
    }
}

fn subkey_exists(subkey: &str) -> bool {
    let k = wz(subkey);
    unsafe {
        let mut hk = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), None, KEY_SET_VALUE, &mut hk) == ERROR_SUCCESS {
            let _guard = KeyGuard(hk);
            true
        } else {
            false
        }
    }
}

fn enum_notification_subkeys() -> Vec<String> {
    let root = wz(SETTINGS_ROOT);
    let mut out = Vec::new();
    unsafe {
        let mut hk = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(root.as_ptr()), None, KEY_READ, &mut hk) != ERROR_SUCCESS {
            return out;
        }
        let _guard = KeyGuard(hk);
        let mut idx = 0u32;
        let mut buf = vec![0u16; 260];
        loop {
            let mut len = buf.len() as u32;
            let err = RegEnumKeyExW(
                hk,
                idx,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            );
            if err == ERROR_NO_MORE_ITEMS {
                break;
            }
            if err == ERROR_MORE_DATA {
                buf.resize(buf.len() * 2, 0);
                continue;
            }
            idx += 1;
            if err == ERROR_SUCCESS {
                let name = String::from_utf16_lossy(&buf[..len as usize]);
                if is_valid_app_id(&name) {
                    out.push(name);
                }
            }
        }
    }
    out
}

fn read_boot_state() -> Option<bool> {
    let mut val = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: чтение одного DWORD из volatile-ключа в HKCU.
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            REBOOT_MARKER_KEY,
            BOOT_STATE_VALUE,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    (err == ERROR_SUCCESS).then_some(val != 0)
}

fn write_boot_state(enabled: bool) {
    let val = u32::from(enabled);
    // SAFETY: создаём volatile-ключ в HKCU и пишем один DWORD; ключ живёт до перезагрузки/выключения ОС.
    unsafe {
        let mut hk = HKEY::default();
        let err = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            REBOOT_MARKER_KEY,
            None,
            PCWSTR::null(),
            REG_OPTION_VOLATILE,
            KEY_WRITE,
            None,
            &mut hk,
            None,
        );
        if err == ERROR_SUCCESS {
            let _guard = KeyGuard(hk);
            let _ = RegSetKeyValueW(
                hk,
                PCWSTR::null(),
                BOOT_STATE_VALUE,
                REG_DWORD.0,
                Some(&val as *const u32 as *const c_void),
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}

fn needs_reboot_for(boot_enabled: bool, current_enabled: bool) -> bool {
    current_enabled != boot_enabled
}

fn apply_app_locked(st: &mut BannerState, app_id: &str, backup_dirty: &mut bool) -> bool {
    if !is_valid_app_id(app_id) {
        return false;
    }
    let subkey = subkey_path(app_id);
    if read_subkey_dword(&subkey, ENABLED) == Some(0) {
        st.processed.insert(app_id.to_ascii_lowercase());
        return false;
    }
    let cur = read_subkey_dword(&subkey, SHOW_BANNER);
    if !st.backup.iter().any(|(id, _)| id.eq_ignore_ascii_case(app_id)) {
        st.backup.push((app_id.to_string(), cur));
        *backup_dirty = true;
    }
    let changed = if cur != Some(0) {
        write_subkey_dword(&subkey, SHOW_BANNER, 0)
    } else {
        false
    };
    st.processed.insert(app_id.to_ascii_lowercase());
    changed
}

fn enable_all_locked(st: &mut BannerState) -> bool {
    ensure_backup_loaded(st);
    st.active = true;
    SUPPRESS_ACTIVE.store(true, Ordering::Relaxed);
    st.processed.clear();

    let mut backup_dirty = false;
    let mut reg_changed = false;
    for app_id in enum_notification_subkeys() {
        reg_changed |= apply_app_locked(st, &app_id, &mut backup_dirty);
    }
    if backup_dirty || (!st.backup.is_empty() && !backup_path().exists()) {
        save_backup_disk(&st.backup);
    }
    reg_changed
}

fn restore_all_locked(st: &mut BannerState) {
    ensure_backup_loaded(st);
    st.active = false;
    SUPPRESS_ACTIVE.store(false, Ordering::Relaxed);
    st.processed.clear();

    for (app_id, orig) in st.backup.drain(..) {
        if !is_valid_app_id(&app_id) {
            continue;
        }
        let subkey = subkey_path(&app_id);
        match orig {
            Some(v) => {
                if subkey_exists(&subkey) {
                    let _ = write_subkey_dword(&subkey, SHOW_BANNER, v);
                }
            }
            None => {
                let k = wz(&subkey);
                unsafe {
                    let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), SHOW_BANNER);
                }
            }
        }
    }
    let _ = std::fs::remove_file(backup_path());
}

/// Инициализация на старте приложения: фиксирует исходное состояние сеанса и синхронизирует `ShowBanner`.
pub fn init_notif_banners(effective_enabled: bool) {
    let boot = match read_boot_state() {
        Some(v) => v,
        None => {
            write_boot_state(effective_enabled);
            effective_enabled
        }
    };
    REBOOT_PENDING.store(needs_reboot_for(boot, effective_enabled), Ordering::Relaxed);
    let Ok(mut guard) = STATE.lock() else { return };
    let st = guard.get_or_insert_with(BannerState::default);
    st.boot_enabled = Some(boot);
    if effective_enabled {
        let _ = enable_all_locked(st);
    } else if backup_path().exists() {
        restore_all_locked(st);
    }
}

/// Включает или отключает (с полным откатом по бэкапу) скрытие баннеров Windows.
pub fn set_notif_banners_suppressed(enable: bool) {
    let Ok(mut guard) = STATE.lock() else { return };
    let st = guard.get_or_insert_with(BannerState::default);
    let boot = *st.boot_enabled.get_or_insert_with(|| {
        read_boot_state().unwrap_or_else(|| {
            write_boot_state(!enable);
            !enable
        })
    });
    if enable {
        let _ = enable_all_locked(st);
    } else {
        restore_all_locked(st);
    }
    REBOOT_PENDING.store(needs_reboot_for(boot, enable), Ordering::Relaxed);
}

/// Для нового `app_id`, появившегося в `UserNotificationListener`: сохраняет бэкап и ставит `ShowBanner = 0`.
pub fn suppress_notif_banner(app_id: &str) {
    if !SUPPRESS_ACTIVE.load(Ordering::Relaxed) || !is_valid_app_id(app_id) {
        return;
    }
    let Ok(mut guard) = STATE.lock() else { return };
    let Some(st) = guard.as_mut() else { return };
    if !st.active || st.processed.contains(&app_id.to_ascii_lowercase()) {
        return;
    }
    let mut backup_dirty = false;
    let _ = apply_app_locked(st, app_id, &mut backup_dirty);
    if backup_dirty {
        save_backup_disk(&st.backup);
    }
}

/// Активно ли сейчас подавление баннеров Windows.
pub fn notif_banners_active() -> bool {
    SUPPRESS_ACTIVE.load(Ordering::Relaxed)
}

/// Нужно ли перезагрузить Windows, чтобы кэш `WpnUserService` подхватил скрытие баннеров.
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
        // Третья запись содержит `\` и невалидна для имени ключа реестра: пропускается при разборе.
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
        // Состояние совпадает с состоянием при старте сеанса — перезагрузка не требуется.
        assert!(!needs_reboot_for(false, false));
        assert!(!needs_reboot_for(true, true));
        // Изменили в любую сторону — требуется перезагрузка; вернули обратно — снова не требуется.
        assert!(needs_reboot_for(false, true));
        assert!(needs_reboot_for(true, false));
    }
}
