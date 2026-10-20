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
    /// Поддерживает ли сессия перемотку позиции.
    pub can_seek: bool,
    /// Допустимые границы перемотки в секундах от начала трека.
    pub min_seek: f32,
    pub max_seek: f32,
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

    /// Срез дорожки воспроизведения на текущий момент.
    pub fn timeline(&self) -> Timeline {
        Timeline {
            pos: self.position_now(),
            dur: self.dur,
            min_seek: self.min_seek,
            max_seek: self.max_seek,
            can_seek: self.can_seek,
        }
    }
}

/// Вычисляет допустимый диапазон перемотки `(min_s, max_s)` в секундах от начала трека.
pub(super) fn seek_bounds(dur: f32, min_seek: f32, max_seek: f32) -> (f32, f32) {
    if !dur.is_finite() || dur <= 0.0 {
        return (0.0, 0.0);
    }
    if max_seek > min_seek {
        let min_s = min_seek.clamp(0.0, dur);
        let max_s = max_seek.clamp(min_s, dur);
        (min_s, max_s)
    } else {
        (0.0, dur)
    }
}

/// Состояние дорожки воспроизведения: текущая позиция, длительность и границы перемотки.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timeline {
    pub pos: f32,
    pub dur: f32,
    pub min_seek: f32,
    pub max_seek: f32,
    pub can_seek: bool,
}

impl Timeline {
    pub const fn empty() -> Self {
        Self {
            pos: 0.0,
            dur: 0.0,
            min_seek: 0.0,
            max_seek: 0.0,
            can_seek: false,
        }
    }

    /// Допустимые границы перемотки (в секундах от начала трека).
    pub fn bounds(&self) -> (f32, f32) {
        seek_bounds(self.dur, self.min_seek, self.max_seek)
    }

    /// Зажимает целевую позицию в допустимые границы трека.
    pub fn clamp(&self, target: f32) -> f32 {
        let (min_s, max_s) = self.bounds();
        if !target.is_finite() {
            min_s
        } else {
            target.clamp(min_s, max_s)
        }
    }

