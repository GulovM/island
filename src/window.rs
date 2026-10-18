use super::*;

impl App {
    pub(crate) unsafe fn place_window(&mut self, force: bool) {
        let now = (self.win_x, self.win_y, self.win_w, self.win_h);
        if !force && now == self.placed {
            return;
        }
        self.placed = now;
        let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), now.0, now.1, now.2, now.3, SWP_NOACTIVATE);
    }

    pub(crate) fn target_size(&self) -> (f32, f32) {
        let collapsed = self.collapsed();
        let (w, h) = if self.expanded {
            let id = self.cur_id();
            (self.tab_w(id), App::tab_h(id))
        } else if self.stripped() {
            // Ширина прежняя: зона наведения не растёт, меняется только высота.
            (collapsed.0, STRIP_H)
        } else {
            match self.live_form() {
                live::Form::Mini => (live::MINI.0.max(collapsed.0), live::MINI.1.max(collapsed.1)),
                live::Form::Toast => (live::TOAST.0.max(collapsed.0), live::TOAST.1.max(collapsed.1)),
                live::Form::Idle => collapsed,
            }
        };
        (w * self.scale, h * self.scale)
    }

    /// Остров стал тонкой полоской: окно развёрнуто на весь экран и пользователь включил
    /// режим полоски (переключатель в Настройках или правый клик по острову). Иначе остров остаётся обычной чёлкой.
    pub(crate) fn stripped(&self) -> bool {
        self.covered && self.cfg.strip_over_maximized
    }

    /// Правый клик по раскрытому острову включает/выключает режим полоски; действует с момента,
    /// когда мышь уйдёт с острова и он свернётся. Выбор сохраняется в `config.toml`.
    pub(crate) unsafe fn toggle_strip_mode(&mut self) -> bool {
        if !self.expanded || !self.cursor_inside() {
            return false;
        }
        self.cfg.strip_over_maximized = !self.cfg.strip_over_maximized;
        self.save_pending = true;
        self.tabs.apply_config(&self.cfg);
        self.retarget();
        true
    }

    /// Форма свёрнутого острова: мини-плеер/уведомление показываются, только когда остров
    /// свёрнут, виден и ничем не закрыт.
    pub(crate) fn live_form(&self) -> live::Form {
        if self.expanded || self.stripped() || self.hidden {
            live::Form::Idle
        } else {
            self.live.form()
        }
    }

    /// Показывает уведомление в свёрнутой форме. `true` — нужен кадр.
    pub(crate) unsafe fn show_toast(&mut self, t: live::Toast) -> bool {
        if !self.cfg.live_toasts || self.expanded || self.stripped() || self.hidden || self.gfx.is_none() {
            return false;
        }
        self.live.show(t);
        SetTimer(Some(self.hwnd), TOAST_TIMER, t.lifetime().as_millis() as u32, None);
        self.retarget();
        true
    }

    pub(crate) unsafe fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
        if expanded && self.live.clear_toast() {
            let _ = KillTimer(Some(self.hwnd), TOAST_TIMER);
        }
        if expanded && self.cfg.system_colors {
            // Подстраховка на случай, если уведомление об смене системных цветов не пришло.
            self.apply_theme();
        }
        if !expanded {
            self.trim_pending = true;
            self.ui.blur();
            self.sync_focus();
            self.ui.down = false;
            // Настройки — временный вид: после сворачивания остров открывается на вкладке.
            if self.settings_open {
                self.settings_open = false;
                self.tab_prev = None;
                self.tab_t.snap();
            }
        }
        self.retarget();
        self.open.target = if expanded { 1.0 } else { 0.0 };
    }

    // ---------- Мониторы ----------

    /// Ставит остров по центру верхнего края монитора `mon`. Если у монитора другой DPI,
    /// пересоздаёт буфер и шрифт. `snap` — перейти к целевой форме без анимации.
    pub(crate) unsafe fn apply_monitor(&mut self, mon: HMONITOR, snap: bool) {
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return;
        }
        let rc = mi.rcMonitor;
        let scale = monitor_scale(mon);
        let cw = (WIN_LOGICAL.0 * scale).ceil() as i32;
        let ch = (WIN_LOGICAL.1 * scale).ceil() as i32;

        let mut resized = false;
        if (scale - self.scale).abs() > 0.001 || cw != self.win_w || ch != self.win_h {
            self.scale = scale;
            self.win_w = cw;
            self.win_h = ch;
            if let Some(g) = &mut self.gfx {
                if let Err(e) = g.resize(cw, ch, scale, &self.dwrite) {
                    log::error(&format!("не удалось изменить размер буфера: {e}"));
                    self.lose_gfx();
                }
            }
            match measure_tabs(&self.dwrite, scale) {
                Ok(w) => self.tab_widths = w,
                Err(e) => log::error(&format!("не удалось измерить вкладки: {e}")),
            }
            resized = true;
        }

        self.mon = mon;
        self.win_x = rc.left + ((rc.right - rc.left) - cw) / 2;
        self.win_y = rc.top;
        self.rgn = (-1, -1, -1);
        self.place_window(false);
        if resized || snap {
            self.retarget();
            self.w.snap();
            self.h.snap();
        }
        self.apply_region();
    }

    /// Остров переезжает на другой монитор свёрнутым, дальше раскрывается обычным образом.
    pub(crate) unsafe fn move_to(&mut self, mon: HMONITOR) {
        self.disarm_leave();
        self.expanded = false;
        self.open.target = 0.0;
        self.open.snap();
        self.apply_monitor(mon, true);
        self.refresh_environment();
        self.rebuild_hot();
    }

    /// Пересоздаёт горячие зоны: по одной на каждый монитор, кроме того, где остров.
    pub(crate) unsafe fn rebuild_hot(&mut self) {
        for z in self.hot.drain(..) {
            let _ = DestroyWindow(z.hwnd);
        }
        let mut list: Vec<(HMONITOR, RECT)> = Vec::new();
        let _ = EnumDisplayMonitors(None, None, Some(enum_monitor), LPARAM(&mut list as *mut _ as isize));
        for (mon, rc) in list {
            if mon == self.mon {
                continue;
            }
            let zw = (self.collapsed().0 * monitor_scale(mon)).ceil() as i32;
            let x = rc.left + ((rc.right - rc.left) - zw) / 2;
            let created = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("IslandHotZone"),
                w!(""),
                WS_POPUP,
                x,
                rc.top,
                zw,
                HOT_H,
                None,
                None,
                Some(self.hinst),
                None,
            );
            match created {
                Ok(hwnd) => {
                    // Альфа 1/255: окно невидимо, но в отличие от полностью прозрачного получает мышь.
                    let _ = SetLayeredWindowAttributes(hwnd, Default::default(), 1, LWA_ALPHA);
                    if !self.hidden {
                        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    }
                    self.hot.push(HotZone { hwnd, mon });
                }
                Err(e) => log::error(&format!("не удалось создать горячую зону: {e}")),
            }
        }
    }

    pub(crate) unsafe fn on_hot(&mut self, hwnd: isize) -> bool {
        if self.hidden {
            return false;
        }
        let Some(mon) = self.hot.iter().find(|z| z.hwnd.0 as isize == hwnd).map(|z| z.mon) else {
            return false;
        };
        self.move_to(mon);
        self.set_expanded(true);
        self.check_hover();
        true
    }

    /// Мониторы, разрешение или DPI изменились.
    pub(crate) unsafe fn reconfigure(&mut self) -> bool {
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mon = if GetMonitorInfoW(self.mon, &mut mi).as_bool() {
            self.mon
        } else {
            // Монитор, на котором был остров, отключили.
            MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY)
        };
        self.apply_monitor(mon, false);
        self.rebuild_hot();
        true
    }

    // ---------- Область острова: регион окна, наведение, окружение ----------

    /// Область, которую остров занимает или займёт к концу анимации: (left, right, bottom)
    /// в пикселях окна, сверху всегда 0. Это и регион окна, и граница «мышь на острове».
    pub(crate) fn hit_rect(&self) -> (i32, i32, i32) {
        let win_w = self.win_w as f32;
        let (tw, th) = self.target_size();
        let l = ((win_w - self.w.value) / 2.0).min((win_w - tw) / 2.0);
        let r = ((win_w + self.w.value) / 2.0).max((win_w + tw) / 2.0);
        let b = self.h.value.max(th);
        // «Ушки» выходят за тело острова с обеих сторон.
        let ears = EAR * self.scale;
        // Допуск нужен только раскрытому острову, чтобы край не «дрожал»; свёрнутый — без запаса.
        let slop = if self.expanded { HIT_SLOP * self.scale } else { 0.0 };
        (
            (l - ears - slop).floor().max(0.0) as i32,
            (r + ears + slop).ceil().min(win_w) as i32,
            (b + slop).ceil().min(self.win_h as f32) as i32,
        )
    }

    /// Обрезает окно регионом по области острова: всё остальное окно не существует для мыши,
    /// и другие приложения получают клики/наведение как обычно. (`HTTRANSPARENT` пропускает
    /// мышь только окнам того же потока, поэтому для чужих процессов не работает.)
    pub(crate) unsafe fn apply_region(&mut self) {
        let rect = self.hit_rect();
        if rect == self.rgn {
            return;
        }
        self.rgn = rect;
        let rgn = CreateRectRgn(rect.0, 0, rect.1, rect.2);
        // Система забирает регион себе, удалять его не нужно.
        SetWindowRgn(self.hwnd, Some(rgn), false);
    }

    pub(crate) unsafe fn cursor_inside(&self) -> bool {
        let mut p = POINT::default();
        if GetCursorPos(&mut p).is_err() {
            return true;
        }
        let (l, r, b) = self.hit_rect();
        let (x, y) = (p.x - self.win_x, p.y - self.win_y);
        x >= l && x < r && y >= 0 && y < b
    }

    pub(crate) unsafe fn arm_leave(&mut self) {
        if !self.leave_armed {
            SetTimer(Some(self.hwnd), LEAVE_TIMER, self.cfg.leave_delay_ms, None);
            self.leave_armed = true;
        }
    }

    pub(crate) unsafe fn disarm_leave(&mut self) {
        if self.leave_armed {
            let _ = KillTimer(Some(self.hwnd), LEAVE_TIMER);
            self.leave_armed = false;
        }
    }

    /// Раскрытый остров: курсор вне занимаемой области — запускаем отсчёт сворачивания,
    /// вернулся — отменяем. Вызывается и на каждом кадре, потому что область меняется
    /// (смена вкладки, анимация), а мышь при этом может стоять на месте.
    pub(crate) unsafe fn check_hover(&mut self) {
        // Перетаскивание ползунка и ввод текста не должны сворачивать остров.
        if self.expanded && !self.cursor_inside() && !self.ui.holds_pointer() {
            self.arm_leave();
        } else {
            self.disarm_leave();
        }
    }

    /// Движение мыши: раскрывает остров; у раскрытого нужен кадр для hover-анимаций.
    pub(crate) unsafe fn on_move(&mut self) -> bool {
        if self.hidden {
            return false;
        }
        let was_collapsed = !self.expanded;
        if was_collapsed {
            self.set_expanded(true);
        }
        self.check_hover();
        true
    }

    pub(crate) unsafe fn on_leave_timer(&mut self) -> bool {
        self.disarm_leave();
        if self.expanded && !self.cursor_inside() && !self.ui.holds_pointer() {
            self.set_expanded(false);
            return true;
        }
        false
    }

    /// Что сейчас на экране: (окно развёрнуто на весь экран, активна D3D-игра на весь экран).
    /// Развёрнутость определяется самым верхним реальным окном монитора острова, а не окном
    /// в фокусе: после сворачивания фокус может уйти на панель задач или рабочий стол,
    /// и по нему состояние не восстановить.
    pub(crate) unsafe fn probe(&self) -> Option<(bool, bool)> {
        let game = matches!(
            SHQueryUserNotificationState(),
            Ok(s) if s == QUNS_RUNNING_D3D_FULL_SCREEN
        );
        let mon = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return Some((false, game));
        }
        let mut ctx = ScanCtx { own: self.hwnd, mon: mi.rcMonitor, covered: false };
        // EnumWindows идёт сверху вниз по Z-порядку; колбэк останавливает перебор на первом подходящем окне.
        let _ = EnumWindows(Some(scan_window), LPARAM(&mut ctx as *mut _ as isize));
        Some((ctx.covered, game))
    }

    /// Возвращает `true`, если нужно перерисовать.
    pub(crate) unsafe fn refresh_environment(&mut self) -> bool {
        let Some((covered, game)) = self.probe() else {
            return false;
        };
        let mut changed = false;
        // Раскладка клавиатуры окна в фокусе: событий об её смене нет, проверяем вместе с окружением.
        if self.face.visible() && self.face.poll_lang() {
            changed = true;
        }
        if game != self.hidden {
            self.hidden = game;
            self.retarget();
            let show = if game { SW_HIDE } else { SW_SHOWNOACTIVATE };
            if game {
                self.disarm_leave();
                self.set_expanded(false);
            }
            let _ = ShowWindow(self.hwnd, show);
            for z in &self.hot {
                let _ = ShowWindow(z.hwnd, show);
            }
            if !game {
                self.place_window(true);
            }
            changed = true;
        }
        if covered != self.covered {
            self.covered = covered;
            self.retarget();
            changed = true;
        }
        changed
    }

    // ---------- Сон, блокировка, потеря устройства ----------

    /// Остров не нужен на экране блокировки: сворачиваем, чтобы после входа не висел раскрытым.
    pub(crate) unsafe fn on_lock(&mut self) -> bool {
        self.disarm_leave();
        if self.expanded {
            self.set_expanded(false);
            return true;
        }
        false
    }

    /// Пробуждение или разблокировка: графика могла умереть, окружение — измениться.
    pub(crate) unsafe fn on_resume(&mut self) -> bool {
        let mut changed = self.refresh_environment();
        if self.gfx.as_ref().is_some_and(|g| g.removed()) {
            log::error("устройство D3D потеряно после сна/блокировки, пересоздаю");
            self.gfx = None;
            self.ui.reset_gfx();
            changed |= self.try_recover();
        }
        changed | self.reconfigure()
    }

    /// Графика недоступна: ждём и пробуем создать заново.
    pub(crate) unsafe fn lose_gfx(&mut self) {
        self.gfx = None;
        self.ui.reset_gfx();
        SetTimer(Some(self.hwnd), RECOVER_TIMER, RECOVER_MS, None);
    }

    pub(crate) unsafe fn try_recover(&mut self) -> bool {
        let _ = KillTimer(Some(self.hwnd), RECOVER_TIMER);
        // Старый Gfx должен быть уничтожен до создания нового: у окна одна цель DComp.
        self.gfx = None;
        let result = Gfx::new(self.hwnd, self.win_w, self.win_h, self.scale, &self.dwrite)
            .and_then(|g| measure_tabs(&self.dwrite, self.scale).map(|t| (g, t)));
        match result {
            Ok((g, tabs)) => {
                self.gfx = Some(g);
                self.tab_widths = tabs;
                self.rgn = (-1, -1, -1);
                log::info("графика восстановлена");
                true
            }
            Err(e) => {
                log::error(&format!("не удалось восстановить графику: {e}"));
                SetTimer(Some(self.hwnd), RECOVER_TIMER, RECOVER_MS * 2, None);
                false
            }
        }
    }

    // ---------- Трей ----------

    pub(crate) unsafe fn tray_menu(&mut self) -> bool {
        let Ok(menu) = CreatePopupMenu() else { return false };
        let autostart = sys::autostart_enabled();
        let item = |id: usize, flags: MENU_ITEM_FLAGS, text: &str| {
            let wide = sys::wz(&crate::i18n::tr(text));
            let _ = AppendMenuW(menu, flags, id, PCWSTR(wide.as_ptr()));
        };
        item(CMD_SETTINGS, MF_STRING, "Открыть config.toml");
        item(CMD_RELOAD, MF_STRING, "Перечитать конфиг");
        item(CMD_AUTOSTART, if autostart { MF_STRING | MF_CHECKED } else { MF_STRING }, "Автозапуск");
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(CMD_EXIT, MF_STRING, "Выход");

        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        // Без этого меню не закроется по клику вне его.
        let _ = SetForegroundWindow(self.hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, p.x, p.y, Some(0), self.hwnd, None);
        let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match cmd.0 as usize {
            CMD_SETTINGS => {
                let path = sys::wz(&format!("\"{}\"", config::path().display()));
                ShellExecuteW(None, w!("open"), w!("notepad.exe"), PCWSTR(path.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
                false
            }
            CMD_RELOAD => {
                self.apply_config(config::load());
                log::info("конфиг перечитан");
                true
            }
            CMD_AUTOSTART => {
                if !sys::set_autostart(!autostart) {
                    log::error("не удалось изменить автозапуск");
                }
                false
            }
            CMD_EXIT => {
                PostQuitMessage(0);
                false
            }
            _ => false,
        }
    }

    pub(crate) unsafe fn add_tray(&self) {
        if let Some(icon) = self.icon {
            if !sys::tray_add(self.hwnd, icon, WM_TRAY) {
                log::error("не удалось добавить значок в трей");
            }
        }
    }

    /// (Пере)регистрирует глобальную горячую клавишу из конфига.
    pub(crate) unsafe fn register_hotkey(&mut self) {
        let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
        if self.cfg.hotkey.is_empty() {
            return;
        }
        let Some((mods, vk)) = config::parse_hotkey(&self.cfg.hotkey) else { return };
        if let Err(e) = RegisterHotKey(Some(self.hwnd), HOTKEY_ID, HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT, vk) {
            log::error(&format!("не удалось занять горячую клавишу «{}»: {e}", self.cfg.hotkey));
        }
    }
}
