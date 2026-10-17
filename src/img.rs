//! Работа с изображениями через WIC: сохранение PNG, загрузка с масштабированием, DIB для
//! буфера обмена. Все функции блокирующие — вызывать из коротких фоновых потоков.

use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

use crate::sys;

unsafe fn factory() -> Option<IWICImagingFactory> {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()
}

/// Сохраняет BGRA-пиксели как PNG (альфа берётся как есть: для снимков экрана она 255).
pub unsafe fn save_png(path: &Path, w: u32, h: u32, bgra: &[u8]) -> Option<()> {
    if bgra.len() < (w * h * 4) as usize {
        return None;
    }
    let f = factory()?;
    let wide = sys::wz(path.to_str()?);
    let stream = f.CreateStream().ok()?;
    // GENERIC_WRITE = 0x4000_0000.
    stream.InitializeFromFilename(PCWSTR(wide.as_ptr()), 0x4000_0000).ok()?;
    let enc = f.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).ok()?;
    enc.Initialize(&stream, WICBitmapEncoderNoCache).ok()?;
    let mut frame: Option<IWICBitmapFrameEncode> = None;
    enc.CreateNewFrame(&mut frame, std::ptr::null_mut()).ok()?;
    let frame = frame?;
    frame.Initialize(None).ok()?;
    frame.SetSize(w, h).ok()?;
    let mut fmt = GUID_WICPixelFormat32bppBGRA;
    frame.SetPixelFormat(&mut fmt).ok()?;
    frame.WritePixels(h, w * 4, bgra).ok()?;
    frame.Commit().ok()?;
    enc.Commit().ok()?;
    // Кодировщик и поток держат файл открытым: без этого переименование при шифровании не пройдёт.
    drop(frame);
    drop(enc);
    drop(stream);
    seal_file(path)
}

/// Шифрует PNG на месте (DPAPI). Уже зашифрованный файл не трогает.
pub fn seal_file(path: &Path) -> Option<()> {
    let bytes = std::fs::read(path).ok()?;
    if sys::is_sealed(&bytes) {
        return Some(());
    }
    let sealed = sys::seal(&bytes)?;
    let tmp = path.with_extension("png.tmp");
    let done = std::fs::write(&tmp, sealed).and_then(|()| std::fs::rename(&tmp, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done.ok()
}

unsafe fn open(f: &IWICImagingFactory, path: &Path) -> Option<(IWICBitmapFrameDecode, u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    let dec = if sys::is_sealed(&bytes) {
        // Зашифрованный PNG: расшифровываем в память, на диск открытый вариант не попадает.
        let plain = sys::unseal(&bytes)?;
        let mem = windows::Win32::UI::Shell::SHCreateMemStream(Some(&plain))?;
        f.CreateDecoderFromStream(&mem, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?
    } else {
        let wide = sys::wz(path.to_str()?);
        f.CreateDecoderFromFilename(PCWSTR(wide.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?
    };
    let frame = dec.GetFrame(0).ok()?;
    let (mut w, mut h) = (0u32, 0u32);
    frame.GetSize(&mut w, &mut h).ok()?;
    (w > 0 && h > 0).then_some((frame, w, h))
}

/// Загружает изображение, вписывая его в `maxw`×`maxh` без увеличения. BGRA premultiplied.
pub unsafe fn load_fit(path: &Path, maxw: u32, maxh: u32) -> Option<(u32, u32, Vec<u8>)> {
    let f = factory()?;
    let (frame, w, h) = open(&f, path)?;
    let k = (maxw as f32 / w as f32).min(maxh as f32 / h as f32).min(1.0);
    let (nw, nh) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
    let scaler = f.CreateBitmapScaler().ok()?;
    scaler.Initialize(&frame, nw, nh, WICBitmapInterpolationModeFant).ok()?;
    let conv = f.CreateFormatConverter().ok()?;
    conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
        .ok()?;
    let mut buf = vec![0u8; (nw * nh * 4) as usize];
    conv.CopyPixels(std::ptr::null(), nw * 4, &mut buf).ok()?;
    Some((nw, nh, buf))
}

/// Изображение целиком в виде DIB (заголовок BITMAPINFOHEADER + 32-битные пиксели снизу вверх).
pub unsafe fn load_dib(path: &Path) -> Option<Vec<u8>> {
    let f = factory()?;
    let (frame, w, h) = open(&f, path)?;
    let conv = f.CreateFormatConverter().ok()?;
    conv.Initialize(&frame, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
        .ok()?;
    let stride = (w * 4) as usize;
    let mut top = vec![0u8; stride * h as usize];
    conv.CopyPixels(std::ptr::null(), w * 4, &mut top).ok()?;
    let mut out = Vec::with_capacity(40 + top.len());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for row in (0..h as usize).rev() {
        out.extend_from_slice(&top[row * stride..(row + 1) * stride]);
    }
    Some(out)
}
