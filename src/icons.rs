//! Иконки приложений и список UWP-приложений через Shell. Всё выполняется в коротком фоновом
//! потоке (в простое потока нет); результаты забирает вкладка Apps по `drain`.
//!
//! Иконка — `IShellItemImageFactory::GetImage` для ярлыка `.lnk` или элемента папки
//! `shell:AppsFolder`; пиксели BGRA premultiplied размером `SIZE_PX`×`SIZE_PX`.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{HANDLE, SIZE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HGDIOBJ,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::{
    FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItemImageFactory, IShellLinkW, SHCreateItemFromParsingName,
    SHGetKnownFolderItem, ShellLink, BHID_EnumItems, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
    SIIGBF_ICONONLY,
};

use crate::{bg, log, sys};

pub const SIZE_PX: u32 = 48;

pub struct Uwp {
    pub name: String,
    /// `shell:AppsFolder\<AUMID>` — запускается через `ShellExecute`.
    pub target: String,
    pub px: Vec<u8>,
}

pub enum Found {
    /// Иконка для записи с таким путём.
    Icon(String, Vec<u8>),
    Uwp(Uwp),
    /// Ярлык с таким путём ведёт в никуда (приложение удалено) — из списка его надо убрать.
    Dead(String),
}

static GEN: AtomicU32 = AtomicU32::new(0);
static READY: Mutex<Vec<(u32, Found)>> = Mutex::new(Vec::new());

/// Запускает загрузку иконок для `paths` (ярлыки) и перечисление UWP-приложений.
/// Возвращает номер поколения: результаты старых поколений отбрасываются в `drain`.
pub fn request(paths: Vec<String>) -> u32 {
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new().name("island-icons".into()).spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let push = |f: Found| {
            if let Ok(mut r) = READY.lock() {
                r.push((gen, f));
            }
        };
        for p in paths {
            if GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            if shortcut_dead(&p) {
                push(Found::Dead(p));
            } else if let Some(px) = path_icon(&p) {
                push(Found::Icon(p, px));
            }
        }
        bg::notify();
        enumerate_uwp(gen, &push);
        bg::notify();
        // Shell и AppX подтянули ~55 МБ библиотек и куч, которые потоку больше не нужны.
        sys::trim_working_set();
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить поток иконок: {e}"));
    }
    gen
}

pub fn drain(gen: u32) -> Vec<Found> {
    let Ok(mut r) = READY.lock() else { return Vec::new() };
    std::mem::take(&mut *r).into_iter().filter(|(g, _)| *g == gen).map(|(_, f)| f).collect()
}

/// Ярлык меню «Пуск» ведёт в никуда. `.lnk` — сохранённая цель задана и файла нет; `.url` —
/// см. `url_dead`. Ярлыки без файловой цели (UWP, MSI-«рекламируемые», сетевые пути) живыми.
pub unsafe fn shortcut_dead(path: &str) -> bool {
    let lower = path.to_lowercase();
    if lower.ends_with(".url") {
        return std::fs::read(path).is_ok_and(|b| url_dead(&String::from_utf8_lossy(&b)));
    }
    if !lower.ends_with(".lnk") {
        return false;
    }
    match lnk_target(path) {
        Some(t) => !t.is_empty() && !t.starts_with("\\\\") && !std::path::Path::new(&t).exists(),
        None => false,
    }
}

