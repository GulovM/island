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
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIcon, CreateWindowExW, DestroyWindow, GetForegroundWindow, GetGUIThreadInfo, GetSystemMetrics,
    GetWindowThreadProcessId, GUITHREADINFO, HICON, SM_CMONITORS, SM_CXSMICON, SW_SHOWNORMAL, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
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
    for (d, s) in nid.szTip.iter_mut().zip("Kite Island".encode_utf16()) {
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

pub fn tray_set_icon(hwnd: HWND, icon: HICON) -> bool {
    let mut nid = tray_data(hwnd);
    nid.uFlags = NIF_ICON;
    nid.hIcon = icon;
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() }
}

/// Фирменный кобальт Kite (#2F5BFF): два клина змея в трее всегда этого цвета, независимо от
/// акцента острова, — значок в трее остаётся узнаваемым знаком, а не перекрашивается вместе с темой.
const KITE_COBALT: [f32; 3] = [0x2F as f32 / 255.0, 0x5B as f32 / 255.0, 0xFF as f32 / 255.0];

/// Значок трея — знак логотипа Kite: ромб воздушного змея из четырёх клиньев (см. assets/brand).
/// Рисуется кодом, а не берётся из .ico: два клина — фирменный кобальт, а два других — цвет,
/// контрастный панели задач, и он меняется на лету вместе с темой Windows.
pub fn make_icon(hinst: HINSTANCE, light_taskbar: bool) -> Option<HICON> {
    // Оболочка показывает значок в размере SM_CXSMICON; рисуем ровно его, чтобы Windows не
    // масштабировала картинку и не размывала крестовину змея.
    let n = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as usize;
    let xor = tray_pixels(n, KITE_COBALT, light_taskbar);
    // Маска AND не нужна (прозрачность в альфе), но её строки выровнены по WORD.
    let and = vec![0u8; n.div_ceil(16) * 2 * n];
    // SAFETY: буферы живут до конца вызова и имеют размеры, которых CreateIcon ждёт для n×n, 32 бит.
    unsafe { CreateIcon(Some(hinst), n as i32, n as i32, 1, 32, and.as_ptr(), xor.as_ptr()).ok() }
}

