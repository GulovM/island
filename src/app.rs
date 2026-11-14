//! Состояние приложения `App` и управление жизненным циклом.

use std::time::Instant;

use super::*;

/// Чем сейчас тикает `LIVE_TIMER` свёрнутого острова.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LiveTick {
    Off,
    /// ~15 кадров в секунду: мигание паузы таймера.
    Frames,
    /// ~30 кадров в секунду: боковые волны под системный звук на обычной свёрнутой чёлке.
    Wave,
    /// Раз в секунду, по смене цифры таймера.
    Seconds,
}

/// Невидимая полоска на верхнем краю монитора, где острова сейчас нет.
pub struct HotZone {
    pub(crate) hwnd: HWND,
    pub(crate) mon: HMONITOR,
}

pub struct App {
    pub(crate) hwnd: HWND,
    pub(crate) hinst: HINSTANCE,
    pub(crate) icon: Option<HICON>,
    /// `icon` создан make_icon и его надо уничтожить при замене; иконки из LoadIconW разделяемые.
    pub(crate) icon_owned: bool,
    /// Светлость панели задач, под которую нарисован `icon`.
    pub(crate) tray_key: Option<bool>,
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
    /// Последняя форма клипа подложки Composition (округлённые x0, y0, aw, ah, ear, r_top, r_bot).
    pub(crate) glass_shape: std::cell::Cell<(i32, i32, i32, i32, i32, i32, i32)>,
    /// Вспомогательное окно под островом: на нём размытие (у основного окна без битмапа оно не обрезается регионом).
    pub(crate) glass_wnd: HWND,
    /// Размытие доступно: новый путь (Composition HostBackdropBrush) или старый (accent blur-behind).
    pub(crate) blur_ok: bool,
    /// Подложка стекла через Windows.UI.Composition (`HostBackdropBrush` + клип по контуру тела).
    /// `Some` — новый путь работает, `None` — откат к старому accent-размытию.
    pub(crate) backdrop: Option<crate::backdrop::Backdrop>,
    /// Контроллер `DispatcherQueue` главного потока (живёт в `App`, переживает пересоздание `Backdrop`).
    dq: Option<windows::System::DispatcherQueueController>,
    /// Системные «Эффекты прозрачности» включены; иначе DWM рисует плоский фон вместо размытия.
    pub(crate) transparency: bool,

    pub(crate) dwrite: IDWriteFactory,
    /// `None` — графика потеряна и ждёт пересоздания (рисовать нечем).
    pub(crate) gfx: Option<Gfx>,
    /// Ширина каждой вкладки по её индексу в `TABS`.
    pub(crate) tab_widths: Vec<f32>,
    /// Видимые вкладки в порядке показа: `order[позиция] = индекс в TABS` (скрытые не входят).
    pub(crate) order: Vec<usize>,
    pub(crate) hot: Vec<HotZone>,

