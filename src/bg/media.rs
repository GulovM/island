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
    /// Скорость воспроизведения: 1 — обычная, 0 — плеер стоит (пауза или буферизация видео,
    /// при которой статус остаётся «играет»).
    pub rate: f32,
    /// Обложка: пиксели BGRA premultiplied `ART`×`ART` (если у приложения есть).
    pub art: Option<Arc<Vec<u8>>>,
    /// Ключ трека (хэш названия и исполнителя), по нему кэшируется битмап и сверяется смена трека.
    pub key: u64,
    /// Поддерживает ли сессия перемотку позиции.
    pub can_seek: bool,
    /// Допустимые границы перемотки в секундах от начала трека.
    pub min_seek: f32,
    pub max_seek: f32,
    /// Имя приложения-источника для шапки плеера («Firefox», «Spotify»); пусто — неизвестно.
    pub source_name: String,
    /// Сколько активных источников звука: стрелки выбора видны при `session_count > 1`.
    pub session_count: usize,
}

impl Media {
    /// Позиция с поправкой на прошедшее с момента опроса время, пока играет.
    pub fn position_now(&self) -> f32 {
        let p = if self.playing { self.pos + self.at.elapsed().as_secs_f32() * self.rate } else { self.pos };
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
    /// AUMID последней активно игравшей медиасессии (для сохранения выбора после постановки на паузу).
    static LAST_PLAYING_AUMID: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Чистая функция выбора индекса медиасессии из списка `(aumid, is_playing, is_current)`.
/// 1) Если есть играющие — предпочитает играющую и текущую, иначе первую играющую.
/// 2) Если ни одна не играет — ищет `last_playing_aumid`, иначе `is_current`, иначе первую.
fn pick_session_index(sessions: &[(&str, bool, bool)], last_playing_aumid: &str) -> Option<usize> {
    if sessions.is_empty() {
        return None;
    }
    if let Some(idx) = sessions.iter().position(|&(_, playing, current)| playing && current) {
        return Some(idx);
    }
    if let Some(idx) = sessions.iter().position(|&(_, playing, _)| playing) {
        return Some(idx);
    }
    if !last_playing_aumid.is_empty() {
        if let Some(idx) = sessions.iter().position(|&(aumid, _, _)| aumid == last_playing_aumid) {
            return Some(idx);
        }
    }
    if let Some(idx) = sessions.iter().position(|&(_, _, current)| current) {
        return Some(idx);
    }
    Some(0)
}

/// Сессия, которую есть смысл показывать (есть название или исполнитель), с прочитанными свойствами.
struct Active {
    session: GlobalSystemMediaTransportControlsSession,
    props: GlobalSystemMediaTransportControlsSessionMediaProperties,
    title: String,
    artist: String,
    aumid: String,
    /// Номер среди сессий с тем же AUMID: у браузера их бывает несколько (по одной на вкладку),
    /// и выбор только по AUMID не дал бы перейти со второй на первую.
    ordinal: usize,
    playing: bool,
    current: bool,
}

/// Все сессии `GetSessions()` с метаданными, в порядке менеджера.
fn active_sessions(mgr: &Mgr) -> Vec<Active> {
    let cur = mgr.GetCurrentSession().ok();
    let aumid_of = |s: &GlobalSystemMediaTransportControlsSession| {
        s.SourceAppUserModelId().ok().map(|h| h.to_string()).unwrap_or_default()
    };
    let cur_aumid = cur.as_ref().map(aumid_of).unwrap_or_default();
    let mut list: Vec<GlobalSystemMediaTransportControlsSession> =
        mgr.GetSessions().ok().map(|v| v.into_iter().collect()).unwrap_or_default();
    if list.is_empty() {
        list.extend(cur);
    }
    let mut out: Vec<Active> = Vec::with_capacity(list.len());
    for session in list {
        let Ok(props) = wait(session.TryGetMediaPropertiesAsync()) else { continue };
        let title = props.Title().map(|t| t.to_string()).unwrap_or_default();
        let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
        if title.is_empty() && artist.is_empty() {
            continue;
        }
        let aumid = aumid_of(&session);
        let playing = session.GetPlaybackInfo().ok().and_then(|i| i.PlaybackStatus().ok()) == Some(Status::Playing);
        let ordinal = out.iter().filter(|a| a.aumid == aumid).count();
        let current = !cur_aumid.is_empty() && aumid == cur_aumid;
        out.push(Active { session, props, title, artist, aumid, ordinal, playing, current });
    }
    out
}

/// Сессия, которая реально звучит, — её показывает свёрнутый остров.
fn live_index(active: &[Active]) -> Option<usize> {
    let refs: Vec<(&str, bool, bool)> = active.iter().map(|a| (a.aumid.as_str(), a.playing, a.current)).collect();
    let idx = LAST_PLAYING_AUMID.with(|last| pick_session_index(&refs, &last.borrow()))?;
    if active[idx].playing && !active[idx].aumid.is_empty() {
        LAST_PLAYING_AUMID.with(|last| *last.borrow_mut() = active[idx].aumid.clone());
    }
    Some(idx)
}

/// Сессия, выбранная стрелками в карточке плеера.
struct Pin {
    aumid: String,
    /// Номер среди одноимённых и трек на момент выбора. Порядок вкладок браузера в `GetSessions()`
    /// не гарантирован, поэтому сначала ищем по треку, а номер — запасной признак (трек сменился).
    ordinal: usize,
    track: String,
}

thread_local! {
    /// Весь SMTC живёт в фоновом потоке, поэтому хватает thread_local, как и у `LAST_PLAYING_AUMID`.
    static PINNED: RefCell<Option<Pin>> = const { RefCell::new(None) };
    /// Сессия карточки из последнего чтения: команды плеера идут в неё без повторного
    /// чтения метаданных всех сессий (при перетаскивании перемотка шлётся каждые 100 мс).
    static CARD_SESSION: RefCell<Option<GlobalSystemMediaTransportControlsSession>> = const { RefCell::new(None) };
}

/// Индекс закреплённой сессии среди `(aumid, ordinal, track)` активных; `None`, если её уже нет.
fn pinned_index(ids: &[(&str, usize, &str)], pin: &Pin) -> Option<usize> {
    ids.iter()
        .position(|&(a, _, t)| a == pin.aumid && t == pin.track)
        .or_else(|| ids.iter().position(|&(a, o, _)| a == pin.aumid && o == pin.ordinal))
}

/// Ключ трека для закрепления: название и исполнитель.
fn track_key(title: &str, artist: &str) -> String {
    format!("{title}\u{1f}{artist}")
}

/// Сдвиг выбора на `delta` по кругу из `n` сессий.
fn cycle_index(cur: usize, n: usize, delta: isize) -> usize {
    if n == 0 {
        return 0;
    }
    (cur as isize + delta).rem_euclid(n as isize) as usize
}

/// Сессия карточки плеера: выбранная стрелками, пока жива; закрытое приложение снимает выбор,
/// и карточка возвращается к живой сессии.
fn card_index(active: &[Active], live: Option<usize>) -> Option<usize> {
    PINNED.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(pin) = p.as_mut() {
            let tracks: Vec<String> = active.iter().map(|a| track_key(&a.title, &a.artist)).collect();
            let ids: Vec<(&str, usize, &str)> =
                active.iter().zip(&tracks).map(|(a, t)| (a.aumid.as_str(), a.ordinal, t.as_str())).collect();
            if let Some(i) = pinned_index(&ids, pin) {
                // Запоминаем, где сессия сейчас: следующий трек или переставленные вкладки найдутся.
                pin.ordinal = active[i].ordinal;
                pin.track = tracks[i].clone();
                return Some(i);
            }
            *p = None;
        }
        live
    })
}

