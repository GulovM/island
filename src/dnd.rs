//! Перетаскивание (drag-and-drop) на острове и передача элементов в другие приложения.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::HWND;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropEffect(pub u32);
#[cfg(test)]
pub const DROPEFFECT_NONE: DropEffect = DropEffect(0);
#[cfg(test)]
pub const DROPEFFECT_COPY: DropEffect = DropEffect(1);
#[cfg(test)]
pub const DROPEFFECT_LINK: DropEffect = DropEffect(4);

static SELF_DRAG: AtomicBool = AtomicBool::new(false);

pub fn set_full(_full: bool) {}

pub fn is_self_drag() -> bool {
    SELF_DRAG.load(Ordering::Relaxed)
}

pub fn init() -> bool {
    true
}

pub unsafe fn register(_hwnd: HWND) -> bool {
    true
}

pub unsafe fn revoke(_hwnd: HWND) {}

pub enum Out {
    Files(Vec<String>),
    Image(std::path::PathBuf),
    Url(String),
    Text(String),
    Multi {
        files: Vec<String>,
        images: Vec<std::path::PathBuf>,
        text: Option<String>,
    },
}

#[cfg(test)]
pub(crate) fn wide_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity((s.encode_utf16().count() + 1) * 2);
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

#[cfg(test)]
pub(crate) fn build_hdrop(paths: &[String]) -> Vec<u8> {
    let mut out = vec![0u8; 20];
    out[0..4].copy_from_slice(&20u32.to_le_bytes());
    out[16..20].copy_from_slice(&1u32.to_le_bytes());
    for p in paths {
        let norm = p.replace('/', "\\");
        for u in norm.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out
}

#[cfg(test)]
pub(crate) fn choose_effect(allowed: u32, supported: bool) -> u32 {
    if !supported {
        return DROPEFFECT_NONE.0;
    }
    if (allowed & DROPEFFECT_COPY.0) != 0 {
        DROPEFFECT_COPY.0
    } else if (allowed & DROPEFFECT_LINK.0) != 0 {
        DROPEFFECT_LINK.0
    } else {
        DROPEFFECT_NONE.0
    }
}

pub(crate) fn plain_png(sealed: &Path) -> Option<String> {
    let tmp = std::env::temp_dir().join("island-drop.png");
    let raw = std::fs::read(sealed).ok()?;
    let bytes = crate::sys::unseal(&raw)?;
    if bytes.len() < 8 {
        return None;
    }
    if std::fs::write(&tmp, bytes).is_ok() {
        Some(tmp.to_string_lossy().into_owned())
    } else {
        None
    }
}

pub unsafe fn drag_out(out: Out) -> bool {
    let hwnd = HWND::default();
    match out {
        Out::Text(t) | Out::Url(t) => crate::sys::set_clipboard_text(hwnd, &t),
        Out::Files(files) => crate::sys::set_clipboard_files(hwnd, &files, None, None),
        Out::Image(sealed) => {
            if let Some(p) = plain_png(&sealed) {
                crate::sys::set_clipboard_files(hwnd, &[p], None, None)
            } else {
                false
            }
        }
        Out::Multi { mut files, images, text } => {
            for img in images {
                if let Some(p) = plain_png(&img) {
                    files.push(p);
                }
            }
            if !files.is_empty() {
                crate::sys::set_clipboard_files(hwnd, &files, text.as_deref(), None)
            } else if let Some(t) = text {
                crate::sys::set_clipboard_text(hwnd, &t)
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdrop_layout_matches_dropfiles() {
        let b = build_hdrop(&["C:\\a.txt".to_string(), "D:/folder/б.png".to_string()]);
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 20, "смещение списка");
        assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 1, "fWide");
        let list: Vec<u16> = b[20..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(list.last(), Some(&0));
        assert_eq!(list[list.len() - 2], 0, "список заканчивается двойным нулём");
        let s = String::from_utf16_lossy(&list);
        assert_eq!(s.split('\0').filter(|x| !x.is_empty()).collect::<Vec<_>>(), ["C:\\a.txt", "D:\\folder\\б.png"]);
    }

    #[test]
    fn effect_prefers_copy_and_refuses_without_support() {
        let all = DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0 | 2;
        assert_eq!(choose_effect(all, true), DROPEFFECT_COPY.0);
        assert_eq!(choose_effect(DROPEFFECT_LINK.0, true), DROPEFFECT_LINK.0);
        assert_eq!(choose_effect(2, true), DROPEFFECT_NONE.0, "только перенос не подходит");
        assert_eq!(choose_effect(all, false), DROPEFFECT_NONE.0);
    }

    #[test]
    fn wide_text_is_nul_terminated_utf16() {
        assert_eq!(wide_bytes("a"), [b'a', 0, 0, 0]);
        assert_eq!(wide_bytes("").len(), 2);
    }
}
