//! Состояние приложения `App` и управление жизненным циклом.

use std::time::Instant;

use super::*;

/// Невидимая полоска на верхнем краю монитора, где острова сейчас нет.
pub struct HotZone {
    pub(crate) hwnd: HWND,
    pub(crate) mon: HMONITOR,
}

pub struct App {
    pub(crate) hwnd: HWND,
    pub(crate) hinst: HINSTANCE,
    pub(crate) icon: Option<HICON>,
    pub(crate) cfg: Config,
    pub(crate) scale: f32,
    pub(crate) mon: HMONITOR,
    pub(crate) win_x: i32,
    pub(crate) win_y: i32,
    pub(crate) win_w: i32,
    pub(crate) win_h: i32,
    /// Последние применённые x, y, w, h окна — чтобы не дёргать `SetWindowPos` зря.
    pub(crate) placed: (i32, i32, i32, i32),
    /// Последняя применённая форма подложки размытия (x, y, ширина, высота, отпечаток контура;
    /// `(0, 0, 0, 0, −1)` — скрыта) — чтобы не дёргать `SetWindowPos`/`SetWindowRgn` зря.
    pub(crate) blur_key: std::cell::Cell<(i32, i32, i32, i32, i32)>,
    /// Последняя форма клипа подложки Composition (округлённые x0, aw, ah, ear, r).
    pub(crate) glass_shape: std::cell::Cell<(i32, i32, i32, i32, i32)>,
    /// Вспомогательное окно под островом: на нём размытие (у основного окна без битмапа оно не обрезается регионом).
    pub(crate) glass_wnd: HWND,
    /// Размытие доступно: новый путь (Composition HostBackdropBrush) или старый (accent blur-behind).
    pub(crate) blur_ok: bool,
    /// Подложка стекла через Windows.UI.Composition (`HostBackdropBrush` + клип по контуру тела).
    /// `Some` — новый путь работает, `None` — откат к старому accent-размытию.
    pub(crate) backdrop: Option<crate::backdrop::Backdrop>,
    /// Системные «Эффекты прозрачности» включены; иначе DWM рисует плоский фон вместо размытия.
    pub(crate) transparency: bool,

    pub(crate) dwrite: IDWriteFactory,
    /// `None` — графика потеряна и ждёт пересоздания (рисовать нечем).
    pub(crate) gfx: Option<Gfx>,
    /// Ширина каждой вкладки по её индексу в `TABS`.
    pub(crate) tab_widths: [f32; TAB_COUNT],
    /// Видимые вкладки в порядке показа: `order[позиция] = индекс в TABS` (скрытые не входят).
    pub(crate) order: Vec<usize>,
    pub(crate) hot: Vec<HotZone>,

    pub(crate) expanded: bool,
    /// Поверх острова развёрнуто окно на весь экран: в свёрнутом виде он — тонкая полоска.
    pub(crate) covered: bool,
    /// Активна полноэкранная игра (D3D): остров скрыт совсем.
    pub(crate) hidden: bool,
    pub(crate) leave_armed: bool,
    /// Свернулись и успокоились — пора сбросить рабочий набор.
    pub(crate) trim_pending: bool,
    /// Последний применённый регион окна: (left, right, bottom) в пикселях окна.
    pub(crate) rgn: (i32, i32, i32),
    /// Позиция выбранной вкладки в `order`.
    pub(crate) tab: usize,
    /// Вместо вкладки показана страница настроек (шестерёнка).
    pub(crate) settings_open: bool,
    /// Свечение острова, пока тянут ползунок радиуса углов.
    pub(crate) glow: Spring,
    /// Время, раскладка и Wi-Fi на свёрнутом острове, когда музыка не играет.
    pub(crate) face: face::Face,
    pub(crate) face_tick_on: bool,
    /// Настройки изменены и ждут записи в config.toml (пишем, когда кнопка мыши отпущена).
    pub(crate) save_pending: bool,
    pub(crate) w: Spring,
    pub(crate) h: Spring,
    pub(crate) open: Spring,

