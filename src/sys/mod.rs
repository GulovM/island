//! Системная интеграция: единственный экземпляр, автозапуск, трей, обрезка working set.

use std::ffi::c_void;
use std::path::PathBuf;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HINSTANCE, HWND};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, HICON};

mod night_light;
mod paths;
mod registry;
mod seal;

pub use night_light::{night_light, set_night_light};
pub use paths::{pictures_dir, system_drive_root, windows_dir};
pub use registry::reg_str;
pub(in crate::sys) use registry::{read_binary, read_dword};
pub use seal::{is_sealed, seal, unseal};

/// `%APPDATA%\island` — конфиг, лог, позже кэши.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("island")
}

/// Строка в UTF-16 с завершающим нулём.
pub fn wz(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 без аллокации для коротких строк: отрисовка зовёт это сотни раз за кадр.
/// Строка длиннее буфера уходит в `Vec`.
pub fn with_utf16<R>(s: &str, f: impl FnOnce(&[u16]) -> R) -> R {
    let mut buf = [0u16; 160];
    let mut n = 0;
    for u in s.encode_utf16() {
        if n == buf.len() {
            let v: Vec<u16> = s.encode_utf16().collect();
            return f(&v);
        }
        buf[n] = u;
        n += 1;
    }
    f(&buf[..n])
}

/// `false` — такой экземпляр уже запущен.
pub fn acquire_single_instance() -> bool {
    unsafe {
        match CreateMutexW(None, true, w!("Local\\Island.SingleInstance")) {
            // Хэндл намеренно не закрываем: HANDLE — просто число, мьютекс живёт до конца процесса.
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            // Не смогли проверить — лучше запуститься, чем молча не запуститься.
            Err(_) => true,
        }
    }
}

// ---------- Автозапуск (HKCU\...\Run) ----------

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Island");

pub fn autostart_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

pub fn set_autostart(on: bool) -> bool {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return false };
            let exe = exe.display().to_string();
            let cmd = wz(&format!("\"{}\"", exe.trim_start_matches(r"\\?\")));
            let bytes = std::slice::from_raw_parts(cmd.as_ptr() as *const u8, cmd.len() * 2);
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(bytes.as_ptr() as *const c_void),
                bytes.len() as u32,
            ) == ERROR_SUCCESS
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND
        }
    }
}

// ---------- Трей ----------

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

pub fn tray_add(hwnd: HWND, icon: HICON, callback_msg: u32) -> bool {
    let mut nid = tray_data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = callback_msg;
    nid.hIcon = icon;
    for (d, s) in nid.szTip.iter_mut().zip("Island".encode_utf16()) {
        *d = s;
    }
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid).as_bool() }
}

pub fn tray_remove(hwnd: HWND) {
    let nid = tray_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Иконка трея, нарисованная кодом: «чёлка» акцентного цвета. Файл иконки появится на этапе 6.
pub fn make_icon(hinst: HINSTANCE, accent: [f32; 3]) -> Option<HICON> {
    const N: usize = 32;
    let (x0, x1, y0, y1, r) = (3.0f32, 29.0f32, 5.0f32, 21.0f32, 7.0f32);
    let mut xor = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if fx < x0 || fx > x1 || fy < y0 || fy > y1 {
                continue;
            }
            // Скруглены только нижние углы.
            let cx = fx.clamp(x0 + r, x1 - r);
            let cy = fy.min(y1 - r);
            if (fx - cx).powi(2) + (fy - cy).powi(2) > r * r {
                continue;
            }
            let i = (y * N + x) * 4;
            xor[i] = (accent[2] * 255.0) as u8;
            xor[i + 1] = (accent[1] * 255.0) as u8;
            xor[i + 2] = (accent[0] * 255.0) as u8;
            xor[i + 3] = 255;
        }
    }
    let and = [0u8; N * N / 8];
    unsafe { CreateIcon(Some(hinst), N as i32, N as i32, 1, 32, and.as_ptr(), xor.as_ptr()).ok() }
}

// ---------- Память ----------

/// Сбрасывает рабочий набор: страницы, которые не нужны сейчас, уходят из RAM и вернутся по требованию.
pub fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

// ---------- Буфер обмена (для текстового поля) ----------

const CF_UNICODETEXT: u32 = 13;

