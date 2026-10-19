//! Определение оборудования: число ядер, название CPU и GPU.

use std::ffi::c_void;

use windows::core::w;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

pub(super) fn cpu_count() -> usize {
    let mut si = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut si) };
    si.dwNumberOfProcessors.clamp(1, 256) as usize
}

/// Модель процессора из реестра, без «(R)», «(TM)», частоты и встроенной графики.
pub(super) fn cpu_name() -> String {
    let mut buf = [0u16; 128];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0"),
            w!("ProcessorNameString"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS {
        return String::new();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    clean_cpu(&String::from_utf16_lossy(&buf[..n]))
}

fn clean_cpu(raw: &str) -> String {
    let mut s = raw.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    for cut in [" with ", " w/ ", " @ ", " CPU"] {
        if let Some(i) = s.find(cut) {
            s.truncate(i);
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn short_gpu(raw: &str) -> String {
    raw.replace("(R)", "").replace("(TM)", "").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_names() {
        assert_eq!(clean_cpu("AMD Ryzen 7 7840HS w/ Radeon 780M Graphics     "), "AMD Ryzen 7 7840HS");
        assert_eq!(clean_cpu("13th Gen Intel(R) Core(TM) i7-13700H"), "13th Gen Intel Core i7-13700H");
        assert_eq!(clean_cpu("Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz"), "Intel Core i5-8250U");
    }
}
