//! Графика острова: устройство D3D11, цепочка обмена с DirectComposition, контекст Direct2D и
//! шрифты. Пересоздаётся целиком после потери устройства (`Gfx::removed`).

use windows::core::{w, Interface, Result};
use windows::Win32::Foundation::{E_FAIL, HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows_numerics::Vector2;

use std::cell::RefCell;

use crate::ui::{self, Fonts};
use crate::{log, sys, theme};

/// Сколько градиентных кистей держать между кадрами: тело, кромка, капля и градиенты вкладок.
const GRADIENT_CACHE: usize = 16;
/// Сколько контуров тела держать: за кадр их до четырёх (тело, кромка, подсветка, свечение).
const PATH_CACHE: usize = 8;

/// Несколько последних ресурсов D2D между кадрами: поиск перебором, при переполнении вытесняется
/// самый давно использованный. Живёт в `Gfx`, поэтому пропадает вместе с устройством.
pub(crate) struct Recent<K, V> {
    cap: usize,
    items: Vec<(K, V)>,
}

impl<K: PartialEq, V: Clone> Recent<K, V> {
    pub(crate) fn new(cap: usize) -> Self {
        Self { cap: cap.max(1), items: Vec::with_capacity(cap.max(1)) }
    }

    pub(crate) fn get(&mut self, key: &K) -> Option<V> {
        let i = self.items.iter().position(|(k, _)| k == key)?;
        let item = self.items.remove(i);
        let v = item.1.clone();
        self.items.push(item);
        Some(v)
    }

    pub(crate) fn put(&mut self, key: K, v: V) {
        if self.items.len() >= self.cap {
            self.items.remove(0);
        }
        self.items.push((key, v));
    }
}

pub(crate) struct Gfx {
    pub(crate) device: ID3D11Device,
    pub(crate) d2d: ID2D1Factory1,
    pub(crate) swapchain: IDXGISwapChain1,
    pub(crate) dc: ID2D1DeviceContext,
    pub(crate) target: Option<ID2D1Bitmap1>,
    pub(crate) brush: ID2D1SolidColorBrush,
    pub(crate) text: IDWriteTextFormat,
    /// Шрифты тулкита (логические размеры, масштаб даёт трансформация).
    pub(crate) fonts: Fonts,
    /// Штрих с круглыми концами для дуг-колец.
    pub(crate) round: ID2D1StrokeStyle1,
    /// Градиентные кисти: ключ — остановки с альфой, нормированной на самую плотную (общая
    /// прозрачность уходит в `SetOpacity`), поэтому затухание не создаёт новых текстур градиента.
    gradients: RefCell<Recent<Vec<u32>, ID2D1LinearGradientBrush>>,
    /// Контуры тела острова (ключ — точные параметры формы): в простое кадр их не перестраивает.
    pub(crate) paths: RefCell<Recent<[u32; 6], ID2D1PathGeometry1>>,
    /// Маска тени капли под выбранной вкладкой (вычитание геометрий — дорогая операция на CPU).
    pub(crate) pill_masks: RefCell<Recent<[u32; 6], ID2D1Geometry>>,
    /// Векторные значки (`ui::symbols`) по коду глифа: строятся один раз, `None` — значка нет.
    pub(crate) symbols: RefCell<std::collections::HashMap<u32, Option<ID2D1Geometry>>>,
    pub(crate) _dcomp: IDCompositionDevice,
    pub(crate) _dcomp_target: IDCompositionTarget,
    pub(crate) _visual: IDCompositionVisual,
}

pub(crate) unsafe fn make_text(dwrite: &IDWriteFactory, scale: f32) -> Result<IDWriteTextFormat> {
    let family = if ui::font_exists(dwrite, w!("Segoe UI Variable Text")) { w!("Segoe UI Variable Text") } else { w!("Segoe UI") };
    // Подписи вкладок рисуются без трансформации (кегль уже в физических пикселях), поэтому
    // оптический размер считается от логического кегля 8.5, а не от `8.5 * scale`.
    let text = ui::fmt_var(dwrite, family, ui::weight::EMPHASIZED, 8.5 * scale, true)?;
    if let Ok(t3) = text.cast::<IDWriteTextFormat3>() {
        let axes = [
            DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT, value: ui::weight::EMPHASIZED as f32 },
            DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_OPTICAL_SIZE, value: ui::optical_size(8.5) },
        ];
        let _ = t3.SetFontAxisValues(&axes);
    }
    // Подпись вкладки не обрезается многоточием (ширина вкладки подобрана под текст).
    text.SetTrimming(&DWRITE_TRIMMING::default(), None)?;
    Ok(text)
}

