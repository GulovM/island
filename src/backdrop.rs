//! Подложка стекла на macOS (`NSVisualEffectView` на `glass_wnd`) + расчёт параметров размытия.

use std::cell::Cell;

use windows::core::Result;
use windows::Win32::Foundation::{E_FAIL, HWND};

#[derive(Clone, Debug)]
pub(crate) struct Backdrop {
    _size: Cell<(f32, f32)>,
    _blur: Cell<f32>,
    _shadow: Cell<(f32, f32, f32)>,
}

pub const fn use_host_backdrop(build: u32) -> bool {
    build >= 22000
}

pub(crate) fn blur_sigma(material: crate::ios::Material, _dark: bool, scale: f32) -> f32 {
    let base = match material {
        crate::ios::Material::Ultrathin
        | crate::ios::Material::Thin
        | crate::ios::Material::Regular
        | crate::ios::Material::Thick => 25.0,
        crate::ios::Material::Chrome => 12.5,
    };
    (base * scale).clamp(6.25, 120.0)
}

pub(crate) unsafe fn effects_supported(_hwnd: HWND) -> bool {
    true
}

pub(crate) unsafe fn enable_host_backdrop(_hwnd: HWND) -> Result<()> {
    Ok(())
}

impl Backdrop {
    /// Возвращает `Err` на macOS, чтобы рендер управлял нативным `NSVisualEffectView` окном `glass_wnd`.
    pub(crate) unsafe fn new(_hwnd: HWND, _dq: &mut Option<()>) -> Result<Self> {
        Err(windows::core::Error::from_hresult(E_FAIL))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_shape(
        &self,
        _x0: f32,
        _y0: f32,
        _aw: f32,
        _ah: f32,
        _ear: f32,
        _r_top: f32,
        _r_bot: f32,
    ) -> Result<()> {
        Ok(())
    }

    pub(crate) fn set_blur(&self, sigma: f32) -> Result<()> {
        self._blur.set(sigma);
        Ok(())
    }

    pub(crate) fn set_shadow(&self, dy: f32, blur: f32, opacity: f32) -> Result<()> {
        self._shadow.set((dy, blur, opacity));
        Ok(())
    }

    pub(crate) fn set_visible(&self, _visible: bool) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{blur_sigma, use_host_backdrop};
    use crate::ios::Material;

    #[test]
    fn host_backdrop_build_threshold() {
        assert!(!use_host_backdrop(0));
        assert!(!use_host_backdrop(19041));
        assert!(!use_host_backdrop(19045));
        assert!(!use_host_backdrop(21999));
        assert!(use_host_backdrop(22000));
        assert!(use_host_backdrop(22631));
        assert!(use_host_backdrop(26100));
    }

    #[test]
    fn blur_sigma_scales_and_clamps() {
        assert_eq!(blur_sigma(Material::Regular, true, 1.0), 25.0);
        assert_eq!(blur_sigma(Material::Chrome, false, 1.0), 12.5);
        assert_eq!(blur_sigma(Material::Regular, true, 1.5), 37.5);
        // Минимальный и максимальный пороги защищают от вырожденного размытия.
        assert_eq!(blur_sigma(Material::Chrome, false, 0.01), 6.25);
        assert_eq!(blur_sigma(Material::Regular, true, 10.0), 120.0);
    }
}