/// Пиксели значка трея n×n: BGRA, альфа не премультиплицирована (так ждёт CreateIcon).
/// Геометрия — доли n, та же, что у знака в assets/brand/mark.svg.
fn tray_pixels(n: usize, brand: [f32; 3], light_taskbar: bool) -> Vec<u8> {
    const SS: usize = 4;
    let nf = n as f32;
    // Крестовина — по границам пикселей: клинья разного цвета стыкуются без полутонового шва.
    let cx = (nf / 2.0).round();
    let cy = (0.44 * nf).round();
    // Ромб чуть уже квадрата и вытянут книзу — так он читается как змей, а не как «алмаз».
    let (top, bot, hw) = (0.04 * nf, 0.98 * nf, 0.40 * nf);
    let fg = if light_taskbar { [0x1A as f32 / 255.0; 3] } else { [1.0; 3] };

    let mut out = vec![0u8; n * n * 4];
    for py in 0..n {
        for px in 0..n {
            let (mut a_fg, mut a_brand) = (0.0f32, 0.0f32);
            for sy in 0..SS {
                for sxi in 0..SS {
                    let x = px as f32 + (sxi as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    let dx = (x - cx).abs() / hw;
                    let dy = if y < cy { (cy - y) / (cy - top) } else { (y - cy) / (bot - cy) };
                    if dx + dy > 1.0 {
                        continue;
                    }
                    // Верх-лево и низ-право — цвет панели, остальные два — кобальт.
                    if (x < cx) == (y < cy) {
                        a_fg += 1.0;
                    } else {
                        a_brand += 1.0;
                    }
                }
            }
            let k = (SS * SS) as f32;
            let (a_fg, a_brand) = (a_fg / k, a_brand / k);
            let a = a_fg + a_brand;
            if a <= 0.0 {
                continue;
            }
            let i = (py * n + px) * 4;
            for c in 0..3 {
                // BGRA: канал 0 — синий.
                let v = (fg[2 - c] * a_fg + brand[2 - c] * a_brand) / a;
                out[i + c] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            out[i + 3] = (a.min(1.0) * 255.0).round() as u8;
        }
    }
    out
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

/// Код языка (младшие 16 бит HKL) раскладки окна в фокусе ввода.
///
/// Возвращает `None`, если активного окна нет (переходное состояние: Alt+Tab, экран
/// блокировки, UAC) — в эти моменты нельзя подставлять раскладку острова, иначе виджет
/// покажет ложный язык вместо сохранения предыдущего.
///
/// Для UWP-приложений (Параметры, Почта и т.п.) окно верхнего уровня принадлежит процессу-хосту
/// `ApplicationFrameHost.exe`. Поток этого хост-окна не всегда держит фокус ввода и раскладка
/// активного ввода к нему не привязана — реальный фокус и раскладка живут в потоке сфокусированного
/// дочернего окна (`hwndFocus` из `GetGUIThreadInfo`).
pub fn foreground_lang() -> Option<u32> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let top_tid = GetWindowThreadProcessId(fg, None);
        if top_tid == 0 {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let tid = if GetGUIThreadInfo(top_tid, &mut info).is_ok() && !info.hwndFocus.0.is_null() {
            let focus_tid = GetWindowThreadProcessId(info.hwndFocus, None);
            if focus_tid != 0 {
                focus_tid
            } else {
                top_tid
            }
        } else {
            top_tid
        };
        Some((GetKeyboardLayout(tid).0 as usize & 0xFFFF) as u32)
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
        let mut owner: Option<HWND> = None;
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
            // Остров лежит поверх всех окон, и диалог без владельца открывается позади него:
            // владелец-невидимка с флагом topmost поднимает диалог над островом.
            owner = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
            .ok();
            // Отмена тоже возвращает ошибку (ERROR_CANCELLED) — это не сбой.
            dlg.Show(owner).ok()?;
            let item = dlg.GetResult().ok()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let s = name.to_string().ok();
            CoTaskMemFree(Some(name.0 as *const _));
            s
        })();
        if let Some(h) = owner {
            let _ = DestroyWindow(h);
        }
        if inited {
            CoUninitialize();
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    fn px(buf: &[u8], n: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * n + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn tray_icon_shapes_and_colors() {
        for n in [16usize, 20, 24, 32] {
            let nf = n as f32;
            let (cx, cy) = ((nf / 2.0).round() as usize, (0.44 * nf).round() as usize);
            let dark = tray_pixels(n, KITE_COBALT, false);
            let light = tray_pixels(n, KITE_COBALT, true);
            assert_eq!(dark.len(), n * n * 4);
            for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
                assert_eq!(px(&dark, n, x, y)[3], 0, "угол прозрачный, n={n}");
            }
            // Клинья у крестовины: верх-лево и низ-право — цвет панели, остальные — кобальт (BGRA).
            let cobalt = [0xFF, 0x5B, 0x2F, 255];
            assert_eq!(px(&dark, n, cx - 2, cy - 2), [255, 255, 255, 255], "n={n}");
            assert_eq!(px(&dark, n, cx + 1, cy + 2), [255, 255, 255, 255], "n={n}");
            assert_eq!(px(&light, n, cx - 2, cy - 2), [0x1A, 0x1A, 0x1A, 255], "n={n}");
            assert_eq!(px(&dark, n, cx + 1, cy - 2), cobalt, "n={n}");
            assert_eq!(px(&dark, n, cx - 2, cy + 2), cobalt, "n={n}");
            // Шов крестовины по границе пикселей: соседние пиксели чистые, без смеси цветов.
            assert_eq!(px(&dark, n, cx - 1, cy - 1), [255, 255, 255, 255], "n={n}");
            assert_eq!(px(&dark, n, cx, cy - 1), cobalt, "n={n}");
        }
    }

    #[test]
    fn test_foreground_lang_does_not_panic() {
        let _ = foreground_lang();
    }

    #[test]
    fn test_lang_code_formatting() {
        assert_eq!(lang_code(0x0409), "EN");
        assert_eq!(lang_code(0x0419), "RU");
    }
}
