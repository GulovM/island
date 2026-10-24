//! Остров как индикатор (этап 4): в свёрнутой форме показывает мини-плеер, пока играет музыка,
//! и короткие уведомления (таймер, громкость, зарядка). Формы — это только целевые размеры тех же
//! пружин ширины/высоты, что и у раскрытия, поэтому «чёлка» ↔ мини-плеер ↔ панель переходят друг в
//! друга одной цепочкой без отдельной анимации. Никаких своих таймеров в простое: данные приходят
//! событиями (медиасессия, громкость, питание), секундный тик — только пока виден играющий плеер.

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::anim::Spring;
use crate::bg::{self, Media, Snapshot};
use crate::ui::{dim, icon, status_green, status_orange, status_red, txt, Canvas, Rect, Ui, WHITE};

/// Размеры форм (логические пиксели). Ширина не меньше ширины свёрнутого острова.
pub const MINI: (f32, f32) = (252.0, 36.0);
pub const TOAST: (f32, f32) = (232.0, 36.0);
/// Заряд, ниже которого (и без питания от сети) показывается предупреждение.
const LOW_PCT: u8 = 20;

/// Что свёрнутый остров знает о запущенном (или стоящем на паузе) таймере вкладки Today.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimerView {
    /// Сколько осталось от полного времени, 0..1.
    pub frac: f32,
    /// Осталось секунд (округление вверх, как на карточке).
    pub secs: u64,
    pub paused: bool,
    /// Через сколько миллисекунд сменится секунда на экране.
    pub next_ms: u32,
}

impl TimerView {
    /// Остаток: «03:25» или «1:02:05».
    pub fn label(&self) -> String {
        let s = self.secs;
        if s >= 3600 {
            format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
        } else {
            format!("{:02}:{:02}", s / 60, s % 60)
        }
    }
}

/// Период мигания полоски на паузе: не быстрее раза в 2 секунды.
const PULSE_SECS: f32 = 2.4;

/// Какую форму принимает свёрнутый остров.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Idle,
    Mini,
    Toast,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Toast {
    Timer,
    Volume { level: f32, muted: bool },
    Brightness { level: f32 },
    Charging { on: bool, percent: u8 },
    LowBattery(u8),
}

impl Toast {
    /// Сколько уведомление остаётся на экране.
    pub fn lifetime(&self) -> Duration {
        Duration::from_millis(match self {
            Toast::Timer => 6000,
            Toast::Volume { .. } | Toast::Brightness { .. } => 1700,
            Toast::Charging { .. } => 3000,
            Toast::LowBattery(_) => 5000,
        })
    }
}

/// Решает, нужно ли уведомление при изменении питания. Возвращает (уведомление, «низкий заряд уже
/// объявлен»). Не завязано на системные вызовы — проверяется тестами.
pub fn power_transition(prev_ac: Option<bool>, low: bool, ac: bool, percent: u8) -> (Option<Toast>, bool) {
    if prev_ac.is_some_and(|p| p != ac) {
        return (Some(Toast::Charging { on: ac, percent }), if ac { false } else { low });
    }
    if !ac && percent <= LOW_PCT && !low {
        return (Some(Toast::LowBattery(percent)), true);
    }
    // Гистерезис: флаг сбрасывается, когда заряд заметно выше порога или подключили сеть.
    (None, if ac || percent > LOW_PCT + 5 { false } else { low })
}

pub struct Live {
    media: Option<Media>,
    media_enabled: bool,
    toast: Option<Toast>,
    /// Последнее показанное уведомление: рисуется, пока гаснет.
    shown: Option<Toast>,
    mini_a: Spring,
    toast_a: Spring,
    rev: u64,
    ac: Option<bool>,
    low: bool,
    volume: Option<(f32, bool)>,
    brightness: Option<f32>,
    /// Таймер Today для свёрнутой формы и плавная доля полоски.
    timer: Option<TimerView>,
    timer_frac: Spring,
    /// Начало отсчёта анимации эквалайзера.
    t0: Instant,
}

