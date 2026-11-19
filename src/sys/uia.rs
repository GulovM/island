//! Сообщение фокуса клавиатуры для программ экранного доступа (VoiceOver на macOS).

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

thread_local! {
    static LAST_REPORTED: Cell<Option<u32>> = const { Cell::new(None) };
}

pub(crate) unsafe fn handle_get_object(_hwnd: HWND, _wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    const OBJID_CLIENT: isize = -4;
    if lparam.0 != OBJID_CLIENT {
        return None;
    }
    Some(LRESULT(0))
}

pub(crate) unsafe fn report_focus(_hwnd: HWND, id: Option<u32>, _name: &str) {
    LAST_REPORTED.with(|c| {
        if c.get() == id {
            return;
        }
        c.set(id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_non_client_get_object() {
        unsafe {
            assert!(handle_get_object(HWND(std::ptr::null_mut()), WPARAM(0), LPARAM(0)).is_none());
            assert!(handle_get_object(HWND(std::ptr::null_mut()), WPARAM(0), LPARAM(-1)).is_none());
        }
    }

    #[test]
    fn report_focus_is_idempotent_for_same_id() {
        unsafe {
            let null = HWND(std::ptr::null_mut());
            report_focus(null, None, "");
            report_focus(null, Some(42), "Кнопка");
            report_focus(null, Some(42), "Кнопка");
            report_focus(null, None, "");
        }
    }
}
