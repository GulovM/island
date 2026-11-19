#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::too_many_arguments)]

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use windows_core::{CastFrom, Error, Interface, Result, BOOL};

use crate::d2d::PixelSurface;
use crate::foundation::{E_FAIL, HWND};

pub mod Common {
    use windows_core::BOOL;

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_FORMAT(pub u32);
    pub const DXGI_FORMAT_UNKNOWN: DXGI_FORMAT = DXGI_FORMAT(0);
    pub const DXGI_FORMAT_B8G8R8A8_UNORM: DXGI_FORMAT = DXGI_FORMAT(87);

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_SAMPLE_DESC {
        pub Count: u32,
        pub Quality: u32,
    }

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_ALPHA_MODE(pub u32);
    pub const DXGI_ALPHA_MODE_UNSPECIFIED: DXGI_ALPHA_MODE = DXGI_ALPHA_MODE(0);
    pub const DXGI_ALPHA_MODE_PREMULTIPLIED: DXGI_ALPHA_MODE = DXGI_ALPHA_MODE(1);
    pub const DXGI_ALPHA_MODE_STRAIGHT: DXGI_ALPHA_MODE = DXGI_ALPHA_MODE(2);
    pub const DXGI_ALPHA_MODE_IGNORE: DXGI_ALPHA_MODE = DXGI_ALPHA_MODE(3);

    pub type _UnusedBool = BOOL;
}

pub mod Dxgi {
    use super::*;
    pub use super::Common;
    pub use super::Common::*;

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_USAGE(pub u32);
    pub const DXGI_USAGE_RENDER_TARGET_OUTPUT: DXGI_USAGE = DXGI_USAGE(32);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_SWAP_EFFECT(pub i32);
    pub const DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL: DXGI_SWAP_EFFECT = DXGI_SWAP_EFFECT(3);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_SCALING(pub i32);
    pub const DXGI_SCALING_STRETCH: DXGI_SCALING = DXGI_SCALING(0);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_SWAP_CHAIN_FLAG(pub i32);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_PRESENT(pub u32);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_ADAPTER_FLAG(pub i32);
    pub const DXGI_ADAPTER_FLAG_NONE: DXGI_ADAPTER_FLAG = DXGI_ADAPTER_FLAG(0);
    pub const DXGI_ADAPTER_FLAG_SOFTWARE: DXGI_ADAPTER_FLAG = DXGI_ADAPTER_FLAG(2);

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DXGI_ADAPTER_DESC1 {
        pub Description: [u16; 128],
        pub VendorId: u32,
        pub DeviceId: u32,
        pub SubSysId: u32,
        pub Revision: u32,
        pub DedicatedVideoMemory: usize,
        pub DedicatedSystemMemory: usize,
        pub SharedSystemMemory: usize,
        pub AdapterLuid: i64,
        pub Flags: u32,
    }

