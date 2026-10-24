//! Режим энергосбережения (Windows 11 — «Экономия энергии», Windows 10 — «Экономия заряда»).
//!
//! Публичного вызова «включить сейчас» нет. Состояние читается из `SYSTEM_POWER_STATUS`, а
//! включается режим порогом автовключения в активной схеме питания (`ESBATTTHRESHOLD`, при работе от
//! батареи): порог 100 % включает режим сразу. Прежний порог запоминается в файле и возвращается
//! при выключении, поэтому автовключение пользователя не теряется.

use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerGetActiveScheme, PowerReadDCValueIndex, PowerSetActiveScheme, PowerWriteDCValueIndex,
    SYSTEM_POWER_STATUS,
};

use super::data_dir;

const SUB_ENERGYSAVER: GUID = GUID::from_u128(0xDE830923_A562_41AF_A086_E3A2C6BAD2DA);
const ES_BATT_THRESHOLD: GUID = GUID::from_u128(0xE69653CA_CF7F_4F05_AA73_CB833FA90AD4);

/// Порог, при котором режим включён всегда, пока работа идёт от батареи.
const ALWAYS: u32 = 100;
/// Порог Windows по умолчанию, если запомненного нет.
const DEFAULT_THRESHOLD: u32 = 30;

fn ok(code: u32) -> bool {
    code == 0
}

/// Режим включён (или взведён: порог 100 % включит его, как только ПК перейдёт на батарею);
/// `None` — батареи нет (настольный ПК), переключать нечего.
pub fn energy_saver() -> Option<bool> {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s).ok()? };
    if s.BatteryFlag == 255 || s.BatteryFlag & 128 != 0 {
        return None;
    }
    let armed = unsafe { scheme().and_then(|g| threshold(&g)) } == Some(ALWAYS);
    Some(s.SystemStatusFlag & 1 != 0 || armed)
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

unsafe fn threshold(s: &GUID) -> Option<u32> {
    let mut v = 0u32;
    ok(PowerReadDCValueIndex(None, Some(s), Some(&SUB_ENERGYSAVER), Some(&ES_BATT_THRESHOLD), &mut v)).then_some(v)
}

unsafe fn write_threshold(s: &GUID, v: u32) -> bool {
    ok(PowerWriteDCValueIndex(None, s, Some(&SUB_ENERGYSAVER), Some(&ES_BATT_THRESHOLD), v)) && ok(PowerSetActiveScheme(None, Some(s)).0)
}

fn saved_path() -> std::path::PathBuf {
    data_dir().join("energy_saver_threshold.txt")
}

fn saved() -> u32 {
    std::fs::read_to_string(saved_path()).ok().and_then(|t| t.trim().parse().ok()).filter(|&v| v < ALWAYS).unwrap_or(DEFAULT_THRESHOLD)
}

/// Включает или выключает режим. `false` — схему питания изменить не удалось.
pub fn set_energy_saver(on: bool) -> bool {
    unsafe {
        let Some(s) = scheme() else { return false };
        let Some(cur) = threshold(&s) else { return false };
        if on {
            if cur != ALWAYS {
                let _ = std::fs::write(saved_path(), cur.to_string());
            }
            return write_threshold(&s, ALWAYS);
        }
        if cur == ALWAYS {
            // Режим включён нами: возвращаем прежний порог пользователя.
            return write_threshold(&s, saved());
        }
        // Режим включила Windows по своему порогу: сбрасываем порог в 0, чтобы режим погас, и тут же
        // возвращаем прежнее значение.
        let done = write_threshold(&s, 0);
        write_threshold(&s, cur) && done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_threshold_is_below_always() {
        assert!(saved() < ALWAYS);
    }

    /// Меняет настоящую схему питания, поэтому запускается вручную: `cargo test energy_saver -- --ignored`.
    #[test]
    #[ignore]
    fn live_toggle_restores_threshold() {
        unsafe {
            let sch = scheme().unwrap();
            let before = threshold(&sch).unwrap();
            assert!(set_energy_saver(true));
            assert_eq!(threshold(&sch), Some(ALWAYS));
            std::thread::sleep(std::time::Duration::from_millis(1500));
            assert_eq!(energy_saver(), Some(true));
            assert!(set_energy_saver(false));
            std::thread::sleep(std::time::Duration::from_millis(1500));
            assert_eq!(energy_saver(), Some(false));
            assert_eq!(threshold(&sch), Some(before));
        }
    }
}
