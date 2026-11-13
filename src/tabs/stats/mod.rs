//! Вкладка Stats: кольца CPU / GPU / RAM, диск, сеть, аптайм, топ приложений по памяти или
//! процессору, замер скорости интернета. Раскладка снята с макета 1:1 (панель 550×210).
//!
//! Дешёвые показатели (CPU, память, диск, сеть) читаются в главном потоке раз в секунду.
//! Тяжёлые (загрузка GPU через PDH, обход процессов) — в отдельном потоке, который живёт только
//! пока вкладка видна.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use windows::core::PCWSTR;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Power::{CallNtPowerInformation, ProcessorInformation, PROCESSOR_POWER_INFORMATION};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

use crate::anim::Spring;
use crate::online::{self, Online};
use crate::ios::SysColor;
use crate::ui::{
    chip_bg, dim, fg, hairline, hue, icon, ink, mix, readable, status_bar_green, status_blue,
    status_cyan, status_orange, status_pink, status_red, status_teal,
    status_violet, track, txt, Align, Canvas, Corner, History, Rect, Ui, ram_segments, speed_scale,
};
use crate::{icons, sys};

use super::grid::{Grid, Item};
use super::layout::{sz, Size, Spec, TabSpec};
use super::{ago, id, STATS};

mod format;
use format::*;

mod hw;
use hw::*;

mod gpu;

mod procs;
use procs::*;

const GB: f32 = 1024.0 * 1024.0 * 1024.0;
/// Значение `GetDriveTypeW` для встроенного накопителя (`DRIVE_FIXED`; константа лежит в фиче
/// `Win32_System_WindowsProgramming`, которую ради неё подключать не стоит).
const DRIVE_FIXED: u32 = 3;

// Номера виджетов в `SPEC`.
const CPU: usize = 0;
const GPU: usize = 1;
const RAM: usize = 2;
const DISK: usize = 3;
const NET: usize = 4;
const UPTIME: usize = 5;
const TOP_APPS: usize = 6;
const SPEEDTEST: usize = 7;

/// Кольцо: мини (кольцо и название), карточка, широкая карточка с подробностями.
static RING: [Size; 3] = [sz(2, 1), sz(2, 2), sz(4, 2)];
/// Диск, сеть, аптайм: узкая и широкая карточки.
static INFO: [Size; 2] = [sz(2, 2), sz(4, 2)];
/// Топ приложений: три строки, пять строк, пять строк на всю ширину.
static APPS_SIZES: [Size; 3] = [sz(4, 2), sz(4, 4), sz(6, 4)];
/// Замер скорости: компактный (2×2) и обычный со спидометром (4×2).
static SPEED_SIZES: [Size; 2] = [sz(2, 2), sz(4, 2)];

/// Сетка вкладки: 10 колонок и 4 ряда крупных ячеек (шаг CELL=56, зазор GAP=8). По умолчанию кольца
/// и карточки занимают по 2×2, топ приложений и замер скорости — по 4×2.
///
/// Арифметика min_rows = 1:
/// Вкладка Stats не имеет выпадающих оверлеев. Минимальная содержательная строка — мини-кольцо
/// (sz(2, 1)). При min_rows = 1 высота содержимого 1 * 56 - 8 = 48 px, что достаточно как для
/// компактного кольца, так и для заглушки «Нет виджетов» с кнопкой «Настроить».
pub static SPEC: TabSpec = TabSpec {
    cols: 10,
    rows: 4,
    min_cols: 2,
    min_rows: 1,
    specs: &[
        Spec { key: "cpu", title: "CPU", sizes: &RING, size: 1, col: 0, row: 0 },
        Spec { key: "gpu", title: "GPU", sizes: &RING, size: 1, col: 2, row: 0 },
        Spec { key: "ram", title: "RAM", sizes: &RING, size: 1, col: 4, row: 0 },
        Spec { key: "disk", title: "Диск", sizes: &INFO, size: 0, col: 0, row: 2 },
        Spec { key: "net", title: "Сеть", sizes: &INFO, size: 0, col: 2, row: 2 },
        Spec { key: "uptime", title: "Аптайм", sizes: &INFO, size: 0, col: 4, row: 2 },
        Spec { key: "apps", title: "Топ приложений", sizes: &APPS_SIZES, size: 0, col: 6, row: 0 },
        Spec { key: "speed", title: "Скорость сети", sizes: &SPEED_SIZES, size: 1, col: 6, row: 2 },
    ],
};

/// Встроенный диск (`DRIVE_FIXED`): буква тома и занятое/полное место в гигабайтах.
#[derive(Clone, Debug)]
pub struct DiskSample {
    pub letter: char,
    pub used_gb: f32,
    pub total_gb: f32,
}

#[derive(Default)]
struct Sample {
    cpu: f32,
    mhz: u32,
    ram_used: f32,
    ram_total: f32,
    disks: Vec<DiskSample>,
    procs: u32,
    uptime_s: u64,
    rx_bps: f32,
    tx_bps: f32,
}

/// Данные тяжёлого потока.
#[derive(Clone, Default)]
pub(super) struct Heavy {
    pub(super) gpu: Option<f32>,
    pub(super) gpu_name: String,
    pub(super) vram_used: f64,
    pub(super) vram_total: f64,
    /// (имя, байты) и (имя, доля CPU 0..1), по убыванию, до 4 строк.
    pub(super) top_ram: Vec<(String, f64)>,
    pub(super) top_cpu: Vec<(String, f64)>,
    /// Иконки программ из топов по названию (`icons::SIZE_PX`² BGRA premultiplied).
    pub(super) app_icons: HashMap<String, Arc<Vec<u8>>>,
}

