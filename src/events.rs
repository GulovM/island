use super::*;

impl App {
    /// Горячая клавиша: раскрыть остров на вкладке Apps и поставить фокус в поиск.
    pub(crate) unsafe fn on_hotkey(&mut self) -> bool {
        if self.hidden {
            return false;
        }
        // Вкладка Apps выключена в настройках — горячей клавише нечего открывать.
        let Some(i) = self.order.iter().position(|&t| t == tabs::APPS) else { return false };
        if i != self.tab || self.settings_open {
            self.switch_tab(i);
        }
        // Остров переезжает на монитор с курсором (как при наведении на горячую зону).
        let mut p = POINT::default();
        if GetCursorPos(&mut p).is_ok() {
            let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
            if mon != self.mon {
                self.move_to(mon);
            }
        }
        self.disarm_leave();
        self.set_expanded(true);
        self.ui.focus = Some(tabs::id(tabs::APPS, 0));
        self.sync_focus();
        true
    }

    /// Обрабатывает событие, возвращает `true`, если нужна перерисовка.
    pub(crate) unsafe fn handle(&mut self, e: Ev) -> bool {
        match e {
            Ev::Move => self.on_move(),
            Ev::MouseLeft => {
                self.check_hover();
                false
            }
            Ev::LeaveTimer => self.on_leave_timer(),
            Ev::Down(x, y) => {
                let switched = self.click(x, y);
                if !switched && self.expanded {
                    self.ui.down = true;
                    self.ui.pressed = true;
                    // Долгое нажатие на пустом месте вкладки с виджетами включает правку.
                    if self.pencil_visible() && !self.tabs.editing(self.normal_id()) && y as f32 / self.scale > 31.0 {
                        self.press_pos = Some((x, y));
                        SetTimer(Some(self.hwnd), LONG_TIMER, LONG_MS, None);
                    }
                }
                true
            }
            Ev::Up => {
                let was = self.ui.down;
                self.ui.down = false;
                self.ui.released = was;
                self.press_pos = None;
                let _ = KillTimer(Some(self.hwnd), LONG_TIMER);
                self.check_hover();
                true
            }
            Ev::LongPress => {
                let _ = KillTimer(Some(self.hwnd), LONG_TIMER);
                let Some((px, py)) = self.press_pos.take() else { return false };
                if !self.expanded || !self.ui.down || !self.ui.press_free() || !self.pencil_visible() {
                    return false;
                }
                // Рука не должна была сдвинуться: иначе это начало перетаскивания, а не удержание.
                let mut p = POINT::default();
                if GetCursorPos(&mut p).is_err() {
                    return false;
                }
                let moved = ((p.x - self.win_x - px) as f32).hypot((p.y - self.win_y - py) as f32);
                if moved > 6.0 * self.scale {
                    return false;
                }
                self.toggle_edit();
                true
            }
            Ev::Wheel(d) => {
                if self.expanded {
                    self.ui.wheel += d as f32 / 120.0;
                }
                self.expanded
            }
            Ev::Char(c) => {
                if self.ui.focus.is_some() && c >= 0x20 && c != 0x7f {
                    // Суррогатные пары приходят двумя WM_CHAR: сводим их в один символ.
                    if (0xD800..0xDC00).contains(&c) {
                        self.high_surrogate = c;
                    } else {
                        let ch = if (0xDC00..0xE000).contains(&c) && self.high_surrogate != 0 {
                            let hi = std::mem::take(&mut self.high_surrogate);
                            char::decode_utf16([hi as u16, c as u16]).next().and_then(|r| r.ok())
                        } else {
                            char::from_u32(c)
                        };
                        if let Some(ch) = ch {
                            self.ui.key(KeyEv::Char(ch));
                        }
                    }
                }
                true
            }
            Ev::Key(vk) => self.on_key(vk),
            Ev::Deactivated => {
                if self.kbd {
                    // Пользователь кликнул в другое окно: возвращать ему фокус не нужно.
                    self.kbd = false;
                    self.ui.blur();
                    self.restore_noactivate();
                    self.check_hover();
                    return true;
                }
                false
            }
            Ev::Caret => {
                self.ui.caret_on = !self.ui.caret_on;
                self.ui.focus.is_some()
            }
            Ev::Recheck => self.refresh_environment(),
            Ev::Tray(m) if m == WM_RBUTTONUP || m == WM_LBUTTONUP => self.tray_menu(),
            Ev::Tray(_) => false,
            Ev::Hot(h) => self.on_hot(h),
            Ev::Hotkey => self.on_hotkey(),
            Ev::Display => self.reconfigure(),
            Ev::Resume => self.on_resume(),
            Ev::Lock => self.on_lock(),
            Ev::RecoverTimer => self.try_recover(),
            Ev::TaskbarCreated => {
                self.add_tray();
                false
            }
            Ev::Poll => match self.shown_tab {
                Some(t) => self.tabs.poll(t),
                None => false,
            },
            Ev::Alarm => self.tabs.alarm() && self.expanded,
            Ev::Bg => {
                let tab = self.tabs.on_bg() && self.expanded;
                let media = self.live.sync_media(&bg::snapshot());
                if media {
                    self.retarget();
                    if !self.expanded {
                        self.trim_pending = true;
                    }
                }
                let net = self.face.sync_net() && self.face.visible();
                // Проверка или скачивание обновления закончились — страница настроек перерисуется.
                tab || media || net || (self.expanded && self.settings_open)
            }
            Ev::UpdateTimer => {
                update::tick(self.cfg.auto_update);
                false
            }
            Ev::FaceTick => {
                if !self.face.visible() {
                    let _ = KillTimer(Some(self.hwnd), FACE_TIMER);
                    return false;
                }
                self.face.tick();
                SetTimer(Some(self.hwnd), FACE_TIMER, self.face.next_tick_ms(), None);
                // Уровень сигнала Wi-Fi меняется без событий — раз в минуту просим перечитать.
                bg::send(bg::Cmd::Net);
                true
            }
            Ev::SysColors => self.cfg.system_colors && self.apply_theme(),
            Ev::Volume(level, muted) => {
                if self.live.volume_changed(level, muted) {
                    self.show_toast(live::Toast::Volume { level, muted })
                } else {
                    false
                }
            }
            Ev::PowerChange => match self.live.power_event() {
                Some(t) => self.show_toast(t),
                None => false,
            },
            Ev::ToastTimer => {
                let _ = KillTimer(Some(self.hwnd), TOAST_TIMER);
                let was = self.live.clear_toast();
                self.retarget();
                self.trim_pending = true;
                was
            }
            Ev::LiveTick => self.live_form() == live::Form::Mini,
            Ev::RightClick => self.toggle_strip_mode(),
            Ev::Clip => self.on_clipboard(),
            Ev::PasteNow => {
                let _ = KillTimer(Some(self.hwnd), PASTE_TIMER);
                sys::send_paste();
                false
            }
        }
    }

