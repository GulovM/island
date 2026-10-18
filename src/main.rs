#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

mod anim;
mod bg;
mod bright;
mod config;
mod face;
mod gfx;
mod i18n;
mod log;
mod online;
mod perf;
mod sys;
mod theme;
mod tabs;
mod thumbs;
mod icons;
mod img;
mod live;
mod ui;
mod events;
mod input;
mod render;
mod window;
mod wnd;

use std::cell::{Cell, RefCell};
use std::time::Instant;

use anim::Spring;
use config::Config;
use gfx::Gfx;
use wnd::*;
use ui::{Canvas, KeyEv, Ui};
use windows_numerics::Vector2;
use windows::core::{w, Interface, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, TRUE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{
    CreateRectRgn, EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, SetWindowRgn,
    HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, ShellExecuteW, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::*;

// ---------- Раскладка (логические пиксели, умножаются на DPI-масштаб) ----------

const TAB_COUNT: usize = 7;
const TABS: [&str; TAB_COUNT] = ["Controls", "Today", "Stats", "Clipboard", "Apps", "Wallpaper", "Power"];
/// Размер панели для каждой вкладки (ширина, высота) — как на скриншотах; последний — страница настроек.
const TAB_SIZES: [(f32, f32); 8] = [
    (430.0, 282.0),
    (660.0, 304.0),
    (640.0, 282.0),
    (590.0, 300.0),
    (480.0, 420.0),
    (700.0, 236.0),
    (420.0, 112.0),
    (620.0, 398.0),
];
/// Высота «тонкой полоски», в которую остров сворачивается поверх развёрнутого на весь экран окна.
const STRIP_H: f32 = 3.0;
const TAB_H: f32 = 17.0;
const TAB_GAP: f32 = 4.0;
const TAB_PAD: f32 = 8.0;
const TAB_TOP: f32 = 9.0;
/// Отступ ряда вкладок от краёв панели.
const TAB_SIDE: f32 = 28.0;
/// Размер окна: самая большая панель (650×400) + запас под перелёт пружины и «ушки».
const WIN_LOGICAL: (f32, f32) = (760.0, 500.0);
/// Допуск вокруг раскрытой формы, где мышь ещё считается «на острове» (только слева/справа/снизу).
const HIT_SLOP: f32 = 4.0;
/// Радиус шестерёнки настроек в ряду вкладок.
const GEAR_R: f32 = 9.0;
/// Радиус «ушек» — загнутых плавных переходов чёлки в верхний край экрана.
const EAR: f32 = 8.0;
/// Высота горячей зоны на верхнем краю других мониторов, физические пиксели.
const HOT_H: i32 = 2;
/// Через сколько мс пробовать пересоздать графику после потери устройства.
const RECOVER_MS: u32 = 500;

const WM_MOUSELEAVE: u32 = 0x02A3;
/// Цвет оформления окон (акцент) поменялся — его подстраивает, например, Wallpaper Engine под обои.
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 7;
const WTS_SESSION_UNLOCK: usize = 8;
/// Будит главный цикл из хука окон (колбэк хука сам сообщение не возвращает из `GetMessage`).
const WM_RECHECK: u32 = WM_APP + 1;
/// Сообщение от значка в трее.
const WM_TRAY: u32 = WM_APP + 2;
const LEAVE_TIMER: usize = 1;
const ENV_TIMER: usize = 2;
const RECOVER_TIMER: usize = 3;
/// Мигание курсора в текстовом поле: работает только пока поле в фокусе.
const CARET_TIMER: usize = 4;
const CARET_MS: u32 = 530;
/// Секундный опрос видимой вкладки (Stats, Controls, Today).
const POLL_TIMER: usize = 5;
const POLL_MS: u32 = 1000;
/// Срабатывание таймера на вкладке Today, в том числе при свёрнутом острове.
const ALARM_TIMER: usize = 6;
/// Пауза между сворачиванием и эмуляцией Ctrl+V: окну нужно вернуть фокус.
const PASTE_TIMER: usize = 7;
const PASTE_DELAY_MS: u32 = 160;
/// Срок жизни уведомления в свёрнутой форме.
const TOAST_TIMER: usize = 8;
/// Кадры мини-плеера (анимация эквалайзера и прогресс): ~30 в секунду, только пока он виден и
/// играет музыка. Таймер, а не vsync, — вдвое меньше кадров при той же плавности для 3 столбиков.
const LIVE_TIMER: usize = 9;
const LIVE_TICK_MS: u32 = 33;
/// Смена минуты для часов на свёрнутом острове: срабатывает раз в минуту и только пока они видны.
const FACE_TIMER: usize = 10;
/// Файл в `%APPDATA%\island` с выбором режима полоски: «1» — включён.
const STRIP_FILE: &str = "strip.txt";
const WM_CLIPBOARDUPDATE: u32 = 0x031D;
const WM_HOTKEY: u32 = 0x0312;
const HOTKEY_ID: i32 = 1;
/// Сдвиг содержимого при смене вкладки (логические пиксели).
const SLIDE: f32 = 16.0;

const CMD_SETTINGS: usize = 1;
const CMD_RELOAD: usize = 2;
const CMD_AUTOSTART: usize = 3;
const CMD_EXIT: usize = 4;

// ---------- Приложение ----------

/// Невидимая полоска на верхнем краю монитора, где острова сейчас нет.
struct HotZone {
    hwnd: HWND,
    mon: HMONITOR,
}

struct App {
    hwnd: HWND,
    hinst: HINSTANCE,
    icon: Option<HICON>,
    cfg: Config,
    scale: f32,
    mon: HMONITOR,
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    /// Последние применённые x, y, w, h окна — чтобы не дёргать `SetWindowPos` зря.
    placed: (i32, i32, i32, i32),

    dwrite: IDWriteFactory,
    /// `None` — графика потеряна и ждёт пересоздания (рисовать нечем).
    gfx: Option<Gfx>,
    /// Ширина каждой вкладки по её индексу в `TABS`.
    tab_widths: [f32; TAB_COUNT],
    /// Видимые вкладки в порядке показа: `order[позиция] = индекс в TABS` (скрытые не входят).
    order: Vec<usize>,
    hot: Vec<HotZone>,

    expanded: bool,
    /// Поверх острова развёрнуто окно на весь экран: в свёрнутом виде он — тонкая полоска.
    covered: bool,
    /// Активна полноэкранная игра (D3D): остров скрыт совсем.
    hidden: bool,
    leave_armed: bool,
    /// Свернулись и успокоились — пора сбросить рабочий набор.
    trim_pending: bool,
    /// Последний применённый регион окна: (left, right, bottom) в пикселях окна.
    rgn: (i32, i32, i32),
    /// Позиция выбранной вкладки в `order`.
    tab: usize,
    /// Вместо вкладки показана страница настроек (шестерёнка).
    settings_open: bool,
    /// Свечение острова, пока тянут ползунок радиуса углов.
    glow: Spring,
    /// Время, раскладка и Wi-Fi на свёрнутом острове, когда музыка не играет.
    face: face::Face,
    face_tick_on: bool,
    /// Настройки изменены и ждут записи в config.toml (пишем, когда кнопка мыши отпущена).
    save_pending: bool,
    w: Spring,
    h: Spring,
    open: Spring,

    /// Тулкит и состояние вкладок.
    ui: Ui,
    tabs: tabs::Tabs,
    /// Вкладка, которая сейчас на экране (индекс в `TABS`), и включённые для неё таймеры.
    shown_tab: Option<usize>,
    poll_on: bool,
    alarm_at: Option<Instant>,
    /// Предыдущая вкладка (индекс в `TABS`) на время кроссфейда и прогресс перехода 0..1.
    tab_prev: Option<usize>,
    tab_t: Spring,
    /// Направление сдвига при смене вкладки: +1 — новая справа.
    tab_dir: f32,
    /// Индикатор в свёрнутой форме: мини-плеер и уведомления.
    live: live::Live,
    /// Включён ли секундный тик мини-плеера.
    live_tick_on: bool,
    /// Режим «полоска над развёрнутыми окнами» (по умолчанию выключен).
    /// Клавиатурный ввод включён: окно временно активно и принимает фокус.
    kbd: bool,
    /// Окно, которое было активным до включения ввода.
    prev_fg: HWND,
    /// Ожидающая вторая половина суррогатной пары из `WM_CHAR`.
    high_surrogate: u32,
}

impl App {
    fn new(cfg: Config) -> Result<Self> {
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
                tab: 0,
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
                tab_dir: 1.0,
                live: live::Live::new(cfg.live_media),
                live_tick_on: false,
                kbd: false,
                prev_fg: HWND(std::ptr::null_mut()),
                high_surrogate: 0,
                cfg,
            };
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

    fn collapsed(&self) -> (f32, f32) {
        self.cfg.collapsed
    }

    /// Акцент, который сейчас действует (свой или системный).
    fn accent(&self) -> D2D1_COLOR_F {
        theme::get().accent
    }

    /// Цвета, которые сейчас должны действовать: (фон, акцент). С «Цвет системы» — из Windows
    /// (там же оказываются цвета, подстроенные Wallpaper Engine под обои).
    fn effective_colors(&self) -> (theme::Rgb, theme::Rgb) {
        if self.cfg.system_colors {
            if let Some(sys) = sys::system_colors() {
                return theme::from_system(sys, !sys::light_mode());
            }
        }
        (self.cfg.bg, self.cfg.accent)
    }

    /// Пересчитывает палитру. `true` — цвета изменились и нужен кадр.
    fn apply_theme(&mut self) -> bool {
        let (bg, accent) = self.effective_colors();
        let p = theme::derive(bg, accent);
        if p == theme::get() {
            return false;
        }
        theme::set(p);
        true
    }

    /// Номер вкладки (индекс в `TABS`), выбранной в ряду.
    fn normal_id(&self) -> usize {
        self.order.get(self.tab).or_else(|| self.order.last()).copied().unwrap_or(0)
    }

    /// Что показано сейчас: вкладка или страница настроек (`tabs::SETTINGS`).
    fn cur_id(&self) -> usize {
        if self.settings_open {
            tabs::SETTINGS
        } else {
            self.normal_id()
        }
    }

    /// Центр шестерёнки в логических пикселях окна: у правого края острова в ряду вкладок.
    fn gear_center(&self) -> (f32, f32) {
        let half = self.w.value / self.scale / 2.0;
        (self.win_w as f32 / self.scale / 2.0 + half - 16.0, TAB_TOP + TAB_H / 2.0)
    }

    /// Открывает страницу настроек или возвращает на вкладку.
    fn toggle_settings(&mut self) {
        self.tab_prev = Some(self.cur_id());
        self.settings_open = !self.settings_open;
        self.tab_dir = if self.settings_open { 1.0 } else { -1.0 };
        self.tab_t.value = 0.0;
        self.tab_t.target = 1.0;
        self.ui.blur();
        self.retarget();
    }

    /// Применяет настройки со страницы: внешний вид живьём, запись в файл — когда мышь отпущена.
    fn apply_settings(&mut self, new: Config) {
        let current = self.normal_id();
        self.cfg.accent = new.accent;
        self.cfg.bg = new.bg;
        self.cfg.system_colors = new.system_colors;
        self.cfg.corner_radius = new.corner_radius;
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

    /// Ширина панели вкладки: не уже ряда вкладок, иначе они обрезаются.
    /// Высота панели вкладки: у «Управления» прибавляется полоска яркости, если она поддерживается.
    fn tab_h(id: usize) -> f32 {
        let extra = if id == tabs::CONTROLS && bright::available() { 34.0 } else { 0.0 };
        TAB_SIZES[id].1 + extra
    }

    fn tab_w(&self, id: usize) -> f32 {
        TAB_SIZES[id].0.max(self.tab_row_width() + 2.0 * TAB_SIDE)
    }

    /// Применяет язык из конфига: подписи вкладок меняют ширину, поэтому ряд измеряется заново.
    unsafe fn refresh_language(&mut self) {
        i18n::set(i18n::from_config(&self.cfg.language));
        match measure_tabs(&self.dwrite, self.scale) {
            Ok(w) => self.tab_widths = w,
            Err(e) => log::error(&format!("не удалось измерить вкладки: {e}")),
        }
        self.retarget();
    }

    unsafe fn apply_config(&mut self, cfg: Config) {
        let current = self.normal_id();
        self.order = cfg.resolve_visible(&TABS);
        self.tab = self.order.iter().position(|&i| i == current).unwrap_or(0);
        self.cfg = cfg;
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

    fn retarget(&mut self) {
        let (w, h) = self.target_size();
        self.w.target = w;
        self.h.target = h;
        let form = self.live_form();
        self.live.set_form(form);
        // Часы, раскладка и Wi-Fi — только когда свёрнутому острову больше нечего показывать.
        let idle = form == live::Form::Idle && !self.expanded && !self.stripped() && !self.hidden;
        self.face.set_visible(idle);
    }

    fn animating(&self) -> bool {
        !(self.w.settled() && self.h.settled() && self.open.settled())
            || !self.tab_t.settled()
            || !self.glow.settled()
            || self.face.animating()
            || self.live.animating()
            || self.ui.animating()
            || (self.expanded && self.tabs.animating())
    }

    fn step(&mut self, dt: f32) {
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

fn main() {
    log::init();
    if !sys::acquire_single_instance() {
        log::info("Island уже запущен, второй экземпляр завершается");
        return;
    }
    if let Err(e) = run() {
        log::error(&format!("фатальная ошибка: {e}"));
    }
}

fn run() -> Result<()> {
    let mut cfg = config::load();
    // Прежний выбор из strip.txt переносится в config.toml один раз.
    if let Some(l) = tabs::read_lines(STRIP_FILE).first() {
        cfg.strip_over_maximized = l.trim() == "1";
        config::save(&cfg, &TABS);
        let _ = std::fs::remove_file(sys::data_dir().join(STRIP_FILE));
    }
    i18n::set(i18n::from_config(&cfg.language));
    log::info("запуск");
    let mut app = App::new(cfg)?;
    let mut last = Instant::now();
    let mut dirty = true;
    let mut perf = perf::Perf::new();

    unsafe {
        // WinRT и COM нужны вкладкам (Wi-Fi, медиа, громкость): MTA, чтобы `.get()` не зависал.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        bg::start(app.hwnd);
        // Мини-плееру нужно знать о музыке и при свёрнутом острове: подписка на события сессии.
        bg::send(bg::Cmd::Media);
        let cfg = app.cfg.clone();
        app.tabs.load(&cfg);
        if let Err(e) = AddClipboardFormatListener(app.hwnd) {
            log::error(&format!("не удалось подписаться на буфер обмена: {e}"));
        }
        app.add_tray();
        app.register_hotkey();
        if let Err(e) = WTSRegisterSessionNotification(app.hwnd, 0) {
            log::error(&format!("не удалось подписаться на блокировку сессии: {e}"));
        }
        // Смена активного окна и сворачивание/разворачивание — мгновенная реакция;
        // развернуть окно без смены фокуса ловит редкий опрос.
        let hooks = [
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(on_win_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            ),
            SetWinEventHook(
                EVENT_SYSTEM_MINIMIZESTART,
                EVENT_SYSTEM_MINIMIZEEND,
                None,
                Some(on_win_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            ),
        ];
        SetTimer(Some(app.hwnd), ENV_TIMER, app.cfg.env_poll_ms, None);

        let mut msg = MSG::default();
        'outer: loop {
            // Простой = 0% CPU: блокируемся в GetMessage. Во время анимации — только PeekMessage.
            // Без графики рисовать нечем — тоже ждём в GetMessage (разбудит таймер восстановления).
            let busy = app.gfx.is_some() && (app.animating() || dirty);
            if busy {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        break 'outer;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            } else {
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                last = Instant::now();
            }

            let events: Vec<Ev> = EVENTS.with(|v| std::mem::take(&mut *v.borrow_mut()));
            for e in events {
                dirty |= app.handle(e);
            }
            dirty |= app.sync_tabs();

            let now = Instant::now();
            let dt = (now - last).as_secs_f32().min(0.05);
            last = now;

            if app.gfx.is_some() && (app.animating() || dirty) {
                app.step(dt);
                // Регион окна обновляем до показа кадра, чтобы он не отставал от картинки.
                app.apply_region();
                if let Err(e) = app.render() {
                    log::error(&format!("ошибка отрисовки, пересоздаю графику: {e}"));
                    app.lose_gfx();
                    dirty = true;
                } else {
                    perf.frame();
                    app.check_hover();
                    dirty = app.animating();
                    if !dirty {
                        perf.flush();
                    }
                    app.sync_focus();
                    app.update_ime();
                    dirty |= app.sync_tabs();
                }
            }

            // Свернулись и успокоились — отдаём системе всё, что не нужно в простое.
            if app.trim_pending && !app.expanded && !app.animating() && !dirty && app.gfx.is_some() {
                app.trim_pending = false;
                sys::trim_working_set();
            }
        }
        for h in hooks {
            let _ = UnhookWinEvent(h);
        }
        let _ = RemoveClipboardFormatListener(app.hwnd);
        sys::tray_remove(app.hwnd);
    }
    log::info("выход");
    Ok(())
}