pub(super) static HEAVY: Mutex<Option<Heavy>> = Mutex::new(None);
/// Поколение тяжёлого потока: поток работает, пока совпадает с текущим.
pub(super) static GEN: AtomicU32 = AtomicU32::new(0);

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
    /// Что показывает компактная карточка диска: `0` — все диски, `1..=disks.len()` — один том.
    disk_view: usize,
    /// Счётчик тиков `poll`: диски опрашиваются раз в 10 с, чтобы не будить спящие HDD.
    disk_tick: u8,
    /// История нагрузки CPU (0..1) за последние 60 с.
    hist_cpu: History<60>,
    /// История нагрузки GPU (0..1) за последние 60 с.
    hist_gpu: History<60>,
    /// История скорости загрузки (байт/с) за последние 60 с.
    hist_rx: History<60>,
    /// История скорости отдачи (байт/с) за последние 60 с.
    hist_tx: History<60>,
    /// Накопленный трафик за сеанс (rx + tx) в байтах.
    session_bytes: u64,
}

impl Default for Stats {
    fn default() -> Self {
        let s = || Spring::unit(0.0, 220.0, 26.0);
        let mut stats = Stats {
            cur: Sample::default(),
            rings: [s(), s(), s()],
            prev_cpu: None,
            prev_net: None,
            cpu_name: String::new(),
            heavy: Heavy::default(),
            // Сохранённый замер скорости виден сразу, до первого опроса вкладки.
            online: online::snapshot(),
            top_by_cpu: false,
            disk_view: 0,
            disk_tick: 0,
            hist_cpu: History::new(),
            hist_gpu: History::new(),
            hist_rx: History::new(),
            hist_tx: History::new(),
            session_bytes: 0,
        };
        // SAFETY: простые вызовы Win32 без указателей на чужую память.
        unsafe { stats.sample_disks() };
        stats
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

pub(super) fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl Stats {
    pub fn load(&mut self, top_by_cpu: bool) {
        self.top_by_cpu = top_by_cpu;
    }

    pub fn show(&mut self) -> bool {
        // Первая выборка «с нуля»: нагрузка и скорость считаются между двумя опросами.
        self.prev_cpu = None;
        self.prev_net = None;
        self.session_bytes = 0;
        if self.cpu_name.is_empty() {
            self.cpu_name = cpu_name();
        }
        let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
        if let Err(e) = std::thread::Builder::new().name("island-stats".into()).spawn(move || unsafe { worker(gen) }) {
            crate::log::error(&format!("не удалось запустить поток статистики: {e}"));
        }
        // SAFETY: простые вызовы Win32 без указателей на чужую память.
        unsafe { self.sample_disks() };
        self.disk_tick = 0;
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
            self.disk_tick = (self.disk_tick + 1) % 10;
            if self.disk_tick == 0 {
                self.sample_disks();
            }
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
        self.hist_cpu.push(self.cur.cpu);
        self.hist_gpu.push(self.heavy.gpu.unwrap_or(0.0));
        self.hist_rx.push(self.cur.rx_bps);
        self.hist_tx.push(self.cur.tx_bps);
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

    /// Перечисляет встроенные тома (`DRIVE_FIXED`): съёмные, сетевые и оптические не учитываются.
    unsafe fn sample_disks(&mut self) {
        let mask = GetLogicalDrives();
        let mut disks = Vec::new();
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(std::iter::once(0)).collect();
            if GetDriveTypeW(PCWSTR(root.as_ptr())) != DRIVE_FIXED {
                continue;
            }
            let (mut free, mut total) = (0u64, 0u64);
            if GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), None, Some(&mut total), Some(&mut free)).is_ok() && total > 0 {
                disks.push(DiskSample { letter, used_gb: total.saturating_sub(free) as f32 / GB, total_gb: total as f32 / GB });
            }
        }
        self.cur.disks = disks;
        if self.disk_view > self.cur.disks.len() {
            self.disk_view = 0;
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
            let d_rx = rx.saturating_sub(prx);
            let d_tx = tx.saturating_sub(ptx);
            self.cur.rx_bps = d_rx as f32 / dt;
            self.cur.tx_bps = d_tx as f32 / dt;
            self.session_bytes += d_rx + d_tx;
        }
        self.prev_net = Some((now, rx, tx));
    }

    /// Рисует вкладку по сетке. `h` — высота панели (с полосой правки).
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, grid: &mut Grid, w: f32, h: f32) {
        let area = grid.area(w, h);
        grid.draw(ui, cv, area, 0, |ui, cv, it| self.paint(ui, cv, it));
    }

    /// Рисует виджет `it.idx` на собственной вкладке.
    pub unsafe fn paint_foreign(&mut self, ui: &mut Ui, cv: &Canvas, it: &Item) {
        self.paint(ui, cv, it);
    }

    unsafe fn paint(&mut self, ui: &mut Ui, cv: &Canvas, it: &Item) {
        match it.idx {
            CPU | GPU | RAM => self.ring_card(cv, it.rect, it.size, it.idx),
            DISK => self.disk_card(ui, cv, it.rect, it.size),
            NET | UPTIME => self.info_card(cv, it.rect, it.size, it.idx),
            TOP_APPS => self.top_apps(ui, cv, it.rect, it.size),
            SPEEDTEST => self.speed_card(ui, cv, it.rect, it.size),
            _ => {}
        }
    }
}

fn clean_cpu_title(raw: &str) -> &str {
    let s = raw.strip_prefix("AMD ").or_else(|| raw.strip_prefix("Intel ")).unwrap_or(raw);
    s.trim()
}

fn clean_gpu_title(raw: &str) -> String {
    let s = short_gpu(raw);
    let s = s.strip_prefix("NVIDIA GeForce ").or_else(|| s.strip_prefix("NVIDIA ")).unwrap_or(&s);
    let s = s.strip_suffix(" GPU").unwrap_or(s);
    s.to_string()
}

