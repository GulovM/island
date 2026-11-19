//! Буфер обмена macOS (`pbpaste` / `pbcopy` / `osascript`) и разбор DIB для тестов.

#[cfg(target_os = "macos")]
use std::io::Write;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

use windows::Win32::Foundation::HWND;

pub fn clipboard_text(_owner: HWND) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("/usr/bin/pbpaste").output().ok()?;
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).into_owned();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

pub fn set_clipboard_text(_owner: HWND, text: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = Command::new("/usr/bin/pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().is_ok_and(|s| s.success());
        }
    }
    let _ = text;
    false
}

pub fn set_clipboard_files(_owner: HWND, files: &[String], text: Option<&str>, _dib: Option<&[u8]>) -> bool {
    if let Some(t) = text {
        return set_clipboard_text(HWND::default(), t);
    }
    if let Some(first) = files.first() {
        return set_clipboard_text(HWND::default(), first);
    }
    false
}

pub fn clipboard_excluded() -> bool {
    false
}

pub fn clipboard_owned_by(_hwnd: HWND) -> bool {
    false
}

pub fn clipboard_has_text() -> bool {
    clipboard_text(HWND::default()).is_some()
}

pub fn clipboard_has_files() -> bool {
    false
}

pub fn clipboard_files(_owner: HWND) -> Option<Vec<String>> {
    None
}

pub fn clipboard_has_image() -> bool {
    false
}

pub fn clipboard_image(_owner: HWND) -> Option<(u32, u32, Vec<u8>)> {
    None
}

#[allow(dead_code)]
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

pub fn set_clipboard_dib_with_file(_owner: HWND, _dib: &[u8], file_path: Option<&str>) -> bool {
    if let Some(p) = file_path {
        #[cfg(target_os = "macos")]
        {
            let esc = p.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!("set the clipboard to (POSIX file \"{esc}\")");
            if Command::new("/usr/bin/osascript")
                .args(["-e", &script])
                .status()
                .is_ok_and(|s| s.success())
            {
                return true;
            }
        }
        return set_clipboard_text(HWND::default(), p);
    }
    false
}

pub fn set_clipboard_dib(owner: HWND, dib: &[u8]) -> bool {
    set_clipboard_dib_with_file(owner, dib, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_24bit_bottom_up() {
        let mut d = vec![0u8; 40 + 8];
        d[0..4].copy_from_slice(&40u32.to_le_bytes());
        d[4..8].copy_from_slice(&2i32.to_le_bytes());
        d[8..12].copy_from_slice(&1i32.to_le_bytes());
        d[14..16].copy_from_slice(&24u16.to_le_bytes());
        d[40..46].copy_from_slice(&[10, 20, 30, 40, 50, 60]);
        let (w, h, px) = parse_dib(&d).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(px, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn dib_rejects_palette() {
        let mut d = vec![0u8; 48];
        d[0..4].copy_from_slice(&40u32.to_le_bytes());
        d[4..8].copy_from_slice(&2i32.to_le_bytes());
        d[8..12].copy_from_slice(&1i32.to_le_bytes());
        d[14..16].copy_from_slice(&8u16.to_le_bytes());
        assert!(parse_dib(&d).is_none());
    }
}