    /// Новая запись в буфере обмена: пишем в историю, даже когда остров свёрнут.
    pub(crate) unsafe fn on_clipboard(&mut self) -> bool {
        // Собственные записи острова и данные менеджеров паролей в историю не идут.
        if sys::clipboard_owned_by(self.hwnd) || sys::clipboard_excluded() {
            return false;
        }
        let has_text = sys::clipboard_has_text();
        if !has_text && !sys::clipboard_has_image() {
            return false;
        }
        // Буфер может быть занят источником копирования — несколько коротких попыток.
        if has_text {
            let mut text = None;
            for _ in 0..5 {
                text = sys::clipboard_text(self.hwnd);
                if text.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            return match text {
                Some(t) => self.tabs.on_clipboard(t) && self.expanded,
                None => false,
            };
        }
        for _ in 0..5 {
            if let Some((w, h, px)) = sys::clipboard_image(self.hwnd) {
                self.tabs.on_clipboard_image(w, h, px);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        false
    }

    /// Приводит вкладки и их таймеры в соответствие с тем, что сейчас на экране, и
    /// исполняет просьбы вкладок. Возвращает `true`, если нужен кадр.
    pub(crate) unsafe fn sync_tabs(&mut self) -> bool {
        let mut dirty = false;
        let want = if self.expanded { Some(self.cur_id()) } else { None };
        if want != self.shown_tab {
            if let Some(old) = self.shown_tab {
                self.tabs.hide(old);
            }
            self.shown_tab = want;
            if let Some(t) = want {
                if t == tabs::SETTINGS {
                    self.tabs.settings.load(&self.cfg);
                }
                dirty |= self.tabs.show(t);
            }
        }

        // Изменения со страницы настроек: применяем сразу, пока ползунок или палитру тянут.
        if let Some(c) = self.tabs.settings.take_change() {
            self.apply_settings(c);
            dirty = true;
        }
        if self.save_pending && !self.ui.down {
            self.save_pending = false;
            config::save(&self.cfg, &TABS);
        }
        // Остров светится, пока тянут ползунок радиуса углов.
        let glow = self.expanded && self.settings_open && self.tabs.settings.glowing();
        self.glow.target = if glow { 1.0 } else { 0.0 };

        // Минутный таймер часов нужен, только пока свёрнутый остров показывает их.
        let face_on = self.face.visible();
        if face_on != self.face_tick_on {
            self.face_tick_on = face_on;
            if face_on {
                self.face.tick();
                SetTimer(Some(self.hwnd), FACE_TIMER, self.face.next_tick_ms(), None);
                dirty = true;
            } else {
                let _ = KillTimer(Some(self.hwnd), FACE_TIMER);
            }
        }

        let poll = want.is_some_and(|t| self.tabs.wants_poll(t));
        if poll != self.poll_on {
            self.poll_on = poll;
            if poll {
                SetTimer(Some(self.hwnd), POLL_TIMER, POLL_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), POLL_TIMER);
            }
        }

        // Таймер Today дошёл до нуля — уведомление в свёрнутой форме (звук играет сама вкладка).
        if self.tabs.take_timer_fired() {
            dirty |= self.show_toast(live::Toast::Timer);
        }

        // Тик кадров нужен, только пока на острове виден играющий плеер.
        let tick = self.live_form() == live::Form::Mini && self.live.playing();
        if tick != self.live_tick_on {
            self.live_tick_on = tick;
            if tick {
                SetTimer(Some(self.hwnd), LIVE_TIMER, LIVE_TICK_MS, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), LIVE_TIMER);
            }
        }

        let alarm = self.tabs.alarm_deadline();
        if alarm != self.alarm_at {
            self.alarm_at = alarm;
            match alarm {
                Some(at) => {
                    let ms = at.saturating_duration_since(Instant::now()).as_millis() as u32;
                    SetTimer(Some(self.hwnd), ALARM_TIMER, ms.saturating_add(30), None);
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), ALARM_TIMER);
                }
            }
        }

