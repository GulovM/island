//! Управление медиасессиями Windows (SMTC): чтение трека, обложки, событий воспроизведения.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager as Mgr,
    GlobalSystemMediaTransportControlsSessionMediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use super::winrt::{wait, wait_timeout};
use super::{send, Cmd, SNAPSHOT};
use crate::log;

/// Таймаут WinRT-вызовов SMTC: при зависшей вкладке браузера не держим фоновый поток по 5 секунд.
const MEDIA_WAIT_MS: u64 = 1500;
/// Пауза перед повторной загрузкой обложки, если поток миниатюры завис или не прочитался.
const COVER_FAIL_COOLDOWN: Duration = Duration::from_secs(10);

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
    /// AUMID и ключ трека последней активно игравшей медиасессии (для сохранения выбора после постановки на паузу).
    static LAST_PLAYING: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
}

/// Чистая функция выбора индекса медиасессии из списка `(aumid, track, is_playing, is_current)`.
/// 1) Если есть играющие — предпочитает играющую и текущую, иначе первую играющую.
/// 2) Если ни одна не играет — ищет последнюю игравшую (сначала по AUMID и треку, чтобы не прыгать
///    между вкладками браузера, затем по AUMID), иначе `is_current`, иначе первую.
fn pick_session_index_with_track(
    sessions: &[(&str, &str, bool, bool)],
    last_aumid: &str,
    last_track: &str,
) -> Option<usize> {
    if sessions.is_empty() {
        return None;
    }
    if let Some(idx) = sessions.iter().position(|&(_, _, playing, current)| playing && current) {
        return Some(idx);
    }
    if let Some(idx) = sessions.iter().position(|&(_, _, playing, _)| playing) {
        return Some(idx);
    }
    if !last_aumid.is_empty() {
        if !last_track.is_empty() {
            if let Some(idx) = sessions
                .iter()
                .position(|&(aumid, track, _, _)| aumid == last_aumid && track == last_track)
            {
                return Some(idx);
            }
        }
        if let Some(idx) = sessions
            .iter()
            .position(|&(aumid, _, _, current)| aumid == last_aumid && current)
        {
            return Some(idx);
        }
        if let Some(idx) = sessions.iter().position(|&(aumid, _, _, _)| aumid == last_aumid) {
            return Some(idx);
        }
    }
    if let Some(idx) = sessions.iter().position(|&(_, _, _, current)| current) {
        return Some(idx);
    }
    Some(0)
}

#[cfg(test)]
fn pick_session_index(sessions: &[(&str, bool, bool)], last_playing_aumid: &str) -> Option<usize> {
    let with_track: Vec<(&str, &str, bool, bool)> =
        sessions.iter().map(|&(a, p, c)| (a, "", p, c)).collect();
    pick_session_index_with_track(&with_track, last_playing_aumid, "")
}

/// Определяет флаг `current` для активных сессий: предпочитает точное совпадение COM-сессии,
/// и только если его нет ни у одной, откатывается к совпадению AUMID.
fn resolve_current_flags(exact: &[bool], aumids: &[&str], cur_aumid: &str) -> Vec<bool> {
    if exact.iter().any(|&e| e) {
        return exact.to_vec();
    }
    if cur_aumid.is_empty() {
        return vec![false; aumids.len()];
    }
    aumids.iter().map(|&a| a == cur_aumid).collect()
}

fn same_session(
    a: &GlobalSystemMediaTransportControlsSession,
    b: &GlobalSystemMediaTransportControlsSession,
) -> bool {
    a == b
}

unsafe fn session_alive(s: &GlobalSystemMediaTransportControlsSession) -> bool {
    s.GetPlaybackInfo()
        .and_then(|i| i.PlaybackStatus())
        .is_ok_and(|st| st != Status::Closed)
}