    /// Пересчитывает координату курсора `click_x` вдоль прогресс-бара шириной `bar_w` от `bar_x`
    /// в позицию трека (в секундах) с зажимом в границы.
    pub fn seek_to(&self, click_x: f32, bar_x: f32, bar_w: f32) -> f32 {
        if !self.dur.is_finite() || self.dur <= 0.0 || !bar_w.is_finite() || bar_w <= 0.0 {
            return 0.0;
        }
        let frac = if click_x.is_finite() {
            ((click_x - bar_x) / bar_w).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.clamp(frac * self.dur)
    }

    /// Смещает текущую позицию `pos` на `offset` секунд с зажимом в границы.
    pub fn relative(&self, offset: f32) -> f32 {
        if !self.dur.is_finite() || self.dur <= 0.0 {
            return 0.0;
        }
        let cur = if self.pos.is_finite() { self.pos } else { 0.0 };
        self.clamp(cur + offset)
    }

    /// Доля воспроизведения `0.0..=1.0` для отображения прогресс-бара.
    pub fn frac(&self) -> f32 {
        if self.dur > 0.0 {
            (self.pos / self.dur).clamp(0.0, 1.0)
        } else {
            0.0
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

struct SessionWatch {
    session: GlobalSystemMediaTransportControlsSession,
    prop_token: i64,
    info_token: i64,
    timeline_token: Option<i64>,
    aumid: String,
}

thread_local! {
    /// Подписка на события текущей медиасессии: сессия, токены событий и AUMID приложения.
    static SESSION_WATCH: RefCell<Option<SessionWatch>> = const { RefCell::new(None) };
}

/// Подписывается на изменения текущей медиасессии: смена трека, пауза/воспроизведение и перемотка
/// будят фоновый поток, поэтому опрашивать медиа по таймеру не нужно.
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
            (Some(old), Some(_)) if old.aumid == id && !resub => return,
            (None, None) => return,
            _ => {}
        }
        if let Some(old) = w.take() {
            let _ = old.session.RemoveMediaPropertiesChanged(old.prop_token);
            let _ = old.session.RemovePlaybackInfoChanged(old.info_token);
            if let Some(t3) = old.timeline_token {
                let _ = old.session.RemoveTimelinePropertiesChanged(t3);
            }
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
        let t3 = s.TimelinePropertiesChanged(&TypedEventHandler::new(|_, _| {
            send(Cmd::Media);
            Ok(())
        })).ok();
        if let (Ok(prop_token), Ok(info_token)) = (t1, t2) {
            *w = Some(SessionWatch {
                session: s,
                prop_token,
                info_token,
                timeline_token: t3,
                aumid: id,
            });
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
    match cmd {
        Cmd::PlayPause => {
            let _ = wait(s.TryTogglePlayPauseAsync());
        }
        Cmd::Next => {
            let _ = wait(s.TrySkipNextAsync());
        }
        Cmd::Prev => {
            let _ = wait(s.TrySkipPreviousAsync());
        }
        Cmd::Seek(pos) => {
            seek_session(&s, pos);
        }
        _ => {}
    }
}

/// Перематывает трек текущей сессии на позицию в секундах от начала, если поддерживается.
unsafe fn seek_session(s: &GlobalSystemMediaTransportControlsSession, target_secs: f32) -> Option<()> {
    let info = s.GetPlaybackInfo().ok()?;
    let ctrls = info.Controls().ok()?;
    if !ctrls.IsPlaybackPositionEnabled().unwrap_or(false) {
        return None;
    }
    let tl = s.GetTimelineProperties().ok()?;
    let secs = |t: windows::Foundation::TimeSpan| t.Duration as f64 / 1e7;
    let start = tl.StartTime().map(secs).unwrap_or(0.0);
    let end = tl.EndTime().map(secs).unwrap_or(0.0);
    let dur = (end - start).max(0.0) as f32;
    if dur <= 0.0 {
        return None;
    }
    let min_s = tl.MinSeekTime().map(secs).unwrap_or(start) - start;
    let max_s = tl.MaxSeekTime().map(secs).unwrap_or(end) - start;
    let (min_seek, max_seek) = seek_bounds(dur, min_s as f32, max_s as f32);
    let clamped = target_secs.clamp(min_seek, max_seek);
    let abs_pos = start + clamped as f64;
    let hns = (abs_pos * 1e7).round() as i64;
    let _ = wait(s.TryChangePlaybackPositionAsync(hns));
    Some(())
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
    let playback_info = s.GetPlaybackInfo().ok();
    let playing = playback_info
        .as_ref()
        .and_then(|i| i.PlaybackStatus().ok())
        == Some(Status::Playing);
    let can_seek_ctrl = playback_info
        .as_ref()
        .and_then(|i| i.Controls().ok())
        .and_then(|c| c.IsPlaybackPositionEnabled().ok())
        .unwrap_or(false);
    let (mut pos, mut dur) = (0.0f32, 0.0f32);
    let (mut min_seek, mut max_seek) = (0.0f32, 0.0f32);
    if let Ok(tl) = s.GetTimelineProperties() {
        let secs = |t: windows::Foundation::TimeSpan| t.Duration as f64 / 1e7;
        let start = tl.StartTime().map(secs).unwrap_or(0.0);
        let end = tl.EndTime().map(secs).unwrap_or(0.0);
        let mut p = tl.Position().map(secs).unwrap_or(0.0) - start;
        dur = (end - start).max(0.0) as f32;
        let min_s = tl.MinSeekTime().map(secs).unwrap_or(start) - start;
        let max_s = tl.MaxSeekTime().map(secs).unwrap_or(end) - start;
        let (min_b, max_b) = seek_bounds(dur, min_s as f32, max_s as f32);
        min_seek = min_b;
        max_seek = max_b;
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
    let can_seek = can_seek_ctrl && dur > 0.0;

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
    Some(Media {
        title,
        artist,
        playing,
        pos: pos.max(0.0),
        dur,
        at: Instant::now(),
        art,
        key,
        can_seek,
        min_seek,
        max_seek,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seek_bounds_normal() {
        assert_eq!(seek_bounds(100.0, 0.0, 100.0), (0.0, 100.0));
        assert_eq!(seek_bounds(100.0, 10.0, 90.0), (10.0, 90.0));
    }

    #[test]
    fn test_seek_bounds_clamps_to_dur() {
        assert_eq!(seek_bounds(100.0, -10.0, 150.0), (0.0, 100.0));
    }

    #[test]
    fn test_seek_bounds_inverted_or_zero() {
        // max <= min falls back to full dur
        assert_eq!(seek_bounds(100.0, 0.0, 0.0), (0.0, 100.0));
        assert_eq!(seek_bounds(100.0, 80.0, 20.0), (0.0, 100.0));
    }

    #[test]
    fn test_seek_bounds_invalid_dur() {
        assert_eq!(seek_bounds(0.0, 0.0, 0.0), (0.0, 0.0));
        assert_eq!(seek_bounds(-10.0, 0.0, 10.0), (0.0, 0.0));
        assert_eq!(seek_bounds(f32::NAN, 0.0, 10.0), (0.0, 0.0));
    }

    #[test]
    fn test_timeline_clamp_and_relative() {
        let tl = Timeline { pos: 50.0, dur: 100.0, min_seek: 20.0, max_seek: 80.0, can_seek: true };
        assert_eq!(tl.clamp(10.0), 20.0);
        assert_eq!(tl.clamp(90.0), 80.0);
        assert_eq!(tl.clamp(50.0), 50.0);
        assert_eq!(tl.relative(10.0), 60.0);
        assert_eq!(tl.relative(-40.0), 20.0);
        assert_eq!(tl.relative(50.0), 80.0);
    }

    #[test]
    fn test_timeline_seek_to() {
        let tl = Timeline { pos: 0.0, dur: 100.0, min_seek: 0.0, max_seek: 100.0, can_seek: true };
        assert_eq!(tl.seek_to(100.0, 100.0, 200.0), 0.0);
        assert_eq!(tl.seek_to(200.0, 100.0, 200.0), 50.0);
        assert_eq!(tl.seek_to(300.0, 100.0, 200.0), 100.0);
    }

    #[test]
    fn test_timeline_frac() {
        let tl = Timeline { pos: 25.0, dur: 100.0, min_seek: 0.0, max_seek: 100.0, can_seek: true };
        assert_eq!(tl.frac(), 0.25);
        let tl_zero = Timeline { pos: 0.0, dur: 0.0, min_seek: 0.0, max_seek: 0.0, can_seek: false };
        assert_eq!(tl_zero.frac(), 0.0);
    }
}

