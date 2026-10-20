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

    /// Переутверждает HWND_TOPMOST для окна острова, подложки стекла и зон наведения.
    /// Вызывается при смене активного окна, развороте окна, смене монитора или потере WS_EX_TOPMOST.
    /// В простое не делает лишних вызовов Win32 (0% CPU).
    pub(crate) unsafe fn ensure_topmost(&mut self, force: bool) {
        if self.hidden {
            return;
        }
        let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32;
        let lost_topmost = (ex & WS_EX_TOPMOST.0) == 0;

        // Проверяем, находится ли окно острова на вершине Z-порядка.
        // GetWindow(hwnd, GW_HWNDPREV) возвращает окно, находящееся выше нашего в Z-порядке.
        let not_on_top = GetWindow(self.hwnd, GW_HWNDPREV)
            .is_ok_and(|prev| !prev.0.is_null() && prev != self.glass_wnd);

        if force || lost_topmost || not_on_top {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            // Подложка стекла должна иметь WS_EX_TOPMOST и стоять сразу за основным окном.
            let g_ex = GetWindowLongW(self.glass_wnd, GWL_EXSTYLE) as u32;
            if (g_ex & WS_EX_TOPMOST.0) == 0 {
                let _ = SetWindowPos(
                    self.glass_wnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            if IsWindowVisible(self.glass_wnd).as_bool() {
                let _ = SetWindowPos(
                    self.glass_wnd,
                    Some(self.hwnd),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            // Горячие зоны на других мониторах тоже должны оставаться поверх окон.
            for z in &self.hot {
                let z_ex = GetWindowLongW(z.hwnd, GWL_EXSTYLE) as u32;
                if force || (z_ex & WS_EX_TOPMOST.0) == 0 {
                    let _ = SetWindowPos(
                        z.hwnd,
                        Some(HWND_TOPMOST),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }

    pub(crate) fn target_size(&self) -> (f32, f32) {
        let collapsed = self.collapsed();
        let (w, h) = if self.expanded {
            let id = self.cur_id();
            // В режиме правки под сеткой появляется полоса с кнопками.
            (self.tab_w(id), App::tab_h(id) + self.tabs.edit_extra(id))
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
        if expanded && !self.expanded {
            self.ensure_topmost(true);
        }
        self.expanded = expanded;
        if expanded && self.live.clear_toast() {
            let _ = KillTimer(Some(self.hwnd), TOAST_TIMER);
        }
        if expanded && self.cfg.system_colors {
            // Подстраховка на случай, если уведомление об смене системных цветов не пришло.
            self.apply_theme();
        }
        if !expanded {
            crate::state::flush();
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

    /// Что сейчас на экране: (окно развёрнуто на весь экран, активна настоящая полноэкранная игра).
    /// Развёрнутость определяется самым верхним реальным окном монитора острова, а не окном
    /// в фокусе: после сворачивания фокус может уйти на панель задач или рабочий стол,
    /// и по нему состояние не восстановить.
    /// Скрытие острова разрешено только для эксклюзивных D3D-игр; обычные приложения и браузеры
    /// в F11 или развёрнутом виде остров не прячут.
    pub(crate) unsafe fn probe(&self) -> Option<(bool, bool)> {
        let quns = SHQueryUserNotificationState().ok().map(|s| s.0 as u32);
        let mon = if !self.mon.0.is_null() {
            self.mon
        } else {
            MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST)
        };
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return Some((false, false));
        }

        let mut game = false;
        let fg = GetForegroundWindow();
        if !fg.0.is_null() && IsWindow(Some(fg)).as_bool() && IsWindowVisible(fg).as_bool() && !IsIconic(fg).as_bool() {
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(
                fg,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut std::ffi::c_void,
                size_of::<u32>() as u32,
            );
            if cloaked == 0 && fg != self.hwnd && fg != self.glass_wnd {
                let fg_mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
                let fg_on_island_mon = fg_mon == mon;
                let mut rc = RECT::default();
                let has_rc = GetWindowRect(fg, &mut rc).is_ok();
                let m = mi.rcMonitor;
                let covers = has_rc && rc.left <= m.left && rc.top <= m.top && rc.right >= m.right && rc.bottom >= m.bottom;
                let zoomed = IsZoomed(fg).as_bool();
                let style = GetWindowLongW(fg, GWL_STYLE) as u32;
                let has_caption = (style & WS_CAPTION.0) == WS_CAPTION.0;
                let mut buf = [0u16; 64];
                let n = GetClassNameW(fg, &mut buf).max(0) as usize;
                let class_name = String::from_utf16_lossy(&buf[..n]);
                game = is_fullscreen_game(quns, fg_on_island_mon, covers, zoomed, has_caption, &class_name);
            }
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
                self.hide_glass();
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
        let covered_changed = covered != self.covered;
        if covered_changed {
            self.covered = covered;
            self.retarget();
            changed = true;
        }

        // Переутверждаем HWND_TOPMOST при смене разворота или потере Z-порядка/стиля.
        self.ensure_topmost(covered_changed);

        // Если остров был развёрнут, но курсор уже вне его (например, после перетаскивания окна),
        // запускаем задержку сворачивания.
        self.check_hover();

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

/// Проверяет, принадлежит ли класс окна известным не-игровым приложениям
/// (браузеры, системные окна, офисные программы, медиаплееры).
pub(crate) fn is_known_non_game_class(class: &str) -> bool {
    if class.starts_with("Qt") {
        return true;
    }
    matches!(
        class,
        // Chromium / Electron (Chrome, Edge, Brave, Opera, Vivaldi, VS Code, Discord, Slack, Spotify и др.)
        "Chrome_WidgetWin_1"
        | "Chrome_WidgetWin_0"
        // Mozilla Firefox, Thunderbird
        | "MozillaWindowClass"
        // Проводник Windows и системные окна
        | "CabinetWClass"
        | "ExploreWClass"
        | "Progman"
        | "WorkerW"
        | "Shell_TrayWnd"
        | "Shell_SecondaryTrayWnd"
        // UWP-приложения
        | "ApplicationFrameWindow"
        | "Windows.UI.Core.CoreWindow"
        // PowerPoint и презентации
        | "screenClass"
        | "PP12FrameClass"
        | "PPTFrameClass"
        // Медиаплееры
        | "mpv"
        | "vlc"
        | "MediaPlayerClassicW"
    )
}

/// Чистая логика определения полноэкранной игры: проверяет, нужно ли скрывать остров (`SW_HIDE`).
///
/// Остров скрывается ТОЛЬКО при эксклюзивном Direct3D полноэкранном режиме (QUNS_RUNNING_D3D_FULL_SCREEN = 3)
/// для настоящего игрового окна на том же мониторе.
/// Обычные приложения, развёрнутые на весь экран, браузеры в F11, плееры и презентации остров не скрывают.
pub(crate) fn is_fullscreen_game(
    quns_state: Option<u32>,
    fg_on_island_monitor: bool,
    fg_covers_monitor: bool,
    fg_is_zoomed: bool,
    fg_has_caption: bool,
    fg_class: &str,
) -> bool {
    // Скрываемся ТОЛЬКО при Direct3D эксклюзивном полноэкранном режиме (QUNS_RUNNING_D3D_FULL_SCREEN = 3).
    // Состояния вроде QUNS_BUSY (2), QUNS_PRESENTATION_MODE (4), QUNS_ACCEPTS_NOTIFICATIONS (5)
    // не являются эксклюзивной игрой и не должны прятать остров.
    const QUNS_D3D_FULL_SCREEN: u32 = 3;
    if quns_state != Some(QUNS_D3D_FULL_SCREEN) {
        return false;
    }

    // Окно должно быть на том же мониторе, где расположен остров.
    if !fg_on_island_monitor {
        return false;
    }

    // Окно должно полностью закрывать монитор острова.
    if !fg_covers_monitor {
        return false;
    }

    // Обычные развёрнутые окна со стилем WS_MAXIMIZE (IsZoomed) — это приложения, а не эксклюзивные игры.
    if fg_is_zoomed {
        return false;
    }

    // Окна со стандартным заголовком (WS_CAPTION) не являются эксклюзивными играми на весь экран.
    if fg_has_caption {
        return false;
    }

    // Браузеры в полноэкранном режиме (F11/видео), системный интерфейс, плееры и презентации не прячут остров.
    if is_known_non_game_class(fg_class) {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quns_non_d3d_states_do_not_hide_island() {
        // Никакие другие состояния (включая QUNS_BUSY и презентации) не должны скрывать остров.
        for state in [None, Some(1), Some(2), Some(4), Some(5), Some(6), Some(7)] {
            assert!(
                !is_fullscreen_game(state, true, true, false, false, "UnrealWindow"),
                "state {state:?} should not hide island"
            );
        }
    }

    #[test]
    fn browsers_in_f11_do_not_hide_island() {
        const QUNS_D3D: Option<u32> = Some(3);
        // Браузеры на базе Chromium / Electron (Chrome, Edge, Brave, VS Code и т.д.)
        assert!(!is_fullscreen_game(QUNS_D3D, true, true, false, false, "Chrome_WidgetWin_1"));
        assert!(!is_fullscreen_game(QUNS_D3D, true, true, false, false, "Chrome_WidgetWin_0"));
        // Mozilla Firefox
        assert!(!is_fullscreen_game(QUNS_D3D, true, true, false, false, "MozillaWindowClass"));
    }

    #[test]
    fn maximized_apps_do_not_hide_island() {
        const QUNS_D3D: Option<u32> = Some(3);
        // Если окно развёрнуто через IsZoomed, это обычное приложение, а не эксклюзивная игра.
        assert!(!is_fullscreen_game(QUNS_D3D, true, true, true, false, "SomeAppWindow"));
        // Окно со стандартным заголовком (WS_CAPTION) также не скрывает остров.
        assert!(!is_fullscreen_game(QUNS_D3D, true, true, false, true, "SomeAppWindow"));
    }

    #[test]
    fn window_on_other_monitor_does_not_hide_island() {
        const QUNS_D3D: Option<u32> = Some(3);
        // Полноэкранная игра на втором мониторе не должна скрывать остров на основном.
        assert!(!is_fullscreen_game(QUNS_D3D, false, true, false, false, "UnrealWindow"));
    }

    #[test]
    fn window_not_covering_monitor_does_not_hide_island() {
        const QUNS_D3D: Option<u32> = Some(3);
        // Окно не на весь экран не должно скрывать остров.
        assert!(!is_fullscreen_game(QUNS_D3D, true, false, false, false, "UnrealWindow"));
    }

    #[test]
    fn system_and_media_apps_do_not_hide_island() {
        const QUNS_D3D: Option<u32> = Some(3);
        for class in [
            "CabinetWClass",
            "ExploreWClass",
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "ApplicationFrameWindow",
            "Windows.UI.Core.CoreWindow",
            "screenClass",
            "PP12FrameClass",
            "PPTFrameClass",
            "mpv",
            "vlc",
            "MediaPlayerClassicW",
            "Qt5QWindowIcon",
        ] {
            assert!(
                !is_fullscreen_game(QUNS_D3D, true, true, false, false, class),
                "class {class} should not hide island"
            );
        }
    }

    #[test]
    fn true_exclusive_d3d_game_hides_island() {
        const QUNS_D3D: Option<u32> = Some(3);
        // Настоящие игры на движках Unreal, Unity, Source или кастомных без заголовка и не zoomed.
        assert!(is_fullscreen_game(QUNS_D3D, true, true, false, false, "UnrealWindow"));
        assert!(is_fullscreen_game(QUNS_D3D, true, true, false, false, "UnityWndClass"));
        assert!(is_fullscreen_game(QUNS_D3D, true, true, false, false, "Valve001"));
        assert!(is_fullscreen_game(QUNS_D3D, true, true, false, false, "D3D Window"));
        assert!(is_fullscreen_game(QUNS_D3D, true, true, false, false, "GameMainWnd"));
    }
}
