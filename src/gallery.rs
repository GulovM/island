//! Галерея виджетов (только с функцией `gallery`): каждый настраиваемый виджет во всех размерах
//! и каждый циферблат часов — в PNG, в светлой и тёмной теме. Проверка дизайна без запуска
//! острова: `cargo run --features gallery -- --gallery <папка>`.
//!
//! Рисуется на скрытом окне тем же `Gfx`, что и остров, но в собственный битмап, который потом
//! копируется в память и пишется через WIC. Данные — демонстрационные (`demo` у вкладок).

use std::path::Path;

use windows::core::{w, Result};
use windows::Win32::Foundation::{E_FAIL, HWND, SYSTEMTIME};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WINDOW_EX_STYLE, WS_POPUP};

use crate::gfx::Gfx;
use crate::online::days_from_civil;
use crate::tabs::layout::{Size, TabSpec};
use crate::tabs::{controls, stats, today};
use crate::theme::{self, PaletteStyle};
use crate::ui::{self, dim, txt, Canvas, Rect, Ui};

/// Масштаб картинки: логический пиксель острова — два пикселя PNG.
const SCALE: f32 = 2.0;
const MARGIN: f32 = 20.0;
const CELL_GAP: f32 = 18.0;
const ROW_GAP: f32 = 26.0;
const LABEL_H: f32 = 16.0;
const TITLE_H: f32 = 18.0;

#[derive(Clone, Copy)]
enum Tab {
    Controls,
    Today,
    Stats,
}

/// Ряд листа: подпись, вкладка, номер виджета в её `SPEC` и циферблат (для часов).
struct Row {
    title: String,
    tab: Tab,
    idx: usize,
    clock: Option<usize>,
}

fn spec(tab: Tab) -> &'static TabSpec {
    let base = match tab {
        Tab::Controls => &controls::SPEC,
        Tab::Today => &today::SPEC,
        Tab::Stats => &stats::SPEC,
    };
    // Виджеты «на весь остров» показываются на ширине в 8 колонок.
    crate::tabs::layout::resolve_full(base, 8)
}

fn sizes(r: &Row) -> &'static [Size] {
    spec(r.tab).specs[r.idx].sizes
}

fn rows(tab: Tab, idx: &[usize]) -> Vec<Row> {
    idx.iter().map(|&i| Row { title: spec(tab).specs[i].title.to_string(), tab, idx: i, clock: None }).collect()
}

fn sheets() -> Vec<(&'static str, Vec<Row>)> {
    let clock = (0..today::CLOCK_STYLES).map(|s| Row { title: format!("Часы — циферблат {}", s + 1), tab: Tab::Today, idx: 0, clock: Some(s) }).collect();
    vec![
        ("clock", clock),
        ("today", rows(Tab::Today, &[1, 2, 3, 4])),
        ("stats", rows(Tab::Stats, &[0, 1, 2, 3, 4, 5, 6, 7])),
        ("controls", rows(Tab::Controls, &[0, 1, 2, 3, 4, 5, 6, 7])),
    ]
}

fn row_h(r: &Row) -> f32 {
    TITLE_H + LABEL_H + sizes(r).iter().map(|z| z.pixel_h()).fold(0.0, f32::max)
}

/// Размер листа в логических пикселях.
fn sheet_size(rows: &[Row]) -> (f32, f32) {
    let w = rows
        .iter()
        .map(|r| {
            let s = sizes(r);
            s.iter().map(|z| z.pixel_w()).sum::<f32>() + CELL_GAP * (s.len() as f32 - 1.0)
        })
        .fold(0.0, f32::max);
    let h: f32 = rows.iter().map(row_h).sum::<f32>() + ROW_GAP * (rows.len() as f32 - 1.0);
    (w + 2.0 * MARGIN, h + 2.0 * MARGIN)
}

struct Tabs {
    controls: controls::Controls,
    today: today::Today,
    stats: stats::Stats,
}

impl Tabs {
    fn step(&mut self, dt: f32) {
        self.controls.step(dt);
        self.today.step(dt);
        self.stats.step(dt);
    }
}

unsafe fn draw_sheet(g: &Gfx, ui: &mut Ui, tabs: &mut Tabs, rows: &[Row], t: &SYSTEMTIME) {
    let cv = Canvas { g, alpha: 1.0, accent: theme::get().accent };
    let f = &g.fonts;
    let mut y = MARGIN;
    for r in rows {
        cv.text(&r.title, Rect::new(MARGIN, y, 400.0, TITLE_H - 4.0), &f.body, txt());
        let mut x = MARGIN;
        let cy = y + TITLE_H;
        for (k, z) in sizes(r).iter().enumerate() {
            let label = format!("{}×{} · {}×{}", z.w, z.h, z.pixel_w(), z.pixel_h());
            cv.text(&label, Rect::new(x, cy, 220.0, LABEL_H - 4.0), &f.small, dim());
            let rect = Rect::new(x, cy + LABEL_H, z.pixel_w(), z.pixel_h());
            match r.tab {
                Tab::Controls => tabs.controls.gallery(ui, &cv, r.idx, k, rect),
                Tab::Stats => tabs.stats.gallery(ui, &cv, r.idx, k, rect),
                Tab::Today => {
                    if let Some(c) = r.clock {
                        tabs.today.set_clock_style(c);
                    }
                    tabs.today.gallery(ui, &cv, r.idx, k, rect, t);
                }
            }
            x += z.pixel_w() + CELL_GAP;
        }
        y += row_h(r) + ROW_GAP;
    }
}