/// Крупная открытая дуга с числом процентов, знаком «%» и названием внутри (карточки 2×2 и 4×2).
unsafe fn big_dial(cv: &Canvas, cx: f32, cy: f32, v: f32, col: D2D1_COLOR_F, pct_num: &str, name: &str) {
    let f = &cv.g.fonts;
    cv.arc_gauge(cx, cy, 34.0, 5.5, v, col);

    let dw = cv.digits_w(pct_num, 28.0, 600);
    let pct_w = cv.text_w("%", &f.small);
    let num_x = cx - (dw + 2.0 + pct_w) / 2.0;
    cv.digits(pct_num, num_x, dw + 2.0, cy - 4.0, 28.0, 600, Align::Leading, txt());
    cv.line("%", &f.small, num_x + dw + 2.0, pct_w + 4.0, cy - 12.0, Align::Leading, dim());
    cv.line(name, &f.small_c, cx - 30.0, 60.0, cy + 13.0, Align::Center, dim());
}

impl Stats {
    /// Кольцо загрузки в трёх видах (`size` — индекс в `RING`):
    /// 2×1: мини (дуга слева, подпись и крупное число справа);
    /// 2×2: крупная дуга по центру, число и название внутри, подробность внизу;
    /// 4×2: слева дуга как в 2×2, справа колонка с названием, спарклайном/шкалой и подробностями.
    unsafe fn ring_card(&self, cv: &Canvas, r: Rect, size: usize, i: usize) {
        let f = &cv.g.fonts;
        let c = &self.cur;
        let h = &self.heavy;
        let (name, glow_col, col) = match i {
            CPU => ("CPU", status_blue(), readable(status_blue(), 45.0)),
            GPU => ("GPU", status_violet(), readable(status_violet(), 45.0)),
            _ => ("RAM", status_orange(), readable(status_orange(), 45.0)),
        };
        cv.card(r);
        cv.glow(r, 14.0, glow_col, Corner::TopRight);

        let v = self.rings[i].value;
        let pct_num = format!("{:.0}", (v * 100.0).round());

        let sub = match i {
            CPU => {
                if c.mhz > 0 {
                    format!("{:.1} {}", c.mhz as f32 / 1000.0, crate::i18n::pick("ГГц", "GHz"))
                } else {
                    "—".to_string()
                }
            }
            GPU => {
                // «занято / всего», как в диспетчере задач (у встроенной графики «всего» — общая память).
                let used = h.vram_used / GB as f64;
                if h.vram_total > 0.0 {
                    format!("{used:.1} / {:.1} {}", h.vram_total / GB as f64, crate::i18n::pick("ГБ", "GB"))
                } else {
                    format!("{} {used:.1} {}", crate::i18n::pick("VRAM", "VRAM"), crate::i18n::pick("ГБ", "GB"))
                }
            }
            _ => {
                format!("{:.1} / {:.0} {}", c.ram_used / GB, c.ram_total / GB, crate::i18n::pick("ГБ", "GB"))
            }
        };

        match size {
            0 => {
                // 2x1 (104x48)
                let cx = r.x + 22.0;
                let cy = r.cy();
                cv.arc_gauge(cx, cy, 15.0, 4.0, v, col);

                let tx = r.x + 46.0;
                let tw = (r.right() - tx - 8.0).max(0.0);
                cv.line(name, &f.caps, tx, tw, r.y + 16.0, Align::Leading, dim());

                let dw = cv.digits_w(&pct_num, 20.0, 600);
                cv.digits(&pct_num, tx, dw + 2.0, r.y + 34.0, 20.0, 600, Align::Leading, txt());
                let rem_w = (tw - dw - 2.0).max(0.0);
                cv.line("%", &f.small, tx + dw + 2.0, rem_w, r.y + 35.0, Align::Leading, dim());
            }
            1 => {
                // 2x2 (104x104)
                big_dial(cv, r.cx(), r.y + 46.0, v, col, &pct_num, name);

                cv.line(&sub, &f.small, r.x + 4.0, r.w - 8.0, r.y + 90.0, Align::Center, dim());
            }
            _ => {
                // 4x2 (216x104)
                big_dial(cv, r.x + 52.0, r.y + 46.0, v, col, &pct_num, name);

                cv.line(&sub, &f.small, r.x + 4.0, 96.0, r.y + 90.0, Align::Center, dim());

                let x0 = r.x + 124.0;
                let col_w = (r.right() - P - x0).max(10.0);

                match i {
                    CPU => {
                        let title = cv.wrap_clip(clean_cpu_title(&self.cpu_name), &f.small_b, col_w, 1);
                        cv.line(&title, &f.small_b, x0, col_w, r.y + 20.0, Align::Leading, txt());

                        let spark_r = Rect::new(x0, r.y + 34.0, col_w, 28.0);
                        cv.spark(spark_r, self.hist_cpu.iter(), (0.0, 1.0), col, true);
                        cv.stroke_line((spark_r.x, spark_r.bottom()), (spark_r.right(), spark_r.bottom()), 1.0, hairline());

                        let note = format!("{:.1} {} · {} {}", c.mhz as f32 / 1000.0, crate::i18n::pick("ГГц", "GHz"), c.procs, crate::i18n::pick("проц.", "cores"));
                        cv.line(&note, &f.small, x0, col_w, r.y + 80.0, Align::Leading, dim());
                    }
                    GPU => {
                        let raw_name = if h.gpu_name.is_empty() { "GPU" } else { &h.gpu_name };
                        let clean = clean_gpu_title(raw_name);
                        let title = cv.wrap_clip(&clean, &f.small_b, col_w, 1);
                        cv.line(&title, &f.small_b, x0, col_w, r.y + 20.0, Align::Leading, txt());

                        let spark_r = Rect::new(x0, r.y + 34.0, col_w, 28.0);
                        cv.spark(spark_r, self.hist_gpu.iter(), (0.0, 1.0), col, true);
                        cv.stroke_line((spark_r.x, spark_r.bottom()), (spark_r.right(), spark_r.bottom()), 1.0, hairline());

                        let tot_gb = h.vram_total / GB as f64;
                        let tot_s = if tot_gb.fract() == 0.0 { format!("{:.0}", tot_gb) } else { format!("{:.1}", tot_gb) };
                        let note = format!("{} {:.1} / {} {}", crate::i18n::pick("VRAM", "VRAM"), h.vram_used / GB as f64, tot_s, crate::i18n::pick("ГБ", "GB"));
                        cv.line(&note, &f.small, x0, col_w, r.y + 80.0, Align::Leading, dim());
                    }
                    _ => {
                        cv.line(crate::i18n::pick("Память", "Memory"), &f.small_b, x0, col_w, r.y + 20.0, Align::Leading, txt());

                        let used_s = format!("{:.1}", c.ram_used / GB);
                        let udw = cv.digits_w(&used_s, 22.0, 600);
                        cv.digits(&used_s, x0, udw + 2.0, r.y + 44.0, 22.0, 600, Align::Leading, txt());
                        let tot_line = format!(" / {:.0} {}", c.ram_total / GB, crate::i18n::pick("ГБ", "GB"));
                        let rem_w = (col_w - udw - 2.0).max(0.0);
                        cv.line(&tot_line, &f.small, x0 + udw + 2.0, rem_w, r.y + 44.0, Align::Leading, dim());

                        let (count, filled, step) = ram_segments(c.ram_total / GB, c.ram_used / GB);
                        let seg_r = Rect::new(x0, r.y + 58.0, col_w, 14.0);
                        cv.segments(seg_r, count, filled, col);

                        let step_s = if step.fract() == 0.0 { format!("{:.0}", step) } else { format!("{:.1}", step) };
                        let note = format!("{} = {} {}", crate::i18n::pick("1 деление", "1 bar"), step_s, crate::i18n::pick("ГБ", "GB"));
                        cv.line(&note, &f.small, x0, col_w, r.y + 82.0, Align::Leading, dim());
                    }
                }
            }
        }
    }

