//! Остров как индикатор (этап 4): в свёрнутой форме показывает мини-плеер (когда включён в
//! настройках) или боковые аудиоволны при воспроизведении музыки, а также короткие уведомления
//! (таймер, громкость, зарядка). Никаких своих таймеров в простое: данные приходят событиями
//! (медиасессия, громкость, питание).

use std::time::Duration;

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::anim::{Spring, FADE_SPRING};
use crate::bg::{self, Media, Snapshot, NOTIF_ICON_PX};
use crate::tabs::notifications::PopupNotice;
use crate::ui::{
    collapsed_chip_bg, collapsed_dim, collapsed_txt, icon, on_color, status_green, status_orange,
    status_pomodoro, status_red, Canvas, Rect, Ui, WHITE,
};

/// Размеры форм (логические пиксели). Ширина не меньше ширины свёрнутого острова.
pub const MINI: (f32, f32) = (252.0, 36.0);
/// Размеры формы уведомления (логические пиксели). Ширина не меньше ширины свёрнутого острова.
pub const TOAST: (f32, f32) = (232.0, 36.0);
/// Плавающая капсула-островок для всплывающего уведомления (приложение + текст).
pub const TOAST_NOTICE: (f32, f32) = (316.0, 50.0);
/// Отступ плавающей капсулы уведомления от верхнего края экрана (логические пиксели).
pub const CAPSULE_TOP_Y: f32 = 8.0;
/// Заряд, ниже которого (и без питания от сети) показывается предупреждение.
const LOW_PCT: u8 = 20;

/// Режим и текущая фаза таймера для согласованной цветовой индикации свёрнутого острова.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimerKind {
    #[default]
    Timer,
    PomoWork,
    PomoBreak,
}

impl TimerKind {
    #[inline]
    pub fn tint(self) -> D2D1_COLOR_F {
        match self {
            Self::Timer => status_orange(),
            Self::PomoWork => status_pomodoro(),
            Self::PomoBreak => status_green(),
        }
    }

    /// Цвет текста и иконок поверх заливки таймера: на оранжевом и коралловом белый
    /// читается лучше; на светло-зелёном подбирается контрастный цвет через `on_color`.
    #[inline]
    pub fn on_fill(self, fill: D2D1_COLOR_F) -> D2D1_COLOR_F {
        match self {
            Self::Timer | Self::PomoWork => WHITE,
            Self::PomoBreak => on_color(fill),
        }
    }
}

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
    pub kind: TimerKind,
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

/// Прозрачность активного таймера на паузе: слегка притухает вместо мигания.
pub const TIMER_PAUSED_ALPHA: f32 = 0.45;

/// Длительность появления капли таймера (согласовано с макетом: 57,5 % от 2 с ≈ 1,05 с).
const DROP_ENTRY_SECS: f32 = 1.05;
/// Длительность плавного возврата капли в чёлку (сброс таймера, появление тоста).
const DROP_EXIT_SECS: f32 = 0.28;
/// Быстрое втягивание капли при раскрытии острова.
const DROP_FAST_EXIT_SECS: f32 = 0.16;
/// Дополнительный зазор между «ушком» чёлки и левым краем капли (логические пиксели).
const DROP_GAP_EXTRA: f32 = 8.0;

/// Вычисленные параметры кадра для отдельной капли таймера и её жидкой перемычки.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimerDropFrame {
    /// Нормализованное положение по X (`0` — у края чёлки, `1` — целевая позиция, >1 — лёгкий перелёт).
    pub pos: f32,
    /// Горизонтальное растяжение/сжатие капли (squash & stretch).
    pub sx: f32,
    /// Вертикальное растяжение/сжатие капли.
    pub sy: f32,
    /// Толщина перешейка перемычки (0..0.72).
    pub bridge_y: f32,
    /// Прозрачность жидкой перемычки (0..1).
    pub bridge_a: f32,
    /// Прозрачность текста обратного отсчёта внутри капли (0..1).
    pub text_a: f32,
    /// Подсветка при наведении курсора (0..1).
    pub hover: f32,
    /// Общая видимость (0..1), гаснет только у самого `u == 0`.
    pub vis: f32,
}

/// Якорная геометрия чёлки в логических пикселях:
/// `(notch_r, notch_y, notch_h, ear, r_bot)`.
pub type DropAnchor = (f32, f32, f32, f32, f32);

/// Кусочно-гладкая (smoothstep) интерполяция по ключевым кадрам `(u, val)` на отрезке `0..=1`.
fn eval_keyframes(u: f32, keys: &[(f32, f32)]) -> f32 {
    if u <= keys[0].0 {
        return keys[0].1;
    }
    let last = keys[keys.len() - 1];
    if u >= last.0 {
        return last.1;
    }
    for w in keys.windows(2) {
        let (t0, v0) = w[0];
        let (t1, v1) = w[1];
        if u <= t1 {
            let f = ((u - t0) / (t1 - t0).max(1e-5)).clamp(0.0, 1.0);
            let s = f * f * (3.0 - 2.0 * f);
            return v0 + (v1 - v0) * s;
        }
    }
    last.1
}