/// Перелистывает карточку плеера на соседнюю активную сессию. Воспроизведение не трогает:
/// свёрнутый остров продолжает показывать то, что реально играет.
pub(super) fn cycle_session(delta: isize) {
    let Some(mgr) = media_manager() else { return };
    let active = active_sessions(&mgr);
    if active.len() < 2 {
        return;
    }
    let live = live_index(&active);
    let Some(cur) = card_index(&active, live) else { return };
    let next = &active[cycle_index(cur, active.len(), delta)];
    let pin = Pin { aumid: next.aumid.clone(), ordinal: next.ordinal, track: track_key(&next.title, &next.artist) };
    PINNED.with(|p| *p.borrow_mut() = Some(pin));
    CARD_SESSION.with(|c| *c.borrow_mut() = Some(next.session.clone()));
}

/// Браузеры то снимают флаг перемотки, то отдают пустой таймлайн, хотя перемотку принимают.
fn is_browser(aumid: &str) -> bool {
    let low = aumid.to_ascii_lowercase();
    !low.contains("yandex.music")
        && ["chrome", "msedge", "firefox", "308046b0af4a39cb", "opera", "brave", "yandex"].iter().any(|k| low.contains(k))
}

/// Человекочитаемое имя приложения по AUMID: известные плееры и браузеры по подстроке,
/// иначе — имя exe или пакета без расширения.
fn source_name(aumid: &str) -> String {
    let low = aumid.to_ascii_lowercase();
    const KNOWN: &[(&str, &str)] = &[
        ("yandex.music", "Yandex Music"),
        ("yandexmusic", "Yandex Music"),
        ("firefox", "Firefox"),
        // Firefox регистрирует сессию под хэшем каталога установки, а не под именем.
        ("308046b0af4a39cb", "Firefox"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("yandex", "Yandex"),
        ("spotify", "Spotify"),
        ("telegram", "Telegram"),
        ("vlc", "VLC"),
        ("aimp", "AIMP"),
        ("applemusic", "Apple Music"),
        ("itunes", "iTunes"),
        ("zunemusic", "Media Player"),
        ("zunevideo", "Movies & TV"),
    ];
    if let Some((_, name)) = KNOWN.iter().find(|(k, _)| low.contains(k)) {
        if *name == "Yandex Music" {
            return crate::i18n::pick("Яндекс Музыка", "Yandex Music").to_string();
        }
        return (*name).to_string();
    }
    // `C:\…\foo.exe`, `Vendor.App_hash!App` → «foo», «App».
    let base = aumid.rsplit(['\\', '/']).next().unwrap_or(aumid);
    let base = base.split('!').next().unwrap_or(base);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let base = base.split('_').next().unwrap_or(base);
    let base = base.rsplit('.').next().unwrap_or(base);
    // Хэш вместо имени ничего не скажет пользователю: пусть шапка покажет устройство вывода.
    if base.len() >= 12 && base.bytes().all(|b| b.is_ascii_hexdigit()) {
        return String::new();
    }
    let mut c = base.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
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
    /// Подписки на события всех активных медиасессий.
    static SESSION_WATCH: RefCell<Vec<SessionWatch>> = const { RefCell::new(Vec::new()) };
}

/// Подписывается на изменения всех активных медиасессий: смена трека, пауза/воспроизведение и перемотка
/// будят фоновый поток, поэтому опрашивать медиа по таймеру не нужно.
unsafe fn watch_media() {
    let Some(mgr) = media_manager() else { return };
    let mut sessions: Vec<GlobalSystemMediaTransportControlsSession> = mgr
        .GetSessions()
        .ok()
        .map(|v| v.into_iter().collect())
        .unwrap_or_default();
    if sessions.is_empty() {
        if let Ok(cur) = mgr.GetCurrentSession() {
            sessions.push(cur);
        }
    }
    let ids: Vec<String> = sessions
        .iter()
        .map(|s| {
            s.SourceAppUserModelId()
                .ok()
                .map(|h| h.to_string())
                .unwrap_or_default()
        })
        .collect();
    let resub = RESUBSCRIBE.swap(false, Ordering::SeqCst);
    SESSION_WATCH.with(|w| {
        let mut w = w.borrow_mut();
        if !resub && w.len() == ids.len() && w.iter().zip(&ids).all(|(old, id)| &old.aumid == id) {
            return;
        }
        for old in w.drain(..) {
            let _ = old.session.RemoveMediaPropertiesChanged(old.prop_token);
            let _ = old.session.RemovePlaybackInfoChanged(old.info_token);
            if let Some(t3) = old.timeline_token {
                let _ = old.session.RemoveTimelinePropertiesChanged(t3);
            }
        }
        for (s, id) in sessions.into_iter().zip(ids) {
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
                w.push(SessionWatch {
                    session: s,
                    prop_token,
                    info_token,
                    timeline_token: t3,
                    aumid: id,
                });
            }
        }
    });
}