    /// Диск: суммарный объём всех встроенных томов. В узкой карточке (`2×2`) клик по ней
    /// переключает «все» → «C:» → «D:» …; в широкой (`4×2`) показываются накопители списком.
    unsafe fn disk_card(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, size: usize) {
        let f = &cv.g.fonts;
        let wide = size > 0;
        let n = self.cur.disks.len();
        cv.card(r);
        cv.glow(r, 14.0, status_teal(), Corner::TopRight);

        if !wide && n > 1 {
            let (_, clicked) = ui.click_area(id(STATS, 25), r);
            if clicked {
                self.disk_view = (self.disk_view + 1) % (n + 1);
            }
        }
        let view = if wide || n < 2 { 0 } else { self.disk_view.min(n) };
        let disks = &self.cur.disks;
        let (used, total) = match view {
            0 => (disks.iter().map(|d| d.used_gb).sum::<f32>(), disks.iter().map(|d| d.total_gb).sum::<f32>()),
            k => (disks[k - 1].used_gb, disks[k - 1].total_gb),
        };
        let one = |d: Option<&DiskSample>| match d {
            Some(d) => format!("{} {}:", crate::i18n::pick("ДИСК", "DISK"), d.letter),
            None => crate::i18n::pick("ДИСК", "DISK").to_string(),
        };
        let title = match (view, n) {
            (0, 0 | 1) => one(disks.first()),
            (0, _) => crate::i18n::pick("ДИСКИ • ВСЕ", "DISKS • ALL").to_string(),
            (k, _) => one(disks.get(k - 1)),
        };

        let frac = if total > 0.0 { used / total } else { 0.0 };
        let bar_col = if frac >= 0.88 { readable(status_red(), 45.0) } else { readable(status_teal(), 45.0) };

        if !wide {
            // 2x2
            cv.line(&title, &f.caps, r.x + P, r.w - 2.0 * P, r.y + 17.0, Align::Leading, dim());

            let free_gb = (total - used).max(0.0);
            let free_s = format!("{:.0}", free_gb);
            let fw = cv.digits_w(&free_s, 30.0, 600);
            cv.digits(&free_s, r.x + P, fw + 4.0, r.y + 44.0, 30.0, 600, Align::Leading, txt());
            let rem_w = (r.w - 2.0 * P - fw - 2.0).max(0.0);
            cv.line(crate::i18n::pick(" ГБ", " GB"), &f.small, r.x + P + fw + 2.0, rem_w, r.y + 44.0, Align::Leading, txt());

            let sub_s = format!("{} {:.0}", crate::i18n::pick("свободно из", "free of"), total);
            cv.line(&sub_s, &f.small, r.x + P, r.w - 2.0 * P, r.y + 65.0, Align::Leading, dim());

            cv.progress(Rect::new(r.x + P, r.bottom() - P - 7.0, r.w - 2.0 * P, 7.0), frac, bar_col);
        } else {
            // 4x2
            cv.line(crate::i18n::pick("НАКОПИТЕЛИ", "STORAGE"), &f.caps, r.x + P, r.w - 2.0 * P, r.y + 17.0, Align::Leading, dim());
            // До 4 строк (при 4 шаг плотнее); остальные накопители не теряются молча: в шапке «+N».
            let rows = disks.len().min(4);
            let hidden = disks.len() - rows;
            let head = if hidden > 0 {
                format!("{} · +{hidden}", crate::i18n::pick("свободно", "free"))
            } else {
                crate::i18n::pick("свободно", "free").to_string()
            };
            cv.line(&head, &f.small, r.x + P, r.w - 2.0 * P, r.y + 17.0, Align::Trailing, dim());

            let (top, step) = if rows > 3 { (34.0, 16.0) } else { (38.0, 22.0) };
            for (k, d) in disks.iter().take(rows).enumerate() {
                let y = r.y + top + k as f32 * step;
                let d_frac = if d.total_gb > 0.0 { d.used_gb / d.total_gb } else { 0.0 };
                let d_col = if d_frac >= 0.88 { readable(status_red(), 45.0) } else { readable(status_teal(), 45.0) };

                let let_s = format!("{}:", d.letter);
                cv.line(&let_s, &f.small_b, r.x + P, 22.0, y + 2.0, Align::Leading, txt());

                let free = (d.total_gb - d.used_gb).max(0.0);
                let val_s = format!("{:.0} / {:.0} {}", free, d.total_gb, crate::i18n::pick("ГБ", "GB"));
                let vw = cv.text_w(&val_s, &f.small);
                cv.line(&val_s, &f.small, r.right() - P - vw, vw + 2.0, y + 2.0, Align::Leading, txt());

                let bx = r.x + P + 24.0;
                let bw = (r.right() - P - vw - 8.0 - bx).max(10.0);
                cv.progress(Rect::new(bx, y - 1.0, bw, 6.0), d_frac, d_col);
            }
        }
    }

