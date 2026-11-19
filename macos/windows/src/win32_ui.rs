#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, clippy::too_many_arguments)]

pub mod Win32 {
    pub mod Foundation {
        pub use crate::foundation::*;
    }

    pub mod Globalization {
        use windows_core::PCWSTR;

        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct LGRPID(pub u32);

        pub unsafe fn GetUserDefaultLocaleName(lpLocaleName: &mut [u16]) -> i32 {
            let loc = {
                #[cfg(target_os = "macos")]
                {
                    let langs = std::process::Command::new("/usr/bin/defaults")
                        .args(["read", "-g", "AppleLanguages"])
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
                        .unwrap_or_default();
                    if langs.contains("\"ru") || langs.contains("\"uk") || langs.contains("\"be") || langs.contains("\"kk") {
                        "ru-RU"
                    } else {
                        "en-US"
                    }
                }
                #[cfg(not(target_os = "macos"))]
                "ru-RU"
            };
            if !lpLocaleName.is_empty() {
                let mut i = 0usize;
                for u in loc.encode_utf16() {
                    if i + 1 >= lpLocaleName.len() {
                        break;
                    }
                    lpLocaleName[i] = u;
                    i += 1;
                }
                lpLocaleName[i] = 0;
                return (i + 1) as i32;
            }
            0
        }

        pub unsafe fn CompareStringOrdinal(
            s1: PCWSTR,
            _c1: i32,
            s2: PCWSTR,
            _c2: i32,
            ignore_case: bool,
        ) -> i32 {
            let a = s1.to_string().unwrap_or_default();
            let b = s2.to_string().unwrap_or_default();
            let ord = if ignore_case {
                a.to_lowercase().cmp(&b.to_lowercase())
            } else {
                a.cmp(&b)
            };
            match ord {
                std::cmp::Ordering::Less => 1,
                std::cmp::Ordering::Equal => 2,
                std::cmp::Ordering::Greater => 3,
            }
        }
    }

    pub mod Graphics {
        pub use crate::d2d as Direct2D;
        pub use crate::dwrite as DirectWrite;
        pub use crate::dxgi_d3d::{Direct3D, Direct3D11, DirectComposition, Dxgi};

        pub mod Gdi {
            use std::ffi::c_void;
            use std::ops::{BitAnd, BitOr};
            use std::sync::atomic::{AtomicIsize, Ordering};
            use std::sync::{Mutex, OnceLock};
            use std::collections::HashMap;

            use windows_core::BOOL;

            use crate::foundation::{HWND, LPARAM, POINT, RECT};

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
            pub struct HMONITOR(pub isize);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct HDC(pub *mut c_void);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub struct HRGN(pub *mut c_void);

            impl Default for HRGN {
                fn default() -> Self {
                    Self(std::ptr::null_mut())
                }
            }
            impl HRGN {
                pub fn is_invalid(self) -> bool {
                    self.0.is_null()
                }
            }

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub struct HBITMAP(pub *mut c_void);

            impl Default for HBITMAP {
                fn default() -> Self {
                    Self(std::ptr::null_mut())
                }
            }
            impl HBITMAP {
                pub fn is_invalid(self) -> bool {
                    self.0.is_null()
                }
            }

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct HBRUSH(pub *mut c_void);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct HGDIOBJ(pub *mut c_void);

            impl From<HRGN> for HGDIOBJ {
                fn from(v: HRGN) -> Self {
                    Self(v.0)
                }
            }
            impl From<HBITMAP> for HGDIOBJ {
                fn from(v: HBITMAP) -> Self {
                    Self(v.0)
                }
            }
            impl From<HBRUSH> for HGDIOBJ {
                fn from(v: HBRUSH) -> Self {
                    Self(v.0)
                }
            }

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct MONITORINFO {
                pub cbSize: u32,
                pub rcMonitor: RECT,
                pub rcWork: RECT,
                pub dwFlags: u32,
            }

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct MONITOR_FROM_FLAGS(pub u32);
            pub const MONITOR_DEFAULTTOPRIMARY: MONITOR_FROM_FLAGS = MONITOR_FROM_FLAGS(1);
            pub const MONITOR_DEFAULTTONEAREST: MONITOR_FROM_FLAGS = MONITOR_FROM_FLAGS(2);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct CREATE_POLYGON_RGN_MODE(pub i32);
            pub const ALTERNATE: CREATE_POLYGON_RGN_MODE = CREATE_POLYGON_RGN_MODE(1);
            pub const WINDING: CREATE_POLYGON_RGN_MODE = CREATE_POLYGON_RGN_MODE(2);

            impl BitOr for MONITOR_FROM_FLAGS {
                type Output = Self;
                fn bitor(self, rhs: Self) -> Self {
                    Self(self.0 | rhs.0)
                }
            }
            impl BitAnd for MONITOR_FROM_FLAGS {
                type Output = Self;
                fn bitand(self, rhs: Self) -> Self {
                    Self(self.0 & rhs.0)
                }
            }