/// Открывает буфер обмена; менеджеры буфера держат его доли секунды, поэтому несколько коротких повторов.
fn open_clipboard(owner: HWND) -> Option<()> {
    use windows::Win32::System::DataExchange::OpenClipboard;
    for attempt in 0..5 {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            return Some(());
        }
        if attempt < 4 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    None
}

/// Текст из буфера обмена, если там есть Unicode-текст.
pub fn clipboard_text(owner: HWND) -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    unsafe {
        open_clipboard(owner)?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT) {
            let hg = HGLOBAL(h.0);
            let p = GlobalLock(hg) as *const u16;
            if !p.is_null() {
                // Читаем не дальше размера блока: строка без завершающего нуля не должна выводить за его границу.
                let max = (GlobalSize(hg) / 2).min(1 << 20);
                let mut n = 0;
                while n < max && *p.add(n) != 0 {
                    n += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                let _ = GlobalUnlock(hg);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

/// Кладёт Unicode-текст в буфер обмена.
pub fn set_clipboard_text(owner: HWND, text: &str) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Foundation::GlobalFree;
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    unsafe {
        let wide = wz(text);
        let bytes = wide.len() * 2;
        let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, bytes) else { return false };
        let p = GlobalLock(hg) as *mut u16;
        if p.is_null() {
            let _ = GlobalFree(Some(hg));
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let _ = GlobalUnlock(hg);
        if open_clipboard(owner).is_none() {
            let _ = GlobalFree(Some(hg));
            return false;
        }
        let _ = EmptyClipboard();
        // После успешного SetClipboardData память принадлежит системе; при отказе освобождаем сами.
        let ok = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hg.0))).is_ok();
        if !ok {
            let _ = GlobalFree(Some(hg));
        }
        let _ = CloseClipboard();
        ok
    }
}

// ---------- IME ----------

/// Ставит окно композиции IME рядом с курсором поля (клиентские физические пиксели).
pub fn ime_set_position(hwnd: HWND, x: i32, y: i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::Ime::{
        ImmGetContext, ImmReleaseContext, ImmSetCompositionWindow, CFS_POINT, COMPOSITIONFORM,
    };
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return;
        }
        let form = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT { x, y },
            rcArea: Default::default(),
        };
        let _ = ImmSetCompositionWindow(himc, &form);
        let _ = ImmReleaseContext(hwnd, himc);
    }
}

// ---------- Тема, обои, ввод, запуск ----------

const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");


/// Запущен ли процесс с одним из имён `names` (без учёта регистра).
pub fn process_running(names: &[&str]) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let exe = String::from_utf16_lossy(&e.szExeFile[..end]);
            if names.iter().any(|n| exe.eq_ignore_ascii_case(n)) {
                found = true;
                break;
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

/// Число мониторов рабочего стола.
pub fn monitor_count() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};
    unsafe { GetSystemMetrics(SM_CMONITORS).max(1) as u32 }
}

/// Тёмная тема приложений включена.
pub fn dark_mode() -> bool {
    read_dword(PERSONALIZE, w!("AppsUseLightTheme")) == Some(0)
}

/// В Windows включена светлая тема приложений (если значения в реестре нет — считаем тёмной).
pub fn light_mode() -> bool {
    read_dword(PERSONALIZE, w!("AppsUseLightTheme")) == Some(1)
}

/// Переключает тему приложений и системы и сообщает всем окнам (`WM_SETTINGCHANGE`).
pub fn set_dark_mode(dark: bool) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    unsafe {
        let v: u32 = if dark { 0 } else { 1 };
        for name in [w!("AppsUseLightTheme"), w!("SystemUsesLightTheme")] {
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PERSONALIZE,
                name,
                windows::Win32::System::Registry::REG_DWORD.0,
                Some(&v as *const u32 as *const c_void),
                4,
            );
        }
        let area = wz("ImmersiveColorSet");
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(area.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            300,
            None,
        );
    }
}

/// В буфере обмена данные, помеченные «не записывать» (менеджеры паролей).
pub fn clipboard_excluded() -> bool {
    use windows::Win32::System::DataExchange::{IsClipboardFormatAvailable, RegisterClipboardFormatW};
    unsafe {
        [w!("ExcludeClipboardContentFromMonitorProcessing"), w!("CanIncludeInClipboardHistory")]
            .iter()
            .enumerate()
            .any(|(i, name)| {
                let f = RegisterClipboardFormatW(*name);
                // Первый формат — «исключить», если он есть; второй — «можно», если его нет, не решает.
                i == 0 && f != 0 && IsClipboardFormatAvailable(f).is_ok()
            })
    }
}