/// Обновляет в снимке только медиа (события сессии), остальное оставляя как есть.
pub(super) unsafe fn refresh_media() {
    let (media, live_media) = read_media_pair();
    if let Ok(mut s) = SNAPSHOT.lock() {
        let mut snap = s.clone().unwrap_or_default();
        snap.media = media;
        snap.live_media = live_media;
        snap.rev += 1;
        *s = Some(snap);
    }
}

/// Команды плеера идут в сессию карточки — ту, что пользователь видит и выбрал стрелками.
pub(super) unsafe fn media_command(cmd: Cmd) {
    let Some(mgr) = media_manager() else { return };
    // Сессия карточки из последнего снимка: снимок обновляется по каждому событию SMTC, а
    // перечитывать метаданные всех сессий на каждую команду дорого. Без метаданных у всех сессий
    // (плеер после «стоп») команда идёт текущей системной, чтобы пустая карточка могла её разбудить.
    let s = CARD_SESSION.with(|c| c.borrow().clone()).or_else(|| mgr.GetCurrentSession().ok());
    let Some(s) = s else { return };
    let is_current = mgr
        .GetCurrentSession()
        .ok()
        .and_then(|c| Some(c.SourceAppUserModelId().ok()? == s.SourceAppUserModelId().ok()?))
        .unwrap_or(false);
    let ok = |r: windows::core::Result<bool>| r.unwrap_or(false);
    match cmd {
        Cmd::PlayPause => {
            let status_before = s.GetPlaybackInfo().ok().and_then(|i| i.PlaybackStatus().ok());
            let playing = status_before == Some(Status::Playing);
            let primary = if playing {
                wait(s.TryPauseAsync())
            } else {
                wait(s.TryPlayAsync())
            };
            log::info(&format!("медиа PlayPause (playing={playing}): {primary:?}"));
            let mut done = ok(primary);
            if !done {
                let status_after = s.GetPlaybackInfo().ok().and_then(|i| i.PlaybackStatus().ok());
                if status_after != status_before {
                    done = true;
                } else {
                    let toggle = wait(s.TryTogglePlayPauseAsync());
                    log::info(&format!("медиа TryTogglePlayPause: {toggle:?}"));
                    done = ok(toggle)
                        || s.GetPlaybackInfo().ok().and_then(|i| i.PlaybackStatus().ok()) != status_before;
                }
            }
            if !done && is_current {
                media_key(0xB3);
            }
        }
        Cmd::Next => {
            let res = wait(s.TrySkipNextAsync());
            log::info(&format!("медиа Next: {res:?}"));
            if !ok(res) && is_current {
                media_key(0xB0);
            }
        }
        Cmd::Prev => {
            let res = wait(s.TrySkipPreviousAsync());
            log::info(&format!("медиа Prev: {res:?}"));
            if !ok(res) && is_current {
                media_key(0xB1);
            }
        }
        Cmd::Seek(pos) => {
            seek_session(&s, pos);
        }
        _ => {}
    }
}

