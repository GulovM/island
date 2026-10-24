//! Таймер: время, пресеты, пауза и сброс, карточка таймера.

use std::time::{Duration, Instant};

use super::Today;
use crate::tabs::{id, TODAY};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::ui::{chip_bg, dim, icon, mix, txt, Align, Canvas, Rect, Submit, Ui};

const PRESETS: [u32; 5] = [1, 5, 10, 15, 20];

/// Время таймера: «03:25» или «1:02:05».
fn clock_label(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

/// Круглая кнопка со значком (пуск, сброс). `true` — нажата.
#[allow(clippy::too_many_arguments)]
unsafe fn round_button(ui: &mut Ui, cv: &Canvas, id: u32, cx: f32, cy: f32, rad: f32, glyph: &str, fill: D2D1_COLOR_F, fg: D2D1_COLOR_F) -> bool {
    let r = Rect::new(cx - rad, cy - rad, 2.0 * rad, 2.0 * rad);
    let (_, clicked) = ui.click_area(id, r);
    let hv = ui.hover_of(id);
    cv.circle(cx, cy, rad, mix(fill, crate::ui::fg(), 0.08 * hv));
    cv.glyph(glyph, r, false, fg);
    clicked
}

/// Число из текста поля: берутся только цифры, пусто — 0.
fn parse_num(s: &str) -> u32 {
    s.chars().filter_map(|c| c.to_digit(10)).fold(0u32, |a, d| a.saturating_mul(10).saturating_add(d)).min(99_999)
}

impl Today {
    pub(super) fn save_timer(&self) {
        let finish = self.deadline.map(|d| {
            let rem = d.saturating_duration_since(Instant::now()).as_secs();
            crate::sys::now_secs().saturating_add(rem)
        }).unwrap_or(0);
        crate::state::set_timer(crate::state::TimerState {
            duration: self.total.as_secs(),
            left: self.left.as_secs(),
            finish,
            preset: self.preset,
            sound: self.sound,
        });
    }

    pub fn load_timer(&mut self, state: &crate::state::State) {
        let now_unix = crate::sys::now_secs();
        let duration = state.timer.duration.max(1);
        self.total = Duration::from_secs(duration);
        self.preset = state.timer.preset;
        self.sound = state.timer.sound;

        let finish = state.timer.finish;
        if finish > 0 {
            if now_unix < finish {
                let rem = Duration::from_secs(finish - now_unix);
                self.left = rem;
                self.deadline = Some(Instant::now() + rem);
                self.fired = false;
            } else {
                self.left = self.total;
                self.deadline = None;
                self.fired = true;
            }
        } else {
            let left = state.timer.left.min(duration).max(1);
            self.left = Duration::from_secs(left);
            self.deadline = None;
            self.fired = false;
        }

        let total_secs = self.total.as_secs() as u32;
        self.hours = total_secs / 3600;
        self.minutes = total_secs / 60 % 60;
        self.seconds = total_secs % 60;
    }

    fn remaining(&self) -> Duration {
        match self.deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => self.left,
        }
    }

    /// Состояние таймера для свёрнутого острова: идёт или стоит на паузе после старта.
    /// Нетронутый таймер (остаток равен полному времени) острову не показывается.
    pub fn view(&self) -> Option<crate::live::TimerView> {
        if self.total.is_zero() {
            return None;
        }
        let paused = self.deadline.is_none();
        if paused && (self.left >= self.total || self.left.is_zero()) {
            return None;
        }
        let rem = self.remaining();
        let secs = rem.as_secs_f32().ceil() as u64;
        // Секунда на экране меняется, когда остаток переходит через целое число секунд.
        let sub_ms = (rem.as_millis() % 1000) as u32;
        Some(crate::live::TimerView {
            frac: (rem.as_secs_f32() / self.total.as_secs_f32()).clamp(0.0, 1.0),
            secs,
            paused,
            next_ms: if sub_ms == 0 { 1000 } else { sub_ms } + 15,
        })
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
        self.save_timer();
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
        self.save_timer();
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
        // Содержимое рассчитано на 230 px по ширине (в узкой карточке 4×2 — на 208 px, элементы
        // чуть теснее); лишнее место делим пополам.
        let narrow = tm.w < 224.0;
        let ox = ((tm.w - if narrow { 208.0 } else { 230.0 }) / 2.0).max(0.0);
        // Сдвиги кольца и блока справа от него, шаги полей времени и пресетов, ширина кнопок.
        let (cx_off, bx_off, f_step, p_step, btn_w) = if narrow { (35.0, 66.0, 46.0, 27.0, 66.0) } else { (40.0, 82.0, 49.0, 29.0, 68.0) };
        let rem = self.remaining();
        let frac = if self.total.is_zero() { 0.0 } else { rem.as_secs_f32() / self.total.as_secs_f32() };
        self.ring.target = frac.clamp(0.0, 1.0);
        let (cx, cy) = (tm.x + ox + cx_off, tm.y + 46.0);
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
        let secs = rem.as_secs_f32().ceil() as u64;
        let label = clock_label(secs);
        // Цвет таймера — оранжевый, как его полоска на свёрнутом острове.
        let tint = crate::ui::status_orange();

        if compact {
            // Полоса 248×56: кольцо, остаток крупно и «из 05:00», справа круглые кнопки.
            const P: f32 = 12.0;
            let r = tm.h / 2.0 - 10.0;
            let (rx, cy) = (tm.x + P + r, tm.cy());
            cv.ring(rx, cy, r, 3.0, self.ring.value, tint);
            cv.glyph(icon::CLOCK, Rect::new(rx - 8.0, cy - 8.0, 16.0, 16.0), false, tint);
            let tx = rx + r + 10.0;
            cv.digits(&label, tx - 1.0, 90.0, cy - 6.0, 22.0, 600, Align::Leading, txt());
            let of = format!("{} {}", crate::i18n::pick("из", "of"), clock_label(self.total.as_secs()));
            cv.line(&of, &f.small, tx, 90.0, cy + 13.0, Align::Leading, dim());
            let reset_x = tm.right() - P - 14.0;
            let run_x = reset_x - 34.0;
            let run_icon = if self.deadline.is_some() { icon::PAUSE } else { icon::PLAY_SOLID };
            if round_button(ui, cv, id(TODAY, 40), run_x, cy, 14.0, run_icon, tint, crate::ui::WHITE) {
                self.toggle_run();
            }
            if round_button(ui, cv, id(TODAY, 41), reset_x, cy, 14.0, icon::RESTART, chip_bg(), txt()) {
                self.left = self.total;
                self.deadline = None;
                self.save_timer();
            }
            if ui.icon_button_tinted(cv, id(TODAY, 21), Rect::new(run_x - 14.0 - 26.0, cy - 10.0, 20.0, 20.0), icon::RINGER, self.sound, tint) {
                self.sound = !self.sound;
                self.save_timer();
            }
            return;
        }

        cv.ring(cx, cy, 25.0, 3.5, self.ring.value, tint);
        // Оставшееся время — крупными табличными цифрами: главное в карточке.
        cv.text(&label, Rect::new(cx - 25.0, cy - 10.0, 50.0, 20.0), &f.title_c, txt());
        // Звук сигнала — под кольцом, по его оси.
        if ui.icon_button_tinted(cv, id(TODAY, 21), Rect::new(cx - 9.0, tm.y + 77.0, 18.0, 18.0), icon::RINGER, self.sound, tint) {
            self.sound = !self.sound;
            self.save_timer();
        }

        // Часы, минуты, секунды: поле ввода; колесо мыши меняет значение.
        let bx = tm.x + ox + bx_off;
        let vals = [self.hours, self.minutes, self.seconds];
        let units = [crate::i18n::pick("ч", "h"), crate::i18n::pick("м", "m"), crate::i18n::pick("с", "s")];
        for k in 0..3 {
            let b = Rect::new(bx + k as f32 * f_step, tm.y + 10.0, f_step - 3.0, 24.0);
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
            let r = Rect::new(bx + i as f32 * p_step, tm.y + 42.0, p_step - 4.0, 18.0);
            if ui.chip_ex(cv, id(TODAY, i as u32), r, &format!("{m}{}", crate::i18n::pick("м", "m")), self.preset == Some(i), false, Some(tint)) {
                self.set_preset(i);
            }
        }

        // Старт / пауза и сброс.
        let start = if self.deadline.is_some() { "Пауза" } else { "Старт" };
        if ui.solid_button(cv, id(TODAY, 40), Rect::new(bx, tm.y + 68.0, btn_w, 26.0), start, tint, crate::ui::WHITE, 6.0) {
            self.toggle_run();
        }
        if ui.solid_button(cv, id(TODAY, 41), Rect::new(bx + btn_w + 6.0, tm.y + 68.0, btn_w, 26.0), "Сброс", chip_bg(), txt(), 6.0) {
            self.left = self.total;
            self.deadline = None;
            self.save_timer();
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
    fn view_hidden_until_started() {
        let mut t = Today::default();
        assert!(t.view().is_none(), "нетронутый таймер острову не нужен");
        t.toggle_run();
        let v = t.view().unwrap();
        assert!(!v.paused && v.frac > 0.99 && v.next_ms <= 1015);
        t.toggle_run();
        let v = t.view().unwrap();
        assert!(v.paused, "после паузы таймер остаётся виден");
        t.left = t.total;
        assert!(t.view().is_none(), "сброс убирает таймер с острова");
    }

    #[test]
    fn view_fraction_follows_remaining() {
        let mut t = Today::default();
        t.total = Duration::from_secs(100);
        t.left = Duration::from_secs(25);
        let v = t.view().unwrap();
        assert!((v.frac - 0.25).abs() < 0.001 && v.secs == 25 && v.paused);
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

    #[test]
    fn timer_load_active_countdown() {
        let mut t = Today::default();
        let now = crate::sys::now_secs();
        let state = crate::state::State {
            timer: crate::state::TimerState {
                duration: 600,
                left: 300,
                finish: now + 200,
                preset: Some(2),
                sound: false,
            },
            ..Default::default()
        };
        t.load_timer(&state);
        assert!(t.deadline.is_some());
        assert!(!t.fired);
        assert!(!t.take_fired());
        assert!(!t.sound);
        assert_eq!(t.preset, Some(2));
        assert_eq!(t.total, Duration::from_secs(600));
        assert!(t.remaining() <= Duration::from_secs(200) && t.remaining() > Duration::from_secs(195));
    }

    #[test]
    fn timer_load_expired_shows_fired_once() {
        let mut t = Today::default();
        let now = crate::sys::now_secs();
        let state = crate::state::State {
            timer: crate::state::TimerState {
                duration: 300,
                left: 100,
                finish: now.saturating_sub(60), // закончился 60 секунд назад
                preset: Some(1),
                sound: true,
            },
            ..Default::default()
        };
        t.load_timer(&state);
        assert!(t.deadline.is_none());
        assert_eq!(t.left, Duration::from_secs(300));
        assert!(t.take_fired());
        assert!(!t.take_fired());
    }

    #[test]
    fn timer_load_paused() {
        let mut t = Today::default();
        let state = crate::state::State {
            timer: crate::state::TimerState {
                duration: 900,
                left: 450,
                finish: 0,
                preset: None,
                sound: true,
            },
            ..Default::default()
        };
        t.load_timer(&state);
        assert!(t.deadline.is_none());
        assert!(!t.fired);
        assert!(!t.take_fired());
        assert_eq!(t.total, Duration::from_secs(900));
        assert_eq!(t.left, Duration::from_secs(450));
        assert_eq!(t.preset, None);
        assert_eq!((t.hours, t.minutes, t.seconds), (0, 15, 0));
    }
}