fn aumid_of(s: &GlobalSystemMediaTransportControlsSession) -> String {
    s.SourceAppUserModelId()
        .ok()
        .map(|h| h.to_string())
        .unwrap_or_default()
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

/// Читает список сессий, пересоздавая менеджер, если WinRT/COM-прокси отвалился.
unsafe fn fetch_sessions() -> Option<(
    Mgr,
    Vec<GlobalSystemMediaTransportControlsSession>,
    Option<GlobalSystemMediaTransportControlsSession>,
)> {
    let mut mgr = media_manager()?;
    let list = match mgr.GetSessions() {
        Ok(v) => v.into_iter().collect(),
        Err(e) => {
            log::error(&format!("Медиа: GetSessions не удался ({e}), пересоздаю менеджер"));
            reset_media_manager();
            mgr = media_manager()?;
            mgr.GetSessions()
                .ok()
                .map(|v| v.into_iter().collect())
                .unwrap_or_default()
        }
    };
    let cur = mgr.GetCurrentSession().ok();
    Some((mgr, list, cur))
}

/// Все активные сессии с метаданными, в порядке менеджера.
unsafe fn active_sessions_from(
    mut list: Vec<GlobalSystemMediaTransportControlsSession>,
    cur: Option<&GlobalSystemMediaTransportControlsSession>,
) -> Vec<Active> {
    let cur_aumid = cur.map(aumid_of).unwrap_or_default();
    let used_current = list.is_empty() && cur.is_some();
    if list.is_empty() {
        if let Some(c) = cur {
            list.push(c.clone());
        }
    }
    let mut out: Vec<Active> = Vec::with_capacity(list.len());
    let mut exact: Vec<bool> = Vec::with_capacity(list.len());
    // Сессии, отброшенные молча, не отличить от «браузер не публикует SMTC» — для журнала
    // запоминаем и их, и причину (только AUMID и статус, без названий треков).
    let mut diag: Vec<String> = Vec::with_capacity(list.len());
    let total = list.len();
    for session in list {
        let aumid = aumid_of(&session);
        let status = session.GetPlaybackInfo().ok().and_then(|i| i.PlaybackStatus().ok());
        if status == Some(Status::Closed) {
            diag.push(diag_entry(&aumid, "closed", "сессия закрыта, отброшена"));
            continue;
        }
        let Ok(props) = wait_timeout(session.TryGetMediaPropertiesAsync(), MEDIA_WAIT_MS) else {
            diag.push(diag_entry(&aumid, status_label(status), "ошибка чтения свойств"));
            continue;
        };
        let title = props.Title().map(|t| t.to_string()).unwrap_or_default();
        let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
        if title.is_empty() && artist.is_empty() {
            diag.push(diag_entry(&aumid, status_label(status), "пустые метаданные, отброшена"));
            continue;
        }
        diag.push(diag_entry(&aumid, status_label(status), "ok"));
        let playing = status == Some(Status::Playing);
        let ordinal = out.iter().filter(|a| a.aumid == aumid).count();
        exact.push(cur.is_some_and(|c| same_session(c, &session)));
        out.push(Active { session, props, title, artist, aumid, ordinal, playing, current: false });
    }
    let flags = {
        let aumids: Vec<&str> = out.iter().map(|a| a.aumid.as_str()).collect();
        resolve_current_flags(&exact, &aumids, &cur_aumid)
    };
    for (a, is_cur) in out.iter_mut().zip(flags) {
        a.current = is_cur;
    }
    let summary = media_summary(total, used_current, &diag, &cur_aumid);
    let changed = LAST_MEDIA_SUMMARY.with(|last| summary_changed(&mut last.borrow_mut(), &summary));
    if changed {
        log::info(&summary);
    }
    out
}

thread_local! {
    /// Последняя записанная сводка сессий: пишем в журнал только изменения, чтобы смена трека
    /// (будит поток часто) не засоряла лог одинаковыми строками.
    static LAST_MEDIA_SUMMARY: RefCell<String> = const { RefCell::new(String::new()) };
}

fn status_label(s: Option<Status>) -> &'static str {
    match s {
        Some(x) if x == Status::Playing => "playing",
        Some(x) if x == Status::Paused => "paused",
        Some(x) if x == Status::Stopped => "stopped",
        Some(x) if x == Status::Closed => "closed",
        Some(_) => "other",
        None => "статус неизвестен",
    }
}

