//! Управление медиасессиями Windows (SMTC): чтение трека, обложки, событий воспроизведения.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager as Mgr,
    GlobalSystemMediaTransportControlsSessionMediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use super::winrt::wait;
use super::{send, Cmd, SNAPSHOT};
use crate::log;

/// Сторона квадрата обложки, пиксели.
pub const ART: u32 = 80;

#[derive(Clone, Debug)]
pub struct Media {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Позиция и длительность в секундах на момент `at`.
    pub pos: f32,
    pub dur: f32,
    pub at: Instant,
    /// Обложка: пиксели BGRA premultiplied `ART`×`ART` (если у приложения есть).
    pub art: Option<Arc<Vec<u8>>>,
    /// Ключ трека (хэш названия и исполнителя), по нему кэшируется битмап и сверяется смена трека.
    pub key: u64,
}

impl Media {
    /// Позиция с поправкой на прошедшее с момента опроса время, пока играет.
    pub fn position_now(&self) -> f32 {
        let p = if self.playing { self.pos + self.at.elapsed().as_secs_f32() } else { self.pos };
        if self.dur > 0.0 {
            p.min(self.dur)
        } else {
            p
        }
    }
}

thread_local! {
    /// Менеджер медиасессий создаётся один раз: его запрос — самая дорогая часть опроса.
    static MEDIA_MGR: RefCell<Option<Mgr>> = const { RefCell::new(None) };
}

/// Менеджер сообщил, что сессии поменялись: подписку на события сессии нужно пересоздать,
/// даже если у новой сессии тот же AUMID (вкладка браузера, следующий плеер того же приложения).
static RESUBSCRIBE: AtomicBool = AtomicBool::new(false);

fn media_manager() -> Option<Mgr> {
    MEDIA_MGR.with(|m| {
        let mut m = m.borrow_mut();
        if m.is_none() {
            let mgr = wait(Mgr::RequestAsync()).ok()?;
            // События самого менеджера: без них запущенная при свёрнутом острове музыка (сессии не
            // было, подписываться не на что) и смена трека другим плеером оставались незамеченными.
            let wake = || {
                log::info("медиасессии изменились (событие менеджера)");
                RESUBSCRIBE.store(true, Ordering::SeqCst);
                send(Cmd::Media);
            };
            let _ = mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                wake();
                Ok(())
            }));
            let _ = mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| {
                wake();
                Ok(())
            }));
            *m = Some(mgr);
        }
        m.clone()
    })
}

thread_local! {
    /// Подписка на события текущей медиасессии: (сессия, токены двух событий, AUMID приложения).
    static SESSION_WATCH: RefCell<Option<(GlobalSystemMediaTransportControlsSession, i64, i64, String)>> =
        const { RefCell::new(None) };
}

/// Подписывается на изменения текущей медиасессии: смена трека и пауза/воспроизведение будят
/// фоновый поток, поэтому опрашивать медиа по таймеру не нужно.
unsafe fn watch_media() {
    let Some(mgr) = media_manager() else { return };
    let cur = mgr.GetCurrentSession().ok();
    let id = cur
        .as_ref()
        .and_then(|s| s.SourceAppUserModelId().ok())
        .map(|h| h.to_string())
        .unwrap_or_default();
    let resub = RESUBSCRIBE.swap(false, Ordering::SeqCst);
    SESSION_WATCH.with(|w| {
        let mut w = w.borrow_mut();
        match (w.as_ref(), &cur) {
            (Some((_, _, _, old)), Some(_)) if *old == id && !resub => return,
            (None, None) => return,
            _ => {}
        }
        if let Some((old, t1, t2, _)) = w.take() {
            let _ = old.RemoveMediaPropertiesChanged(t1);
            let _ = old.RemovePlaybackInfoChanged(t2);
        }
        let Some(s) = cur else { return };
        let t1 = s.MediaPropertiesChanged(&TypedEventHandler::new(|_, _| {
            send(Cmd::Media);
            Ok(())
        }));
        let t2 = s.PlaybackInfoChanged(&TypedEventHandler::new(|_, _| {
            send(Cmd::Media);
            Ok(())
        }));
        if let (Ok(t1), Ok(t2)) = (t1, t2) {
            *w = Some((s, t1, t2, id));
        }
    });
}

/// Обновляет в снимке только медиа (события сессии), остальное оставляя как есть.
pub(super) unsafe fn refresh_media() {
    let media = read_media();
    if let Ok(mut s) = SNAPSHOT.lock() {
        let mut snap = s.clone().unwrap_or_default();
        snap.media = media;
        snap.rev += 1;
        *s = Some(snap);
    }
}

