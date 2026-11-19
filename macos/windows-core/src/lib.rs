#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::ffi::c_void;
use std::fmt;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HRESULT(pub i32);

impl HRESULT {
    pub const fn is_ok(self) -> bool {
        self.0 >= 0
    }
    pub const fn is_err(self) -> bool {
        self.0 < 0
    }
    pub const fn ok(self) -> Result<()> {
        if self.0 >= 0 {
            Ok(())
        } else {
            Err(Error::from_hresult(self))
        }
    }
    pub const fn from_win32(err: u32) -> Self {
        Self(if err == 0 { 0 } else { (0x8007_0000u32 | (err & 0xFFFF)) as i32 })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: HRESULT,
    pub message: String,
}

impl Error {
    pub const fn from_hresult(code: HRESULT) -> Self {
        Self { code, message: String::new() }
    }
    pub fn new(code: HRESULT, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl From<HRESULT> for Error {
    fn from(code: HRESULT) -> Self {
        Self::from_hresult(code)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            write!(f, "HRESULT(0x{:08X})", self.code.0 as u32)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BOOL(pub i32);

impl BOOL {
    pub const fn as_bool(self) -> bool {
        self.0 != 0
    }
}

impl From<bool> for BOOL {
    fn from(b: bool) -> Self {
        Self(if b { 1 } else { 0 })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GUID(pub u128);

impl GUID {
    pub const fn from_u128(v: u128) -> Self {
        Self(v)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PCWSTR(pub *const u16);

impl Default for PCWSTR {
    fn default() -> Self {
        Self::null()
    }
}

impl PCWSTR {
    pub const fn null() -> Self {
        Self(std::ptr::null())
    }
    pub const fn from_raw(ptr: *const u16) -> Self {
        Self(ptr)
    }
    pub fn is_null(self) -> bool {
        self.0.is_null()
    }
    pub const fn as_ptr(self) -> *const u16 {
        self.0
    }
    /// # Safety
    /// Caller must ensure `self.0` points to a valid null-terminated UTF-16 string.
    pub unsafe fn to_string(self) -> Result<String> {
        if self.0.is_null() {
            return Err(Error::from_hresult(HRESULT(-2147467261)));
        }
        let mut len = 0usize;
        while *self.0.add(len) != 0 {
            len += 1;
        }
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(self.0, len)))
    }
    /// # Safety
    /// Same safety requirements as `to_string`.
    pub unsafe fn as_slice(&self) -> &[u16] {
        if self.0.is_null() {
            return &[];
        }
        let mut len = 0usize;
        while *self.0.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(self.0, len)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PWSTR(pub *mut u16);

impl Default for PWSTR {
    fn default() -> Self {
        Self::null()
    }
}

impl PWSTR {
    pub const fn null() -> Self {
        Self(std::ptr::null_mut())
    }
    pub fn is_null(self) -> bool {
        self.0.is_null()
    }
    /// # Safety
    /// Caller must ensure `self.0` points to a valid null-terminated UTF-16 string.
    pub unsafe fn to_string(self) -> Result<String> {
        PCWSTR(self.0).to_string()
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PCSTR(pub *const u8);

impl Default for PCSTR {
    fn default() -> Self {
        Self::null()
    }
}

impl PCSTR {
    pub const fn null() -> Self {
        Self(std::ptr::null())
    }
    pub const fn as_ptr(self) -> *const u8 {
        self.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HSTRING(pub String);

impl From<&str> for HSTRING {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for HSTRING {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for HSTRING {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct IUnknown;

pub type Ref<'a, T> = Option<&'a T>;

pub trait CastFrom<T> {
    fn cast_into(&self) -> Result<T>;
}

impl<T: Clone> CastFrom<T> for T {
    fn cast_into(&self) -> Result<T> {
        Ok(self.clone())
    }
}

pub trait Interface: Sized {
    fn as_raw(&self) -> *mut c_void;
    fn cast<T>(&self) -> Result<T>
    where
        Self: CastFrom<T>,
    {
        self.cast_into()
    }
}

pub const fn utf16_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0;
    let mut n = 0;
    while i < b.len() {
        let c = b[i];
        if c < 0x80 {
            n += 1;
            i += 1;
        } else if c < 0xE0 {
            n += 1;
            i += 2;
        } else if c < 0xF0 {
            n += 1;
            i += 3;
        } else {
            n += 2;
            i += 4;
        }
    }
    n + 1
}

pub const fn utf16_arr<const N: usize>(s: &str) -> [u16; N] {
    let b = s.as_bytes();
    let mut out = [0u16; N];
    let mut i = 0;
    let mut o = 0;
    while i < b.len() {
        let cp: u32;
        let c = b[i] as u32;
        if c < 0x80 {
            cp = c;
            i += 1;
        } else if c < 0xE0 {
            cp = ((c & 0x1F) << 6) | (b[i + 1] as u32 & 0x3F);
            i += 2;
        } else if c < 0xF0 {
            cp = ((c & 0x0F) << 12) | ((b[i + 1] as u32 & 0x3F) << 6) | (b[i + 2] as u32 & 0x3F);
            i += 3;
        } else {
            cp = ((c & 0x07) << 18)
                | ((b[i + 1] as u32 & 0x3F) << 12)
                | ((b[i + 2] as u32 & 0x3F) << 6)
                | (b[i + 3] as u32 & 0x3F);
            i += 4;
        }
        if cp <= 0xFFFF {
            out[o] = cp as u16;
            o += 1;
        } else {
            let v = cp - 0x10000;
            out[o] = (0xD800 + (v >> 10)) as u16;
            out[o + 1] = (0xDC00 + (v & 0x3FF)) as u16;
            o += 2;
        }
    }
    out[o] = 0;
    out
}

#[macro_export]
macro_rules! w {
    ($s:expr) => {{
        const _ARR: &[u16] = &$crate::utf16_arr::<{ $crate::utf16_len($s) }>($s);
        $crate::PCWSTR(_ARR.as_ptr())
    }};
}

#[macro_export]
macro_rules! s {
    ($s:expr) => {{
        const _S: &[u8] = concat!($s, "\0").as_bytes();
        $crate::PCSTR(_S.as_ptr())
    }};
}