unsafe fn render(g: &Gfx, ui: &mut Ui, tabs: &mut Tabs, rows: &[Row], t: &SYSTEMTIME, path: &Path) -> Result<()> {
    let (lw, lh) = sheet_size(rows);
    let (pw, ph) = ((lw * SCALE).ceil() as u32, (lh * SCALE).ceil() as u32);
    let size = D2D_SIZE_U { width: pw, height: ph };
    let props = |opts| D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: opts,
        ..Default::default()
    };
    let target = g.dc.CreateBitmap(size, None, 0, &props(D2D1_BITMAP_OPTIONS_TARGET))?;
    g.dc.SetTarget(&target);
    // Два прохода: первый ставит цели пружин (кольца, отметки), между ними пружины доезжают.
    for pass in 0..2 {
        g.dc.BeginDraw();
        g.dc.Clear(Some(&theme::get().bg));
        g.dc.SetTransform(&ui::matrix(SCALE, 0.0, 0.0));
        ui.begin_frame((-1000.0, -1000.0), false);
        ui.begin_content(0.0, false);
        draw_sheet(g, ui, tabs, rows, t);
        ui.end_frame();
        g.dc.EndDraw(None, None)?;
        if pass == 0 {
            for _ in 0..240 {
                tabs.step(1.0 / 60.0);
                ui.step(1.0 / 60.0);
            }
        }
    }
    g.dc.SetTarget(None);
    let cpu = g.dc.CreateBitmap(size, None, 0, &props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW))?;
    cpu.CopyFromBitmap(None, &target, None)?;
    let map = cpu.Map(D2D1_MAP_OPTIONS_READ)?;
    let mut px = Vec::with_capacity((pw * ph * 4) as usize);
    for row in 0..ph {
        let line = std::slice::from_raw_parts(map.bits.add((row * map.pitch) as usize), (pw * 4) as usize);
        px.extend_from_slice(line);
    }
    cpu.Unmap()?;
    save_png(path, pw, ph, &px)
}

unsafe fn save_png(path: &Path, w: u32, h: u32, bgra: &[u8]) -> Result<()> {
    let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    let wide = crate::sys::wz(&path.to_string_lossy());
    let stream = f.CreateStream()?;
    // GENERIC_WRITE = 0x4000_0000.
    stream.InitializeFromFilename(windows::core::PCWSTR(wide.as_ptr()), 0x4000_0000)?;
    let enc = f.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
    enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
    let mut frame: Option<IWICBitmapFrameEncode> = None;
    enc.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
    let frame = frame.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
    frame.Initialize(None)?;
    frame.SetSize(w, h)?;
    let mut fmt = GUID_WICPixelFormat32bppBGRA;
    frame.SetPixelFormat(&mut fmt)?;
    frame.WritePixels(h, w * 4, bgra)?;
    frame.Commit()?;
    enc.Commit()
}

pub fn run(dir: &Path) {
    if let Err(e) = unsafe { run_inner(dir) } {
        eprintln!("галерея: {e}");
        std::process::exit(1);
    }
}

unsafe fn run_inner(dir: &Path) -> Result<()> {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    std::fs::create_dir_all(dir).map_err(|_| windows::core::Error::from(E_FAIL))?;
    let inst = GetModuleHandleW(None)?;
    let hwnd: HWND = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!("island-gallery"), WS_POPUP, 0, 0, 16, 16, None, None, Some(inst.into()), None)?;
    let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
    let g = Gfx::new(hwnd, 16, 16, SCALE, &dwrite)?;

    // Время 10:09:30 — классическое «витринное» положение стрелок; дата сегодняшняя.
    let mut t = GetLocalTime();
    (t.wHour, t.wMinute, t.wSecond) = (10, 9, 30);
    let day = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);

    let accent = [0.184, 0.435, 0.929];
    for (name, bg) in [("light", [0.945, 0.953, 0.965]), ("dark", [0.05, 0.05, 0.05])] {
        theme::set(theme::derive(bg, accent, PaletteStyle::Ios));
        theme::set_glass(true);
        for (sheet, rows) in sheets() {
            let mut tabs = Tabs { controls: Default::default(), today: Default::default(), stats: Default::default() };
            tabs.controls.demo();
            tabs.today.demo(day);
            tabs.stats.demo();
            let mut ui = Ui::default();
            let path = dir.join(format!("{sheet}-{name}.png"));
            render(&g, &mut ui, &mut tabs, &rows, &t, &path)?;
            println!("{}", path.display());
        }
    }
    Ok(())
}
