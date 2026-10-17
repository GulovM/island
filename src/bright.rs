//! Яркость экрана: встроенная панель ноутбука (IOCTL к `\\.\LCD`) или внешний монитор по DDC/CI.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, SetMonitorBrightness, PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::{CloseHandle, POINT};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::core::w;

const IOCTL_QUERY: u32 = 0x0023_0498;
const IOCTL_SET: u32 = 0x0023_049C;

static OK: AtomicBool = AtomicBool::new(false);

/// Есть ли у устройства управляемая яркость (проверено при запуске фонового потока).
pub fn available() -> bool {
    OK.load(Ordering::Relaxed)
}

/// Определяет, поддерживается ли регулировка; вызывать один раз из фонового потока.
pub fn probe() {
    OK.store(get().is_some(), Ordering::Relaxed);
}

/// Текущая яркость, 0..=1.
pub fn get() -> Option<f32> {
    unsafe { lcd_get().or_else(|| ddc_get()) }
}

/// Устанавливает яркость, 0..=1.
pub fn set(v: f32) {
    let pct = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
    unsafe {
        if !lcd_set(pct) {
            ddc_set(pct);
        }
    }
}

unsafe fn on_ac() -> bool {
    let mut st = SYSTEM_POWER_STATUS::default();
    GetSystemPowerStatus(&mut st).is_ok() && st.ACLineStatus == 1
}

unsafe fn open_lcd() -> Option<windows::Win32::Foundation::HANDLE> {
    CreateFileW(
        w!(r"\\.\LCD"),
        (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        None,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        None,
    )
    .ok()
}

unsafe fn lcd_get() -> Option<f32> {
    let h = open_lcd()?;
    let mut out = [0u8; 3];
    let mut got = 0u32;
    let ok = DeviceIoControl(
        h,
        IOCTL_QUERY,
        None,
        0,
        Some(out.as_mut_ptr().cast()),
        3,
        Some(&mut got),
        None,
    )
    .is_ok();
    let _ = CloseHandle(h);
    if !ok || got < 3 {
        return None;
    }
    let pct = if on_ac() { out[1] } else { out[2] };
    Some(f32::from(pct.min(100)) / 100.0)
}

unsafe fn lcd_set(pct: u8) -> bool {
    let Some(h) = open_lcd() else { return false };
    let policy = if on_ac() { 1u8 } else { 2u8 };
    let inp = [policy, pct, pct];
    let mut got = 0u32;
    let ok = DeviceIoControl(
        h,
        IOCTL_SET,
        Some(inp.as_ptr().cast()),
        3,
        None,
        0,
        Some(&mut got),
        None,
    )
    .is_ok();
    let _ = CloseHandle(h);
    ok
}

/// Физические мониторы основного дисплея; освобождать через `DestroyPhysicalMonitors`.
unsafe fn physical() -> Vec<PHYSICAL_MONITOR> {
    let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let mut n = 0u32;
    if GetNumberOfPhysicalMonitorsFromHMONITOR(mon, &mut n).is_err() || n == 0 {
        return Vec::new();
    }
    let mut v = vec![PHYSICAL_MONITOR::default(); n as usize];
    if GetPhysicalMonitorsFromHMONITOR(mon, &mut v).is_err() {
        return Vec::new();
    }
    v
}

unsafe fn ddc_get() -> Option<f32> {
    let mons = physical();
    let mut res = None;
    for m in &mons {
        let (mut lo, mut cur, mut hi) = (0u32, 0u32, 0u32);
        let handle = m.hPhysicalMonitor;
        if GetMonitorBrightness(handle, &mut lo, &mut cur, &mut hi) != 0 && hi > lo {
            res = Some((cur.saturating_sub(lo)) as f32 / (hi - lo) as f32);
            break;
        }
    }
    if !mons.is_empty() {
        let _ = DestroyPhysicalMonitors(&mons);
    }
    res
}

unsafe fn ddc_set(pct: u8) {
    let mons = physical();
    for m in &mons {
        let (mut lo, mut cur, mut hi) = (0u32, 0u32, 0u32);
        let handle = m.hPhysicalMonitor;
        if GetMonitorBrightness(handle, &mut lo, &mut cur, &mut hi) != 0 && hi > lo {
            let val = lo + (f32::from(pct) / 100.0 * (hi - lo) as f32).round() as u32;
            SetMonitorBrightness(handle, val);
        }
    }
    if !mons.is_empty() {
        let _ = DestroyPhysicalMonitors(&mons);
    }
}
