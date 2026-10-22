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
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Power::{CallNtPowerInformation, ProcessorInformation, PROCESSOR_POWER_INFORMATION};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

use crate::anim::Spring;
use crate::online::{self, Online};
use crate::ios::SysColor;
use crate::ui::{chip_bg, dim, hue, icon, ink, mix, status_bar_green, status_green, status_violet, track, txt, Align, Canvas, Rect, Ui};
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
static RING: [Size; 3] = [sz(8, 4), sz(8, 10), sz(16, 6)];
static INFO: [Size; 2] = [sz(8, 5), sz(16, 5)];
/// Топ приложений: три, четыре или пять строк.
static APPS_SIZES: [Size; 3] = [sz(16, 6), sz(16, 10), sz(24, 10)];
/// Замер скорости: компактный (кнопка «▶» в строке с числами) и обычный.
static SPEED_SIZES: [Size; 3] = [sz(8, 5), sz(16, 5), sz(24, 5)];

/// Сетка вкладки: 40 колонок по 16 px (шаг CELL=16, зазор GAP=8), 15 рядов. Кольца и карточки занимают
/// по 8 колонок, топ приложений и замер скорости — по 16.
///
/// Арифметика min_rows = 5:
/// Вкладка Stats не имеет выпадающих оверлеев. Минимальная содержательная строка — инфо-карточки
/// (sz(8, 5)) или кольца (sz(8, 4) / sz(8, 6)). При min_rows = 5 высота содержимого:
/// 5 * 16 - 8 = 72 px, что достаточно как для компактного ряда карточек, так и для
/// размещения заглушки «Нет виджетов» с кнопкой «Настроить».
pub static SPEC: TabSpec = TabSpec {
    cols: 40,
    rows: 15,
    min_cols: 8,
    min_rows: 5,
    specs: &[
        Spec { key: "cpu", title: "CPU", sizes: &RING, size: 1, col: 0, row: 0 },
        Spec { key: "gpu", title: "GPU", sizes: &RING, size: 1, col: 8, row: 0 },
        Spec { key: "ram", title: "RAM", sizes: &RING, size: 1, col: 16, row: 0 },
        Spec { key: "disk", title: "Диск", sizes: &INFO, size: 0, col: 0, row: 10 },
        Spec { key: "net", title: "Сеть", sizes: &INFO, size: 0, col: 8, row: 10 },
        Spec { key: "uptime", title: "Аптайм", sizes: &INFO, size: 0, col: 16, row: 10 },
        Spec { key: "apps", title: "Топ приложений", sizes: &APPS_SIZES, size: 1, col: 24, row: 0 },
        Spec { key: "speed", title: "Скорость сети", sizes: &SPEED_SIZES, size: 1, col: 24, row: 10 },
    ],
};

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

    /// Рисует вкладку по сетке. `h` — высота панели без полосы правки.
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, grid: &mut Grid, w: f32, _h: f32) {
        let area = grid.area(w);
        grid.draw(ui, cv, area, 0, |ui, cv, it| self.paint(ui, cv, it));
    }

    unsafe fn paint(&mut self, ui: &mut Ui, cv: &Canvas, it: &Item) {
        match it.idx {
            CPU | GPU | RAM => self.ring_card(cv, it.rect, it.size, it.idx),
            DISK | NET | UPTIME => self.info_card(cv, it.rect, it.size, it.idx),
            TOP_APPS => self.top_apps(ui, cv, it.rect, it.size),
            SPEEDTEST => self.speed_card(ui, cv, it.rect, it.size),
            _ => {}
        }
    }

    /// Кольцо загрузки в трёх видах (`size` — индекс в `RING`): мини (кольцо и название), обычная
    /// карточка и широкая (крупное кольцо, справа название, подробности и плашки).
    unsafe fn ring_card(&self, cv: &Canvas, r: Rect, size: usize, i: usize) {
        let f = &cv.g.fonts;
        let c = &self.cur;
        let h = &self.heavy;
        let gb = |v: f32| format!("{:.1}", v / GB);
        let (name, sub, chips, col): (&str, String, Vec<String>, D2D1_COLOR_F) = match i {
            CPU => {
                let mut chips = Vec::new();
                if c.mhz > 0 {
                    chips.push(format!("{:.1} {}", c.mhz as f32 / 1000.0, crate::i18n::pick("ГГц", "GHz")));
                }
                chips.push(format!("{} {}", c.procs, crate::i18n::pick("проц.", "cores")));
                ("CPU", self.cpu_name.clone(), chips, crate::ui::hue(crate::ios::SysColor::Blue, cv.accent))
            }
            GPU => {
                let chips = if h.vram_total > 0.0 {
                    vec![format!("{:.1} / {:.1} {}", h.vram_used / GB as f64, h.vram_total / GB as f64, crate::i18n::pick("ГБ", "GB"))]
                } else {
                    Vec::new()
                };
                ("GPU", if h.gpu_name.is_empty() { "—".to_string() } else { short_gpu(&h.gpu_name) }, chips, status_violet())
            }
            _ => (
                "RAM",
                format!("{} / {} {}", gb(c.ram_used), gb(c.ram_total), crate::i18n::pick("ГБ", "GB")),
                vec![format!("{} {}", gb((c.ram_total - c.ram_used).max(0.0)), crate::i18n::pick("ГБ своб.", "GB free"))],
                status_green(),
            ),
        };
        cv.card(r);
        let v = self.rings[i].value;
        let pct = format!("{}%", (v * 100.0).round() as i32);
        match size {
            0 => {
                let (rad, cx, cy) = (19.0, r.x + 12.0 + 19.0, r.cy());
                cv.ring(cx, cy, rad, 4.0, v, col);
                cv.text(&pct, Rect::new(cx - 19.0, cy - 7.0, 38.0, 14.0), &f.small_c, txt());
                let tx = cx + rad + 9.0;
                let tw = (r.right() - tx - 8.0).max(0.0);
                // Название и короткая подробность: частота, видеопамять, занятая память.
                let brief = if i == RAM { sub.clone() } else { chips.first().cloned().unwrap_or_default() };
                cv.line(name, &f.body, tx, tw, cy - 6.0, Align::Leading, txt());
                cv.line(&brief, &f.small, tx, tw, cy + 7.0, Align::Leading, dim());
            }
            1 => {
                let (rx, ry) = (r.cx(), r.y + 46.0);
                cv.ring(rx, ry, 31.0, 5.0, v, col);
                cv.text(&pct, Rect::new(rx - 26.0, ry - 8.0, 52.0, 16.0), &f.body_c, txt());
                cv.text(name, Rect::new(r.x, r.y + 88.0, r.w, 14.0), &f.body_c, txt());
                cv.text(&sub, Rect::new(r.x + 6.0, r.y + 106.0, r.w - 12.0, 12.0), &f.small_c, dim());
                chips_row(cv, r.cx(), r.y + 126.0, &chips);
            }
            _ => {
                let (rad, cx, cy) = (38.0, r.x + 16.0 + 38.0, r.cy());
                cv.ring(cx, cy, rad, 6.0, v, col);
                cv.text(&pct, Rect::new(cx - 30.0, cy - 9.0, 60.0, 18.0), &f.num_c, txt());
                let tx = cx + rad + 18.0;
                let tw = (r.right() - tx - 10.0).max(0.0);
                let top = cy - 30.0;
                cv.text(name, Rect::new(tx, top, tw, 18.0), &f.title, txt());
                cv.text(&sub, Rect::new(tx, top + 22.0, tw, 14.0), &f.small, dim());
                chips_from(cv, tx, top + 44.0, &chips);
            }
        }
    }

    /// Диск, сеть и аптайм. Узкая (120×72) — главное значение и одна подробность; широкая
    /// (248×72) добавляет вторую колонку: свободное место, загрузку и отдачу рядом, число процессов.
    unsafe fn info_card(&self, cv: &Canvas, r: Rect, size: usize, i: usize) {
        let f = &cv.g.fonts;
        let c = &self.cur;
        let wide = size > 0;
        let iw = r.w - 2.0 * P;
        let vy = r.y + 41.0;
        cv.card(r);
        match i {
            DISK => {
                let tint = hue(SysColor::Teal, cv.accent);
                header(cv, r, icon::FOLDER, "Диск C:", tint);
                let frac = if c.disk_total > 0.0 { c.disk_used / c.disk_total } else { 0.0 };
                let used = format!("{:.0}", c.disk_used / GB);
                let uw = cv.digits_w(&used, 22.0, 600);
                cv.digits(&used, r.x + P - 1.0, uw + 4.0, vy, 22.0, 600, Align::Leading, txt());
                let total = format!("{} {:.0} {}", crate::i18n::pick("из", "of"), c.disk_total / GB, crate::i18n::pick("ГБ", "GB"));
                cv.line(&total, &f.small, r.x + P + uw + 4.0, iw - uw - 4.0, vy + 3.0, Align::Leading, dim());
                if wide {
                    let free = format!("{:.0} {}", (c.disk_total - c.disk_used).max(0.0) / GB, crate::i18n::pick("ГБ свободно", "GB free"));
                    cv.line(&free, &f.small, r.x + P, iw, vy + 3.0, Align::Trailing, dim());
                }
                cv.progress(Rect::new(r.x + P, r.bottom() - P - 4.0, iw, 4.0), frac, tint);
            }
            NET => {
                header(cv, r, icon::WIFI, "Сеть", hue(SysColor::Blue, cv.accent));
                let down = hue(SysColor::Blue, cv.accent);
                let up = hue(SysColor::Green, cv.accent);
                let (rx, tx) = (fmt_rate(c.rx_bps), fmt_rate(c.tx_bps));
                if wide {
                    // Две колонки: подпись со стрелкой цвета направления, под ней значение.
                    for (k, (arrow, label, value, col)) in [("↓ ", "Загрузка", &rx, down), ("↑ ", "Отдача", &tx, up)].into_iter().enumerate() {
                        let x = r.x + P + k as f32 * iw / 2.0;
                        let aw = cv.line(arrow, &f.small_b, x, 12.0, r.y + 35.0, Align::Leading, col);
                        cv.line(label, &f.small, x + aw, iw / 2.0 - aw, r.y + 35.0, Align::Leading, dim());
                        cv.line(value, &f.title, x, iw / 2.0 - 6.0, r.y + 53.0, Align::Leading, txt());
                    }
                } else {
                    for (k, (arrow, value, col)) in [("↓", &rx, down), ("↑", &tx, up)].into_iter().enumerate() {
                        let y = r.y + 39.0 + k as f32 * 17.0;
                        let aw = cv.line(arrow, &f.body, r.x + P, 12.0, y, Align::Leading, col);
                        cv.line(value, &f.body, r.x + P + aw + 2.0, iw - aw - 2.0, y, Align::Leading, txt());
                    }
                }
            }
            _ => {
                header(cv, r, icon::POWER, "В работе", hue(SysColor::Indigo, cv.accent));
                cv.line(&fmt_uptime(c.uptime_s), &f.num, r.x + P, iw, vy, Align::Leading, txt());
                let since = fmt_since(online::now_local_minutes(), c.uptime_s);
                cv.line(&since, &f.small, r.x + P, iw, r.bottom() - P - 4.0, Align::Leading, dim());
                if wide {
                    cv.digits(&c.procs.to_string(), r.x + P, iw, vy, 22.0, 600, Align::Trailing, txt());
                    cv.line(crate::i18n::pick("процессов", "processes"), &f.small, r.x + P, iw, r.bottom() - P - 4.0, Align::Trailing, dim());
                }
            }
        }
    }

    unsafe fn top_apps(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, _size: usize) {
        let f = &cv.g.fonts;
        cv.card(r);
        header(cv, r, icon::APPS, "Топ приложений", hue(SysColor::Purple, cv.accent));
        // Переключатель метрики — сегмент из двух капсул справа в шапке.
        for (k, (name, cpu)) in [("RAM", false), ("CPU", true)].into_iter().enumerate() {
            let seg = Rect::new(r.right() - P - 64.0 + k as f32 * 33.0, r.y + 10.0, 31.0, 14.0);
            if ui.chip(cv, id(STATS, 10 + 2 * k as u32), seg, name, self.top_by_cpu == cpu) && self.top_by_cpu != cpu {
                self.top_by_cpu = cpu;
                crate::state::set_stats_top_by_cpu(cpu);
            }
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

    /// Замер скорости. `size` — индекс в `SPEED_SIZES`: в узкой карточке (120×72) загрузка крупно,
    /// отдача строкой под ней и круглая кнопка в шапке; в широких — обе скорости рядом и
    /// кнопка-капсула справа.
    unsafe fn speed_card(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, size: usize) {
        let f = &cv.g.fonts;
        cv.card(r);
        let sp = &self.online.speed;
        header(cv, r, icon::BOLT, if size == 0 { "Скорость" } else { "Скорость сети" }, hue(SysColor::Cyan, cv.accent));
        let iw = r.w - 2.0 * P;
        let num = |v: f32| if sp.at > 0 { format!("{:.0}", v) } else { "—".into() };
        let (d, u) = (num(sp.down), num(sp.up));
        let unit = crate::i18n::pick("Мбит/с", "Mbps");
        let vy = r.y + 44.0;
        let (bg, fg) = if sp.running { (chip_bg(), dim()) } else { (cv.accent, ink()) };
        let go;
        if size == 0 {
            let dw = cv.digits_w(&d, 22.0, 600);
            cv.digits(&d, r.x + P - 1.0, dw + 4.0, vy - 3.0, 22.0, 600, Align::Leading, txt());
            cv.line(&format!("{unit} ↓"), &f.small, r.x + P + dw + 4.0, iw - dw - 4.0, vy, Align::Leading, dim());
            cv.line(&format!("{u} {unit} ↑"), &f.small, r.x + P, iw, r.bottom() - P - 4.0, Align::Leading, dim());
            let btn = Rect::new(r.right() - P - 18.0, r.y + 8.0, 18.0, 18.0);
            let (_, clicked) = ui.click_area(id(STATS, 11), btn);
            let hv = ui.hover_of(id(STATS, 11));
            cv.circle(btn.cx(), btn.cy(), 9.0, mix(bg, crate::ui::fg(), 0.08 * hv));
            cv.glyph(if sp.running { icon::REFRESH } else { icon::PLAY_SOLID }, btn, false, fg);
            go = clicked;
        } else {
            let info = if sp.running {
                crate::i18n::pick("идёт замер…", "testing…").to_string()
            } else if sp.at > 0 {
                let when = ago(sys::now_secs(), sp.at);
                if sp.place.is_empty() { when } else { format!("{when} • {}", sp.place) }
            } else {
                crate::i18n::pick("ещё не замеряли", "not measured yet").to_string()
            };
            cv.line(&info, &f.small, r.x + P + iw / 2.0, iw / 2.0, r.y + 17.0, Align::Trailing, dim());
            let label = match (sp.running, sp.failed, size) {
                (true, _, _) => crate::i18n::pick("Замер…", "Testing…"),
                (_, true, _) => crate::i18n::pick("Повторить", "Retry"),
                (_, _, 1) => crate::i18n::pick("Замерить", "Test"),
                _ => crate::i18n::pick("Замерить скорость", "Run speed test"),
            };
            let bw = cv.text_w(label, &f.small_c) + 26.0;
            let btn = Rect::new(r.right() - P - bw, vy - 11.0, bw, 22.0);
            go = ui.solid_button(cv, id(STATS, 11), btn, label, bg, fg, 11.0);
            // Загрузка и отдача: число крупно, единица со стрелкой мелко.
            let col_w = ((btn.x - r.x - P - 8.0) / 2.0).min(120.0);
            for (k, (v, arrow)) in [(&d, "↓"), (&u, "↑")].into_iter().enumerate() {
                let x = r.x + P + k as f32 * col_w;
                let vw = cv.digits_w(v, 22.0, 600);
                cv.digits(v, x - 1.0, vw + 4.0, vy, 22.0, 600, Align::Leading, txt());
                cv.line(&format!("{unit} {arrow}"), &f.small, x + vw + 4.0, col_w - vw - 8.0, vy + 3.0, Align::Leading, dim());
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
        c.ram_used = 11.2 * GB;
        c.ram_total = 16.0 * GB;
        c.disk_used = 312.0 * GB;
        c.disk_total = 476.0 * GB;
        c.procs = 241;
        c.uptime_s = 3 * 86400 + 4 * 3600 + 17 * 60;
        c.rx_bps = 2.4e6;
        c.tx_bps = 180e3;
        self.cpu_name = "AMD Ryzen 7 5800H".into();
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
        for (ring, v) in self.rings.iter_mut().zip([0.34, 0.18, 0.70]) {
            ring.target = v;
            ring.value = v;
        }
        self.online.speed = online::Speed { down: 94.6, up: 41.2, place: "FRA".into(), at: sys::now_secs() - 300, running: false, failed: false };
    }

    /// Один виджет вне сетки: номер в `SPEC`, индекс размера и прямоугольник.
    pub unsafe fn gallery(&mut self, ui: &mut Ui, cv: &Canvas, idx: usize, size: usize, rect: Rect) {
        self.paint(ui, cv, &Item { idx, size, rect });
    }
}

/// Внутренний отступ карточек вкладки.
const P: f32 = 12.0;

/// Шапка карточки: значок цвета метрики и название прописными (как подписи виджетов iOS).
unsafe fn header(cv: &Canvas, r: Rect, glyph: &str, title: &str, tint: D2D1_COLOR_F) {
    let cy = r.y + 17.0;
    cv.symbol(glyph, Rect::new(r.x + P, cy - 6.0, 12.0, 12.0), 11.0, tint);
    let title = crate::i18n::tr(title).to_uppercase();
    cv.line(&title, &cv.g.fonts.caps, r.x + P + 16.0, r.w - 2.0 * P - 16.0, cy, Align::Leading, dim());
}

/// Ширина ряда плашек.
unsafe fn chips_width(cv: &Canvas, labels: &[String]) -> f32 {
    labels.iter().map(|l| cv.text_w(l, &cv.g.fonts.small_c) + 12.0).sum::<f32>() + 5.0 * labels.len().saturating_sub(1) as f32
}

/// Ряд маленьких плашек по центру (частота, «N ГБ своб.»).
unsafe fn chips_row(cv: &Canvas, cx: f32, y: f32, labels: &[String]) {
    chips_from(cv, cx - chips_width(cv, labels) / 2.0, y, labels);
}

/// Ряд плашек от левого края `x`.
unsafe fn chips_from(cv: &Canvas, x: f32, y: f32, labels: &[String]) {
    let mut x = x;
    for l in labels {
        let w = cv.text_w(l, &cv.g.fonts.small_c) + 12.0;
        let r = Rect::new(x, y, w, 15.0);
        cv.rr(r, 6.0, chip_bg());
        cv.text(l, r, &cv.g.fonts.small_c, txt());
        x += w + 5.0;
    }
}