/// Трекинг подписей вкладок (роль Callout в масштабе острова с учётом DPI-масштаба `scale`).
pub(crate) fn tab_tracking(scale: f32) -> f32 {
    crate::ios::tracking_px(crate::ios::TextRole::Callout, 8.5 * scale)
}
impl Gfx {
    /// Окно и буфер фиксированы на максимум и меняются только при смене DPI: анимируется
    /// форма внутри, поэтому геометрия и картинка не рассинхронизируются.
    /// Прежний `Gfx` для этого окна к моменту вызова должен быть уничтожен (цель DComp одна).
    pub(crate) unsafe fn new(hwnd: HWND, w: i32, h: i32, scale: f32, dwrite: &IDWriteFactory) -> Result<Self> {
        // Без GPU-драйвера (виртуалка, RDP, «Базовый видеоадаптер») берём программный WARP.
        let mut device: Option<ID3D11Device> = None;
        for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
            let r = D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            );
            match r {
                Ok(()) if device.is_some() => break,
                Ok(()) => {}
                Err(e) if driver == D3D_DRIVER_TYPE_WARP => return Err(e),
                Err(e) => log::error(&format!("D3D11: аппаратное устройство недоступно ({e}), пробую WARP")),
            }
        }
        let device = device.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let dxgi: IDXGIDevice = device.cast()?;
        let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w as u32,
            Height: h as u32,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };
        let swapchain = factory.CreateSwapChainForComposition(&device, &desc, None)?;

        let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let d2d_dev = d2d.CreateDevice(&dxgi)?;
        let dc = d2d_dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        let brush = dc.CreateSolidColorBrush(&theme::get().bg, None)?;
        let text = make_text(dwrite, scale)?;
        let fonts = Fonts::new(dwrite)?;
        fonts.register_tracking(&text, tab_tracking(scale));
        let round = d2d.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES1 {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
                transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
            },
            None,
        )?;

        let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
        let dcomp_target = dcomp.CreateTargetForHwnd(hwnd, true)?;
        let visual = dcomp.CreateVisual()?;
        visual.SetContent(&swapchain)?;
        dcomp_target.SetRoot(&visual)?;
        dcomp.Commit()?;

        let mut g = Gfx {
            device,
            d2d,
            swapchain,
            dc,
            target: None,
            brush,
            text,
            fonts,
            round,
            gradients: RefCell::new(Recent::new(GRADIENT_CACHE)),
            paths: RefCell::new(Recent::new(PATH_CACHE)),
            pill_masks: RefCell::new(Recent::new(2)),
            symbols: RefCell::new(std::collections::HashMap::new()),
            _dcomp: dcomp,
            _dcomp_target: dcomp_target,
            _visual: visual,
        };
        g.create_target()?;
        Ok(g)
    }

    pub(crate) unsafe fn create_target(&mut self) -> Result<()> {
        let surface: IDXGISurface = self.swapchain.GetBuffer(0)?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        self.target = Some(self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?);
        Ok(())
    }

    /// Новый размер буфера и шрифта при смене DPI (остров переехал на монитор с другим масштабом).
    pub(crate) unsafe fn resize(&mut self, w: i32, h: i32, scale: f32, dwrite: &IDWriteFactory) -> Result<()> {
        self.dc.SetTarget(None);
        self.target = None;
        self.swapchain.ResizeBuffers(0, w as u32, h as u32, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
        self.create_target()?;
        self.fonts.unregister_tracking(&self.text);
        self.text = make_text(dwrite, scale)?;
        self.fonts.clear_layouts();
        self.fonts.register_tracking(&self.text, tab_tracking(scale));
        Ok(())
    }

    /// Устройство D3D потеряно (сброс драйвера, смена GPU, сон) — нужно пересоздать всё.
    pub(crate) unsafe fn removed(&self) -> bool {
        self.device.GetDeviceRemovedReason().is_err()
    }

    /// Линейный градиент от `a` к `b`; цвета остановок уже с нужной прозрачностью. Кисть берётся из
    /// кэша: альфа остановок нормируется на самую плотную, а общий множитель ставится в `SetOpacity`
    /// (для линейной интерполяции это то же самое), поэтому плавное затухание и сдвиг концов не
    /// создают новых коллекций остановок. `None` — градиент полностью прозрачен или D2D отказал.
    pub(crate) unsafe fn linear_gradient(&self, a: Vector2, b: Vector2, stops: &[(f32, D2D1_COLOR_F)]) -> Option<ID2D1LinearGradientBrush> {
        let peak = stops.iter().fold(0.0f32, |m, s| m.max(s.1.a));
        if peak <= 0.0 {
            return None;
        }
        // Шаг 1/4096 — тоньше 8-битного канала: на экране неотличимо, а ключ не «дрожит» от округлений.
        let q = |v: f32| (v.clamp(0.0, 1.0) * 4096.0).round() as u32;
        let key: Vec<u32> = stops.iter().flat_map(|&(p, c)| [q(p), q(c.r), q(c.g), q(c.b), q(c.a / peak)]).collect();
        let mut cache = self.gradients.borrow_mut();
        let br = match cache.get(&key) {
            Some(br) => br,
            None => {
                let list: Vec<D2D1_GRADIENT_STOP> = stops
                    .iter()
                    .map(|&(position, c)| D2D1_GRADIENT_STOP { position, color: D2D1_COLOR_F { a: c.a / peak, ..c } })
                    .collect();
                let coll = ID2D1RenderTarget::CreateGradientStopCollection(&self.dc, &list, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok()?;
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: a, endPoint: b };
                let br = self.dc.CreateLinearGradientBrush(&props, None, &coll).ok()?;
                cache.put(key, br.clone());
                br
            }
        };
        br.SetStartPoint(a);
        br.SetEndPoint(b);
        br.SetOpacity(peak.min(1.0));
        Some(br)
    }

    pub(crate) unsafe fn draw_text(&self, s: &str, x: f32, y: f32, w: f32, h: f32, color: D2D1_COLOR_F) {
        self.brush.SetColor(&color);
        if let Some(layout) = self.fonts.layout(s, &self.text, w, h) {
            self.dc.DrawTextLayout(Vector2 { X: x, Y: y }, &layout, &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
            return;
        }
        sys::with_utf16(s, |wide| {
            self.dc.DrawText(
                wide,
                &self.text,
                &D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h },
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::Recent;

    #[test]
    fn recent_evicts_least_recently_used() {
        let mut c = Recent::new(2);
        c.put(1, "a");
        c.put(2, "b");
        // Обращение к 1 делает её свежей: вытесняется 2.
        assert_eq!(c.get(&1), Some("a"));
        c.put(3, "c");
        assert_eq!(c.get(&2), None);
        assert_eq!(c.get(&1), Some("a"));
        assert_eq!(c.get(&3), Some("c"));
        assert_eq!(c.items.len(), 2);
    }
}