    impl Default for DXGI_ADAPTER_DESC1 {
        fn default() -> Self {
            Self {
                Description: [0; 128],
                VendorId: 0,
                DeviceId: 0,
                SubSysId: 0,
                Revision: 0,
                DedicatedVideoMemory: 0,
                DedicatedSystemMemory: 0,
                SharedSystemMemory: 0,
                AdapterLuid: 0,
                Flags: 0,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DXGI_SWAP_CHAIN_DESC1 {
        pub Width: u32,
        pub Height: u32,
        pub Format: DXGI_FORMAT,
        pub Stereo: BOOL,
        pub SampleDesc: DXGI_SAMPLE_DESC,
        pub BufferUsage: DXGI_USAGE,
        pub BufferCount: u32,
        pub Scaling: DXGI_SCALING,
        pub SwapEffect: DXGI_SWAP_EFFECT,
        pub AlphaMode: DXGI_ALPHA_MODE,
        pub Flags: u32,
    }

    #[derive(Clone, Debug, Default)]
    pub struct IDXGIOutput;

    #[derive(Clone)]
    pub struct IDXGISurface(pub Arc<Mutex<PixelSurface>>);

    impl IDXGISurface {
        pub fn surface(&self) -> Arc<Mutex<PixelSurface>> {
            self.0.clone()
        }
    }

    #[derive(Clone)]
    pub struct SwapChainInner {
        pub surface: Arc<Mutex<PixelSurface>>,
        pub hwnd: Arc<Mutex<Option<HWND>>>,
    }

    #[derive(Clone)]
    pub struct IDXGISwapChain1(pub Arc<SwapChainInner>);

    impl IDXGISwapChain1 {
        pub unsafe fn GetBuffer(&self, _buffer: u32) -> Result<IDXGISurface> {
            Ok(IDXGISurface(self.0.surface.clone()))
        }

        pub unsafe fn ResizeBuffers(
            &self,
            _buffer_count: u32,
            width: u32,
            height: u32,
            _new_format: DXGI_FORMAT,
            _flags: DXGI_SWAP_CHAIN_FLAG,
        ) -> Result<()> {
            let mut s = self.0.surface.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
            *s = PixelSurface::new(width.max(1), height.max(1));
            Ok(())
        }

        pub unsafe fn Present(&self, _sync_interval: u32, _flags: DXGI_PRESENT) -> windows_core::HRESULT {
            #[cfg(target_os = "macos")]
            {
                let hwnd = self.0.hwnd.lock().ok().and_then(|g| *g);
                if let Some(h) = hwnd {
                    if let Ok(surf) = self.0.surface.lock() {
                        crate::mac::present_surface(h, &surf);
                    }
                }
            }
            crate::foundation::S_OK
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct IDXGIDevice;

    #[derive(Clone, Debug, Default)]
    pub struct IDXGIDevice1;

    impl Interface for IDXGIDevice {
        fn as_raw(&self) -> *mut c_void {
            self as *const _ as *mut c_void
        }
    }
    impl Interface for IDXGIDevice1 {
        fn as_raw(&self) -> *mut c_void {
            self as *const _ as *mut c_void
        }
    }

    impl CastFrom<IDXGIDevice1> for IDXGIDevice {
        fn cast_into(&self) -> Result<IDXGIDevice1> {
            Ok(IDXGIDevice1)
        }
    }

    impl IDXGIDevice {
        pub unsafe fn GetAdapter(&self) -> Result<IDXGIAdapter> {
            Ok(IDXGIAdapter)
        }
    }

    impl IDXGIDevice1 {
        pub unsafe fn SetMaximumFrameLatency(&self, _max_latency: u32) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct IDXGIAdapter;

    impl IDXGIAdapter {
        pub unsafe fn GetParent<T: Default>(&self) -> Result<T> {
            Ok(T::default())
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct IDXGIAdapter1(pub u32);

    impl IDXGIAdapter1 {
        pub unsafe fn GetDesc1(&self) -> Result<DXGI_ADAPTER_DESC1> {
            let mut desc = DXGI_ADAPTER_DESC1::default();
            #[cfg(target_os = "macos")]
            let name = crate::mac::gpu_name();
            #[cfg(not(target_os = "macos"))]
            let name = "Apple GPU".to_string();
            for (dst, src) in desc.Description.iter_mut().zip(name.encode_utf16()) {
                *dst = src;
            }
            Ok(desc)
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct IDXGIFactory1;

    impl IDXGIFactory1 {
        pub unsafe fn EnumAdapters1(&self, index: u32) -> Result<IDXGIAdapter1> {
            if index == 0 {
                Ok(IDXGIAdapter1(0))
            } else {
                Err(Error::from_hresult(E_FAIL))
            }
        }
    }

    pub unsafe fn CreateDXGIFactory1<T: Default>() -> Result<T> {
        Ok(T::default())
    }

    #[derive(Clone, Debug, Default)]
    pub struct IDXGIFactory2;

    impl IDXGIFactory2 {
        pub unsafe fn CreateSwapChainForComposition(
            &self,
            _device: &super::Direct3D11::ID3D11Device,
            desc: &DXGI_SWAP_CHAIN_DESC1,
            _output: Option<&IDXGIOutput>,
        ) -> Result<IDXGISwapChain1> {
            Ok(IDXGISwapChain1(Arc::new(SwapChainInner {
                surface: Arc::new(Mutex::new(PixelSurface::new(desc.Width.max(1), desc.Height.max(1)))),
                hwnd: Arc::new(Mutex::new(None)),
            })))
        }
    }
}

pub mod Direct3D {
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D3D_DRIVER_TYPE(pub i32);
    pub const D3D_DRIVER_TYPE_HARDWARE: D3D_DRIVER_TYPE = D3D_DRIVER_TYPE(1);
    pub const D3D_DRIVER_TYPE_WARP: D3D_DRIVER_TYPE = D3D_DRIVER_TYPE(5);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D3D_FEATURE_LEVEL(pub i32);
}

pub mod Direct3D11 {
    use super::Direct3D::*;
    use super::Dxgi::*;
    use super::*;
    use crate::foundation::HMODULE;

    pub const D3D11_SDK_VERSION: u32 = 7;

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D3D11_CREATE_DEVICE_FLAG(pub u32);
    pub const D3D11_CREATE_DEVICE_BGRA_SUPPORT: D3D11_CREATE_DEVICE_FLAG =
        D3D11_CREATE_DEVICE_FLAG(0x20);

    #[derive(Clone, Debug, Default)]
    pub struct ID3D11Device;

    #[derive(Clone, Debug, Default)]
    pub struct ID3D11DeviceContext;

    impl Interface for ID3D11Device {
        fn as_raw(&self) -> *mut c_void {
            self as *const _ as *mut c_void
        }
    }

    impl CastFrom<IDXGIDevice> for ID3D11Device {
        fn cast_into(&self) -> Result<IDXGIDevice> {
            Ok(IDXGIDevice)
        }
    }

    impl ID3D11Device {
        pub unsafe fn GetDeviceRemovedReason(&self) -> Result<()> {
            Ok(())
        }
    }

    pub unsafe fn D3D11CreateDevice(
        _adapter: Option<&IDXGIAdapter>,
        _driver_type: D3D_DRIVER_TYPE,
        _software: HMODULE,
        _flags: D3D11_CREATE_DEVICE_FLAG,
        _feature_levels: Option<&[D3D_FEATURE_LEVEL]>,
        _sdk_version: u32,
        device: Option<*mut Option<ID3D11Device>>,
        _feature_level: Option<*mut D3D_FEATURE_LEVEL>,
        immediate_context: Option<*mut Option<ID3D11DeviceContext>>,
    ) -> Result<()> {
        if let Some(d) = device {
            if !d.is_null() {
                *d = Some(ID3D11Device);
            }
        }
        if let Some(c) = immediate_context {
            if !c.is_null() {
                *c = Some(ID3D11DeviceContext);
            }
        }
        Ok(())
    }
}

pub mod DirectComposition {
    use super::Dxgi::*;
    use super::*;

    #[derive(Clone, Debug, Default)]
    pub struct IDCompositionDevice;

    #[derive(Clone)]
    pub struct IDCompositionTarget(pub HWND);

    #[derive(Clone, Default)]
    pub struct IDCompositionVisual {
        pub swapchain: Arc<Mutex<Option<IDXGISwapChain1>>>,
    }

    pub unsafe fn DCompositionCreateDevice<T: Default>(_dxgi: &IDXGIDevice) -> Result<T> {
        Ok(T::default())
    }

    impl IDCompositionDevice {
        pub unsafe fn CreateTargetForHwnd(
            &self,
            hwnd: HWND,
            _topmost: bool,
        ) -> Result<IDCompositionTarget> {
            Ok(IDCompositionTarget(hwnd))
        }

        pub unsafe fn CreateVisual(&self) -> Result<IDCompositionVisual> {
            Ok(IDCompositionVisual::default())
        }

        pub unsafe fn Commit(&self) -> Result<()> {
            Ok(())
        }
    }

    impl IDCompositionVisual {
        pub unsafe fn SetContent(&self, swapchain: &IDXGISwapChain1) -> Result<()> {
            if let Ok(mut s) = self.swapchain.lock() {
                *s = Some(swapchain.clone());
            }
            Ok(())
        }
    }

    impl IDCompositionTarget {
        pub unsafe fn SetRoot(&self, visual: &IDCompositionVisual) -> Result<()> {
            if let Ok(s) = visual.swapchain.lock() {
                if let Some(sc) = &*s {
                    if let Ok(mut h) = sc.0.hwnd.lock() {
                        *h = Some(self.0);
                    }
                }
            }
            Ok(())
        }
    }
}

pub use Dxgi::IDXGISurface;
