//! Кэш битмапов для тяжёлых элементов оформления.

use std::ffi::c_void;

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::*;

const CACHE_CAP: usize = 128;

/// Кэш `ID2D1Bitmap1` для тяжёлых элементов (обложки, иконки, миниатюры). Привязан к устройству:
/// при потере графики очищается вместе с `Gfx`. Вытесняет самый давно использованный.
#[derive(Default)]
pub struct BitmapCache {
    items: Vec<(u64, ID2D1Bitmap1, u64)>,
    tick: u64,
}

impl BitmapCache {
    /// Возвращает битмап по ключу; если его нет — `make` отдаёт пиксели BGRA (premultiplied, w×h).
    pub unsafe fn get(
        &mut self,
        dc: &ID2D1DeviceContext,
        key: u64,
        w: u32,
        h: u32,
        make: impl FnOnce() -> Vec<u8>,
    ) -> Option<ID2D1Bitmap1> {
        self.tick += 1;
        if let Some(it) = self.items.iter_mut().find(|i| i.0 == key) {
            it.2 = self.tick;
            return Some(it.1.clone());
        }
        let pixels = make();
        let need = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4));
        if need.is_none_or(|n| w == 0 || h == 0 || pixels.len() < n) {
            return None;
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        let bmp = dc
            .CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(pixels.as_ptr() as *const c_void), w * 4, &props)
            .ok()?;
        if self.items.len() >= CACHE_CAP {
            if let Some(oldest) = self.items.iter().enumerate().min_by_key(|(_, i)| i.2).map(|(n, _)| n) {
                self.items.swap_remove(oldest);
            }
        }
        self.items.push((key, bmp.clone(), self.tick));
        Some(bmp)
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}