unsafe fn lnk_target(path: &str) -> Option<String> {
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
    let file: IPersistFile = link.cast().ok()?;
    let wide = sys::wz(path);
    file.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
    let mut buf = [0u16; 520];
    link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// `.url` мёртв, если ведёт на `file:` без файла либо на нестандартный протокол (`steam://`,
/// `goggalaxy://`…), а его иконка лежит по абсолютному пути, которого уже нет: так выглядят
/// удалённые игры, оставившие ярлык в меню «Пуск».
fn url_dead(text: &str) -> bool {
    let (mut url, mut icon) = ("", "");
    for l in text.lines() {
        let l = l.trim();
        if let Some(v) = l.strip_prefix("URL=") {
            url = v.trim();
        } else if let Some(v) = l.strip_prefix("IconFile=") {
            icon = v.trim().trim_matches('"');
        }
    }
    let low = url.to_ascii_lowercase();
    let absolute = |p: &str| p.len() > 2 && p.as_bytes()[1] == b':';
    if let Some(rest) = low.strip_prefix("file:") {
        let rest = rest.trim_start_matches('/');
        return !rest.contains('%') && absolute(rest) && !std::path::Path::new(rest).exists();
    }
    let web = ["http:", "https:", "ftp:"].iter().any(|s| low.starts_with(s));
    !url.is_empty() && !web && absolute(icon) && !std::path::Path::new(icon).exists()
}

/// Иконка по пути ярлыка. Используется также тестом-«выгрузкой» для проверки.
pub unsafe fn path_icon(path: &str) -> Option<Vec<u8>> {
    let wide = sys::wz(&path.replace('/', "\\"));
    let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
    image_px(&factory)
}

unsafe fn image_px(f: &IShellItemImageFactory) -> Option<Vec<u8>> {
    let hbmp = f.GetImage(SIZE { cx: SIZE_PX as i32, cy: SIZE_PX as i32 }, SIIGBF_ICONONLY).ok()?;
    let mut bm = BITMAP::default();
    let got = GetObjectW(HGDIOBJ(hbmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void));
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        return None;
    }
    let (w, h) = (bm.bmWidth as usize, bm.bmHeight as usize);
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; w * h * 4];
    let dc = GetDC(None);
    let lines = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    let _ = DeleteObject(HGDIOBJ(hbmp.0));
    if lines == 0 {
        return None;
    }
    Some(normalize(&buf, w, h))
}

/// Приводит пиксели к `SIZE_PX`×`SIZE_PX` BGRA premultiplied. Альфа Shell приходит «прямая»;
/// если альфы нет вовсе (все нули) — картинка считается непрозрачной.
fn normalize(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let n = SIZE_PX as usize;
    let opaque = src.chunks_exact(4).all(|p| p[3] == 0);
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        let (y0, y1) = (y * h / n, ((y + 1) * h / n).max(y * h / n + 1).min(h));
        for x in 0..n {
            let (x0, x1) = (x * w / n, ((x + 1) * w / n).max(x * w / n + 1).min(w));
            // Среднее по блоку исходных пикселей: крупная иконка Shell уменьшается без «лесенки».
            let (mut acc, mut cnt) = ([0u32; 4], 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let s = (sy * w + sx) * 4;
                    let a = if opaque { 255 } else { src[s + 3] as u32 };
                    for c in 0..3 {
                        acc[c] += if opaque { src[s + c] as u32 } else { src[s + c] as u32 * a / 255 };
                    }
                    acc[3] += a;
                    cnt += 1;
                }
            }
            let o = (y * n + x) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / cnt.max(1)) as u8;
            }
        }
    }
    out
}

