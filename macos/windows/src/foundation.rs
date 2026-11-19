#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::ffi::c_void;
use windows_core::{BOOL, GUID, HRESULT, PCWSTR};

pub trait OptPcwstr {
    fn into_opt(self) -> Option<PCWSTR>;
}
impl OptPcwstr for PCWSTR {
    fn into_opt(self) -> Option<PCWSTR> {
        Some(self)
    }
}
impl OptPcwstr for Option<PCWSTR> {
    fn into_opt(self) -> Option<PCWSTR> {
        self
    }
}

pub const TRUE: BOOL = BOOL(1);
pub const FALSE: BOOL = BOOL(0);

pub const S_OK: HRESULT = HRESULT(0);
pub const E_FAIL: HRESULT = HRESULT(-2147467259);
pub const E_NOTIMPL: HRESULT = HRESULT(-2147467263);
pub const E_INVALIDARG: HRESULT = HRESULT(-2147024809);
pub const DATA_S_SAMEFORMATETC: HRESULT = HRESULT(0x0004_0130);
pub const DRAGDROP_S_CANCEL: HRESULT = HRESULT(0x0004_0101);
pub const DRAGDROP_S_DROP: HRESULT = HRESULT(0x0004_0100);
pub const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = HRESULT(0x0004_0102);
pub const DV_E_FORMATETC: HRESULT = HRESULT(-2147221404);
pub const OLE_E_ADVISENOTSUPPORTED: HRESULT = HRESULT(-2147221501);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WIN32_ERROR(pub u32);

pub const ERROR_SUCCESS: WIN32_ERROR = WIN32_ERROR(0);
pub const ERROR_FILE_NOT_FOUND: WIN32_ERROR = WIN32_ERROR(2);
pub const ERROR_ALREADY_EXISTS: WIN32_ERROR = WIN32_ERROR(183);
pub const ERROR_MORE_DATA: WIN32_ERROR = WIN32_ERROR(234);
pub const ERROR_NO_MORE_ITEMS: WIN32_ERROR = WIN32_ERROR(259);

pub const GENERIC_READ: u32 = 0x8000_0000;

macro_rules! def_handle {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name(pub *mut c_void);

        impl Default for $name {
            fn default() -> Self {
                Self(std::ptr::null_mut())
            }
        }

        impl $name {
            pub fn is_invalid(self) -> bool {
                self.0.is_null()
            }
        }

        unsafe impl Send for $name {}
        unsafe impl Sync for $name {}
    };
}

def_handle!(HWND);
def_handle!(HINSTANCE);
def_handle!(HMODULE);
def_handle!(HANDLE);
def_handle!(HLOCAL);
def_handle!(HGLOBAL);

impl From<HMODULE> for HINSTANCE {
    fn from(m: HMODULE) -> Self {
        Self(m.0)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WPARAM(pub usize);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LPARAM(pub isize);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LRESULT(pub isize);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct POINTL {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SIZE {
    pub cx: i32,
    pub cy: i32,
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct COLORREF(pub u32);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SYSTEMTIME {
    pub wYear: u16,
    pub wMonth: u16,
    pub wDayOfWeek: u16,
    pub wDay: u16,
    pub wHour: u16,
    pub wMinute: u16,
    pub wSecond: u16,
    pub wMilliseconds: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PROPERTYKEY {
    pub fmtid: GUID,
    pub pid: u32,
}

pub unsafe fn GetLastError() -> WIN32_ERROR {
    ERROR_SUCCESS
}

pub unsafe fn CloseHandle(_h: HANDLE) -> windows_core::Result<()> {
    Ok(())
}

pub unsafe fn LocalFree(_h: Option<HLOCAL>) -> HLOCAL {
    HLOCAL(std::ptr::null_mut())
}

pub unsafe fn GlobalFree(_h: Option<HGLOBAL>) -> HGLOBAL {
    HGLOBAL(std::ptr::null_mut())
}
