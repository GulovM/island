//! Опрос видеокарты на macOS: загрузка (`IOAccelerator` / `ioreg`) и модель адаптера (Metal/DXGI).

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};

pub(super) struct GpuQuery;

impl GpuQuery {
    pub(super) unsafe fn new() -> Option<Self> {
        #[cfg(target_os = "macos")]
        {
            Some(GpuQuery)
        }
        #[cfg(not(target_os = "macos"))]
        None
    }

    /// (загрузка 0..1, выделенная и общая видеопамять в байтах).
    pub(super) unsafe fn sample(&self) -> (Option<f32>, Option<f64>, Option<f64>) {
        #[cfg(target_os = "macos")]
        {
            let Ok(out) = std::process::Command::new("/usr/sbin/ioreg")
                .args(["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"])
                .output()
            else {
                return (None, None, None);
            };
            let s = String::from_utf8_lossy(&out.stdout);
            let mut util: Option<f32> = None;
            let mut mem: Option<f64> = None;
            if let Some(i) = s.find("\"Device Utilization %\"=") {
                let rest = &s[i + "\"Device Utilization %\"=".len()..];
                if let Some(num) = rest.split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse::<f32>().ok()) {
                    util = Some((num / 100.0).clamp(0.0, 1.0));
                }
            }
            if let Some(i) = s.find("\"In use system memory\"=") {
                let rest = &s[i + "\"In use system memory\"=".len()..];
                if let Some(num) = rest.split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse::<f64>().ok()) {
                    mem = Some(num);
                }
            }
            (util, mem, mem)
        }
        #[cfg(not(target_os = "macos"))]
        (None, None, None)
    }
}

pub(super) unsafe fn best_adapter() -> Option<(String, f64, f64)> {
    let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
    let mut best: Option<(String, f64, f64)> = None;
    let mut i = 0;
    while let Ok(a) = f.EnumAdapters1(i) {
        i += 1;
        let Ok(d) = a.GetDesc1() else { continue };
        if d.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
            continue;
        }
        let mem = d.DedicatedVideoMemory as f64;
        if best.as_ref().is_none_or(|b| mem > b.1) {
            let n = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
            best = Some((String::from_utf16_lossy(&d.Description[..n]), mem, d.SharedSystemMemory as f64));
        }
    }
    best
}