impl Live {
    pub fn new(media_enabled: bool) -> Self {
        let mut l = Live {
            media: None,
            media_enabled,
            toast: None,
            shown: None,
            mini_a: Spring::unit(0.0, 260.0, 30.0),
            toast_a: Spring::unit(0.0, 260.0, 30.0),
            rev: 0,
            ac: None,
            low: false,
            volume: None,
            brightness: None,
            timer: None,
            timer_frac: Spring::unit(1.0, 160.0, 22.0),
            t0: Instant::now(),
        };
        // Начальное состояние питания запоминаем без уведомления.
        l.power_event();
        l
    }

    pub fn set_media_enabled(&mut self, on: bool) {
        self.media_enabled = on;
    }

    /// Какая форма нужна, если остров свёрнут и ничто его не закрывает.
    pub fn form(&self) -> Form {
        if self.toast.is_some() {
            Form::Toast
        } else if self.playing() {
            Form::Mini
        } else {
            Form::Idle
        }
    }

    pub fn timer(&self) -> Option<TimerView> {
        self.timer
    }

    /// Новое состояние таймера. Возвращает `(изменилось видимое, появился или пропал)`.
    pub fn set_timer(&mut self, t: Option<TimerView>) -> (bool, bool) {
        let appeared = self.timer.is_some() != t.is_some();
        // Миллисекунды до смены секунды видимого не меняют.
        let changed = appeared || self.timer.map(|o| (o.secs, o.paused)) != t.map(|n| (n.secs, n.paused));
        if let Some(n) = t {
            self.timer_frac.target = n.frac.clamp(0.0, 1.0);
            if self.timer.is_none() {
                self.timer_frac.snap();
            }
        }
        self.timer = t;
        (changed, appeared)
    }

    /// Плавная доля оставшегося времени для полоски.
    pub fn timer_frac(&self) -> f32 {
        self.timer_frac.value.clamp(0.0, 1.0)
    }

    /// Прозрачность полоски таймера: на паузе плавно мигает, иначе 1.
    pub fn timer_pulse(&self) -> f32 {
        match self.timer {
            Some(t) if t.paused => {
                let k = 0.5 + 0.5 * (self.t0.elapsed().as_secs_f32() * std::f32::consts::TAU / PULSE_SECS).sin();
                0.3 + 0.7 * k
            }
            _ => 1.0,
        }
    }

    pub fn playing(&self) -> bool {
        self.media_enabled && self.media.as_ref().is_some_and(|m| m.playing)
    }

    /// Забирает медиа из снимка фонового потока. `true` — изменилось то, что видно в мини-плеере.
    pub fn sync_media(&mut self, snap: &Snapshot) -> bool {
        if snap.rev == self.rev {
            return false;
        }
        self.rev = snap.rev;
        let old = self.media.take();
        let mut new = snap.media.clone();
        // Позиция трека у приложений приходит неточной: пока трек тот же, остаёмся на своём отсчёте.
        if let (Some(n), Some(o)) = (&mut new, &old) {
            if n.key == o.key && n.playing && o.playing && n.rate == o.rate && (n.position_now() - o.position_now()).abs() < 2.0 {
                n.pos = o.pos;
                n.at = o.at;
            }
        }
        let changed = match (&old, &new) {
            (None, None) => false,
            (Some(a), Some(b)) => a.key != b.key || a.playing != b.playing || a.art.is_some() != b.art.is_some(),
            _ => true,
        };
        self.media = new;
        changed
    }

    pub fn show(&mut self, t: Toast) {
        self.toast = Some(t);
        self.shown = Some(t);
    }

    pub fn clear_toast(&mut self) -> bool {
        self.toast.take().is_some()
    }

    /// Громкость изменилась по сравнению с прошлым известным значением.
    pub fn volume_changed(&mut self, level: f32, muted: bool) -> bool {
        let changed = match self.volume {
            None => true,
            Some((l, m)) => m != muted || (l - level).abs() > 0.004,
        };
        self.volume = Some((level, muted));
        changed
    }

    /// Яркость изменилась по сравнению с прошлой известной. Первое чтение — только запоминается:
    /// при запуске приложения показывать нечего.
    pub fn brightness_changed(&mut self, level: f32) -> bool {
        let changed = self.brightness.is_some_and(|l| (l - level).abs() > 0.004);
        self.brightness = Some(level);
        changed
    }