    pub(crate) expanded: bool,
    /// Поверх острова развёрнуто окно на весь экран: в свёрнутом виде он — тонкая полоска.
    pub(crate) covered: bool,
    /// Активна полноэкранная игра (D3D): остров скрыт совсем.
    pub(crate) hidden: bool,
    /// Остров спрятан на время скриншота до этого момента.
    pub(crate) shot_until: Option<Instant>,
    pub(crate) leave_armed: bool,
    /// Позиция курсора в момент смены размера острова (настройки, вкладка, правка): пока мышь
    /// не двинулась и не вернулась на остров, он не сворачивается из-за того, что сам съёжился.
    pub(crate) leave_hold: Option<POINT>,
    /// Свернулись и успокоились — пора сбросить рабочий набор.
    pub(crate) trim_pending: bool,
    /// Последний применённый регион окна: (left, top, right, bottom) в пикселях окна.
    pub(crate) rgn: (i32, i32, i32, i32),
    /// Области последних кадров анимации: регион окна не сжимается быстрее, чем обновляется картинка.
    pub(crate) rgn_hist: Vec<(i32, i32, i32, i32)>,
    /// Позиция выбранной вкладки в `order`.
    pub(crate) tab: usize,
    /// Вместо вкладки показана страница настроек (шестерёнка).
    pub(crate) settings_open: bool,
    /// Вместо вкладки показана вкладка уведомлений (колокольчик).
    pub(crate) notifications_open: bool,
    /// Последнее известное состояние Windows «Не беспокоить» для автопереключения всплывающих уведомлений.
    pub(crate) dnd_known: Option<bool>,
    /// Свечение острова, пока тянут ползунок радиуса углов.
    pub(crate) glow: Spring,
    /// Время, раскладка и Wi-Fi на свёрнутом острове, когда музыка не играет.
    pub(crate) face: face::Face,
    pub(crate) face_tick_on: bool,
    /// Настройки изменены и ждут записи в config.toml (пишем, когда кнопка мыши отпущена).
    pub(crate) save_pending: bool,
    /// Свёрнутая и раскрытая формы тела; фактический размер — `body_size()`.
    pub(crate) body: anim::Body,
    /// Единый прогресс раскрытия 0..1: от него зависят размер тела, скругление и гашение свёрнутой формы.
    pub(crate) open: Spring,
    /// Плавный переход свёрнутой чёлки в плавающую капсулу-островок (0 — чёлка у края, 1 — капсула).
    pub(crate) capsule: Spring,

    /// Надпись перетаскивания на острове (`None` — на остров ничего не тащат).
    pub(crate) drop_msg: Option<crate::dragdrop::DropMsg>,
    /// Появление надписи перетаскивания, 0..1.
    pub(crate) drag_a: Spring,

    /// Тулкит и состояние вкладок.
    pub(crate) ui: Ui,
    pub(crate) tabs: tabs::Tabs,
    /// Вкладка, которая сейчас на экране (номер вкладки, `tabs`), и включённые для неё таймеры.
    pub(crate) shown_tab: Option<usize>,
    pub(crate) poll_on: bool,
    pub(crate) alarm_at: Option<Instant>,
    /// Сессия заблокирована или экран погашен: напоминания о событиях ждут.
    pub(crate) locked: bool,
    pub(crate) screen_off: bool,
    /// Предыдущая вкладка (номер вкладки, `tabs`) на время кроссфейда и прогресс перехода 0..1.
    pub(crate) tab_prev: Option<usize>,
    pub(crate) tab_t: Spring,
    /// Капля под выбранной вкладкой: левый край, ширина (логические пиксели) и прозрачность; `pill_sig` — отпечаток ряда вкладок, при смене которого капля прыгает без анимации.
    pub(crate) pill_x: Spring,
    pub(crate) pill_w: Spring,
    pub(crate) pill_a: Spring,
    pub(crate) pill_sig: (usize, i32, i32),
    /// Направление сдвига при смене вкладки: +1 — новая справа.
    pub(crate) tab_dir: f32,
    /// Индикатор в свёрнутой форме: эквалайзер и уведомления.
    pub(crate) live: live::Live,
    /// Включён ли секундный/кадровый тик свёрнутого острова.
    pub(crate) live_tick_on: LiveTick,
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
                w!("Kite Island"),
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
            let mut dq = None;
            let (glass_wnd, backdrop, blur_ok) = init_glass_window(hinst, &mut dq)?;
            MAIN_WND.with(|w| w.set(hwnd));
            TASKBAR_MSG.with(|c| c.set(RegisterWindowMessageW(w!("TaskbarCreated"))));

            let mon = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            set_ui_scale(cfg.ui_scale);
            let scale = monitor_scale(mon);
            let cw = (WIN_LOGICAL.0 * scale).ceil() as i32;
            let ch = (WIN_LOGICAL.1 * scale).ceil() as i32;

            // До создания `Fonts` внутри `Gfx::new`: размер формата фиксируется при создании,
            // живьём не перечитываем (см. theme::text_scale).
            theme::set_text_scale(sys::text_scale_factor());
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let gfx = Gfx::new(hwnd, cw, ch, scale, &dwrite)?;
            let tab_widths = measure_tabs(&dwrite, scale, &cfg)?;
            let order = cfg.visible_tabs();
            let initial_tab = if !state.last_tab.is_empty() {
                order.iter().position(|&idx| cfg.tab_key(idx).eq_ignore_ascii_case(&state.last_tab)).unwrap_or(0)
            } else {
                0
            };

