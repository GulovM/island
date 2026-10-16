//! Вкладка Today: часы, погода (Open-Meteo), неделя и ближайшее событие календаря (iCal),
//! таймер (часы/минуты, пресеты, кольцо, сигнал по окончании), список задач.
//! Раскладка снята с макета 1:1 (панель 550×275).

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::MB_ICONEXCLAMATION;

use crate::anim::Spring;
use crate::config::Config;
use crate::online::{self, civil_from_days, days_from_civil, weekday, Event, Online};
use crate::ui::{icon, mix, Canvas, Rect, Submit, Ui, chip_bg, dim, GREEN, ink, inset, ORANGE, PINK, txt, track, fg};

use super::{id, read_lines, write_lines, PAD, TODAY, TOP};

const PRESETS: [u32; 5] = [1, 5, 10, 15, 20];
const MAX_TODO: usize = 100;
const TODO_FILE: &str = "todo.txt";

/// Дни недели с понедельника.
const WEEK: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];
const MONTHS: [&str; 12] = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
const SUN_COLOR: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.96, g: 0.75, b: 0.19, a: 1.0 };

/// Даты текущей недели с понедельника: (день, месяц).
fn week_dates(y: i32, m: u32, d: u32) -> [(u32, u32); 7] {
    let today = days_from_civil(y, m, d);
    let monday = today - weekday(today);
    let mut out = [(0, 0); 7];
    for (i, o) in out.iter_mut().enumerate() {
        let (_, mm, dd) = civil_from_days(monday + i as i64);
        *o = (dd, mm);
    }
    out
}

/// Первая строка блока события: «Далее • 11:00», «Завтра • 09:30», «Пт 16 окт • весь день».
fn event_when(ev: &Event, now: i64) -> String {
    let day = ev.start.div_euclid(1440);
    let today = now.div_euclid(1440);
    let tod = ev.start.rem_euclid(1440);
    let when = if ev.all_day { "весь день".to_string() } else { format!("{:02}:{:02}", tod / 60, tod % 60) };
    if day == today {
        format!("Далее • {when}")
    } else if day == today + 1 {
        format!("Завтра • {when}")
    } else {
        let (_, m, d) = civil_from_days(day);
        format!("{} {} {} • {when}", WEEK[weekday(day) as usize], d, MONTHS[(m as usize).clamp(1, 12) - 1])
    }
}

pub struct Today {
    /// Выбранный пресет (индекс в `PRESETS`); `None` — время задано вручную.
    preset: Option<usize>,
    hours: u32,
    minutes: u32,
    total: Duration,
    /// Остаток на паузе или до старта.
    left: Duration,
    deadline: Option<Instant>,
    /// Таймер дошёл до нуля; забирается через `take_fired` (индикатор в свёрнутой форме).
    fired: bool,
    sound: bool,
    ring: Spring,
    todo: Vec<(bool, String)>,
    city: String,
    /// Карточка погоды в режиме ввода города.
    editing: bool,
    ics: String,
    online: Online,
}

impl Default for Today {
    fn default() -> Self {
        let mut t = Today {
            preset: None,
            hours: 0,
            minutes: 0,
            total: Duration::ZERO,
            left: Duration::ZERO,
            deadline: None,
            fired: false,
            sound: true,
            ring: Spring::unit(1.0, 160.0, 22.0),
            todo: Vec::new(),
            city: String::new(),
            editing: false,
            ics: String::new(),
            online: Online::default(),
        };
        t.set_preset(1);
        t
    }
}

impl Today {
    pub fn load(&mut self) {
        self.todo = read_lines(TODO_FILE)
            .iter()
            .filter_map(|l| l.split_once('|'))
            .map(|(d, t)| (d == "1", t.to_string()))
            .take(MAX_TODO)
            .collect();
    }

    pub fn configure(&mut self, cfg: &Config) {
        self.city = cfg.weather_city.clone();
        self.ics = cfg.calendar_ics.clone();
    }

    fn save(&self) {
        let lines: Vec<String> = self.todo.iter().map(|(d, t)| format!("{}|{t}", if *d { 1 } else { 0 })).collect();
        write_lines(TODO_FILE, &lines);
    }

    /// Вкладка открылась: обновить погоду и календарь, если устарели.
    pub fn show(&mut self) -> bool {
        online::refresh_weather(&self.city);
        online::refresh_calendar(&self.ics);
        self.sync()
    }

    pub fn sync(&mut self) -> bool {
        self.online = online::snapshot();
        true
    }

    pub fn step(&mut self, dt: f32) {
        self.ring.step(dt);
    }