/// Нажимает системную медиа-клавишу (VK_MEDIA_NEXT_TRACK 0xB0, PREV_TRACK 0xB1, PLAY_PAUSE 0xB3).
unsafe fn media_key(vk: u8) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
    keybd_event(vk, 0, Default::default(), 0);
    keybd_event(vk, 0, KEYEVENTF_KEYUP, 0);
}

/// Перематывает трек сессии на позицию в секундах от начала.
/// Флаг `IsPlaybackPositionEnabled` не проверяем: браузеры часто сбрасывают его и обнуляют
/// таймлайн между опросами, хотя перемотку принимают; неподдерживаемый запрос плеер просто отклонит.
unsafe fn seek_session(s: &GlobalSystemMediaTransportControlsSession, target_secs: f32) -> Option<()> {
    if !target_secs.is_finite() {
        return None;
    }
    let tl = s.GetTimelineProperties().ok()?;
    let secs = |t: windows::Foundation::TimeSpan| t.Duration as f64 / 1e7;
    let start = tl.StartTime().map(secs).unwrap_or(0.0);
    let end = tl.EndTime().map(secs).unwrap_or(0.0);
    let dur = (end - start).max(0.0) as f32;
    let clamped = if dur > 0.0 {
        let min_s = tl.MinSeekTime().map(secs).unwrap_or(start) - start;
        let max_s = tl.MaxSeekTime().map(secs).unwrap_or(end) - start;
        let (min_seek, max_seek) = seek_bounds(dur, min_s as f32, max_s as f32);
        target_secs.clamp(min_seek, max_seek)
    } else {
        // Таймлайн сейчас пуст — позиция взята из последнего непустого, который видел интерфейс.
        target_secs.max(0.0)
    };
    let abs_pos = start + clamped as f64;
    let hns = (abs_pos * 1e7).round() as i64;
    let _ = wait(s.TryChangePlaybackPositionAsync(hns));
    Some(())
}

