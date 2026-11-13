//! Таймер: время, пресеты, пауза и сброс, карточка таймера.

use std::time::{Duration, Instant};

use super::Today;
use crate::tabs::{id, TODAY};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::ui::{chip_bg, dim, icon, mix, txt, Align, Canvas, Corner, Rect, Submit, Ui};

const PRESETS: [u32; 5] = [1, 5, 10, 15, 20];
static PRESET_LABELS: [&str; 5] = ["1", "5", "10", "15", "20"];

// ID таймера на вкладке: пресеты, звук, циферблат, пуск/сброс и поля часов/минут/секунд.
const ID_PRESET: u32 = 0;
const ID_SOUND: u32 = 21;
const ID_DIGITS: u32 = 39;
const ID_RUN: u32 = 40;
const ID_RESET: u32 = 41;
const ID_FIELDS: u32 = 110;

/// Занятые таймером ID вкладки (полуинтервалы) для теста непересечения в `mod.rs`.
#[cfg(test)]
pub(super) const ID_RANGES: &[(u32, u32)] = &[
    (ID_PRESET, ID_PRESET + PRESETS.len() as u32),
    (ID_SOUND, ID_SOUND + 1),
    (ID_DIGITS, ID_DIGITS + 1),
    (ID_RUN, ID_RESET + 1),
    (ID_FIELDS, ID_FIELDS + 3),
];