        for a in std::mem::take(&mut self.tabs.actions) {
            dirty = true;
            match a {
                tabs::Action::Paste(text) => {
                    sys::set_clipboard_text(self.hwnd, &text);
                    self.disarm_leave();
                    self.set_expanded(false);
                    SetTimer(Some(self.hwnd), PASTE_TIMER, PASTE_DELAY_MS, None);
                }
                tabs::Action::Copy(text) => {
                    sys::set_clipboard_text(self.hwnd, &text);
                }
                tabs::Action::PasteImage(file) => {
                    if self.put_image(&file) {
                        self.disarm_leave();
                        self.set_expanded(false);
                        SetTimer(Some(self.hwnd), PASTE_TIMER, PASTE_DELAY_MS, None);
                    }
                }
                tabs::Action::CopyImage(file) => {
                    self.put_image(&file);
                }
                tabs::Action::Collapse => {
                    self.disarm_leave();
                    self.set_expanded(false);
                }
                tabs::Action::Layout(tab, text) => {
                    if let (Some(slot), Some(key)) = (self.cfg.layouts.get_mut(tab), config::LAYOUT_KEYS.get(tab)) {
                        config::save_str(key, &text);
                        *slot = text;
                    }
                }
                tabs::Action::Resize => self.retarget(),
            }
        }
        dirty
    }

    /// Кладёт PNG из истории в буфер обмена как изображение.
    pub(crate) unsafe fn put_image(&self, file: &str) -> bool {
        let path = tabs::clipboard::clip_dir().join(file);
        match img::load_dib(&path) {
            Some(dib) => sys::set_clipboard_dib(self.hwnd, &dib),
            None => {
                log::error(&format!("не удалось прочитать изображение {file}"));
                false
            }
        }
    }
}
