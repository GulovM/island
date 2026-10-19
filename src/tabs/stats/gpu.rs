//! Опрос видеокарты: загрузка 3D-движков, видеопамять (PDH) и модель адаптера (DXGI).

use windows::core::{w, PCWSTR};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
use windows::Win32::System::Performance::*;

/// Счётчики PDH: загрузка 3D-движков GPU и занятая видеопамять.
pub(super) struct GpuQuery {
    q: PDH_HQUERY,
    util: PDH_HCOUNTER,
    mem: PDH_HCOUNTER,
    shared: PDH_HCOUNTER,
}

impl GpuQuery {
    pub(super) unsafe fn new() -> Option<Self> {
        let mut q = PDH_HQUERY::default();
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut q) != 0 {
            return None;
        }
        let (mut util, mut mem, mut shared) = (PDH_HCOUNTER::default(), PDH_HCOUNTER::default(), PDH_HCOUNTER::default());
        let ok_util = PdhAddEnglishCounterW(q, w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"), 0, &mut util) == 0;
        let ok_mem = PdhAddEnglishCounterW(q, w!("\\GPU Adapter Memory(*)\\Dedicated Usage"), 0, &mut mem) == 0;
        let _ = PdhAddEnglishCounterW(q, w!("\\GPU Adapter Memory(*)\\Shared Usage"), 0, &mut shared);
        if !ok_util && !ok_mem {
            let _ = PdhCloseQuery(q);
            return None;
        }
        let _ = PdhCollectQueryData(q);
        Some(GpuQuery { q, util, mem, shared })
    }

    unsafe fn sum(c: PDH_HCOUNTER) -> Option<f64> {
        let (mut size, mut count) = (0u32, 0u32);
        let st = PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, None);
        if st != PDH_MORE_DATA || size == 0 {
            return Some(0.0);
        }
        let n = size as usize / size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>() + 1;
        let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = (0..n).map(|_| PDH_FMT_COUNTERVALUE_ITEM_W::default()).collect();
        if PdhGetFormattedCounterArrayW(c, PDH_FMT_DOUBLE, &mut size, &mut count, Some(buf.as_mut_ptr())) != 0 {
            return None;
        }
        Some(
            buf[..(count as usize).min(n)]
                .iter()
                .filter(|i| i.FmtValue.CStatus == 0)
                .map(|i| i.FmtValue.Anonymous.doubleValue)
                .sum(),
        )
    }

    /// (загрузка 0..1, занятая выделенная и общая видеопамять в байтах).
    pub(super) unsafe fn sample(&self) -> (Option<f32>, Option<f64>, Option<f64>) {
        if PdhCollectQueryData(self.q) != 0 {
            return (None, None, None);
        }
        let util = Self::sum(self.util).map(|v| (v / 100.0).clamp(0.0, 1.0) as f32);
        (util, Self::sum(self.mem), Self::sum(self.shared))
    }
}

impl Drop for GpuQuery {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.q);
        }
    }
}

/// Видеокарта с наибольшей выделенной памятью: (название, выделенная, общая память в байтах).
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