    /// Сеть и аптайм. Узкая (2×2) — главное значение и спарклайн/капсулы;
    /// широкая (4×2) — слева главное значение, справа зеркальный график / недельная шкала.
    unsafe fn info_card(&self, cv: &Canvas, r: Rect, size: usize, i: usize) {
        let f = &cv.g.fonts;
        let c = &self.cur;
        let wide = size > 0;
        cv.card(r);
        match i {
            NET => {
                cv.glow(r, 14.0, status_cyan(), Corner::TopRight);
                cv.line(crate::i18n::pick("СЕТЬ", "NETWORK"), &f.caps, r.x + P, r.w - 2.0 * P, r.y + 17.0, Align::Leading, dim());

                let down_col = readable(status_cyan(), 45.0);
                let up_col = readable(status_pink(), 45.0);
                let (rx_num, rx_unit) = fmt_rate_parts(c.rx_bps);
                let (tx_num, tx_unit) = fmt_rate_parts(c.tx_bps);

                // Строка загрузки (вниз)
                cv.arrow(r.x + P + 4.0, r.y + 36.0, 9.0, false, down_col);
                let aw = 12.0;
                let nw = cv.digits_w(&rx_num, 20.0, 600);
                cv.digits(&rx_num, r.x + P + aw, nw + 2.0, r.y + 36.0, 20.0, 600, Align::Leading, txt());
                let rem_rx = (100.0 - aw - nw - 2.0).max(0.0);
                cv.line(rx_unit, &f.small, r.x + P + aw + nw + 2.0, rem_rx, r.y + 36.0, Align::Leading, dim());

                // Строка отдачи (вверх)
                cv.arrow(r.x + P + 4.0, r.y + 54.0, 9.0, true, up_col);
                let aw_u = 12.0;
                let nw_u = cv.digits_w(&tx_num, 16.0, 600);
                cv.digits(&tx_num, r.x + P + aw_u, nw_u + 2.0, r.y + 54.0, 16.0, 600, Align::Leading, txt());
                let rem_tx = (100.0 - aw_u - nw_u - 2.0).max(0.0);
                cv.line(tx_unit, &f.small, r.x + P + aw_u + nw_u + 2.0, rem_tx, r.y + 54.0, Align::Leading, dim());

                if !wide {
                    // 2x2: спарклайн загрузки внизу карточки
                    let sp_r = Rect::new(r.x + 14.0, r.bottom() - 10.0 - 28.0, r.w - 28.0, 28.0);
                    let max_rx = self.hist_rx.iter().fold(1024.0f32, |m, v| m.max(v));
                    cv.spark(sp_r, self.hist_rx.iter(), (0.0, max_rx), down_col, true);
                } else {
                    // 4x2: слева «Всего за сеанс», справа зеркальный график
                    let sess_s = format!("{}: {}", crate::i18n::pick("Всего за сеанс", "Session total"), fmt_bytes(self.session_bytes as f64));
                    cv.line(&sess_s, &f.small, r.x + P, 100.0, r.y + 86.0, Align::Leading, dim());

                    let gx = r.x + 114.0;
                    let gw = (r.right() - 12.0 - gx).max(10.0);
                    let gy = r.y + 14.0;
                    let gh = 66.0;
                    let mid_y = gy + gh / 2.0;

                    let max_rx = self.hist_rx.iter().fold(1024.0f32, |m, v| m.max(v));
                    let max_tx = self.hist_tx.iter().fold(1024.0f32, |m, v| m.max(v));
                    cv.stroke_line((gx, mid_y), (gx + gw, mid_y), 1.0, hairline());

                    let r_rx = Rect::new(gx, gy, gw, gh / 2.0);
                    cv.spark(r_rx, self.hist_rx.iter(), (0.0, max_rx), down_col, true);

                    let r_tx = Rect::new(gx, mid_y, gw, gh / 2.0);
                    cv.spark(r_tx, self.hist_tx.iter(), (max_tx, 0.0), up_col, true);

                    cv.line(crate::i18n::pick("60 с", "60 s"), &f.small, gx, gw, r.y + 88.0, Align::Trailing, dim());
                }
            }
            _ => {
                // UPTIME
                cv.glow(r, 14.0, cv.accent, Corner::TopRight);
                cv.line(crate::i18n::pick("АПТАЙМ", "UPTIME"), &f.caps, r.x + P, r.w - 2.0 * P, r.y + 17.0, Align::Leading, dim());

                let days = c.uptime_s / 86400;
                let hours = (c.uptime_s % 86400) / 3600;
                let mins = (c.uptime_s % 3600) / 60;
                let (d_s, d_unit, h_s, h_unit) = if days > 0 {
                    (days.to_string(), crate::i18n::pick("д ", "d "), hours.to_string(), crate::i18n::pick("ч", "h"))
                } else {
                    (hours.to_string(), crate::i18n::pick("ч ", "h "), mins.to_string(), crate::i18n::pick("м", "m"))
                };
                let w1 = cv.digits_w(&d_s, 28.0, 600);
                let u1 = cv.text_w(d_unit, &f.small_b);
                let w2 = cv.digits_w(&h_s, 28.0, 600);
                let mut cur_x = r.x + P;
                cv.digits(&d_s, cur_x, w1 + 2.0, r.y + 44.0, 28.0, 600, Align::Leading, txt());
                cur_x += w1 + 2.0;
                cv.line(d_unit, &f.small_b, cur_x, u1 + 4.0, r.y + 44.0, Align::Leading, dim());
                cur_x += u1 + 4.0;
                cv.digits(&h_s, cur_x, w2 + 2.0, r.y + 44.0, 28.0, 600, Align::Leading, txt());
                cur_x += w2 + 2.0;
                cv.line(h_unit, &f.small_b, cur_x, 20.0, r.y + 44.0, Align::Leading, dim());

                let since = fmt_since(online::now_local_minutes(), c.uptime_s);
                cv.line(&since, &f.small, r.x + P, 100.0, r.y + 65.0, Align::Leading, dim());

                let (fractions, today_idx) = uptime_week(online::now_local_minutes(), c.uptime_s);

                if !wide {
                    // 2x2: 7 капсул
                    let cw = (r.w - 2.0 * P - 6.0 * 3.0) / 7.0;
                    let cy_cap = r.bottom() - P - 6.0;
                    for (k, frac) in fractions.iter().enumerate() {
                        let cap_x = r.x + P + k as f32 * (cw + 3.0);
                        let cap_r = Rect::new(cap_x, cy_cap, cw, 6.0);
                        cv.rr(cap_r, 3.0, track());
                        if *frac > 0.0 {
                            cv.rr(Rect::new(cap_x, cy_cap, (cw * frac).max(3.0), 6.0), 3.0, cv.accent);
                        }
                    }
                } else {
                    // 4x2: число процессов слева, 7 столбиков справа
                    let procs_s = format!("{} {}", c.procs, crate::i18n::pick("процессов", "processes"));
                    cv.line(&procs_s, &f.small_b, r.x + P, 100.0, r.y + 86.0, Align::Leading, txt());

                    let x0 = r.x + 124.0;
                    let w0 = (r.right() - P - x0).max(10.0);
                    let bar_w = 8.0f32;
                    let gap = (w0 - 7.0 * bar_w) / 6.0;
                    let bar_h = 56.0f32;
                    let top_y = r.y + 18.0;

                    let weekdays = crate::i18n::weekdays();
                    for (k, &frac) in fractions.iter().enumerate() {
                        let bx = x0 + k as f32 * (bar_w + gap);
                        let bg_r = Rect::new(bx, top_y, bar_w, bar_h);
                        cv.rr(bg_r, 4.0, track());
                        if frac > 0.0 {
                            let fh = (bar_h * frac).max(bar_w);
                            let fill_r = Rect::new(bx, top_y + bar_h - fh, bar_w, fh);
                            cv.rr(fill_r, 4.0, cv.accent);
                        }
                        let day_char = weekdays.get(k).and_then(|s| s.chars().next()).unwrap_or(' ');
                        let day_str = day_char.to_string();
                        let day_col = if k == today_idx { txt() } else { dim() };
                        cv.line(&day_str, &f.small_c, bx - 2.0, bar_w + 4.0, top_y + bar_h + 12.0, Align::Center, day_col);
                    }
                }
            }
        }
    }

