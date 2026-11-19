#![allow(non_snake_case, non_camel_case_types, dead_code)]

pub use windows_core as core;

pub mod d2d;
pub mod dwrite;
pub mod dxgi_d3d;
pub mod foundation;
#[cfg(target_os = "macos")]
pub mod mac;
pub mod win32_ui;

pub use win32_ui::Win32;