/// Время таймера: «03:25» или «1:02:05».
fn clock_label(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

/// Подпись общей длительности для карточки 4x2: «из 5:00» или «из 1:02:05».
fn total_label_4x2(secs: u64) -> String {
    let of = crate::i18n::pick("из", "of");
    if secs >= 3600 {
        format!("{of} {}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{of} {}:{:02}", secs / 60, secs % 60)
    }
}

/// Подпись общей длительности для карточки 6x2: «из 5 мин», «из 1:30», «из 1 ч».
fn total_label_6x2(secs: u64) -> String {
    let of = crate::i18n::pick("из", "of");
    if secs % 60 == 0 && secs < 3600 {
        format!("{of} {} {}", secs / 60, crate::i18n::pick("мин", "min"))
    } else if secs % 3600 == 0 {
        format!("{of} {} {}", secs / 3600, crate::i18n::pick("ч", "h"))
    } else if secs >= 3600 {
        format!("{of} {}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{of} {}:{:02}", secs / 60, secs % 60)
    }
}

/// Круглая кнопка со значком (пуск, сброс). `true` — нажата.
#[allow(clippy::too_many_arguments)]
unsafe fn round_button(
    ui: &mut Ui,
    cv: &Canvas,
    id: u32,
    cx: f32,
    cy: f32,
    rad: f32,
    glyph: &str,
    fill: D2D1_COLOR_F,
    fg: D2D1_COLOR_F,
    tip: &str,
) -> bool {
    let r = Rect::new(cx - rad, cy - rad, 2.0 * rad, 2.0 * rad);
    let (_, clicked) = ui.click_area(id, r);
    let hv = ui.hover_of(id);
    cv.circle(cx, cy, rad, mix(fill, crate::ui::fg(), 0.08 * hv));
    cv.glyph(glyph, r, false, fg);
    if !tip.is_empty() {
        ui.tip(id, r, tip);
    }
    clicked
}

/// Число из текста поля: берутся только цифры, пусто — 0.
fn parse_num(s: &str) -> u32 {
    s.chars().filter_map(|c| c.to_digit(10)).fold(0u32, |a, d| a.saturating_mul(10).saturating_add(d)).min(99_999)
}

/// Поля ч:м:с (боксы 24x24 с ":" между ними) для режима правки времени.
unsafe fn render_time_fields(
    today: &mut Today,
    ui: &mut Ui,
    cv: &Canvas,
    tf: [u32; 3],
    tx: f32,
    by: f32,
) {
    let f = &cv.g.fonts;
    let vals = [today.hours, today.minutes, today.seconds];
    let (box_w, box_h, gap) = (24.0f32, 24.0f32, 6.0f32);
    let mut wheel_handled = false;
    for (slot, k) in (0..3).enumerate() {
        let bx = tx + slot as f32 * (box_w + gap);
        let b = Rect::new(bx, by, box_w, box_h);
        if today.time_focus != Some(k) {
            ui.set_field_text(tf[k], &if k == 0 { vals[k].to_string() } else { format!("{:02}", vals[k]) });
        }
        ui.text_field_at(cv, tf[k], b, "00", "", Submit::Keep, 0.0, 0.0);
        if k < 2 {
            cv.text(":", Rect::new(b.right(), by - 1.0, gap, box_h), &f.mono_c, dim());
        }
        let w = ui.wheel_in(b);
        if w != 0.0 {
            today.wheel_time(k, w);
            wheel_handled = true;
        }
    }
    if !wheel_handled {
        let w = ui.wheel_in(Rect::new(tx, by, 3.0 * box_w + 2.0 * gap, box_h));
        if w != 0.0 {
            today.wheel_time(today.time_focus.unwrap_or(1), w);
        }
    }
}

/// Клик и колесо мыши над цифрами времени / кольцом: пауза (если шёл) и фокус на поле минут.
unsafe fn handle_digits_interaction(today: &mut Today, ui: &mut Ui, tf: [u32; 3], area: Rect) {
    let (_, clicked) = ui.click_area(id(TODAY, ID_DIGITS), area);
    if clicked {
        if today.deadline.is_some() {
            today.toggle_run();
        }
        ui.set_field_text(tf[1], &format!("{:02}", today.minutes));
        ui.select_all(tf[1]);
        ui.focus = Some(tf[1]);
        ui.caret_on = true;
        today.time_focus = Some(1);
    }
    let wheel = ui.wheel_in(area);
    today.wheel_time(1, wheel);
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

    /// Колёсико мыши меняет часы (0), минуты (1) или секунды (2).
    pub(crate) fn wheel_time(&mut self, k: usize, wheel: f32) {
        if wheel != 0.0 {
            let d = if wheel > 0.0 { 1i32 } else { -1 };
            let mut v = [self.hours as i32, self.minutes as i32, self.seconds as i32];
            v[k] = (v[k] + d).max(0);
            self.preset = None;
            self.set_time_hms(v[0] as u32, v[1] as u32, v[2] as u32);
        }
    }

    /// Карточка таймера по макету:
    /// - 4x1 (216x48): кольцо с колокольчиком звука, крупные цифры времени и круглые кнопки Сброс / Старт-Пауза.
    /// - 4x2 (216x104): шапка с «ТАЙМЕР» и общей длительностью, огромное время с кнопками и 30 сегментов внизу.
    /// - 6x2 (328x104): кольцо с временем слева, шапка «ПРЕСЕТЫ», 5 чипов-пресетов, широкая кнопка Старт/Пауза и сброс.
    pub(super) unsafe fn timer_card(&mut self, ui: &mut Ui, cv: &Canvas, tm: Rect, size: usize) {
        let f = &cv.g.fonts;
        // Оранжевый акцент таймера и мягкий оранжевый ореол в левом верхнем углу карточки.
        let tint = crate::ui::status_orange();
        cv.card(tm);
        cv.glow(tm, 14.0, tint, Corner::TopLeft);

        let rem = self.remaining();
        let frac = if self.total.is_zero() { 0.0 } else { rem.as_secs_f32() / self.total.as_secs_f32() };
        self.ring.target = frac.clamp(0.0, 1.0);

        // Поля времени (ч, мин, с): ввод цифр, Enter или уход из поля применяет.
        let tf = [id(TODAY, ID_FIELDS), id(TODAY, ID_FIELDS + 1), id(TODAY, ID_FIELDS + 2)];
        let mut focused = (0..3).find(|&k| ui.has_focus(tf[k]));
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
                focused = None;
            }
        }
        if let Some(k) = focused.filter(|_| focused != self.time_focus) {
            // Новое поле: показать нормализованное значение и выделить его, чтобы ввод заменил текст.
            let v = [self.hours, self.minutes, self.seconds][k];
            ui.set_field_text(tf[k], &v.to_string());
            ui.select_all(tf[k]);
        }
        self.time_focus = focused;

        let secs = rem.as_secs_f32().ceil() as u64;
        let label = clock_label(secs);
        let p = 12.0f32;

        let sound_tip = if self.sound {
            crate::i18n::pick("Выключить звук сигнала", "Mute alarm sound")
        } else {
            crate::i18n::pick("Включить звук сигнала", "Unmute alarm sound")
        };
        let run_glyph = if self.deadline.is_some() { icon::PAUSE } else { icon::PLAY_SOLID };
        let run_tip = if self.deadline.is_some() {
            crate::i18n::pick("Пауза", "Pause")
        } else {
            crate::i18n::pick("Старт", "Start")
        };
        let reset_tip = crate::i18n::pick("Сброс", "Reset");

        if size == 0 || tm.h < 65.0 {
            // 4x1 (216x48): кольцо с колокольчиком звука внутри, правее время, справа кнопки сброс и пуск/пауза.
            let r = 13.0f32;
            let (rx, cy) = (tm.x + p + r, tm.cy());
            cv.ring(rx, cy, r, 3.0, self.ring.value, tint);
            let bell_r = Rect::new(rx - 9.0, cy - 9.0, 18.0, 18.0);
            if ui.icon_button_tinted(cv, id(TODAY, ID_SOUND), bell_r, icon::RINGER, self.sound, tint, sound_tip) {
                self.sound = !self.sound;
                self.save_timer();
            }

            let btn_rad = 14.0f32;
            let run_x = tm.right() - p - btn_rad;
            let reset_x = run_x - 2.0 * btn_rad - 6.0;

            if round_button(ui, cv, id(TODAY, ID_RESET), reset_x, cy, btn_rad, icon::RESTART, chip_bg(), txt(), reset_tip) {
                self.left = self.total;
                self.deadline = None;
                self.save_timer();
            }
            if round_button(ui, cv, id(TODAY, ID_RUN), run_x, cy, btn_rad, run_glyph, tint, crate::ui::WHITE, run_tip) {
                self.toggle_run();
            }

            let tx = rx + r + 10.0;
            let avail_w = (reset_x - btn_rad - 6.0 - tx).max(60.0);
            if self.time_focus.is_some() {
                // В режиме правки вместо цифр показываем поля ввода ч:м:с.
                render_time_fields(self, ui, cv, tf, tx, cy - 12.0);
            } else {
                let digit_size = if secs >= 3600 { 18.0 } else { 22.0 };
                cv.digits(&label, tx, avail_w, cy, digit_size, 600, Align::Leading, txt());
                let digits_r = Rect::new(tx, tm.y + 4.0, avail_w, tm.h - 8.0);
                handle_digits_interaction(self, ui, tf, digits_r);
            }
        } else if size == 2 || tm.w > 260.0 {
            // 6x2 (328x104): большое кольцо с временем и «из 5 мин» слева, шапка «ПРЕСЕТЫ», чипы и кнопки справа.
            let ring_rad = 38.0f32;
            let (rx, cy) = (tm.x + p + ring_rad, tm.cy());
            cv.ring(rx, cy, ring_rad, 5.0, self.ring.value, tint);

            let time_size = if secs >= 3600 { 16.0 } else { 20.0 };
            cv.digits(&label, rx - 35.0, 70.0, cy - 7.0, time_size, 600, Align::Center, txt());
            let sub_text = total_label_6x2(self.total.as_secs());
            cv.line(&sub_text, &f.small, rx - 35.0, 70.0, cy + 13.0, Align::Center, dim());

            let ring_area = Rect::new(rx - ring_rad, cy - ring_rad, 2.0 * ring_rad, 2.0 * ring_rad);
            handle_digits_interaction(self, ui, tf, ring_area);

            let bx = rx + ring_rad + 16.0;
            let bw = (tm.right() - p - bx).max(120.0);

            // Шапка «ПРЕСЕТЫ» и колокольчик звука.
            let bell_r = Rect::new(tm.right() - p - 18.0, tm.y + 7.0, 18.0, 18.0);
            cv.line(crate::i18n::pick("ПРЕСЕТЫ", "PRESETS"), &f.caps, bx, bell_r.x - 6.0 - bx, tm.y + 16.0, Align::Leading, dim());
            if ui.icon_button_tinted(cv, id(TODAY, ID_SOUND), bell_r, icon::RINGER, self.sound, tint, sound_tip) {
                self.sound = !self.sound;
                self.save_timer();
            }

            // Ряд пресетов; в режиме правки подменяется на поля ввода ч:м:с.
            if self.time_focus.is_some() {
                render_time_fields(self, ui, cv, tf, bx, tm.y + 34.0);
            } else {
                let n_presets = PRESETS.len();
                let gap = 6.0f32;
                let chip_w = ((bw - (n_presets - 1) as f32 * gap) / n_presets as f32).floor();
                for (i, label) in PRESET_LABELS.iter().enumerate() {
                    let cx = bx + i as f32 * (chip_w + gap);
                    let cr = Rect::new(cx, tm.y + 36.0, chip_w, 20.0);
                    if ui.chip_ex(cv, id(TODAY, ID_PRESET + i as u32), cr, label, self.preset == Some(i), false, Some(tint)) {
                        self.set_preset(i);
                    }
                }
            }

            // Нижний ряд: широкая кнопка-пилюля «Пауза»/«Старт» и круглая кнопка сброса.
            let rad = 14.0f32;
            let reset_cx = tm.right() - p - rad;
            let reset_cy = tm.y + 66.0 + rad;
            if round_button(ui, cv, id(TODAY, ID_RESET), reset_cx, reset_cy, rad, icon::RESTART, chip_bg(), txt(), reset_tip) {
                self.left = self.total;
                self.deadline = None;
                self.save_timer();
            }

            let pill_w = (reset_cx - rad - 8.0 - bx).max(60.0);
            let pill_r = Rect::new(bx, tm.y + 66.0, pill_w, 28.0);
            let (_, clicked) = ui.click_area(id(TODAY, ID_RUN), pill_r);
            let hv = ui.hover_of(id(TODAY, ID_RUN));
            cv.rr(pill_r, pill_r.h / 2.0, mix(tint, crate::ui::fg(), 0.08 * hv));

            let text_w = cv.text_w(run_tip, &f.small);
            let icon_sz = 14.0f32;
            let gap = 6.0f32;
            let content_w = icon_sz + gap + text_w;
            let start_x = pill_r.x + (pill_r.w - content_w).max(0.0) / 2.0;
            cv.glyph(run_glyph, Rect::new(start_x, pill_r.cy() - icon_sz / 2.0, icon_sz, icon_sz), false, crate::ui::WHITE);
            cv.line(run_tip, &f.small, start_x + icon_sz + gap, text_w + 2.0, pill_r.cy(), Align::Leading, crate::ui::WHITE);
            ui.tip(id(TODAY, ID_RUN), pill_r, run_tip);
            if clicked {
                self.toggle_run();
            }
        } else {
            // 4x2 (216x104): шапка с «ТАЙМЕР» и «из 5:00», крупное время с кнопками, 30 сегментов внизу.
            let bell_r = Rect::new(tm.right() - p - 16.0, tm.y + 8.0, 16.0, 16.0);
            if ui.icon_button_tinted(cv, id(TODAY, ID_SOUND), bell_r, icon::RINGER, self.sound, tint, sound_tip) {
                self.sound = !self.sound;
                self.save_timer();
            }

            let of_str = total_label_4x2(self.total.as_secs());
            let of_w = cv.text_w(&of_str, &f.small);
            cv.line(&of_str, &f.small, bell_r.x - 6.0 - of_w, of_w + 2.0, tm.y + 16.0, Align::Leading, dim());
            cv.line(crate::i18n::pick("ТАЙМЕР", "TIMER"), &f.caps, tm.x + p, (bell_r.x - 12.0 - of_w - (tm.x + p)).max(40.0), tm.y + 16.0, Align::Leading, dim());

            let btn_rad = 15.0f32;
            let cy = tm.y + 51.0;
            let run_x = tm.right() - p - btn_rad;
            let reset_x = run_x - 2.0 * btn_rad - 6.0;

            if round_button(ui, cv, id(TODAY, ID_RESET), reset_x, cy, btn_rad, icon::RESTART, chip_bg(), txt(), reset_tip) {
                self.left = self.total;
                self.deadline = None;
                self.save_timer();
            }
            if round_button(ui, cv, id(TODAY, ID_RUN), run_x, cy, btn_rad, run_glyph, tint, crate::ui::WHITE, run_tip) {
                self.toggle_run();
            }

            let tx = tm.x + p;
            let avail_w = (reset_x - btn_rad - 6.0 - tx).max(60.0);
            if self.time_focus.is_some() {
                // В режиме правки вместо цифр показываем поля ввода ч:м:с.
                render_time_fields(self, ui, cv, tf, tx, cy - 12.0);
            } else {
                let font_sz = if secs >= 3600 { 28.0 } else { 38.0 };
                cv.digits(&label, tx, avail_w, cy, font_sz, 600, Align::Leading, txt());
                let digits_r = Rect::new(tx, cy - 20.0, avail_w, 40.0);
                handle_digits_interaction(self, ui, tf, digits_r);
            }

            if self.deadline.is_none() && self.left == self.total {
                // Таймер ещё не запущен: внизу ряд пресетов (по умолчанию именно этот размер), как в 6×2.
                let n_presets = PRESETS.len();
                let gap = 6.0f32;
                let row_w = tm.w - 2.0 * p;
                let chip_w = ((row_w - (n_presets - 1) as f32 * gap) / n_presets as f32).floor();
                for (i, label) in PRESET_LABELS.iter().enumerate() {
                    let cr = Rect::new(tm.x + p + i as f32 * (chip_w + gap), tm.bottom() - p - 20.0, chip_w, 20.0);
                    if ui.chip_ex(cv, id(TODAY, ID_PRESET + i as u32), cr, label, self.preset == Some(i), false, Some(tint)) {
                        self.set_preset(i);
                    }
                }
            } else {
                // Идёт отсчёт или пауза: полоса из 30 сегментов внизу карточки (оставшееся время).
                let seg_h = 10.0f32;
                let seg_r = Rect::new(tm.x + p, tm.bottom() - p - seg_h, tm.w - 2.0 * p, seg_h);
                let filled = (frac * 30.0).round().clamp(0.0, 30.0) as usize;
                cv.segments(seg_r, 30, filled, tint);
            }
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
    fn wheel_time_compact_and_full() {
        let mut t = Today::default();
        t.set_time(0, 5);
        assert_eq!(t.total, Duration::from_secs(5 * 60));
        assert_eq!(t.left, Duration::from_secs(5 * 60));

        // Колёсико мыши в компактном виде (k == 1, минуты) увеличивает время
        t.wheel_time(1, 1.0);
        assert_eq!(t.minutes, 6);
        assert_eq!(t.total, Duration::from_secs(6 * 60));
        assert_eq!(t.left, Duration::from_secs(6 * 60));

        // Колёсико мыши вниз уменьшает время
        t.wheel_time(1, -1.0);
        assert_eq!(t.minutes, 5);
        assert_eq!(t.total, Duration::from_secs(5 * 60));
        assert_eq!(t.left, Duration::from_secs(5 * 60));

        // Сброс пресета при прокрутке колёсиком
        t.preset = Some(0);
        t.wheel_time(1, 1.0);
        assert_eq!(t.preset, None);

        // В полном виде k == 0 (часы) и k == 2 (секунды) также меняют время
        t.wheel_time(0, 1.0);
        assert_eq!(t.hours, 1);
        assert_eq!(t.minutes, 6);
        assert_eq!(t.total, Duration::from_secs(3600 + 6 * 60));

        t.wheel_time(2, 30.0);
        assert_eq!(t.seconds, 1);
        assert_eq!(t.total, Duration::from_secs(3600 + 6 * 60 + 1));
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
