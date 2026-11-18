//! Буфер обмена Windows: текст, изображения (DIB), проверка форматов, отправка вставки.

use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use super::wz;

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;

/// Открывает буфер обмена; менеджеры буфера держат его доли секунды, поэтому несколько коротких повторов.
fn open_clipboard(owner: HWND) -> Option<()> {
    for attempt in 0..5 {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            return Some(());
        }
        if attempt < 4 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    None
}

/// Текст из буфера обмена, если там есть Unicode-текст.
pub fn clipboard_text(owner: HWND) -> Option<String> {
    unsafe {
        open_clipboard(owner)?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT) {
            let hg = HGLOBAL(h.0);
            let p = GlobalLock(hg) as *const u16;
            if !p.is_null() {
                // Читаем не дальше размера блока: строка без завершающего нуля не должна выводить за его границу.
                let max = (GlobalSize(hg) / 2).min(1 << 20);
                let mut n = 0;
                while n < max && *p.add(n) != 0 {
                    n += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                let _ = GlobalUnlock(hg);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

/// Копирует сырые байты в глобальный блок памяти и передаёт его в открытый буфер обмена.
unsafe fn set_format_bytes(cf: u32, bytes: &[u8]) -> bool {
    let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return false };
    let p = GlobalLock(hg) as *mut u8;
    if p.is_null() {
        let _ = GlobalFree(Some(hg));
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    let _ = GlobalUnlock(hg);
    // После успешного SetClipboardData память принадлежит системе; при отказе освобождаем сами.
    let ok = SetClipboardData(cf, Some(HANDLE(hg.0))).is_ok();
    if !ok {
        let _ = GlobalFree(Some(hg));
    }
    ok
}

fn wide_bytes(s: &str) -> Vec<u8> {
    wz(s).into_iter().flat_map(u16::to_le_bytes).collect()
}

/// Кладёт Unicode-текст в буфер обмена.
pub fn set_clipboard_text(owner: HWND, text: &str) -> bool {
    unsafe {
        if open_clipboard(owner).is_none() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = set_format_bytes(CF_UNICODETEXT, &wide_bytes(text));
        let _ = CloseClipboard();
        ok
    }
}

unsafe fn set_hdrop_with_copy_effect(files: &[String]) -> bool {
    let hdrop = crate::dnd::build_hdrop(files);
    if !set_format_bytes(CF_HDROP, &hdrop) {
        return false;
    }
    let drop_eff = RegisterClipboardFormatW(w!("Preferred DropEffect"));
    if drop_eff != 0 {
        let _ = set_format_bytes(drop_eff, &1u32.to_le_bytes());
    }
    true
}

/// Кладёт файлы (CF_HDROP + Preferred DropEffect=Copy) и опционально изображение/текст в буфер обмена.
pub fn set_clipboard_files(owner: HWND, files: &[String], text: Option<&str>, dib: Option<&[u8]>) -> bool {
    let norm_files: Vec<String> = files
        .iter()
        .map(|p| p.trim().trim_matches('"').replace('/', "\\"))
        .filter(|p| !p.is_empty())
        .collect();
    if norm_files.is_empty() {
        return false;
    }
    unsafe {
        if open_clipboard(owner).is_none() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = set_hdrop_with_copy_effect(&norm_files);
        if ok {
            if let Some(d) = dib {
                let _ = set_format_bytes(CF_DIB, d);
            }
            let fallback;
            let txt_str = match text {
                Some(t) => t,
                None => {
                    fallback = norm_files.join("\r\n");
                    &fallback
                }
            };
            let _ = set_format_bytes(CF_UNICODETEXT, &wide_bytes(txt_str));
        }
        let _ = CloseClipboard();
        ok
    }
}

/// В буфере обмена данные, помеченные «не записывать» (менеджеры паролей).
pub fn clipboard_excluded() -> bool {
    unsafe {
        [w!("ExcludeClipboardContentFromMonitorProcessing"), w!("CanIncludeInClipboardHistory")]
            .iter()
            .enumerate()
            .any(|(i, name)| {
                let f = RegisterClipboardFormatW(*name);
                // Первый формат — «исключить», если он есть; второй — «можно», если его нет, не решает.
                i == 0 && f != 0 && IsClipboardFormatAvailable(f).is_ok()
            })
    }
}

/// Владелец буфера обмена — наше окно (мы сами записали текст).
pub fn clipboard_owned_by(hwnd: HWND) -> bool {
    unsafe { GetClipboardOwner().map(|h| h == hwnd).unwrap_or(false) }
}

/// В буфере обмена есть Unicode-текст.
pub fn clipboard_has_text() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() }
}

/// В буфере обмена есть список файлов (CF_HDROP).
pub fn clipboard_has_files() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_HDROP).is_ok() }
}

