#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod bg;
mod config;
mod face;
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

use std::cell::{Cell, RefCell};
use std::time::Instant;

use anim::Spring;
use config::Config;
use ui::{Canvas, Fonts, KeyEv, Ui};
use windows_numerics::Vector2;
use windows::core::{w, Interface, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, TRUE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
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

const TABS: [&str; 7] = ["Controls", "Today", "Stats", "Clipboard", "Apps", "Wallpaper", "Power"];
/// Размер панели для каждой вкладки (ширина, высота) — как на скриншотах; последний — страница настроек.
const TAB_SIZES: [(f32, f32); 8] = [
    (400.0, 242.0),
    (550.0, 275.0),
    (550.0, 210.0),
    (550.0, 278.0),
    (450.0, 395.0),
    (650.0, 194.0),
    (400.0, 103.0),
    (580.0, 322.0),
];
/// Высота «тонкой полоски», в которую остров сворачивается поверх развёрнутого на весь экран окна.
const STRIP_H: f32 = 3.0;
/// Общий коэффициент увеличения интерфейса поверх масштаба DPI (размеры и шрифты растут вместе).
const UI_ZOOM: f32 = 1.2;
const TAB_H: f32 = 17.0;
const TAB_GAP: f32 = 4.0;
const TAB_PAD: f32 = 8.0;
const TAB_TOP: f32 = 9.0;
/// Отступ ряда вкладок от краёв панели.
const TAB_SIDE: f32 = 28.0;
/// Размер окна: самая большая панель (650×400) + запас под перелёт пружины и «ушки».
const WIN_LOGICAL: (f32, f32) = (700.0, 430.0);
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

// ---------- События от оконной процедуры к главному циклу ----------

enum Ev {
    /// Мышь двигается над окном (окно обрезано регионом до области острова).
    Move,
    /// Система сообщила, что мышь покинула окно.
    MouseLeft,
    /// Истекла задержка сворачивания.
    LeaveTimer,
    /// Левая кнопка нажата / отпущена (координаты клиентской области).
    Down(i32, i32),
    Up,
    /// Колесо мыши, сырое значение `WHEEL_DELTA` (120 на «ступеньку»).
    Wheel(i32),
    /// Символ из `WM_CHAR` и код клавиши из `WM_KEYDOWN`.
    Char(u32),
    Key(u32),
    /// Окно потеряло активацию (клик в другое окно).
    Deactivated,
    /// Таймер мигания курсора.
    Caret,
    /// Пора перепроверить, что сейчас на экране (развёрнутое окно, игра).
    Recheck,
    /// Событие мыши от значка в трее (`WM_RBUTTONUP` и т. п.).
    Tray(u32),
    /// Курсор у верхнего края другого монитора (хэндл окна горячей зоны).
    Hot(isize),
    /// Сработала глобальная горячая клавиша.
    Hotkey,
    /// Сменились мониторы, разрешение или DPI.
    Display,
    /// Компьютер проснулся или разблокирован.
    Resume,
    Lock,
    RecoverTimer,
    /// Explorer перезапустился — значок трея нужно добавить заново.
    TaskbarCreated,
    /// Секундный тик видимой вкладки.
    Poll,
    /// Сработал таймер вкладки Today.
    Alarm,
    /// Фоновый поток или поток миниатюр сообщил о новых данных.
    Bg,
    /// Изменился буфер обмена.
    Clip,
    /// Пора вставлять текст (Ctrl+V) в окно, где работали.
    PasteNow,
    /// Громкость (0..1) или «без звука» изменились.
    Volume(f32, bool),
    /// Изменилось питание: сеть/батарея или заряд.
    PowerChange,
    /// Уведомление в свёрнутой форме отработало своё.
    ToastTimer,
    /// Секундный тик мини-плеера.
    LiveTick,
    /// Правый клик по острову.
    RightClick,
    /// Началась новая минута: обновить часы на свёрнутом острове.
    FaceTick,
    /// Системные цвета (акцент Windows, светлая/тёмная тема) изменились.
    SysColors,
}

thread_local! {
    static EVENTS: RefCell<Vec<Ev>> = const { RefCell::new(Vec::new()) };
    static MAIN_WND: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static TASKBAR_MSG: Cell<u32> = const { Cell::new(0) };
}

fn push(e: Ev) {
    EVENTS.with(|v| v.borrow_mut().push(e));
}

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let main = MAIN_WND.with(|w| w.get());
    let _ = PostMessageW(Some(main), WM_RECHECK, WPARAM(0), LPARAM(0));
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let mut t = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut t);
            push(Ev::Move);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            push(Ev::MouseLeft);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == LEAVE_TIMER => {
            push(Ev::LeaveTimer);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == ENV_TIMER => {
            push(Ev::Recheck);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == RECOVER_TIMER => {
            push(Ev::RecoverTimer);
            LRESULT(0)
        }
        WM_RECHECK => {
            push(Ev::Recheck);
            LRESULT(0)
        }
        WM_TRAY => {
            push(Ev::Tray((lp.0 & 0xffff) as u32));
            LRESULT(0)
        }
        WM_HOTKEY => {
            push(Ev::Hotkey);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            push(Ev::Display);
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wp.0 as u32 == PBT_APMRESUMEAUTOMATIC || wp.0 as u32 == PBT_APMRESUMESUSPEND {
                push(Ev::Resume);
            } else if wp.0 as u32 == PBT_APMPOWERSTATUSCHANGE {
                push(Ev::PowerChange);
            }
            LRESULT(1)
        }
        WM_WTSSESSION_CHANGE => {
            match wp.0 {
                WTS_SESSION_LOCK => push(Ev::Lock),
                WTS_SESSION_UNLOCK => push(Ev::Resume),
                _ => {}
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lp.0 & 0xffff) as i16 as i32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
            // Захват мыши: перетаскивание ползунка и выделение текста за пределами острова.
            SetCapture(hwnd);
            push(Ev::Down(x, y));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            push(Ev::Up);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            push(Ev::RightClick);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            push(Ev::Wheel(((wp.0 >> 16) & 0xffff) as i16 as i32));
            LRESULT(0)
        }
        WM_CHAR => {
            push(Ev::Char(wp.0 as u32));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            push(Ev::Key(wp.0 as u32));
            LRESULT(0)
        }
        WM_ACTIVATE => {
            if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                push(Ev::Deactivated);
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == CARET_TIMER => {
            push(Ev::Caret);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == POLL_TIMER => {
            push(Ev::Poll);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == ALARM_TIMER => {
            push(Ev::Alarm);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == PASTE_TIMER => {
            push(Ev::PasteNow);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TOAST_TIMER => {
            push(Ev::ToastTimer);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == LIVE_TIMER => {
            push(Ev::LiveTick);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == FACE_TIMER => {
            push(Ev::FaceTick);
            LRESULT(0)
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            push(Ev::SysColors);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // Тема и акцент приходят как «ImmersiveColorSet»; остальные настройки не интересуют.
            if lp.0 != 0 && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                push(Ev::SysColors);
            }
            LRESULT(0)
        }
        bg::WM_BG => {
            push(Ev::Bg);
            LRESULT(0)
        }
        bg::WM_VOLUME => {
            push(Ev::Volume(wp.0 as f32 / 1000.0, lp.0 != 0));
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE => {
            push(Ev::Clip);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        m if m != 0 && m == TASKBAR_MSG.with(|c| c.get()) => {
            push(Ev::TaskbarCreated);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Окно горячей зоны на верхнем краю другого монитора: почти невидимое, но ловит мышь.
unsafe extern "system" fn hot_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            push(Ev::Hot(hwnd.0 as isize));
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn enum_monitor(m: HMONITOR, _dc: HDC, rc: *mut RECT, lp: LPARAM) -> BOOL {
    let list = &mut *(lp.0 as *mut Vec<(HMONITOR, RECT)>);
    list.push((m, *rc));
    TRUE
}

/// Состояние перебора окон в `probe`.
struct ScanCtx {
    own: HWND,
    mon: RECT,
    covered: bool,
}

unsafe extern "system" fn scan_window(hwnd: HWND, lp: LPARAM) -> BOOL {
    let ctx = &mut *(lp.0 as *mut ScanCtx);
    if hwnd == ctx.own || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return TRUE;
    }
    // Оверлеи, всплывашки и наши зоны окном приложения не считаются.
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TRANSPARENT.0) != 0 {
        return TRUE;
    }
    // Скрытые окна UWP и окна с других виртуальных рабочих столов.
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut std::ffi::c_void,
        size_of::<u32>() as u32,
    );
    if cloaked != 0 {
        return TRUE;
    }
    let mut buf = [0u16; 64];
    let n = GetClassNameW(hwnd, &mut buf).max(0) as usize;
    match String::from_utf16_lossy(&buf[..n]).as_str() {
        // Панель задач не закрывает остров и не должна менять состояние.
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" => return TRUE,
        // Дошли до рабочего стола — выше него ничего нет.
        "Progman" | "WorkerW" => {
            ctx.covered = false;
            return BOOL(0);
        }
        _ => {}
    }
    let mut rc = RECT::default();
    if GetWindowRect(hwnd, &mut rc).is_err() {
        return TRUE;
    }
    let m = ctx.mon;
    let on_monitor = rc.right > m.left && rc.left < m.right && rc.bottom > m.top && rc.top < m.bottom;
    // Крошечные «видимые» служебные окна пропускаем.
    if !on_monitor || rc.right - rc.left < 32 || rc.bottom - rc.top < 32 {
        return TRUE;
    }
    let covers = rc.left <= m.left && rc.top <= m.top && rc.right >= m.right && rc.bottom >= m.bottom;
    ctx.covered = IsZoomed(hwnd).as_bool() || covers;
    BOOL(0)
}

unsafe fn monitor_scale(mon: HMONITOR) -> f32 {
    let (mut x, mut y) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    x as f32 / 96.0 * UI_ZOOM
}

// ---------- Графика (пересоздаётся целиком после потери устройства) ----------

struct Gfx {
    device: ID3D11Device,
    d2d: ID2D1Factory1,
    swapchain: IDXGISwapChain1,
    dc: ID2D1DeviceContext,
    target: Option<ID2D1Bitmap1>,
    brush: ID2D1SolidColorBrush,
    text: IDWriteTextFormat,
    /// Шрифты тулкита (логические размеры, масштаб даёт трансформация).
    fonts: Fonts,
    /// Штрих с круглыми концами для дуг-колец.
    round: ID2D1StrokeStyle1,
    _dcomp: IDCompositionDevice,
    _dcomp_target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

unsafe fn make_text(dwrite: &IDWriteFactory, scale: f32) -> Result<IDWriteTextFormat> {
    let family = if ui::font_exists(dwrite, w!("Segoe UI Variable Text")) { w!("Segoe UI Variable Text") } else { w!("Segoe UI") };
    let text = dwrite.CreateTextFormat(
        family,
        None,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STRETCH_NORMAL,
        8.5 * scale,
        w!("ru-RU"),
    )?;
    text.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
    text.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    Ok(text)
}

/// Ширина подписи каждой вкладки (логические пиксели, вместе с внутренними полями).
unsafe fn measure_tabs(dwrite: &IDWriteFactory, scale: f32) -> Result<[f32; 7]> {
    let text = make_text(dwrite, scale)?;
    let mut widths = [0.0f32; 7];
    for (i, name) in TABS.iter().enumerate() {
        let wide: Vec<u16> = name.encode_utf16().collect();
        let layout = dwrite.CreateTextLayout(&wide, &text, 1000.0, 100.0)?;
        let mut m = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut m)?;
        widths[i] = m.widthIncludingTrailingWhitespace / scale + TAB_PAD * 2.0;
    }
    Ok(widths)
}

impl Gfx {
    /// Окно и буфер фиксированы на максимум и меняются только при смене DPI: анимируется
    /// форма внутри, поэтому геометрия и картинка не рассинхронизируются.
    /// Прежний `Gfx` для этого окна к моменту вызова должен быть уничтожен (цель DComp одна).
    unsafe fn new(hwnd: HWND, w: i32, h: i32, scale: f32, dwrite: &IDWriteFactory) -> Result<Self> {
        let mut device: Option<ID3D11Device> = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
        let device = device.unwrap();
        let dxgi: IDXGIDevice = device.cast()?;
        let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w as u32,
            Height: h as u32,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };
        let swapchain = factory.CreateSwapChainForComposition(&device, &desc, None)?;

        let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let d2d_dev = d2d.CreateDevice(&dxgi)?;
        let dc = d2d_dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        let brush = dc.CreateSolidColorBrush(&theme::get().bg, None)?;
        let text = make_text(dwrite, scale)?;
        let fonts = Fonts::new(dwrite)?;
        let round = d2d.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES1 {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
                transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
            },
            None,
        )?;

        let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
        let dcomp_target = dcomp.CreateTargetForHwnd(hwnd, true)?;
        let visual = dcomp.CreateVisual()?;
        visual.SetContent(&swapchain)?;
        dcomp_target.SetRoot(&visual)?;
        dcomp.Commit()?;

        let mut g = Gfx {
            device,
            d2d,
            swapchain,
            dc,
            target: None,
            brush,
            text,
            fonts,
            round,
            _dcomp: dcomp,
            _dcomp_target: dcomp_target,
            _visual: visual,
        };
        g.create_target()?;
        Ok(g)
    }

    unsafe fn create_target(&mut self) -> Result<()> {
        let surface: IDXGISurface = self.swapchain.GetBuffer(0)?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        self.target = Some(self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?);
        Ok(())
    }

    /// Новый размер буфера и шрифта при смене DPI (остров переехал на монитор с другим масштабом).
    unsafe fn resize(&mut self, w: i32, h: i32, scale: f32, dwrite: &IDWriteFactory) -> Result<()> {
        self.dc.SetTarget(None);
        self.target = None;
        self.swapchain.ResizeBuffers(0, w as u32, h as u32, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
        self.create_target()?;
        self.text = make_text(dwrite, scale)?;
        self.fonts.clear_layouts();
        Ok(())
    }

    /// Устройство D3D потеряно (сброс драйвера, смена GPU, сон) — нужно пересоздать всё.
    unsafe fn removed(&self) -> bool {
        self.device.GetDeviceRemovedReason().is_err()
    }

    unsafe fn draw_text(&self, s: &str, x: f32, y: f32, w: f32, h: f32, color: D2D1_COLOR_F) {
        self.brush.SetColor(&color);
        if let Some(layout) = self.fonts.layout(s, &self.text, w, h) {
            self.dc.DrawTextLayout(Vector2 { X: x, Y: y }, &layout, &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
            return;
        }
        sys::with_utf16(s, |wide| {
            self.dc.DrawText(
                wide,
                &self.text,
                &D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h },
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        });
    }
}

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
    tab_widths: [f32; 7],
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
    strip_mode: bool,
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
                strip_mode: tabs::read_lines(STRIP_FILE).first().is_some_and(|l| l.trim() == "1"),
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
        self.order[self.tab.min(self.order.len() - 1)]
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
        self.cfg.tabs = new.tabs;
        self.cfg.hidden_tabs = new.hidden_tabs;
        self.cfg.idle_face = new.idle_face;
        self.order = self.cfg.resolve_visible(&TABS);
        self.tab = self.order.iter().position(|&i| i == current).unwrap_or(0);
        self.face.set_enabled(self.cfg.idle_face);
        self.apply_theme();
        self.retarget();
        self.save_pending = true;
    }

    unsafe fn place_window(&mut self, force: bool) {
        let now = (self.win_x, self.win_y, self.win_w, self.win_h);
        if !force && now == self.placed {
            return;
        }
        self.placed = now;
        let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), now.0, now.1, now.2, now.3, SWP_NOACTIVATE);
    }

    fn target_size(&self) -> (f32, f32) {
        let collapsed = self.collapsed();
        let (w, h) = if self.expanded {
            let id = self.cur_id();
            (self.tab_w(id), TAB_SIZES[id].1)
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
    /// режим полного сворачивания (правый клик по острову). Иначе остров остаётся обычной чёлкой.
    fn stripped(&self) -> bool {
        self.covered && self.strip_mode
    }

    /// Правый клик по раскрытому острову включает/выключает режим полоски; действует с момента,
    /// когда мышь уйдёт с острова и он свернётся. Выбор запоминается в `strip.txt`.
    unsafe fn toggle_strip_mode(&mut self) -> bool {
        if !self.expanded || !self.cursor_inside() {
            return false;
        }
        self.strip_mode = !self.strip_mode;
        tabs::write_lines(STRIP_FILE, &[(self.strip_mode as u8).to_string()]);
        self.retarget();
        true
    }

    /// Форма свёрнутого острова: мини-плеер/уведомление показываются, только когда остров
    /// свёрнут, виден и ничем не закрыт.
    fn live_form(&self) -> live::Form {
        if self.expanded || self.stripped() || self.hidden {
            live::Form::Idle
        } else {
            self.live.form()
        }
    }

    /// Показывает уведомление в свёрнутой форме. `true` — нужен кадр.
    unsafe fn show_toast(&mut self, t: live::Toast) -> bool {
        if !self.cfg.live_toasts || self.expanded || self.stripped() || self.hidden || self.gfx.is_none() {
            return false;
        }
        self.live.show(t);
        SetTimer(Some(self.hwnd), TOAST_TIMER, t.lifetime().as_millis() as u32, None);
        self.retarget();
        true
    }

    /// Ширина панели вкладки: не уже ряда вкладок, иначе они обрезаются.
    fn tab_w(&self, id: usize) -> f32 {
        TAB_SIZES[id].0.max(self.tab_row_width() + 2.0 * TAB_SIDE)
    }

    unsafe fn set_expanded(&mut self, expanded: bool) {
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
    unsafe fn apply_monitor(&mut self, mon: HMONITOR, snap: bool) {
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
    unsafe fn move_to(&mut self, mon: HMONITOR) {
        self.disarm_leave();
        self.expanded = false;
        self.open.target = 0.0;
        self.open.snap();
        self.apply_monitor(mon, true);
        self.refresh_environment();
        self.rebuild_hot();
    }

    /// Пересоздаёт горячие зоны: по одной на каждый монитор, кроме того, где остров.
    unsafe fn rebuild_hot(&mut self) {
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

    unsafe fn on_hot(&mut self, hwnd: isize) -> bool {
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
    unsafe fn reconfigure(&mut self) -> bool {
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
    fn hit_rect(&self) -> (i32, i32, i32) {
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
    unsafe fn apply_region(&mut self) {
        let rect = self.hit_rect();
        if rect == self.rgn {
            return;
        }
        self.rgn = rect;
        let rgn = CreateRectRgn(rect.0, 0, rect.1, rect.2);
        // Система забирает регион себе, удалять его не нужно.
        SetWindowRgn(self.hwnd, Some(rgn), false);
    }

    unsafe fn cursor_inside(&self) -> bool {
        let mut p = POINT::default();
        if GetCursorPos(&mut p).is_err() {
            return true;
        }
        let (l, r, b) = self.hit_rect();
        let (x, y) = (p.x - self.win_x, p.y - self.win_y);
        x >= l && x < r && y >= 0 && y < b
    }

    unsafe fn arm_leave(&mut self) {
        if !self.leave_armed {
            SetTimer(Some(self.hwnd), LEAVE_TIMER, self.cfg.leave_delay_ms, None);
            self.leave_armed = true;
        }
    }

    unsafe fn disarm_leave(&mut self) {
        if self.leave_armed {
            let _ = KillTimer(Some(self.hwnd), LEAVE_TIMER);
            self.leave_armed = false;
        }
    }

    /// Раскрытый остров: курсор вне занимаемой области — запускаем отсчёт сворачивания,
    /// вернулся — отменяем. Вызывается и на каждом кадре, потому что область меняется
    /// (смена вкладки, анимация), а мышь при этом может стоять на месте.
    unsafe fn check_hover(&mut self) {
        // Перетаскивание ползунка и ввод текста не должны сворачивать остров.
        if self.expanded && !self.cursor_inside() && !self.ui.holds_pointer() {
            self.arm_leave();
        } else {
            self.disarm_leave();
        }
    }

    /// Движение мыши: раскрывает остров; у раскрытого нужен кадр для hover-анимаций.
    unsafe fn on_move(&mut self) -> bool {
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

    unsafe fn on_leave_timer(&mut self) -> bool {
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
    unsafe fn probe(&self) -> Option<(bool, bool)> {
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
    unsafe fn refresh_environment(&mut self) -> bool {
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
    unsafe fn on_lock(&mut self) -> bool {
        self.disarm_leave();
        if self.expanded {
            self.set_expanded(false);
            return true;
        }
        false
    }

    /// Пробуждение или разблокировка: графика могла умереть, окружение — измениться.
    unsafe fn on_resume(&mut self) -> bool {
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
    unsafe fn lose_gfx(&mut self) {
        self.gfx = None;
        self.ui.reset_gfx();
        SetTimer(Some(self.hwnd), RECOVER_TIMER, RECOVER_MS, None);
    }

    unsafe fn try_recover(&mut self) -> bool {
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

    unsafe fn tray_menu(&mut self) -> bool {
        let Ok(menu) = CreatePopupMenu() else { return false };
        let autostart = sys::autostart_enabled();
        let item = |id: usize, flags: MENU_ITEM_FLAGS, text: &str| {
            let wide = sys::wz(text);
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

    unsafe fn add_tray(&self) {
        if let Some(icon) = self.icon {
            if !sys::tray_add(self.hwnd, icon, WM_TRAY) {
                log::error("не удалось добавить значок в трей");
            }
        }
    }

    /// (Пере)регистрирует глобальную горячую клавишу из конфига.
    unsafe fn register_hotkey(&mut self) {
        let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
        if self.cfg.hotkey.is_empty() {
            return;
        }
        let Some((mods, vk)) = config::parse_hotkey(&self.cfg.hotkey) else { return };
        if let Err(e) = RegisterHotKey(Some(self.hwnd), HOTKEY_ID, HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT, vk) {
            log::error(&format!("не удалось занять горячую клавишу «{}»: {e}", self.cfg.hotkey));
        }
    }

    /// Горячая клавиша: раскрыть остров на вкладке Apps и поставить фокус в поиск.
    unsafe fn on_hotkey(&mut self) -> bool {
        if self.hidden {
            return false;
        }
        // Вкладка Apps выключена в настройках — горячей клавише нечего открывать.
        let Some(i) = self.order.iter().position(|&t| t == tabs::APPS) else { return false };
        if i != self.tab || self.settings_open {
            self.tab_prev = Some(self.cur_id());
            self.tab_dir = if !self.settings_open && i > self.tab { 1.0 } else { -1.0 };
            self.tab_t.value = 0.0;
            self.tab_t.target = 1.0;
            self.tab = i;
            self.settings_open = false;
            self.ui.blur();
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

    unsafe fn apply_config(&mut self, cfg: Config) {
        let current = self.normal_id();
        self.order = cfg.resolve_visible(&TABS);
        self.tab = self.order.iter().position(|&i| i == current).unwrap_or(0);
        self.cfg = cfg;
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

    /// Обрабатывает событие, возвращает `true`, если нужна перерисовка.
    unsafe fn handle(&mut self, e: Ev) -> bool {
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
                }
                true
            }
            Ev::Up => {
                let was = self.ui.down;
                self.ui.down = false;
                self.ui.released = was;
                self.check_hover();
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
                tab || media || net
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
    unsafe fn on_clipboard(&mut self) -> bool {
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
    unsafe fn sync_tabs(&mut self) -> bool {
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
            }
        }
        dirty
    }

    /// Кладёт PNG из истории в буфер обмена как изображение.
    unsafe fn put_image(&self, file: &str) -> bool {
        let path = tabs::clipboard::clip_dir().join(file);
        match img::load_dib(&path) {
            Some(dib) => sys::set_clipboard_dib(self.hwnd, &dib),
            None => {
                log::error(&format!("не удалось прочитать изображение {file}"));
                false
            }
        }
    }

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

    fn tab_row_width(&self) -> f32 {
        self.order.iter().map(|&i| self.tab_widths[i]).sum::<f32>() + TAB_GAP * self.order.len().saturating_sub(1) as f32
    }

    /// Для каждой позиции вкладки: (левый край, ширина) в логических пикселях.
    fn tab_rects(&self) -> Vec<(f32, f32)> {
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
    fn click(&mut self, px: i32, py: i32) -> bool {
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
                    // Кроссфейд: старое содержимое гаснет и уезжает, новое проявляется.
                    self.tab_prev = Some(self.cur_id());
                    self.tab_dir = if !self.settings_open && i > self.tab { 1.0 } else { -1.0 };
                    self.tab_t.value = 0.0;
                    self.tab_t.target = 1.0;
                    self.tab = i;
                    self.settings_open = false;
                    self.ui.blur();
                    self.retarget();
                }
                return true;
            }
        }
        false
    }

    // ---------- Клавиатура ----------

    unsafe fn on_key(&mut self, vk: u32) -> bool {
        if self.ui.focus.is_none() {
            return false;
        }
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
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
    unsafe fn sync_focus(&mut self) {
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

    unsafe fn restore_noactivate(&mut self) {
        let _ = KillTimer(Some(self.hwnd), CARET_TIMER);
        let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32 | WS_EX_NOACTIVATE.0;
        SetWindowLongW(self.hwnd, GWL_EXSTYLE, ex as i32);
    }

    /// Окно IME (подсказки раскладки/кандидаты) рядом с курсором поля.
    unsafe fn update_ime(&self) {
        if let Some((x, y)) = self.ui.ime_pos {
            sys::ime_set_position(self.hwnd, (x * self.scale) as i32, (y * self.scale) as i32);
        }
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

    /// Тулкит и состояние вкладок на время кадра выносятся из `self`, чтобы рисовать с `&self`.
    unsafe fn render(&mut self) -> Result<()> {
        let mut ui = std::mem::take(&mut self.ui);
        let mut tabs = std::mem::take(&mut self.tabs);
        let r = self.render_frame(&mut ui, &mut tabs);
        self.ui = ui;
        self.tabs = tabs;
        r
    }

    unsafe fn render_frame(&self, ui: &mut Ui, tabs: &mut tabs::Tabs) -> Result<()> {
        let Some(g) = &self.gfx else { return Ok(()) };
        let t0 = Instant::now();
        let s = self.scale;
        let aw = self.w.value.max(1.0);
        let ah = self.h.value.max(1.0);
        let x0 = (self.win_w as f32 - aw) / 2.0;
        let open = self.open.value.clamp(0.0, 1.0);
        let accent = self.accent();
        let collapsed_h = self.collapsed().1;

        let img: Option<ID2D1Image> = g.target.as_ref().and_then(|b| b.cast().ok());
        g.dc.SetTarget(img.as_ref());
        g.dc.BeginDraw();
        g.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

        // «Ушки» уменьшаются вместе с высотой, чтобы в полоске их не было.
        let ear = (EAR * s * (ah / (collapsed_h * s)).min(1.0)).min(ah);
        let ear = if ear < 0.5 { 0.0 } else { ear };
        // Скругление только снизу: из настроек, в свёрнутом виде чуть меньше (0,72 → 1 при раскрытии).
        let r = (self.cfg.corner_radius * (0.72 + 0.28 * open) * s).min(aw / 2.0).min(ah - ear).max(0.0);
        // По мере сжатия в полоску цвет уходит от фона к чистому чёрному.
        let thin = ((collapsed_h * s - ah) / ((collapsed_h - STRIP_H) * s)).clamp(0.0, 1.0);
        let k = 1.0 - thin;
        let pal = theme::get();
        g.brush.SetColor(&D2D1_COLOR_F { r: pal.bg.r * k, g: pal.bg.g * k, b: pal.bg.b * k, a: 1.0 });
        let body = self.body_path(g, x0, aw, ah, ear, r, false)?;
        g.dc.FillGeometry(&body, &g.brush, None);

        // Пока тянут ползунок радиуса, остров «засвечивается»: заливка подкрашивается акцентом, а по бокам
        // и снизу идёт светящийся контур — по нему хорошо видно, как меняется скругление.
        let glow = self.glow.value.clamp(0.0, 1.0);
        if glow > 0.01 {
            let mut c = accent;
            c.a = 0.12 * glow;
            g.brush.SetColor(&c);
            g.dc.FillGeometry(&body, &g.brush, None);
            let outline = self.body_path(g, x0, aw, ah, ear, r, true)?;
            for (width, alpha) in [(9.0, 0.07), (6.0, 0.12), (3.5, 0.24), (1.6, 0.95)] {
                c.a = alpha * glow;
                g.brush.SetColor(&c);
                g.dc.DrawGeometry(&outline, &g.brush, width * s, None);
            }
        }

        let mut cp = POINT::default();
        let _ = GetCursorPos(&mut cp);
        let cursor = ((cp.x - self.win_x) as f32 / s, (cp.y - self.win_y) as f32 / s);
        ui.begin_frame(cursor, self.expanded && self.cursor_inside());

        // Индикатор свёрнутой формы: мини-плеер и уведомление проявляются и гаснут пружинами,
        // раскрытие панели их гасит (множитель 1 − open). Раскладка — по целевому размеру формы,
        // чтобы содержимое не «сплющивалось» вместе с высотой острова.
        let live_open = 1.0 - open;
        let centre = self.win_w as f32 / s / 2.0;
        // Что рисуем в свёрнутом виде: мини-плеер, уведомление или «лицо» (раскладка, время, Wi-Fi).
        #[derive(PartialEq)]
        enum Slot {
            Mini,
            Toast,
            Face,
        }
        for (alpha, slot) in [
            (self.live.mini_alpha(), Slot::Mini),
            (self.live.toast_alpha(), Slot::Toast),
            (self.face.alpha(), Slot::Face),
        ] {
            let alpha = alpha * live_open;
            if alpha < 0.01 {
                continue;
            }
            let (fw, fh) = match slot {
                Slot::Mini => live::MINI,
                Slot::Toast => live::TOAST,
                Slot::Face => self.collapsed(),
            };
            let fw = fw.max(self.collapsed().0);
            g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
            g.dc.PushAxisAlignedClip(
                &D2D_RECT_F { left: x0, top: 0.0, right: x0 + aw, bottom: ah },
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            g.dc.SetTransform(&ui::matrix(s, 0.0, 0.0));
            let cv = Canvas { g, alpha, accent };
            let r = ui::Rect::new(centre - fw / 2.0, 0.0, fw, fh);
            match slot {
                Slot::Mini => self.live.draw_mini(ui, &cv, r),
                Slot::Toast => self.live.draw_toast(&cv, r),
                Slot::Face => self.face.draw(&cv, r),
            }
            g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
            g.dc.PopAxisAlignedClip();
        }

        if open > 0.02 {
            // Точка слева в ряду вкладок: режим полоски включён.
            if self.strip_mode {
                g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
                let mut c = accent;
                c.a = open;
                g.brush.SetColor(&c);
                g.dc.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: Vector2 { X: x0 + 14.0 * s, Y: (TAB_TOP + TAB_H / 2.0) * s },
                        radiusX: 2.5 * s,
                        radiusY: 2.5 * s,
                    },
                    &g.brush,
                );
            }
            let rects = self.tab_rects();
            for (i, (tx, tw)) in rects.iter().enumerate() {
                let (px, pw) = (tx * s, tw * s);
                let (py, ph) = (TAB_TOP * s, TAB_H * s);
                let active = !self.settings_open && i == self.tab;
                let hot = cursor.0 >= *tx && cursor.0 <= tx + tw && cursor.1 >= TAB_TOP && cursor.1 <= TAB_TOP + TAB_H;
                let hv = ui.hover_amount(0xF000 + i as u32, hot && self.expanded);
                if !active && hv > 0.01 {
                    let mut c = ui::fg();
                    c.a = 0.08 * hv * open;
                    g.brush.SetColor(&c);
                    g.dc.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F { left: px, top: py, right: px + pw, bottom: py + ph },
                            radiusX: ph / 2.0,
                            radiusY: ph / 2.0,
                        },
                        &g.brush,
                    );
                }
                if active {
                    let mut c = accent;
                    c.a = 0.26 * open;
                    g.brush.SetColor(&c);
                    g.dc.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F { left: px, top: py, right: px + pw, bottom: py + ph },
                            radiusX: ph / 2.0,
                            radiusY: ph / 2.0,
                        },
                        &g.brush,
                    );
                }
                let mut c = if active { accent } else { ui::dim() };
                c.a *= open;
                g.draw_text(TABS[self.order[i]], px, py, pw, ph, c);
            }

            // Шестерёнка настроек: при наведении поворачивается, при открытых настройках залита акцентом.
            let (gx, gy) = self.gear_center();
            let gear_hot = self.expanded && (cursor.0 - gx).hypot(cursor.1 - gy) <= GEAR_R + 3.0;
            let gv = ui.hover_amount(0xF100, gear_hot);
            let gcv = Canvas { g, alpha: open, accent };
            if self.settings_open {
                g.dc.SetTransform(&ui::matrix(s, 0.0, 0.0));
                gcv.circle(gx, gy, GEAR_R, D2D1_COLOR_F { a: 0.26, ..accent });
            }
            g.dc.SetTransform(&ui::matrix_rot(s, gx, gy, 60.0 * gv));
            let gcol = if self.settings_open { accent } else { ui::mix(ui::dim(), ui::txt(), gv) };
            gcv.glyph(ui::icon::SETTINGS, ui::Rect::new(gx - GEAR_R, gy - GEAR_R, 2.0 * GEAR_R, 2.0 * GEAR_R), false, gcol);

            // Содержимое вкладок: клип по текущему телу острова, кроссфейд старой и новой.
            g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
            // При большом радиусе нижние углы карточек вылезли бы за скругление острова: обрезаем по его форме.
            let mask = if r > 16.0 * s {
                let params = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: D2D_RECT_F { left: f32::MIN, top: f32::MIN, right: f32::MAX, bottom: f32::MAX },
                    geometricMask: std::mem::ManuallyDrop::new(Some(body.cast::<ID2D1Geometry>()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: ui::matrix(1.0, 0.0, 0.0),
                    opacity: 1.0,
                    opacityBrush: std::mem::ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                };
                g.dc.PushLayer(&params, None);
                Some(params)
            } else {
                None
            };
            g.dc.PushAxisAlignedClip(
                &D2D_RECT_F {
                    left: x0 + 6.0 * s,
                    top: 31.0 * s,
                    right: x0 + aw - 6.0 * s,
                    bottom: (ah - 4.0 * s).max(31.0 * s),
                },
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            let fade = self.tab_t.value.clamp(0.0, 1.0);
            let centre = self.win_w as f32 / s / 2.0;
            let layer = |id: usize, alpha: f32, dx: f32, interactive: bool, ui: &mut Ui, tabs: &mut tabs::Tabs| {
                if alpha < 0.01 {
                    return;
                }
                let (w, h) = (self.tab_w(id), TAB_SIZES[id].1);
                let ox = centre - w / 2.0 + dx;
                g.dc.SetTransform(&ui::matrix(s, ox * s, 0.0));
                ui.begin_content(ox, interactive);
                let cv = Canvas { g, alpha, accent };
                tabs.draw(id, ui, &cv, w, h);
            };
            if let Some(prev) = self.tab_prev {
                layer(prev, open * (1.0 - fade), -self.tab_dir * SLIDE * fade, false, ui, tabs);
            }
            layer(self.cur_id(), open * fade, self.tab_dir * SLIDE * (1.0 - fade), true, ui, tabs);
            g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
            g.dc.PopAxisAlignedClip();
            if let Some(params) = mask {
                g.dc.PopLayer();
                drop(std::mem::ManuallyDrop::into_inner(params.geometricMask));
            }
        }

        g.dc.EndDraw(None, None)?;
        perf::draw_done(t0);
        g.swapchain.Present(1, DXGI_PRESENT(0)).ok()?;
        ui.end_frame();
        Ok(())
    }

    /// Контур чёлки: плоский верх на уровне края экрана, вогнутые «ушки» по бокам,
    /// скруглённые нижние углы.
    /// `outline` — открытый контур без верхней грани (для свечения): сверху остров упирается в край экрана.
    unsafe fn body_path(&self, g: &Gfx, x0: f32, aw: f32, ah: f32, ear: f32, r: f32, outline: bool) -> Result<ID2D1PathGeometry1> {
        let pt = |x: f32, y: f32| Vector2 { X: x, Y: y };
        let arc = |x: f32, y: f32, radius: f32, dir: D2D1_SWEEP_DIRECTION| D2D1_ARC_SEGMENT {
            point: pt(x, y),
            size: D2D_SIZE_F { width: radius, height: radius },
            rotationAngle: 0.0,
            sweepDirection: dir,
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        let geo = g.d2d.CreatePathGeometry()?;
        let sink = geo.Open()?;
        sink.BeginFigure(pt(x0 - ear, 0.0), if outline { D2D1_FIGURE_BEGIN_HOLLOW } else { D2D1_FIGURE_BEGIN_FILLED });
        if ear > 0.0 {
            sink.AddArc(&arc(x0, ear, ear, D2D1_SWEEP_DIRECTION_CLOCKWISE));
        }
        sink.AddLine(pt(x0, ah - r));
        if r > 0.0 {
            sink.AddArc(&arc(x0 + r, ah, r, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
        }
        sink.AddLine(pt(x0 + aw - r, ah));
        if r > 0.0 {
            sink.AddArc(&arc(x0 + aw, ah - r, r, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
        }
        sink.AddLine(pt(x0 + aw, ear));
        if ear > 0.0 {
            sink.AddArc(&arc(x0 + aw + ear, 0.0, ear, D2D1_SWEEP_DIRECTION_CLOCKWISE));
        }
        sink.EndFigure(if outline { D2D1_FIGURE_END_OPEN } else { D2D1_FIGURE_END_CLOSED });
        sink.Close()?;
        Ok(geo)
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
    let cfg = config::load();
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