    /// Читает состояние питания и возвращает уведомление, если оно изменилось. Настольный ПК без
    /// батареи уведомлений не даёт.
    pub fn power_event(&mut self) -> Option<Toast> {
        let mut s = SYSTEM_POWER_STATUS::default();
        unsafe {
            if GetSystemPowerStatus(&mut s).is_err() {
                return None;
            }
        }
        // BatteryFlag: 128 — батареи нет, 255 — неизвестно; процент 255 — неизвестен.
        if s.BatteryFlag == 255 || s.BatteryFlag & 128 != 0 || s.BatteryLifePercent > 100 || s.ACLineStatus > 1 {
            return None;
        }
        let ac = s.ACLineStatus == 1;
        let (toast, low) = power_transition(self.ac, self.low, ac, s.BatteryLifePercent);
        self.ac = Some(ac);
        self.low = low;
        toast
    }

    /// Выставляет целевую прозрачность содержимого форм.
    pub fn set_form(&mut self, form: Form) {
        self.mini_a.target = if form == Form::Mini { 1.0 } else { 0.0 };
        self.toast_a.target = if form == Form::Toast { 1.0 } else { 0.0 };
    }

    pub fn step(&mut self, dt: f32) {
        self.mini_a.step(dt);
        self.toast_a.step(dt);
        self.timer_frac.step(dt);
        if self.timer_frac.settled() {
            self.timer_frac.snap();
        }
        if self.mini_a.settled() {
            self.mini_a.snap();
        }
        if self.toast_a.settled() {
            self.toast_a.snap();
            if self.toast.is_none() && self.toast_a.value == 0.0 {
                self.shown = None;
            }
        }
    }

    pub fn animating(&self) -> bool {
        !(self.mini_a.settled() && self.toast_a.settled() && self.timer_frac.settled())
    }

    /// Тонкая линия таймера под прогрессом трека в мини-плеере.
    pub unsafe fn draw_timer_line(&self, cv: &Canvas, r: Rect) {
        if self.timer.is_none() {
            return;
        }
        let line = Canvas { g: cv.g, alpha: cv.alpha * self.timer_pulse(), accent: cv.accent };
        let track_r = Rect::new(r.x + 14.0, r.bottom() - 3.0, r.w - 28.0, 1.5);
        line.rr(Rect::new(track_r.x, track_r.y, track_r.w * self.timer_frac(), track_r.h), 0.75, status_orange());
    }

    pub fn mini_alpha(&self) -> f32 {
        self.mini_a.value.clamp(0.0, 1.0)
    }

    pub fn toast_alpha(&self) -> f32 {
        self.toast_a.value.clamp(0.0, 1.0)
    }

    // ---------- Отрисовка ----------

    /// Мини-плеер: обложка, название, исполнитель, «эквалайзер» и тонкий прогресс внизу.
    pub unsafe fn draw_mini(&self, ui: &mut Ui, cv: &Canvas, r: Rect) {
        let Some(m) = &self.media else { return };
        let f = &cv.g.fonts;
        let cover = Rect::new(r.x + 12.0, r.y + 6.0, 22.0, 22.0);
        let mut has_cover = false;
        if let Some(px) = &m.art {
            // Тот же ключ, что у плеера на вкладке Controls: битмап общий.
            let key = (0x4Du64 << 56) | (m.key >> 8);
            if let Some(bmp) = ui.cache.get(&cv.g.dc, key, bg::ART, bg::ART, || px.to_vec()) {
                cv.bitmap_quad(Some(&bmp), cover, 0.0, 1.0);
                has_cover = true;
            }
        }
        if !has_cover {
            let top = D2D1_COLOR_F { r: 0.35, g: 0.22, b: 0.47, a: 1.0 };
            let bottom = D2D1_COLOR_F { r: 0.76, g: 0.42, b: 0.60, a: 1.0 };
            cv.gradient_rr(cover, 5.0, top, bottom);
            cv.glyph(icon::MUSIC, cover, false, D2D1_COLOR_F { a: 0.7, ..WHITE });
        }
        let tx = cover.right() + 8.0;
        let tw = (r.right() - 36.0 - tx).max(10.0);
        cv.text(&m.title, Rect::new(tx, r.y + 4.0, tw, 14.0), &f.small_b, txt());
        cv.text(&m.artist, Rect::new(tx, r.y + 16.0, tw, 12.0), &f.small, dim());
        // «Эквалайзер»: столбики плавно качаются по своим синусам (не под музыку). Кадры идут
        // от таймера мини-плеера (~30 в секунду) и только пока он виден и музыка играет.
        let t = self.t0.elapsed().as_secs_f32();
        let bx = r.right() - 27.0;
        for (i, (a, b, p)) in [(5.1f32, 8.3f32, 0.0f32), (6.7, 4.9, 2.1), (4.3, 7.7, 4.2)].iter().enumerate() {
            let k = 0.5 + 0.5 * (0.6 * (t * a + p).sin() + 0.4 * (t * b + 1.3 * p).sin());
            let h = 3.0 + 10.0 * k;
            cv.rr(Rect::new(bx + i as f32 * 5.0, r.y + 17.0 - h / 2.0, 3.0, h), 1.5, cv.accent);
        }
        let frac = if m.dur > 0.0 { m.position_now() / m.dur } else { 0.0 };
        // Прогресс трека поднят на 1 px, чтобы под ним поместилась линия таймера.
        cv.progress(Rect::new(r.x + 14.0, r.bottom() - 7.0, r.w - 28.0, 2.0), frac, cv.accent);
        self.draw_timer_line(cv, r);
    }