/// Список путей файлов из буфера обмена (CF_HDROP).
pub fn clipboard_files(owner: HWND) -> Option<Vec<String>> {
    unsafe {
        open_clipboard(owner)?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_HDROP) {
            let hdrop = HDROP(h.0);
            let count = DragQueryFileW(hdrop, u32::MAX, None);
            let mut files = Vec::with_capacity(count as usize);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let n = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
                let path = String::from_utf16_lossy(&buf[..n.min(len)]);
                if !path.trim().is_empty() {
                    files.push(path);
                }
            }
            if !files.is_empty() {
                out = Some(files);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

// ---------- Изображения в буфере обмена ----------

const CF_DIB: u32 = 8;

/// В буфере обмена есть растровое изображение (DIB; из DIBV5 и bitmap Windows синтезирует его сама).
pub fn clipboard_has_image() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_DIB).is_ok() }
}

/// Изображение из буфера обмена: (ширина, высота, BGRA сверху вниз, непрозрачное).
pub fn clipboard_image(owner: HWND) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        open_clipboard(owner)?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_DIB) {
            let hg = HGLOBAL(h.0);
            let size = GlobalSize(hg);
            let p = GlobalLock(hg) as *const u8;
            if !p.is_null() {
                out = parse_dib(std::slice::from_raw_parts(p, size));
                let _ = GlobalUnlock(hg);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

/// Разбор DIB (24/32 бита, BI_RGB или BI_BITFIELDS) в BGRA сверху вниз.
fn parse_dib(d: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if d.len() < 40 {
        return None;
    }
    let u32le = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    let header = u32le(0) as usize;
    let (w, hraw) = (u32le(4) as i32, u32le(8) as i32);
    let bits = u16::from_le_bytes([d[14], d[15]]) as usize;
    let comp = u32le(16);
    if w <= 0 || hraw == 0 || (bits != 32 && bits != 24) || (comp != 0 && comp != 3) || header < 40 {
        return None;
    }
    let (w, h) = (w as usize, hraw.unsigned_abs() as usize);
    if w * h > 50_000_000 {
        return None;
    }
    let top_down = hraw < 0;
    let mut off = header;
    if header == 40 && comp == 3 {
        off += 12;
    }
    let stride = (w * bits).div_ceil(32) * 4;
    if d.len() < off + stride * h {
        return None;
    }
    let step = bits / 8;
    let mut out = vec![255u8; w * h * 4];
    for row in 0..h {
        let src = off + stride * if top_down { row } else { h - 1 - row };
        for x in 0..w {
            let s = src + x * step;
            let o = (row * w + x) * 4;
            out[o..o + 3].copy_from_slice(&d[s..s + 3]);
        }
    }
    Some((w as u32, h as u32, out))
}

/// Кладёт изображение (готовый DIB: заголовок + пиксели снизу вверх) и опционально временный файл
/// PNG (`CF_HDROP`) в буфер обмена, чтобы вставка работала и в редакторах, и в папках Проводника.
pub fn set_clipboard_dib_with_file(owner: HWND, dib: &[u8], file_path: Option<&str>) -> bool {
    unsafe {
        if open_clipboard(owner).is_none() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = set_format_bytes(CF_DIB, dib);
        if ok {
            if let Some(path) = file_path.map(str::trim).filter(|p| !p.is_empty()) {
                let _ = set_hdrop_with_copy_effect(&[path.to_string()]);
            }
        }
        let _ = CloseClipboard();
        ok
    }
}

/// Кладёт изображение (готовый DIB: заголовок + пиксели снизу вверх) в буфер обмена.
pub fn set_clipboard_dib(owner: HWND, dib: &[u8]) -> bool {
    set_clipboard_dib_with_file(owner, dib, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_24bit_bottom_up() {
        // 2×1, 24 бита, строка 8 байт (6 + 2 выравнивание).
        let mut d = vec![0u8; 40];
        d[0] = 40;
        d[4] = 2;
        d[8] = 1;
        d[12] = 1;
        d[14] = 24;
        d.extend_from_slice(&[1, 2, 3, 4, 5, 6, 0, 0]);
        let (w, h, px) = parse_dib(&d).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(&px[..8], &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn dib_rejects_palette() {
        let mut d = vec![0u8; 48];
        d[0] = 40;
        d[4] = 1;
        d[8] = 1;
        d[14] = 8;
        assert!(parse_dib(&d).is_none());
    }
}