/// Габариты капли `(w, h, font_size)` в логических пикселях по высоте чёлки и ширине текста.
/// Капля чуть приплюснута по высоте (`~75 %` от чёлки, `18 px` при `h = 24`) и вытянута в пилюлю,
/// чтобы стоять ровно по вертикальной середине чёлки с крупным отсчётом (`~9.8pt`) и расширяться под часы (`1:02:05`).
pub fn timer_drop_box(collapsed_h: f32, text_w: f32) -> (f32, f32, f32) {
    let base_h = collapsed_h.max(16.0);
    let h = (base_h * 0.75).clamp(15.0, (base_h - 3.0).max(15.0));
    let font_size = (base_h * 0.41).clamp(8.5, 14.0).min(h * 0.56);
    let w = (base_h * 1.67).max(text_w + 15.0);
    (w, h, font_size)
}

/// Грубая оценка ширины текста без DirectWrite (резервный путь для headless-тестов).
pub fn approx_timer_text_w(label: &str, font_size: f32) -> f32 {
    let units: f32 = label.chars().map(|c| if c == ':' { 0.26 } else { 0.51 }).sum();
    (units * font_size).ceil()
}

/// Центр `(cx, cy)` капли таймера в логических пикселях окна.
pub fn timer_drop_center(anchor: DropAnchor, drop_w: f32, pos: f32) -> (f32, f32) {
    let (notch_r, notch_y, notch_h, ear, _) = anchor;
    let gap = (ear + DROP_GAP_EXTRA).max(12.0);
    let target_cx = notch_r + gap + drop_w * 0.5;
    let start_cx = notch_r - drop_w * 0.18;
    let cx = start_cx + (target_cx - start_cx) * pos;
    let cy = notch_y + notch_h * 0.5;
    (cx, cy)
}

/// Текущий прямоугольник капли в логических пикселях с учётом её сдвига и масштаба.
pub fn timer_drop_rect(anchor: DropAnchor, drop_w: f32, drop_h: f32, pos: f32, sx: f32, sy: f32) -> Rect {
    let (cx, cy) = timer_drop_center(anchor, drop_w, pos);
    let w = (drop_w * sx).max(1.0);
    let h = (drop_h * sy).max(1.0);
    Rect::new(cx - w * 0.5, cy - h * 0.5, w, h)
}

/// Состояние и анимация отделяющейся от чёлки капли активного таймера.
pub struct TimerDrop {
    active: bool,
    fast_hide: bool,
    retract_clean: bool,
    u: f32,
    hover: Spring,
    shown_label: String,
    shown_kind: TimerKind,
    shown_paused: bool,
    anchor: Option<DropAnchor>,
}

impl TimerDrop {
    pub fn new() -> Self {
        Self {
            active: false,
            fast_hide: false,
            retract_clean: false,
            u: 0.0,
            hover: Spring::unit(0.0, 520.0, 38.0),
            shown_label: String::new(),
            shown_kind: TimerKind::Timer,
            shown_paused: false,
            anchor: None,
        }
    }

    pub fn sync_timer(&mut self, t: Option<&TimerView>) {
        if let Some(tv) = t {
            self.shown_label = tv.label();
            self.shown_kind = tv.kind;
            self.shown_paused = tv.paused;
        }
    }

    pub fn set_active(&mut self, active: bool, fast_hide: bool) {
        if self.active == active {
            if !active {
                self.fast_hide |= fast_hide;
            }
            return;
        }
        self.active = active;
        if active {
            self.fast_hide = false;
            self.retract_clean = false;
        } else {
            self.fast_hide = fast_hide;
            self.retract_clean = self.u >= 0.95;
            self.hover.target = 0.0;
        }
    }

    pub fn set_hover(&mut self, hovered: bool) -> bool {
        let target = if hovered && self.clickable() { 1.0 } else { 0.0 };
        if (self.hover.target - target).abs() > 0.01 {
            self.hover.target = target;
            true
        } else {
            false
        }
    }

    pub fn latch_anchor(&mut self, anchor: DropAnchor) {
        self.anchor = Some(anchor);
    }

    pub fn anchor(&self) -> Option<DropAnchor> {
        self.anchor
    }

    pub fn label(&self) -> &str {
        &self.shown_label
    }

    pub fn kind(&self) -> TimerKind {
        self.shown_kind
    }

    pub fn paused(&self) -> bool {
        self.shown_paused
    }

    pub fn visible(&self) -> bool {
        self.u > 0.004 && !self.shown_label.is_empty() && self.anchor.is_some()
    }

    pub fn clickable(&self) -> bool {
        self.active && self.u > 0.25 && !self.shown_label.is_empty() && self.anchor.is_some()
    }

