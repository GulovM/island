//! Вкладка Stats: кольца CPU / GPU / RAM, диск, сеть, аптайм, топ приложений по памяти или
//! процессору, замер скорости интернета. Раскладка снята с макета 1:1 (панель 550×210).
//!
//! Дешёвые показатели (CPU, память, диск, сеть) читаются в главном потоке раз в секунду.
//! Тяжёлые (загрузка GPU через PDH, обход процессов) — в отдельном потоке, который живёт только
//! пока вкладка видна.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, FILETIME};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Performance::*;
use windows::Win32::System::Power::{CallNtPowerInformation, ProcessorInformation, PROCESSOR_POWER_INFORMATION};
use windows::Win32::System::ProcessStatus::{EnumProcesses, GetPerformanceInfo, GetProcessMemoryInfo, PERFORMANCE_INFORMATION, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows::Win32::System::SystemInformation::{GetSystemInfo, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO};
use windows::Win32::System::Threading::{
    GetProcessTimes, GetSystemTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::anim::Spring;
use crate::online::{self, Online};
use crate::ui::{icon, Canvas, Rect, Ui, BAR_GREEN, chip_bg, dim, GREEN, ink, txt, track, VIOLET};
use crate::{bg, icons, sys};

use super::{ago, id, PAD, STATS, TOP};

mod format;
use format::*;

const GB: f32 = 1024.0 * 1024.0 * 1024.0;

#[derive(Default)]
struct Sample {
    cpu: f32,
    mhz: u32,
    ram_used: f32,
    ram_total: f32,
    disk_used: f32,
    disk_total: f32,
    procs: u32,
    uptime_s: u64,
    rx_bps: f32,
    tx_bps: f32,
}

/// Данные тяжёлого потока.
#[derive(Clone, Default)]
struct Heavy {
    gpu: Option<f32>,
    gpu_name: String,
    vram_used: f64,
    vram_total: f64,
    /// (имя, байты) и (имя, доля CPU 0..1), по убыванию, до 4 строк.
    top_ram: Vec<(String, f64)>,
    top_cpu: Vec<(String, f64)>,
    /// Иконки программ из топов по названию (`icons::SIZE_PX`² BGRA premultiplied).
    app_icons: HashMap<String, Arc<Vec<u8>>>,
}

static HEAVY: Mutex<Option<Heavy>> = Mutex::new(None);
/// Поколение тяжёлого потока: поток работает, пока совпадает с текущим.
static GEN: AtomicU32 = AtomicU32::new(0);

pub struct Stats {
    cur: Sample,
    rings: [Spring; 3],
    prev_cpu: Option<(u64, u64, u64)>,
    prev_net: Option<(Instant, u64, u64)>,
    cpu_name: String,
    heavy: Heavy,
    online: Online,
    /// Топ по процессору вместо памяти.
    top_by_cpu: bool,
}

impl Default for Stats {
    fn default() -> Self {
        let s = || Spring::unit(0.0, 220.0, 26.0);
        Stats {
            cur: Sample::default(),
            rings: [s(), s(), s()],
            prev_cpu: None,
            prev_net: None,
            cpu_name: String::new(),
            heavy: Heavy::default(),
            online: Online::default(),
            top_by_cpu: false,
        }
    }
}

/// Новая цель кольца. Мёртвая зона 2%: дрожание на доли процента не запускает анимацию (и кадры).
/// Малые сдвиги — обычное дело раз в секунду — доезжают быстро (~12 кадров вместо ~25),
/// большие идут мягкой пружиной.
fn aim(ring: &mut Spring, v: f32) {
    let d = (ring.target - v).abs();
    if d < 0.02 {
        return;
    }
    if d < 0.12 {
        ring.tune(900.0, 60.0);
    } else {
        ring.tune(220.0, 26.0);
    }
    ring.target = v;
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl Stats {
    pub fn show(&mut self) -> bool {
        // Первая выборка «с нуля»: нагрузка и скорость считаются между двумя опросами.
        self.prev_cpu = None;
        self.prev_net = None;
        if self.cpu_name.is_empty() {
            self.cpu_name = cpu_name();
        }
        let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
        if let Err(e) = std::thread::Builder::new().name("island-stats".into()).spawn(move || unsafe { worker(gen) }) {
            crate::log::error(&format!("не удалось запустить поток статистики: {e}"));
        }
        self.poll();
        self.sync();
        true
    }

    pub fn hide(&mut self) {
        GEN.fetch_add(1, Ordering::SeqCst);
    }

    pub fn sync(&mut self) -> bool {
        if let Some(h) = HEAVY.lock().ok().and_then(|h| h.clone()) {
            self.heavy = h;
            aim(&mut self.rings[1], self.heavy.gpu.unwrap_or(0.0));
        }
        self.online = online::snapshot();
        true
    }

    pub fn step(&mut self, dt: f32) {
        for r in &mut self.rings {
            r.step(dt);
        }
    }

    pub fn animating(&self) -> bool {
        self.rings.iter().any(|r| !r.settled())
    }

    pub fn poll(&mut self) -> bool {
        unsafe {
            self.sample_cpu();
            self.sample_memory();
            self.sample_disk();
            self.sample_net();
            let mut pi = PERFORMANCE_INFORMATION { cb: size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
            if GetPerformanceInfo(&mut pi, pi.cb).is_ok() {
                self.cur.procs = pi.ProcessCount;
            }
            self.cur.uptime_s = GetTickCount64() / 1000;
        }
        let c = &self.cur;
        let ram = if c.ram_total > 0.0 { c.ram_used / c.ram_total } else { 0.0 };
        for (i, v) in [(0usize, c.cpu), (2, ram)] {
            aim(&mut self.rings[i], v);
        }
        true
    }

    unsafe fn sample_cpu(&mut self) {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        if GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).is_ok() {
            let now = (ft(idle), ft(kernel), ft(user));
            if let Some(p) = self.prev_cpu {
                // Время ядра в Windows уже включает простой.
                let total = (now.1 - p.1) + (now.2 - p.2);
                let busy = total.saturating_sub(now.0 - p.0);
                if total > 0 {
                    self.cur.cpu = (busy as f32 / total as f32).clamp(0.0, 1.0);
                }
            }
            self.prev_cpu = Some(now);
        }
        let n = cpu_count();
        let mut buf = vec![PROCESSOR_POWER_INFORMATION::default(); n];
        let size = (n * size_of::<PROCESSOR_POWER_INFORMATION>()) as u32;
        if CallNtPowerInformation(ProcessorInformation, None, 0, Some(buf.as_mut_ptr() as *mut c_void), size).is_ok() {
            let sum: u64 = buf.iter().map(|p| p.CurrentMhz as u64).sum();
            self.cur.mhz = (sum / n as u64) as u32;
        }
    }

    unsafe fn sample_memory(&mut self) {
        let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if GlobalMemoryStatusEx(&mut m).is_ok() {
            self.cur.ram_total = m.ullTotalPhys as f32;
            self.cur.ram_used = (m.ullTotalPhys - m.ullAvailPhys) as f32;
        }
    }

    unsafe fn sample_disk(&mut self) {
        let (mut free, mut total) = (0u64, 0u64);
        if GetDiskFreeSpaceExW(PCWSTR(crate::sys::system_drive_root().as_ptr()), None, Some(&mut total), Some(&mut free)).is_ok() {
            self.cur.disk_total = total as f32;
            self.cur.disk_used = total.saturating_sub(free) as f32;
        }
    }

    unsafe fn sample_net(&mut self) {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return;
        }
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let (mut rx, mut tx) = (0u64, 0u64);
        for r in rows {
            // Только физические адаптеры, чтобы VPN и виртуальные не удваивали трафик.
            let hardware = r.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            if hardware && r.OperStatus.0 == 1 {
                rx += r.InOctets;
                tx += r.OutOctets;
            }
        }
        FreeMibTable(table as *const c_void);
        let now = Instant::now();
        if let Some((t, prx, ptx)) = self.prev_net {
            let dt = (now - t).as_secs_f32().max(0.05);
            self.cur.rx_bps = rx.saturating_sub(prx) as f32 / dt;
            self.cur.tx_bps = tx.saturating_sub(ptx) as f32 / dt;
        }
        self.prev_net = Some((now, rx, tx));
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32) {
        let f = &cv.g.fonts;
        let c = &self.cur;
        let gb = |v: f32| format!("{:.1}", v / GB);
        let (cw, gap) = (120.0, 10.0);

        // Кольца.
        let mut cpu_chips = Vec::new();
        if c.mhz > 0 {
            cpu_chips.push(format!("{:.1} {}", c.mhz as f32 / 1000.0, crate::i18n::pick("ГГц", "GHz")));
        }
        cpu_chips.push(format!("{} {}", c.procs, crate::i18n::pick("проц.", "cores")));
        let h = &self.heavy;
        let gpu_chip = if h.vram_total > 0.0 {
            vec![format!("{:.1} / {:.1} {}", h.vram_used / GB as f64, h.vram_total / GB as f64, crate::i18n::pick("ГБ", "GB"))]
        } else {
            Vec::new()
        };
        let gpu_name = if h.gpu_name.is_empty() { "—".to_string() } else { short_gpu(&h.gpu_name) };
        let cards: [(&str, String, Vec<String>, D2D1_COLOR_F); 3] = [
            ("CPU", self.cpu_name.clone(), cpu_chips, cv.accent),
            ("GPU", gpu_name, gpu_chip, VIOLET),
            (
                "RAM",
                format!("{} / {} {}", gb(c.ram_used), gb(c.ram_total), crate::i18n::pick("ГБ", "GB")),
                vec![format!("{} {}", gb((c.ram_total - c.ram_used).max(0.0)), crate::i18n::pick("ГБ своб.", "GB free"))],
                GREEN,
            ),
        ];
        for (i, (name, sub, chips, col)) in cards.iter().enumerate() {
            let r = Rect::new(PAD + i as f32 * (cw + gap), TOP, cw, 150.0);
            cv.card(r);
            let (rx, ry) = (r.cx(), r.y + 46.0);
            cv.ring(rx, ry, 31.0, 5.0, self.rings[i].value, *col);
            cv.text(&format!("{}%", (self.rings[i].value * 100.0).round() as i32), Rect::new(rx - 26.0, ry - 8.0, 52.0, 16.0), &f.body_c, txt());
            cv.text(name, Rect::new(r.x, r.y + 88.0, r.w, 14.0), &f.body_c, txt());
            cv.text(sub, Rect::new(r.x + 6.0, r.y + 106.0, r.w - 12.0, 12.0), &f.small_c, dim());
            chips_row(cv, r.cx(), r.y + 126.0, chips);
        }

        // Нижний ряд: диск, сеть, аптайм.
        let y = TOP + 160.0;
        let infos = [
            ("ДИСК C:", format!("{:.0} / {:.0} {}", c.disk_used / GB, c.disk_total / GB, crate::i18n::pick("ГБ", "GB"))),
            ("СЕТЬ", format!("↓{}  ↑{}", fmt_rate(c.rx_bps), fmt_rate(c.tx_bps))),
            ("В РАБОТЕ", fmt_uptime(c.uptime_s)),
        ];
        for (i, (label, value)) in infos.iter().enumerate() {
            let r = Rect::new(PAD + i as f32 * (cw + gap), y, cw, 70.0);
            cv.card(r);
            cv.text(label, Rect::new(r.x + 12.0, r.y + 15.0, r.w - 20.0, 11.0), &f.caps, dim());
            cv.text(value, Rect::new(r.x + 12.0, r.y + 36.0, r.w - 18.0, 16.0), &f.body, txt());
        }

        let col_x = PAD + 3.0 * (cw + gap);
        let col_w = (w - PAD - col_x).max(160.0);
        self.top_apps(ui, cv, Rect::new(col_x, TOP, col_w, 142.0));
        self.speed_card(ui, cv, Rect::new(col_x, TOP + 152.0, col_w, 78.0));
    }

    unsafe fn top_apps(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect) {
        let f = &cv.g.fonts;
        cv.card(r);
        let mode = if self.top_by_cpu { "CPU" } else { "RAM" };
        cv.text(&format!("{} • {mode}", crate::i18n::pick("Топ приложений", "Top apps")), Rect::new(r.x + 12.0, r.y + 9.0, 140.0, 12.0), &f.small, dim());
        let sw = Rect::new(r.right() - 46.0, r.y + 8.0, 36.0, 13.0);
        let (_, clicked) = ui.click_area(id(STATS, 10), sw);
        let other = if self.top_by_cpu { "RAM ⇄" } else { "CPU ⇄" };
        cv.text(other, sw, &f.body_r, cv.accent);
        if clicked {
            self.top_by_cpu = !self.top_by_cpu;
        }
        let list = if self.top_by_cpu { &self.heavy.top_cpu } else { &self.heavy.top_ram };
        let max = list.first().map(|x| x.1).unwrap_or(0.0).max(1e-9);
        for (k, (name, v)) in list.iter().take(5).enumerate() {
            let y = r.y + 28.0 + k as f32 * 22.0;
            let tile = Rect::new(r.x + 12.0, y, 19.0, 19.0);
            let bmp = self.heavy.app_icons.get(name).and_then(|px| {
                let key = {
                    use std::hash::{Hash, Hasher};
                    let mut hs = std::collections::hash_map::DefaultHasher::new();
                    name.hash(&mut hs);
                    (0x42u64 << 56) | (hs.finish() >> 8)
                };
                ui.cache.get(&cv.g.dc, key, icons::SIZE_PX, icons::SIZE_PX, || px.to_vec())
            });
            if let Some(bmp) = bmp {
                cv.bitmap(&bmp, tile, 1.0);
            } else {
                cv.rr(tile, 3.0, chip_bg());
                let letter: String = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                cv.text(&letter, tile, &f.caps_c, txt());
            }
            let value = if self.top_by_cpu { format!("{:.0}%", v * 100.0) } else { fmt_bytes(*v) };
            cv.text(name, Rect::new(r.x + 38.0, y - 2.0, r.w - 110.0, 13.0), &f.small_b, txt());
            cv.text(&value, Rect::new(r.right() - 68.0, y - 2.0, 56.0, 13.0), &f.small_r, dim());
            let bar = Rect::new(r.x + 38.0, y + 14.0, r.w - 50.0, 3.0);
            cv.rr(bar, 1.5, track());
            cv.rr(Rect::new(bar.x, bar.y, (bar.w * (*v / max) as f32).max(3.0), 3.0), 1.5, BAR_GREEN);
        }
        if list.is_empty() {
            cv.text("Собираю данные…", Rect::new(r.x + 12.0, r.y + 34.0, r.w - 24.0, 12.0), &f.small, dim());
        }
    }

    unsafe fn speed_card(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect) {
        let f = &cv.g.fonts;
        cv.card(r);
        let sp = &self.online.speed;
        cv.text("Скорость сети", Rect::new(r.x + 12.0, r.y + 8.0, 90.0, 12.0), &f.small, dim());
        let info = if sp.at > 0 {
            let when = ago(sys::now_secs(), sp.at);
            if sp.place.is_empty() { when } else { format!("{when} • {}", sp.place) }
        } else {
            "ещё не замеряли".into()
        };
        cv.text(&info, Rect::new(r.right() - 112.0, r.y + 8.0, 100.0, 12.0), &f.small_r, dim());
        let num = |v: f32| if sp.at > 0 { format!("{:.0}", v) } else { "—".into() };
        let (d, u) = (num(sp.down), num(sp.up));
        let digits = |t: &str| if sp.at > 0 { t.chars().count() as f32 * 8.6 + 3.0 } else { 22.0 };
        let dw = digits(&d);
        cv.text(&d, Rect::new(r.x + 12.0, r.y + 21.0, dw + 4.0, 22.0), &f.num, txt());
        cv.text("Мбит/с ↓", Rect::new(r.x + 12.0 + dw, r.y + 28.0, 50.0, 12.0), &f.small, dim());
        let ux = r.x + 12.0 + r.w / 2.0 - 8.0;
        let uw = digits(&u);
        cv.text(&u, Rect::new(ux, r.y + 21.0, uw + 4.0, 22.0), &f.num, txt());
        cv.text("Мбит/с ↑", Rect::new(ux + uw, r.y + 28.0, 50.0, 12.0), &f.small, dim());
        let label = if sp.running {
            "Идёт замер…"
        } else if sp.failed {
            "▶ Повторить замер"
        } else {
            "▶ Замерить скорость"
        };
        let bg = if sp.running { chip_bg() } else { cv.accent };
        let fg = if sp.running { dim() } else { ink() };
        if ui.solid_button(cv, id(STATS, 11), Rect::new(r.x + 12.0, r.y + 49.0, r.w - 24.0, 21.0), label, bg, fg, 8.0) && !sp.running {
            online::run_speedtest();
        }
        let _ = icon::PLAY;
    }
}

/// Ряд маленьких плашек по центру (частота, «N ГБ своб.»).
unsafe fn chips_row(cv: &Canvas, cx: f32, y: f32, labels: &[String]) {
    let widths: Vec<f32> = labels.iter().map(|l| cv.text_w(l, &cv.g.fonts.small_c) + 12.0).collect();
    let total = widths.iter().sum::<f32>() + 5.0 * labels.len().saturating_sub(1) as f32;
    let mut x = cx - total / 2.0;
    for (l, w) in labels.iter().zip(widths) {
        let r = Rect::new(x, y, w, 15.0);
        cv.rr(r, 6.0, chip_bg());
        cv.text(l, r, &cv.g.fonts.small_c, txt());
        x += w + 5.0;
    }
}

fn cpu_count() -> usize {
    let mut si = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut si) };
    si.dwNumberOfProcessors.clamp(1, 256) as usize
}

