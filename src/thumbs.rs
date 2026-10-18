//! Миниатюры обоев: декодирование через WIC в коротком фоновом потоке (в простое потока нет).
//! Пиксели BGRA premultiplied запрошенного размера (его выбирает вкладка по масштабу экрана),
//! обрезка по центру до пропорций кадра. Готовые миниатюры лежат на диске в
//! `%APPDATA%\island\thumbs` (ключ — путь, размер и время изменения файла, размер миниатюры):
//! при следующем запуске картинки целиком не декодируются.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use windows::core::PCWSTR;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

use crate::{bg, log, sys};

static GEN: AtomicU32 = AtomicU32::new(0);
static READY: Mutex<Vec<(u32, usize, Vec<u8>)>> = Mutex::new(Vec::new());

/// Начинает декодирование списка в миниатюры `w`×`h`; возвращает номер поколения. Результаты
/// старых поколений отбрасываются в `drain`.
pub fn request(paths: Vec<PathBuf>, w: u32, h: u32) -> u32 {
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut r) = READY.lock() {
        r.clear();
    }
    let spawned = std::thread::Builder::new().name("island-thumbs".into()).spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(factory) = CoCreateInstance::<_, IWICImagingFactory>(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
        else {
            log::error("не удалось создать WIC");
            return;
        };
        for (i, p) in paths.iter().enumerate() {
            if GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            if let Some(px) = load(p, w, h).or_else(|| {
                let px = decode(&factory, p, w, h)?;
                store(p, w, h, &px);
                Some(px)
            }) {
                if let Ok(mut r) = READY.lock() {
                    r.push((gen, i, px));
                }
                bg::notify();
            }
        }
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить поток миниатюр: {e}"));
    }
    gen
}

/// Готовые миниатюры поколения `gen`: (индекс в списке, пиксели).
pub fn drain(gen: u32) -> Vec<(usize, Vec<u8>)> {
    let Ok(mut r) = READY.lock() else { return Vec::new() };
    let all = std::mem::take(&mut *r);
    all.into_iter().filter(|(g, _, _)| *g == gen).map(|(_, i, px)| (i, px)).collect()
}

/// Путь файла кэша для картинки; меняется при изменении файла.
fn cache_path(src: &std::path::Path, w: u32, h: u32) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(src).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    (src.to_string_lossy().to_lowercase(), meta.len(), mtime, w, h).hash(&mut hs);
    Some(sys::data_dir().join("thumbs").join(format!("{:016x}.bin", hs.finish())))
}

/// Готовая миниатюра из кэша на диске (`None` — её ещё не делали или картинка изменилась).
pub fn load(src: &std::path::Path, w: u32, h: u32) -> Option<Vec<u8>> {
    let px = std::fs::read(cache_path(src, w, h)?).ok()?;
    (px.len() == (w * h * 4) as usize).then_some(px)
}

fn store(src: &std::path::Path, w: u32, h: u32, px: &[u8]) {
    let Some(path) = cache_path(src, w, h) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, px);
}

unsafe fn decode(factory: &IWICImagingFactory, path: &std::path::Path, tw: u32, th: u32) -> Option<Vec<u8>> {
    let wide = sys::wz(path.to_str()?);
    let dec = factory
        .CreateDecoderFromFilename(PCWSTR(wide.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
        .ok()?;
    let frame = dec.GetFrame(0).ok()?;
    let (mut w, mut h) = (0u32, 0u32);
    frame.GetSize(&mut w, &mut h).ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    let k = (tw as f32 / w as f32).max(th as f32 / h as f32);
    let nw = ((w as f32 * k).ceil() as u32).max(tw);
    let nh = ((h as f32 * k).ceil() as u32).max(th);

    // Fant хорошо уменьшает, но при увеличении (мелкие превью WE) даёт «кубики» — там кубическая.
    let mode = if k < 1.0 { WICBitmapInterpolationModeFant } else { WICBitmapInterpolationModeHighQualityCubic };
    let scaler = factory.CreateBitmapScaler().ok()?;
    scaler.Initialize(&frame, nw, nh, mode).ok()?;
    let conv = factory.CreateFormatConverter().ok()?;
    conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
        .ok()?;
    let rect = WICRect {
        X: ((nw - tw) / 2) as i32,
        Y: ((nh - th) / 2) as i32,
        Width: tw as i32,
        Height: th as i32,
    };
    let mut buf = vec![0u8; (tw * th * 4) as usize];
    conv.CopyPixels(&rect, tw * 4, &mut buf).ok()?;
    Some(buf)
}