            let mut app = App {
                hwnd,
                hinst,
                // Рисуется в refresh_tray_icon под текущую тему панели задач.
                icon: None,
                icon_owned: false,
                tray_key: None,
                scale,
                mon,
                win_x: 0,
                win_y: 0,
                win_w: cw,
                win_h: ch,
                placed: (i32::MIN, 0, 0, 0),
                blur_key: std::cell::Cell::new((i32::MIN, 0, 0, 0, 0)),
                glass_shape: std::cell::Cell::new((i32::MIN, 0, 0, 0, 0, 0, 0)),
                glass_wnd,
                blur_ok,
                backdrop,
                dq,
                transparency: {
                    theme::set_reduced_motion(!sys::animations_enabled());
                    sys::transparency_enabled()
                },
                dwrite,
                gfx: Some(gfx),
                tab_widths,
                order,
                hot: Vec::new(),
                expanded: false,
                covered: false,
                hidden: false,
                shot_until: None,
                leave_armed: false,
                leave_hold: None,
                trim_pending: true,
                rgn: (-1, -1, -1, -1),
                rgn_hist: Vec::new(),
                tab: initial_tab,
                settings_open: false,
                notifications_open: false,
                dnd_known: None,
                glow: Spring::unit(0.0, 220.0, 26.0),
                face: face::Face::new(cfg.idle_face, cfg.collapsed_left, cfg.collapsed_right),
                face_tick_on: false,
                save_pending: false,
                body: anim::Body::new((cfg.collapsed.0 * scale, cfg.collapsed.1 * scale)),
                // Затухание чуть выше критического порога 0,7: почти без отскока, чтобы при сворачивании
                // тело не уходило меньше свёрнутого размера.
                open: Spring::unit(0.0, 420.0, 34.0),
                capsule: Spring::unit(0.0, 340.0, 27.0),
                drop_msg: None,
                drag_a: Spring::unit(0.0, 260.0, 30.0),
                ui: Ui::default(),
                tabs: tabs::Tabs::default(),
                shown_tab: None,
                poll_on: false,
                alarm_at: None,
                locked: false,
                screen_off: false,
                tab_prev: None,
                tab_t: Spring::unit(1.0, 260.0, 30.0),
                pill_x: Spring::new(0.0, 420.0, 30.0),
                pill_w: Spring::new(0.0, 420.0, 30.0),
                pill_a: Spring::unit(1.0, 420.0, 30.0),
                pill_sig: (0, 0, 0),
                tab_dir: 1.0,
                live: live::Live::new(cfg.live_media),
                live_tick_on: LiveTick::Off,
                kbd: false,
                prev_fg: HWND(std::ptr::null_mut()),
                high_surrogate: 0,
                press_pos: None,
                color_retry: sys::ColorRetry::new(),
                cfg,
            };
            theme::set_glass(app.cfg.glass);
            app.apply_theme();
            app.refresh_tray_icon();
            app.apply_monitor(mon, true);
            // Стартовое состояние сразу, без анимации.
            app.refresh_environment();
            app.body.snap();
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
        let (material, opacity) =
            crate::glass::effective_density(self.cfg.palette_style, self.cfg.material, self.cfg.glass_opacity);
        let layers =
            crate::glass::body_layers(material, bg, self.cfg.glass, self.real_glass(), opacity, 1.0, 1.0);
        let p = theme::derive_with_alpha(
            bg,
            accent,
            self.cfg.palette_style,
            layers.tint_alpha,
            layers.sheen.alpha,
        );
        if p == theme::get() {
            return false;
        }
        theme::set(p);
        true
    }

    /// Проверяет системные цвета и эффекты прозрачности. Возвращает `true`, если палитра или
    /// прозрачность изменились и нужен кадр перерисовки.
    pub(crate) fn check_sys_colors(&mut self) -> bool {
        theme::set_reduced_motion(!sys::animations_enabled());
        let t = sys::transparency_enabled();
        let flipped = std::mem::replace(&mut self.transparency, t) != t;
        let theme_changed = if self.cfg.system_colors || flipped { self.apply_theme() } else { false };
        self.refresh_tray_icon();
        theme_changed || flipped
    }

    /// Пересоздаёт подложку стекла и окно `glass_wnd` (при смене состояния DWM, RDP или после TDR).
    pub(crate) fn refresh_glass(&mut self) -> bool {
        if self.hinst.0.is_null() {
            return false;
        }
        let Ok((new_wnd, new_bd, new_ok)) = (unsafe { init_glass_window(self.hinst, &mut self.dq) })
        else {
            return false;
        };
        let old_wnd = std::mem::replace(&mut self.glass_wnd, new_wnd);
        self.backdrop = new_bd;
        self.blur_ok = new_ok;
        self.apply_theme();
        self.blur_key.set((i32::MIN, 0, 0, 0, 0));
        self.glass_shape.set((i32::MIN, 0, 0, 0, 0, 0, 0));
        if !old_wnd.0.is_null() {
            unsafe {
                let _ = DestroyWindow(old_wnd);
            }
        }
        true
    }

    /// Перерисовывает значок трея, если сменилась тема панели задач.
    pub(crate) fn refresh_tray_icon(&mut self) {
        let light = sys::taskbar_light();
        if self.tray_key == Some(light) {
            return;
        }
        let Some(new) = sys::make_icon(self.hinst, light) else {
            if self.icon.is_none() {
                // Запасной путь — значок самого exe (ICON 1 в island.rc), он хотя бы узнаваем.
                // SAFETY: id ресурса передаётся как MAKEINTRESOURCE, указатель не разыменовывается.
                self.icon = unsafe { LoadIconW(Some(self.hinst), PCWSTR(1 as _)) }
                    .or_else(|_| unsafe { LoadIconW(None, IDI_APPLICATION) })
                    .ok();
            }
            return;
        };
        self.tray_key = Some(light);
        // До add_tray значка в трее ещё нет и NIM_MODIFY ничего не сделает — это нормально.
        sys::tray_set_icon(self.hwnd, new);
        if let Some(old) = self.icon.replace(new) {
            if self.icon_owned {
                // SAFETY: старый значок создан нами в make_icon, и оболочка уже получила новый.
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(old);
                }
            }
        }
        self.icon_owned = true;
    }

    /// Номер вкладки (номер вкладки, `tabs`), выбранной в ряду.
    pub(crate) fn normal_id(&self) -> usize {
        self.order.get(self.tab).or_else(|| self.order.last()).copied().unwrap_or(0)
    }

    /// Что показано сейчас: вкладка, страница настроек (`tabs::SETTINGS`) или уведомления (`tabs::NOTIFICATIONS`).
    pub(crate) fn cur_id(&self) -> usize {
        if self.settings_open {
            tabs::SETTINGS
        } else if self.notifications_open {
            tabs::NOTIFICATIONS
        } else {
            self.normal_id()
        }
    }

    /// Центр шестерёнки в логических пикселях окна: у правого края острова в ряду вкладок.
    pub(crate) fn gear_center(&self) -> (f32, f32) {
        let half = self.body_size().0 / self.scale / 2.0;
        (self.win_w as f32 / self.scale / 2.0 + half - 16.0, TAB_TOP + TAB_H / 2.0)
    }

    /// Центр иконки колокольчика: левее шестерёнки и правее карандаша.
    pub(crate) fn bell_center(&self) -> (f32, f32) {
        let (gx, gy) = self.gear_center();
        (gx - 24.0, gy)
    }

    /// Открывает страницу настроек или возвращает на вкладку.
    pub(crate) fn toggle_settings(&mut self) {
        self.tab_prev = Some(self.cur_id());
        self.settings_open = !self.settings_open;
        if self.settings_open {
            self.notifications_open = false;
        }
        self.tab_dir = if self.settings_open { 1.0 } else { -1.0 };
        self.tab_t.value = 0.0;
        self.tab_t.target = 1.0;
        self.ui.blur();
        self.hold_leave();
        self.retarget();
    }

    /// Открывает вкладку уведомлений или возвращает на предыдущую вкладку.
    pub(crate) fn toggle_notifications(&mut self) {
        self.tab_prev = Some(self.cur_id());
        self.notifications_open = !self.notifications_open;
        if self.notifications_open {
            self.settings_open = false;
        }
        self.tab_dir = if self.notifications_open { 1.0 } else { -1.0 };
        self.tab_t.value = 0.0;
        self.tab_t.target = 1.0;
        self.ui.blur();
        self.hold_leave();
        self.retarget();
    }

    /// Остров сам меняет размер под неподвижным курсором (шестерёнка стоит у края, а страница
    /// уже): запоминаем место курсора, и до первого движения мыши остров не сворачивается.
    pub(crate) fn hold_leave(&mut self) {
        let mut p = POINT::default();
        // SAFETY: простой вызов Win32 без указателей на чужую память.
        self.leave_hold = unsafe { GetCursorPos(&mut p) }.ok().map(|_| p);
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
        let custom_changed = self.cfg.custom != new.custom;
        self.cfg.custom = new.custom;
        self.cfg.hidden_tabs = new.hidden_tabs;
        self.cfg.strip_over_maximized = new.strip_over_maximized;
        self.cfg.always_strip = new.always_strip;
        self.cfg.idle_face = new.idle_face;
        self.cfg.collapsed_left = new.collapsed_left;
        self.cfg.collapsed_right = new.collapsed_right;
        self.cfg.todo_clean_done = new.todo_clean_done;
        self.cfg.free_layout = new.free_layout;
        self.cfg.hide_on_screenshot = new.hide_on_screenshot;
        self.cfg.popup_notifications = new.popup_notifications;
        self.tabs.set_compact(!self.cfg.free_layout);
        self.cfg.auto_update = new.auto_update;
        if self.cfg.language != new.language {
            self.cfg.language = new.language;
            // SAFETY: оконный поток, DirectWrite-фабрика жива.
            unsafe { self.refresh_language() };
        }
        if custom_changed {
            // Порядок обновляется раньше ширин: иначе ряд вкладок обратится к удалённой вкладке.
            self.order = self.cfg.visible_tabs();
            self.tab = self.order.iter().position(|&i| i == current).unwrap_or(0);
            // Новые и переименованные вкладки: сетки и ширины подписей в ряду.
            self.tabs.sync_custom(&self.cfg);
            // SAFETY: оконный поток, DirectWrite-фабрика жива.
            unsafe { self.refresh_language() };
        }
        self.order = self.cfg.visible_tabs();
        self.tab = self.order.iter().position(|&i| i == current).unwrap_or(0);
        self.face.set_enabled(self.cfg.idle_face);
        self.face.set_slots(self.cfg.collapsed_left, self.cfg.collapsed_right);
        self.apply_theme();
        self.retarget();
        self.save_pending = true;
    }

    /// Высота панели вкладки: берётся через `Tabs::island_size`.
    pub(crate) fn tab_h(&self, id: usize) -> f32 {
        self.tabs.island_size(id).1
    }

    /// То же для `tabs`, которые отрисовка забрала у `self` на время кадра (`self.tabs` в это время пуст).
    pub(crate) fn tab_w_in(&self, tabs: &tabs::Tabs, id: usize) -> f32 {
        tabs.island_size(id).0.max(self.tab_row_width() + TAB_SIDE_LEFT + TAB_SIDE_RIGHT)
    }

    /// Ширина панели вкладки: не уже ряда вкладок, иначе они обрезаются.
    pub(crate) fn tab_w(&self, id: usize) -> f32 {
        self.tabs.island_size(id).0.max(self.tab_row_width() + TAB_SIDE_LEFT + TAB_SIDE_RIGHT)
    }

    /// Применяет язык из конфига: подписи вкладок меняют ширину, поэтому ряд измеряется заново.
    pub(crate) unsafe fn refresh_language(&mut self) {
        i18n::set(i18n::from_config(&self.cfg.language));
        match measure_tabs(&self.dwrite, self.scale, &self.cfg) {
            Ok(w) => self.tab_widths = w,
            Err(e) => log::error(&format!("не удалось измерить вкладки: {e}")),
        }
        self.retarget();
    }

    pub(crate) unsafe fn apply_config(&mut self, cfg: Config) {
        let current = self.normal_id();
        self.order = cfg.visible_tabs();
        self.tab = self.order.iter().position(|&i| i == current).unwrap_or(0);
        self.cfg = cfg;
        theme::set_glass(self.cfg.glass);
        set_ui_scale(self.cfg.ui_scale);
        self.refresh_language();
        self.apply_monitor(self.mon, true);
        self.face.set_enabled(self.cfg.idle_face);
        self.face.set_slots(self.cfg.collapsed_left, self.cfg.collapsed_right);
        self.apply_theme();
        self.live.set_media_enabled(self.cfg.live_media);
        self.tabs.apply_config(&self.cfg);
        self.register_hotkey();
        start_env_timer(self.hwnd, self.cfg.env_poll_ms);
        self.retarget();
        self.body.snap();
        self.rgn = (-1, -1, -1, -1);
        self.apply_region();
        self.rebuild_hot();
    }

    // ---------- События, анимация, отрисовка ----------

    /// Минимальная ширина сетки виджетов в ячейках: столько целых колонок, сколько помещается
    /// под рядом вкладок. Округление вниз, а не вверх: ширину панели и так дотягивает до ряда
    /// вкладок `tab_w`, а сетка в ней центрируется. С округлением вверх ряд, которому не хватало
    /// нескольких пикселей, добавлял сетке целую пустую колонку справа.
    pub(crate) fn min_grid_cols(&self) -> u8 {
        ((self.tab_row_width() + TAB_SIDE_LEFT + TAB_SIDE_RIGHT - 2.0 * tabs::PAD + tabs::layout::GAP)
            / tabs::layout::CELL)
            .floor()
            .max(1.0) as u8
    }

    pub(crate) fn retarget(&mut self) {
        let min_cols = self.min_grid_cols();
        self.tabs.set_min_cols(min_cols);
        self.body.set_targets(self.collapsed_target(), self.expanded_target());
        // Форма, которой сейчас не видно, переходит к цели без анимации.
        if !self.expanded && self.open.value < 0.001 {
            self.body.snap_expanded();
        } else if self.expanded && self.open.value > 0.999 {
            self.body.snap_collapsed();
        }
        self.retarget_pill();
        let form = self.live_form();
        self.live.set_form(form);
        self.capsule.target = if !self.expanded && !self.hidden && self.live.capsule_active() {
            1.0
        } else {
            0.0
        };
        // Часы, раскладка, Wi-Fi и таймер — только когда свёрнутому острову больше нечего показывать.
        let idle = form == live::Form::Idle && !self.expanded && !self.stripped() && !self.hidden;
        self.live.set_wave_active(self.collapsed_wave_on());
        self.face.set_visible(idle, self.live.timer().is_some());
    }

    /// Активны ли боковые музыкальные волны (строго только в обычной свёрнутой чёлке).
    pub(crate) fn collapsed_wave_on(&self) -> bool {
        self.cfg.live_media
            && self.live.playing()
            && !self.expanded
            && !self.stripped()
            && !self.hidden
            && self.live_form() == live::Form::Idle
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
        self.pill_a.target = if self.settings_open || self.notifications_open { 0.0 } else { 1.0 };
        if jump {
            self.pill_x.snap();
            self.pill_w.snap();
            self.pill_a.snap();
        }
    }

    pub(crate) fn animating(&self) -> bool {
        !(self.body.settled() && self.open.settled() && self.capsule.settled())
            || !(self.pill_x.settled() && self.pill_w.settled() && self.pill_a.settled())
            || !self.tab_t.settled()
            || !self.glow.settled()
            || !self.drag_a.settled()
            || self.face.animating()
            || self.live.animating()
            || self.ui.animating()
            || (self.expanded && self.tabs.animating())
    }

    pub(crate) fn step(&mut self, dt: f32) {
        self.body.step(dt);
        self.open.step(dt);
        self.capsule.step(dt);
        if self.capsule.settled() {
            self.capsule.snap();
        }
        self.ui.step(dt);
        self.tabs.step(dt);
        self.live.step(dt);
        self.face.step(dt);
        self.glow.step(dt);
        if self.glow.settled() {
            self.glow.snap();
        }
        self.drag_a.step(dt);
        if self.drag_a.settled() {
            self.drag_a.snap();
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
        if self.body.settled() {
            self.body.snap();
        }
        if self.open.settled() {
            self.open.snap();
        }
    }
}

