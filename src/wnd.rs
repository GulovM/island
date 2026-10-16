use super::*;

// ---------- События от оконной процедуры к главному циклу ----------

pub(crate) enum Ev {
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
    pub(crate) static EVENTS: RefCell<Vec<Ev>> = const { RefCell::new(Vec::new()) };
    pub(crate) static MAIN_WND: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    pub(crate) static TASKBAR_MSG: Cell<u32> = const { Cell::new(0) };
}

pub(crate) fn push(e: Ev) {
    EVENTS.with(|v| v.borrow_mut().push(e));
}

pub(crate) unsafe extern "system" fn on_win_event(
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

pub(crate) unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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
pub(crate) unsafe extern "system" fn hot_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            push(Ev::Hot(hwnd.0 as isize));
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub(crate) unsafe extern "system" fn enum_monitor(m: HMONITOR, _dc: HDC, rc: *mut RECT, lp: LPARAM) -> BOOL {
    let list = &mut *(lp.0 as *mut Vec<(HMONITOR, RECT)>);
    list.push((m, *rc));
    TRUE
}

/// Состояние перебора окон в `probe`.
pub(crate) struct ScanCtx {
    pub(crate) own: HWND,
    pub(crate) mon: RECT,
    pub(crate) covered: bool,
}

pub(crate) unsafe extern "system" fn scan_window(hwnd: HWND, lp: LPARAM) -> BOOL {
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

pub(crate) unsafe fn monitor_scale(mon: HMONITOR) -> f32 {
    let (mut x, mut y) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    x as f32 / 96.0 * UI_ZOOM
}

/// Ширина подписи каждой вкладки (логические пиксели, вместе с внутренними полями).
pub(crate) unsafe fn measure_tabs(dwrite: &IDWriteFactory, scale: f32) -> Result<[f32; TAB_COUNT]> {
    let text = gfx::make_text(dwrite, scale)?;
    let mut widths = [0.0f32; TAB_COUNT];
    for (i, name) in TABS.iter().enumerate() {
        let wide: Vec<u16> = name.encode_utf16().collect();
        let layout = dwrite.CreateTextLayout(&wide, &text, 1000.0, 100.0)?;
        let mut m = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut m)?;
        widths[i] = m.widthIncludingTrailingWhitespace / scale + TAB_PAD * 2.0;
    }
    Ok(widths)
}
