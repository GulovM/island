//! Оболочка Windows: автозапуск, трей, запуск программ, диалог выбора папки, IME, процессы, мониторы, раскладка.

use std::ffi::c_void;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HINSTANCE, HWND, POINT};
use windows::Win32::Globalization::{GetLocaleInfoW, LOCALE_SISO639LANGNAME};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmReleaseContext, ImmSetCompositionWindow, CFS_POINT, COMPOSITIONFORM,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, ShellExecuteW, Shell_NotifyIconW,
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIcon, GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, HICON, SM_CMONITORS, SW_SHOWNORMAL,
};

use super::wz;

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

// ---------- IME ----------

/// Ставит окно композиции IME рядом с курсором поля (клиентские физические пиксели).
pub fn ime_set_position(hwnd: HWND, x: i32, y: i32) {
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

/// Запущен ли процесс с одним из имён `names` (без учёта регистра).
pub fn process_running(names: &[&str]) -> bool {
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
    unsafe { GetSystemMetrics(SM_CMONITORS).max(1) as u32 }
}

/// Открывает файл, ярлык или ссылку программой по умолчанию.
/// Открывает файл, ссылку или ярлык через Shell. `false` — Shell отказался (ShellExecute
/// возвращает значение ≤ 32): например, цель ярлыка удалена.
pub fn open(target: &str) -> bool {
    unsafe {
        let t = wz(target);
        let h = ShellExecuteW(None, w!("open"), PCWSTR(t.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        h.0 as isize > 32
    }
}

/// Код языка (младшие 16 бит HKL) раскладки окна в фокусе: у каждого окна своя раскладка.
pub fn foreground_lang() -> u32 {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        (GetKeyboardLayout(tid).0 as usize & 0xFFFF) as u32
    }
}

/// Двухбуквенный код языка раскладки: «RU», «EN».
pub fn lang_code(lang: u32) -> String {
    let mut buf = [0u16; 16];
    let n = unsafe { GetLocaleInfoW(lang, LOCALE_SISO639LANGNAME, Some(&mut buf)) };
    if n <= 1 {
        return format!("{lang:X}");
    }
    String::from_utf16_lossy(&buf[..(n as usize - 1).min(buf.len())]).to_uppercase()
}

/// Системный диалог выбора папки. Блокирует поток до закрытия диалога — вызывать только из
/// отдельного потока. `initial` — папка, с которой диалог открывается. `None` — отмена или сбой.
pub fn pick_folder(initial: Option<&str>) -> Option<String> {
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