/// Создаёт окно подложки `glass_wnd` и инициализирует Composition (`HostBackdropBrush` +
/// `GaussianBlurEffect`) на Win11+ либо `accent blur-behind` на Win10 / при откате.
unsafe fn init_glass_window(
    hinst: HINSTANCE,
    dq: &mut Option<windows::System::DispatcherQueueController>,
) -> windows::core::Result<(HWND, Option<crate::backdrop::Backdrop>, bool)> {
    let build = crate::sys::windows_build_number();
    if crate::backdrop::use_host_backdrop(build) {
        let wnd = crate::wnd::create_glass_window(hinst, true)?;
        match crate::backdrop::enable_host_backdrop(wnd)
            .and_then(|()| crate::backdrop::Backdrop::new(wnd, dq))
        {
            Ok(b) => {
                log::info("стекло: подложка Composition HostBackdropBrush создана");
                return Ok((wnd, Some(b), true));
            }
            Err(e) => {
                log::info(&format!("стекло: Composition недоступна ({e}), откат к accent-размытию"));
                // WS_EX_NOREDIRECTIONBITMAP нельзя снять после создания — пересоздаём окно под SetWindowRgn.
                let _ = DestroyWindow(wnd);
            }
        }
    } else {
        log::info(&format!("стекло: Windows 10 (build {build}), используется accent blur-behind"));
    }
    let wnd = crate::wnd::create_glass_window(hinst, false)?;
    // На Win11 без поддержки композиционных эффектов (RDP / Basic Display Driver / VM)
    // SetWindowCompositionAttribute часто возвращает 1 без реального размытия — не занижаем альфу впустую.
    let blur_ok = if crate::backdrop::use_host_backdrop(build) && !crate::backdrop::effects_supported(wnd) {
        false
    } else {
        crate::wnd::enable_blur_behind(wnd)
    };
    if !blur_ok {
        log::info("стекло: blur-behind недоступен, тело острова остаётся без размытия");
    }
    Ok((wnd, None, blur_ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    impl App {
        pub(crate) fn for_test(cfg: Config) -> Self {
            let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap() };
            Self {
                hwnd: HWND(std::ptr::null_mut()),
                hinst: HINSTANCE(std::ptr::null_mut()),
                icon: None,
                icon_owned: false,
                tray_key: None,
                scale: 1.0,
                mon: HMONITOR(std::ptr::null_mut()),
                win_x: 0,
                win_y: 0,
                win_w: 200,
                win_h: 50,
                placed: (0, 0, 0, 0),
                blur_key: std::cell::Cell::new((0, 0, 0, 0, 0)),
                glass_shape: std::cell::Cell::new((0, 0, 0, 0, 0, 0, 0)),
                glass_wnd: HWND(std::ptr::null_mut()),
                blur_ok: false,
                backdrop: None,
                dq: None,
                transparency: false,
                dwrite,
                gfx: None,
                tab_widths: Vec::new(),
                order: Vec::new(),
                hot: Vec::new(),
                expanded: false,
                covered: false,
                hidden: false,
                shot_until: None,
                leave_armed: false,
                leave_hold: None,
                trim_pending: false,
                rgn: (0, 0, 0, 0),
                rgn_hist: Vec::new(),
                tab: 0,
                settings_open: false,
                notifications_open: false,
                dnd_known: None,
                glow: Spring::unit(0.0, 220.0, 26.0),
                face: face::Face::new(cfg.idle_face, cfg.collapsed_left, cfg.collapsed_right),
                face_tick_on: false,
                save_pending: false,
                body: anim::Body::new((180.0, 24.0)),
                open: Spring::unit(0.0, 420.0, 34.0),
                capsule: Spring::unit(0.0, 340.0, 27.0),
                drop_msg: None,
                drag_a: Spring::unit(0.0, 260.0, 30.0),
                ui: Ui::default(),
                tabs: tabs::Tabs::default(),
                shown_tab: None,
                poll_on: false,
                alarm_at: None,
                locked: false,
                screen_off: false,
                tab_prev: None,
                tab_t: Spring::unit(1.0, 260.0, 30.0),
                pill_x: Spring::new(0.0, 420.0, 30.0),
                pill_w: Spring::new(0.0, 420.0, 30.0),
                pill_a: Spring::unit(1.0, 420.0, 30.0),
                pill_sig: (0, 0, 0),
                tab_dir: 1.0,
                live: live::Live::new(cfg.live_media),
                live_tick_on: LiveTick::Off,
                kbd: false,
                prev_fg: HWND(std::ptr::null_mut()),
                high_surrogate: 0,
                press_pos: None,
                color_retry: sys::ColorRetry::new(),
                cfg,
            }
        }
    }

    #[test]
    fn test_idle_pill_size_and_wave_visibility() {
        let mut cfg = Config::default();
        cfg.idle_face = false;
        let mut app = App::for_test(cfg);
        let base_w = app.collapsed().0;

        app.retarget();
        assert_eq!(app.idle_pill_size().0, base_w);
        assert!(!app.face.visible());

        // Симулируем воспроизведение музыки
        let track = bg::Media {
            title: "Track".into(),
            artist: "Artist".into(),
            playing: true,
            pos: 0.0,
            dur: 100.0,
            at: Instant::now(),
            rate: 1.0,
            art: None,
            key: 1,
            can_seek: true,
            min_seek: 0.0,
            max_seek: 100.0,
            source_name: "Spotify".into(),
            session_count: 1,
        };
        let snap = bg::Snapshot {
            wifi: None,
            bluetooth: None,
            bt_connected: false,
            airplane: None,
            volume: None,
            brightness: None,
            muted: false,
            media: Some(track.clone()),
            live_media: Some(track),
            ssid: None,
            audio: None,
            rev: 1,
            ..Default::default()
        };
        app.live.sync_media(&snap);
        app.retarget();

        // Свёрнутая чёлка не расширяется под эквалайзер, лицо скрыто при idle_face = false, но активны боковые волны
        assert_eq!(app.live_form(), live::Form::Idle);
        assert_eq!(app.idle_pill_size().0, base_w);
        assert!(!app.face.visible());
        assert!(app.collapsed_wave_on());
        while app.live.animating() {
            app.live.step(0.05);
        }
        assert!(app.live.wave_alpha() > 0.99);

        // При активном таймере добавляется только TIMER_SLOT_W, лицо показывает плашку таймера
        app.live.set_timer(Some(live::TimerView {
            frac: 0.5,
            secs: 60,
            paused: false,
            next_ms: 1000,
        }));
        app.retarget();
        assert_eq!(app.idle_pill_size().0, base_w + face::TIMER_SLOT_W);
        assert!(app.face.visible());

        // В режиме полоски боковые волны отключаются и гаснут
        app.cfg.always_strip = true;
        app.retarget();
        assert!(!app.collapsed_wave_on());
        while app.live.animating() {
            app.live.step(0.05);
        }
        assert!(app.live.wave_alpha() < 0.01);

        // В обычной чёлке волны возвращаются, а при разворачивании острова — снова гаснут
        app.cfg.always_strip = false;
        app.retarget();
        assert!(app.collapsed_wave_on());
        while app.live.animating() {
            app.live.step(0.05);
        }
        assert!(app.live.wave_alpha() > 0.99);

        app.expanded = true;
        app.retarget();
        assert!(!app.collapsed_wave_on());
        assert!(!app.face.visible());
        while app.live.animating() {
            app.live.step(0.05);
        }
        assert!(app.live.wave_alpha() < 0.01);
    }
}