    pub fn animating(&self) -> bool {
        !self.ring.settled()
    }

    /// Сработал ли таймер с прошлого вызова.
    pub fn take_fired(&mut self) -> bool {
        std::mem::take(&mut self.fired)
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn remaining(&self) -> Duration {
        match self.deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => self.left,
        }
    }

    /// Тик: вышло время таймера — сигнал и сброс. Всегда просит кадр (часы).
    pub fn poll(&mut self) -> bool {
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            self.deadline = None;
            self.left = self.total;
            self.fired = true;
            if self.sound {
                unsafe {
                    let _ = MessageBeep(MB_ICONEXCLAMATION);
                }
            }
        }
        // Проверки «устарело ли» дешёвые: сеть дёргается раз в 15–20 минут.
        online::refresh_weather(&self.city);
        online::refresh_calendar(&self.ics);
        true
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
        self.hours = hours.min(9);
        self.minutes = minutes.min(59);
        if self.hours == 0 && self.minutes == 0 {
            self.minutes = 1;
        }
        self.total = Duration::from_secs((self.hours * 60 + self.minutes) as u64 * 60);
        self.left = self.total;
        self.deadline = None;
    }

    fn set_preset(&mut self, i: usize) {
        self.preset = Some(i);
        self.set_time(0, PRESETS[i]);
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, _h: f32) {
        let t = GetLocalTime();
        let f = &cv.g.fonts;
        let dow = (t.wDayOfWeek as usize + 6) % 7;

        // Часы: часы и минуты столбиком, между ними три золотые черточки.
        let c = Rect::new(PAD, TOP, 108.5, 132.0);
        cv.card(c);
        cv.text(&format!("{:02}", t.wHour), Rect::new(c.x, c.y + 13.0, c.w, 36.0), &f.big_c, txt());
        for k in -1..=1 {
            cv.rr(Rect::new(c.cx() + k as f32 * 8.0 - 2.5, c.y + 58.5, 5.0, 3.0), 1.5, cv.accent);
        }
        cv.text(&format!("{:02}", t.wMinute), Rect::new(c.x, c.y + 67.0, c.w, 36.0), &f.big_c, txt());
        let foot = format!("{} • {} {}", WEEK[dow], t.wDay, MONTHS[(t.wMonth as usize).clamp(1, 12) - 1]);
        cv.text(&foot, Rect::new(c.x, c.y + 112.0, c.w, 13.0), &f.small_c, dim());

        // Погода. Клик по карточке — поле для города (пусто — определить по IP).
        let wc = Rect::new(PAD, TOP + 141.0, 108.5, 87.0);
        cv.card(wc);
        let field = id(TODAY, 61);
        if self.editing {
            if let Some(city) = ui.take_submitted(field) {
                self.city = city.trim().to_string();
                crate::config::save_str("weather_city", &self.city);
                online::refresh_weather(&self.city);
                self.online = online::snapshot();
                self.editing = false;
                ui.blur();
            } else if !ui.has_focus(field) {
                // Esc или клик мимо — без изменений.
                self.editing = false;
            }
        }
        if self.editing {
            cv.text("Город", Rect::new(wc.x + 9.0, wc.y + 8.0, wc.w - 18.0, 12.0), &f.small_b, dim());
            ui.text_field_ex(cv, field, Rect::new(wc.x + 5.0, wc.y + 24.0, wc.w - 10.0, 24.0), "по IP", icon::SEARCH, Submit::Keep);
            cv.text("Enter — сохранить", Rect::new(wc.x + 4.0, wc.y + 53.0, wc.w - 8.0, 12.0), &f.small_c, dim());
            cv.text("пусто — по IP", Rect::new(wc.x + 4.0, wc.y + 66.0, wc.w - 8.0, 12.0), &f.small_c, mix(dim(), crate::ui::card(), 0.35));
        } else {
            let (hover, clicked) = ui.click_area(id(TODAY, 60), wc);
            if clicked {
                self.editing = true;
                ui.set_field_text(field, &self.city);
                ui.focus = Some(field);
                ui.caret_on = true;
            }
            let hv = ui.hover_of(id(TODAY, 60));
            if hover || hv > 0.01 {
                cv.rr(wc, 11.0, D2D1_COLOR_F { a: 0.04 * hv, ..fg() });
                cv.glyph(icon::EDIT, Rect::new(wc.right() - 22.0, wc.y + 5.0, 16.0, 16.0), false, D2D1_COLOR_F { a: hv, ..dim() });
            }
        }
        match &self.online.weather {
            _ if self.editing => {}
            Some(wt) => {
                let (g, col) = if wt.code <= 1 { (icon::SUN, SUN_COLOR) } else { (icon::CLOUD, mix(dim(), txt(), 0.5)) };
                cv.glyph(g, Rect::new(wc.x, wc.y + 12.0, wc.w, 20.0), true, col);
                cv.text(&format!("{}°C", wt.temp.round() as i32), Rect::new(wc.x, wc.y + 35.0, wc.w, 22.0), &f.num_c, txt());
                cv.text(online::weather_text(wt.code), Rect::new(wc.x + 4.0, wc.y + 57.0, wc.w - 8.0, 12.0), &f.small_c, dim());
                cv.text(&wt.city, Rect::new(wc.x + 4.0, wc.y + 69.0, wc.w - 8.0, 12.0), &f.small_c, mix(dim(), crate::ui::card(), 0.35));
            }
            None => {
                cv.glyph(icon::CLOUD, Rect::new(wc.x, wc.y + 12.0, wc.w, 20.0), true, dim());
                cv.text("—", Rect::new(wc.x, wc.y + 35.0, wc.w, 22.0), &f.num_c, dim());
                let note = self.online.weather_note.as_deref().unwrap_or("Загрузка…");
                cv.text(note, Rect::new(wc.x + 4.0, wc.y + 57.0, wc.w - 8.0, 12.0), &f.small_c, dim());
            }
        }

        self.week_card(cv, Rect::new(PAD + 116.5, TOP, 221.0, 113.0), &t, dow);
        self.timer_card(ui, cv, Rect::new(PAD + 116.5, TOP + 121.0, 221.0, 107.0));
        let tx = PAD + 116.5 + 221.0 + 7.5;
        self.todo_card(ui, cv, Rect::new(tx, TOP, (w - PAD - tx).max(120.0), 228.0));
    }

    unsafe fn week_card(&self, cv: &Canvas, wk: Rect, t: &windows::Win32::Foundation::SYSTEMTIME, dow: usize) {
        let f = &cv.g.fonts;
        cv.card(wk);
        let dates = week_dates(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        cv.text("Эта неделя", Rect::new(wk.x + 9.0, wk.y + 10.0, 110.0, 14.0), &f.body, txt());
        let range = format!(
            "{} {} – {} {}",
            dates[0].0,
            MONTHS[(dates[0].1 as usize).clamp(1, 12) - 1],
            dates[6].0,
            MONTHS[(dates[6].1 as usize).clamp(1, 12) - 1]
        );
        cv.text(&range, Rect::new(wk.right() - 111.0, wk.y + 10.0, 100.0, 14.0), &f.small_r, dim());
        for (i, (day, _)) in dates.iter().enumerate() {
            let cx = wk.x + 21.0 + i as f32 * 29.4;
            cv.text(WEEK[i], Rect::new(cx - 14.0, wk.y + 25.0, 28.0, 12.0), &f.small_c, dim());
            let b = Rect::new(cx - 12.5, wk.y + 39.0, 25.0, 23.0);
            let today = i == dow;
            if today {
                cv.rr(b, 5.0, cv.accent);
            }
            cv.text(&day.to_string(), b, &f.body_c, if today { ink() } else { txt() });
        }

        // Ближайшее событие календаря.
        let ev = Rect::new(wk.x + 7.0, wk.y + 70.0, wk.w - 14.0, 36.0);
        cv.rr(ev, 8.0, inset());
        cv.rr(Rect::new(ev.x + 8.0, ev.y + 8.0, 3.0, 20.0), 1.5, PINK);
        let (l1, l2) = match (&self.online.event, self.online.calendar_set) {
            (Some(e), _) => (event_when(e, online::now_local_minutes()), e.title.clone()),
            (None, true) => ("Событий нет".to_string(), "Календарь пуст на ближайшее время".to_string()),
            (None, false) => ("Календарь не подключён".to_string(), "calendar_ics в config.toml".to_string()),
        };
        cv.text(&l1, Rect::new(ev.x + 17.0, ev.y + 5.0, ev.w - 24.0, 13.0), &f.small_b, txt());
        cv.text(&l2, Rect::new(ev.x + 17.0, ev.y + 18.0, ev.w - 24.0, 13.0), &f.small, dim());
    }

    unsafe fn timer_card(&mut self, ui: &mut Ui, cv: &Canvas, tm: Rect) {
        let f = &cv.g.fonts;
        cv.card(tm);
        let rem = self.remaining();
        let frac = if self.total.is_zero() { 0.0 } else { rem.as_secs_f32() / self.total.as_secs_f32() };
        self.ring.target = frac.clamp(0.0, 1.0);
        let (cx, cy) = (tm.x + 38.0, tm.y + 54.0);
        cv.ring(cx, cy, 24.0, 3.0, self.ring.value, ORANGE);
        let secs = rem.as_secs_f32().ceil() as u64;
        let label = if secs >= 3600 {
            format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
        } else {
            format!("{:02}:{:02}", secs / 60, secs % 60)
        };
        cv.text(&label, Rect::new(cx - 24.0, cy - 8.0, 48.0, 16.0), &f.body_c, txt());
        if ui.icon_button(cv, id(TODAY, 21), Rect::new(tm.x + 5.0, tm.y + 5.0, 16.0, 16.0), icon::RINGER, self.sound) {
            self.sound = !self.sound;
        }

        // Часы и минуты: колесо мыши меняет на 1, клик — на 1 час / 5 минут по кругу.
        let bx = tm.x + 78.0;
        let boxes = [(Rect::new(bx, tm.y + 10.0, 65.0, 22.0), "ч", true), (Rect::new(bx + 70.0, tm.y + 10.0, 64.0, 22.0), "мин", false)];
        for (k, (b, unit, is_h)) in boxes.iter().enumerate() {
            let bid = id(TODAY, 50 + k as u32);
            let (_, clicked) = ui.click_area(bid, *b);
            let hv = ui.hover_of(bid);
            let wheel = ui.wheel_in(*b);
            cv.rr(*b, 6.0, mix(inset(), fg(), 0.04 * hv));
            let value = if *is_h { self.hours.to_string() } else { format!("{:02}", self.minutes) };
            cv.text(&value, Rect::new(b.x, b.y, b.w - 26.0, b.h), &f.body_r, txt());
            cv.text(unit, Rect::new(b.right() - 23.0, b.y + 1.0, 22.0, b.h), &f.small, dim());
            let step = if wheel > 0.0 { 1i32 } else if wheel < 0.0 { -1 } else { 0 };
            if clicked || step != 0 {
                let (mut hh, mut mm) = (self.hours as i32, self.minutes as i32);
                if *is_h {
                    hh = if clicked { (hh + 1) % 10 } else { (hh + step).clamp(0, 9) };
                } else {
                    mm = if clicked { (mm / 5 * 5 + 5) % 60 } else { (mm + step).clamp(0, 59) };
                }
                self.preset = None;
                self.set_time(hh as u32, mm as u32);
            }
        }

        // Пресеты.
        for (i, m) in PRESETS.iter().enumerate() {
            let r = Rect::new(bx + i as f32 * 27.6, tm.y + 40.0, 24.0, 17.0);
            if ui.chip_ex(cv, id(TODAY, i as u32), r, &format!("{m}м"), self.preset == Some(i), false, Some(ORANGE)) {
                self.set_preset(i);
            }
        }

        // Старт / пауза и сброс.
        let start = if self.deadline.is_some() { "Пауза" } else { "Старт" };
        if ui.solid_button(cv, id(TODAY, 40), Rect::new(bx, tm.y + 62.0, 65.0, 23.0), start, ORANGE, ink(), 6.0) {
            self.toggle_run();
        }
        if ui.solid_button(cv, id(TODAY, 41), Rect::new(bx + 70.0, tm.y + 62.0, 64.0, 23.0), "Сброс", chip_bg(), txt(), 6.0) {
            self.left = self.total;
            self.deadline = None;
        }
    }

    unsafe fn todo_card(&mut self, ui: &mut Ui, cv: &Canvas, card: Rect) {
        cv.card(card);
        let f = &cv.g.fonts;
        let done = self.todo.iter().filter(|(d, _)| *d).count();
        cv.text("Задачи", Rect::new(card.x + 9.0, card.y + 10.0, 90.0, 14.0), &f.body, txt());
        // Счётчик «выполнено/всего» в кружке с дугой прогресса.
        let (bx, by) = (card.right() - 19.0, card.y + 16.0);
        cv.circle(bx, by, 9.0, chip_bg());
        if !self.todo.is_empty() {
            cv.ring(bx, by, 8.0, 1.4, done as f32 / self.todo.len() as f32, GREEN);
        }
        cv.text(&format!("{done}/{}", self.todo.len()), Rect::new(bx - 9.0, by - 6.0, 18.0, 12.0), &f.caps_c, txt());

        // Поле «добавить» внизу и золотая кнопка со стрелкой.
        let fid = id(TODAY, 30);
        let field = Rect::new(card.x + 8.0, card.bottom() - 33.0, card.w - 16.0, 26.0);
        ui.text_field_ex(cv, fid, field, "Добавить задачу…", icon::ADD, Submit::Clear);
        cv.rr_line(field, 8.0, track(), 1.0);
        let add = Rect::new(field.right() - 22.0, field.y + 5.0, 16.0, 16.0);
        let (_, add_clicked) = ui.click_area(id(TODAY, 31), add);
        let hv = ui.hover_of(id(TODAY, 31));
        cv.rr(add, 4.0, mix(cv.accent, fg(), 0.12 * hv));
        cv.glyph(icon::FORWARD, add, false, ink());
        let mut submitted = ui.take_submitted(fid);
        if add_clicked {
            submitted = ui.submit_field(fid);
        }
        if let Some(text) = submitted {
            let text = text.trim();
            if !text.is_empty() && self.todo.len() < MAX_TODO {
                self.todo.insert(0, (false, text.to_string()));
                self.save();
            }
        }

        const ROW: f32 = 25.0;
        const PITCH: f32 = 27.0;
        let view = Rect::new(card.x + 7.0, card.y + 32.0, card.w - 14.0, (card.h - 32.0 - 40.0).max(ROW));
        let sid = id(TODAY, 100);
        let off = ui.scroll_begin(cv, sid, view, self.todo.len() as f32 * PITCH);
        let mut toggled = None;
        let mut removed = None;
        for (i, (is_done, text)) in self.todo.iter().enumerate() {
            let y = view.y + i as f32 * PITCH - off;
            if y + ROW < view.y || y > view.bottom() {
                continue;
            }
            let r = Rect::new(view.x, y, view.w - 4.0, ROW);
            let rid = id(TODAY, 200 + i as u32);
            let (_, clicked) = ui.click_area(rid, r);
            let hv = ui.hover_of(rid);
            cv.rr(r, 8.0, mix(inset(), fg(), 0.035 * hv));
            let (cx, cy) = (r.x + 13.0, r.cy());
            if *is_done {
                cv.circle(cx, cy, 6.0, GREEN);
                cv.text(icon::CHECK, Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), &f.icon_small, ink());
            } else {
                cv.rr_line(Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), 6.0, dim(), 1.2);
            }
            let tr = Rect::new(r.x + 25.0, r.y, r.w - 25.0 - 22.0, r.h);
            cv.text(text, tr, &f.small_b, if *is_done { dim() } else { txt() });
            if *is_done {
                let tw = (text.chars().count() as f32 * 4.7).min(tr.w);
                cv.rr(Rect::new(tr.x, cy - 0.5, tw, 1.0), 0.5, dim());
            }
            if clicked {
                toggled = Some(i);
            }
            let del = Rect::new(r.right() - 21.0, cy - 9.0, 18.0, 18.0);
            let did = id(TODAY, 400 + i as u32);
            if (hv > 0.3 || ui.hover_of(did) > 0.3) && ui.icon_button(cv, did, del, icon::CLOSE, false) {
                removed = Some(i);
            }
        }
        ui.scroll_end(cv, sid);
        if let Some(i) = removed {
            self.todo.remove(i);
            self.save();
        } else if let Some(i) = toggled {
            self.todo[i].0 = !self.todo[i].0;
            self.save();
        }
        if self.todo.is_empty() {
            cv.text("Список пуст", Rect::new(view.x + 4.0, view.y + 4.0, view.w, 14.0), &f.small, dim());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn week_starts_monday() {
        // 15 октября 2026 — четверг: неделя 12–18 октября.
        let w = week_dates(2026, 10, 15);
        assert_eq!(w[0], (12, 10));
        assert_eq!(w[3], (15, 10));
        assert_eq!(w[6], (18, 10));
        // Переход через границу месяца: 1 октября 2026 — четверг, неделя с 28 сентября.
        let w = week_dates(2026, 10, 1);
        assert_eq!(w[0], (28, 9));
        assert_eq!(w[6], (4, 10));
    }

    #[test]
    fn event_labels() {
        let now = days_from_civil(2026, 10, 15) * 1440 + 9 * 60;
        let ev = |day: i64, tod: i64, all_day| Event { start: day * 1440 + tod, all_day, title: String::new() };
        let today = now / 1440;
        assert_eq!(event_when(&ev(today, 11 * 60, false), now), "Далее • 11:00");
        assert_eq!(event_when(&ev(today + 1, 9 * 60 + 30, false), now), "Завтра • 09:30");
        assert_eq!(event_when(&ev(today + 2, 0, true), now), "Сб 17 окт • весь день");
    }

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