pub(super) unsafe fn media_command(cmd: Cmd) {
    let Some(mgr) = media_manager() else { return };
    let Ok(s) = mgr.GetCurrentSession() else { return };
    let _ = match cmd {
        Cmd::PlayPause => wait(s.TryTogglePlayPauseAsync()),
        Cmd::Next => wait(s.TrySkipNextAsync()),
        Cmd::Prev => wait(s.TrySkipPreviousAsync()),
        _ => return,
    };
}

/// Обложка последнего трека: повторно не декодируем, пока трек тот же.
static COVER: Mutex<Option<(u64, Arc<Vec<u8>>)>> = Mutex::new(None);

pub(super) unsafe fn read_media() -> Option<Media> {
    watch_media();
    let mgr = media_manager()?;
    let s = mgr.GetCurrentSession().ok()?;
    let props = wait(s.TryGetMediaPropertiesAsync()).ok()?;
    let title = props.Title().ok()?.to_string();
    let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
    if title.is_empty() && artist.is_empty() {
        return None;
    }
    let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).ok() == Some(Status::Playing);
    let (mut pos, mut dur) = (0.0f32, 0.0f32);
    if let Ok(tl) = s.GetTimelineProperties() {
        let secs = |t: windows::Foundation::TimeSpan| t.Duration as f64 / 1e7;
        let start = tl.StartTime().map(secs).unwrap_or(0.0);
        let mut p = tl.Position().map(secs).unwrap_or(0.0) - start;
        dur = (tl.EndTime().map(secs).unwrap_or(0.0) - start).max(0.0) as f32;
        // Позиция относится к моменту `LastUpdatedTime`, а не к «сейчас»: добавляем прошедшее.
        if playing {
            if let Ok(t) = tl.LastUpdatedTime() {
                // 100-нс интервалы с 1601 года (как FILETIME).
                const UNIX_OFFSET: f64 = 11_644_473_600.0;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0)
                    + UNIX_OFFSET;
                let behind = now - t.UniversalTime as f64 / 1e7;
                if (0.0..43_200.0).contains(&behind) {
                    p += behind;
                }
            }
        }
        pos = p as f32;
    }
    if dur > 0.0 {
        pos = pos.min(dur);
    }

    let mut h = DefaultHasher::new();
    (&title, &artist).hash(&mut h);
    let key = h.finish();
    let cached = COVER.lock().ok().and_then(|c| c.as_ref().filter(|(k, _)| *k == key).map(|(_, a)| a.clone()));
    let art = cached.or_else(|| {
        let px = Arc::new(read_cover(&props)?);
        if let Ok(mut c) = COVER.lock() {
            *c = Some((key, px.clone()));
        }
        Some(px)
    });
    Some(Media { title, artist, playing, pos: pos.max(0.0), dur, at: Instant::now(), art, key })
}

/// Обложка из миниатюры медиасессии → квадрат `ART`×`ART` (обрезка по центру).
unsafe fn read_cover(props: &GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<Vec<u8>> {
    let stream = wait(props.Thumbnail().ok()?.OpenReadAsync()).ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 16 << 20 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    wait(reader.LoadAsync(size)).ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;

    let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let ws = factory.CreateStream().ok()?;
    ws.InitializeFromMemory(&bytes).ok()?;
    let dec = factory.CreateDecoderFromStream(&ws, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
    let frame = dec.GetFrame(0).ok()?;
    let (mut w, mut h) = (0u32, 0u32);
    frame.GetSize(&mut w, &mut h).ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    let k = (ART as f32 / w as f32).max(ART as f32 / h as f32);
    let nw = ((w as f32 * k).ceil() as u32).max(ART);
    let nh = ((h as f32 * k).ceil() as u32).max(ART);
    let scaler = factory.CreateBitmapScaler().ok()?;
    scaler.Initialize(&frame, nw, nh, WICBitmapInterpolationModeFant).ok()?;
    let conv = factory.CreateFormatConverter().ok()?;
    conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
        .ok()?;
    let rect = WICRect { X: ((nw - ART) / 2) as i32, Y: ((nh - ART) / 2) as i32, Width: ART as i32, Height: ART as i32 };
    let mut buf = vec![0u8; (ART * ART * 4) as usize];
    conv.CopyPixels(&rect, ART * 4, &mut buf).ok()?;
    Some(buf)
}

