//! «Лицо» свёрнутого острова, когда музыка не играет: слева раскладка клавиатуры, по центру время,
//! справа значок Wi-Fi. Данные приходят без опроса в простое: время обновляет минутный таймер
//! (он включён, только пока лицо на экране), раскладку проверяет уже существующий редкий
//! опрос окружения, сеть — событие смены интерфейсов в фоновом потоке (`bg::NetStatus`).

use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::anim::Spring;
use crate::bg::{self, NetStatus};
use crate::ui::{dim, icon, status_orange, track, txt, Canvas, Rect};

pub struct Face {
    enabled: bool,
    visible: bool,
    alpha: Spring,
    time: String,
    lang: u32,
    lang_text: String,
    net: NetStatus,
}

/// «14:05».
pub fn fmt_time(hour: u16, minute: u16) -> String {
    format!("{hour:02}:{minute:02}")
}

/// Через сколько миллисекунд начнётся следующая минута (с небольшим запасом, чтобы таймер не
/// сработал за миг до смены минуты).
pub fn ms_to_next_minute(second: u16, millis: u16) -> u32 {
    ((60 - second.min(59) as u32) * 1000).saturating_sub(millis.min(999) as u32) + 25
}

/// Сколько линий сигнала горит: 0 — нет сети, 1–3 — Wi-Fi по качеству, проводная сеть — все три.
pub fn signal_level(n: NetStatus) -> u8 {
    if n.wifi_connected {
        1 + (n.quality > 30) as u8 + (n.quality > 62) as u8
    } else if n.wired_up {
        3
    } else {
        0
    }
}