/// Обложки последних треков: повторно не декодируем, пока трек тот же. Несколько записей,
/// потому что карточка и свёрнутый остров могут показывать разные сессии одновременно.
static COVER: Mutex<Vec<(u64, Arc<Vec<u8>>)>> = Mutex::new(Vec::new());
const COVER_CACHE: usize = 8;

/// `(сессия карточки плеера, сессия, которая реально играет)`; часто это одна и та же.
pub(super) unsafe fn read_media_pair() -> (Option<Media>, Option<Media>) {
    watch_media();
    let Some(mgr) = media_manager() else { return (None, None) };
    let active = active_sessions(&mgr);
    let live = live_index(&active);
    let card = card_index(&active, live);
    CARD_SESSION.with(|c| *c.borrow_mut() = card.map(|i| active[i].session.clone()));
    let n = active.len();
    let card_media = card.map(|i| build_media(&active[i], n));
    let live_media = if live == card { card_media.clone() } else { live.map(|i| build_media(&active[i], n)) };
    (card_media, live_media)
}

unsafe fn build_media(a: &Active, session_count: usize) -> Media {
    let (s, props) = (&a.session, &a.props);
    let (title, artist) = (a.title.clone(), a.artist.clone());
    let playback_info = s.GetPlaybackInfo().ok();
    let playing = playback_info
        .as_ref()
        .and_then(|i| i.PlaybackStatus().ok())
        == Some(Status::Playing);
    // Не у всех плееров скорость заполнена: тогда считаем, что она обычная.
    let rate = playback_info
        .as_ref()
        .and_then(|i| i.PlaybackRate().ok())
        .and_then(|r| r.Value().ok())
        .map_or(1.0, |r| r.clamp(0.0, 4.0) as f32);
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
                    p += behind * rate as f64;
                }
            }
        }
        pos = p as f32;
    }
    if dur > 0.0 {
        pos = pos.min(dur);
    }
    // Флагу плеера верим, кроме браузеров: у них известная длительность — достаточный признак.
    let can_seek = can_seek_ctrl || (dur > 0.0 && is_browser(&a.aumid));

    let mut h = DefaultHasher::new();
    (&a.aumid, &title, &artist).hash(&mut h);
    let key = h.finish();
    let cached = COVER.lock().ok().and_then(|c| c.iter().find(|(k, _)| *k == key).map(|(_, px)| px.clone()));
    let art = cached.or_else(|| {
        let px = Arc::new(read_cover(props)?);
        if let Ok(mut c) = COVER.lock() {
            if c.len() >= COVER_CACHE {
                c.remove(0);
            }
            c.push((key, px.clone()));
        }
        Some(px)
    });
    Media {
        title,
        artist,
        playing,
        pos: pos.max(0.0),
        dur,
        at: Instant::now(),
        rate,
        art,
        key,
        can_seek,
        min_seek,
        max_seek,
        source_name: source_name(&a.aumid),
        session_count,
    }
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
    fn test_pinned_index_tracks_aumid_and_ordinal() {
        let pin = |a: &str, o: usize, t: &str| Pin { aumid: a.into(), ordinal: o, track: t.into() };
        let ids = [("Chrome", 0, "A"), ("Spotify.exe", 0, "S"), ("Chrome", 1, "B")];
        assert_eq!(pinned_index(&ids, &pin("Chrome", 1, "B")), Some(2));
        assert_eq!(pinned_index(&ids, &pin("Spotify.exe", 0, "S")), Some(1));
        // Вкладки переставились: трек «B» теперь первый — выбор идёт за треком, а не за номером.
        let swapped = [("Chrome", 0, "B"), ("Spotify.exe", 0, "S"), ("Chrome", 1, "A")];
        assert_eq!(pinned_index(&swapped, &pin("Chrome", 1, "B")), Some(0));
        // Трек сменился — остаётся номер среди одноимённых.
        assert_eq!(pinned_index(&ids, &pin("Chrome", 1, "старый")), Some(2));
        // Вторая вкладка браузера закрылась — выбора больше нет, карточка вернётся к живой сессии.
        assert_eq!(pinned_index(&ids[..2], &pin("Chrome", 1, "старый")), None);
    }

    #[test]
    fn test_cycle_index_wraps_both_ways() {
        assert_eq!(cycle_index(0, 3, -1), 2);
        assert_eq!(cycle_index(2, 3, 1), 0);
        assert_eq!(cycle_index(1, 3, 1), 2);
        assert_eq!(cycle_index(0, 0, 1), 0);
    }

    #[test]
    fn test_source_name_from_aumid() {
        assert_eq!(source_name("Chrome"), "Chrome");
        assert_eq!(source_name("308046B0AF4A39CB"), "Firefox");
        assert_eq!(source_name("MSEdge"), "Edge");
        assert_eq!(source_name("Spotify.exe"), "Spotify");
        assert_eq!(source_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(source_name(r"C:\Program Files\Foo\foobar2000.exe"), "Foobar2000");
        assert_eq!(source_name("Vendor.CoolPlayer_abc123!App"), "CoolPlayer");
        assert_eq!(source_name("6F193CCC56814779"), "", "неизвестный хэш не показываем");
        assert_eq!(source_name(""), "");
    }

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

    #[test]
    fn test_pick_session_index_prefers_playing_over_paused_current() {
        let sessions = [
            ("chrome.exe", false, true),
            ("Spotify.exe", true, false),
        ];
        assert_eq!(pick_session_index(&sessions, ""), Some(1));
    }

    #[test]
    fn test_pick_session_index_prefers_playing_current_when_multiple_playing() {
        let sessions = [
            ("chrome.exe", true, false),
            ("Spotify.exe", true, true),
        ];
        assert_eq!(pick_session_index(&sessions, ""), Some(1));
    }

    #[test]
    fn test_pick_session_index_falls_back_to_last_playing_then_current() {
        let sessions = [
            ("chrome.exe", false, true),
            ("Spotify.exe", false, false),
        ];
        assert_eq!(pick_session_index(&sessions, "Spotify.exe"), Some(1));
        assert_eq!(pick_session_index(&sessions, "unknown"), Some(0));
        assert_eq!(pick_session_index(&[], "Spotify.exe"), None);
    }
}

