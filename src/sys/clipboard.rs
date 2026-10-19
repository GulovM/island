//! Буфер обмена Windows: текст, изображения (DIB), проверка форматов, отправка вставки.

use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL,
};

use super::wz;

const CF_UNICODETEXT: u32 = 13;

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

/// Кладёт Unicode-текст в буфер обмена.
pub fn set_clipboard_text(owner: HWND, text: &str) -> bool {
    unsafe {
        let wide = wz(text);
        let bytes = wide.len() * 2;
        let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, bytes) else { return false };
        let p = GlobalLock(hg) as *mut u16;
        if p.is_null() {
            let _ = GlobalFree(Some(hg));
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let _ = GlobalUnlock(hg);
        if open_clipboard(owner).is_none() {
            let _ = GlobalFree(Some(hg));
            return false;
        }
        let _ = EmptyClipboard();
        // После успешного SetClipboardData память принадлежит системе; при отказе освобождаем сами.
        let ok = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hg.0))).is_ok();
        if !ok {
            let _ = GlobalFree(Some(hg));
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

/// Эмулирует Ctrl+V в окне, которое сейчас в фокусе.
pub fn send_paste() {
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(VK_CONTROL, false), key(VIRTUAL_KEY(0x56), false), key(VIRTUAL_KEY(0x56), true), key(VK_CONTROL, true)];
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

/// В буфере обмена есть Unicode-текст.
pub fn clipboard_has_text() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() }
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
        OpenClipboard(Some(owner)).ok()?;
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

/// Кладёт изображение (готовый DIB: заголовок + пиксели снизу вверх) в буфер обмена.
pub fn set_clipboard_dib(owner: HWND, dib: &[u8]) -> bool {
    unsafe {
        let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, dib.len()) else { return false };
        let p = GlobalLock(hg) as *mut u8;
        if p.is_null() {
            return false;
        }
        std::ptr::copy_nonoverlapping(dib.as_ptr(), p, dib.len());
        let _ = GlobalUnlock(hg);
        if OpenClipboard(Some(owner)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = SetClipboardData(CF_DIB, Some(HANDLE(hg.0))).is_ok();
        let _ = CloseClipboard();
        ok
    }
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