impl Face {
    pub fn new(enabled: bool) -> Self {
        let mut f = Face {
            enabled,
            visible: false,
            alpha: Spring::unit(0.0, 260.0, 30.0),
            time: String::new(),
            lang: 0,
            lang_text: String::new(),
            net: NetStatus::default(),
        };
        f.refresh_all();
        f
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Нужно ли показывать лицо (остров свёрнут, ничего не играет, ничто не закрывает).
    /// `timer` — на острове есть таймер: лицо показывается, даже если выключено в настройках.
    pub fn set_visible(&mut self, v: bool, timer: bool) {
        self.visible = v && (self.enabled || timer);
        self.alpha.target = if self.visible { 1.0 } else { 0.0 };
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn animating(&self) -> bool {
        !self.alpha.settled()
    }

    pub fn alpha(&self) -> f32 {
        self.alpha.value.clamp(0.0, 1.0)
    }

    pub fn step(&mut self, dt: f32) {
        self.alpha.step(dt);
        if self.alpha.settled() {
            self.alpha.snap();
        }
    }

    fn read_time() -> (String, u32) {
        let t = unsafe { GetLocalTime() };
        (fmt_time(t.wHour, t.wMinute), ms_to_next_minute(t.wSecond, t.wMilliseconds))
    }

    fn refresh_all(&mut self) {
        self.time = Self::read_time().0;
        self.net = bg::net_status();
        self.poll_lang();
    }

    /// Минутный тик: время, сеть (уровень сигнала меняется без событий) и раскладка.
    /// Возвращает `true`, если что-то изменилось.
    pub fn tick(&mut self) -> bool {
        let before = (self.time.clone(), self.net, self.lang);
        self.refresh_all();
        before != (self.time.clone(), self.net, self.lang)
    }

    /// Через сколько миллисекунд нужен следующий минутный тик.
    pub fn next_tick_ms(&self) -> u32 {
        Self::read_time().1
    }

    /// Раскладка окна в фокусе. Дёшево (два вызова user32), поэтому её проверяет общий опрос окружения.
    pub fn poll_lang(&mut self) -> bool {
        let lang = crate::sys::foreground_lang();
        if lang == self.lang && !self.lang_text.is_empty() {
            return false;
        }
        self.lang = lang;
        self.lang_text = crate::sys::lang_code(lang);
        true
    }

    /// Забирает свежее состояние сети из фонового потока.
    pub fn sync_net(&mut self) -> bool {
        let n = bg::net_status();
        if n == self.net {
            return false;
        }
        self.net = n;
        true
    }

    /// Рисует лицо внутри свёрнутого острова `r` (логические пиксели).
    /// Если есть таймер, по центру вместо времени — его остаток (на паузе мигает).
    pub unsafe fn draw(&self, cv: &Canvas, r: Rect, timer: Option<(&str, f32)>) {
        let f = &cv.g.fonts;
        let pad = (r.w * 0.09).clamp(8.0, 16.0);
        cv.text(&self.lang_text, Rect::new(r.x + pad, r.y, 34.0, r.h - 1.0), &f.small_b, dim());
        match timer {
            Some((label, pulse)) => {
                let t = Canvas { g: cv.g, alpha: cv.alpha * pulse, accent: cv.accent };
                // Значок слева от текста: ширина цифр табличная, около 7 px на знак.
                let half = label.chars().count() as f32 * 3.6;
                t.glyph(icon::CLOCK, Rect::new(r.cx() - half - 20.0, r.cy() - 9.0, 16.0, 16.0), false, status_orange());
                t.text(label, Rect::new(r.x, r.y, r.w, r.h - 1.0), &f.body_c, txt());
            }
            None => cv.text(&self.time, Rect::new(r.x, r.y, r.w, r.h - 1.0), &f.body_c, txt()),
        }
        self.wifi(cv, r.right() - pad - 6.5, r.cy() + 3.2);
    }

    /// Значок Wi-Fi: точка и три дуги, нарисованные примитивами (глифов в шрифте может не быть).
    /// `(cx, cy)` — точка-источник у нижнего края значка.
    unsafe fn wifi(&self, cv: &Canvas, cx: f32, cy: f32) {
        let level = signal_level(self.net);
        let off = track();
        let on = txt();
        for i in 0..3u8 {
            let radius = 3.4 + 3.0 * i as f32;
            cv.stroke_arc(cx, cy, radius, -46.0, 46.0, 1.4, if level > i { on } else { off });
        }
        cv.circle(cx, cy, 1.25, if level > 0 { on } else { off });
        if level == 0 {
            // Нет сети: перечёркнутый значок.
            cv.stroke_line((cx - 6.5, cy - 9.5), (cx + 6.5, cy + 1.5), 1.4, dim());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_is_zero_padded() {
        assert_eq!(fmt_time(9, 5), "09:05");
        assert_eq!(fmt_time(23, 59), "23:59");
    }

    #[test]
    fn next_minute_delay() {
        assert_eq!(ms_to_next_minute(0, 0), 60_025);
        assert_eq!(ms_to_next_minute(59, 999), 26);
        assert_eq!(ms_to_next_minute(30, 500), 29_525);
        // Мусор на входе не даёт переполнения.
        assert!(ms_to_next_minute(99, 5000) < 2000);
    }

    #[test]
    fn signal_levels() {
        let n = |wifi, q, wired| NetStatus { wifi_connected: wifi, quality: q, wired_up: wired };
        assert_eq!(signal_level(n(false, 0, false)), 0);
        assert_eq!(signal_level(n(true, 10, false)), 1);
        assert_eq!(signal_level(n(true, 50, false)), 2);
        assert_eq!(signal_level(n(true, 90, false)), 3);
        assert_eq!(signal_level(n(false, 0, true)), 3);
        // Wi-Fi важнее провода: слабый Wi-Fi не маскируется Ethernet.
        assert_eq!(signal_level(n(true, 10, true)), 1);
    }

    #[test]
    fn hidden_face_fades_out() {
        let mut f = Face::new(true);
        f.set_visible(true, false);
        for _ in 0..120 {
            f.step(1.0 / 60.0);
        }
        assert!((f.alpha() - 1.0).abs() < 0.01 && !f.animating());
        f.set_visible(false, false);
        for _ in 0..120 {
            f.step(1.0 / 60.0);
        }
        assert_eq!(f.alpha(), 0.0);
        // Выключенное в настройках лицо не показывается, кроме случая, когда на острове идёт таймер.
        let mut off = Face::new(false);
        off.set_visible(true, false);
        assert!(!off.visible());
        off.set_visible(true, true);
        assert!(off.visible());
    }
}
