//! Таймер и режим Pomodoro: время, пресеты, циклы работа/перерыв, пауза и сброс, карточка таймера.

use std::time::{Duration, Instant};

use super::Today;
use crate::tabs::{id, TODAY};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE;

use crate::ui::{
    chip_bg, dim, fg, hairline, icon, mix, press_scale, readable, status_green,
    status_pomodoro, track, txt, Align, Canvas, Corner, Nav, Rect, Submit, Ui,
};

const PRESETS: [u32; 5] = [1, 5, 10, 15, 20];
static PRESET_LABELS: [&str; 5] = ["1", "5", "10", "15", "20"];

// ID таймера на вкладке: пресеты, звук, циферблат, пуск/сброс, режим/помодоро и поля ввода.
const ID_PRESET: u32 = 0;
const ID_SOUND: u32 = 21;
const ID_DIGITS: u32 = 39;
const ID_RUN: u32 = 40;
const ID_RESET: u32 = 41;
const ID_MODE_TIMER: u32 = 42;
const ID_MODE_POMO: u32 = 43;
const ID_POMO_PARAM: u32 = 44; // 44, 45, 46 (Работа, Перерыв, Циклы)
const ID_POMO_SKIP: u32 = 47;
const ID_FIELDS: u32 = 110;

/// Занятые таймером ID вкладки (полуинтервалы) для теста непересечения в `mod.rs`.
#[cfg(test)]
pub(super) const ID_RANGES: &[(u32, u32)] = &[
    (ID_PRESET, ID_PRESET + PRESETS.len() as u32),
    (ID_SOUND, ID_SOUND + 1),
    (ID_DIGITS, ID_DIGITS + 1),
    (ID_RUN, ID_POMO_SKIP + 1),
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

/// Подпись текущей фазы и цикла Pomodoro: «Фокус · 1/4» / «Отдых · 1/4».
fn pomo_phase_label(is_break: bool, cycle: u32, cycles: u32) -> String {
    let phase = if is_break {
        crate::i18n::pick("Отдых", "Break")
    } else {
        crate::i18n::pick("Фокус", "Focus")
    };
    format!("{phase} · {cycle}/{cycles}")
}

/// Подпись числа циклов с правильным склонением.
fn pomo_cycles_label(n: u32) -> String {
    if crate::i18n::en() {
        if n == 1 {
            "1 cycle".to_string()
        } else {
            format!("{n} cycles")
        }
    } else {
        let word = if n % 10 == 1 && n % 100 != 11 {
            "цикл"
        } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
            "цикла"
        } else {
            "циклов"
        };
        format!("{n} {word}")
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
    fg_col: D2D1_COLOR_F,
    tip: &str,
) -> bool {
    let r = Rect::new(cx - rad, cy - rad, 2.0 * rad, 2.0 * rad);
    let (_, clicked) = ui.click_area(id, r);
    let hv = ui.hover_of(id);
    cv.circle(cx, cy, rad, mix(fill, fg(), 0.08 * hv));
    cv.glyph(glyph, r, false, fg_col);
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

/// Поля правки параметров Pomodoro (Работа, Перерыв, Циклы).
unsafe fn render_pomo_fields(
    today: &mut Today,
    ui: &mut Ui,
    cv: &Canvas,
    tf: [u32; 3],
    area: Rect,
    compact: bool,
    tint: D2D1_COLOR_F,
) {
    let f = &cv.g.fonts;
    let vals = [today.pomo_work, today.pomo_break, today.pomo_cycles];
    let gap = if compact { 4.0f32 } else { 6.0f32 };
    let box_w = ((area.w - 2.0 * gap) / 3.0).floor().max(24.0);
    let box_h = area.h.min(24.0);
    let by = area.cy() - box_h / 2.0;
    let dot_cols = [status_pomodoro(), status_green(), tint];
    let suffixes = [
        crate::i18n::pick("м", "m"),
        crate::i18n::pick("м", "m"),
        "×",
    ];
    let mut wheel_handled = false;
    for k in 0..3 {
        let bx = area.x + k as f32 * (box_w + gap);
        let b = Rect::new(bx, by, box_w, box_h);
        if today.time_focus != Some(k) {
            ui.set_field_text(tf[k], &vals[k].to_string());
        }
        let (left, right) = if compact { (0.0, 0.0) } else { (13.0, 14.0) };
        ui.text_field_at(cv, tf[k], b, "1", "", Submit::Keep, left, right);
        if today.time_focus == Some(k) {
            cv.rr_line(b, 10.0, D2D1_COLOR_F { a: 0.75, ..dot_cols[k] }, 1.2);
        } else {
            cv.hairline(b, 10.0);
        }
        if !compact {
            if k < 2 {
                cv.circle(b.x + 8.0, b.cy(), 2.2, dot_cols[k]);
                cv.line(suffixes[k], &f.small, b.right() - 14.0, 11.0, b.cy(), Align::Center, dim());
            } else {
                cv.line(suffixes[k], &f.small, b.x + 4.0, 10.0, b.cy(), Align::Center, dim());
            }
        }
        let w = ui.wheel_in(b);
        if w != 0.0 {
            today.wheel_pomo(k, w);
            wheel_handled = true;
        }
    }
    if !wheel_handled {
        let w = ui.wheel_in(area);
        if w != 0.0 {
            today.wheel_pomo(today.time_focus.unwrap_or(0), w);
        }
    }
}

/// Компактный переключатель режимов «Таймер» / «Помодоро» в шапке карточек 4×2 и 6×2.
/// Возвращает правую границу x переключателя.
unsafe fn draw_mode_pills(
    today: &mut Today,
    ui: &mut Ui,
    cv: &Canvas,
    x: f32,
    y: f32,
    tint: D2D1_COLOR_F,
) -> f32 {
    let f = &cv.g.fonts;
    let labels = [
        crate::i18n::pick("Таймер", "Timer"),
        crate::i18n::pick("Помодоро", "Pomodoro"),
    ];
    let ids = [id(TODAY, ID_MODE_TIMER), id(TODAY, ID_MODE_POMO)];
    let gap = 4.0f32;
    let h = 17.0f32;
    let mut cx = x;
    for (idx, (&label, &bid)) in labels.iter().zip(&ids).enumerate() {
        let is_pomo = idx == 1;
        let selected = today.pomodoro == is_pomo;
        let w = (cv.text_w(label, &f.small_c) + 12.0).ceil();
        let r = Rect::new(cx, y, w, h);
        let (_, clicked) = ui.click_area(bid, r);
        let hv = ui.hover_of(bid);
        let pr = ui.press_of(bid);
        let draw_r = press_scale(r, pr);
        if selected {
            let a = if crate::ui::is_ios_palette() { 0.22 } else { 0.18 };
            let fill = D2D1_COLOR_F { a, ..tint };
            cv.rr(draw_r, draw_r.h / 2.0, mix(fill, fg(), 0.06 * hv));
            cv.rr_line(draw_r, draw_r.h / 2.0, D2D1_COLOR_F { a: 0.40, ..tint }, 1.0);
        } else if hv > 0.01 {
            let mut bg = chip_bg();
            bg.a *= hv;
            cv.rr(draw_r, draw_r.h / 2.0, bg);
            if crate::theme::glass() {
                let mut hl = hairline();
                hl.a *= hv;
                cv.rr_line(draw_r, draw_r.h / 2.0, hl, 1.0);
            }
        }
        let fg_col = if selected { txt() } else { mix(dim(), txt(), hv) };
        cv.text(label, r, &f.small_c, fg_col);
        if clicked {
            today.set_pomodoro(is_pomo);
        }
        cx += w + gap;
    }
    cx - gap
}

/// Три тактильные капсулы настроек Pomodoro: Работа, Перерыв, Циклы.
/// Колёсико мыши меняет значение на лету, клик открывает точный ввод с клавиатуры.
unsafe fn draw_pomo_params(
    today: &mut Today,
    ui: &mut Ui,
    cv: &Canvas,
    tf: [u32; 3],
    area: Rect,
    highlight_active: bool,
    tint: D2D1_COLOR_F,
) {
    let f = &cv.g.fonts;
    let gap = 6.0f32;
    let chip_w = ((area.w - 2.0 * gap) / 3.0).floor();
    let chip_h = area.h.min(20.0);
    let cy = area.y + (area.h - chip_h) / 2.0;

    let min_unit = crate::i18n::pick("мин", "m");
    let labels = [
        format!("{} {min_unit}", today.pomo_work),
        format!("{} {min_unit}", today.pomo_break),
        pomo_cycles_label(today.pomo_cycles),
    ];
    let tips = [
        crate::i18n::pick("Время работы, мин (колёсико или клик)", "Work duration, min (scroll or click)"),
        crate::i18n::pick("Время перерыва, мин (колёсико или клик)", "Break duration, min (scroll or click)"),
        crate::i18n::pick("Количество циклов (колёсико или клик)", "Cycles count (scroll or click)"),
    ];
    let dots = [Some(status_pomodoro()), Some(status_green()), None];

    for k in 0..3 {
        let cr = Rect::new(area.x + k as f32 * (chip_w + gap), cy, chip_w, chip_h);
        let pid = id(TODAY, ID_POMO_PARAM + k as u32);
        let (_, clicked) = ui.click_area(pid, cr);
        let hv = ui.hover_of(pid);
        let pr = ui.press_of(pid);
        let draw_r = press_scale(cr, pr);

        let is_active_phase = highlight_active && ((k == 0 && !today.pomo_is_break) || (k == 1 && today.pomo_is_break));
        let bg = if is_active_phase {
            mix(D2D1_COLOR_F { a: 0.18, ..tint }, fg(), 0.06 * hv)
        } else {
            mix(chip_bg(), fg(), 0.06 * hv)
        };
        cv.rr(draw_r, draw_r.h / 2.0, bg);
        if is_active_phase {
            cv.rr_line(draw_r, draw_r.h / 2.0, D2D1_COLOR_F { a: 0.65, ..tint }, 1.0);
        } else {
            cv.hairline(draw_r, draw_r.h / 2.0);
        }

        let text_col = if is_active_phase { txt() } else { mix(mix(txt(), dim(), 0.2), txt(), hv) };
        if let Some(dot) = dots[k] {
            let tw = cv.text_w(&labels[k], &f.small_c);
            let content_w = 5.0 + 4.0 + tw;
            let sx = cr.x + ((cr.w - content_w) / 2.0).max(5.0);
            cv.circle(sx + 2.5, cr.cy(), 2.2, dot);
            cv.line(&labels[k], &f.small_c, sx + 8.5, (cr.right() - 4.0 - (sx + 8.5)).max(10.0), cr.cy(), Align::Leading, text_col);
        } else {
            cv.text(&labels[k], cr, &f.small_c, text_col);
        }
        ui.tip(pid, cr, tips[k]);

        if clicked {
            today.focus_edit_field(ui, tf, k);
        }
        let w = ui.wheel_in(cr);
        if w != 0.0 {
            today.wheel_pomo(k, w);
        }
    }
}

/// Структурная шкала циклов Pomodoro внизу карточки 4×2: каждый цикл разбит на работу и перерыв.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_pomo_progress(
    cv: &Canvas,
    rect: Rect,
    cycles: u32,
    cur_cycle: u32,
    is_break: bool,
    rem_frac: f32,
    work_col: D2D1_COLOR_F,
    break_col: D2D1_COLOR_F,
) {
    let n = cycles.max(1) as usize;
    let cycle_gap = if n <= 6 { 4.0f32 } else { 3.0f32 };
    let inner_gap = if n <= 6 { 2.0f32 } else { 1.5f32 };
    let total_gaps = (n - 1) as f32 * cycle_gap;
    let cycle_w = ((rect.w - total_gaps) / n as f32).max(6.0);
    let elapsed = (1.0 - rem_frac).clamp(0.0, 1.0);

    let paint_seg = |seg_r: Rect, done: bool, active: bool, col: D2D1_COLOR_F| {
        let rad = (seg_r.h / 2.0).min(seg_r.w / 2.0);
        if done {
            cv.rr(seg_r, rad, D2D1_COLOR_F { a: 0.55, ..col });
        } else if active {
            cv.rr(seg_r, rad, D2D1_COLOR_F { a: 0.22, ..col });
            let fill_w = seg_r.w * elapsed;
            if fill_w > 0.5 {
                cv.g.dc.PushAxisAlignedClip(&Rect::new(seg_r.x, seg_r.y, fill_w, seg_r.h).d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                cv.rr(seg_r, rad, col);
                cv.g.dc.PopAxisAlignedClip();
            }
        } else {
            cv.rr(seg_r, rad, track());
        }
    };

    for i in 0..n {
        let c_num = i as u32 + 1;
        let cx = rect.x + i as f32 * (cycle_w + cycle_gap);
        let avail = (cycle_w - inner_gap).max(4.0);
        let work_w = (avail * 0.72).max(2.0);
        let break_w = (avail - work_w).max(2.0);
        let work_r = Rect::new(cx, rect.y, work_w, rect.h);
        let break_r = Rect::new(cx + work_w + inner_gap, rect.y, break_w, rect.h);

        let work_done = c_num < cur_cycle || (c_num == cur_cycle && is_break);
        let work_active = c_num == cur_cycle && !is_break;
        let break_done = c_num < cur_cycle;
        let break_active = c_num == cur_cycle && is_break;

        paint_seg(work_r, work_done, work_active, work_col);
        paint_seg(break_r, break_done, break_active, break_col);
    }
}

/// Клик и колесо мыши над цифрами времени / кольцом: пауза (если шёл) и фокус на поле.
unsafe fn handle_digits_interaction(today: &mut Today, ui: &mut Ui, tf: [u32; 3], area: Rect) {
    let (_, clicked) = ui.click_area(id(TODAY, ID_DIGITS), area);
    if clicked {
        let slot = if today.pomodoro { usize::from(today.pomo_is_break) } else { 1 };
        today.focus_edit_field(ui, tf, slot);
    }
    let wheel = ui.wheel_in(area);
    if wheel != 0.0 {
        if today.pomodoro {
            let slot = usize::from(today.pomo_is_break);
            today.wheel_pomo(slot, wheel);
        } else {
            today.wheel_time(1, wheel);
        }
    }
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
            pomodoro: self.pomodoro,
            pomo_work: self.pomo_work,
            pomo_break: self.pomo_break,
            pomo_cycles: self.pomo_cycles,
            pomo_cycle: self.pomo_cycle,
            pomo_is_break: self.pomo_is_break,
        });
    }

    pub fn load_timer(&mut self, state: &crate::state::State) {
        let now_unix = crate::sys::now_secs();
        self.pomodoro = state.timer.pomodoro;
        self.pomo_work = state.timer.pomo_work.clamp(1, 120);
        self.pomo_break = state.timer.pomo_break.clamp(1, 60);
        self.pomo_cycles = state.timer.pomo_cycles.clamp(1, 12);
        self.pomo_cycle = state.timer.pomo_cycle.clamp(1, self.pomo_cycles);
        self.pomo_is_break = state.timer.pomo_is_break;

        let default_phase = u64::from(if self.pomo_is_break { self.pomo_break } else { self.pomo_work }) * 60;
        let duration = if self.pomodoro && state.timer.duration == 0 {
            default_phase.max(60)
        } else {
            state.timer.duration.max(1)
        };
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
            } else if self.pomodoro {
                self.pomo_cycle = 1;
                self.pomo_is_break = false;
                self.total = Duration::from_secs(u64::from(self.pomo_work) * 60);
                self.left = self.total;
                self.deadline = None;
                self.fired = true;
            } else {
                self.left = self.total;
                self.deadline = None;
                self.fired = true;
            }
        } else {
            let left = if self.pomodoro && state.timer.left == 0 {
                duration
            } else {
                state.timer.left.min(duration).max(1)
            };
            self.left = Duration::from_secs(left);
            self.deadline = None;
            self.fired = false;
        }

        if !self.pomodoro {
            let total_secs = self.total.as_secs() as u32;
            self.hours = total_secs / 3600;
            self.minutes = total_secs / 60 % 60;
            self.seconds = total_secs % 60;
        } else if self.hours == 0 && self.minutes == 0 && self.seconds == 0 {
            self.minutes = 5;
        }
    }

    fn remaining(&self) -> Duration {
        match self.deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => self.left,
        }
    }

    fn is_pomo_idle(&self) -> bool {
        self.deadline.is_none() && self.pomo_cycle == 1 && !self.pomo_is_break && self.left == self.total
    }

    fn active_kind(&self) -> crate::live::TimerKind {
        if !self.pomodoro {
            crate::live::TimerKind::Timer
        } else if self.pomo_is_break {
            crate::live::TimerKind::PomoBreak
        } else {
            crate::live::TimerKind::PomoWork
        }
    }

    fn active_tint(&self) -> D2D1_COLOR_F {
        self.active_kind().tint()
    }

    /// Состояние таймера для свёрнутого острова: идёт или стоит на паузе после старта.
    /// Нетронутый таймер (остаток равен полному времени) острову не показывается.
    pub fn view(&self) -> Option<crate::live::TimerView> {
        if self.total.is_zero() {
            return None;
        }
        let paused = self.deadline.is_none();
        let mid_pomo = self.pomodoro && (self.pomo_cycle > 1 || self.pomo_is_break);
        if paused && !mid_pomo && (self.left >= self.total || self.left.is_zero()) {
            return None;
        }
        let rem = self.remaining();
        let secs = rem.as_secs_f32().ceil() as u64;
        // Секунда на экране меняется, когда остаток переходит через целое число секунд.
        let sub_ms = (rem.as_millis() % 1000) as u32;
        let kind = if !self.pomodoro {
            crate::live::TimerKind::Timer
        } else if self.pomo_is_break {
            crate::live::TimerKind::PomoBreak
        } else {
            crate::live::TimerKind::PomoWork
        };
        Some(crate::live::TimerView {
            frac: (rem.as_secs_f32() / self.total.as_secs_f32()).clamp(0.0, 1.0),
            secs,
            paused,
            next_ms: if sub_ms == 0 { 1000 } else { sub_ms } + 15,
            kind,
        })
    }

    /// Вызывается из `Today::poll`, когда `deadline` истёк.
    pub(super) fn on_timer_expired(&mut self) {
        if self.pomodoro {
            self.advance_pomo_phase(true);
        } else {
            self.deadline = None;
            self.left = self.total;
            self.fired = true;
            if self.sound {
                super::alarm::play_alarm();
            }
            self.save_timer();
        }
    }

    /// Переход к следующему этапу Pomodoro (работа ↔ перерыв → следующий цикл).
    /// При `from_expiry == true` играет сигнал смены фазы и автозапускает следующий этап,
    /// а после последнего перерыва завершает сессию с `fired = true` и финальным будильником.
    pub(super) fn advance_pomo_phase(&mut self, from_expiry: bool) {
        if !self.pomodoro {
            return;
        }
        let keep_running = if from_expiry { true } else { self.deadline.is_some() };
        if !self.pomo_is_break {
            self.pomo_is_break = true;
            self.total = Duration::from_secs(u64::from(self.pomo_break) * 60);
            self.left = self.total;
            self.deadline = if keep_running { Some(Instant::now() + self.left) } else { None };
            if from_expiry && self.sound {
                super::alarm::play_ding();
            }
        } else if self.pomo_cycle < self.pomo_cycles {
            self.pomo_cycle += 1;
            self.pomo_is_break = false;
            self.total = Duration::from_secs(u64::from(self.pomo_work) * 60);
            self.left = self.total;
            self.deadline = if keep_running { Some(Instant::now() + self.left) } else { None };
            if from_expiry && self.sound {
                super::alarm::play_ding();
            }
        } else {
            self.pomo_cycle = 1;
            self.pomo_is_break = false;
            self.total = Duration::from_secs(u64::from(self.pomo_work) * 60);
            self.left = self.total;
            self.deadline = None;
            if from_expiry {
                self.fired = true;
                if self.sound {
                    super::alarm::play_alarm();
                }
            }
        }
        self.save_timer();
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

    fn reset_timer(&mut self) {
        if self.pomodoro {
            self.pomo_cycle = 1;
            self.pomo_is_break = false;
            self.total = Duration::from_secs(u64::from(self.pomo_work) * 60);
            self.left = self.total;
            self.deadline = None;
        } else {
            self.left = self.total;
            self.deadline = None;
        }
        self.save_timer();
    }

    fn set_pomodoro(&mut self, on: bool) {
        if self.pomodoro == on {
            return;
        }
        self.pomodoro = on;
        self.deadline = None;
        self.time_focus = None;
        if on {
            self.pomo_cycle = 1;
            self.pomo_is_break = false;
            self.total = Duration::from_secs(u64::from(self.pomo_work) * 60);
            self.left = self.total;
        } else {
            let raw = self.hours.min(99) * 3600 + self.minutes.min(9999) * 60 + self.seconds.min(99_999);
            let total = if raw == 0 { 60 } else { raw.min(9 * 3600 + 59 * 60 + 59) };
            self.hours = total / 3600;
            self.minutes = total / 60 % 60;
            self.seconds = total % 60;
            self.total = Duration::from_secs(u64::from(total));
            self.left = self.total;
        }
        self.save_timer();
    }

    fn set_pomo_params(&mut self, work: u32, brk: u32, cycles: u32) {
        self.pomo_work = work.clamp(1, 120);
        self.pomo_break = brk.clamp(1, 60);
        self.pomo_cycles = cycles.clamp(1, 12);
        self.pomo_cycle = 1;
        self.pomo_is_break = false;
        self.deadline = None;
        self.total = Duration::from_secs(u64::from(self.pomo_work) * 60);
        self.left = self.total;
        self.save_timer();
    }

    pub(crate) fn wheel_pomo(&mut self, slot: usize, wheel: f32) {
        if wheel == 0.0 {
            return;
        }
        let d = if wheel > 0.0 { 1i32 } else { -1 };
        let mut v = [self.pomo_work as i32, self.pomo_break as i32, self.pomo_cycles as i32];
        if slot < 3 {
            v[slot] = (v[slot] + d).max(1);
            self.set_pomo_params(v[0] as u32, v[1] as u32, v[2] as u32);
        }
    }

    fn focus_edit_field(&mut self, ui: &mut Ui, tf: [u32; 3], k: usize) {
        if self.deadline.is_some() {
            self.toggle_run();
        }
        if self.pomodoro {
            let vals = [self.pomo_work, self.pomo_break, self.pomo_cycles];
            for i in 0..3 {
                ui.set_field_text(tf[i], &vals[i].to_string());
            }
        } else {
            let vals = [self.hours, self.minutes, self.seconds];
            for i in 0..3 {
                ui.set_field_text(tf[i], &if i == 0 { vals[i].to_string() } else { format!("{:02}", vals[i]) });
            }
        }
        ui.select_all(tf[k]);
        ui.claim_focus(tf[k]);
        self.time_focus = Some(k);
    }

    fn edit_field_value(&self, k: usize) -> String {
        if self.pomodoro {
            [self.pomo_work, self.pomo_break, self.pomo_cycles][k].to_string()
        } else {
            [self.hours, self.minutes, self.seconds][k].to_string()
        }
    }

    fn apply_edit_fields(&mut self, ui: &Ui, tf: [u32; 3]) {
        let n: Vec<u32> = tf.iter().map(|&t| parse_num(&ui.field_text(t))).collect();
        if self.pomodoro {
            self.set_pomo_params(n[0].max(1), n[1].max(1), n[2].max(1));
        } else {
            self.preset = None;
            self.set_time_hms(n[0], n[1], n[2]);
        }
    }

    /// Новое время таймера (останавливает его).
    #[cfg(test)]
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
        self.set_time_hms(0, PRESETS[i], 0);
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

    /// Карточка таймера и Pomodoro по макету:
    /// - 4x1 (216x48): кольцо с колокольчиком звука, цифры времени, компактный чип режима/этапа, Сброс и Старт-Пауза.
    /// - 4x2 (216x104): шапка с переключателем «Таймер / Помодоро», длительностью или фазой, крупным временем и шкалой/капсулами внизу.
    /// - 6x2 (328x104): большое кольцо с временем и статусом слева, переключатель режимов, пресеты/параметры Pomodoro и кнопки справа.
    pub(super) unsafe fn timer_card(&mut self, ui: &mut Ui, cv: &Canvas, tm: Rect, size: usize) {
        let f = &cv.g.fonts;
        let tint = self.active_tint();
        cv.card(tm);
        cv.glow(tm, 14.0, tint, Corner::TopLeft);

        let rem = self.remaining();
        let frac = if self.total.is_zero() { 0.0 } else { rem.as_secs_f32() / self.total.as_secs_f32() };
        self.ring.target = frac.clamp(0.0, 1.0);

        // Поля ввода (ч:м:с или работа/перерыв/циклы): ввод цифр, Tab между полями, Enter или уход из поля применяет.
        let tf = [id(TODAY, ID_FIELDS), id(TODAY, ID_FIELDS + 1), id(TODAY, ID_FIELDS + 2)];
        let mut focused = (0..3).find(|&k| ui.has_focus(tf[k]));
        let entered = (0..3).any(|k| ui.take_submitted(tf[k]).is_some());
        for &t in &tf {
            ui.set_digits(t, true);
        }
        let mut tabbed = false;
        if let Some(k) = focused {
            if ui.take_nav().into_iter().any(|n| n == Nav::Tab) {
                let next = (k + 1) % 3;
                self.apply_edit_fields(ui, tf);
                let next_val = self.edit_field_value(next);
                ui.set_field_text(tf[next], &next_val);
                ui.select_all(tf[next]);
                ui.claim_focus(tf[next]);
                focused = Some(next);
                tabbed = true;
            }
        }
        if !tabbed && (entered || (self.time_focus.is_some() && focused != self.time_focus)) {
            self.apply_edit_fields(ui, tf);
            if entered {
                ui.blur();
                focused = None;
            }
        }
        if !tabbed {
            if let Some(k) = focused.filter(|_| focused != self.time_focus) {
                let v = self.edit_field_value(k);
                ui.set_field_text(tf[k], &v);
                ui.select_all(tf[k]);
            }
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
        let run_fg = self.active_kind().on_fill(tint);

        if size == 0 || tm.h < 65.0 {
            // 4x1 (216x48): кольцо с колокольчиком звука внутри, время, компактная плашка режима/этапа, сброс и пуск/пауза.
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
                self.reset_timer();
            }
            if round_button(ui, cv, id(TODAY, ID_RUN), run_x, cy, btn_rad, run_glyph, tint, run_fg, run_tip) {
                self.toggle_run();
            }

            let tx = rx + r + 8.0;
            if self.time_focus.is_some() {
                let edit_w = (reset_x - btn_rad - 6.0 - tx).max(84.0);
                if self.pomodoro {
                    render_pomo_fields(self, ui, cv, tf, Rect::new(tx, cy - 12.0, edit_w, 24.0), true, tint);
                } else {
                    render_time_fields(self, ui, cv, tf, tx, cy - 12.0);
                }
            } else {
                let pill_w = 26.0f32;
                let pill_h = 16.0f32;
                let pill_x = reset_x - btn_rad - 6.0 - pill_w;
                let pill_r = Rect::new(pill_x, cy - pill_h / 2.0, pill_w, pill_h);
                let pid = id(TODAY, ID_POMO_SKIP);
                let (_, clicked) = ui.click_area(pid, pill_r);
                let hv = ui.hover_of(pid);
                let pr = ui.press_of(pid);
                let draw_r = press_scale(pill_r, pr);

                if self.pomodoro {
                    let bg = mix(D2D1_COLOR_F { a: 0.18, ..tint }, fg(), 0.06 * hv);
                    cv.rr(draw_r, draw_r.h / 2.0, bg);
                    cv.rr_line(draw_r, draw_r.h / 2.0, D2D1_COLOR_F { a: 0.42, ..tint }, 1.0);
                    let cyc_str = format!("{}/{}", self.pomo_cycle, self.pomo_cycles);
                    cv.text(&cyc_str, pill_r, &f.small_c, readable(tint, 60.0));
                    let tip = if self.is_pomo_idle() { "Обычный таймер" } else { "Следующий этап" };
                    ui.tip(pid, pill_r, tip);
                    if clicked {
                        if self.is_pomo_idle() {
                            self.set_pomodoro(false);
                        } else {
                            self.advance_pomo_phase(false);
                        }
                    }
                } else {
                    cv.rr(draw_r, draw_r.h / 2.0, mix(chip_bg(), fg(), 0.06 * hv));
                    cv.hairline(draw_r, draw_r.h / 2.0);
                    cv.circle(pill_r.x + 6.5, pill_r.cy(), 2.0, status_pomodoro());
                    cv.line("P", &f.small_c, pill_r.x + 10.0, pill_r.w - 13.0, pill_r.cy(), Align::Center, mix(dim(), txt(), hv));
                    ui.tip(pid, pill_r, "Режим Помодоро");
                    if clicked {
                        self.set_pomodoro(true);
                    }
                }

                let avail_w = (pill_x - 4.0 - tx).max(48.0);
                let digit_size = if secs >= 3600 { 16.5 } else { 21.0 };
                cv.digits(&label, tx, avail_w, cy, digit_size, 600, Align::Leading, txt());
                let digits_r = Rect::new(tx, tm.y + 4.0, avail_w, tm.h - 8.0);
                handle_digits_interaction(self, ui, tf, digits_r);
            }
        } else if size == 2 || tm.w > 260.0 {
            // 6x2 (328x104): большое кольцо с временем и статусом слева, переключатель режимов, параметры/пресеты и кнопки справа.
            let ring_rad = 38.0f32;
            let (rx, cy) = (tm.x + p + ring_rad, tm.cy());
            cv.ring(rx, cy, ring_rad, 5.0, self.ring.value, tint);

            let time_size = if secs >= 3600 { 16.0 } else { 20.0 };
            cv.digits(&label, rx - 35.0, 70.0, cy - 7.0, time_size, 600, Align::Center, txt());
            let (sub_text, sub_col) = if self.pomodoro {
                let col = if self.is_pomo_idle() { dim() } else { readable(tint, 60.0) };
                (pomo_phase_label(self.pomo_is_break, self.pomo_cycle, self.pomo_cycles), col)
            } else {
                (total_label_6x2(self.total.as_secs()), dim())
            };
            cv.line(&sub_text, &f.small, rx - 35.0, 70.0, cy + 13.0, Align::Center, sub_col);

            let ring_area = Rect::new(rx - ring_rad, cy - ring_rad, 2.0 * ring_rad, 2.0 * ring_rad);
            handle_digits_interaction(self, ui, tf, ring_area);

            let bx = rx + ring_rad + 16.0;
            let bw = (tm.right() - p - bx).max(120.0);

            // Шапка: переключатель режимов слева, кнопка «Следующий этап» (в активном Pomodoro) или «ПРЕСЕТЫ» и колокольчик звука справа.
            let bell_r = Rect::new(tm.right() - p - 18.0, tm.y + 7.0, 18.0, 18.0);
            let pills_right = draw_mode_pills(self, ui, cv, bx, tm.y + 7.0, tint);
            if self.pomodoro && !self.is_pomo_idle() {
                let skip_r = Rect::new(bell_r.x - 22.0, tm.y + 7.0, 18.0, 18.0);
                if ui.icon_button_tinted(cv, id(TODAY, ID_POMO_SKIP), skip_r, icon::NEXT, true, readable(tint, 60.0), "Следующий этап") {
                    self.advance_pomo_phase(false);
                }
            } else if !self.pomodoro {
                let caps_x = pills_right + 8.0;
                let caps_w = (bell_r.x - 6.0 - caps_x).max(0.0);
                if caps_w >= 36.0 {
                    cv.line(crate::i18n::pick("ПРЕСЕТЫ", "PRESETS"), &f.caps, caps_x, caps_w, tm.y + 15.5, Align::Trailing, dim());
                }
            }
            if ui.icon_button_tinted(cv, id(TODAY, ID_SOUND), bell_r, icon::RINGER, self.sound, tint, sound_tip) {
                self.sound = !self.sound;
                self.save_timer();
            }

            // Средний ряд: поля ввода при правке, капсулы Pomodoro или пресеты обычного таймера.
            if self.time_focus.is_some() {
                if self.pomodoro {
                    render_pomo_fields(self, ui, cv, tf, Rect::new(bx, tm.y + 34.0, bw, 24.0), false, tint);
                } else {
                    render_time_fields(self, ui, cv, tf, bx, tm.y + 34.0);
                }
            } else if self.pomodoro {
                draw_pomo_params(self, ui, cv, tf, Rect::new(bx, tm.y + 36.0, bw, 20.0), !self.is_pomo_idle(), tint);
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
                self.reset_timer();
            }

            let pill_w = (reset_cx - rad - 8.0 - bx).max(60.0);
            let pill_r = Rect::new(bx, tm.y + 66.0, pill_w, 28.0);
            let (_, clicked) = ui.click_area(id(TODAY, ID_RUN), pill_r);
            let hv = ui.hover_of(id(TODAY, ID_RUN));
            cv.rr(pill_r, pill_r.h / 2.0, mix(tint, fg(), 0.08 * hv));

            let text_w = cv.text_w(run_tip, &f.small);
            let icon_sz = 14.0f32;
            let gap = 6.0f32;
            let content_w = icon_sz + gap + text_w;
            let start_x = pill_r.x + (pill_r.w - content_w).max(0.0) / 2.0;
            cv.glyph(run_glyph, Rect::new(start_x, pill_r.cy() - icon_sz / 2.0, icon_sz, icon_sz), false, run_fg);
            cv.line(run_tip, &f.small, start_x + icon_sz + gap, text_w + 2.0, pill_r.cy(), Align::Leading, run_fg);
            ui.tip(id(TODAY, ID_RUN), pill_r, run_tip);
            if clicked {
                self.toggle_run();
            }
        } else {
            // 4x2 (216x104): шапка с переключателем режимов, статусом/длительностью и звуком; крупное время и шкала/параметры внизу.
            let bell_r = Rect::new(tm.right() - p - 16.0, tm.y + 8.0, 16.0, 16.0);
            if ui.icon_button_tinted(cv, id(TODAY, ID_SOUND), bell_r, icon::RINGER, self.sound, tint, sound_tip) {
                self.sound = !self.sound;
                self.save_timer();
            }

            let pills_right = draw_mode_pills(self, ui, cv, tm.x + p, tm.y + 8.0, tint);
            let badge_x = pills_right + 4.0;
            let badge_max_w = (bell_r.x - 4.0 - badge_x).max(10.0);
            if self.pomodoro {
                let phase_str = pomo_phase_label(self.pomo_is_break, self.pomo_cycle, self.pomo_cycles);
                if !self.is_pomo_idle() {
                    let pw = (cv.text_w(&phase_str, &f.small_c) + 10.0).min(badge_max_w);
                    let skip_r = Rect::new(bell_r.x - 4.0 - pw, tm.y + 8.0, pw, 17.0);
                    let pid = id(TODAY, ID_POMO_SKIP);
                    let (_, clicked) = ui.click_area(pid, skip_r);
                    let hv = ui.hover_of(pid);
                    let pr = ui.press_of(pid);
                    let draw_r = press_scale(skip_r, pr);
                    let bg = mix(D2D1_COLOR_F { a: 0.16, ..tint }, fg(), 0.06 * hv);
                    cv.rr(draw_r, draw_r.h / 2.0, bg);
                    cv.rr_line(draw_r, draw_r.h / 2.0, D2D1_COLOR_F { a: 0.38, ..tint }, 1.0);
                    cv.line(&phase_str, &f.small_c, skip_r.x + 4.0, (skip_r.w - 8.0).max(4.0), skip_r.cy(), Align::Center, readable(tint, 60.0));
                    ui.tip(pid, skip_r, "Следующий этап");
                    if clicked {
                        self.advance_pomo_phase(false);
                    }
                } else {
                    cv.line(&phase_str, &f.small, badge_x, badge_max_w, tm.y + 16.5, Align::Trailing, dim());
                }
            } else {
                let of_str = total_label_4x2(self.total.as_secs());
                cv.line(&of_str, &f.small, badge_x, badge_max_w, tm.y + 16.5, Align::Trailing, dim());
            }

            let btn_rad = 15.0f32;
            let cy = tm.y + 51.0;
            let run_x = tm.right() - p - btn_rad;
            let reset_x = run_x - 2.0 * btn_rad - 6.0;

            if round_button(ui, cv, id(TODAY, ID_RESET), reset_x, cy, btn_rad, icon::RESTART, chip_bg(), txt(), reset_tip) {
                self.reset_timer();
            }
            if round_button(ui, cv, id(TODAY, ID_RUN), run_x, cy, btn_rad, run_glyph, tint, run_fg, run_tip) {
                self.toggle_run();
            }

            let tx = tm.x + p;
            let avail_w = (reset_x - btn_rad - 6.0 - tx).max(60.0);
            if !self.pomodoro && self.time_focus.is_some() {
                // В режиме правки обычного таймера вместо цифр показываем поля ввода ч:м:с.
                render_time_fields(self, ui, cv, tf, tx, cy - 12.0);
            } else {
                let font_sz = if secs >= 3600 { 28.0 } else { 38.0 };
                cv.digits(&label, tx, avail_w, cy, font_sz, 600, Align::Leading, txt());
                let digits_r = Rect::new(tx, cy - 20.0, avail_w, 40.0);
                handle_digits_interaction(self, ui, tf, digits_r);
            }

            if self.pomodoro {
                if self.time_focus.is_some() {
                    let bot_r = Rect::new(tm.x + p, tm.bottom() - p - 21.0, tm.w - 2.0 * p, 22.0);
                    render_pomo_fields(self, ui, cv, tf, bot_r, false, tint);
                } else if self.is_pomo_idle() {
                    let bot_r = Rect::new(tm.x + p, tm.bottom() - p - 20.0, tm.w - 2.0 * p, 20.0);
                    draw_pomo_params(self, ui, cv, tf, bot_r, false, tint);
                } else {
                    let seg_h = 10.0f32;
                    let seg_r = Rect::new(tm.x + p, tm.bottom() - p - seg_h, tm.w - 2.0 * p, seg_h);
                    draw_pomo_progress(cv, seg_r, self.pomo_cycles, self.pomo_cycle, self.pomo_is_break, self.ring.value, status_pomodoro(), status_green());
                }
            } else if self.deadline.is_none() && self.left == self.total {
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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

    #[test]
    fn pomodoro_cycle_transitions_and_completion() {
        let mut t = Today::default();
        t.sound = false;
        t.city.clear();
        t.set_pomodoro(true);
        t.set_pomo_params(25, 5, 2);

        assert!(t.is_pomo_idle());
        assert_eq!(t.pomo_cycle, 1);
        assert!(!t.pomo_is_break);
        assert_eq!(t.total, Duration::from_secs(25 * 60));

        // Во время запуска первой рабочей фазы свёрнутый остров получает TimerKind::PomoWork.
        t.toggle_run();
        assert_eq!(t.view().map(|v| v.kind), Some(crate::live::TimerKind::PomoWork));

        // 1. Работа 1 заканчивается -> автопереход в Перерыв 1 (TimerKind::PomoBreak)
        t.deadline = Some(Instant::now() - Duration::from_millis(1));
        t.poll();
        assert_eq!((t.pomo_cycle, t.pomo_is_break), (1, true));
        assert_eq!(t.total, Duration::from_secs(5 * 60));
        assert!(t.deadline.is_some());
        assert!(!t.take_fired());
        assert_eq!(t.view().map(|v| v.kind), Some(crate::live::TimerKind::PomoBreak));

        // На паузе между фазами вид на острове остаётся.
        t.toggle_run();
        assert!(t.deadline.is_none());
        assert!(t.view().is_some(), "Pomodoro на паузе посреди сессии виден на острове");
        t.toggle_run();

        // 2. Перерыв 1 заканчивается -> автопереход в Работа 2 (TimerKind::PomoWork)
        t.deadline = Some(Instant::now() - Duration::from_millis(1));
        t.poll();
        assert_eq!((t.pomo_cycle, t.pomo_is_break), (2, false));
        assert_eq!(t.total, Duration::from_secs(25 * 60));
        assert!(t.deadline.is_some());
        assert!(!t.take_fired());
        assert_eq!(t.view().map(|v| v.kind), Some(crate::live::TimerKind::PomoWork));

        // 3. Работа 2 заканчивается -> автопереход в Перерыв 2
        t.deadline = Some(Instant::now() - Duration::from_millis(1));
        t.poll();
        assert_eq!((t.pomo_cycle, t.pomo_is_break), (2, true));
        assert_eq!(t.total, Duration::from_secs(5 * 60));
        assert!(t.deadline.is_some());

        // 4. Перерыв 2 (последний) заканчивается -> полный сброс и срабатывание fired
        t.deadline = Some(Instant::now() - Duration::from_millis(1));
        t.poll();
        assert!(t.deadline.is_none());
        assert_eq!((t.pomo_cycle, t.pomo_is_break), (1, false));
        assert_eq!(t.total, Duration::from_secs(25 * 60));
        assert_eq!(t.left, t.total);
        assert!(t.take_fired());
        assert!(!t.take_fired());
        assert!(t.view().is_none());
    }

    #[test]
    fn pomodoro_params_clamp_and_wheel() {
        let mut t = Today::default();
        t.set_preset(2); // 10 минут обычного таймера
        assert_eq!((t.hours, t.minutes, t.seconds), (0, 10, 0));

        t.set_pomodoro(true);
        assert_eq!(t.total, Duration::from_secs(25 * 60));

        t.set_pomo_params(0, 999, 99);
        assert_eq!((t.pomo_work, t.pomo_break, t.pomo_cycles), (1, 60, 12));
        assert_eq!(t.total, Duration::from_secs(60));

        t.wheel_pomo(0, 1.0);
        t.wheel_pomo(1, -1.0);
        t.wheel_pomo(2, -1.0);
        assert_eq!((t.pomo_work, t.pomo_break, t.pomo_cycles), (2, 59, 11));

        // Возврат в обычный таймер восстанавливает установленные 10 минут.
        t.set_pomodoro(false);
        assert_eq!(t.total, Duration::from_secs(10 * 60));
        assert_eq!((t.hours, t.minutes, t.seconds), (0, 10, 0));
    }

    #[test]
    fn pomodoro_state_persistence() {
        let mut t = Today::default();
        let now = crate::sys::now_secs();
        let state = crate::state::State {
            timer: crate::state::TimerState {
                duration: 600,
                left: 600,
                finish: now + 420,
                preset: Some(1),
                sound: false,
                pomodoro: true,
                pomo_work: 50,
                pomo_break: 10,
                pomo_cycles: 3,
                pomo_cycle: 2,
                pomo_is_break: true,
            },
            ..Default::default()
        };
        t.load_timer(&state);
        assert!(t.pomodoro);
        assert_eq!((t.pomo_work, t.pomo_break, t.pomo_cycles), (50, 10, 3));
        assert_eq!((t.pomo_cycle, t.pomo_is_break), (2, true));
        assert!(t.deadline.is_some());
        assert!(t.remaining() <= Duration::from_secs(420) && t.remaining() > Duration::from_secs(415));

        // Пауза в середине перерыва
        let paused_state = crate::state::State {
            timer: crate::state::TimerState {
                duration: 600,
                left: 240,
                finish: 0,
                preset: Some(1),
                sound: true,
                pomodoro: true,
                pomo_work: 50,
                pomo_break: 10,
                pomo_cycles: 3,
                pomo_cycle: 2,
                pomo_is_break: true,
            },
            ..Default::default()
        };
        t.load_timer(&paused_state);
        assert!(t.deadline.is_none());
        assert_eq!(t.left, Duration::from_secs(240));
        assert!(t.view().is_some());
    }

    #[test]
    fn orange_timer_button_uses_white_foreground() {
        let mut t = Today::default();
        t.pomodoro = false;
        let kind = t.active_kind();
        assert_eq!(kind, crate::live::TimerKind::Timer);
        assert_eq!(kind.on_fill(t.active_tint()), crate::ui::WHITE);
    }
}