    pub fn animating(&self) -> bool {
        let target = if self.active { 1.0 } else { 0.0 };
        (self.u - target).abs() > 1e-4 || !self.hover.settled()
    }

    pub fn step(&mut self, dt: f32) {
        let scale = if crate::theme::reduced_motion() { 8.0 } else { 1.0 };
        if self.active {
            if self.u < 1.0 {
                self.u = (self.u + (dt * scale) / DROP_ENTRY_SECS).min(1.0);
            }
        } else if self.u > 0.0 {
            let secs = if self.fast_hide { DROP_FAST_EXIT_SECS } else { DROP_EXIT_SECS };
            self.u = (self.u - (dt * scale) / secs).max(0.0);
            if self.u == 0.0 {
                self.retract_clean = false;
                self.fast_hide = false;
            }
        }
        self.hover.step(dt);
        if self.hover.settled() {
            self.hover.snap();
        }
    }

    pub fn frame(&self) -> Option<TimerDropFrame> {
        if !self.visible() {
            return None;
        }
        let u = self.u.clamp(0.0, 1.0);
        let vis = (u / 0.08).clamp(0.0, 1.0);
        let hover = self.hover.value.clamp(0.0, 1.0);

        if !self.active && self.retract_clean {
            let s = u * u * (3.0 - 2.0 * u);
            let sx = 0.20 + 0.80 * u.sqrt();
            let sy = 0.10 + 0.90 * u * u;
            let bridge_env = (1.0 - (u - 0.35).abs() / 0.32).clamp(0.0, 1.0);
            let bridge_y = 0.55 * bridge_env;
            let bridge_a = (1.0 - (u - 0.35).abs() / 0.28).clamp(0.0, 1.0);
            let text_a = ((u - 0.55) / 0.35).clamp(0.0, 1.0);
            return Some(TimerDropFrame { pos: s, sx, sy, bridge_y, bridge_a, text_a, hover, vis });
        }

        const K: f32 = 1.0 / 0.575;
        const POS_KEYS: [(f32, f32); 6] = [
            (0.0, 0.0),
            (0.075 * K, 0.042),
            (0.21 * K, 0.438),
            (0.36 * K, 0.938),
            (0.45 * K, 1.042),
            (1.0, 1.0),
        ];
        const SX_KEYS: [(f32, f32); 6] = [
            (0.0, 0.05),
            (0.075 * K, 0.32),
            (0.21 * K, 1.12),
            (0.36 * K, 0.91),
            (0.45 * K, 1.035),
            (1.0, 1.0),
        ];
        const SY_KEYS: [(f32, f32); 6] = [
            (0.0, 0.05),
            (0.075 * K, 0.55),
            (0.21 * K, 0.73),
            (0.36 * K, 1.08),
            (0.45 * K, 0.97),
            (1.0, 1.0),
        ];
        const BRIDGE_Y_KEYS: [(f32, f32); 5] = [
            (0.0, 0.60),
            (0.10 * K, 0.72),
            (0.21 * K, 0.45),
            (0.30 * K, 0.035),
            (0.325 * K, 0.0),
        ];
        const BRIDGE_A_KEYS: [(f32, f32); 5] = [
            (0.0, 0.0),
            (0.06 * K, 0.0),
            (0.10 * K, 1.0),
            (0.265 * K, 1.0),
            (0.325 * K, 0.0),
        ];
        const TEXT_A_KEYS: [(f32, f32); 3] = [(0.0, 0.0), (0.29 * K, 0.0), (0.45 * K, 1.0)];

        Some(TimerDropFrame {
            pos: eval_keyframes(u, &POS_KEYS),
            sx: eval_keyframes(u, &SX_KEYS),
            sy: eval_keyframes(u, &SY_KEYS),
            bridge_y: eval_keyframes(u, &BRIDGE_Y_KEYS),
            bridge_a: eval_keyframes(u, &BRIDGE_A_KEYS),
            text_a: eval_keyframes(u, &TEXT_A_KEYS),
            hover,
            vis,
        })
    }
}

/// Какую форму принимает свёрнутый остров.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Idle,
    Mini,
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

    /// Уведомление, при наведении на которое свёрнутый остров не должен раскрываться,
    /// чтобы пользователь успел нажать × или перейти в приложение.
    pub fn blocks_hover_expand(&self) -> bool {
        matches!(self, Toast::Timer | Toast::Event | Toast::Notice)
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
    player_enabled: bool,
    toast: Option<Toast>,
    /// Последнее показанное уведомление: рисуется, пока гаснет.
    shown: Option<Toast>,
    /// Текст уведомления о событии календаря.
    event: Option<(String, String)>,
    /// Входящее системное уведомление для всплывающего баннера.
    notice: Option<PopupNotice>,
    mini_a: Spring,
    wave_a: Spring,
    /// Измеритель пиков системного аудио и сглаженная огибающая по каналам `[L, R]`.
    wave_meter: crate::bg::AudioMeter,
    wave_audio_dev: Option<String>,
    wave_lvl: [f32; 2],
    wave_flux: [f32; 2],
    wave_agc: f32,
    wave_phase: f32,
    toast_a: Spring,
    rev: u64,
    ac: Option<bool>,
    low: bool,
    volume: Option<(f32, bool)>,
    brightness: Option<f32>,
    /// Таймер Today для свёрнутой формы и плавная доля полоски.
    timer: Option<TimerView>,
    timer_frac: Spring,
    /// Отделяющаяся капля активного таймера справа от чёлки.
    timer_drop: TimerDrop,
}