/// Перечисляет UWP-приложения папки `shell:AppsFolder` (у них в имени разбора есть «!»).
unsafe fn enumerate_uwp(gen: u32, push: &dyn Fn(Found)) {
    let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None::<HANDLE>) else {
        log::error("не удалось открыть shell:AppsFolder");
        return;
    };
    let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return };
    loop {
        if GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let mut item: [Option<IShellItem>; 1] = [None];
        let mut got = 0u32;
        let _ = en.Next(&mut item, Some(&mut got));
        let Some(item) = item[0].take() else { break };
        if got == 0 {
            break;
        }
        let take = |sigdn| -> Option<String> {
            let p = item.GetDisplayName(sigdn).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            s
        };
        let (Some(name), Some(parse)) = (take(SIGDN_NORMALDISPLAY), take(SIGDN_PARENTRELATIVEPARSING)) else { continue };
        if !parse.contains('!') {
            continue;
        }
        let Ok(factory) = item.cast::<IShellItemImageFactory>() else { continue };
        let Some(px) = image_px(&factory) else { continue };
        push(Found::Uwp(Uwp { name, target: format!("shell:AppsFolder\\{parse}"), px }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Живая проверка (нужен рабочий стол): иконки ярлыков и UWP в сетку PNG.
    #[test]
    #[ignore]
    fn live_dump() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let dir = std::path::PathBuf::from(std::env::var("ISLAND_DUMP").unwrap());
            let mut lnks = Vec::new();
            for base in [std::env::var("ProgramData").unwrap(), std::env::var("APPDATA").unwrap()] {
                let d = std::path::PathBuf::from(base).join("Microsoft/Windows/Start Menu/Programs");
                if let Ok(rd) = std::fs::read_dir(d) {
                    for e in rd.flatten() {
                        if e.path().extension().is_some_and(|x| x == "lnk") {
                            lnks.push(e.path().display().to_string());
                        }
                    }
                }
            }
            let mut icons: Vec<Vec<u8>> = lnks.iter().take(24).filter_map(|p| path_icon(p)).collect();
            let n_lnk = icons.len();
            let found = std::sync::Mutex::new(Vec::new());
            enumerate_uwp(0, &|f| {
                if let Found::Uwp(u) = f {
                    found.lock().unwrap().push((u.name, u.px));
                }
            });
            let uwp = found.into_inner().unwrap();
            println!("lnk icons: {n_lnk} of {}, uwp: {}", lnks.len(), uwp.len());
            for (n, _) in uwp.iter().take(10) {
                println!("  uwp: {n}");
            }
            icons.extend(uwp.into_iter().take(16).map(|(_, px)| px));
            let cols = 8usize;
            let n = SIZE_PX as usize;
            let rows = icons.len().div_ceil(cols);
            let (w, h) = (cols * n, rows * n);
            // Фон шахматкой, поверх — иконка с учётом premultiplied-альфы.
            let mut out = vec![0u8; w * h * 4];
            for (k, px) in icons.iter().enumerate() {
                let (ox, oy) = ((k % cols) * n, (k / cols) * n);
                for y in 0..n {
                    for x in 0..n {
                        let bg = if (x / 4 + y / 4) % 2 == 0 { 90u32 } else { 40u32 };
                        let s = (y * n + x) * 4;
                        let o = ((oy + y) * w + ox + x) * 4;
                        let a = px[s + 3] as u32;
                        for c in 0..3 {
                            out[o + c] = (px[s + c] as u32 + bg * (255 - a) / 255).min(255) as u8;
                        }
                        out[o + 3] = 255;
                    }
                }
            }
            crate::img::save_png(&dir.join("icons.png"), w as u32, h as u32, &out).expect("png");
        }
    }

    #[test]
    fn url_shortcuts_to_removed_games_are_dead() {
        // Удалённая игра Steam: протокол нестандартный, иконки на диске нет.
        assert!(url_dead("[InternetShortcut]\r\nURL=steam://rungameid/1\r\nIconFile=C:\\no\\such\\dir\\game.ico\r\n"));
        // Иконка на месте (берём существующий файл) — игра считается установленной.
        let exe = std::env::current_exe().unwrap();
        assert!(!url_dead(&format!("URL=steam://rungameid/1\nIconFile={}\n", exe.display())));
        // Обычные ссылки и ссылки без иконки не трогаем.
        assert!(!url_dead("URL=https://example.com\nIconFile=C:\\no\\such.ico\n"));
        assert!(!url_dead("URL=steam://rungameid/1\n"));
        assert!(!url_dead(""));
        // file: на несуществующий файл.
        assert!(url_dead("URL=file:///C:/no/such/file.exe\n"));
        assert!(!url_dead(&format!("URL=file:///{}\n", exe.display().to_string().replace('\\', "/"))));
    }

    #[test]
    fn normalize_premultiplies_and_resizes() {
        // 1×1 полупрозрачный → SIZE_PX² с цветом, умноженным на альфу.
        let px = normalize(&[200, 100, 50, 128], 1, 1);
        assert_eq!(px.len(), (SIZE_PX * SIZE_PX * 4) as usize);
        assert_eq!(&px[..4], &[100, 50, 25, 128]);
    }

    #[test]
    fn normalize_without_alpha_is_opaque() {
        let px = normalize(&[10, 20, 30, 0], 1, 1);
        assert_eq!(&px[..4], &[10, 20, 30, 255]);
    }
}
