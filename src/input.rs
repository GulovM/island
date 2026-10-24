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
        self.hold_leave();
        self.ui.blur();
        if let Some(&id) = self.order.get(i) {
            crate::state::set_last_tab(TABS[id]);
        }
        // В буфере обмена клавиатура сразу управляет списком: фокус в поиске.
        if self.order.get(i) == Some(&tabs::CLIPBOARD) {
            self.ui.focus = Some(tabs::id(tabs::CLIPBOARD, 0));
        }
    }

    pub(crate) fn tab_row_width(&self) -> f32 {
        self.order.iter().map(|&i| self.tab_widths[i]).sum::<f32>() + TAB_GAP * self.order.len().saturating_sub(1) as f32
    }

    /// Для каждой позиции вкладки: (левый край, ширина) в логических пикселях.
    pub(crate) fn tab_rects(&self) -> Vec<(f32, f32)> {
        let total = self.tab_row_width();
        let mut x = self.win_w as f32 / self.scale / 2.0 - total / 2.0 + (TAB_SIDE_LEFT - TAB_SIDE_RIGHT) / 2.0;
        let mut out = Vec::with_capacity(self.order.len());
        for &id in &self.order {
            out.push((x, self.tab_widths[id]));
            x += self.tab_widths[id] + TAB_GAP;
        }
        out
    }

    /// Карандаш показан, когда на экране вкладка, виджеты которой можно настраивать.
    pub(crate) fn pencil_visible(&self) -> bool {
        self.expanded && !self.settings_open && self.tabs.editable(self.normal_id())
    }

    /// Центр карандаша — левее шестерёнки настроек в ряду вкладок.
    pub(crate) fn pencil_center(&self) -> (f32, f32) {
        let (gx, gy) = self.gear_center();
        (gx - 24.0, gy)
    }

    /// Включает или выключает режим правки текущей вкладки; остров подстраивает высоту под полосу кнопок.
    pub(crate) fn toggle_edit(&mut self) {
        if self.tabs.toggle_edit(self.normal_id()) {
            self.hold_leave();
            self.ui.blur();
            self.retarget();
        }
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
        // Карандаш справа (левее шестерёнки): режим правки виджетов (только на вкладках с виджетами).
        if self.pencil_visible() {
            let (ex, ey) = self.pencil_center();
            if (x - ex).hypot(y - ey) <= GEAR_R + 3.0 {
                self.toggle_edit();
                return true;
            }
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
        // Фокус у острова только ради уведомления о таймере: Esc закрывает его, остальное не нужно.
        if self.live.alert() && self.ui.focus == Some(NOTICE_FOCUS) {
            if vk != VK_ESCAPE.0 as u32 {
                return false;
            }
            self.tabs.stop_alarm();
            return self.dismiss_toast();
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
            self.ensure_topmost(true);
            if !SetForegroundWindow(self.hwnd).as_bool() {
                // Фоновому процессу Windows не отдаёт передний план просто так (таймер сработал сам по себе):
                // на миг присоединяемся к потоку окна, которое сейчас в фокусе.
                let fg_thread = GetWindowThreadProcessId(self.prev_fg, None);
                let me = windows::Win32::System::Threading::GetCurrentThreadId();
                if fg_thread != 0 && fg_thread != me {
                    let _ = windows::Win32::System::Threading::AttachThreadInput(me, fg_thread, true);
                    let _ = SetForegroundWindow(self.hwnd);
                    let _ = windows::Win32::System::Threading::AttachThreadInput(me, fg_thread, false);
                }
            }
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
        self.ensure_topmost(true);
    }

    /// Окно IME (подсказки раскладки/кандидаты) рядом с курсором поля.
    pub(crate) unsafe fn update_ime(&self) {
        if let Some((x, y)) = self.ui.ime_pos {
            sys::ime_set_position(self.hwnd, (x * self.scale) as i32, (y * self.scale) as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pencil_and_gear_layout_no_overlap() {
        // Проверяем взаимное расположение кнопок справа и ряда вкладок:
        // шестерёнка справа, карандаш левее шестерёнки, ряд вкладок не налезает на карандаш.
        let win_w = crate::WIN_LOGICAL.0;
        let total = 400.0;
        let island_w = total + TAB_SIDE_LEFT + TAB_SIDE_RIGHT;
        let half = island_w / 2.0;
        let cx = win_w / 2.0;
        let gx = cx + half - 16.0;
        let gy = TAB_TOP + TAB_H / 2.0;
        let ex = gx - 24.0;
        let ey = gy;

        // Хитбоксы кнопок радиуса GEAR_R + 3.0:
        let hit_r = GEAR_R + 3.0;
        let gear_left = gx - hit_r;
        let pencil_right = ex + hit_r;
        let pencil_left = ex - hit_r;
        assert!(pencil_right <= gear_left, "хитбоксы карандаша и шестерёнки не должны перекрываться");
        assert_eq!(ey, gy, "карандаш и шестерёнка на одной высоте");

        // Ряд вкладок при минимальной ширине острова:
        let first_tab_x = cx - total / 2.0 + (TAB_SIDE_LEFT - TAB_SIDE_RIGHT) / 2.0;
        let last_tab_right = first_tab_x + total;
        let left_edge = cx - half;
        let right_edge = cx + half;

        assert!((first_tab_x - left_edge - TAB_SIDE_LEFT).abs() < 0.001, "левый отступ должен быть равен TAB_SIDE_LEFT");
        assert!((right_edge - last_tab_right - TAB_SIDE_RIGHT).abs() < 0.001, "правый отступ должен быть равен TAB_SIDE_RIGHT");
        assert!(last_tab_right <= pencil_left, "ряд вкладок не должен налезать на хитбокс карандаша");
        assert!(pencil_left - last_tab_right >= TAB_GAP, "зазор от вкладки до хитбокса карандаша не меньше TAB_GAP");
    }
}