impl Live {
    pub fn new(media_enabled: bool, player_enabled: bool) -> Self {
        let mut l = Live {
            media: None,
            media_enabled,
            player_enabled,
            toast: None,
            shown: None,
            event: None,
            notice: None,
            mini_a: Spring::unit(0.0, FADE_SPRING.0, FADE_SPRING.1),
            wave_a: Spring::unit(0.0, FADE_SPRING.0, FADE_SPRING.1),
            wave_meter: crate::bg::AudioMeter::new(),
            wave_audio_dev: None,
            wave_lvl: [0.0; 2],
            wave_flux: [0.0; 2],
            wave_agc: 0.25,
            wave_phase: 0.0,
            toast_a: Spring::unit(0.0, 260.0, 30.0),
            rev: 0,
            ac: None,
            low: false,
            volume: None,
            brightness: None,
            timer: None,
            timer_frac: Spring::unit(1.0, 160.0, 22.0),
            timer_drop: TimerDrop::new(),
        };
        // Начальное состояние питания запоминаем без уведомления.
        l.power_event();
        l
    }

    pub fn set_media_enabled(&mut self, on: bool) {
        self.media_enabled = on;
    }

    pub fn set_player_enabled(&mut self, on: bool) {
        self.player_enabled = on;
    }

    /// Какая форма нужна, если остров свёрнут и ничто его не закрывает.
    pub fn form(&self) -> Form {
        if self.toast.is_some() {
            Form::Toast
        } else if self.playing() && self.player_enabled {
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
        let changed = appeared
            || self.timer.map(|o| (o.secs, o.paused, o.kind)) != t.map(|n| (n.secs, n.paused, n.kind));
        if let Some(n) = t {
            self.timer_frac.target = n.frac.clamp(0.0, 1.0);
            if self.timer.is_none() {
                self.timer_frac.snap();
            }
        }
        self.timer_drop.sync_timer(t.as_ref());
        self.timer = t;
        (changed, appeared)
    }

    pub fn set_timer_drop(&mut self, active: bool, fast_hide: bool) {
        self.timer_drop.set_active(active, fast_hide);
    }

    pub fn set_timer_drop_hover(&mut self, hovered: bool) -> bool {
        self.timer_drop.set_hover(hovered)
    }

    pub fn latch_timer_drop_anchor(&mut self, anchor: DropAnchor) {
        self.timer_drop.latch_anchor(anchor);
    }

    pub fn timer_drop_anchor(&self) -> Option<DropAnchor> {
        self.timer_drop.anchor()
    }

    pub fn timer_drop_visible(&self) -> bool {
        self.timer_drop.visible()
    }

    pub fn timer_drop_clickable(&self) -> bool {
        self.timer_drop.clickable()
    }

    pub fn timer_drop_frame(&self) -> Option<TimerDropFrame> {
        self.timer_drop.frame()
    }

    pub fn timer_drop_label(&self) -> &str {
        self.timer_drop.label()
    }

    pub fn timer_drop_tint(&self) -> D2D1_COLOR_F {
        self.timer_drop.kind().tint()
    }

    pub fn timer_drop_kind(&self) -> TimerKind {
        self.timer_drop.kind()
    }

    pub fn timer_drop_alpha(&self) -> f32 {
        if self.timer_drop.paused() { TIMER_PAUSED_ALPHA } else { 1.0 }
    }

    pub fn timer_drop_paused(&self) -> bool {
        self.timer_drop.paused()
    }

    /// Цвет активного таймера (или фазы Pomodoro) для свёрнутого острова.
    #[inline]
    pub fn timer_tint(&self) -> D2D1_COLOR_F {
        self.timer.map_or_else(status_orange, |t| t.kind.tint())
    }

    /// Плавная доля оставшегося времени для полоски.
    pub fn timer_frac(&self) -> f32 {
        self.timer_frac.value.clamp(0.0, 1.0)
    }

    /// Прозрачность индикатора таймера: на паузе притухает вместо мигания, иначе 1.
    pub fn timer_alpha(&self) -> f32 {
        match self.timer {
            Some(t) if t.paused => TIMER_PAUSED_ALPHA,
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
        if snap.audio != self.wave_audio_dev {
            self.wave_audio_dev = snap.audio.clone();
            self.wave_meter.invalidate();
        }
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

    /// Наведение на активное уведомление не должно раскрывать остров.
    pub fn blocks_hover_expand(&self) -> bool {
        self.toast.is_some_and(|t| t.blocks_hover_expand())
    }

    /// Сейчас показано всплывающее системное уведомление (`Toast::Notice`).
    pub fn is_notice(&self) -> bool {
        matches!(self.toast, Some(Toast::Notice))
    }

    /// Данные активного всплывающего уведомления (если сейчас показано `Toast::Notice`).
    pub fn notice(&self) -> Option<&PopupNotice> {
        self.is_notice().then_some(self.notice.as_ref()).flatten()
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
        self.mini_a.target = if form == Form::Mini { 1.0 } else { 0.0 };
        self.toast_a.target = if form == Form::Toast { 1.0 } else { 0.0 };
        if !matches!(form, Form::Idle | Form::Mini) {
            self.wave_a.target = 0.0;
        }
    }

    /// Точечное управление целевой альфой мини-плеера (для временного peek лица).
    pub fn set_mini_target(&mut self, target: f32) {
        self.mini_a.target = target;
    }

    /// Меняет жёсткость и затухание пружины мини-плеера (медленный peek).
    pub fn tune_mini(&mut self, (stiffness, damping): (f32, f32)) {
        self.mini_a.tune(stiffness, damping);
    }

    pub fn mini_settled(&self) -> bool {
        self.mini_a.settled()
    }

    pub fn mini_alpha(&self) -> f32 {
        self.mini_a.value.clamp(0.0, 1.0)
    }

    /// Выставляет целевую прозрачность боковых волн в обычной свёрнутой чёлке.
    pub fn set_wave_active(&mut self, active: bool) {
        self.wave_a.target = if active { 1.0 } else { 0.0 };
        if !active {
            self.wave_meter.reset();
        }
    }

    pub fn wave_alpha(&self) -> f32 {
        self.wave_a.value.clamp(0.0, 1.0)
    }

    /// Возвращает `(alpha, lvl [L, R], flux [L, R], phase)`, если волны видны.
    pub fn wave_state(&self) -> Option<(f32, [f32; 2], [f32; 2], f32)> {
        let a = self.wave_alpha();
        (a > 0.004).then_some((a, self.wave_lvl, self.wave_flux, self.wave_phase))
    }

    /// Обновляет сглаженные уровни, всплеск фронта и фазу волн по стерео-пикам `[L, R]`.
    pub fn feed_wave_peaks(&mut self, raw_l: f32, raw_r: f32, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        const MIN_AGC: f32 = 0.08;
        let l = if raw_l.is_finite() { raw_l.clamp(0.0, 1.0) } else { 0.0 };
        let r = if raw_r.is_finite() { raw_r.clamp(0.0, 1.0) } else { 0.0 };
        let raw_max = l.max(r);
        if raw_max > self.wave_agc {
            let k = 1.0 - (-dt / 0.08).exp();
            self.wave_agc += (raw_max - self.wave_agc) * k;
        } else {
            let k = 1.0 - (-dt / 3.5).exp();
            self.wave_agc += (raw_max.max(MIN_AGC) - self.wave_agc) * k;
        }
        let denom = self.wave_agc.max(MIN_AGC);
        let norm = |v: f32| if v <= 0.002 { 0.0 } else { (v / denom).clamp(0.0, 1.0).powf(0.75) };
        let nl = norm(l);
        let nr = norm(r);
        let targets = [nl * 0.84 + nr * 0.16, nr * 0.84 + nl * 0.16];

        let flux_decay = (-dt / 0.14).exp();
        for (i, target) in targets.into_iter().enumerate() {
            let cur = self.wave_lvl[i];
            let onset = (target - cur).max(0.0);
            let tau = if target > cur { 0.035 } else { 0.16 };
            let k = 1.0 - (-dt / tau).exp();
            self.wave_lvl[i] = (cur + (target - cur) * k).clamp(0.0, 1.0);
            self.wave_flux[i] = (self.wave_flux[i] * flux_decay).max((onset * 2.2).clamp(0.0, 1.0));
        }

        let avg_l = 0.5 * (self.wave_lvl[0] + self.wave_lvl[1]);
        let avg_f = 0.5 * (self.wave_flux[0] + self.wave_flux[1]);
        self.wave_phase = (self.wave_phase + dt * (1.6 + 4.5 * avg_l + 2.8 * avg_f)) % 1000.0;
    }

    pub fn step(&mut self, dt: f32) {
        self.mini_a.step(dt);
        self.wave_a.step(dt);
        self.toast_a.step(dt);
        self.timer_frac.step(dt);
        self.timer_drop.step(dt);
        if self.wave_a.target > 0.5 {
            #[cfg(not(test))]
            {
                let [rl, rr] = unsafe { self.wave_meter.sample() };
                self.feed_wave_peaks(rl, rr, dt);
            }
            #[cfg(test)]
            {
                let avg_l = 0.5 * (self.wave_lvl[0] + self.wave_lvl[1]);
                self.wave_phase = (self.wave_phase + dt * (1.6 + 4.5 * avg_l)) % 1000.0;
            }
        } else {
            self.wave_meter.reset();
            let decay = (-dt / 0.12).exp();
            for i in 0..2 {
                self.wave_lvl[i] *= decay;
                self.wave_flux[i] *= decay;
            }
            if self.wave_a.value <= 0.001 {
                self.wave_lvl = [0.0; 2];
                self.wave_flux = [0.0; 2];
                self.wave_agc = 0.25;
            }
        }
        if self.timer_frac.settled() {
            self.timer_frac.snap();
        }
        if self.mini_a.settled() {
            self.mini_a.snap();
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
        !(self.mini_a.settled() && self.wave_a.settled() && self.toast_a.settled() && self.timer_frac.settled())
            || self.timer_drop.animating()
    }

    /// Тонкая линия таймера под прогрессом трека в мини-плеере.
    pub unsafe fn draw_timer_line(&self, cv: &Canvas, r: Rect) {
        if self.timer.is_none() {
            return;
        }
        let line = Canvas { g: cv.g, alpha: cv.alpha * self.timer_alpha(), accent: cv.accent };
        let track_r = Rect::new(r.x + 14.0, r.bottom() - 3.0, r.w - 28.0, 1.5);
        line.rr(Rect::new(track_r.x, track_r.y, track_r.w * self.timer_frac(), track_r.h), 0.75, self.timer_tint());
    }

    pub fn toast_alpha(&self) -> f32 {
        self.toast_a.value.clamp(0.0, 1.0)
    }

    // ---------- Отрисовка ----------

    /// Мини-плеер: обложка, название, исполнитель и тонкий прогресс внизу.
    /// Эквалайзер рисуется живыми боковыми аудиоволнами по краям чёлки (`draw_edge_waves`).
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
        let tw = (r.right() - 12.0 - tx).max(10.0);
        cv.text(&m.title, Rect::new(tx, r.y + 4.0, tw, 14.0), &f.small_b, collapsed_txt());
        cv.text(&m.artist, Rect::new(tx, r.y + 16.0, tw, 12.0), &f.small, collapsed_dim());
        let frac = if m.dur > 0.0 { m.position_now() / m.dur } else { 0.0 };
        // Прогресс трека поднят на 1 px, чтобы под ним поместилась линия таймера.
        cv.progress(Rect::new(r.x + 14.0, r.bottom() - 7.0, r.w - 28.0, 2.0), frac, cv.accent);
        self.draw_timer_line(cv, r);
    }

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
                    if r.contains(cursor) {
                        cv.rr(r, r.h / 2.0, D2D1_COLOR_F { a: 0.06, ..WHITE });
                    }
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
                            let ch_fmt = f.notif_at(9.0, crate::ui::weight::EMPHASIZED, false, true);
                            cv.text_styled(&ch, ir, &ch_fmt, WHITE, None);
                        }
                    }

                    let ntx = ir.right() + 10.0;
                    let ntw = (r.right() - 18.0 - ntx).max(40.0);
                    let meta_fmt = f.notif_at(8.0, crate::ui::weight::REGULAR, false, false);
                    let body_fmt = f.notif_at(9.0, crate::ui::weight::REGULAR, false, false);
                    let (meta_ctx, msg) = crate::tabs::notifications::split_display_fields(&n.title, &n.body, true);
                    let (meta, meta_bold) = crate::tabs::notifications::format_notif_meta(&n.app_name, &meta_ctx, "");
                    let meta_w = (ntw - 9.0).max(20.0);
                    cv.circle(ntx + 3.0, r.cy() - 9.0, 2.5, cv.accent);
                    cv.text_styled(&meta, Rect::new(ntx + 9.0, r.cy() - 16.0, meta_w, 14.0), &meta_fmt, collapsed_dim(), meta_bold);

                    cv.text_styled(&msg, Rect::new(ntx, r.cy() + 1.0, ntw, 17.0), &body_fmt, collapsed_txt(), None);
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
                let title_fmt = f.notif_at(9.0, crate::ui::weight::EMPHASIZED, false, false);
                cv.text_styled(title, Rect::new(tx, cy - 8.0, x.x - tx - tag_w - 4.0, 16.0), &title_fmt, collapsed_txt(), None);
                if tag_w > 0.0 {
                    let tag_fmt = f.notif_at(8.0, crate::ui::weight::REGULAR, false, false);
                    cv.text_styled(tag, Rect::new(x.x - tag_w, cy - 8.0, tag_w, 16.0), &tag_fmt, collapsed_dim(), None);
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
    fn mini_only_while_playing_and_enabled() {
        let mut l = Live::new(true, true);
        assert_eq!(l.form(), Form::Idle);
        assert!(!l.playing());
        assert!(l.sync_media(&media(true, 1, 1)));
        assert_eq!(l.form(), Form::Mini);
        assert!(l.playing());
        l.set_player_enabled(false);
        assert_eq!(l.form(), Form::Idle);
        assert!(l.playing());
        l.set_player_enabled(true);
        assert_eq!(l.form(), Form::Mini);
        assert!(l.sync_media(&media(false, 1, 2)));
        assert_eq!(l.form(), Form::Idle);
        assert!(!l.playing());
    }

    #[test]
    fn same_revision_is_ignored() {
        let mut l = Live::new(true, true);
        assert!(l.sync_media(&media(true, 1, 1)));
        assert!(!l.sync_media(&media(true, 1, 1)));
    }

    #[test]
    fn disabled_media_suppresses_playing() {
        let mut l = Live::new(false, true);
        l.sync_media(&media(true, 1, 1));
        assert!(!l.playing());
        assert_eq!(l.form(), Form::Idle);
        l.set_media_enabled(true);
        assert!(l.playing());
        assert_eq!(l.form(), Form::Mini);
    }

    #[test]
    fn toast_beats_mini_and_expires() {
        let mut l = Live::new(true, true);
        l.sync_media(&media(true, 1, 1));
        l.show(Toast::Timer);
        assert_eq!(l.form(), Form::Toast);
        assert!(l.clear_toast());
        assert!(!l.clear_toast());
        assert_eq!(l.form(), Form::Mini);
    }

    #[test]
    fn shown_toast_fades_before_it_is_dropped() {
        let mut l = Live::new(true, true);
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
        let mut l = Live::new(true, true);
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
        let mut l = Live::new(true, true);
        l.show(Toast::Volume { level: 0.5, muted: false });
        assert!(!l.alert());
        assert!(!l.blocks_hover_expand());
        l.show(Toast::Timer);
        assert!(l.alert());
        assert!(l.blocks_hover_expand());
        l.clear_toast();
        assert!(!l.alert(), "после закрытия остров возвращается в прежнее состояние");
        assert!(!l.blocks_hover_expand());
    }

    #[test]
    fn notice_toast_blocks_hover_expand_and_exposes_notice_while_active() {
        let mut l = Live::new(true, true);
        l.show_notice(PopupNotice {
            key: "win:42:100".into(),
            win_id: Some(42),
            app_id: "Telegram.TelegramDesktop".into(),
            app_name: "Telegram Desktop".into(),
            title: "Алексей".into(),
            body: "Привет".into(),
            icon: None,
            is_calendar: false,
        });
        assert!(l.is_notice());
        assert!(l.blocks_hover_expand());
        assert!(!l.alert(), "уведомление не использует кнопку × таймера");
        assert_eq!(l.notice().map(|n| n.app_id.as_str()), Some("Telegram.TelegramDesktop"));
        assert!(l.clear_toast());
        assert!(!l.is_notice());
        assert!(!l.blocks_hover_expand());
        assert!(l.notice().is_none());
    }

    #[test]
    fn wave_envelope_reacts_to_stereo_peaks_and_resets_when_disabled() {
        let mut l = Live::new(true, true);
        l.set_wave_active(true);
        for _ in 0..5 {
            l.feed_wave_peaks(0.8, 0.1, 0.033);
            l.step(0.033);
        }
        let Some((_a, lvl, flux, phase)) = l.wave_state().or_else(|| {
            for _ in 0..20 {
                l.step(0.033);
            }
            l.wave_state()
        }) else {
            panic!("wave_state должен появиться после активации");
        };
        assert!(lvl[0] > lvl[1], "левый пик должен давать более сильную волну слева: {lvl:?}");
        assert!(flux[0] >= 0.0 && phase > 0.0);

        // Тишина уводит амплитуду обратно к нулю без фантомного усиления шума.
        for _ in 0..40 {
            l.feed_wave_peaks(0.0, 0.0, 0.033);
        }
        assert!(l.wave_lvl[0] < 0.05 && l.wave_lvl[1] < 0.05);

        // Выключение волны затухает к 0 и сбрасывает огибающую.
        l.feed_wave_peaks(0.9, 0.9, 0.033);
        l.set_wave_active(false);
        for _ in 0..40 {
            l.step(0.033);
        }
        assert!(l.wave_state().is_none());
        assert_eq!(l.wave_lvl, [0.0, 0.0]);
    }

    #[test]
    fn timer_tint_tracks_phase_and_triggers_change_on_phase_switch() {
        let mut l = Live::new(true, true);
        assert_eq!(l.timer_tint(), status_orange());

        let work = TimerView { frac: 0.8, secs: 1200, paused: false, next_ms: 500, kind: TimerKind::PomoWork };
        let (changed, appeared) = l.set_timer(Some(work));
        assert!(changed && appeared);
        assert_eq!(l.timer_tint(), status_pomodoro());

        // Переключение работа -> перерыв при тех же секундах должно помечать изменение.
        let brk = TimerView { kind: TimerKind::PomoBreak, ..work };
        let (changed, appeared) = l.set_timer(Some(brk));
        assert!(changed && !appeared);
        assert_eq!(l.timer_tint(), status_green());

        let std_timer = TimerView { kind: TimerKind::Timer, ..work };
        let (changed, appeared) = l.set_timer(Some(std_timer));
        assert!(changed && !appeared);
        assert_eq!(l.timer_tint(), status_orange());
    }

    #[test]
    fn paused_timer_dims_instead_of_blinking() {
        let mut l = Live::new(true, true);
        assert_eq!(l.timer_alpha(), 1.0);

        let running = TimerView { frac: 0.5, secs: 60, paused: false, next_ms: 1000, kind: TimerKind::Timer };
        l.set_timer(Some(running));
        assert_eq!(l.timer_alpha(), 1.0);

        let paused = TimerView { paused: true, ..running };
        let (changed, appeared) = l.set_timer(Some(paused));
        assert!(changed && !appeared);
        assert_eq!(l.timer_alpha(), TIMER_PAUSED_ALPHA);

        l.set_timer(Some(running));
        assert_eq!(l.timer_alpha(), 1.0);
    }

    #[test]
    fn timer_drop_emerge_bridge_squash_and_retract() {
        let mut l = Live::new(true, true);
        l.latch_timer_drop_anchor((490.0, 0.0, 24.0, 8.0, 8.0));
        let tv = TimerView { frac: 0.9, secs: 1500, paused: false, next_ms: 1000, kind: TimerKind::PomoWork };
        l.set_timer(Some(tv));
        l.set_timer_drop(true, false);

        assert!(l.animating());
        assert!(!l.timer_drop_visible());

        // В первой трети вылета появляется жидкая перемычка и капля вытягивается по X.
        for _ in 0..18 {
            l.step(1.0 / 60.0);
        }
        let mid = l.timer_drop_frame().expect("капля должна быть видна в полёте");
        assert!(mid.bridge_a > 0.5, "перемычка должна быть видна: {}", mid.bridge_a);
        assert!(mid.sx > mid.sy, "при вылете капля вытянута по X: sx={}, sy={}", mid.sx, mid.sy);
        assert_eq!(mid.text_a, 0.0, "текст проявляется позже, когда капля округлилась");

        // К концу входа перемычка исчезает, текст проявляется, анимация успокаивается (0% CPU).
        for _ in 0..60 {
            l.step(1.0 / 60.0);
        }
        assert!(!l.animating());
        let done = l.timer_drop_frame().unwrap();
        assert!((done.pos - 1.0).abs() < 1e-4);
        assert!((done.sx - 1.0).abs() < 1e-4);
        assert!((done.sy - 1.0).abs() < 1e-4);
        assert_eq!(done.bridge_a, 0.0);
        assert!((done.text_a - 1.0).abs() < 1e-4);
        assert_eq!(l.timer_drop_tint(), status_pomodoro());
        assert_eq!(l.timer_drop_label(), "25:00");

        // При сбросе таймера капля сохраняет прошлую метку и цвет на время втягивания.
        l.set_timer(None);
        l.set_timer_drop(false, false);
        l.step(1.0 / 60.0);
        assert!(l.timer_drop_visible());
        assert_eq!(l.timer_drop_label(), "25:00");
        assert_eq!(l.timer_drop_tint(), status_pomodoro());

        for _ in 0..30 {
            l.step(1.0 / 60.0);
        }
        assert!(!l.timer_drop_visible());
        assert!(!l.animating());
    }

    #[test]
    fn timer_drop_box_is_wider_pill_for_mm_ss_and_hours() {
        let (_, h0, fs) = timer_drop_box(24.0, 0.0);
        assert_eq!(h0, 18.0);
        let tw_short = approx_timer_text_w("25:00", fs);
        let (w_short, h_short, _) = timer_drop_box(24.0, tw_short);
        assert!(w_short > h_short * 2.0 && w_short >= 40.0, "капля должна быть приплюснутой пилюлей: {w_short}×{h_short}");
        assert_eq!(h_short, 18.0);
        assert!((fs - 9.84).abs() < 0.05);

        let r = timer_drop_rect((490.0, 0.0, 24.0, 8.0, 8.0), w_short, h_short, 1.0, 1.0, 1.0);
        assert!((r.cy() - 12.0).abs() < 1e-4 && (r.y - (24.0 - r.bottom())).abs() < 1e-4, "капля должна стоять по вертикальной середине чёлки: {r:?}");

        let tw_long = approx_timer_text_w("1:02:05", fs);
        let (w_long, h_long, _) = timer_drop_box(24.0, tw_long);
        assert!(w_long > w_short && w_long >= tw_long + 15.0 && h_long == 18.0);

        // На оранжевом фоне (обычный таймер) и коралловом (Pomodoro) текст всегда белый.
        assert_eq!(TimerKind::Timer.on_fill(status_orange()), WHITE);
        assert_eq!(TimerKind::PomoWork.on_fill(status_pomodoro()), WHITE);
    }
}