fn diag_entry(aumid: &str, status: &str, verdict: &str) -> String {
    let id = if aumid.is_empty() { "(пусто)" } else { aumid };
    format!("{id}: {status}, {verdict}")
}

/// `used_current` — список `GetSessions()` был пуст и взята только `GetCurrentSession()`.
fn media_summary(total: usize, used_current: bool, entries: &[String], cur_aumid: &str) -> String {
    let source = if used_current { ", список пуст — взята текущая" } else { "" };
    let cur = if cur_aumid.is_empty() { "нет" } else { cur_aumid };
    format!("Медиа: сессий {total}{source}, [{}], текущая: {cur}", entries.join("; "))
}

/// `true` и запоминает сводку, если она отличается от прошлой.
fn summary_changed(last: &mut String, new: &str) -> bool {
    if last == new {
        return false;
    }
    last.clear();
    last.push_str(new);
    true
}

/// Сессия, которая реально звучит, — её показывает свёрнутый остров.
fn live_index(active: &[Active]) -> Option<usize> {
    let tracks: Vec<String> = active.iter().map(|a| track_key(&a.title, &a.artist)).collect();
    let refs: Vec<(&str, &str, bool, bool)> = active
        .iter()
        .zip(&tracks)
        .map(|(a, t)| (a.aumid.as_str(), t.as_str(), a.playing, a.current))
        .collect();
    let idx = LAST_PLAYING.with(|last| {
        let (aumid, track) = &*last.borrow();
        pick_session_index_with_track(&refs, aumid, track)
    })?;
    if active[idx].playing && !active[idx].aumid.is_empty() {
        LAST_PLAYING.with(|last| {
            *last.borrow_mut() = (active[idx].aumid.clone(), tracks[idx].clone());
        });
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
    /// Весь SMTC живёт в фоновом потоке, поэтому хватает thread_local, как и у `LAST_PLAYING`.
    static PINNED: RefCell<Option<Pin>> = const { RefCell::new(None) };
    /// Сессия карточки из последнего чтения: команды плеера идут в неё без повторного
    /// чтения метаданных всех сессий (при перетаскивании перемотка шлётся каждые 100 мс).
    static CARD_SESSION: RefCell<Option<GlobalSystemMediaTransportControlsSession>> = const { RefCell::new(None) };
}

/// Есть ли выбор стрелками. Флаг читает поток интерфейса, поэтому он атомарный, а не `thread_local`:
/// сбрасывать выбор на каждую смену вкладки без этой проверки значило бы будить фоновый поток зря.
static PIN_SET: AtomicBool = AtomicBool::new(false);

pub(super) fn pin_set() -> bool {
    PIN_SET.load(Ordering::Relaxed)
}

/// Снимает выбор стрелками: карточка возвращается к сессии, которая реально играет.
pub(super) fn reset_pin() {
    PIN_SET.store(false, Ordering::Relaxed);
    PINNED.with(|p| *p.borrow_mut() = None);
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
            PIN_SET.store(false, Ordering::Relaxed);
        }
        live
    })
}

/// Перелистывает карточку плеера на соседнюю активную сессию. Воспроизведение не трогает:
/// свёрнутый остров продолжает показывать то, что реально играет.
pub(super) unsafe fn cycle_session(delta: isize) {
    let Some((_, list, cur_s)) = fetch_sessions() else { return };
    let active = active_sessions_from(list, cur_s.as_ref());
    if active.len() < 2 {
        return;
    }
    let live = live_index(&active);
    let Some(cur) = card_index(&active, live) else { return };
    let next = &active[cycle_index(cur, active.len(), delta)];
    let pin = Pin { aumid: next.aumid.clone(), ordinal: next.ordinal, track: track_key(&next.title, &next.artist) };
    PINNED.with(|p| *p.borrow_mut() = Some(pin));
    PIN_SET.store(true, Ordering::Relaxed);
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
            match wait(Mgr::RequestAsync()) {
                Ok(mgr) => {
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
                Err(e) => log::error(&format!("Медиа: RequestAsync не удался: {e}")),
            }
        }
        m.clone()
    })
}

