//! Режим энергосбережения (Windows 11 — «Экономия энергии», Windows 10 — «Экономия заряда»).
//!
//! Публичного вызова «включить сейчас» нет. Состояние читается из `SYSTEM_POWER_STATUS`, а
//! включается режим порогом автовключения в активной схеме питания (`ESBATTTHRESHOLD`):
//! порог 100 % для текущего источника питания включает режим сразу. На Windows 11 24H2+
//! «Экономия энергии» работает и от сети (AC), в том числе на настольных ПК без батареи,
//! поэтому читаются и записываются оба порога — AC и DC. Прежние пороги запоминаются в файле
//! и возвращаются при выключении, чтобы не потерять настройки автовключения пользователя.

use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerGetActiveScheme, PowerReadACValueIndex, PowerReadDCValueIndex,
    PowerSetActiveScheme, PowerWriteACValueIndex, PowerWriteDCValueIndex, SYSTEM_POWER_STATUS,
};

use super::data_dir;

const SUB_ENERGYSAVER: GUID = GUID::from_u128(0xDE830923_A562_41AF_A086_E3A2C6BAD2DA);
const ES_BATT_THRESHOLD: GUID = GUID::from_u128(0xE69653CA_CF7F_4F05_AA73_CB833FA90AD4);

/// Начиная с Windows 11 24H2 (сборка 26100) экономия энергии работает от сети (AC) и без батареи.
const WIN11_24H2_BUILD: u32 = 26100;
/// Порог, при котором режим включён постоянно.
const ALWAYS: u32 = 100;
/// Порог Windows по умолчанию для AC (0 = от сети не включать автоматически).
const DEFAULT_AC_THRESHOLD: u32 = 0;
/// Порог Windows по умолчанию для DC (батарея), если запомненного нет.
const DEFAULT_THRESHOLD: u32 = 30;

fn ok(code: u32) -> bool {
    code == 0
}

fn power_status() -> Option<SYSTEM_POWER_STATUS> {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s).ok()? };
    Some(s)
}

#[inline]
fn has_battery(s: &SYSTEM_POWER_STATUS) -> bool {
    (s.BatteryFlag & 128) == 0 && s.BatteryFlag != 255
}

#[inline]
fn is_saver_active(saver_on: bool, on_ac: bool, ac_thresh: u32, dc_thresh: u32) -> bool {
    saver_on || if on_ac { ac_thresh == ALWAYS } else { dc_thresh == ALWAYS }
}

/// Режим включён (или взведён порогом 100 % для текущего источника питания — AC или DC);
/// `None` — на системе до Windows 11 24H2 без батареи переключать нечего.
pub fn energy_saver() -> Option<bool> {
    let s = power_status()?;
    if !has_battery(&s) && super::windows_build_number() < WIN11_24H2_BUILD {
        return None;
    }
    let (ac, dc) = unsafe { scheme().and_then(|g| read_thresholds(&g)) }?;
    let saver_on = (s.SystemStatusFlag & 1) != 0;
    let on_ac = s.ACLineStatus == 1;
    Some(is_saver_active(saver_on, on_ac, ac, dc))
}

/// Активная схема питания.
unsafe fn scheme() -> Option<GUID> {
    let mut p: *mut GUID = std::ptr::null_mut();
    if !ok(PowerGetActiveScheme(None, &mut p).0) || p.is_null() {
        return None;
    }
    let g = *p;
    let _ = LocalFree(Some(HLOCAL(p as *mut _)));
    Some(g)
}

unsafe fn read_thresholds(s: &GUID) -> Option<(u32, u32)> {
    let (mut ac, mut dc) = (0u32, 0u32);
    let r_ac = PowerReadACValueIndex(None, Some(s), Some(&SUB_ENERGYSAVER), Some(&ES_BATT_THRESHOLD), &mut ac).0;
    let r_dc = PowerReadDCValueIndex(None, Some(s), Some(&SUB_ENERGYSAVER), Some(&ES_BATT_THRESHOLD), &mut dc);
    (ok(r_ac) && ok(r_dc)).then_some((ac, dc))
}

unsafe fn write_thresholds(s: &GUID, ac: u32, dc: u32) -> bool {
    let r_ac = PowerWriteACValueIndex(None, s, Some(&SUB_ENERGYSAVER), Some(&ES_BATT_THRESHOLD), ac).0;
    if !ok(r_ac) {
        crate::log::error(&format!("PowerWriteACValueIndex вернул {r_ac}"));
        return false;
    }
    let r_dc = PowerWriteDCValueIndex(None, s, Some(&SUB_ENERGYSAVER), Some(&ES_BATT_THRESHOLD), dc);
    if !ok(r_dc) {
        crate::log::error(&format!("PowerWriteDCValueIndex вернул {r_dc}"));
        return false;
    }
    let r_act = PowerSetActiveScheme(None, Some(s)).0;
    if !ok(r_act) {
        crate::log::error(&format!("PowerSetActiveScheme вернул {r_act}"));
        return false;
    }
    true
}

