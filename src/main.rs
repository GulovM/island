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
mod we;
mod update;
mod app;

pub use app::{App, HotZone};

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

pub(crate) const TAB_COUNT: usize = 7;
pub(crate) const TABS: [&str; TAB_COUNT] = ["Controls", "Today", "Stats", "Clipboard", "Apps", "Wallpaper", "Power"];
/// Размер панели для каждой вкладки (ширина, высота) — как на скриншотах; последний — страница настроек.
pub(crate) const TAB_SIZES: [(f32, f32); 8] = [
    (430.0, 282.0),
    (660.0, 304.0),
    (640.0, 282.0),
    (590.0, 300.0),
    (480.0, 420.0),
    (700.0, 236.0),
    (420.0, 112.0),
    (620.0, 428.0),
];
/// Высота «тонкой полоски», в которую остров сворачивается поверх развёрнутого на весь экран окна.
pub(crate) const STRIP_H: f32 = 3.0;
pub(crate) const TAB_H: f32 = 17.0;
pub(crate) const TAB_GAP: f32 = 4.0;
pub(crate) const TAB_PAD: f32 = 8.0;
pub(crate) const TAB_TOP: f32 = 9.0;
/// Отступ ряда вкладок от краёв панели.
pub(crate) const TAB_SIDE: f32 = 28.0;
/// Размер окна: самая большая панель (650×400) + запас под перелёт пружины и «ушки».
pub(crate) const WIN_LOGICAL: (f32, f32) = (760.0, 500.0);
/// Допуск вокруг раскрытой формы, где мышь ещё считается «на острове» (только слева/справа/снизу).
pub(crate) const HIT_SLOP: f32 = 4.0;
/// Радиус шестерёнки настроек в ряду вкладок.
pub(crate) const GEAR_R: f32 = 9.0;
/// Радиус «ушек» — загнутых плавных переходов чёлки в верхний край экрана.
pub(crate) const EAR: f32 = 8.0;
/// Высота горячей зоны на верхнем краю других мониторов, физические пиксели.
pub(crate) const HOT_H: i32 = 2;
/// Через сколько мс пробовать пересоздать графику после потери устройства.
pub(crate) const RECOVER_MS: u32 = 500;

pub(crate) const WM_MOUSELEAVE: u32 = 0x02A3;
/// Цвет оформления окон (акцент) поменялся — его подстраивает, например, Wallpaper Engine под обои.
pub(crate) const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
pub(crate) const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
pub(crate) const WTS_SESSION_LOCK: usize = 7;
pub(crate) const WTS_SESSION_UNLOCK: usize = 8;
/// Будит главный цикл из хука окон (колбэк хука сам сообщение не возвращает из `GetMessage`).
pub(crate) const WM_RECHECK: u32 = WM_APP + 1;
/// Сообщение от значка в трее.
pub(crate) const WM_TRAY: u32 = WM_APP + 2;
pub(crate) const LEAVE_TIMER: usize = 1;
pub(crate) const ENV_TIMER: usize = 2;
pub(crate) const RECOVER_TIMER: usize = 3;
/// Мигание курсора в текстовом поле: работает только пока поле в фокусе.
pub(crate) const CARET_TIMER: usize = 4;
pub(crate) const CARET_MS: u32 = 530;
/// Секундный опрос видимой вкладки (Stats, Controls, Today).
pub(crate) const POLL_TIMER: usize = 5;
pub(crate) const POLL_MS: u32 = 1000;
/// Срабатывание таймера на вкладке Today, в том числе при свёрнутом острове.
pub(crate) const ALARM_TIMER: usize = 6;
/// Пауза между сворачиванием и эмуляцией Ctrl+V: окну нужно вернуть фокус.
pub(crate) const PASTE_TIMER: usize = 7;
pub(crate) const PASTE_DELAY_MS: u32 = 160;
/// Срок жизни уведомления в свёрнутой форме.
pub(crate) const TOAST_TIMER: usize = 8;
/// Кадры мини-плеера (анимация эквалайзера и прогресс): ~30 в секунду, только пока он виден и
/// играет музыка. Таймер, а не vsync, — вдвое меньше кадров при той же плавности для 3 столбиков.
pub(crate) const LIVE_TIMER: usize = 9;
pub(crate) const LIVE_TICK_MS: u32 = 33;
/// Смена минуты для часов на свёрнутом острове: срабатывает раз в минуту и только пока они видны.
pub(crate) const FACE_TIMER: usize = 10;
/// Файл в `%APPDATA%\island` с выбором режима полоски: «1» — включён.
pub(crate) const STRIP_FILE: &str = "strip.txt";
/// Раз в час проверяет, не прошли ли сутки с последней проверки обновлений.
pub(crate) const UPDATE_TIMER: usize = 11;
pub(crate) const UPDATE_TICK_MS: u32 = 60 * 60 * 1000;
pub(crate) const WM_CLIPBOARDUPDATE: u32 = 0x031D;
pub(crate) const WM_HOTKEY: u32 = 0x0312;
pub(crate) const HOTKEY_ID: i32 = 1;
/// Сдвиг содержимого при смене вкладки (логические пиксели).
pub(crate) const SLIDE: f32 = 16.0;

pub(crate) const CMD_SETTINGS: usize = 1;
pub(crate) const CMD_RELOAD: usize = 2;
pub(crate) const CMD_AUTOSTART: usize = 3;
pub(crate) const CMD_EXIT: usize = 4;



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
        SetTimer(Some(app.hwnd), UPDATE_TIMER, UPDATE_TICK_MS, None);
        update::tick(app.cfg.auto_update);

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