    unsafe fn top_apps(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, _size: usize) {
        let f = &cv.g.fonts;
        cv.card(r);
        header(cv, r, icon::APPS, "Топ приложений", hue(SysColor::Purple, cv.accent));
        // Переключатель метрики — сегмент из двух капсул справа в шапке.
        let metrics = [("RAM", false), ("CPU", true)];
        let widths: Vec<f32> = metrics.iter().map(|&(name, _)| cv.chip_w(name)).collect();
        let total_w = crate::ui::chips_total_width(&widths, 2.0);
        let mut cx = r.right() - P - total_w;
        for (k, &(name, cpu)) in metrics.iter().enumerate() {
            let seg = Rect::new(cx, r.y + 10.0, widths[k], 14.0);
            if ui.chip(cv, id(STATS, 10 + 2 * k as u32), seg, name, self.top_by_cpu == cpu) && self.top_by_cpu != cpu {
                self.top_by_cpu = cpu;
                crate::state::set_stats_top_by_cpu(cpu);
            }
            cx += widths[k] + 2.0;
        }
        let list = if self.top_by_cpu { &self.heavy.top_cpu } else { &self.heavy.top_ram };
        let max = list.first().map(|x| x.1).unwrap_or(0.0).max(1e-9);
        let rows = (((r.h - 34.0) / 22.0).floor() as usize).clamp(1, 5);
        // Полосы — цветом выбранной метрики, как её кольцо: CPU синий, RAM зелёный.
        let bar_col = if self.top_by_cpu { crate::ui::hue(crate::ios::SysColor::Blue, cv.accent) } else { status_bar_green() };
        for (k, (name, v)) in list.iter().take(rows).enumerate() {
            let y = r.y + 32.0 + k as f32 * 22.0;
            let tile = Rect::new(r.x + P, y, 19.0, 19.0);
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
            cv.rr(Rect::new(bar.x, bar.y, (bar.w * (*v / max) as f32).max(3.0), 3.0), 1.5, bar_col);
        }
        if list.is_empty() {
            cv.text("Собираю данные…", Rect::new(r.x + 12.0, r.y + 34.0, r.w - 24.0, 12.0), &f.small, dim());
        }
    }