fn saved_path() -> std::path::PathBuf {
    data_dir().join("energy_saver_threshold.txt")
}

/// Разбирает сохранённые пороги AC/DC из файла (`"<ac> <dc>"` или устаревшее одиночное `"<dc>"`).
fn parse_saved(text: &str) -> (u32, u32) {
    let mut nums = text.split_whitespace().map(|w| w.parse::<u32>().ok());
    let first = nums.next();
    let second = nums.next();
    match (first, second) {
        (Some(ac), Some(dc)) => (
            ac.filter(|&v| v < ALWAYS).unwrap_or(DEFAULT_AC_THRESHOLD),
            dc.filter(|&v| v < ALWAYS).unwrap_or(DEFAULT_THRESHOLD),
        ),
        (Some(dc), None) => (DEFAULT_AC_THRESHOLD, dc.filter(|&v| v < ALWAYS).unwrap_or(DEFAULT_THRESHOLD)),
        _ => (DEFAULT_AC_THRESHOLD, DEFAULT_THRESHOLD),
    }
}

fn saved() -> (u32, u32) {
    std::fs::read_to_string(saved_path()).ok().map_or((DEFAULT_AC_THRESHOLD, DEFAULT_THRESHOLD), |t| parse_saved(&t))
}

/// Включает или выключает режим. `false` — схему питания изменить не удалось.
pub fn set_energy_saver(on: bool) -> bool {
    unsafe {
        let Some(s) = scheme() else { return false };
        let Some((cur_ac, cur_dc)) = read_thresholds(&s) else { return false };
        if on {
            let (prev_ac, prev_dc) = saved();
            let save_ac = if cur_ac < ALWAYS { cur_ac } else { prev_ac };
            let save_dc = if cur_dc < ALWAYS { cur_dc } else { prev_dc };
            let _ = std::fs::write(saved_path(), format!("{save_ac} {save_dc}"));
            return write_thresholds(&s, ALWAYS, ALWAYS);
        }
        let (prev_ac, prev_dc) = saved();
        if cur_ac == ALWAYS || cur_dc == ALWAYS {
            // Режим включён нами: возвращаем прежние пороги пользователя.
            let next_ac = if cur_ac == ALWAYS { prev_ac } else { cur_ac };
            let next_dc = if cur_dc == ALWAYS { prev_dc } else { cur_dc };
            return write_thresholds(&s, next_ac, next_dc);
        }
        // Режим включила Windows по своему порогу: сбрасываем оба порога в 0, чтобы режим погас,
        // и тут же возвращаем прежние значения.
        let done = write_thresholds(&s, 0, 0);
        write_thresholds(&s, cur_ac, cur_dc) && done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_threshold_is_below_always() {
        let (ac, dc) = saved();
        assert!(ac < ALWAYS && dc < ALWAYS);
    }

    #[test]
    fn parse_saved_handles_pair_legacy_and_invalid() {
        assert_eq!(parse_saved("0 30"), (0, 30));
        assert_eq!(parse_saved("20 40\n"), (20, 40));
        assert_eq!(parse_saved("25"), (DEFAULT_AC_THRESHOLD, 25));
        assert_eq!(parse_saved("100 100"), (DEFAULT_AC_THRESHOLD, DEFAULT_THRESHOLD));
        assert_eq!(parse_saved("bad data"), (DEFAULT_AC_THRESHOLD, DEFAULT_THRESHOLD));
    }

    #[test]
    fn is_saver_active_matches_active_power_source() {
        // От сети: высокий DC-порог не делает экономию энергии активной, пока AC-порог не 100.
        assert!(!is_saver_active(false, true, 0, 100));
        assert!(is_saver_active(false, true, 100, 30));
        // От батареи: решает DC-порог.
        assert!(!is_saver_active(false, false, 100, 30));
        assert!(is_saver_active(false, false, 0, 100));
        // Если Windows уже рапортует SystemStatusFlag = 1 — режим активен независимо от порогов.
        assert!(is_saver_active(true, true, 0, 30));
        assert!(is_saver_active(true, false, 0, 0));
    }

    /// Меняет настоящую схему питания, поэтому запускается вручную: `cargo test energy_saver -- --ignored`.
    #[test]
    #[ignore]
    fn live_toggle_restores_threshold() {
        unsafe {
            let sch = scheme().unwrap();
            let before = read_thresholds(&sch).unwrap();
            assert!(set_energy_saver(true));
            assert_eq!(read_thresholds(&sch), Some((ALWAYS, ALWAYS)));
            std::thread::sleep(std::time::Duration::from_millis(1500));
            assert_eq!(energy_saver(), Some(true));
            assert!(set_energy_saver(false));
            std::thread::sleep(std::time::Duration::from_millis(1500));
            assert_eq!(energy_saver(), Some(false));
            assert_eq!(read_thresholds(&sch), Some(before));
        }
    }
}
