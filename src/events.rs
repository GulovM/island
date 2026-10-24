use super::*;
use crate::app::LiveTick;

impl App {
    /// Горячая клавиша: раскрыть остров на вкладке Apps и поставить фокус в поиск.
    pub(crate) unsafe fn on_hotkey(&mut self) -> bool {
        if self.hidden {
            return false;
        }
        // Вкладка Apps выключена в настройках — горячей клавише нечего открывать.
        let Some(i) = self.order.iter().position(|&t| t == tabs::APPS) else { return false };
        self.open_from_hotkey(i, tabs::APPS);
        true
    }

    /// Раскрывает остров на вкладке в позиции `i` (индекс `tab` в `TABS`) и ставит фокус в её поле поиска.
    pub(crate) unsafe fn open_from_hotkey(&mut self, i: usize, tab: usize) {
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
        self.ui.focus = Some(tabs::id(tab, 0));
        self.sync_focus();
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
                if self.hits_toast_close(x, y) {
                    self.tabs.stop_alarm();
                    self.dismiss_toast();
                    return true;
                }
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
                if self.ui.focus.is_some() && !self.live.alert() && !self.tabs.recording_hotkey() && c >= 0x20 && c != 0x7f {
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
            Ev::Hotkey(id) if id == QUICK_HOTKEY_ID => self.on_quick_hotkey(),
            Ev::Hotkey(_) => self.on_hotkey(),
            Ev::Shot => {
                if self.cfg.hide_on_screenshot {
                    self.hide_for_shot();
                }
                false
            }
            Ev::Display => self.reconfigure(),
            Ev::Resume => self.on_resume(),
            Ev::Lock => {
                self.locked = true;
                self.on_lock()
            }
            Ev::Unlock => {
                self.locked = false;
                // Пропущенные за время блокировки напоминания показываем сразу, не дожидаясь тика.
                let shown = self.show_event_notice();
                self.on_resume() | shown
            }
            Ev::ScreenPower(on) => {
                self.screen_off = !on;
                on && self.show_event_notice()
            }
            Ev::EventTick => self.show_event_notice(),
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
                update::hourly(self.cfg.auto_update);
                // Следующая проверка — в начале следующего часа, а не через 60 минут от этой.
                SetTimer(Some(self.hwnd), UPDATE_TIMER, update::ms_to_next_hour(), None);
                // Заодно история буфера теряет записи старше суток.
                self.tabs.prune_clip() && self.expanded
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
            Ev::SysColors => self.on_sys_colors(),
            Ev::SysColorsRetry => self.on_sys_colors_retry(),
            Ev::Volume(level, muted) => {
                // Вкладка Controls берёт значения из снимка: обновляем его сразу, а не по секундному опросу.
                bg::send(bg::Cmd::Tick);
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
                // Уведомление о таймере живёт, пока звучит сигнал: закрываем его вслед за звуком.
                if self.live.alert() && self.tabs.alarm_playing() {
                    SetTimer(Some(self.hwnd), TOAST_TIMER, 400, None);
                    return false;
                }
                self.dismiss_toast()
            }
            Ev::LiveTick => {
                // Раз в секунду таймер заводится заново под следующую смену цифры.
                if self.live_tick_on == LiveTick::Seconds {
                    if let Some(t) = self.tabs.timer_view() {
                        SetTimer(Some(self.hwnd), LIVE_TIMER, t.next_ms, None);
                    }
                }
                self.live_form() == live::Form::Mini || self.live.timer().is_some()
            }
            Ev::RightClick => self.toggle_strip_mode(),
            Ev::Clip => self.on_clipboard(),
            Ev::PasteNow => {
                let _ = KillTimer(Some(self.hwnd), PASTE_TIMER);
                sys::send_paste();
                false
            }
            Ev::DragEnter(ok) => self.on_drag_enter(ok),
            Ev::DragLeave => self.on_drag_leave(),
            Ev::Dropped(items) => self.on_dropped(items),
            Ev::DropTimer => self.on_drop_timer(),
        }
    }

