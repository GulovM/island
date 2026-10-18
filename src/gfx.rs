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

use crate::ui::{self, Fonts};
use crate::{log, sys, theme};

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
    pub(crate) _dcomp: IDCompositionDevice,
    pub(crate) _dcomp_target: IDCompositionTarget,
    pub(crate) _visual: IDCompositionVisual,
}

pub(crate) unsafe fn make_text(dwrite: &IDWriteFactory, scale: f32) -> Result<IDWriteTextFormat> {
    let family = if ui::font_exists(dwrite, w!("Segoe UI Variable Text")) { w!("Segoe UI Variable Text") } else { w!("Segoe UI") };
    let text = dwrite.CreateTextFormat(
        family,
        None,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STRETCH_NORMAL,
        8.5 * scale,
        w!("ru-RU"),
    )?;
    text.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
    text.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    Ok(text)
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
        self.text = make_text(dwrite, scale)?;
        self.fonts.clear_layouts();
        Ok(())
    }

    /// Устройство D3D потеряно (сброс драйвера, смена GPU, сон) — нужно пересоздать всё.
    pub(crate) unsafe fn removed(&self) -> bool {
        self.device.GetDeviceRemovedReason().is_err()
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