    /// Тулкит и состояние вкладок.
    pub(crate) ui: Ui,
    pub(crate) tabs: tabs::Tabs,
    /// Вкладка, которая сейчас на экране (индекс в `TABS`), и включённые для неё таймеры.
    pub(crate) shown_tab: Option<usize>,
    pub(crate) poll_on: bool,
    pub(crate) alarm_at: Option<Instant>,
    /// Предыдущая вкладка (индекс в `TABS`) на время кроссфейда и прогресс перехода 0..1.
    pub(crate) tab_prev: Option<usize>,
    pub(crate) tab_t: Spring,
    /// Капля под выбранной вкладкой: левый край, ширина (логические пиксели) и прозрачность; `pill_sig` — отпечаток ряда вкладок, при смене которого капля прыгает без анимации.
    pub(crate) pill_x: Spring,
    pub(crate) pill_w: Spring,
    pub(crate) pill_a: Spring,
    pub(crate) pill_sig: (usize, i32, i32),
    /// Направление сдвига при смене вкладки: +1 — новая справа.
    pub(crate) tab_dir: f32,
    /// Индикатор в свёрнутой форме: мини-плеер и уведомления.
    pub(crate) live: live::Live,
    /// Включён ли секундный тик мини-плеера.
    pub(crate) live_tick_on: bool,
    /// Режим «полоска над развёрнутыми окнами» (по умолчанию выключен).
    /// Клавиатурный ввод включён: окно временно активно и принимает фокус.
    pub(crate) kbd: bool,
    /// Окно, которое было активным до включения ввода.
    pub(crate) prev_fg: HWND,
    /// Ожидающая вторая половина суррогатной пары из `WM_CHAR`.
    pub(crate) high_surrogate: u32,
    /// Где нажали кнопку мыши (пиксели окна), пока ждём долгого нажатия.
    pub(crate) press_pos: Option<(i32, i32)>,
    /// Планировщик повторных проверок темы (гонка записи в реестр Windows/WE).
    pub(crate) color_retry: sys::ColorRetry,
}

impl App {
    pub(crate) fn new(cfg: Config, state: &crate::state::State) -> Result<Self> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