    /// Уведомление: значок слева, текст и значение справа.
    pub unsafe fn draw_toast(&self, cv: &Canvas, r: Rect) {
        let Some(t) = self.shown else { return };
        let f = &cv.g.fonts;
        let cy = r.y + r.h / 2.0 - 1.0;
        let ic = Rect::new(r.x + 12.0, cy - 9.0, 18.0, 18.0);
        let tx = r.x + 38.0;
        let right = r.right() - 14.0;
        let value = |s: &str| cv.text(s, Rect::new(right - 40.0, cy - 8.0, 40.0, 16.0), &f.body_r, txt());
        match t {
            Toast::Timer => {
                cv.glyph(icon::CLOCK, ic, false, cv.accent);
                cv.text("Таймер завершён", Rect::new(tx, cy - 8.0, right - tx, 16.0), &f.body, txt());
            }
            Toast::Volume { level, muted } => {
                let g = if muted || level <= 0.0 {
                    icon::MUTE
                } else if level < 0.34 {
                    icon::VOLUME1
                } else if level < 0.67 {
                    icon::VOLUME2
                } else {
                    icon::VOLUME3
                };
                cv.glyph(g, ic, false, txt());
                value(&if muted { "Выкл".to_string() } else { format!("{}%", (level * 100.0).round() as i32) });
                cv.progress(Rect::new(tx, cy - 1.5, right - 46.0 - tx, 3.0), if muted { 0.0 } else { level }, cv.accent);
            }
            Toast::Brightness { level } => {
                cv.glyph(icon::BRIGHTNESS, ic, false, txt());
                value(&format!("{}%", (level * 100.0).round() as i32));
                cv.progress(Rect::new(tx, cy - 1.5, right - 46.0 - tx, 3.0), level, cv.accent);
            }
            Toast::Charging { on, percent } => {
                self.battery(cv, ic, percent, if on { status_green() } else { txt() });
                cv.text(if on { "Зарядка" } else { "От батареи" }, Rect::new(tx, cy - 8.0, right - 44.0 - tx, 16.0), &f.body, txt());
                value(&format!("{percent}%"));
            }
            Toast::LowBattery(percent) => {
                self.battery(cv, ic, percent, status_red());
                cv.text("Низкий заряд", Rect::new(tx, cy - 8.0, right - 44.0 - tx, 16.0), &f.body, status_orange());
                value(&format!("{percent}%"));
            }
        }
    }

