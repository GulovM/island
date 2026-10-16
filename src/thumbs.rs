//! Миниатюры обоев: декодирование через WIC в коротком фоновом потоке (в простое потока нет).
//! Пиксели BGRA premultiplied размером `W`×`H`, обрезка по центру до пропорций кадра.
//! Готовые миниатюры лежат на диске в `%APPDATA%\island	humbs` (ключ — путь, размер и время
//! изменения файла): при следующем запуске картинки целиком не декодируются.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use windows::core::PCWSTR;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

use crate::{bg, log, sys};

pub const W: u32 = 160;
pub const H: u32 = 90;

static GEN: AtomicU32 = AtomicU32::new(0);
static READY: Mutex<Vec<(u32, usize, Vec<u8>)>> = Mutex::new(Vec::new());

/// Начинает декодирование списка; возвращает номер поколения. Результаты старых поколений
/// отбрасываются в `drain`.
pub fn request(paths: Vec<PathBuf>) -> u32 {
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
            if let Some(px) = cached(p).or_else(|| {
                let px = decode(&factory, p)?;
                store(p, &px);
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
fn cache_path(src: &std::path::Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(src).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (src.to_string_lossy().to_lowercase(), meta.len(), mtime, W, H).hash(&mut h);
    Some(sys::data_dir().join("thumbs").join(format!("{:016x}.bin", h.finish())))
}

fn cached(src: &std::path::Path) -> Option<Vec<u8>> {
    let px = std::fs::read(cache_path(src)?).ok()?;
    (px.len() == (W * H * 4) as usize).then_some(px)
}

fn store(src: &std::path::Path, px: &[u8]) {
    let Some(path) = cache_path(src) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, px);
}

unsafe fn decode(factory: &IWICImagingFactory, path: &std::path::Path) -> Option<Vec<u8>> {
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
    let k = (W as f32 / w as f32).max(H as f32 / h as f32);
    let nw = ((w as f32 * k).ceil() as u32).max(W);
    let nh = ((h as f32 * k).ceil() as u32).max(H);

    let scaler = factory.CreateBitmapScaler().ok()?;
    scaler.Initialize(&frame, nw, nh, WICBitmapInterpolationModeFant).ok()?;
    let conv = factory.CreateFormatConverter().ok()?;
    conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
        .ok()?;
    let rect = WICRect {
        X: ((nw - W) / 2) as i32,
        Y: ((nh - H) / 2) as i32,
        Width: W as i32,
        Height: H as i32,
    };
    let mut buf = vec![0u8; (W * H * 4) as usize];
    conv.CopyPixels(&rect, W * 4, &mut buf).ok()?;
    Some(buf)
}