            let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
            let cursor = LoadCursorW(None, IDC_ARROW)?;
            let class = w!("IslandWindow");
            RegisterClassExW(&WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                hCursor: cursor,
                lpszClassName: class,
                ..Default::default()
            });
            RegisterClassExW(&WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(hot_proc),
                hInstance: hinst,
                hCursor: cursor,
                lpszClassName: w!("IslandHotZone"),
                ..Default::default()
            });

            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("Island"),
                WS_POPUP,
                0,
                0,
                16,
                16,
                None,
                None,
                Some(hinst),
                None,
            )?;
            RegisterClassExW(&WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(glass_proc),
                hInstance: hinst,
                lpszClassName: w!("IslandGlass"),
                ..Default::default()
            });
            let glass_wnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                w!("IslandGlass"),
                w!(""),
                WS_POPUP,
                0,
                0,
                16,
                16,
                None,
                None,
                Some(hinst),
                None,
            )?;

            // Новый путь: Windows.UI.Composition HostBackdropBrush с клипом по контуру тела.
            // DWMWA_USE_HOSTBACKDROPBRUSH (17) + LWA_ALPHA разрешают HostBackdropBrush в окне.
            let _ = SetLayeredWindowAttributes(glass_wnd, windows::Win32::Foundation::COLORREF(0), 255, LWA_ALPHA);
            let on = BOOL(1);
            let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                glass_wnd,
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(17), // DWMWA_USE_HOSTBACKDROPBRUSH
                &on as *const _ as _,
                4,
            );
            let backdrop = match crate::backdrop::Backdrop::new(glass_wnd) {
                Ok(b) => {
                    log::info("стекло: подложка Composition HostBackdropBrush создана");
                    Some(b)
                }
                Err(e) => {
                    log::info(&format!("стекло: Composition недоступна ({e}), откат к accent-размытию"));
                    None
                }
            };
            // Если новый путь не удался — пробуем старый accent blur-behind.
            let blur_ok = if backdrop.is_some() {
                true
            } else {
                crate::wnd::enable_blur_behind(glass_wnd)
            };
            if !blur_ok {
                log::info("стекло: blur-behind недоступен, тело острова остаётся без размытия");
            }
            MAIN_WND.with(|w| w.set(hwnd));
            TASKBAR_MSG.with(|c| c.set(RegisterWindowMessageW(w!("TaskbarCreated"))));

            let mon = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            set_ui_scale(cfg.ui_scale);
            let scale = monitor_scale(mon);
            let cw = (WIN_LOGICAL.0 * scale).ceil() as i32;
            let ch = (WIN_LOGICAL.1 * scale).ceil() as i32;

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let gfx = Gfx::new(hwnd, cw, ch, scale, &dwrite)?;
            let tab_widths = measure_tabs(&dwrite, scale)?;
            let order = cfg.resolve_visible(&TABS);
            let initial_tab = if !state.last_tab.is_empty() {
                order.iter().position(|&idx| TABS[idx].eq_ignore_ascii_case(&state.last_tab)).unwrap_or(0)
            } else {
                0
            };

            let icon = sys::make_icon(hinst, cfg.accent)
                .or_else(|| LoadIconW(None, IDI_APPLICATION).ok());

            let mut app = App {
                hwnd,
                hinst,
                icon,
                scale,
                mon,
                win_x: 0,
                win_y: 0,
                win_w: cw,
                win_h: ch,
                placed: (i32::MIN, 0, 0, 0),
                blur_key: std::cell::Cell::new((i32::MIN, 0, 0, 0, 0)),
                glass_shape: std::cell::Cell::new((i32::MIN, 0, 0, 0, 0)),
                glass_wnd,
                blur_ok,
                backdrop,
                transparency: sys::transparency_enabled(),
                dwrite,
                gfx: Some(gfx),
                tab_widths,
                order,
                hot: Vec::new(),
                expanded: false,
                covered: false,
                hidden: false,
                leave_armed: false,
                trim_pending: true,
                rgn: (-1, -1, -1),
                tab: initial_tab,
                settings_open: false,
                glow: Spring::unit(0.0, 220.0, 26.0),
                face: face::Face::new(cfg.idle_face),
                face_tick_on: false,
                save_pending: false,
                w: Spring::new(cfg.collapsed.0 * scale, 420.0, 30.0),
                h: Spring::new(cfg.collapsed.1 * scale, 420.0, 30.0),
                open: Spring::new(0.0, 300.0, 34.0),
                ui: Ui::default(),
                tabs: tabs::Tabs::default(),
                shown_tab: None,
                poll_on: false,
                alarm_at: None,
                tab_prev: None,
                tab_t: Spring::unit(1.0, 260.0, 30.0),
                pill_x: Spring::new(0.0, 420.0, 30.0),
                pill_w: Spring::new(0.0, 420.0, 30.0),
                pill_a: Spring::unit(1.0, 420.0, 30.0),
                pill_sig: (0, 0, 0),
                tab_dir: 1.0,
                live: live::Live::new(cfg.live_media),
                live_tick_on: false,
                kbd: false,
                prev_fg: HWND(std::ptr::null_mut()),
                high_surrogate: 0,
                press_pos: None,
                color_retry: sys::ColorRetry::new(),
                cfg,
            };
            theme::set_glass(app.cfg.glass);
            app.apply_theme();
            app.apply_monitor(mon, true);
            // Стартовое состояние сразу, без анимации.
            app.refresh_environment();
            app.w.snap();
            app.h.snap();
            app.apply_region();
            app.rebuild_hot();
            if !app.hidden {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            Ok(app)
        }
    }

    pub(crate) fn collapsed(&self) -> (f32, f32) {
        self.cfg.collapsed
    }

    /// Под телом настоящее размытие: стекло включено в конфиге, в Windows включены «Эффекты прозрачности»
    /// и окно подложки приняло blur-behind. Иначе подложка скрыта, а тело плотнее (`glass::body_alpha`).
    pub(crate) fn real_glass(&self) -> bool {
        self.cfg.glass && self.transparency && self.blur_ok
    }

    /// Акцент, который сейчас действует (свой или системный).
    pub(crate) fn accent(&self) -> D2D1_COLOR_F {
        theme::get().accent
    }

    /// Цвета, которые сейчас должны действовать: (фон, акцент). С «Цвет системы» — из Windows
    /// (там же оказываются цвета, подстроенные Wallpaper Engine под обои).
    pub(crate) fn effective_colors(&self) -> (theme::Rgb, theme::Rgb) {
        if self.cfg.system_colors {
            if let Some(sys) = sys::system_colors() {
                return theme::from_system(sys, !sys::light_mode());
            }
        }
        (self.cfg.bg, self.cfg.accent)
    }

    /// Пересчитывает палитру. `true` — цвета изменились и нужен кадр.
    pub(crate) fn apply_theme(&mut self) -> bool {
        let (bg, accent) = self.effective_colors();
        let p = theme::derive(bg, accent, self.cfg.palette_style);
        if p == theme::get() {
            return false;
        }
        theme::set(p);
        true
    }

    /// Проверяет системные цвета и эффекты прозрачности. Возвращает `true`, если палитра или
    /// прозрачность изменились и нужен кадр перерисовки.
    pub(crate) fn check_sys_colors(&mut self) -> bool {
        let t = sys::transparency_enabled();
        let flipped = std::mem::replace(&mut self.transparency, t) != t;
        let theme_changed = self.cfg.system_colors && self.apply_theme();
        theme_changed || flipped
    }

    /// Номер вкладки (индекс в `TABS`), выбранной в ряду.
    pub(crate) fn normal_id(&self) -> usize {
        self.order.get(self.tab).or_else(|| self.order.last()).copied().unwrap_or(0)
    }

    /// Что показано сейчас: вкладка или страница настроек (`tabs::SETTINGS`).
    pub(crate) fn cur_id(&self) -> usize {
        if self.settings_open {
            tabs::SETTINGS
        } else {
            self.normal_id()
        }
    }

    /// Центр шестерёнки в логических пикселях окна: у правого края острова в ряду вкладок.
    pub(crate) fn gear_center(&self) -> (f32, f32) {
        let half = self.w.value / self.scale / 2.0;
        (self.win_w as f32 / self.scale / 2.0 + half - 16.0, TAB_TOP + TAB_H / 2.0)
    }

    /// Открывает страницу настроек или возвращает на вкладку.
    pub(crate) fn toggle_settings(&mut self) {
        self.tab_prev = Some(self.cur_id());
        self.settings_open = !self.settings_open;
        self.tab_dir = if self.settings_open { 1.0 } else { -1.0 };
        self.tab_t.value = 0.0;
        self.tab_t.target = 1.0;
        self.ui.blur();
        self.retarget();
    }

    /// Применяет настройки со страницы: внешний вид живьём, запись в файл — когда мышь отпущена.
    pub(crate) fn apply_settings(&mut self, new: Config) {
        let current = self.normal_id();
        self.cfg.accent = new.accent;
        self.cfg.bg = new.bg;
        self.cfg.system_colors = new.system_colors;
        self.cfg.corner_radius = new.corner_radius;
        self.cfg.glass = new.glass;
        self.cfg.glass_opacity = new.glass_opacity;
        self.cfg.material = new.material;
        self.cfg.palette_style = new.palette_style;
        self.cfg.accent_pill = new.accent_pill;
        theme::set_glass(new.glass);
        if (self.cfg.ui_scale - new.ui_scale).abs() > 0.001 {
            self.cfg.ui_scale = new.ui_scale;
            set_ui_scale(new.ui_scale);
            // SAFETY: вызывается из оконного потока, `mon` — монитор, на котором стоит остров.
            unsafe { self.apply_monitor(self.mon, true) };
        }
        self.cfg.tabs = new.tabs;
        self.cfg.hidden_tabs = new.hidden_tabs;
        self.cfg.strip_over_maximized = new.strip_over_maximized;
        self.cfg.idle_face = new.idle_face;
        self.cfg.auto_update = new.auto_update;
        if self.cfg.language != new.language {
            self.cfg.language = new.language;
            // SAFETY: оконный поток, DirectWrite-фабрика жива.
            unsafe { self.refresh_language() };
        }
        self.order = self.cfg.resolve_visible(&TABS);
        self.tab = self.order.iter().position(|&i| i == current).unwrap_or(0);
        self.face.set_enabled(self.cfg.idle_face);
        self.apply_theme();
        self.retarget();
        self.save_pending = true;
    }

    /// Высота панели вкладки: берётся через `Tabs::island_size`.
    pub(crate) fn tab_h(&self, id: usize) -> f32 {
        self.tabs.island_size(id).1
    }

    /// Ширина панели вкладки: не уже ряда вкладок, иначе они обрезаются.
    pub(crate) fn tab_w(&self, id: usize) -> f32 {
        self.tabs.island_size(id).0.max(self.tab_row_width() + TAB_SIDE_LEFT + TAB_SIDE_RIGHT)
    }

    /// Применяет язык из конфига: подписи вкладок меняют ширину, поэтому ряд измеряется заново.
    pub(crate) unsafe fn refresh_language(&mut self) {
        i18n::set(i18n::from_config(&self.cfg.language));
        match measure_tabs(&self.dwrite, self.scale) {
            Ok(w) => self.tab_widths = w,
            Err(e) => log::error(&format!("не удалось измерить вкладки: {e}")),
        }
        self.retarget();
    }

    pub(crate) unsafe fn apply_config(&mut self, cfg: Config) {
        let current = self.normal_id();
        self.order = cfg.resolve_visible(&TABS);
        self.tab = self.order.iter().position(|&i| i == current).unwrap_or(0);
        self.cfg = cfg;
        theme::set_glass(self.cfg.glass);
        set_ui_scale(self.cfg.ui_scale);
        self.refresh_language();
        self.apply_monitor(self.mon, true);
        self.face.set_enabled(self.cfg.idle_face);
        self.apply_theme();
        self.live.set_media_enabled(self.cfg.live_media);
        self.tabs.apply_config(&self.cfg);
        self.register_hotkey();
        SetTimer(Some(self.hwnd), ENV_TIMER, self.cfg.env_poll_ms, None);
        self.retarget();
        self.w.snap();
        self.h.snap();
        self.rgn = (-1, -1, -1);
        self.apply_region();
        self.rebuild_hot();
    }

    // ---------- События, анимация, отрисовка ----------

    /// Минимальная ширина сетки виджетов в ячейках, достаточная для размещения ряда вкладок.
    pub(crate) fn min_grid_cols(&self) -> u8 {
        ((self.tab_row_width() + TAB_SIDE_LEFT + TAB_SIDE_RIGHT - 2.0 * tabs::PAD + tabs::layout::GAP)
            / tabs::layout::CELL)
            .ceil()
            .max(1.0) as u8
    }

    pub(crate) fn retarget(&mut self) {
        let min_cols = self.min_grid_cols();
        self.tabs.set_min_cols(min_cols);
        let (w, h) = self.target_size();
        self.w.target = w;
        self.h.target = h;
        self.retarget_pill();
        let form = self.live_form();
        self.live.set_form(form);
        // Часы, раскладка и Wi-Fi — только когда свёрнутому острову больше нечего показывать.
        let idle = form == live::Form::Idle && !self.expanded && !self.stripped() && !self.hidden;
        self.face.set_visible(idle);
    }

    /// Наводит каплю на выбранную вкладку; при свёрнутом острове или смене набора вкладок — без анимации.
    fn retarget_pill(&mut self) {
        let rects = self.tab_rects();
        let Some(&(x, w)) = rects.get(self.tab) else { return };
        let sig = (rects.len(), (self.tab_row_width() * 100.0) as i32, self.win_w);
        let jump = !self.expanded || sig != self.pill_sig;
        self.pill_sig = sig;
        self.pill_x.target = x;
        self.pill_w.target = w;
        self.pill_a.target = if self.settings_open { 0.0 } else { 1.0 };
        if jump {
            self.pill_x.snap();
            self.pill_w.snap();
            self.pill_a.snap();
        }
    }

    pub(crate) fn animating(&self) -> bool {
        !(self.w.settled() && self.h.settled() && self.open.settled())
            || !(self.pill_x.settled() && self.pill_w.settled() && self.pill_a.settled())
            || !self.tab_t.settled()
            || !self.glow.settled()
            || self.face.animating()
            || self.live.animating()
            || self.ui.animating()
            || (self.expanded && self.tabs.animating())
    }

    pub(crate) fn step(&mut self, dt: f32) {
        self.w.step(dt);
        self.h.step(dt);
        self.open.step(dt);
        self.ui.step(dt);
        self.tabs.step(dt);
        self.live.step(dt);
        self.face.step(dt);
        self.glow.step(dt);
        if self.glow.settled() {
            self.glow.snap();
        }
        for p in [&mut self.pill_x, &mut self.pill_w, &mut self.pill_a] {
            p.step(dt);
            if p.settled() {
                p.snap();
            }
        }
        self.tab_t.step(dt);
        if self.tab_t.settled() {
            self.tab_t.snap();
            self.tab_prev = None;
        }
        if self.w.settled() && self.h.settled() {
            self.w.snap();
            self.h.snap();
        }
        if self.open.settled() {
            self.open.snap();
        }
    }
}