/// Владелец буфера обмена — наше окно (мы сами записали текст).
pub fn clipboard_owned_by(hwnd: HWND) -> bool {
    use windows::Win32::System::DataExchange::GetClipboardOwner;
    unsafe { GetClipboardOwner().map(|h| h == hwnd).unwrap_or(false) }
}

/// Эмулирует Ctrl+V в окне, которое сейчас в фокусе.
pub fn send_paste() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        VK_CONTROL,
    };
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(VK_CONTROL, false), key(VIRTUAL_KEY(0x56), false), key(VIRTUAL_KEY(0x56), true), key(VK_CONTROL, true)];
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

/// Ставит обои рабочего стола.
pub fn set_wallpaper(path: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
    };
    unsafe {
        let wide = wz(path);
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_ptr() as *mut c_void),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
        .is_ok()
    }
}

/// Путь текущих обоев.
pub fn current_wallpaper() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    unsafe {
        let mut buf = [0u16; 520];
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
    }
}

/// Открывает файл, ярлык или ссылку программой по умолчанию.
/// Открывает файл, ссылку или ярлык через Shell. `false` — Shell отказался (ShellExecute
/// возвращает значение ≤ 32): например, цель ярлыка удалена.
pub fn open(target: &str) -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        let t = wz(target);
        let h = ShellExecuteW(None, w!("open"), PCWSTR(t.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        h.0 as isize > 32
    }
}

// ---------- Цвета системы и раскладка клавиатуры ----------


/// Акцентные цвета Windows. Их же подстраивает под обои Wallpaper Engine (и сама Windows при
/// «автоматическом выборе цвета фона»): читаем палитру из реестра — это мгновенно и без WinRT.
pub fn system_colors() -> Option<crate::theme::SystemColors> {
    use crate::theme::{parse_accent_palette, SystemColors};
    if let Some(p) = read_binary(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"), w!("AccentPalette"))
        .and_then(|b| parse_accent_palette(&b))
    {
        return Some(p);
    }
    // Запасной путь: цвет акцента окон (ABGR).
    let v = read_dword(w!("Software\\Microsoft\\Windows\\DWM"), w!("AccentColor"))?;
    let ch = |shift: u32| ((v >> shift) & 0xFF) as f32 / 255.0;
    Some(SystemColors::from_accent([ch(0), ch(8), ch(16)]))
}


/// Код языка (младшие 16 бит HKL) раскладки окна в фокусе: у каждого окна своя раскладка.
pub fn foreground_lang() -> u32 {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let fg = GetForegroundWindow();
        let tid = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        (GetKeyboardLayout(tid).0 as usize & 0xFFFF) as u32
    }
}

/// Двухбуквенный код языка раскладки: «RU», «EN».
pub fn lang_code(lang: u32) -> String {
    use windows::Win32::Globalization::{GetLocaleInfoW, LOCALE_SISO639LANGNAME};
    let mut buf = [0u16; 16];
    let n = unsafe { GetLocaleInfoW(lang, LOCALE_SISO639LANGNAME, Some(&mut buf)) };
    if n <= 1 {
        return format!("{lang:X}");
    }
    String::from_utf16_lossy(&buf[..(n as usize - 1).min(buf.len())]).to_uppercase()
}

/// Секунды с 1970 года.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// В буфере обмена есть Unicode-текст.
pub fn clipboard_has_text() -> bool {
    use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() }
}

// ---------- Изображения в буфере обмена ----------

const CF_DIB: u32 = 8;

/// В буфере обмена есть растровое изображение (DIB; из DIBV5 и bitmap Windows синтезирует его сама).
pub fn clipboard_has_image() -> bool {
    use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
    unsafe { IsClipboardFormatAvailable(CF_DIB).is_ok() }
}