    /// Системные цвета или тема Windows изменились (или сменились обои рабочего стола).
    /// Запускаем немедленную проверку и планируем повторные проверки на случай задержки/гонки в реестре.
    pub(crate) unsafe fn on_sys_colors(&mut self) -> bool {
        let delay_ms = self.color_retry.arm();
        SetTimer(Some(self.hwnd), SYS_COLOR_TIMER, delay_ms, None);
        self.check_sys_colors()
    }

    /// Повторная проверка системных цветов по таймеру (реестр мог обновиться с задержкой).
    pub(crate) unsafe fn on_sys_colors_retry(&mut self) -> bool {
        match self.color_retry.on_tick() {
            Some(delay_ms) => {
                SetTimer(Some(self.hwnd), SYS_COLOR_TIMER, delay_ms, None);
            }
            None => {
                let _ = KillTimer(Some(self.hwnd), SYS_COLOR_TIMER);
            }
        }
        self.check_sys_colors()
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
            config::save(&self.cfg);
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

        // Таймер Today на свёрнутом острове (полоска, линия под музыкой или остаток вместо часов).
        let (timer_changed, timer_appeared) = self.live.set_timer(self.tabs.timer_view());
        if timer_appeared {
            self.retarget();
        }
        dirty |= timer_changed && !self.expanded;

        // Тик нужен, только пока на острове виден играющий плеер или таймер: кадры 30 в секунду для
        // эквалайзера и мигания паузы, иначе — раз в секунду по смене цифры.
        let timer_shown = !self.expanded && !self.hidden && self.live.timer().is_some();
        let tick = if self.live_form() == live::Form::Mini && self.live.playing() || timer_shown && self.live.timer().is_some_and(|t| t.paused) {
            LiveTick::Frames
        } else if timer_shown {
            LiveTick::Seconds
        } else {
            LiveTick::Off
        };
        if tick != self.live_tick_on {
            self.live_tick_on = tick;
            match tick {
                LiveTick::Frames => {
                    SetTimer(Some(self.hwnd), LIVE_TIMER, LIVE_TICK_MS, None);
                }
                LiveTick::Seconds => {
                    SetTimer(Some(self.hwnd), LIVE_TIMER, self.live.timer().map_or(1000, |t| t.next_ms), None);
                }
                LiveTick::Off => {
                    let _ = KillTimer(Some(self.hwnd), LIVE_TIMER);
                }
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
                tabs::Action::PasteImage(path) => {
                    if self.put_image(&path) {
                        self.disarm_leave();
                        self.set_expanded(false);
                        SetTimer(Some(self.hwnd), PASTE_TIMER, PASTE_DELAY_MS, None);
                    }
                }
                tabs::Action::CopyImage(path) => {
                    self.put_image(&path);
                }
                tabs::Action::Open(target) => {
                    if sys::open(&target) {
                        self.disarm_leave();
                        self.set_expanded(false);
                    } else {
                        // Файл исчез после последней проверки: запись помечается недоступной.
                        self.tabs.mark_missing(&target);
                    }
                }
                tabs::Action::DragOut(out) => self.run_drag_out(out),
                tabs::Action::SetQuickHotkey(combo) => self.set_quick_hotkey(combo),
                tabs::Action::Collapse => {
                    self.disarm_leave();
                    self.set_expanded(false);
                }
                tabs::Action::Layout(tab, text) if tab >= tabs::CUSTOM_BASE => {
                    if let Some(c) = self.cfg.custom.get_mut(tab - tabs::CUSTOM_BASE) {
                        c.layout = text;
                        config::save(&self.cfg);
                    }
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
        // Приёму перетаскивания нужно знать об этом сразу, не дожидаясь события.
        dnd::set_full(self.tabs.quick_full());
        RECORDING.store(self.tabs.recording_hotkey(), std::sync::atomic::Ordering::Relaxed);
        dirty
    }

    /// Кладёт PNG из истории или быстрого доступа в буфер обмена как изображение.
    pub(crate) unsafe fn put_image(&self, path: &std::path::Path) -> bool {
        match img::load_dib(path) {
            Some(dib) => sys::set_clipboard_dib(self.hwnd, &dib),
            None => {
                log::error(&format!("не удалось прочитать изображение {}", path.display()));
                false
            }
        }
    }
}
