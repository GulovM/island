use super::*;

impl App {
    /// Переход на вкладку `i` (позиция в `order`) с кроссфейдом: старое содержимое гаснет и уезжает,
    /// новое проявляется. Закрывает настройки и снимает фокус.
    pub(crate) fn switch_tab(&mut self, i: usize) {
        self.tab_prev = Some(self.cur_id());
        self.tab_dir = if !self.settings_open && i > self.tab { 1.0 } else { -1.0 };
        self.tab_t.value = 0.0;
        self.tab_t.target = 1.0;
        self.tab = i;
        self.settings_open = false;
        self.ui.blur();
    }

    pub(crate) fn tab_row_width(&self) -> f32 {
        self.order.iter().map(|&i| self.tab_widths[i]).sum::<f32>() + TAB_GAP * self.order.len().saturating_sub(1) as f32
    }

    /// Для каждой позиции вкладки: (левый край, ширина) в логических пикселях.
    pub(crate) fn tab_rects(&self) -> Vec<(f32, f32)> {
        let total = self.tab_row_width();
        let mut x = self.win_w as f32 / self.scale / 2.0 - total / 2.0;
        let mut out = Vec::with_capacity(self.order.len());
        for &id in &self.order {
            out.push((x, self.tab_widths[id]));
            x += self.tab_widths[id] + TAB_GAP;
        }
        out
    }

    /// Клик по ряду вкладок. `true` — клик пришёлся на ряд (вкладка сменилась или та же).
    pub(crate) fn click(&mut self, px: i32, py: i32) -> bool {
        if !self.expanded || self.open.value < 0.5 {
            return false;
        }
        let x = px as f32 / self.scale;
        let y = py as f32 / self.scale;
        // Шестерёнка справа в ряду вкладок.
        let (gx, gy) = self.gear_center();
        if (x - gx).hypot(y - gy) <= GEAR_R + 3.0 {
            self.toggle_settings();
            return true;
        }
        if y < TAB_TOP || y > TAB_TOP + TAB_H {
            return false;
        }
        for (i, (tx, tw)) in self.tab_rects().iter().enumerate() {
            if x >= *tx && x <= tx + tw {
                if i != self.tab || self.settings_open {
                    self.switch_tab(i);
                    self.retarget();
                }
                return true;
            }
        }
        false
    }

    // ---------- Клавиатура ----------

    pub(crate) unsafe fn on_key(&mut self, vk: u32) -> bool {
        if self.ui.focus.is_none() {
            return false;
        }
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        // AltGr (польская, немецкая и др. раскладки) приходит как Ctrl+Alt: это ввод символа, не шорткат.
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0 && GetKeyState(VK_MENU.0 as i32) >= 0;
        let ev = match vk {
            v if v == VK_LEFT.0 as u32 => KeyEv::Left(shift),
            v if v == VK_RIGHT.0 as u32 => KeyEv::Right(shift),
            v if v == VK_HOME.0 as u32 => KeyEv::Home(shift),
            v if v == VK_END.0 as u32 => KeyEv::End(shift),
            v if v == VK_BACK.0 as u32 => KeyEv::Backspace,
            v if v == VK_DELETE.0 as u32 => KeyEv::Delete,
            v if v == VK_RETURN.0 as u32 => KeyEv::Enter,
            v if v == VK_ESCAPE.0 as u32 => KeyEv::Escape,
            v if v == VK_UP.0 as u32 => KeyEv::Up,
            v if v == VK_DOWN.0 as u32 => KeyEv::Down,
            v if v == VK_TAB.0 as u32 => KeyEv::Tab,
            0x50 if ctrl => KeyEv::Pin,
            0x41 if ctrl => KeyEv::SelectAll,
            0x43 if ctrl => KeyEv::Copy,
            0x58 if ctrl => KeyEv::Cut,
            0x56 if ctrl => KeyEv::Paste(sys::clipboard_text(self.hwnd).unwrap_or_default()),
            _ => return false,
        };
        self.ui.key(ev);
        if let Some(t) = self.ui.clipboard_out.take() {
            sys::set_clipboard_text(self.hwnd, &t);
        }
        true
    }

    /// Включает/выключает активацию окна под фокус ввода. Остров не NOACTIVATE-окно только
    /// пока в поле есть фокус; после этого активным снова становится прежнее окно.
    pub(crate) unsafe fn sync_focus(&mut self) {
        let want = self.ui.focus.is_some();
        if want && !self.kbd {
            self.kbd = true;
            self.prev_fg = GetForegroundWindow();
            let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32 & !WS_EX_NOACTIVATE.0;
            SetWindowLongW(self.hwnd, GWL_EXSTYLE, ex as i32);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
            self.ui.caret_on = true;
            SetTimer(Some(self.hwnd), CARET_TIMER, CARET_MS, None);
        } else if !want && self.kbd {
            self.kbd = false;
            self.restore_noactivate();
            if GetForegroundWindow() == self.hwnd && !self.prev_fg.0.is_null() && IsWindow(Some(self.prev_fg)).as_bool() {
                let _ = SetForegroundWindow(self.prev_fg);
            }
        }
    }

    pub(crate) unsafe fn restore_noactivate(&mut self) {
        let _ = KillTimer(Some(self.hwnd), CARET_TIMER);
        let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32 | WS_EX_NOACTIVATE.0;
        SetWindowLongW(self.hwnd, GWL_EXSTYLE, ex as i32);
    }

    /// Окно IME (подсказки раскладки/кандидаты) рядом с курсором поля.
    pub(crate) unsafe fn update_ime(&self) {
        if let Some((x, y)) = self.ui.ime_pos {
            sys::ime_set_position(self.hwnd, (x * self.scale) as i32, (y * self.scale) as i32);
        }
    }
}