/// Изображение из буфера обмена: (ширина, высота, BGRA сверху вниз, непрозрачное).
pub fn clipboard_image(owner: HWND) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    unsafe {
        OpenClipboard(Some(owner)).ok()?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_DIB) {
            let hg = HGLOBAL(h.0);
            let size = GlobalSize(hg);
            let p = GlobalLock(hg) as *const u8;
            if !p.is_null() {
                out = parse_dib(std::slice::from_raw_parts(p, size));
                let _ = GlobalUnlock(hg);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

/// Разбор DIB (24/32 бита, BI_RGB или BI_BITFIELDS) в BGRA сверху вниз.
fn parse_dib(d: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if d.len() < 40 {
        return None;
    }
    let u32le = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    let header = u32le(0) as usize;
    let (w, hraw) = (u32le(4) as i32, u32le(8) as i32);
    let bits = u16::from_le_bytes([d[14], d[15]]) as usize;
    let comp = u32le(16);
    if w <= 0 || hraw == 0 || (bits != 32 && bits != 24) || (comp != 0 && comp != 3) || header < 40 {
        return None;
    }
    let (w, h) = (w as usize, hraw.unsigned_abs() as usize);
    if w * h > 50_000_000 {
        return None;
    }
    let top_down = hraw < 0;
    let mut off = header;
    if header == 40 && comp == 3 {
        off += 12;
    }
    let stride = (w * bits).div_ceil(32) * 4;
    if d.len() < off + stride * h {
        return None;
    }
    let step = bits / 8;
    let mut out = vec![255u8; w * h * 4];
    for row in 0..h {
        let src = off + stride * if top_down { row } else { h - 1 - row };
        for x in 0..w {
            let s = src + x * step;
            let o = (row * w + x) * 4;
            out[o..o + 3].copy_from_slice(&d[s..s + 3]);
        }
    }
    Some((w as u32, h as u32, out))
}

/// Кладёт изображение (готовый DIB: заголовок + пиксели снизу вверх) в буфер обмена.
pub fn set_clipboard_dib(owner: HWND, dib: &[u8]) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    unsafe {
        let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, dib.len()) else { return false };
        let p = GlobalLock(hg) as *mut u8;
        if p.is_null() {
            return false;
        }
        std::ptr::copy_nonoverlapping(dib.as_ptr(), p, dib.len());
        let _ = GlobalUnlock(hg);
        if OpenClipboard(Some(owner)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = SetClipboardData(CF_DIB, Some(HANDLE(hg.0))).is_ok();
        let _ = CloseClipboard();
        ok
    }
}

#[cfg(test)]
mod utf16_tests {
    use super::with_utf16;

    #[test]
    fn matches_encode_utf16() {
        for s in ["", "abc", "Привет, мир", "😀 emoji", &"я".repeat(160), &"я".repeat(161), &"😀".repeat(200)] {
            let want: Vec<u16> = s.encode_utf16().collect();
            assert_eq!(with_utf16(s, |w| w.to_vec()), want, "{s:?}");
        }
    }
}


#[cfg(test)]
mod dib_tests {
    use super::parse_dib;

    #[test]
    fn dib_24bit_bottom_up() {
        // 2×1, 24 бита, строка 8 байт (6 + 2 выравнивание).
        let mut d = vec![0u8; 40];
        d[0] = 40;
        d[4] = 2;
        d[8] = 1;
        d[12] = 1;
        d[14] = 24;
        d.extend_from_slice(&[1, 2, 3, 4, 5, 6, 0, 0]);
        let (w, h, px) = parse_dib(&d).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(&px[..8], &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn dib_rejects_palette() {
        let mut d = vec![0u8; 48];
        d[0] = 40;
        d[4] = 1;
        d[8] = 1;
        d[14] = 8;
        assert!(parse_dib(&d).is_none());
    }
}


/// Системный диалог выбора папки. Блокирует поток до закрытия диалога — вызывать только из
/// отдельного потока. `initial` — папка, с которой диалог открывается. `None` — отмена или сбой.
pub fn pick_folder(initial: Option<&str>) -> Option<String> {
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };
    unsafe {
        // Парный CoUninitialize нужен и при S_FALSE; при RPC_E_CHANGED_MODE (is_err) — нет.
        let inited = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let path = (|| {
            let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let opts = dlg.GetOptions().ok()?;
            dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
            if let Some(dir) = initial {
                let wide = wz(&dir.replace('/', "\\"));
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None) {
                    let _ = dlg.SetFolder(&item);
                }
            }
            // Отмена тоже возвращает ошибку (ERROR_CANCELLED) — это не сбой.
            dlg.Show(None).ok()?;
            let item = dlg.GetResult().ok()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let s = name.to_string().ok();
            CoTaskMemFree(Some(name.0 as *const _));
            s
        })();
        if inited {
            CoUninitialize();
        }
        path
    }
}
