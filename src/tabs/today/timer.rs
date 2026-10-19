//! Таймер: время, пресеты, пауза и сброс, карточка таймера.

use std::time::{Duration, Instant};

use super::Today;
use crate::tabs::{id, TODAY};
use crate::ui::{chip_bg, dim, icon, ink, txt, Canvas, Rect, Submit, Ui, ORANGE};

const PRESETS: [u32; 5] = [1, 5, 10, 15, 20];

/// Число из текста поля: берутся только цифры, пусто — 0.
fn parse_num(s: &str) -> u32 {
    s.chars().filter_map(|c| c.to_digit(10)).fold(0u32, |a, d| a.saturating_mul(10).saturating_add(d)).min(99_999)
}

impl Today {
    fn remaining(&self) -> Duration {
        match self.deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => self.left,
        }
    }

    fn toggle_run(&mut self) {
        if let Some(d) = self.deadline.take() {
            self.left = d.saturating_duration_since(Instant::now());
        } else {
            if self.left.is_zero() {
                self.left = self.total;
            }
            self.deadline = Some(Instant::now() + self.left);
        }
    }

    /// Новое время таймера (останавливает его).
    fn set_time(&mut self, hours: u32, minutes: u32) {
        self.set_time_hms(hours, minutes, 0);
    }

    /// Время в часах, минутах и секундах: лишнее переносится в старшие единицы, от 1 минуты (пусто) до 9:59:59.
    fn set_time_hms(&mut self, hours: u32, minutes: u32, seconds: u32) {
        let mut total = (hours.min(99) * 3600 + minutes.min(9999) * 60 + seconds.min(99_999)).min(9 * 3600 + 59 * 60 + 59);
        if total == 0 {
            total = 60;
        }
        self.hours = total / 3600;
        self.minutes = total / 60 % 60;
        self.seconds = total % 60;
        self.total = Duration::from_secs(u64::from(total));
        self.left = self.total;
        self.deadline = None;
    }

    pub(super) fn set_preset(&mut self, i: usize) {
        self.preset = Some(i);
        self.set_time(0, PRESETS[i]);
    }

    /// Таймер. `size`: 0 — низкая карточка (круг, Старт/Сброс, без ввода времени и пресетов),
    /// 1 и 2 — полная; в широкой содержимое центрируется по ширине.
    pub(super) unsafe fn timer_card(&mut self, ui: &mut Ui, cv: &Canvas, tm: Rect, size: usize) {
        let f = &cv.g.fonts;
        cv.card(tm);
        let compact = size == 0;
        // Содержимое рассчитано на 230 px по ширине; лишнее место делим пополам.
        let ox = ((tm.w - 230.0) / 2.0).max(0.0);
        let rem = self.remaining();
        let frac = if self.total.is_zero() { 0.0 } else { rem.as_secs_f32() / self.total.as_secs_f32() };
        self.ring.target = frac.clamp(0.0, 1.0);
        let (cx, cy) = if compact { (tm.x + ox + 42.0, tm.cy()) } else { (tm.x + ox + 40.0, tm.y + 62.0) };
        // Поля времени (ч, мин, с): ввод цифр, Enter или уход из поля применяет.
        let tf = [id(TODAY, 110), id(TODAY, 111), id(TODAY, 112)];
        let focused = (0..3).find(|&k| ui.has_focus(tf[k]));
        let entered = (0..3).any(|k| ui.take_submitted(tf[k]).is_some());
        for &t in &tf {
            ui.set_digits(t, true);
        }
        // Применяем при Enter и при любом уходе из поля (в том числе в соседнее).
        if entered || (self.time_focus.is_some() && focused != self.time_focus) {
            let n: Vec<u32> = tf.iter().map(|&t| parse_num(&ui.field_text(t))).collect();
            self.preset = None;
            self.set_time_hms(n[0], n[1], n[2]);
            if entered {
                ui.blur();
            }
        }
        if let Some(k) = focused.filter(|_| focused != self.time_focus) {
            // Новое поле: показать нормализованное значение и выделить его, чтобы ввод заменил текст.
            let v = [self.hours, self.minutes, self.seconds][k];
            ui.set_field_text(tf[k], &v.to_string());
            ui.select_all(tf[k]);
        }
        self.time_focus = focused;
        if compact && focused.is_some() {
            // Полей времени в этом виде нет: ввод завершается (значение применится в следующем кадре).
            ui.blur();
        }
        let radius = if compact { 22.0 } else { 24.0 };
        cv.ring(cx, cy, radius, 3.0, self.ring.value, ORANGE);
        let secs = rem.as_secs_f32().ceil() as u64;
        let label = if secs >= 3600 {
            format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
        } else {
            format!("{:02}:{:02}", secs / 60, secs % 60)
        };
        cv.text(&label, Rect::new(cx - 24.0, cy - 8.0, 48.0, 16.0), &f.body_c, txt());
        if ui.icon_button(cv, id(TODAY, 21), Rect::new(tm.x + ox + 5.0, tm.y + 5.0, 16.0, 16.0), icon::RINGER, self.sound) {
            self.sound = !self.sound;
        }

        if compact {
            let by = tm.cy() - 13.0;
            let bx = tm.x + ox + 78.0;
            let start = if self.deadline.is_some() { "Пауза" } else { "Старт" };
            if ui.solid_button(cv, id(TODAY, 40), Rect::new(bx, by, 68.0, 26.0), start, ORANGE, ink(), 6.0) {
                self.toggle_run();
            }
            if ui.solid_button(cv, id(TODAY, 41), Rect::new(bx + 74.0, by, 68.0, 26.0), "Сброс", chip_bg(), txt(), 6.0) {
                self.left = self.total;
                self.deadline = None;
            }
            return;
        }

        // Часы, минуты, секунды: поле ввода; колесо мыши меняет значение.
        let bx = tm.x + ox + 82.0;
        let vals = [self.hours, self.minutes, self.seconds];
        let units = [crate::i18n::pick("ч", "h"), crate::i18n::pick("м", "m"), crate::i18n::pick("с", "s")];
        for k in 0..3 {
            let b = Rect::new(bx + k as f32 * 49.0, tm.y + 14.0, 46.0, 24.0);
            if self.time_focus != Some(k) {
                ui.set_field_text(tf[k], &if k == 0 { vals[k].to_string() } else { format!("{:02}", vals[k]) });
            }
            ui.text_field_at(cv, tf[k], b, "0", "", Submit::Keep, 8.0, 16.0);
            cv.text(units[k], Rect::new(b.right() - 15.0, b.y + 1.0, 12.0, b.h), &f.small, dim());
            let wheel = ui.wheel_in(b);
            if wheel != 0.0 {
                let d = if wheel > 0.0 { 1i32 } else { -1 };
                let mut v = [self.hours as i32, self.minutes as i32, self.seconds as i32];
                v[k] = (v[k] + d).max(0);
                self.preset = None;
                self.set_time_hms(v[0] as u32, v[1] as u32, v[2] as u32);
            }
        }

        // Пресеты.
        for (i, m) in PRESETS.iter().enumerate() {
            let r = Rect::new(bx + i as f32 * 29.0, tm.y + 48.0, 25.0, 18.0);
            if ui.chip_ex(cv, id(TODAY, i as u32), r, &format!("{m}{}", crate::i18n::pick("м", "m")), self.preset == Some(i), false, Some(ORANGE)) {
                self.set_preset(i);
            }
        }

        // Старт / пауза и сброс.
        let start = if self.deadline.is_some() { "Пауза" } else { "Старт" };
        if ui.solid_button(cv, id(TODAY, 40), Rect::new(bx, tm.y + 76.0, 68.0, 26.0), start, ORANGE, ink(), 6.0) {
            self.toggle_run();
        }
        if ui.solid_button(cv, id(TODAY, 41), Rect::new(bx + 74.0, tm.y + 76.0, 68.0, 26.0), "Сброс", chip_bg(), txt(), 6.0) {
            self.left = self.total;
            self.deadline = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_pause_resume() {
        let mut t = Today::default();
        t.toggle_run();
        assert!(t.deadline.is_some());
        t.toggle_run();
        assert!(t.deadline.is_none());
        assert!(t.left <= t.total && t.left > Duration::from_secs(290));
        t.set_preset(0);
        assert_eq!(t.left, Duration::from_secs(60));
    }

    #[test]
    fn manual_time() {
        let mut t = Today::default();
        t.set_time(1, 30);
        assert_eq!(t.total, Duration::from_secs(90 * 60));
        t.set_time(0, 0);
        assert_eq!(t.total, Duration::from_secs(60));
        t.set_time(20, 99);
        assert_eq!((t.hours, t.minutes), (9, 59));
    }

    #[test]
    fn number_parsing() {
        assert_eq!(parse_num("25"), 25);
        assert_eq!(parse_num(" 7 "), 7);
        assert_eq!(parse_num(""), 0);
        assert_eq!(parse_num("x"), 0);
        assert_eq!(parse_num("99999999999"), 99_999);
    }

    #[test]
    fn hms_normalizes() {
        let mut t = Today::default();
        t.set_time_hms(0, 90, 75);
        assert_eq!((t.hours, t.minutes, t.seconds), (1, 31, 15));
        t.set_time_hms(0, 0, 0);
        assert_eq!(t.total, Duration::from_secs(60));
        t.set_time_hms(0, 0, 30);
        assert_eq!(t.total, Duration::from_secs(30));
        t.set_time_hms(50, 0, 0);
        assert_eq!((t.hours, t.minutes, t.seconds), (9, 59, 59));
    }

    #[test]
    fn finished_timer_resets() {
        let mut t = Today::default();
        t.sound = false;
        t.deadline = Some(Instant::now() - Duration::from_millis(1));
        t.city.clear();
        t.poll();
        assert!(t.deadline.is_none());
        assert_eq!(t.left, t.total);
        assert!(t.take_fired());
        assert!(!t.take_fired());
    }
}