    /// Батарейка, нарисованная примитивами (глифов зарядки в шрифте может не быть).
    unsafe fn battery(&self, cv: &Canvas, r: Rect, percent: u8, fill: D2D1_COLOR_F) {
        let body = Rect::new(r.x, r.y + 4.5, 15.0, 9.0);
        cv.rr_line(body, 2.5, dim(), 1.2);
        cv.rr(Rect::new(body.right() + 0.8, body.y + 2.5, 1.8, 4.0), 0.9, dim());
        let w = 11.0 * (percent.min(100) as f32 / 100.0);
        if w > 0.5 {
            cv.rr(Rect::new(body.x + 2.0, body.y + 2.0, w, 5.0), 1.0, fill);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(playing: bool, key: u64, rev: u64) -> Snapshot {
        Snapshot {
            rev,
            media: Some(Media {
                title: "t".into(),
                artist: "a".into(),
                playing,
                pos: 1.0,
                dur: 100.0,
                at: std::time::Instant::now(),
                rate: 1.0,
                art: None,
                key,
                can_seek: true,
                min_seek: 0.0,
                max_seek: 100.0,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn mini_only_while_playing() {
        let mut l = Live::new(true);
        assert_eq!(l.form(), Form::Idle);
        assert!(l.sync_media(&media(true, 1, 1)));
        assert_eq!(l.form(), Form::Mini);
        assert!(l.sync_media(&media(false, 1, 2)));
        assert_eq!(l.form(), Form::Idle);
    }

    #[test]
    fn same_revision_is_ignored() {
        let mut l = Live::new(true);
        assert!(l.sync_media(&media(true, 1, 1)));
        assert!(!l.sync_media(&media(true, 1, 1)));
    }

    #[test]
    fn disabled_media_stays_idle() {
        let mut l = Live::new(false);
        l.sync_media(&media(true, 1, 1));
        assert_eq!(l.form(), Form::Idle);
        l.set_media_enabled(true);
        assert_eq!(l.form(), Form::Mini);
    }

    #[test]
    fn toast_beats_mini_and_expires() {
        let mut l = Live::new(true);
        l.sync_media(&media(true, 1, 1));
        l.show(Toast::Timer);
        assert_eq!(l.form(), Form::Toast);
        assert!(l.clear_toast());
        assert!(!l.clear_toast());
        assert_eq!(l.form(), Form::Mini);
    }

    #[test]
    fn shown_toast_fades_before_it_is_dropped() {
        let mut l = Live::new(true);
        l.show(Toast::Timer);
        l.set_form(Form::Toast);
        for _ in 0..120 {
            l.step(1.0 / 60.0);
        }
        assert!((l.toast_alpha() - 1.0).abs() < 0.01);
        l.clear_toast();
        l.set_form(Form::Idle);
        l.step(1.0 / 60.0);
        assert!(l.shown.is_some(), "содержимое нужно, пока уведомление гаснет");
        for _ in 0..120 {
            l.step(1.0 / 60.0);
        }
        assert!(l.shown.is_none());
        assert!(!l.animating());
    }

    #[test]
    fn volume_change_detection() {
        let mut l = Live::new(true);
        assert!(l.volume_changed(0.5, false));
        assert!(!l.volume_changed(0.5, false));
        assert!(l.volume_changed(0.6, false));
        assert!(l.volume_changed(0.6, true));
    }

    #[test]
    fn power_transitions() {
        // Подключили зарядку.
        assert_eq!(power_transition(Some(false), false, true, 50), (Some(Toast::Charging { on: true, percent: 50 }), false));
        // Отключили.
        assert_eq!(power_transition(Some(true), false, false, 80), (Some(Toast::Charging { on: false, percent: 80 }), false));
        // Первое чтение — без уведомления.
        assert_eq!(power_transition(None, false, true, 100).0, None);
        // Низкий заряд объявляется один раз.
        let (t, low) = power_transition(Some(false), false, false, 19);
        assert_eq!(t, Some(Toast::LowBattery(19)));
        assert_eq!(power_transition(Some(false), low, false, 18).0, None);
        // Гистерезис: чуть выше порога флаг держится, заметно выше — сбрасывается.
        assert!(power_transition(Some(false), true, false, 22).1);
        assert!(!power_transition(Some(false), true, false, 40).1);
        // Сеть подключена — предупреждения нет.
        assert_eq!(power_transition(Some(true), false, true, 10).0, None);
    }

    #[test]
    fn toast_lifetimes_are_sane() {
        assert!(Toast::Volume { level: 0.5, muted: false }.lifetime() < Toast::Timer.lifetime());
    }
}