/// Модель процессора из реестра, без «(R)», «(TM)», частоты и встроенной графики.
fn cpu_name() -> String {
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

fn short_gpu(raw: &str) -> String {
    raw.replace("(R)", "").replace("(TM)", "").split_whitespace().collect::<Vec<_>>().join(" ")
}


// ---------- Тяжёлый поток ----------

/// Счётчики PDH: загрузка 3D-движков GPU и занятая видеопамять.
struct GpuQuery {
    q: PDH_HQUERY,
    util: PDH_HCOUNTER,
    mem: PDH_HCOUNTER,
    shared: PDH_HCOUNTER,
}

impl GpuQuery {
    unsafe fn new() -> Option<Self> {
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
    unsafe fn sample(&self) -> (Option<f32>, Option<f64>, Option<f64>) {
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
unsafe fn best_adapter() -> Option<(String, f64, f64)> {
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

#[derive(Default)]
struct Procs {
    top_ram: Vec<(String, f64)>,
    top_cpu: Vec<(String, f64)>,
    app_icons: HashMap<String, Arc<Vec<u8>>>,
}

/// Обход процессов: память и процессорное время, сгруппированные по имени программы.
#[derive(Default)]
struct ProcSampler {
    prev: HashMap<u32, u64>,
    at: Option<Instant>,
    /// Название программы по пути к exe (описание читается из ресурсов файла — дорого).
    names: HashMap<String, String>,
    /// Процесс → (время создания, название): путь запрашивается один раз, а не на каждом проходе.
    /// Время создания отличает новый процесс, занявший тот же pid.
    known: HashMap<u32, (u64, String, String)>,
    /// Иконка по пути к exe (None — не удалось получить, повторно не просим).
    icons: HashMap<String, Option<Arc<Vec<u8>>>>,
}

impl ProcSampler {
    unsafe fn sample(&mut self, ncpu: f64) -> Procs {
        let mut pids = vec![0u32; 4096];
        let mut needed = 0u32;
        if EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut needed).is_err() {
            return Procs::default();
        }
        let now = Instant::now();
        let dt = self.at.map(|t| (now - t).as_secs_f64()).unwrap_or(0.0);
        let (mut ram, mut cpu): (HashMap<String, f64>, HashMap<String, f64>) = (HashMap::new(), HashMap::new());
        let mut next = HashMap::new();
        let mut paths: HashMap<String, String> = HashMap::new();
        let mut known = HashMap::with_capacity(self.known.len());
        let own = std::process::id();
        for &pid in &pids[..(needed / 4) as usize] {
            if pid == 0 || pid == own {
                continue;
            }
            let Ok(hp) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
            let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            let (created, t) = if GetProcessTimes(hp, &mut c, &mut e, &mut k, &mut u).is_ok() { (ft(c), ft(k) + ft(u)) } else { (0, 0) };
            let (name, path) = match self.known.remove(&pid) {
                Some((cr, name, path)) if cr == created => (name, path),
                _ => {
                    let mut buf = [0u16; 520];
                    let mut len = buf.len() as u32;
                    if QueryFullProcessImageNameW(hp, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_err() {
                        let _ = CloseHandle(hp);
                        continue;
                    }
                    let path = String::from_utf16_lossy(&buf[..len as usize]);
                    let name = self.names.entry(path.clone()).or_insert_with(|| describe(&path)).clone();
                    (name, path)
                }
            };
            let mut pmc = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
            let ws = if GetProcessMemoryInfo(hp, &mut pmc, pmc.cb).is_ok() { pmc.WorkingSetSize as f64 } else { 0.0 };
            let _ = CloseHandle(hp);
            paths.entry(name.clone()).or_insert_with(|| path.clone());
            *ram.entry(name.clone()).or_default() += ws;
            if let (Some(&p), true) = (self.prev.get(&pid), dt > 0.0) {
                *cpu.entry(name.clone()).or_default() += t.saturating_sub(p) as f64 / (dt * 1e7 * ncpu);
            }
            next.insert(pid, t);
            known.insert(pid, (created, name, path));
        }
        self.prev = next;
        self.known = known;
        self.at = Some(now);
        let top = |m: HashMap<String, f64>| {
            let mut v: Vec<(String, f64)> = m.into_iter().filter(|x| x.1 > 0.0).collect();
            v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            v.truncate(5);
            v
        };
        let (top_ram, top_cpu) = (top(ram), top(cpu));
        let mut app_icons = HashMap::new();
        for (name, _) in top_ram.iter().chain(&top_cpu) {
            let Some(path) = paths.get(name) else { continue };
            let icon = self.icons.entry(path.clone()).or_insert_with(|| icons::path_icon(path).map(Arc::new));
            if let Some(px) = icon {
                app_icons.insert(name.clone(), px.clone());
            }
        }
        Procs { top_ram, top_cpu, app_icons }
    }
}

/// Название программы из ресурса версии (FileDescription), иначе имя файла.
unsafe fn describe(path: &str) -> String {
    let stem = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_string();
    let wp = sys::wz(path);
    let size = GetFileVersionInfoSizeW(PCWSTR(wp.as_ptr()), None);
    if size == 0 {
        return stem;
    }
    let mut data = vec![0u8; size as usize];
    if GetFileVersionInfoW(PCWSTR(wp.as_ptr()), None, size, data.as_mut_ptr() as *mut c_void).is_err() {
        return stem;
    }
    let (mut p, mut len) = (std::ptr::null_mut::<c_void>(), 0u32);
    if !VerQueryValueW(data.as_ptr() as *const c_void, w!("\\VarFileInfo\\Translation"), &mut p, &mut len).as_bool() || len < 4 {
        return stem;
    }
    let lang = *(p as *const u16);
    let cp = *(p as *const u16).add(1);
    let key = sys::wz(&format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\FileDescription"));
    if VerQueryValueW(data.as_ptr() as *const c_void, PCWSTR(key.as_ptr()), &mut p, &mut len).as_bool() && len > 1 {
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize - 1));
        let s = s.trim().to_string();
        if !s.is_empty() && s.chars().count() <= 40 {
            return s;
        }
    }
    stem
}

unsafe fn worker(gen: u32) {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let gpu = GpuQuery::new();
    let adapter = best_adapter();
    let mut procs = ProcSampler::default();
    let ncpu = cpu_count() as f64;
    let mut tick = 0u32;
    while GEN.load(Ordering::SeqCst) == gen {
        let mut h = HEAVY.lock().ok().and_then(|h| h.clone()).unwrap_or_default();
        // У встроенной графики своей памяти почти нет (128 МБ) — показываем общую, как диспетчер задач.
        let integrated = adapter.as_ref().is_some_and(|a| a.1 < 512.0 * 1024.0 * 1024.0);
        // Счётчик «GPU Engine(*)» перебирает сотни экземпляров — раз в 2 секунды, не в ту же секунду,
        // что и обход процессов.
        if let Some(q) = gpu.as_ref().filter(|_| tick == 0 || tick % 2 == 1) {
            let (util, mem, shared) = q.sample();
            h.gpu = util;
            if let Some(m) = if integrated { shared } else { mem } {
                h.vram_used = m;
            }
        }
        if let Some((name, dedicated, shared)) = &adapter {
            h.gpu_name = name.clone();
            h.vram_total = if integrated { *shared } else { *dedicated };
        }
        // Процессы обходим раз в 2 секунды; первая выборка только запоминает времена.
        if tick % 2 == 0 {
            let p = procs.sample(ncpu);
            h.top_ram = p.top_ram;
            if tick > 0 {
                h.top_cpu = p.top_cpu;
            }
            h.app_icons = p.app_icons;
        }
        if let Ok(mut g) = HEAVY.lock() {
            *g = Some(h);
        }
        bg::notify();
        tick += 1;
        // PDH и обход процессов при первых опросах подтягивают десятки МБ страниц, которые
        // дальше не нужны: отдаём их системе один раз, когда оба опроса уже сделаны.
        if tick == 2 {
            sys::trim_working_set();
        }
        for _ in 0..10 {
            if GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
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
