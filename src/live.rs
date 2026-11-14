//! Остров как индикатор (этап 4): в свёрнутой форме отслеживает воспроизведение музыки (для волны
//! эквалайзера в `Face`) и короткие уведомления (таймер, громкость, зарядка). Никаких своих таймеров
//! в простое: данные приходят событиями (медиасессия, громкость, питание).

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::anim::{Spring, FADE_SPRING};
use crate::bg::{Media, Snapshot, NOTIF_ICON_PX};
use crate::tabs::notifications::PopupNotice;
use crate::ui::{collapsed_chip_bg, collapsed_dim, collapsed_txt, icon, status_green, status_orange, status_red, Canvas, Rect, Ui, WHITE};

/// Размеры формы уведомления (логические пиксели). Ширина не меньше ширины свёрнутого острова.
pub const TOAST: (f32, f32) = (232.0, 36.0);
/// Плавающая капсула-островок для всплывающего уведомления (приложение + текст).
pub const TOAST_NOTICE: (f32, f32) = (316.0, 50.0);
/// Отступ плавающей капсулы уведомления от верхнего края экрана (логические пиксели).
pub const CAPSULE_TOP_Y: f32 = 8.0;
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
    Toast,
    /// Плашка «Быстрый доступ», пока на остров тащат файл, ссылку или текст.
    Drop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Toast {
    Timer,
    /// Наступило событие календаря; название и подпись лежат в `Live::event`.
    Event,
    /// Входящее системное уведомление; заголовок, текст и иконка в `Live::notice`.
    Notice,
    Volume { level: f32, muted: bool },
    Brightness { level: f32 },
    Charging { on: bool, percent: u8 },
    LowBattery(u8),
}

impl Toast {
    /// Заметное уведомление: остров ради него выходит из полоски, не раскрывается от наведения
    /// и закрывается по ×, Esc или таймауту.
    pub fn is_alert(&self) -> bool {
        matches!(self, Toast::Timer | Toast::Event)
    }

    /// Уведомление, которое должно показаться даже если чёлка свёрнута в полоску.
    pub fn emerges_from_strip(&self) -> bool {
        matches!(self, Toast::Timer | Toast::Event | Toast::Notice)
    }

    /// Нужно ли передавать клавиатурный фокус острову (только для сигнала таймера).
    pub fn steals_focus(&self) -> bool {
        matches!(self, Toast::Timer)
    }

    /// Относится к настройке «Всплывающие уведомления».
    pub fn is_popup_notif(&self) -> bool {
        matches!(self, Toast::Notice | Toast::Event)
    }

    /// Показывается в виде отдельной плавающей капсулы-островка, отрываясь от верхней кромки.
    pub fn is_capsule(&self) -> bool {
        matches!(self, Toast::Notice | Toast::Event)
    }