    /// Замер скорости. `size` — индекс в `SPEED_SIZES`: 2×2, 4×2.
    unsafe fn speed_card(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, size: usize) {
        let f = &cv.g.fonts;
        cv.card(r);
        cv.glow(r, 14.0, status_bar_green(), Corner::BottomRight);

        let sp = &self.online.speed;
        let salate = readable(status_bar_green(), 45.0);
        let d_str = if sp.at > 0 { format!("{:.0}", sp.down) } else { "—".into() };
        let u_str = if sp.at > 0 { format!("{:.0}", sp.up) } else { "—".into() };
        let unit = crate::i18n::pick("Мбит/с", "Mbps");

        let btn_bg = if sp.running { chip_bg() } else { status_bar_green() };
        let btn_fg = if sp.running { dim() } else { ink() };
        let glyph_icon = if sp.running { icon::REFRESH } else { icon::PLAY_SOLID };
        let go;

        match size {
            0 => {
                // 2x2
                cv.line(crate::i18n::pick("СКОРОСТЬ", "SPEED"), &f.caps, r.x + P, r.w - 2.0 * P - 24.0, r.y + 17.0, Align::Leading, dim());

                let btn = Rect::new(r.right() - P - 18.0, r.y + 8.0, 18.0, 18.0);
                let (_, clicked) = ui.click_area(id(STATS, 11), btn);
                let hv = ui.hover_of(id(STATS, 11));
                cv.circle(btn.cx(), btn.cy(), 9.0, mix(btn_bg, fg(), 0.08 * hv));
                cv.glyph(glyph_icon, btn, false, btn_fg);
                go = clicked;

                cv.arrow(r.x + P + 4.0, r.y + 44.0, 9.0, false, salate);
                let aw = 12.0;
                let dw = cv.digits_w(&d_str, 30.0, 600);
                cv.digits(&d_str, r.x + P + aw, dw + 4.0, r.y + 44.0, 30.0, 600, Align::Leading, txt());
                if sp.failed && !sp.running {
                    // Замер не удался: вместо единицы подсказка «Повторить» (кнопка «▶» справа вверху).
                    cv.line(crate::i18n::pick("Повторить", "Retry"), &f.small_b, r.x + P, r.w - 2.0 * P, r.y + 65.0, Align::Leading, readable(status_red(), 45.0));
                } else {
                    cv.line(unit, &f.small, r.x + P, r.w - 2.0 * P, r.y + 65.0, Align::Leading, dim());
                }

                let up_y = r.bottom() - P - 4.0;
                cv.arrow(r.x + P + 4.0, up_y, 9.0, true, txt());
                let up_text = format!("{} {}", u_str, unit);
                cv.line(&up_text, &f.small_b, r.x + P + 12.0, r.w - 2.0 * P - 12.0, up_y, Align::Leading, txt());
            }
            _ => {
                // 4x2 по макету Figma: слева крупный полукруглый спидометр (диаметр 88, толщина 7), справа колонка
                // «СКОРОСТЬ» + кнопка, загрузка и отдача числами 15 px, внизу — когда и где был замер.
                let (cx, cy, rad, th) = (r.x + 57.0, r.y + 77.0, 40.5, 7.0);
                let scale = speed_scale(sp.down);
                let frac = if sp.at > 0 && scale > 0.0 { sp.down / scale } else { 0.0 };
                cv.stroke_arc(cx, cy, rad, -90.0, 90.0, th, track());
                if frac >= 0.005 {
                    cv.stroke_arc(cx, cy, rad, -90.0, -90.0 + 180.0 * frac.clamp(0.0, 1.0), th, salate);
                }
                // Число внутри дуги (в макете 24 px); длинное число уменьшает кегль.
                let max_w = 2.0 * (rad - th) - 6.0;
                let mut font_sz = 24.0;
                while font_sz > 14.0 && cv.digits_w(&d_str, font_sz, 600) > max_w {
                    font_sz -= 2.0;
                }
                let dw = cv.digits_w(&d_str, font_sz, 600);
                cv.digits(&d_str, cx - dw / 2.0, dw + 2.0, r.y + 63.0, font_sz, 600, Align::Leading, txt());
                cv.line(unit, &f.small, cx - 25.0, 50.0, r.y + 81.0, Align::Center, dim());

                let x0 = r.x + 119.0;
                cv.line(crate::i18n::pick("СКОРОСТЬ", "SPEED"), &f.caps, x0, r.right() - P - 24.0 - x0, r.y + 17.0, Align::Leading, dim());

                let btn = Rect::new(r.x + 181.0, r.y + 9.0, 22.0, 22.0);
                let (_, clicked) = ui.click_area(id(STATS, 11), btn);
                let hv = ui.hover_of(id(STATS, 11));
                cv.circle(btn.cx(), btn.cy(), 11.0, mix(btn_bg, fg(), 0.08 * hv));
                cv.glyph(glyph_icon, btn.inset(5.5), false, btn_fg);
                go = clicked;

                cv.arrow(x0 + 5.5, r.y + 44.5, 10.0, false, salate);
                let dw_down = cv.digits_w(&d_str, 15.0, 600);
                cv.digits(&d_str, x0 + 14.0, dw_down + 2.0, r.y + 44.0, 15.0, 600, Align::Leading, txt());

                cv.arrow(x0 + 5.5, r.y + 64.5, 10.0, true, txt());
                let uw = cv.digits_w(&u_str, 15.0, 600);
                cv.digits(&u_str, x0 + 14.0, uw + 2.0, r.y + 64.0, 15.0, 600, Align::Leading, txt());

                let failed = sp.failed && !sp.running;
                let info = if sp.running {
                    crate::i18n::pick("идёт замер…", "testing…").to_string()
                } else if failed {
                    // Замер не удался: подсказка вместо времени последнего замера (кнопка «▶» справа вверху).
                    crate::i18n::pick("Повторить", "Retry").to_string()
                } else if sp.at > 0 {
                    let when = ago(sys::now_secs(), sp.at);
                    if sp.place.is_empty() { when } else { format!("{when} • {}", sp.place) }
                } else {
                    crate::i18n::pick("ещё не замеряли", "not measured yet").to_string()
                };
                let info_col = if failed { readable(status_red(), 45.0) } else { dim() };
                cv.line(&info, &f.small, x0, r.right() - P - x0, r.y + 86.0, Align::Leading, info_col);
            }
        }

        if go && !sp.running {
            online::run_speedtest();
        }
    }
}

