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
use crate::ui::{chip_bg, dim, icon, ink, status_bar_green, status_green, status_violet, track, txt, Canvas, Rect, Ui};
use crate::{icons, sys};

use super::grid::Grid;
use super::layout::{sz, Size, Spec, TabSpec};
use super::{ago, id, PAD, STATS, TOP};

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
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, grid: &mut Grid, _w: f32, _h: f32) {
        let area = Rect::new(PAD, TOP, grid.window().content_w(), grid.window().content_h());
        grid.draw(ui, cv, area, 0, |ui, cv, it| match it.idx {
            CPU | GPU | RAM => self.ring_card(cv, it.rect, it.size, it.idx),
            DISK | NET | UPTIME => self.info_card(cv, it.rect, it.size, it.idx),
            TOP_APPS => self.top_apps(ui, cv, it.rect),
            SPEEDTEST => self.speed_card(ui, cv, it.rect, it.size),
            _ => {}
        });
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
                ("CPU", self.cpu_name.clone(), chips, cv.accent)
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
                let (rad, cx, cy) = (21.0, r.x + 12.0 + 21.0, r.cy());
                cv.ring(cx, cy, rad, 4.0, v, col);
                cv.text(&pct, Rect::new(cx - 19.0, cy - 7.0, 38.0, 14.0), &f.small_c, txt());
                let tx = cx + rad + 8.0;
                cv.text(name, Rect::new(tx, cy - 8.0, (r.right() - tx - 4.0).max(0.0), 16.0), &f.body, txt());
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

    /// Диск, сеть и аптайм: подпись и значение; в широкой карточке значение крупнее.
    unsafe fn info_card(&self, cv: &Canvas, r: Rect, size: usize, i: usize) {
        let f = &cv.g.fonts;
        let c = &self.cur;
        let (label, value) = match i {
            DISK => ("ДИСК C:", format!("{:.0} / {:.0} {}", c.disk_used / GB, c.disk_total / GB, crate::i18n::pick("ГБ", "GB"))),
            NET => ("СЕТЬ", format!("↓{}  ↑{}", fmt_rate(c.rx_bps), fmt_rate(c.tx_bps))),
            _ => ("В РАБОТЕ", fmt_uptime(c.uptime_s)),
        };
        cv.card(r);
        cv.text(label, Rect::new(r.x + 12.0, r.y + 15.0, r.w - 20.0, 11.0), &f.caps, dim());
        let font = if size == 0 { &f.body } else { &f.title };
        cv.text(&value, Rect::new(r.x + 12.0, r.y + 36.0, r.w - 18.0, 18.0), font, txt());
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
            crate::state::set_stats_top_by_cpu(self.top_by_cpu);
        }
        let list = if self.top_by_cpu { &self.heavy.top_cpu } else { &self.heavy.top_ram };
        let max = list.first().map(|x| x.1).unwrap_or(0.0).max(1e-9);
        let rows = (((r.h - 32.0) / 22.0).floor() as usize).clamp(1, 5);
        for (k, (name, v)) in list.iter().take(rows).enumerate() {
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
            cv.rr(Rect::new(bar.x, bar.y, (bar.w * (*v / max) as f32).max(3.0), 3.0), 1.5, status_bar_green());
        }
        if list.is_empty() {
            cv.text("Собираю данные…", Rect::new(r.x + 12.0, r.y + 34.0, r.w - 24.0, 12.0), &f.small, dim());
        }
    }

    /// `size` — индекс в `SPEED_SIZES`: в компактном виде кнопка запуска уменьшается до «▶» справа
    /// от чисел, в остальных — во всю ширину под ними.
    unsafe fn speed_card(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, size: usize) {
        let f = &cv.g.fonts;
        let compact = size == 0;
        cv.card(r);
        let sp = &self.online.speed;
        let tight_y = r.h < 70.0;
        let title_y = if tight_y { r.y + 5.0 } else { r.y + 8.0 };
        cv.text("Скорость сети", Rect::new(r.x + 12.0, title_y, 90.0, 12.0), &f.small, dim());
        let info = if sp.at > 0 {
            let when = ago(sys::now_secs(), sp.at);
            if sp.place.is_empty() { when } else { format!("{when} • {}", sp.place) }
        } else {
            "ещё не замеряли".into()
        };
        cv.text(&info, Rect::new(r.right() - 112.0, title_y, 100.0, 12.0), &f.small_r, dim());
        let num = |v: f32| if sp.at > 0 { format!("{:.0}", v) } else { "—".into() };
        let (d, u) = (num(sp.down), num(sp.up));
        let digits = |t: &str| if sp.at > 0 { t.chars().count() as f32 * 8.6 + 3.0 } else { 22.0 };
        let dw = digits(&d);
        let num_y = if tight_y { r.y + 16.0 } else { r.y + 21.0 };
        let sub_y = if tight_y { r.y + 22.0 } else { r.y + 28.0 };
        cv.text(&d, Rect::new(r.x + 12.0, num_y, dw + 4.0, 22.0), &f.num, txt());
        cv.text("Мбит/с ↓", Rect::new(r.x + 12.0 + dw, sub_y, 50.0, 12.0), &f.small, dim());
        let nums_w = if compact { r.w - 46.0 } else { r.w };
        let ux = r.x + 12.0 + nums_w / 2.0 - 8.0;
        let uw = digits(&u);
        cv.text(&u, Rect::new(ux, num_y, uw + 4.0, 22.0), &f.num, txt());
        cv.text("Мбит/с ↑", Rect::new(ux + uw, sub_y, 50.0, 12.0), &f.small, dim());
        let (label, btn) = if compact {
            (if sp.running { "…" } else { "▶" }, Rect::new(r.right() - 44.0, num_y, 32.0, 20.0))
        } else {
            let label = if sp.running {
                "Идёт замер…"
            } else if sp.failed {
                "▶ Повторить замер"
            } else {
                "▶ Замерить скорость"
            };
            let (by, bh) = if tight_y { (r.y + 35.0, 18.0) } else { (r.y + 49.0, 21.0) };
            (label, Rect::new(r.x + 12.0, by, r.w - 24.0, bh))
        };
        let bg = if sp.running { chip_bg() } else { cv.accent };
        let fg = if sp.running { dim() } else { ink() };
        if ui.solid_button(cv, id(STATS, 11), btn, label, bg, fg, 8.0) && !sp.running {
            online::run_speedtest();
        }
        let _ = icon::PLAY;
    }
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