            pub const BI_RGB: u32 = 0;
            pub const DIB_RGB_COLORS: u32 = 0;

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default)]
            pub struct RGBQUAD {
                pub rgbBlue: u8,
                pub rgbGreen: u8,
                pub rgbRed: u8,
                pub rgbReserved: u8,
            }

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default)]
            pub struct BITMAPINFOHEADER {
                pub biSize: u32,
                pub biWidth: i32,
                pub biHeight: i32,
                pub biPlanes: u16,
                pub biBitCount: u16,
                pub biCompression: u32,
                pub biSizeImage: u32,
                pub biXPelsPerMeter: i32,
                pub biYPelsPerMeter: i32,
                pub biClrUsed: u32,
                pub biClrImportant: u32,
            }

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default)]
            pub struct BITMAPINFO {
                pub bmiHeader: BITMAPINFOHEADER,
                pub bmiColors: [RGBQUAD; 1],
            }

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default)]
            pub struct BITMAP {
                pub bmType: i32,
                pub bmWidth: i32,
                pub bmHeight: i32,
                pub bmWidthBytes: i32,
                pub bmPlanes: u16,
                pub bmBitsPixel: u16,
                pub bmBits: *mut c_void,
            }

            #[derive(Clone, Debug)]
            pub enum RgnShape {
                Rect(RECT),
                Polygon(Vec<POINT>),
            }

            static RGN_ID: AtomicIsize = AtomicIsize::new(1);

            fn rgn_store() -> &'static Mutex<HashMap<isize, RgnShape>> {
                static STORE: OnceLock<Mutex<HashMap<isize, RgnShape>>> = OnceLock::new();
                STORE.get_or_init(|| Mutex::new(HashMap::new()))
            }

            pub fn lookup_rgn(rgn: HRGN) -> Option<RgnShape> {
                rgn_store().lock().ok()?.get(&(rgn.0 as isize)).cloned()
            }

            pub type MONITORENUMPROC =
                Option<unsafe extern "system" fn(HMONITOR, HDC, *mut RECT, LPARAM) -> BOOL>;

            pub unsafe fn MonitorFromPoint(_pt: POINT, _flags: MONITOR_FROM_FLAGS) -> HMONITOR {
                #[cfg(target_os = "macos")]
                {
                    return crate::mac::monitor_from_point(_pt);
                }
                #[cfg(not(target_os = "macos"))]
                HMONITOR(1)
            }

            pub unsafe fn MonitorFromWindow(_hwnd: HWND, _flags: MONITOR_FROM_FLAGS) -> HMONITOR {
                #[cfg(target_os = "macos")]
                {
                    return crate::mac::monitor_from_window(_hwnd);
                }
                #[cfg(not(target_os = "macos"))]
                HMONITOR(1)
            }

            pub unsafe fn GetMonitorInfoW(mon: HMONITOR, lpmi: *mut MONITORINFO) -> BOOL {
                if lpmi.is_null() {
                    return BOOL(0);
                }
                #[cfg(target_os = "macos")]
                {
                    return BOOL::from(crate::mac::get_monitor_info(mon, &mut *lpmi));
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = mon;
                    (*lpmi).rcMonitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
                    (*lpmi).rcWork = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
                    (*lpmi).dwFlags = 1;
                    BOOL(1)
                }
            }

            /// Returns physical MacBook notch `(width, height)` in logical points, or `(0.0, 0.0)` when absent.
            pub unsafe fn GetMonitorNotch(mon: HMONITOR) -> (f32, f32) {
                #[cfg(target_os = "macos")]
                {
                    return crate::mac::get_monitor_notch(mon);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = mon;
                    (0.0, 0.0)
                }
            }

            pub unsafe fn EnumDisplayMonitors(
                _hdc: Option<HDC>,
                _clip: Option<*const RECT>,
                callback: MONITORENUMPROC,
                data: LPARAM,
            ) -> BOOL {
                let Some(cb) = callback else { return BOOL(0) };
                #[cfg(target_os = "macos")]
                {
                    for (mon, mut rc) in crate::mac::enum_monitors() {
                        if !cb(mon, HDC::default(), &mut rc, data).as_bool() {
                            break;
                        }
                    }
                    return BOOL(1);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let mut rc = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
                    cb(HMONITOR(1), HDC::default(), &mut rc, data)
                }
            }

            pub unsafe fn CreateRectRgn(left: i32, top: i32, right: i32, bottom: i32) -> HRGN {
                let id = RGN_ID.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut s) = rgn_store().lock() {
                    s.insert(id, RgnShape::Rect(RECT { left, top, right, bottom }));
                }
                HRGN(id as *mut c_void)
            }

            pub unsafe fn CreatePolygonRgn(
                pts: &[POINT],
                _mode: CREATE_POLYGON_RGN_MODE,
            ) -> HRGN {
                let id = RGN_ID.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut s) = rgn_store().lock() {
                    s.insert(id, RgnShape::Polygon(pts.to_vec()));
                }
                HRGN(id as *mut c_void)
            }

            pub unsafe fn SetWindowRgn(hwnd: HWND, hrgn: Option<HRGN>, _redraw: bool) -> i32 {
                #[cfg(target_os = "macos")]
                {
                    let shape = hrgn.and_then(lookup_rgn);
                    crate::mac::set_window_rgn(hwnd, shape);
                }
                if let Some(r) = hrgn {
                    if let Ok(mut s) = rgn_store().lock() {
                        s.remove(&(r.0 as isize));
                    }
                }
                let _ = hwnd;
                1
            }

            pub unsafe fn DeleteObject(obj: HGDIOBJ) -> BOOL {
                if !obj.0.is_null() {
                    if let Ok(mut s) = rgn_store().lock() {
                        s.remove(&(obj.0 as isize));
                    }
                }
                BOOL(1)
            }

            pub unsafe fn GetDC(_hwnd: Option<HWND>) -> HDC {
                HDC::default()
            }
            pub unsafe fn ReleaseDC(_hwnd: Option<HWND>, _hdc: HDC) -> i32 {
                1
            }
            pub unsafe fn CreateCompatibleDC(_hdc: Option<HDC>) -> HDC {
                HDC::default()
            }
            pub unsafe fn DeleteDC(_hdc: HDC) -> BOOL {
                BOOL(1)
            }
            pub unsafe fn CreateCompatibleBitmap(_hdc: HDC, _w: i32, _h: i32) -> HBITMAP {
                HBITMAP::default()
            }
            pub unsafe fn CreateDIBSection(
                _hdc: Option<HDC>,
                _pbmi: *const BITMAPINFO,
                _usage: u32,
                _bits: *mut *mut c_void,
                _sec: Option<crate::foundation::HANDLE>,
                _offset: u32,
            ) -> windows_core::Result<HBITMAP> {
                Ok(HBITMAP::default())
            }
            pub unsafe fn CreateSolidBrush(_color: crate::foundation::COLORREF) -> HBRUSH {
                HBRUSH::default()
            }
            pub unsafe fn SelectObject(_hdc: HDC, _obj: HGDIOBJ) -> HGDIOBJ {
                HGDIOBJ::default()
            }
            pub unsafe fn GetObjectW(_h: HGDIOBJ, _c: i32, _pv: Option<*mut c_void>) -> i32 {
                0
            }
        }

        pub mod Dwm {
            use std::ffi::c_void;
            use windows_core::{Result, BOOL};
            use crate::foundation::HWND;

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct DWMWINDOWATTRIBUTE(pub i32);
            pub const DWMWA_CLOAKED: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(14);
            pub const DWMWA_USE_HOSTBACKDROPBRUSH: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(17);

            pub unsafe fn DwmGetWindowAttribute(
                _hwnd: HWND,
                _attr: DWMWINDOWATTRIBUTE,
                pv: *mut c_void,
                cb: u32,
            ) -> Result<()> {
                if !pv.is_null() && cb >= 4 {
                    *(pv as *mut u32) = 0;
                }
                Ok(())
            }

            pub unsafe fn DwmSetWindowAttribute(
                _hwnd: HWND,
                _attr: DWMWINDOWATTRIBUTE,
                _pv: *const c_void,
                _cb: u32,
            ) -> Result<()> {
                let _ = BOOL(1);
                Ok(())
            }
        }

        pub mod Imaging {
            use std::ffi::c_void;
            use std::path::PathBuf;
            use std::sync::{Arc, Mutex};

            use windows_core::{CastFrom, Error, Interface, PCWSTR, Result, GUID};

            use crate::d2d::PixelSurface;
            use crate::foundation::E_FAIL;

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WICRect {
                pub X: i32,
                pub Y: i32,
                pub Width: i32,
                pub Height: i32,
            }

            pub const CLSID_WICImagingFactory: GUID =
                GUID::from_u128(0xCACAF262_9370_4615_A13B_9F5539DA4C0A);
            pub const GUID_WICPixelFormat32bppPBGRA: GUID =
                GUID::from_u128(0x6FDDC324_4E03_4BFE_B185_3D77768DC910);
            pub const GUID_WICPixelFormat32bppBGRA: GUID =
                GUID::from_u128(0x6FDDC324_4E03_4BFE_B185_3D77768DC90F);
            pub const GUID_ContainerFormatPng: GUID =
                GUID::from_u128(0x1B7CFAF4_713F_473C_BBCD_6137425FAEAF);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WICDecodeOptions(pub i32);
            pub const WICDecodeMetadataCacheOnDemand: WICDecodeOptions = WICDecodeOptions(0);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WICBitmapCreateCacheOption(pub i32);
            pub const WICBitmapCacheOnLoad: WICBitmapCreateCacheOption =
                WICBitmapCreateCacheOption(2);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WICBitmapDitherType(pub i32);
            pub const WICBitmapDitherTypeNone: WICBitmapDitherType = WICBitmapDitherType(0);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WICBitmapPaletteType(pub i32);
            pub const WICBitmapPaletteTypeCustom: WICBitmapPaletteType = WICBitmapPaletteType(0);
            pub const WICBitmapPaletteTypeMedianCut: WICBitmapPaletteType =
                WICBitmapPaletteType(1);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WICBitmapInterpolationMode(pub i32);
            pub const WICBitmapInterpolationModeFant: WICBitmapInterpolationMode =
                WICBitmapInterpolationMode(2);
            pub const WICBitmapInterpolationModeHighQualityCubic: WICBitmapInterpolationMode =
                WICBitmapInterpolationMode(3);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WICBitmapEncoderCacheOption(pub i32);
            pub const WICBitmapEncoderNoCache: WICBitmapEncoderCacheOption =
                WICBitmapEncoderCacheOption(2);

            #[derive(Clone, Default)]
            pub struct IWICImagingFactory;

            #[derive(Clone)]
            pub struct IWICBitmap(pub Arc<Mutex<PixelSurface>>);

            impl IWICBitmap {
                pub fn surface(&self) -> Arc<Mutex<PixelSurface>> {
                    self.0.clone()
                }
                pub unsafe fn CopyPixels(
                    &self,
                    rc: *const WICRect,
                    stride: u32,
                    buf: &mut [u8],
                ) -> Result<()> {
                    let s = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
                    let (rx, ry, rw, rh) = if !rc.is_null() {
                        let r = *rc;
                        (
                            r.X.max(0) as usize,
                            r.Y.max(0) as usize,
                            r.Width.max(0) as usize,
                            r.Height.max(0) as usize,
                        )
                    } else {
                        (0, 0, s.width as usize, s.height as usize)
                    };
                    let copy_w = rw.min((s.width as usize).saturating_sub(rx));
                    let copy_h = rh.min((s.height as usize).saturating_sub(ry));
                    let row_bytes = copy_w * 4;
                    let src_stride = (s.width as usize) * 4;
                    for y in 0..copy_h {
                        let dst_off = y * stride as usize;
                        let src_off = (ry + y) * src_stride + rx * 4;
                        if dst_off + row_bytes <= buf.len() && src_off + row_bytes <= s.pixels.len() {
                            buf[dst_off..dst_off + row_bytes]
                                .copy_from_slice(&s.pixels[src_off..src_off + row_bytes]);
                        }
                    }
                    Ok(())
                }
            }

            #[derive(Clone)]
            pub struct IWICBitmapSource(pub Arc<Mutex<PixelSurface>>);

            #[derive(Clone)]
            pub struct IWICBitmapFrameDecode(pub Arc<Mutex<PixelSurface>>);

            #[derive(Clone)]
            pub struct IWICBitmapScaler(pub Arc<Mutex<PixelSurface>>);

            #[derive(Clone)]
            pub struct IWICFormatConverter(pub Arc<Mutex<PixelSurface>>);

            #[derive(Clone)]
            pub struct IWICBitmapDecoder {
                pub frames: Vec<Arc<Mutex<PixelSurface>>>,
            }

            #[derive(Clone, Default)]
            pub struct IWICStream {
                pub path: Arc<Mutex<Option<PathBuf>>>,
                pub data: Arc<Mutex<Vec<u8>>>,
            }

            #[derive(Clone, Default)]
            pub struct IWICBitmapEncoder {
                pub stream: Arc<Mutex<Option<IWICStream>>>,
            }

            #[derive(Clone, Default)]
            pub struct IWICBitmapFrameEncode {
                pub stream: Arc<Mutex<Option<IWICStream>>>,
                pub width: Arc<Mutex<u32>>,
                pub height: Arc<Mutex<u32>>,
                pub pixels: Arc<Mutex<Vec<u8>>>,
            }

            impl Interface for IWICBitmap {
                fn as_raw(&self) -> *mut c_void {
                    Arc::as_ptr(&self.0) as *mut c_void
                }
            }
            impl Interface for IWICBitmapSource {
                fn as_raw(&self) -> *mut c_void {
                    Arc::as_ptr(&self.0) as *mut c_void
                }
            }
            impl Interface for IWICBitmapFrameDecode {
                fn as_raw(&self) -> *mut c_void {
                    Arc::as_ptr(&self.0) as *mut c_void
                }
            }
            impl Interface for IWICBitmapScaler {
                fn as_raw(&self) -> *mut c_void {
                    Arc::as_ptr(&self.0) as *mut c_void
                }
            }
            impl Interface for IWICFormatConverter {
                fn as_raw(&self) -> *mut c_void {
                    Arc::as_ptr(&self.0) as *mut c_void
                }
            }

            impl CastFrom<IWICBitmapSource> for IWICBitmap {
                fn cast_into(&self) -> Result<IWICBitmapSource> {
                    Ok(IWICBitmapSource(self.0.clone()))
                }
            }
            impl CastFrom<IWICBitmapSource> for IWICBitmapFrameDecode {
                fn cast_into(&self) -> Result<IWICBitmapSource> {
                    Ok(IWICBitmapSource(self.0.clone()))
                }
            }
            impl CastFrom<IWICBitmapSource> for IWICBitmapScaler {
                fn cast_into(&self) -> Result<IWICBitmapSource> {
                    Ok(IWICBitmapSource(self.0.clone()))
                }
            }
            impl CastFrom<IWICBitmapSource> for IWICFormatConverter {
                fn cast_into(&self) -> Result<IWICBitmapSource> {
                    Ok(IWICBitmapSource(self.0.clone()))
                }
            }

            pub trait AsWicSource {
                fn wic_surface(&self) -> Arc<Mutex<PixelSurface>>;
            }
            impl AsWicSource for IWICBitmapSource {
                fn wic_surface(&self) -> Arc<Mutex<PixelSurface>> {
                    self.0.clone()
                }
            }
            impl AsWicSource for IWICBitmapFrameDecode {
                fn wic_surface(&self) -> Arc<Mutex<PixelSurface>> {
                    self.0.clone()
                }
            }
            impl AsWicSource for IWICBitmapScaler {
                fn wic_surface(&self) -> Arc<Mutex<PixelSurface>> {
                    self.0.clone()
                }
            }
            impl AsWicSource for IWICFormatConverter {
                fn wic_surface(&self) -> Arc<Mutex<PixelSurface>> {
                    self.0.clone()
                }
            }

            macro_rules! impl_wic_source {
                ($t:ty) => {
                    impl $t {
                        pub unsafe fn GetSize(&self, w: &mut u32, h: &mut u32) -> Result<()> {
                            let s = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
                            *w = s.width;
                            *h = s.height;
                            Ok(())
                        }

                        pub unsafe fn CopyPixels(
                            &self,
                            rc: *const WICRect,
                            stride: u32,
                            buf: &mut [u8],
                        ) -> Result<()> {
                            IWICBitmap(self.0.clone()).CopyPixels(rc, stride, buf)
                        }
                    }
                };
            }

            impl_wic_source!(IWICBitmapSource);
            impl_wic_source!(IWICBitmapFrameDecode);
            impl_wic_source!(IWICBitmapScaler);
            impl_wic_source!(IWICFormatConverter);

            impl IWICImagingFactory {
                pub unsafe fn CreateBitmap(
                    &self,
                    w: u32,
                    h: u32,
                    _fmt: &GUID,
                    _cache: WICBitmapCreateCacheOption,
                ) -> Result<IWICBitmap> {
                    Ok(IWICBitmap(Arc::new(Mutex::new(PixelSurface::new(w, h)))))
                }

                pub unsafe fn CreateStream(&self) -> Result<IWICStream> {
                    Ok(IWICStream::default())
                }

                pub unsafe fn CreateEncoder(
                    &self,
                    _container: &GUID,
                    _vendor: *const GUID,
                ) -> Result<IWICBitmapEncoder> {
                    Ok(IWICBitmapEncoder::default())
                }

                pub unsafe fn CreateBitmapScaler(&self) -> Result<IWICBitmapScaler> {
                    Ok(IWICBitmapScaler(Arc::new(Mutex::new(PixelSurface::new(0, 0)))))
                }

                pub unsafe fn CreateFormatConverter(&self) -> Result<IWICFormatConverter> {
                    Ok(IWICFormatConverter(Arc::new(Mutex::new(PixelSurface::new(0, 0)))))
                }

                pub unsafe fn CreateDecoderFromStream(
                    &self,
                    stream: &IWICStream,
                    _vendor: *const GUID,
                    _opts: WICDecodeOptions,
                ) -> Result<IWICBitmapDecoder> {
                    let bytes = stream.data.lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
                    decode_bytes_to_decoder(&bytes)
                }

                pub unsafe fn CreateDecoderFromFilename(
                    &self,
                    filename: PCWSTR,
                    _vendor: Option<*const GUID>,
                    _access: u32,
                    _opts: WICDecodeOptions,
                ) -> Result<IWICBitmapDecoder> {
                    let p = filename.to_string()?;
                    let bytes = std::fs::read(&p).map_err(|_| Error::from_hresult(E_FAIL))?;
                    decode_bytes_to_decoder(&bytes)
                }
            }

            fn decode_bytes_to_decoder(bytes: &[u8]) -> Result<IWICBitmapDecoder> {
                #[cfg(target_os = "macos")]
                if let Some((w, h, px)) = unsafe { crate::mac::wic_decode_image(bytes) } {
                    return Ok(IWICBitmapDecoder {
                        frames: vec![Arc::new(Mutex::new(PixelSurface { width: w, height: h, pixels: px }))],
                    });
                }
                let _ = bytes;
                Err(Error::from_hresult(E_FAIL))
            }

            impl IWICBitmapDecoder {
                pub unsafe fn GetFrameCount(&self) -> Result<u32> {
                    Ok(self.frames.len() as u32)
                }
                pub unsafe fn GetFrame(&self, index: u32) -> Result<IWICBitmapFrameDecode> {
                    self.frames
                        .get(index as usize)
                        .cloned()
                        .map(IWICBitmapFrameDecode)
                        .ok_or_else(|| Error::from_hresult(E_FAIL))
                }
            }

            impl IWICBitmapScaler {
                pub unsafe fn Initialize(
                    &self,
                    src: &impl AsWicSource,
                    w: u32,
                    h: u32,
                    _mode: WICBitmapInterpolationMode,
                ) -> Result<()> {
                    let s = src.wic_surface().lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
                    let mut out = PixelSurface::new(w, h);
                    if s.width > 0 && s.height > 0 && w > 0 && h > 0 {
                        for y in 0..h {
                            let sy = ((y as f32 + 0.5) * s.height as f32 / h as f32)
                                .floor()
                                .clamp(0.0, (s.height - 1) as f32) as usize;
                            for x in 0..w {
                                let sx = ((x as f32 + 0.5) * s.width as f32 / w as f32)
                                    .floor()
                                    .clamp(0.0, (s.width - 1) as f32) as usize;
                                let si = (sy * s.width as usize + sx) * 4;
                                let di = (y as usize * w as usize + x as usize) * 4;
                                out.pixels[di..di + 4].copy_from_slice(&s.pixels[si..si + 4]);
                            }
                        }
                    }
                    *self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))? = out;
                    Ok(())
                }
            }

            impl IWICFormatConverter {
                pub unsafe fn Initialize(
                    &self,
                    src: &impl AsWicSource,
                    _dst_fmt: &GUID,
                    _dither: WICBitmapDitherType,
                    _palette: Option<&IWICBitmapSource>,
                    _alpha_thresh: f64,
                    _palette_trans: WICBitmapPaletteType,
                ) -> Result<()> {
                    let s = src.wic_surface().lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
                    *self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))? = s;
                    Ok(())
                }
            }

            impl IWICStream {
                pub unsafe fn InitializeFromFilename(
                    &self,
                    filename: PCWSTR,
                    _access: u32,
                ) -> Result<()> {
                    let p = PathBuf::from(filename.to_string()?);
                    *self.path.lock().map_err(|_| Error::from_hresult(E_FAIL))? = Some(p);
                    Ok(())
                }

                pub unsafe fn InitializeFromMemory(&self, buffer: &[u8]) -> Result<()> {
                    *self.data.lock().map_err(|_| Error::from_hresult(E_FAIL))? = buffer.to_vec();
                    Ok(())
                }
            }

            impl IWICBitmapEncoder {
                pub unsafe fn Initialize(
                    &self,
                    stream: &IWICStream,
                    _cache: WICBitmapEncoderCacheOption,
                ) -> Result<()> {
                    *self.stream.lock().map_err(|_| Error::from_hresult(E_FAIL))? =
                        Some(stream.clone());
                    Ok(())
                }

                pub unsafe fn CreateNewFrame(
                    &self,
                    frame: &mut Option<IWICBitmapFrameEncode>,
                    _props: *mut *mut c_void,
                ) -> Result<()> {
                    let st = self.stream.lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
                    *frame = Some(IWICBitmapFrameEncode {
                        stream: Arc::new(Mutex::new(st)),
                        width: Arc::new(Mutex::new(0)),
                        height: Arc::new(Mutex::new(0)),
                        pixels: Arc::new(Mutex::new(Vec::new())),
                    });
                    Ok(())
                }

                pub unsafe fn Commit(&self) -> Result<()> {
                    Ok(())
                }
            }

            impl IWICBitmapFrameEncode {
                pub unsafe fn Initialize(&self, _props: Option<*const c_void>) -> Result<()> {
                    Ok(())
                }
                pub unsafe fn SetSize(&self, w: u32, h: u32) -> Result<()> {
                    *self.width.lock().map_err(|_| Error::from_hresult(E_FAIL))? = w;
                    *self.height.lock().map_err(|_| Error::from_hresult(E_FAIL))? = h;
                    Ok(())
                }
                pub unsafe fn SetPixelFormat(&self, fmt: &mut GUID) -> Result<()> {
                    *fmt = GUID_WICPixelFormat32bppBGRA;
                    Ok(())
                }
                pub unsafe fn WritePixels(
                    &self,
                    line_count: u32,
                    stride: u32,
                    buffer: &[u8],
                ) -> Result<()> {
                    let w = *self.width.lock().map_err(|_| Error::from_hresult(E_FAIL))? as usize;
                    let mut out = self.pixels.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
                    for y in 0..line_count as usize {
                        let off = y * stride as usize;
                        if off + w * 4 <= buffer.len() {
                            out.extend_from_slice(&buffer[off..off + w * 4]);
                        }
                    }
                    Ok(())
                }
                pub unsafe fn Commit(&self) -> Result<()> {
                    let w = *self.width.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
                    let h = *self.height.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
                    let bgra = self.pixels.lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
                    let png = encode_bgra_png(w, h, &bgra);
                    if let Some(st) = self.stream.lock().ok().and_then(|g| g.clone()) {
                        if let Some(path) = st.path.lock().ok().and_then(|g| g.clone()) {
                            std::fs::write(path, png).map_err(|_| Error::from_hresult(E_FAIL))?;
                        } else if let Ok(mut d) = st.data.lock() {
                            *d = png;
                        }
                    }
                    Ok(())
                }
            }

            fn crc32(data: &[u8]) -> u32 {
                let mut c = 0xFFFF_FFFFu32;
                for &b in data {
                    c ^= b as u32;
                    for _ in 0..8 {
                        c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                    }
                }
                !c
            }

            fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
                out.extend_from_slice(&(body.len() as u32).to_be_bytes());
                out.extend_from_slice(tag);
                out.extend_from_slice(body);
                let mut crc_in = Vec::with_capacity(4 + body.len());
                crc_in.extend_from_slice(tag);
                crc_in.extend_from_slice(body);
                out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
            }

            fn encode_bgra_png(w: u32, h: u32, bgra: &[u8]) -> Vec<u8> {
                let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
                for y in 0..h as usize {
                    raw.push(0); // filter none
                    for x in 0..w as usize {
                        let i = (y * w as usize + x) * 4;
                        if i + 3 < bgra.len() {
                            raw.extend_from_slice(&[bgra[i + 2], bgra[i + 1], bgra[i], bgra[i + 3]]);
                        } else {
                            raw.extend_from_slice(&[0, 0, 0, 0]);
                        }
                    }
                }
                // Zlib stream with uncompressed DEFLATE blocks
                let mut zlib = vec![0x78u8, 0x01];
                let mut pos = 0;
                while pos < raw.len() || raw.is_empty() {
                    let take = (raw.len() - pos).min(65535);
                    let last = if pos + take >= raw.len() { 1u8 } else { 0u8 };
                    zlib.push(last);
                    let len = take as u16;
                    zlib.extend_from_slice(&len.to_le_bytes());
                    zlib.extend_from_slice(&(!len).to_le_bytes());
                    zlib.extend_from_slice(&raw[pos..pos + take]);
                    pos += take;
                    if raw.is_empty() {
                        break;
                    }
                }
                let (mut s1, mut s2) = (1u32, 0u32);
                for &b in &raw {
                    s1 = (s1 + b as u32) % 65521;
                    s2 = (s2 + s1) % 65521;
                }
                zlib.extend_from_slice(&((s2 << 16) | s1).to_be_bytes());

                let mut out = Vec::from(&b"\x89PNG\r\n\x1A\n"[..]);
                let mut ihdr = Vec::with_capacity(13);
                ihdr.extend_from_slice(&w.to_be_bytes());
                ihdr.extend_from_slice(&h.to_be_bytes());
                ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
                push_chunk(&mut out, b"IHDR", &ihdr);
                push_chunk(&mut out, b"IDAT", &zlib);
                push_chunk(&mut out, b"IEND", &[]);
                out
            }
        }
    }

    pub mod UI {
        pub mod HiDpi {
            use windows_core::Result;
            use crate::win32_ui::Win32::Graphics::Gdi::HMONITOR;

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct DPI_AWARENESS_CONTEXT(pub isize);
            pub const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: DPI_AWARENESS_CONTEXT =
                DPI_AWARENESS_CONTEXT(-4);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct MONITOR_DPI_TYPE(pub i32);
            pub const MDT_EFFECTIVE_DPI: MONITOR_DPI_TYPE = MONITOR_DPI_TYPE(0);

            pub unsafe fn SetProcessDpiAwarenessContext(_ctx: DPI_AWARENESS_CONTEXT) -> Result<()> {
                Ok(())
            }

            pub unsafe fn GetDpiForMonitor(
                mon: HMONITOR,
                _dpi_type: MONITOR_DPI_TYPE,
                dpi_x: *mut u32,
                dpi_y: *mut u32,
            ) -> Result<()> {
                #[cfg(target_os = "macos")]
                let dpi = crate::mac::get_monitor_dpi(mon);
                #[cfg(not(target_os = "macos"))]
                let dpi = {
                    let _ = mon;
                    96u32
                };
                if !dpi_x.is_null() {
                    *dpi_x = dpi;
                }
                if !dpi_y.is_null() {
                    *dpi_y = dpi;
                }
                Ok(())
            }
        }

        pub mod Controls {
            use windows_core::Result;
            use crate::foundation::HWND;

            pub const TME_LEAVE: u32 = 0x0000_0002;

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default)]
            pub struct TRACKMOUSEEVENT {
                pub cbSize: u32,
                pub dwFlags: u32,
                pub hwndTrack: HWND,
                pub dwHoverTime: u32,
            }

            pub unsafe fn TrackMouseEvent(_ev: *mut TRACKMOUSEEVENT) -> Result<()> {
                Ok(())
            }
        }

        pub mod Accessibility {
            use std::ffi::c_void;
            use windows_core::Result;
            use crate::foundation::HWND;

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct HWINEVENTHOOK(pub *mut c_void);

            pub type WINEVENTPROC = Option<
                unsafe extern "system" fn(HWINEVENTHOOK, u32, HWND, i32, i32, u32, u32),
            >;

            pub const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
            pub const EVENT_SYSTEM_CAPTURESTART: u32 = 0x0008;
            pub const EVENT_SYSTEM_CAPTUREEND: u32 = 0x0009;
            pub const EVENT_SYSTEM_MOVESIZEEND: u32 = 0x000B;
            pub const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
            pub const EVENT_SYSTEM_MINIMIZEEND: u32 = 0x0017;
            pub const EVENT_SYSTEM_DRAGDROPSTART: u32 = 0x000E;
            pub const EVENT_SYSTEM_DRAGDROPEND: u32 = 0x000F;
            pub const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
            pub const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;

            pub unsafe fn SetWinEventHook(
                _min: u32,
                _max: u32,
                _hmod: Option<crate::foundation::HMODULE>,
                _proc: WINEVENTPROC,
                _pid: u32,
                _tid: u32,
                _flags: u32,
            ) -> HWINEVENTHOOK {
                HWINEVENTHOOK::default()
            }

            pub unsafe fn UnhookWinEvent(_hook: HWINEVENTHOOK) -> Result<()> {
                Ok(())
            }
        }

        pub mod Input {
            pub mod KeyboardAndMouse {
                use std::ffi::c_void;
                use std::ops::{BitAnd, BitOr};
                use windows_core::Result;
                use crate::foundation::HWND;

                #[repr(transparent)]
                #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
                pub struct VIRTUAL_KEY(pub u16);

                pub const VK_LBUTTON: VIRTUAL_KEY = VIRTUAL_KEY(0x01);
                pub const VK_RBUTTON: VIRTUAL_KEY = VIRTUAL_KEY(0x02);
                pub const VK_BACK: VIRTUAL_KEY = VIRTUAL_KEY(0x08);
                pub const VK_TAB: VIRTUAL_KEY = VIRTUAL_KEY(0x09);
                pub const VK_RETURN: VIRTUAL_KEY = VIRTUAL_KEY(0x0D);
                pub const VK_SHIFT: VIRTUAL_KEY = VIRTUAL_KEY(0x10);
                pub const VK_CONTROL: VIRTUAL_KEY = VIRTUAL_KEY(0x11);
                pub const VK_MENU: VIRTUAL_KEY = VIRTUAL_KEY(0x12);
                pub const VK_ESCAPE: VIRTUAL_KEY = VIRTUAL_KEY(0x1B);
                pub const VK_SPACE: VIRTUAL_KEY = VIRTUAL_KEY(0x20);
                pub const VK_END: VIRTUAL_KEY = VIRTUAL_KEY(0x23);
                pub const VK_HOME: VIRTUAL_KEY = VIRTUAL_KEY(0x24);
                pub const VK_LEFT: VIRTUAL_KEY = VIRTUAL_KEY(0x25);
                pub const VK_UP: VIRTUAL_KEY = VIRTUAL_KEY(0x26);
                pub const VK_RIGHT: VIRTUAL_KEY = VIRTUAL_KEY(0x27);
                pub const VK_DOWN: VIRTUAL_KEY = VIRTUAL_KEY(0x28);
                pub const VK_SNAPSHOT: VIRTUAL_KEY = VIRTUAL_KEY(0x2C);
                pub const VK_DELETE: VIRTUAL_KEY = VIRTUAL_KEY(0x2E);
                pub const VK_LWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5B);
                pub const VK_RWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5C);
                pub const VK_MEDIA_NEXT_TRACK: VIRTUAL_KEY = VIRTUAL_KEY(0xB0);
                pub const VK_MEDIA_PREV_TRACK: VIRTUAL_KEY = VIRTUAL_KEY(0xB1);
                pub const VK_MEDIA_PLAY_PAUSE: VIRTUAL_KEY = VIRTUAL_KEY(0xB3);
                pub const VK_VOLUME_MUTE: VIRTUAL_KEY = VIRTUAL_KEY(0xAD);
                pub const VK_VOLUME_DOWN: VIRTUAL_KEY = VIRTUAL_KEY(0xAE);
                pub const VK_VOLUME_UP: VIRTUAL_KEY = VIRTUAL_KEY(0xAF);

                #[repr(transparent)]
                #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
                pub struct HOT_KEY_MODIFIERS(pub u32);
                pub const MOD_ALT: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(0x0001);
                pub const MOD_CONTROL: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(0x0002);
                pub const MOD_SHIFT: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(0x0004);
                pub const MOD_WIN: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(0x0008);
                pub const MOD_NOREPEAT: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(0x4000);

                impl BitOr for HOT_KEY_MODIFIERS {
                    type Output = Self;
                    fn bitor(self, rhs: Self) -> Self {
                        Self(self.0 | rhs.0)
                    }
                }
                impl BitAnd for HOT_KEY_MODIFIERS {
                    type Output = Self;
                    fn bitand(self, rhs: Self) -> Self {
                        Self(self.0 & rhs.0)
                    }
                }

                #[repr(transparent)]
                #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
                pub struct HKL(pub *mut c_void);

                pub unsafe fn RegisterHotKey(
                    _hwnd: Option<HWND>,
                    _id: i32,
                    _mods: HOT_KEY_MODIFIERS,
                    _vk: u32,
                ) -> Result<()> {
                    #[cfg(target_os = "macos")]
                    {
                        crate::mac::register_hotkey(_hwnd, _id, _mods.0, _vk);
                    }
                    Ok(())
                }

                pub unsafe fn UnregisterHotKey(_hwnd: Option<HWND>, _id: i32) -> Result<()> {
                    #[cfg(target_os = "macos")]
                    {
                        crate::mac::unregister_hotkey(_hwnd, _id);
                    }
                    Ok(())
                }

                pub unsafe fn GetAsyncKeyState(_vkey: i32) -> i16 {
                    #[cfg(target_os = "macos")]
                    {
                        return crate::mac::get_key_state(_vkey);
                    }
                    #[cfg(not(target_os = "macos"))]
                    0
                }

                pub unsafe fn GetKeyState(_vkey: i32) -> i16 {
                    #[cfg(target_os = "macos")]
                    {
                        return crate::mac::get_key_state(_vkey);
                    }
                    #[cfg(not(target_os = "macos"))]
                    0
                }

                pub unsafe fn GetKeyboardLayout(_id_thread: u32) -> HKL {
                    HKL::default()
                }

                pub use super::super::Controls::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};

                pub unsafe fn SetFocus(hwnd: Option<HWND>) -> Result<HWND> {
                    #[cfg(target_os = "macos")]
                    if let Some(h) = hwnd {
                        crate::mac::activate_window(h);
                    }
                    Ok(hwnd.unwrap_or_default())
                }

                pub unsafe fn SetCapture(hwnd: HWND) -> HWND {
                    hwnd
                }

                pub unsafe fn ReleaseCapture() -> Result<()> {
                    Ok(())
                }
            }

            pub mod Ime {
                use std::ffi::c_void;
                use windows_core::BOOL;
                use crate::foundation::{HWND, POINT, RECT};

                #[repr(transparent)]
                #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
                pub struct HIMC(pub *mut c_void);

                pub const CFS_POINT: u32 = 0x0002;
                pub const CFS_FORCE_POSITION: u32 = 0x0020;

                #[repr(C)]
                #[derive(Clone, Copy, Debug, Default)]
                pub struct COMPOSITIONFORM {
                    pub dwStyle: u32,
                    pub ptCurrentPos: POINT,
                    pub rcArea: RECT,
                }

                #[repr(C)]
                #[derive(Clone, Copy, Debug, Default)]
                pub struct CANDIDATEFORM {
                    pub dwIndex: u32,
                    pub dwStyle: u32,
                    pub ptCurrentPos: POINT,
                    pub rcArea: RECT,
                }

                pub unsafe fn ImmGetContext(_hwnd: HWND) -> HIMC {
                    HIMC::default()
                }
                pub unsafe fn ImmReleaseContext(_hwnd: HWND, _himc: HIMC) -> BOOL {
                    BOOL(1)
                }
                pub unsafe fn ImmSetCompositionWindow(
                    _himc: HIMC,
                    _comp: *const COMPOSITIONFORM,
                ) -> BOOL {
                    BOOL(1)
                }
                pub unsafe fn ImmSetCandidateWindow(
                    _himc: HIMC,
                    _cand: *const CANDIDATEFORM,
                ) -> BOOL {
                    BOOL(1)
                }
            }
        }

        pub mod Shell {
            use windows_core::{PCWSTR, Result};
            use crate::foundation::{HINSTANCE, HWND};
            use crate::win32_ui::Win32::UI::WindowsAndMessaging::{HICON, SHOW_WINDOW_CMD};

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct NOTIFY_ICON_MESSAGE(pub u32);
            pub const NIM_ADD: NOTIFY_ICON_MESSAGE = NOTIFY_ICON_MESSAGE(0);
            pub const NIM_MODIFY: NOTIFY_ICON_MESSAGE = NOTIFY_ICON_MESSAGE(1);
            pub const NIM_DELETE: NOTIFY_ICON_MESSAGE = NOTIFY_ICON_MESSAGE(2);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct NOTIFY_ICON_INFOTIP_FLAGS(pub u32);

            pub const NIF_MESSAGE: u32 = 1;
            pub const NIF_ICON: u32 = 2;
            pub const NIF_TIP: u32 = 4;

            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct NOTIFYICONDATAW {
                pub cbSize: u32,
                pub hWnd: HWND,
                pub uID: u32,
                pub uFlags: u32,
                pub uCallbackMessage: u32,
                pub hIcon: HICON,
                pub szTip: [u16; 128],
                pub dwState: u32,
                pub dwStateMask: u32,
                pub szInfo: [u16; 256],
                pub Anonymous: u32,
                pub szInfoTitle: [u16; 64],
                pub dwInfoFlags: NOTIFY_ICON_INFOTIP_FLAGS,
                pub guidItem: windows_core::GUID,
                pub hBalloonIcon: HICON,
            }

            impl Default for NOTIFYICONDATAW {
                fn default() -> Self {
                    Self {
                        cbSize: std::mem::size_of::<Self>() as u32,
                        hWnd: HWND::default(),
                        uID: 0,
                        uFlags: 0,
                        uCallbackMessage: 0,
                        hIcon: HICON::default(),
                        szTip: [0; 128],
                        dwState: 0,
                        dwStateMask: 0,
                        szInfo: [0; 256],
                        Anonymous: 0,
                        szInfoTitle: [0; 64],
                        dwInfoFlags: NOTIFY_ICON_INFOTIP_FLAGS(0),
                        guidItem: windows_core::GUID::default(),
                        hBalloonIcon: HICON::default(),
                    }
                }
            }

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct QUERY_USER_NOTIFICATION_STATE(pub i32);
            pub const QUNS_NOT_PRESENT: QUERY_USER_NOTIFICATION_STATE =
                QUERY_USER_NOTIFICATION_STATE(1);
            pub const QUNS_BUSY: QUERY_USER_NOTIFICATION_STATE = QUERY_USER_NOTIFICATION_STATE(2);
            pub const QUNS_RUNNING_D3D_FULL_SCREEN: QUERY_USER_NOTIFICATION_STATE =
                QUERY_USER_NOTIFICATION_STATE(3);
            pub const QUNS_PRESENTATION_MODE: QUERY_USER_NOTIFICATION_STATE =
                QUERY_USER_NOTIFICATION_STATE(4);
            pub const QUNS_ACCEPTS_NOTIFICATIONS: QUERY_USER_NOTIFICATION_STATE =
                QUERY_USER_NOTIFICATION_STATE(5);

            pub unsafe fn Shell_NotifyIconW(
                _msg: NOTIFY_ICON_MESSAGE,
                _data: *const NOTIFYICONDATAW,
            ) -> windows_core::BOOL {
                #[cfg(target_os = "macos")]
                if !_data.is_null() {
                    crate::mac::tray_notify(_msg.0, &*_data);
                }
                windows_core::BOOL(1)
            }

            pub unsafe fn ShellExecuteW(
                _hwnd: Option<HWND>,
                _verb: PCWSTR,
                file: PCWSTR,
                params: PCWSTR,
                _dir: PCWSTR,
                _show: SHOW_WINDOW_CMD,
            ) -> HINSTANCE {
                let target = file.to_string().unwrap_or_default();
                let args = params.to_string().unwrap_or_default();
                #[cfg(target_os = "macos")]
                crate::mac::shell_open(&target, &args);
                let _ = (target, args);
                HINSTANCE(33 as *mut std::ffi::c_void)
            }

            pub unsafe fn SHQueryUserNotificationState() -> Result<QUERY_USER_NOTIFICATION_STATE> {
                #[cfg(target_os = "macos")]
                {
                    if crate::mac::is_fullscreen_app_active() {
                        return Ok(QUNS_RUNNING_D3D_FULL_SCREEN);
                    }
                }
                Ok(QUNS_ACCEPTS_NOTIFICATIONS)
            }

            pub unsafe fn SHCreateMemStream(
                pinit: Option<&[u8]>,
            ) -> Option<crate::win32_ui::Win32::Graphics::Imaging::IWICStream> {
                let s = crate::win32_ui::Win32::Graphics::Imaging::IWICStream::default();
                if let Some(bytes) = pinit {
                    s.InitializeFromMemory(bytes).ok()?;
                }
                Some(s)
            }
        }

        pub mod WindowsAndMessaging {
            use std::ffi::c_void;
            use std::ops::{BitAnd, BitOr};

            use windows_core::{Error, PCWSTR, Result, BOOL};

            use crate::foundation::{E_FAIL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub struct HICON(pub *mut c_void);
            impl Default for HICON {
                fn default() -> Self {
                    Self(std::ptr::null_mut())
                }
            }
            impl HICON {
                pub fn is_invalid(self) -> bool {
                    self.0.is_null()
                }
            }

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct HCURSOR(pub *mut c_void);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub struct HMENU(pub *mut c_void);
            impl Default for HMENU {
                fn default() -> Self {
                    Self(std::ptr::null_mut())
                }
            }
            impl HMENU {
                pub fn is_invalid(self) -> bool {
                    self.0.is_null()
                }
            }

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct HHOOK(pub *mut c_void);

            pub type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
            pub type WNDENUMPROC = Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>;
            pub type TIMERPROC = Option<unsafe extern "system" fn(HWND, u32, usize, u32)>;
            pub type HOOKPROC = Option<unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT>;

            pub use super::Accessibility::{
                EVENT_SYSTEM_CAPTUREEND, EVENT_SYSTEM_CAPTURESTART, EVENT_SYSTEM_DRAGDROPEND,
                EVENT_SYSTEM_DRAGDROPSTART, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
                EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MOVESIZEEND, WINEVENT_OUTOFCONTEXT,
                WINEVENT_SKIPOWNPROCESS,
            };
            pub use super::Controls::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};

            pub const WA_INACTIVE: u32 = 0;
            pub const MA_NOACTIVATE: u32 = 3;
            pub const IDI_APPLICATION: PCWSTR = PCWSTR(32512 as *const u16);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct GET_WINDOW_CMD(pub u32);
            pub const GW_HWNDNEXT: GET_WINDOW_CMD = GET_WINDOW_CMD(2);
            pub const GW_HWNDPREV: GET_WINDOW_CMD = GET_WINDOW_CMD(3);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WINDOW_STYLE(pub u32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WINDOW_EX_STYLE(pub u32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct SHOW_WINDOW_CMD(pub i32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct SET_WINDOW_POS_FLAGS(pub u32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct PEEK_MESSAGE_REMOVE_TYPE(pub u32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct TRACK_POPUP_MENU_FLAGS(pub u32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct MENU_ITEM_FLAGS(pub u32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct SYSTEM_PARAMETERS_INFO_ACTION(pub u32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct WINDOW_LONG_PTR_INDEX(pub i32);
            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct LAYERED_WINDOW_ATTRIBUTES_FLAGS(pub u32);

            macro_rules! impl_bitops {
                ($($t:ty),*) => {
                    $(
                        impl BitOr for $t {
                            type Output = Self;
                            fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
                        }
                        impl BitAnd for $t {
                            type Output = Self;
                            fn bitand(self, rhs: Self) -> Self { Self(self.0 & rhs.0) }
                        }
                    )*
                };
            }

            impl_bitops!(
                WINDOW_STYLE,
                WINDOW_EX_STYLE,
                SET_WINDOW_POS_FLAGS,
                PEEK_MESSAGE_REMOVE_TYPE,
                TRACK_POPUP_MENU_FLAGS,
                MENU_ITEM_FLAGS,
                LAYERED_WINDOW_ATTRIBUTES_FLAGS
            );

            pub const WS_POPUP: WINDOW_STYLE = WINDOW_STYLE(0x8000_0000);
            pub const WS_CAPTION: WINDOW_STYLE = WINDOW_STYLE(0x00C0_0000);
            pub const WS_MAXIMIZE: WINDOW_STYLE = WINDOW_STYLE(0x0100_0000);

            pub const WS_EX_TRANSPARENT: WINDOW_EX_STYLE = WINDOW_EX_STYLE(0x0000_0020);
            pub const WS_EX_TOOLWINDOW: WINDOW_EX_STYLE = WINDOW_EX_STYLE(0x0000_0080);
            pub const WS_EX_TOPMOST: WINDOW_EX_STYLE = WINDOW_EX_STYLE(0x0000_0008);
            pub const WS_EX_LAYERED: WINDOW_EX_STYLE = WINDOW_EX_STYLE(0x0008_0000);
            pub const WS_EX_NOACTIVATE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(0x0800_0000);
            pub const WS_EX_NOREDIRECTIONBITMAP: WINDOW_EX_STYLE = WINDOW_EX_STYLE(0x0020_0000);

            pub const SW_HIDE: SHOW_WINDOW_CMD = SHOW_WINDOW_CMD(0);
            pub const SW_SHOWNORMAL: SHOW_WINDOW_CMD = SHOW_WINDOW_CMD(1);
            pub const SW_SHOWNOACTIVATE: SHOW_WINDOW_CMD = SHOW_WINDOW_CMD(4);

            pub const SWP_NOSIZE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(0x0001);
            pub const SWP_NOMOVE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(0x0002);
            pub const SWP_NOACTIVATE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(0x0010);
            pub const SWP_SHOWWINDOW: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(0x0040);

            pub const HWND_TOPMOST: HWND = HWND(-1isize as *mut c_void);

            pub const PM_REMOVE: PEEK_MESSAGE_REMOVE_TYPE = PEEK_MESSAGE_REMOVE_TYPE(0x0001);

            pub const TPM_RIGHTBUTTON: TRACK_POPUP_MENU_FLAGS = TRACK_POPUP_MENU_FLAGS(0x0002);
            pub const TPM_BOTTOMALIGN: TRACK_POPUP_MENU_FLAGS = TRACK_POPUP_MENU_FLAGS(0x0020);
            pub const TPM_RETURNCMD: TRACK_POPUP_MENU_FLAGS = TRACK_POPUP_MENU_FLAGS(0x0100);

            pub const MF_STRING: MENU_ITEM_FLAGS = MENU_ITEM_FLAGS(0x0000);
            pub const MF_CHECKED: MENU_ITEM_FLAGS = MENU_ITEM_FLAGS(0x0008);
            pub const MF_SEPARATOR: MENU_ITEM_FLAGS = MENU_ITEM_FLAGS(0x0800);

            pub const LWA_ALPHA: LAYERED_WINDOW_ATTRIBUTES_FLAGS = LAYERED_WINDOW_ATTRIBUTES_FLAGS(2);

            pub const GWL_STYLE: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(-16);
            pub const GWL_EXSTYLE: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(-20);
            pub const GWLP_USERDATA: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(-21);

            pub const IDC_ARROW: PCWSTR = PCWSTR(32512usize as *const u16);
            pub const CW_USEDEFAULT: i32 = -2147483648;
            pub const HTTRANSPARENT: i32 = -1;
            pub const WH_KEYBOARD_LL: i32 = 13;
            pub const DEVICE_NOTIFY_WINDOW_HANDLE: u32 = 0;

            pub const SPI_SETDESKWALLPAPER: SYSTEM_PARAMETERS_INFO_ACTION =
                SYSTEM_PARAMETERS_INFO_ACTION(0x0014);
            pub const SPI_SETCLIENTAREAANIMATION: SYSTEM_PARAMETERS_INFO_ACTION =
                SYSTEM_PARAMETERS_INFO_ACTION(0x1043);
            pub const SPI_GETCLIENTAREAANIMATION: SYSTEM_PARAMETERS_INFO_ACTION =
                SYSTEM_PARAMETERS_INFO_ACTION(0x1042);

            // Window Messages
            pub const WM_NULL: u32 = 0x0000;
            pub const WM_DESTROY: u32 = 0x0002;
            pub const WM_ACTIVATE: u32 = 0x0006;
            pub const WM_KILLFOCUS: u32 = 0x0008;
            pub const WM_QUERYENDSESSION: u32 = 0x0011;
            pub const WM_QUIT: u32 = 0x0012;
            pub const WM_ERASEBKGND: u32 = 0x0014;
            pub const WM_SYSCOLORCHANGE: u32 = 0x0015;
            pub const WM_ENDSESSION: u32 = 0x0016;
            pub const WM_SETTINGCHANGE: u32 = 0x001A;
            pub const WM_MOUSEACTIVATE: u32 = 0x0021;
            pub const WM_GETOBJECT: u32 = 0x003D;
            pub const WM_DISPLAYCHANGE: u32 = 0x007E;
            pub const WM_NCHITTEST: u32 = 0x0084;
            pub const WM_KEYDOWN: u32 = 0x0100;
            pub const WM_CHAR: u32 = 0x0102;
            pub const WM_SYSKEYDOWN: u32 = 0x0104;
            pub const WM_TIMER: u32 = 0x0113;
            pub const WM_MOUSEMOVE: u32 = 0x0200;
            pub const WM_LBUTTONDOWN: u32 = 0x0201;
            pub const WM_LBUTTONUP: u32 = 0x0202;
            pub const WM_RBUTTONUP: u32 = 0x0205;
            pub const WM_MOUSEWHEEL: u32 = 0x020A;
            pub const WM_POWERBROADCAST: u32 = 0x0218;
            pub const WM_CAPTURECHANGED: u32 = 0x0215;
            pub const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
            pub const WM_MOUSELEAVE: u32 = 0x02A3;
            pub const WM_DPICHANGED: u32 = 0x02E0;
            pub const WM_HOTKEY: u32 = 0x0312;
            pub const WM_THEMECHANGED: u32 = 0x031A;
            pub const WM_DWMCOMPOSITIONCHANGED: u32 = 0x031E;
            pub const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
            pub const WM_CLIPBOARDUPDATE: u32 = 0x031D;
            pub const WM_APP: u32 = 0x8000;

            pub const PBT_APMRESUMESUSPEND: u32 = 0x0007;
            pub const PBT_APMPOWERSTATUSCHANGE: u32 = 0x000A;
            pub const PBT_APMRESUMEAUTOMATIC: u32 = 0x0012;
            pub const PBT_POWERSETTINGCHANGE: u32 = 0x8013;

            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct WNDCLASSEXW {
                pub cbSize: u32,
                pub style: u32,
                pub lpfnWndProc: WNDPROC,
                pub cbClsExtra: i32,
                pub cbWndExtra: i32,
                pub hInstance: HINSTANCE,
                pub hIcon: HICON,
                pub hCursor: HCURSOR,
                pub hbrBackground: crate::win32_ui::Win32::Graphics::Gdi::HBRUSH,
                pub lpszMenuName: PCWSTR,
                pub lpszClassName: PCWSTR,
                pub hIconSm: HICON,
            }

            impl Default for WNDCLASSEXW {
                fn default() -> Self {
                    Self {
                        cbSize: std::mem::size_of::<Self>() as u32,
                        style: 0,
                        lpfnWndProc: None,
                        cbClsExtra: 0,
                        cbWndExtra: 0,
                        hInstance: HINSTANCE::default(),
                        hIcon: HICON::default(),
                        hCursor: HCURSOR::default(),
                        hbrBackground: crate::win32_ui::Win32::Graphics::Gdi::HBRUSH::default(),
                        lpszMenuName: PCWSTR::null(),
                        lpszClassName: PCWSTR::null(),
                        hIconSm: HICON::default(),
                    }
                }
            }

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default)]
            pub struct MSG {
                pub hwnd: HWND,
                pub message: u32,
                pub wParam: WPARAM,
                pub lParam: LPARAM,
                pub time: u32,
                pub pt: POINT,
            }
            unsafe impl Send for MSG {}
            unsafe impl Sync for MSG {}

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default)]
            pub struct ICONINFO {
                pub fIcon: BOOL,
                pub xHotspot: u32,
                pub yHotspot: u32,
                pub hbmMask: crate::win32_ui::Win32::Graphics::Gdi::HBITMAP,
                pub hbmColor: crate::win32_ui::Win32::Graphics::Gdi::HBITMAP,
            }

            #[repr(C)]
            #[derive(Clone, Copy, Debug, Default)]
            pub struct KBDLLHOOKSTRUCT {
                pub vkCode: u32,
                pub scanCode: u32,
                pub flags: u32,
                pub time: u32,
                pub dwExtraInfo: usize,
            }

            // Software class & message queue for non-macOS tests and macOS integration
            use std::collections::{HashMap, VecDeque};
            use std::sync::{Mutex, OnceLock};

            static CLASSES: OnceLock<Mutex<HashMap<String, WNDPROC>>> = OnceLock::new();
            static QUEUE: OnceLock<Mutex<VecDeque<MSG>>> = OnceLock::new();
            static WIN_PROCS: OnceLock<Mutex<HashMap<usize, WNDPROC>>> = OnceLock::new();
            static NEXT_HWND: std::sync::atomic::AtomicUsize =
                std::sync::atomic::AtomicUsize::new(0x1000);

            fn classes() -> &'static Mutex<HashMap<String, WNDPROC>> {
                CLASSES.get_or_init(|| Mutex::new(HashMap::new()))
            }
            pub fn queue() -> &'static Mutex<VecDeque<MSG>> {
                QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
            }
            pub fn win_procs() -> &'static Mutex<HashMap<usize, WNDPROC>> {
                WIN_PROCS.get_or_init(|| Mutex::new(HashMap::new()))
            }

            pub unsafe fn RegisterClassExW(wc: *const WNDCLASSEXW) -> u16 {
                if wc.is_null() {
                    return 0;
                }
                let name = (*wc).lpszClassName.to_string().unwrap_or_default();
                if let Ok(mut c) = classes().lock() {
                    c.insert(name, (*wc).lpfnWndProc);
                }
                1
            }

            pub unsafe fn CreateWindowExW(
                ex_style: WINDOW_EX_STYLE,
                class_name: PCWSTR,
                title: PCWSTR,
                style: WINDOW_STYLE,
                x: i32,
                y: i32,
                w: i32,
                h: i32,
                _parent: Option<HWND>,
                _menu: Option<HMENU>,
                _inst: Option<HINSTANCE>,
                _param: Option<*const c_void>,
            ) -> Result<HWND> {
                let cname = class_name.to_string().unwrap_or_default();
                let tname = title.to_string().unwrap_or_default();
                let proc_fn = classes().lock().ok().and_then(|c| c.get(&cname).copied().flatten());
                #[cfg(target_os = "macos")]
                {
                    let hwnd = crate::mac::create_window(&cname, &tname, ex_style.0, style.0, x, y, w, h, proc_fn);
                    if let Ok(mut wp) = win_procs().lock() {
                        wp.insert(hwnd.0 as usize, proc_fn);
                    }
                    return Ok(hwnd);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (ex_style, style, x, y, w, h, tname);
                    let id = NEXT_HWND.fetch_add(16, std::sync::atomic::Ordering::Relaxed);
                    let hwnd = HWND(id as *mut c_void);
                    if let Ok(mut wp) = win_procs().lock() {
                        wp.insert(id, proc_fn);
                    }
                    Ok(hwnd)
                }
            }

            pub unsafe fn DestroyWindow(hwnd: HWND) -> Result<()> {
                #[cfg(target_os = "macos")]
                crate::mac::destroy_window(hwnd);
                if let Ok(mut wp) = win_procs().lock() {
                    wp.remove(&(hwnd.0 as usize));
                }
                Ok(())
            }

            pub unsafe fn ShowWindow(hwnd: HWND, cmd: SHOW_WINDOW_CMD) -> BOOL {
                #[cfg(target_os = "macos")]
                crate::mac::show_window(hwnd, cmd.0);
                let _ = (hwnd, cmd);
                BOOL(1)
            }

            pub unsafe fn SetWindowPos(
                hwnd: HWND,
                _insert_after: Option<HWND>,
                x: i32,
                y: i32,
                cx: i32,
                cy: i32,
                flags: SET_WINDOW_POS_FLAGS,
            ) -> Result<()> {
                #[cfg(target_os = "macos")]
                crate::mac::set_window_pos(hwnd, x, y, cx, cy, flags.0);
                let _ = (hwnd, x, y, cx, cy, flags);
                Ok(())
            }

            pub unsafe fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> Result<()> {
                if rect.is_null() {
                    return Err(Error::from_hresult(E_FAIL));
                }
                #[cfg(target_os = "macos")]
                if crate::mac::get_window_rect(hwnd, &mut *rect) {
                    return Ok(());
                }
                let _ = hwnd;
                *rect = RECT { left: 0, top: 0, right: 800, bottom: 530 };
                Ok(())
            }

            pub unsafe fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> Result<()> {
                let mut r = RECT::default();
                GetWindowRect(hwnd, &mut r)?;
                *rect = RECT { left: 0, top: 0, right: r.right - r.left, bottom: r.bottom - r.top };
                Ok(())
            }

            pub unsafe fn GetCursorPos(pt: *mut POINT) -> Result<()> {
                if pt.is_null() {
                    return Err(Error::from_hresult(E_FAIL));
                }
                #[cfg(target_os = "macos")]
                {
                    *pt = crate::mac::get_cursor_pos();
                    return Ok(());
                }
                #[cfg(not(target_os = "macos"))]
                {
                    *pt = POINT::default();
                    Ok(())
                }
            }

            pub unsafe fn PostMessageW(
                hwnd: Option<HWND>,
                msg: u32,
                wp: WPARAM,
                lp: LPARAM,
            ) -> Result<()> {
                #[cfg(target_os = "macos")]
                {
                    crate::mac::post_message(hwnd, msg, wp, lp);
                    return Ok(());
                }
                #[cfg(not(target_os = "macos"))]
                {
                    if let Ok(mut q) = queue().lock() {
                        q.push_back(MSG {
                            hwnd: hwnd.unwrap_or_default(),
                            message: msg,
                            wParam: wp,
                            lParam: lp,
                            time: 0,
                            pt: POINT::default(),
                        });
                    }
                    Ok(())
                }
            }

            pub unsafe fn SendMessageW(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
                let p = win_procs().lock().ok().and_then(|m| m.get(&(hwnd.0 as usize)).copied().flatten());
                if let Some(f) = p {
                    f(hwnd, msg, wp, lp)
                } else {
                    LRESULT(0)
                }
            }

            pub unsafe fn PostQuitMessage(exit_code: i32) {
                #[cfg(target_os = "macos")]
                crate::mac::post_quit(exit_code);
                let _ = PostMessageW(None, WM_QUIT, WPARAM(exit_code as usize), LPARAM(0));
            }

            pub unsafe fn DefWindowProcW(
                _hwnd: HWND,
                _msg: u32,
                _wp: WPARAM,
                _lp: LPARAM,
            ) -> LRESULT {
                LRESULT(0)
            }

            pub unsafe fn GetMessageW(
                msg: *mut MSG,
                _hwnd: Option<HWND>,
                _min: u32,
                _max: u32,
            ) -> BOOL {
                if msg.is_null() {
                    return BOOL(0);
                }
                #[cfg(target_os = "macos")]
                {
                    if let Some(m) = crate::mac::next_message(true) {
                        *msg = m;
                        return BOOL(if m.message == WM_QUIT { 0 } else { 1 });
                    }
                    return BOOL(0);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    if let Ok(mut q) = queue().lock() {
                        if let Some(m) = q.pop_front() {
                            *msg = m;
                            return BOOL(if m.message == WM_QUIT { 0 } else { 1 });
                        }
                    }
                    BOOL(0)
                }
            }

            pub unsafe fn PeekMessageW(
                msg: *mut MSG,
                _hwnd: Option<HWND>,
                _min: u32,
                _max: u32,
                _remove: PEEK_MESSAGE_REMOVE_TYPE,
            ) -> BOOL {
                if msg.is_null() {
                    return BOOL(0);
                }
                #[cfg(target_os = "macos")]
                {
                    if let Some(m) = crate::mac::next_message(false) {
                        *msg = m;
                        return BOOL(1);
                    }
                    return BOOL(0);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    if let Ok(mut q) = queue().lock() {
                        let popped = if (_remove.0 & PM_REMOVE.0) != 0 {
                            q.pop_front()
                        } else {
                            q.front().copied()
                        };
                        if let Some(m) = popped {
                            *msg = m;
                            return BOOL(1);
                        }
                    }
                    BOOL(0)
                }
            }

            pub unsafe fn TranslateMessage(_msg: *const MSG) -> BOOL {
                BOOL(1)
            }

            pub unsafe fn DispatchMessageW(msg: *const MSG) -> LRESULT {
                if msg.is_null() {
                    return LRESULT(0);
                }
                let m = *msg;
                let proc_fn = win_procs()
                    .lock()
                    .ok()
                    .and_then(|map| map.get(&(m.hwnd.0 as usize)).copied().flatten());
                if let Some(f) = proc_fn {
                    f(m.hwnd, m.message, m.wParam, m.lParam)
                } else {
                    LRESULT(0)
                }
            }

            pub unsafe fn SetTimer(
                hwnd: Option<HWND>,
                id: usize,
                elapse_ms: u32,
                _timer_func: TIMERPROC,
            ) -> usize {
                #[cfg(target_os = "macos")]
                crate::mac::set_timer(hwnd, id, elapse_ms);
                let _ = (hwnd, elapse_ms);
                id.max(1)
            }

            pub unsafe fn KillTimer(hwnd: Option<HWND>, id: usize) -> Result<()> {
                #[cfg(target_os = "macos")]
                crate::mac::kill_timer(hwnd, id);
                let _ = (hwnd, id);
                Ok(())
            }

            pub unsafe fn LoadCursorW<P: crate::foundation::OptPcwstr>(_inst: Option<HINSTANCE>, _name: P) -> Result<HCURSOR> {
                Ok(HCURSOR::default())
            }
            pub unsafe fn LoadIconW<P: crate::foundation::OptPcwstr>(_inst: Option<HINSTANCE>, _name: P) -> Result<HICON> {
                Ok(HICON(1 as *mut c_void))
            }
            pub unsafe fn SetCoalescableTimer(
                hwnd: Option<HWND>,
                id: usize,
                elapse_ms: u32,
                timer_func: TIMERPROC,
                _tol: u32,
            ) -> usize {
                SetTimer(hwnd, id, elapse_ms, timer_func)
            }
            pub unsafe fn CreateIconIndirect(_info: *const ICONINFO) -> Result<HICON> {
                Ok(HICON(1 as *mut c_void))
            }
            pub unsafe fn DestroyIcon(_icon: HICON) -> Result<()> {
                Ok(())
            }

            static MENUS: OnceLock<Mutex<HashMap<usize, Vec<(u32, usize, String)>>>> =
                OnceLock::new();
            static NEXT_MENU: std::sync::atomic::AtomicUsize =
                std::sync::atomic::AtomicUsize::new(1);

            fn menus() -> &'static Mutex<HashMap<usize, Vec<(u32, usize, String)>>> {
                MENUS.get_or_init(|| Mutex::new(HashMap::new()))
            }

            pub unsafe fn CreatePopupMenu() -> Result<HMENU> {
                let id = NEXT_MENU.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut m) = menus().lock() {
                    m.insert(id, Vec::new());
                }
                Ok(HMENU(id as *mut c_void))
            }

            pub unsafe fn AppendMenuW(
                menu: HMENU,
                flags: MENU_ITEM_FLAGS,
                id: usize,
                item: PCWSTR,
            ) -> Result<()> {
                let title = if item.is_null() {
                    String::new()
                } else {
                    item.to_string().unwrap_or_default()
                };
                if let Ok(mut m) = menus().lock() {
                    if let Some(list) = m.get_mut(&(menu.0 as usize)) {
                        list.push((flags.0, id, title));
                    }
                }
                Ok(())
            }

            pub unsafe fn TrackPopupMenu(
                menu: HMENU,
                _flags: TRACK_POPUP_MENU_FLAGS,
                x: i32,
                y: i32,
                _reserved: Option<i32>,
                hwnd: HWND,
                _rect: Option<*const RECT>,
            ) -> BOOL {
                let items = menus()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(menu.0 as usize)).cloned())
                    .unwrap_or_default();
                #[cfg(target_os = "macos")]
                {
                    let cmd = crate::mac::track_popup_menu(hwnd, &items, x, y);
                    return BOOL(cmd as i32);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (items, x, y, hwnd);
                    BOOL(0)
                }
            }

            pub unsafe fn DestroyMenu(menu: HMENU) -> Result<()> {
                if let Ok(mut m) = menus().lock() {
                    m.remove(&(menu.0 as usize));
                }
                Ok(())
            }

            pub unsafe fn RegisterWindowMessageW(_name: PCWSTR) -> u32 {
                0xC100
            }

            pub unsafe fn SetLayeredWindowAttributes(
                _hwnd: HWND,
                _key: crate::foundation::COLORREF,
                _alpha: u8,
                _flags: LAYERED_WINDOW_ATTRIBUTES_FLAGS,
            ) -> Result<()> {
                Ok(())
            }

            pub unsafe fn GetForegroundWindow() -> HWND {
                HWND::default()
            }
            pub unsafe fn SetForegroundWindow(hwnd: HWND) -> BOOL {
                #[cfg(target_os = "macos")]
                crate::mac::activate_window(hwnd);
                let _ = hwnd;
                BOOL(1)
            }
            pub unsafe fn GetWindowThreadProcessId(_hwnd: HWND, pid: Option<*mut u32>) -> u32 {
                if let Some(p) = pid {
                    if !p.is_null() {
                        *p = 0;
                    }
                }
                0
            }
            pub unsafe fn IsWindowVisible(_hwnd: HWND) -> BOOL {
                BOOL(1)
            }
            pub unsafe fn IsIconic(_hwnd: HWND) -> BOOL {
                BOOL(0)
            }
            pub unsafe fn GetWindowLongW(_hwnd: HWND, _idx: WINDOW_LONG_PTR_INDEX) -> i32 {
                0
            }
            pub unsafe fn SetWindowLongW(
                _hwnd: HWND,
                _idx: WINDOW_LONG_PTR_INDEX,
                val: i32,
            ) -> i32 {
                val
            }
            pub unsafe fn GetWindow(_hwnd: HWND, _cmd: GET_WINDOW_CMD) -> Result<HWND> {
                Err(Error::from_hresult(E_FAIL))
            }
            pub unsafe fn IsWindow(hwnd: Option<HWND>) -> BOOL {
                BOOL(i32::from(hwnd.is_some_and(|h| !h.0.is_null())))
            }
            pub unsafe fn IsZoomed(_hwnd: HWND) -> BOOL {
                BOOL(0)
            }
            pub unsafe fn GetClassNameW(_hwnd: HWND, _class_name: &mut [u16]) -> i32 {
                0
            }
            pub unsafe fn BringWindowToTop(hwnd: HWND) -> Result<()> {
                #[cfg(target_os = "macos")]
                crate::mac::activate_window(hwnd);
                let _ = hwnd;
                Ok(())
            }
            pub unsafe fn EnumWindows(_proc: WNDENUMPROC, _lp: LPARAM) -> Result<()> {
                Ok(())
            }
            pub unsafe fn FindWindowW<P0: crate::foundation::OptPcwstr, P1: crate::foundation::OptPcwstr>(
                _class: P0,
                _title: P1,
            ) -> Result<HWND> {
                Err(Error::from_hresult(E_FAIL))
            }
            pub unsafe fn GetWindowLongPtrW(_hwnd: HWND, _idx: WINDOW_LONG_PTR_INDEX) -> isize {
                0
            }
            pub unsafe fn SetWindowLongPtrW(
                _hwnd: HWND,
                _idx: WINDOW_LONG_PTR_INDEX,
                val: isize,
            ) -> isize {
                val
            }
            pub unsafe fn SetWindowsHookExW(
                _id: i32,
                _proc: HOOKPROC,
                _mod: Option<HINSTANCE>,
                _tid: u32,
            ) -> Result<HHOOK> {
                Ok(HHOOK::default())
            }
            pub unsafe fn UnhookWindowsHookEx(_hook: HHOOK) -> Result<()> {
                Ok(())
            }
            pub unsafe fn CallNextHookEx(
                _hook: Option<HHOOK>,
                _code: i32,
                _wp: WPARAM,
                _lp: LPARAM,
            ) -> LRESULT {
                LRESULT(0)
            }
            pub unsafe fn SystemParametersInfoW(
                _action: SYSTEM_PARAMETERS_INFO_ACTION,
                _p1: u32,
                pv: Option<*mut c_void>,
                _winini: u32,
            ) -> Result<()> {
                if let Some(p) = pv {
                    if !p.is_null() {
                        *(p as *mut i32) = 1;
                    }
                }
                Ok(())
            }
        }
    }

    pub mod Media {
        pub mod Audio {
            use windows_core::{Error, PCWSTR, Result};
            use crate::foundation::E_NOTIMPL;

            pub mod Endpoints {
                use windows_core::Result;

                #[derive(Clone, Debug, Default)]
                pub struct IAudioEndpointVolume;
                impl IAudioEndpointVolume {
                    pub unsafe fn SetMasterVolumeLevelScalar(
                        &self,
                        _v: f32,
                        _ctx: *const windows_core::GUID,
                    ) -> Result<()> {
                        Ok(())
                    }
                    pub unsafe fn GetMasterVolumeLevelScalar(&self) -> Result<f32> {
                        Ok(0.5)
                    }
                    pub unsafe fn SetMute(
                        &self,
                        _mute: bool,
                        _ctx: *const windows_core::GUID,
                    ) -> Result<()> {
                        Ok(())
                    }
                    pub unsafe fn GetMute(&self) -> Result<windows_core::BOOL> {
                        Ok(windows_core::BOOL(0))
                    }
                }
            }

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct EDataFlow(pub i32);
            pub const eRender: EDataFlow = EDataFlow(0);
            pub const eCapture: EDataFlow = EDataFlow(1);

            #[repr(transparent)]
            #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
            pub struct ERole(pub i32);
            pub const eConsole: ERole = ERole(0);

            pub const DEVICE_STATE_ACTIVE: u32 = 1;

            #[derive(Clone, Debug, Default)]
            pub struct IMMDeviceEnumerator;
            #[derive(Clone, Debug, Default)]
            pub struct IMMDevice;

            impl IMMDeviceEnumerator {
                pub unsafe fn GetDefaultAudioEndpoint(
                    &self,
                    _flow: EDataFlow,
                    _role: ERole,
                ) -> Result<IMMDevice> {
                    Err(Error::from_hresult(E_NOTIMPL))
                }
            }

            impl IMMDevice {
                pub unsafe fn GetId(&self) -> Result<windows_core::PWSTR> {
                    Err(Error::from_hresult(E_NOTIMPL))
                }
            }

            pub unsafe fn PlaySoundW(_name: PCWSTR, _hmod: usize, _flags: u32) -> windows_core::BOOL {
                windows_core::BOOL(0)
            }
        }
    }

    pub mod System {
        pub mod Com {
            use std::ffi::c_void;
            use windows_core::{GUID, HRESULT, Result};
            use crate::foundation::S_OK;

            pub const CLSCTX_INPROC_SERVER: u32 = 0x1;
            pub const CLSCTX_LOCAL_SERVER: u32 = 0x4;
            pub const CLSCTX_ALL: u32 = 0x17;
            pub const COINIT_APARTMENTTHREADED: u32 = 0x2;
            pub const COINIT_MULTITHREADED: u32 = 0x0;

            pub unsafe fn CoInitializeEx(_res: Option<*const c_void>, _coinit: u32) -> HRESULT {
                S_OK
            }
            pub unsafe fn CoUninitialize() {}
            pub unsafe fn CoTaskMemFree(_pv: Option<*const c_void>) {}

            pub trait OptUnknown {}
            impl OptUnknown for Option<&windows_core::IUnknown> {}

            pub unsafe fn CoCreateInstance<P0: OptUnknown, T: Default>(
                _clsid: *const GUID,
                _outer: P0,
                _ctx: u32,
            ) -> Result<T> {
                Ok(T::default())
            }
        }

        pub mod LibraryLoader {
            use windows_core::{PCSTR, Result};
            use crate::foundation::HMODULE;

            pub unsafe fn GetModuleHandleW<P: crate::foundation::OptPcwstr>(_name: P) -> Result<HMODULE> {
                Ok(HMODULE(1 as *mut std::ffi::c_void))
            }
            pub unsafe fn GetProcAddress(
                _hmod: HMODULE,
                _name: PCSTR,
            ) -> Option<unsafe extern "system" fn() -> isize> {
                None
            }
        }

        pub mod SystemInformation {
            use crate::foundation::SYSTEMTIME;

            #[cfg(unix)]
            #[repr(C)]
            struct Tm {
                tm_sec: i32,
                tm_min: i32,
                tm_hour: i32,
                tm_mday: i32,
                tm_mon: i32,
                tm_year: i32,
                tm_wday: i32,
                tm_yday: i32,
                tm_isdst: i32,
                tm_gmtoff: isize,
                tm_zone: *const i8,
            }

            #[cfg(unix)]
            extern "C" {
                fn time(t: *mut isize) -> isize;
                fn localtime_r(timep: *const isize, result: *mut Tm) -> *mut Tm;
            }

            pub unsafe fn GetLocalTime() -> SYSTEMTIME {
                #[cfg(unix)]
                {
                    let secs = time(std::ptr::null_mut());
                    let mut tm: Tm = std::mem::zeroed();
                    if !localtime_r(&secs, &mut tm).is_null() {
                        let sub_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| (d.subsec_millis() % 1000) as u16)
                            .unwrap_or(0);
                        return SYSTEMTIME {
                            wYear: (tm.tm_year + 1900) as u16,
                            wMonth: (tm.tm_mon + 1) as u16,
                            wDayOfWeek: tm.tm_wday as u16,
                            wDay: tm.tm_mday as u16,
                            wHour: tm.tm_hour as u16,
                            wMinute: tm.tm_min as u16,
                            wSecond: tm.tm_sec.min(59) as u16,
                            wMilliseconds: sub_ms,
                        };
                    }
                }
                // Fallback wall clock decomposition from UNIX_EPOCH
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let sub_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| (d.subsec_millis() % 1000) as u16)
                    .unwrap_or(0);
                let days = (secs / 86400) as i64;
                let rem = (secs % 86400) as u32;
                let dow = ((days + 4).rem_euclid(7)) as u16;
                let z = days + 719468;
                let era = z.div_euclid(146097);
                let doe = z.rem_euclid(146097) as u64;
                let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
                let y = yoe as i64 + era * 400;
                let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                let mp = (5 * doy + 2) / 153;
                let d = (doy - (153 * mp + 2) / 5 + 1) as u16;
                let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u16;
                let y = (y + if m <= 2 { 1 } else { 0 }) as u16;
                SYSTEMTIME {
                    wYear: y,
                    wMonth: m,
                    wDayOfWeek: dow,
                    wDay: d,
                    wHour: (rem / 3600) as u16,
                    wMinute: ((rem % 3600) / 60) as u16,
                    wSecond: (rem % 60) as u16,
                    wMilliseconds: sub_ms,
                }
            }
        }

        pub mod Threading {
            use windows_core::Result;
            use crate::foundation::HANDLE;

            pub unsafe fn GetCurrentThreadId() -> u32 {
                1
            }
            pub unsafe fn AttachThreadInput(_id1: u32, _id2: u32, _attach: bool) -> Result<()> {
                Ok(())
            }
            pub unsafe fn CreateMutexW(
                _attr: Option<*const std::ffi::c_void>,
                _initial_owner: bool,
                _name: windows_core::PCWSTR,
            ) -> Result<HANDLE> {
                Ok(HANDLE(1 as *mut std::ffi::c_void))
            }
        }

        pub mod DataExchange {
            use windows_core::Result;
            use crate::foundation::HWND;

            pub unsafe fn AddClipboardFormatListener(_hwnd: HWND) -> Result<()> {
                Ok(())
            }
            pub unsafe fn RemoveClipboardFormatListener(_hwnd: HWND) -> Result<()> {
                Ok(())
            }
        }

        pub mod Memory {
            use crate::foundation::HGLOBAL;

            pub unsafe fn GlobalLock(h: HGLOBAL) -> *mut std::ffi::c_void {
                h.0
            }
            pub unsafe fn GlobalUnlock(_h: HGLOBAL) -> windows_core::BOOL {
                windows_core::BOOL(1)
            }
        }

        pub mod Power {
            use windows_core::{GUID, Result};
            use crate::foundation::HANDLE;

            #[repr(C)]
            #[derive(Clone, Copy)]
            pub struct POWERBROADCAST_SETTING {
                pub PowerSetting: GUID,
                pub DataLength: u32,
                pub Data: [u8; 1],
            }

            pub unsafe fn RegisterPowerSettingNotification(
                _recipient: HANDLE,
                _guid: *const GUID,
                _flags: u32,
            ) -> Result<HANDLE> {
                Ok(HANDLE(1 as *mut std::ffi::c_void))
            }

            pub unsafe fn SetSuspendState(_hibernate: bool, _force: bool, _wakeup_disabled: bool) -> bool {
                #[cfg(target_os = "macos")]
                {
                    std::process::Command::new("pmset")
                        .args(["sleepnow"])
                        .status()
                        .is_ok_and(|s| s.success())
                }
                #[cfg(not(target_os = "macos"))]
                true
            }
        }

        pub mod Shutdown {
            use windows_core::Result;

            pub unsafe fn LockWorkStation() -> Result<()> {
                #[cfg(target_os = "macos")]
                {
                    let _ = std::process::Command::new("pmset").args(["displaysleepnow"]).spawn();
                }
                Ok(())
            }
        }

        pub mod RemoteDesktop {
            use windows_core::Result;
            use crate::foundation::HWND;

            pub const NOTIFY_FOR_THIS_SESSION: u32 = 0;

            pub unsafe fn WTSRegisterSessionNotification(
                _hwnd: HWND,
                _flags: u32,
            ) -> Result<()> {
                Ok(())
            }
        }

        pub mod SystemServices {
            use windows_core::GUID;

            pub const GUID_CONSOLE_DISPLAY_STATE: GUID =
                GUID::from_u128(0x6FE69556_704A_47A0_8F24_C28D936FDA47);
            pub const GUID_POWER_SAVING_STATUS: GUID =
                GUID::from_u128(0xE00958C0_C213_4ACE_AC77_FECCED2EEEA5);
        }

        pub mod Time {
            use crate::foundation::FILETIME;

            pub unsafe fn GetSystemTimeAsFileTime() -> FILETIME {
                let d = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                let ticks = (d.as_nanos() / 100) as u64 + 116_444_736_000_000_000u64;
                FILETIME {
                    dwLowDateTime: ticks as u32,
                    dwHighDateTime: (ticks >> 32) as u32,
                }
            }
        }

        pub mod Diagnostics {
            pub mod Debug {
                use windows_core::PCWSTR;
                pub unsafe fn OutputDebugStringW(_s: PCWSTR) {}
            }
        }
    }
}