#[cfg(feature = "gallery")]
impl Stats {
    /// Демо-данные галереи: типичная нагрузка ноутбука, топ программ, последний замер скорости.
    pub fn demo(&mut self) {
        let c = &mut self.cur;
        c.cpu = 0.34;
        c.mhz = 3400;
        c.ram_used = 10.9 * GB;
        c.ram_total = 16.0 * GB;
        c.disks = vec![
            DiskSample { letter: 'C', used_gb: 312.0, total_gb: 476.0 },
            DiskSample { letter: 'D', used_gb: 379.0, total_gb: 931.0 },
            DiskSample { letter: 'E', used_gb: 210.0, total_gb: 238.0 },
        ];
        c.procs = 312;
        c.uptime_s = 3 * 86400 + 14 * 3600;
        c.rx_bps = 12.4 * 1024.0 * 1024.0;
        c.tx_bps = 1.1 * 1024.0 * 1024.0;
        self.session_bytes = (2.3 * 1024.0 * 1024.0 * 1024.0) as u64;
        self.cpu_name = "AMD Ryzen 7 5800H".into();
        self.disk_view = 1;
        let gb = f64::from(GB);
        let h = &mut self.heavy;
        h.gpu = Some(0.18);
        h.gpu_name = "NVIDIA GeForce RTX 3060 Laptop GPU".into();
        h.vram_used = 2.1 * gb;
        h.vram_total = 6.0 * gb;
        h.top_ram = vec![
            ("Google Chrome".into(), 2.8 * gb),
            ("Visual Studio Code".into(), 1.4 * gb),
            ("Telegram Desktop".into(), 0.62 * gb),
            ("Проводник".into(), 0.31 * gb),
            ("Spotify".into(), 0.24 * gb),
        ];
        for (ring, v) in self.rings.iter_mut().zip([0.34, 0.18, 10.9 / 16.0]) {
            ring.target = v;
            ring.value = v;
        }
        self.online.speed = online::Speed {
            down: 487.0,
            up: 92.0,
            place: "Москва".into(),
            at: sys::now_secs() - 300,
            running: false,
            failed: false,
        };

        // Заполнение истории детерминированными гладкими кривыми:
        self.hist_cpu = History::new();
        self.hist_gpu = History::new();
        self.hist_rx = History::new();
        self.hist_tx = History::new();
        for i in 0..60 {
            let t = i as f32 / 59.0;
            let cpu_v = 0.34 + 0.15 * (t * std::f32::consts::TAU * 2.0).sin() + 0.05 * (t * std::f32::consts::TAU * 4.5).cos() + 0.02 * (t * std::f32::consts::TAU * 8.0).sin();
            self.hist_cpu.push(cpu_v.clamp(0.05, 0.90));

            let gpu_v = 0.18 + 0.09 * (t * std::f32::consts::TAU * 1.5 + 1.0).sin() + 0.03 * (t * std::f32::consts::TAU * 4.0).cos();
            self.hist_gpu.push(gpu_v.clamp(0.02, 0.90));

            let rx_v = (12.4e6 * (0.7 + 0.3 * (t * std::f32::consts::TAU * 1.8).sin() + 0.1 * (t * std::f32::consts::TAU * 4.5).cos())).max(0.0);
            self.hist_rx.push(rx_v);

            let tx_v = (1.1e6 * (0.7 + 0.3 * (t * std::f32::consts::TAU * 2.2 + 0.5).sin() + 0.1 * (t * std::f32::consts::TAU * 4.0).cos())).max(0.0);
            self.hist_tx.push(tx_v);
        }
    }

    /// Один виджет вне сетки: номер в `SPEC`, индекс размера и прямоугольник.
    pub unsafe fn gallery(&mut self, ui: &mut Ui, cv: &Canvas, idx: usize, size: usize, rect: Rect) {
        self.paint(ui, cv, &Item { idx, size, rect });
    }
}

/// Внутренний отступ карточек вкладки (и карточки батареи в «Управлении» — тот же стиль).
pub(super) const P: f32 = 12.0;

/// Шапка карточки: значок цвета метрики и название прописными (как подписи виджетов iOS).
pub(super) unsafe fn header(cv: &Canvas, r: Rect, glyph: &str, title: &str, tint: D2D1_COLOR_F) {
    let cy = r.y + 17.0;
    cv.symbol(glyph, Rect::new(r.x + P, cy - 6.0, 12.0, 12.0), 11.0, tint);
    let title = crate::i18n::tr(title).to_uppercase();
    cv.line(&title, &cv.g.fonts.caps, r.x + P + 16.0, r.w - 2.0 * P - 16.0, cy, Align::Leading, dim());
}