    /// Сколько уведомление остаётся на экране.
    pub fn lifetime(&self) -> Duration {
        Duration::from_millis(match self {
            // Минимум; пока звучит сигнал, остров ждёт его конца (см. `ToastTimer`).
            Toast::Timer => 4000,
            Toast::Event => 6000,
            Toast::Notice => 3000,
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
    /// Текст уведомления о событии календаря.
    event: Option<(String, String)>,
    /// Входящее системное уведомление для всплывающего баннера.
    notice: Option<PopupNotice>,
    wave_a: Spring,
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
            event: None,
            notice: None,
            wave_a: Spring::unit(0.0, FADE_SPRING.0, FADE_SPRING.1),
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
        // Свёрнутый остров показывает то, что звучит, а не источник, выбранный стрелками в карточке.
        let mut new = snap.live_media.clone();
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

    /// Уведомление о событии календаря: название и короткая подпись («Сейчас», «10 мин»).
    pub fn show_event(&mut self, title: String, tag: String) {
        self.event = Some((title, tag));
        self.show(Toast::Event);
    }

    /// Входящее системное уведомление в чёлке (на 1,5 секунды, без перехвата фокуса).
    pub fn show_notice(&mut self, notice: PopupNotice) {
        self.notice = Some(notice);
        self.show(Toast::Notice);
    }

    /// Показано заметное уведомление (окончание таймера, событие календаря): не раскрывается от наведения
    /// и закрывается по ×, Esc или таймауту.
    pub fn alert(&self) -> bool {
        self.toast.is_some_and(|t| t.is_alert())
    }

    /// Остров должен выехать из полоски ради текущего тоста.
    pub fn emerges_from_strip(&self) -> bool {
        self.toast.is_some_and(|t| t.emerges_from_strip())
    }

    /// Нужен ли клавиатурный фокус для активного тоста.
    pub fn steals_focus(&self) -> bool {
        self.toast.is_some_and(|t| t.steals_focus())
    }

    /// Остров должен плавно перейти в плавающую капсулу ради активного уведомления.
    pub fn capsule_active(&self) -> bool {
        self.toast.is_some_and(|t| t.is_capsule())
    }

    /// Текущий (или гаснущий) тип тоста для выбора габаритов свёрнутой чёлки.
    pub fn display_toast(&self) -> Option<Toast> {
        self.toast.or(self.shown)
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
        let b = crate::sys::battery_status()?;
        let (toast, low) = power_transition(self.ac, self.low, b.ac_online, b.percent);
        self.ac = Some(b.ac_online);
        self.low = low;
        toast
    }

    /// Выставляет целевую прозрачность содержимого форм.
    pub fn set_form(&mut self, form: Form) {
        self.toast_a.target = if form == Form::Toast { 1.0 } else { 0.0 };
        if form != Form::Idle {
            self.wave_a.target = 0.0;
        }
    }

    /// Выставляет целевую прозрачность волны эквалайзера в свёрнутой чёлке.
    pub fn set_wave_active(&mut self, active: bool) {
        self.wave_a.target = if active { 1.0 } else { 0.0 };
    }

    pub fn wave_alpha(&self) -> f32 {
        self.wave_a.value.clamp(0.0, 1.0)
    }

    pub fn wave_phase(&self) -> f32 {
        self.t0.elapsed().as_secs_f32()
    }

    pub fn step(&mut self, dt: f32) {
        self.wave_a.step(dt);
        self.toast_a.step(dt);
        self.timer_frac.step(dt);
        if self.timer_frac.settled() {
            self.timer_frac.snap();
        }
        if self.wave_a.settled() {
            self.wave_a.snap();
        }
        if self.toast_a.settled() {
            self.toast_a.snap();
            if self.toast.is_none() && self.toast_a.value == 0.0 {
                self.shown = None;
            }
        }
    }

    pub fn animating(&self) -> bool {
        !(self.wave_a.settled() && self.toast_a.settled() && self.timer_frac.settled())
    }

    pub fn toast_alpha(&self) -> f32 {
        self.toast_a.value.clamp(0.0, 1.0)
    }

    // ---------- Отрисовка ----------

    /// Кнопка × в уведомлении о таймере (логические пиксели, `r` — прямоугольник уведомления).
    pub fn close_rect(r: Rect) -> Rect {
        Rect::new(r.right() - 32.0, r.cy() - 10.0, 20.0, 20.0)
    }

    /// Уведомление: значок слева, текст и значение справа. `cursor` — для подсветки ×.
    pub unsafe fn draw_toast(&self, ui: &mut Ui, cv: &Canvas, r: Rect, cursor: (f32, f32)) {
        let Some(t) = self.shown else { return };
        let f = &cv.g.fonts;
        let cy = r.y + r.h / 2.0 - 0.5;
        let pad_x = if t.is_capsule() { 15.0 } else { 12.0 };
        let ic = Rect::new(r.x + pad_x, cy - 9.0, 18.0, 18.0);
        let tx = r.x + pad_x + 26.0;
        let right = r.right() - if t.is_capsule() { 16.0 } else { 14.0 };
        let value = |s: &str| cv.text(s, Rect::new(right - 40.0, cy - 8.0, 40.0, 16.0), &f.body_r, collapsed_txt());
        match t {
            Toast::Notice => {
                if let Some(n) = &self.notice {
                    let ir = Rect::new(r.x + 12.0, r.cy() - 15.0, 30.0, 30.0);
                    let tile_rad = if n.is_calendar { ir.h / 2.0 } else { 10.0 };
                    let mut painted_icon = false;
                    if let Some(px) = &n.icon {
                        let key = crate::icons::bitmap_key(b'N', &n.app_id, &n.app_name);
                        if let Some(bmp) = ui.cache.get(&cv.g.dc, key, NOTIF_ICON_PX, NOTIF_ICON_PX, || px.as_ref().clone()) {
                            cv.rr(ir, tile_rad, D2D1_COLOR_F { a: 0.14, ..WHITE });
                            cv.bitmap(&bmp, ir.inset(3.0), 1.0);
                            painted_icon = true;
                        }
                    }
                    if !painted_icon {
                        cv.rr(ir, tile_rad, D2D1_COLOR_F { a: 0.24, ..cv.accent });
                        if n.is_calendar {
                            cv.symbol(icon::CALENDAR, ir, 14.0, cv.accent);
                        } else {
                            let ch = n.app_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "W".into());
                            cv.text(&ch, ir, &f.body_b, WHITE);
                        }
                    }

                    let ntx = ir.right() + 10.0;
                    let ntw = (r.right() - 18.0 - ntx).max(40.0);
                    cv.circle(ntx + 3.0, r.cy() - 9.0, 2.5, cv.accent);
                    cv.text(&n.app_name, Rect::new(ntx + 9.0, r.cy() - 16.0, (ntw - 9.0).max(20.0), 14.0), &f.small_b, collapsed_dim());

                    let clean_title = n.title.replace(['\r', '\n'], " ");
                    let clean_body = n.body.replace(['\r', '\n'], " ");
                    let msg = if clean_title.is_empty() {
                        clean_body
                    } else if clean_body.is_empty() {
                        clean_title
                    } else {
                        format!("{clean_title} — {clean_body}")
                    };
                    let clip_msg = cv.wrap_clip(&msg, &f.body, ntw, 1);
                    cv.text(&clip_msg, Rect::new(ntx, r.cy() + 1.0, ntw, 17.0), &f.body, collapsed_txt());
                }
            }
            Toast::Timer => {
                cv.glyph(icon::CLOCK, ic, false, cv.accent);
                cv.text(&crate::i18n::tr("Таймер завершён"), Rect::new(tx, cy - 8.0, Self::close_rect(r).x - tx, 16.0), &f.body, collapsed_txt());
                let x = Self::close_rect(r);
                let hot = cursor.0 >= x.x - 4.0 && cursor.0 < x.right() + 4.0 && cursor.1 >= x.y - 4.0 && cursor.1 < x.bottom() + 4.0;
                cv.circle(x.cx(), x.cy(), 9.0, crate::ui::mix(collapsed_chip_bg(), collapsed_txt(), if hot { 0.18 } else { 0.0 }));
                cv.glyph(icon::CLOSE, x.inset(4.0), false, if hot { collapsed_txt() } else { collapsed_dim() });
            }
            Toast::Event => {
                let (title, tag) = self.event.as_ref().map_or(("", ""), |(a, b)| (a.as_str(), b.as_str()));
                cv.glyph(icon::CALENDAR, ic, false, cv.accent);
                let x = Self::close_rect(r);
                let tag_w = if tag.is_empty() { 0.0 } else { 44.0 };
                cv.text(title, Rect::new(tx, cy - 8.0, x.x - tx - tag_w - 4.0, 16.0), &f.body, collapsed_txt());
                if tag_w > 0.0 {
                    cv.text(tag, Rect::new(x.x - tag_w, cy - 8.0, tag_w, 16.0), &f.small, collapsed_dim());
                }
                let hot = cursor.0 >= x.x - 4.0 && cursor.0 < x.right() + 4.0 && cursor.1 >= x.y - 4.0 && cursor.1 < x.bottom() + 4.0;
                cv.circle(x.cx(), x.cy(), 9.0, crate::ui::mix(collapsed_chip_bg(), collapsed_txt(), if hot { 0.18 } else { 0.0 }));
                cv.glyph(icon::CLOSE, x.inset(4.0), false, if hot { collapsed_txt() } else { collapsed_dim() });
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
                cv.glyph(g, ic, false, collapsed_txt());
                value(&if muted { "Выкл".to_string() } else { format!("{}%", (level * 100.0).round() as i32) });
                cv.progress(Rect::new(tx, cy - 1.5, right - 46.0 - tx, 3.0), if muted { 0.0 } else { level }, cv.accent);
            }
            Toast::Brightness { level } => {
                cv.glyph(icon::BRIGHTNESS, ic, false, collapsed_txt());
                value(&format!("{}%", (level * 100.0).round() as i32));
                cv.progress(Rect::new(tx, cy - 1.5, right - 46.0 - tx, 3.0), level, cv.accent);
            }
            Toast::Charging { on, percent } => {
                self.battery(cv, ic, percent, if on { status_green() } else { collapsed_txt() });
                cv.text(if on { "Зарядка" } else { "От батареи" }, Rect::new(tx, cy - 8.0, right - 44.0 - tx, 16.0), &f.body, collapsed_txt());
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
        cv.rr_line(body, 2.5, collapsed_dim(), 1.2);
        cv.rr(Rect::new(body.right() + 0.8, body.y + 2.5, 1.8, 4.0), 0.9, collapsed_dim());
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
            live_media: Some(Media {
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
                source_name: "Spotify".into(),
                session_count: 1,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn playing_stays_idle_form() {
        let mut l = Live::new(true);
        assert_eq!(l.form(), Form::Idle);
        assert!(!l.playing());
        assert!(l.sync_media(&media(true, 1, 1)));
        assert_eq!(l.form(), Form::Idle);
        assert!(l.playing());
        assert!(l.sync_media(&media(false, 1, 2)));
        assert_eq!(l.form(), Form::Idle);
        assert!(!l.playing());
    }

    #[test]
    fn same_revision_is_ignored() {
        let mut l = Live::new(true);
        assert!(l.sync_media(&media(true, 1, 1)));
        assert!(!l.sync_media(&media(true, 1, 1)));
    }

    #[test]
    fn disabled_media_suppresses_playing() {
        let mut l = Live::new(false);
        l.sync_media(&media(true, 1, 1));
        assert!(!l.playing());
        l.set_media_enabled(true);
        assert!(l.playing());
    }

    #[test]
    fn toast_sets_form_and_expires_to_idle() {
        let mut l = Live::new(true);
        l.sync_media(&media(true, 1, 1));
        l.show(Toast::Timer);
        assert_eq!(l.form(), Form::Toast);
        assert!(l.clear_toast());
        assert!(!l.clear_toast());
        assert_eq!(l.form(), Form::Idle);
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
        assert_eq!(Toast::Notice.lifetime(), Duration::from_millis(3000));
        assert!(Toast::Notice.emerges_from_strip());
        assert!(!Toast::Notice.steals_focus());
        assert!(Toast::Event.emerges_from_strip());
        assert!(!Toast::Event.steals_focus());
        assert!(Toast::Timer.steals_focus());
    }

    #[test]
    fn only_timer_toast_is_an_alert() {
        let mut l = Live::new(true);
        l.show(Toast::Volume { level: 0.5, muted: false });
        assert!(!l.alert());
        l.show(Toast::Timer);
        assert!(l.alert());
        l.clear_toast();
        assert!(!l.alert(), "после закрытия остров возвращается в прежнее состояние");
    }
}
