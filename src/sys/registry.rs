//! Чтение реестра Windows (DWORD, REG_SZ/EXPAND_SZ, бинарные данные).

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD,
    RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
};

use super::wz;

pub fn read_dword(key: PCWSTR, name: PCWSTR) -> Option<u32> {
    unsafe {
        let (mut v, mut size) = (0u32, 4u32);
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut size),
        );
        (r == ERROR_SUCCESS).then_some(v)
    }
}

pub fn read_binary(key: PCWSTR, name: PCWSTR) -> Option<Vec<u8>> {
    unsafe {
        let mut size = 0u32;
        if RegGetValueW(HKEY_CURRENT_USER, key, name, RRF_RT_REG_BINARY, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        );
        (r == ERROR_SUCCESS).then(|| {
            buf.truncate(size as usize);
            buf
        })
    }
}

/// Строковое значение реестра (`REG_SZ`/`REG_EXPAND_SZ`). `machine` — HKLM, иначе HKCU.
pub fn reg_str(machine: bool, key: &str, name: &str) -> Option<String> {
    let root = if machine { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER };
    let (k, n) = (wz(key), wz(name));
    unsafe {
        let mut size = 0u32;
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
        if RegGetValueW(root, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), flags, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let r = RegGetValueW(
            root,
            PCWSTR(k.as_ptr()),
            PCWSTR(n.as_ptr()),
            flags,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        );
        if r != ERROR_SUCCESS {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end])).filter(|s| !s.is_empty())
    }
}