unsafe fn reset_media_manager() {
    MEDIA_MGR.with(|m| *m.borrow_mut() = None);
    CARD_SESSION.with(|c| *c.borrow_mut() = None);
    SESSION_WATCH.with(|w| {
        for old in w.borrow_mut().drain(..) {
            let _ = old.session.RemoveMediaPropertiesChanged(old.prop_token);
            let _ = old.session.RemovePlaybackInfoChanged(old.info_token);
            if let Some(t3) = old.timeline_token {
                let _ = old.session.RemoveTimelinePropertiesChanged(t3);
            }
        }
    });
    RESUBSCRIBE.store(true, Ordering::SeqCst);
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
unsafe fn watch_media_with(
    sessions_in: &[GlobalSystemMediaTransportControlsSession],
    cur: Option<&GlobalSystemMediaTransportControlsSession>,
) {
    let mut sessions: Vec<GlobalSystemMediaTransportControlsSession> = sessions_in.to_vec();
    if sessions.is_empty() {
        if let Some(c) = cur {
            sessions.push(c.clone());
        }
    }
    let ids: Vec<String> = sessions.iter().map(aumid_of).collect();
    let resub = RESUBSCRIBE.swap(false, Ordering::SeqCst);
    SESSION_WATCH.with(|w| {
        let mut w = w.borrow_mut();
        let same_set = !resub
            && w.len() == sessions.len()
            && w.iter()
                .zip(sessions.iter().zip(&ids))
                .all(|(old, (s, id))| &old.aumid == id && same_session(&old.session, s));
        if same_set {
            return;
        }
        // Пишется только при смене набора/событии менеджера: видно, подписались ли мы на сессию
        // браузера, когда она появилась.
        let meta_changed = resub || w.len() != ids.len() || w.iter().zip(&ids).any(|(old, id)| &old.aumid != id);
        if meta_changed {
            log::info(&format!("Медиа: подписки пересозданы, сессий {}", sessions.len()));
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum CmdOutcome {
    Ok,
    Rejected,
    Broken,
}

fn is_timeout(e: &windows::core::Error) -> bool {
    e.code() == windows::core::HRESULT::from_win32(1460)
}

unsafe fn refresh_card_session() -> Option<(Mgr, GlobalSystemMediaTransportControlsSession)> {
    CARD_SESSION.with(|c| *c.borrow_mut() = None);
    let (mgr, list, cur) = fetch_sessions()?;
    let active = active_sessions_from(list, cur.as_ref());
    let live = live_index(&active);
    let card = card_index(&active, live);
    if let Some(i) = card {
        let s = active[i].session.clone();
        CARD_SESSION.with(|c| *c.borrow_mut() = Some(s.clone()));
        return Some((mgr, s));
    }
    let s = cur.filter(|c| session_alive(c))?;
    Some((mgr, s))
}

unsafe fn resolve_command_session(force_refresh: bool) -> Option<(Mgr, GlobalSystemMediaTransportControlsSession)> {
    if !force_refresh {
        if let Some(s) = CARD_SESSION.with(|c| c.borrow().clone()) {
            if session_alive(&s) {
                if let Some(mgr) = media_manager() {
                    return Some((mgr, s));
                }
            }
        }
    }
    refresh_card_session()
}

unsafe fn is_current_session(mgr: &Mgr, s: &GlobalSystemMediaTransportControlsSession) -> bool {
    match mgr.GetCurrentSession() {
        Ok(cur) => {
            if same_session(&cur, s) {
                return true;
            }
            let cur_id = aumid_of(&cur);
            let s_id = aumid_of(s);
            if cur_id.is_empty() || cur_id != s_id {
                return false;
            }
            // На случай разных COM-обёрток разрешаем AUMID-совпадение, если под этим AUMID ровно одна сессия.
            mgr.GetSessions()
                .ok()
                .map(|v| v.into_iter().filter(|x| aumid_of(x) == s_id).count() <= 1)
                .unwrap_or(true)
        }
        // Если в Windows нет текущей сессии, глобальная медиа-клавиша не перехватит чужой плеер.
        Err(_) => true,
    }
}

unsafe fn execute_session_cmd(s: &GlobalSystemMediaTransportControlsSession, cmd: &Cmd) -> CmdOutcome {
    match cmd {
        Cmd::PlayPause => {
            let Ok(info) = s.GetPlaybackInfo() else {
                return CmdOutcome::Broken;
            };
            let status_before = info.PlaybackStatus().ok();
            if status_before == Some(Status::Closed) {
                return CmdOutcome::Broken;
            }
            let playing = status_before == Some(Status::Playing);
            let primary = if playing {
                wait_timeout(s.TryPauseAsync(), MEDIA_WAIT_MS)
            } else {
                wait_timeout(s.TryPlayAsync(), MEDIA_WAIT_MS)
            };
            log::info(&format!("медиа PlayPause (playing={playing}): {primary:?}"));
            match primary {
                Ok(true) => return CmdOutcome::Ok,
                Ok(false) => {}
                Err(e) if is_timeout(&e) || !session_alive(s) => return CmdOutcome::Broken,
                Err(_) => {}
            }
            if s.GetPlaybackInfo().ok().and_then(|i| i.PlaybackStatus().ok()) != status_before {
                return CmdOutcome::Ok;
            }
            let toggle = wait_timeout(s.TryTogglePlayPauseAsync(), MEDIA_WAIT_MS);
            log::info(&format!("медиа TryTogglePlayPause: {toggle:?}"));
            match toggle {
                Ok(true) => CmdOutcome::Ok,
                Ok(false) => {
                    if s.GetPlaybackInfo().ok().and_then(|i| i.PlaybackStatus().ok()) != status_before {
                        CmdOutcome::Ok
                    } else {
                        CmdOutcome::Rejected
                    }
                }
                Err(e) if is_timeout(&e) || !session_alive(s) => CmdOutcome::Broken,
                Err(_) => CmdOutcome::Rejected,
            }
        }
        Cmd::Next => {
            let res = wait_timeout(s.TrySkipNextAsync(), MEDIA_WAIT_MS);
            log::info(&format!("медиа Next: {res:?}"));
            match res {
                Ok(true) => CmdOutcome::Ok,
                Ok(false) => CmdOutcome::Rejected,
                Err(_) => CmdOutcome::Broken,
            }
        }
        Cmd::Prev => {
            let res = wait_timeout(s.TrySkipPreviousAsync(), MEDIA_WAIT_MS);
            log::info(&format!("медиа Prev: {res:?}"));
            match res {
                Ok(true) => CmdOutcome::Ok,
                Ok(false) => CmdOutcome::Rejected,
                Err(_) => CmdOutcome::Broken,
            }
        }
        Cmd::Seek(pos) => seek_session(s, *pos),
        _ => CmdOutcome::Ok,
    }
}

unsafe fn send_fallback_media_key(cmd: &Cmd) {
    match cmd {
        Cmd::PlayPause => media_key(0xB3),
        Cmd::Next => media_key(0xB0),
        Cmd::Prev => media_key(0xB1),
        _ => {}
    }
}

/// Команды плеера идут в сессию карточки — ту, что пользователь видит и выбрал стрелками.
pub(super) unsafe fn media_command(cmd: Cmd) {
    let Some((mgr, s)) = resolve_command_session(false) else {
        if media_manager().and_then(|m| m.GetCurrentSession().ok()).is_none() {
            send_fallback_media_key(&cmd);
        }
        return;
    };
    match execute_session_cmd(&s, &cmd) {
        CmdOutcome::Ok => {}
        CmdOutcome::Rejected => {
            if is_current_session(&mgr, &s) {
                send_fallback_media_key(&cmd);
            }
        }
        CmdOutcome::Broken => {
            log::info("медиа: сессия не ответила, обновляю выбор");
            if mgr.GetSessions().is_err() {
                reset_media_manager();
            }
            if let Some((mgr2, s2)) = resolve_command_session(true) {
                if !same_session(&s2, &s) {
                    if execute_session_cmd(&s2, &cmd) != CmdOutcome::Ok && is_current_session(&mgr2, &s2) {
                        send_fallback_media_key(&cmd);
                    }
                    return;
                }
                if is_current_session(&mgr2, &s2) {
                    send_fallback_media_key(&cmd);
                }
            } else if media_manager().and_then(|m| m.GetCurrentSession().ok()).is_none() {
                send_fallback_media_key(&cmd);
            }
        }
    }
}

/// Нажимает системную медиа-клавишу (VK_MEDIA_NEXT_TRACK 0xB0, PREV_TRACK 0xB1, PLAY_PAUSE 0xB3).
unsafe fn media_key(vk: u8) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    let make = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk as u16),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [make(KEYEVENTF_EXTENDEDKEY), make(KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

/// Перематывает трек сессии на позицию в секундах от начала.
/// Флаг `IsPlaybackPositionEnabled` не проверяем: браузеры часто сбрасывают его и обнуляют
/// таймлайн между опросами, хотя перемотку принимают; неподдерживаемый запрос плеер просто отклонит.
unsafe fn seek_session(s: &GlobalSystemMediaTransportControlsSession, target_secs: f32) -> CmdOutcome {
    if !target_secs.is_finite() {
        return CmdOutcome::Ok;
    }
    let Ok(tl) = s.GetTimelineProperties() else {
        return CmdOutcome::Broken;
    };
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
    match wait_timeout(s.TryChangePlaybackPositionAsync(hns), MEDIA_WAIT_MS) {
        Ok(true) => CmdOutcome::Ok,
        Ok(false) => CmdOutcome::Rejected,
        Err(_) => CmdOutcome::Broken,
    }
}

#[derive(Clone)]
enum CoverEntry {
    Ready(Arc<Vec<u8>>),
    Failed(Instant),
}

fn lookup_cover(cache: &[(u64, CoverEntry)], key: u64, now: Instant) -> Option<Option<Arc<Vec<u8>>>> {
    let (_, entry) = cache.iter().find(|(k, _)| *k == key)?;
    match entry {
        CoverEntry::Ready(px) => Some(Some(px.clone())),
        CoverEntry::Failed(until) => (now < *until).then_some(None),
    }
}

fn store_cover(cache: &mut Vec<(u64, CoverEntry)>, key: u64, entry: CoverEntry) {
    cache.retain(|(k, _)| *k != key);
    if cache.len() >= COVER_CACHE {
        cache.remove(0);
    }
    cache.push((key, entry));
}

/// Обложки последних треков: повторно не декодируем, пока трек тот же. Несколько записей,
/// потому что карточка и свёрнутый остров могут показывать разные сессии одновременно.
static COVER: Mutex<Vec<(u64, CoverEntry)>> = Mutex::new(Vec::new());
const COVER_CACHE: usize = 8;

/// `(сессия карточки плеера, сессия, которая реально играет)`; часто это одна и та же.
pub(super) unsafe fn read_media_pair() -> (Option<Media>, Option<Media>) {
    let Some((_, list, cur)) = fetch_sessions() else { return (None, None) };
    watch_media_with(&list, cur.as_ref());
    let active = active_sessions_from(list, cur.as_ref());
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
    let now = Instant::now();
    let cached = COVER.lock().ok().and_then(|c| lookup_cover(&c, key, now));
    let art = match cached {
        Some(px) => px,
        None => {
            let read = read_cover(props).map(Arc::new);
            let entry = match &read {
                Some(px) => CoverEntry::Ready(px.clone()),
                None => CoverEntry::Failed(now + COVER_FAIL_COOLDOWN),
            };
            if let Ok(mut c) = COVER.lock() {
                store_cover(&mut c, key, entry);
            }
            read
        }
    };
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
    let stream = wait_timeout(props.Thumbnail().ok()?.OpenReadAsync(), MEDIA_WAIT_MS).ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 16 << 20 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    wait_timeout(reader.LoadAsync(size), MEDIA_WAIT_MS).ok()?;
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
    fn reset_pin_clears_selection_and_flag() {
        PINNED.with(|p| *p.borrow_mut() = Some(Pin { aumid: "a".into(), ordinal: 0, track: "t".into() }));
        PIN_SET.store(true, Ordering::Relaxed);
        reset_pin();
        assert!(!pin_set());
        assert!(PINNED.with(|p| p.borrow().is_none()));
    }

    #[test]
    fn media_summary_format_and_dedup() {
        let entries = vec![
            diag_entry("Yandex.Browser", "playing", "ok"),
            diag_entry("", "paused", "пустые метаданные, отброшена"),
        ];
        let s = media_summary(2, false, &entries, "Yandex.Browser");
        assert_eq!(
            s,
            "Медиа: сессий 2, [Yandex.Browser: playing, ok; (пусто): paused, пустые метаданные, отброшена], текущая: Yandex.Browser"
        );
        assert!(media_summary(1, true, &[], "").contains("список пуст — взята текущая"));
        let mut last = String::new();
        assert!(summary_changed(&mut last, &s));
        assert!(!summary_changed(&mut last, &s));
        assert!(summary_changed(&mut last, "другая"));
    }

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

    #[test]
    fn test_pick_session_index_with_track_keeps_paused_browser_tab() {
        let sessions = [
            ("Chrome", "Old video\u{1f}Channel A", false, false),
            ("Chrome", "YouTube Track\u{1f}Artist B", false, false),
        ];
        assert_eq!(
            pick_session_index_with_track(&sessions, "Chrome", "YouTube Track\u{1f}Artist B"),
            Some(1)
        );
    }

    #[test]
    fn test_resolve_current_flags_prefers_exact_match_over_aumid() {
        assert_eq!(
            resolve_current_flags(&[false, true], &["Chrome", "Chrome"], "Chrome"),
            vec![false, true]
        );
        assert_eq!(
            resolve_current_flags(&[false, false], &["Chrome", "Spotify.exe"], "Chrome"),
            vec![true, false]
        );
        assert_eq!(
            resolve_current_flags(&[false, false], &["Chrome", "Spotify.exe"], ""),
            vec![false, false]
        );
    }

    #[test]
    fn test_cover_cache_ready_and_failure_cooldown() {
        let mut cache = Vec::new();
        let t0 = Instant::now();
        assert!(lookup_cover(&cache, 1, t0).is_none());

        store_cover(&mut cache, 1, CoverEntry::Failed(t0 + Duration::from_secs(10)));
        assert!(matches!(lookup_cover(&cache, 1, t0 + Duration::from_secs(2)), Some(None)));
        assert!(lookup_cover(&cache, 1, t0 + Duration::from_secs(11)).is_none());

        let px = Arc::new(vec![1, 2, 3, 4]);
        store_cover(&mut cache, 1, CoverEntry::Ready(px.clone()));
        assert!(matches!(lookup_cover(&cache, 1, t0 + Duration::from_secs(12)), Some(Some(v)) if v == px));

        for k in 2..=10u64 {
            store_cover(&mut cache, k, CoverEntry::Ready(px.clone()));
        }
        assert_eq!(cache.len(), COVER_CACHE);
    }
}

