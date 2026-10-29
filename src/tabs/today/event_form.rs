//! Форма создания и правки события календаря, которое хранится в самом острове.

use super::Today;
use crate::online::{self, civil_from_days, weekday, LocalEvent, Repeat, REMIND_CHOICES};
use crate::tabs::{id, TODAY};
use crate::ui::{dim, icon, status_pink, txt, Align, Canvas, Rect, Submit, Ui};

const TITLE: u32 = 120;
const START: u32 = 121;
const END: u32 = 122;
const ALL_DAY: u32 = 123;
const REPEAT: u32 = 124;
const REMIND: u32 = 125;
const SAVE: u32 = 126;
const DELETE: u32 = 127;
const CLOSE: u32 = 128;
const DAY_PREV: u32 = 129;
const DAY_NEXT: u32 = 137;
/// Кнопки дней недели: 130..=136.
const WEEKDAY: u32 = 130;

/// Что не так с введёнными данными.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    Title,
    Start,
    End,
    Limit,
}

/// Состояние открытой формы.
#[derive(Clone, Debug)]
pub(super) struct EventForm {
    /// Правка существующего события (0 — новое).
    pub id: u64,
    /// День начала (дни от 1970-01-01).
    pub day: i64,
    pub all_day: bool,
    pub repeat: Repeat,
    pub days: u8,
    /// Индекс в `REMIND_CHOICES`.
    pub remind: usize,
    pub problem: Option<Problem>,
}

impl EventForm {
    pub fn new(day: i64) -> Self {
        Self { id: 0, day, all_day: false, repeat: Repeat::None, days: 0, remind: 0, problem: None }
    }
}

/// «9:30», «09:30», «9.30», «930» → минуты от полуночи.
pub(super) fn parse_hm(s: &str) -> Option<i64> {
    let s = s.trim();
    let (h, m) = match s.find([':', '.', ' ']) {
        Some(i) => (&s[..i], s[i + 1..].trim()),
        None if s.len() == 3 || s.len() == 4 => s.split_at(s.len() - 2),
        None => (s, "0"),
    };
    let (h, m) = (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

fn hm(minutes: i64) -> String {
    format!("{:02}:{:02}", minutes.div_euclid(60), minutes.rem_euclid(60))
}

impl Today {
    /// Открывает форму нового события на день `day`.
    pub(super) fn begin_event(&mut self, ui: &mut Ui, day: i64) {
        self.ev_form = Some(EventForm::new(day));
        ui.set_field_text(id(TODAY, TITLE), "");
        // Новое событие по умолчанию — на ближайший час.
        let now = online::now_local_minutes();
        let start = (now.rem_euclid(1440) / 60 + 1).min(23) * 60;
        ui.set_field_text(id(TODAY, START), &hm(start));
        ui.set_field_text(id(TODAY, END), "");
        ui.focus = Some(id(TODAY, TITLE));
        ui.caret_on = true;
    }

    /// Открывает форму для события острова `ev_id`.
    pub(super) fn edit_event(&mut self, ui: &mut Ui, ev_id: u64) {
        let Some(e) = self.local.iter().find(|e| e.id == ev_id).cloned() else { return };
        let day = e.start.div_euclid(1440);
        self.ev_form = Some(EventForm {
            id: e.id,
            day,
            all_day: e.all_day,
            repeat: e.repeat,
            days: e.days,
            remind: REMIND_CHOICES.iter().position(|&m| m == e.remind).unwrap_or(0),
            problem: None,
        });
        ui.set_field_text(id(TODAY, TITLE), &e.title);
        ui.set_field_text(id(TODAY, START), &hm(e.start.rem_euclid(1440)));
        let end = if e.end > e.start && !e.all_day { hm(e.end - day * 1440) } else { String::new() };
        ui.set_field_text(id(TODAY, END), &end);
        ui.focus = Some(id(TODAY, TITLE));
        ui.caret_on = true;
    }

    /// Собирает событие из полей формы.
    fn read_event(ui: &Ui, form: &EventForm) -> Result<LocalEvent, Problem> {
        let title = ui.field_text(id(TODAY, TITLE)).trim().to_string();
        if title.is_empty() {
            return Err(Problem::Title);
        }
        let base = form.day * 1440;
        let (start, end) = if form.all_day {
            (base, base + 1440)
        } else {
            let s = parse_hm(&ui.field_text(id(TODAY, START))).ok_or(Problem::Start)?;
            let end_text = ui.field_text(id(TODAY, END));
            let end = if end_text.trim().is_empty() {
                0
            } else {
                let e = parse_hm(&end_text).ok_or(Problem::End)?;
                if e <= s {
                    return Err(Problem::End);
                }
                base + e
            };
            (base + s, end)
        };
        let days = if form.repeat == Repeat::Weekly { form.days } else { 0 };
        Ok(LocalEvent { id: form.id, title, start, end, all_day: form.all_day, repeat: form.repeat, days, remind: REMIND_CHOICES[form.remind] })
    }

    /// Форма события на месте месячного календаря `r`.
    pub(super) unsafe fn event_form(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect) {
        let Some(mut form) = self.ev_form.take() else { return };
        let f = &cv.g.fonts;
        let pick = crate::i18n::pick;
        let x = r.x + 9.0;
        let w = r.w - 18.0;
        let mut keep = true;

        let head = if form.id == 0 { pick("Новое событие", "New event") } else { pick("Событие", "Event") };
        cv.text(head, Rect::new(x, r.y + 7.0, w - 28.0, 18.0), &f.body, txt());
        if ui.icon_button(cv, id(TODAY, CLOSE), Rect::new(r.right() - 26.0, r.y + 7.0, 18.0, 18.0), icon::CLOSE, false, crate::i18n::pick("Закрыть", "Close")) {
            keep = false;
        }

        // Название.
        let title_id = id(TODAY, TITLE);
        ui.text_field_at(cv, title_id, Rect::new(x, r.y + 28.0, w, 24.0), pick("Название", "Title"), "", Submit::Keep, 10.0, 8.0);
        let mut save = ui.take_submitted(title_id).is_some();

        // Дата и «весь день».
        let dy = r.y + 58.0;
        if ui.icon_button(cv, id(TODAY, DAY_PREV), Rect::new(x, dy, 18.0, 18.0), icon::LEFT, false, crate::i18n::pick("Предыдущий день", "Previous day")) {
            form.day -= 1;
        }
        let (_, m, d) = civil_from_days(form.day);
        let date = format!("{} {} {}", crate::i18n::weekdays()[weekday(form.day) as usize], d, crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1]);
        cv.line(&date, &f.small_b, x + 18.0, 62.0, dy + 9.0, Align::Center, txt());
        if ui.icon_button(cv, id(TODAY, DAY_NEXT), Rect::new(x + 80.0, dy, 18.0, 18.0), icon::RIGHT, false, crate::i18n::pick("Следующий день", "Next day")) {
            form.day += 1;
        }
        cv.line(pick("Весь день", "All day"), &f.small, r.right() - 9.0 - 34.0 - 6.0 - 70.0, 70.0, dy + 9.0, Align::Trailing, dim());
        let mut all = form.all_day;
        ui.switch(cv, id(TODAY, ALL_DAY), Rect::new(r.right() - 9.0 - 34.0, dy + 1.0, 34.0, 16.0), &mut all, true);
        form.all_day = all;

        // Время начала и конца.
        let ty = r.y + 82.0;
        if !form.all_day {
            ui.text_field_at(cv, id(TODAY, START), Rect::new(x, ty, 64.0, 24.0), "09:00", "", Submit::Keep, 10.0, 6.0);
            cv.line("–", &f.body, x + 64.0, 16.0, ty + 12.0, Align::Center, dim());
            ui.text_field_at(cv, id(TODAY, END), Rect::new(x + 80.0, ty, 64.0, 24.0), pick("конец", "end"), "", Submit::Keep, 10.0, 6.0);
            save |= ui.take_submitted(id(TODAY, START)).is_some();
            save |= ui.take_submitted(id(TODAY, END)).is_some();
        }

        // Повтор и напоминание.
        let cy = r.y + 112.0;
        let (rl_ru, rl_en) = form.repeat.label();
        if ui.chip(cv, id(TODAY, REPEAT), Rect::new(x, cy, 100.0, 22.0), pick(rl_ru, rl_en), form.repeat != Repeat::None) {
            let i = Repeat::ALL.iter().position(|&p| p == form.repeat).unwrap_or(0);
            form.repeat = Repeat::ALL[(i + 1) % Repeat::ALL.len()];
        }
        let remind = REMIND_CHOICES[form.remind];
        let rl = match remind {
            0 => pick("Без напоминания", "No reminder").to_string(),
            60 => pick("За 1 ч", "1 h before").to_string(),
            m if crate::i18n::en() => format!("{m} min before"),
            m => format!("За {m} мин"),
        };
        if ui.chip(cv, id(TODAY, REMIND), Rect::new(x + 106.0, cy, w - 106.0, 22.0), &rl, remind != 0) {
            form.remind = (form.remind + 1) % REMIND_CHOICES.len();
        }

        // Дни недели для недельного повтора.
        if form.repeat == Repeat::Weekly {
            let cw = w / 7.0;
            for (i, name) in crate::i18n::weekdays().iter().enumerate() {
                let rect = Rect::new(x + i as f32 * cw + 1.0, r.y + 140.0, cw - 2.0, 20.0);
                // Если дни не выбраны, повтор идёт по дню недели начала: подсвечиваем его.
                let on = if form.days == 0 { weekday(form.day) as usize == i } else { form.days >> i & 1 == 1 };
                if ui.chip(cv, id(TODAY, WEEKDAY + i as u32), rect, name, on) {
                    let cur = if form.days == 0 { 1u8 << weekday(form.day) } else { form.days };
                    form.days = cur ^ (1 << i);
                }
            }
        }

        // Подсказка об ошибке и кнопки.
        if let Some(p) = form.problem {
            let msg = match p {
                Problem::Title => pick("Введите название", "Enter a title"),
                Problem::Start => pick("Время вида 09:30", "Time like 09:30"),
                Problem::End => pick("Конец позже начала, вида 10:30", "End after start, like 10:30"),
                Problem::Limit => pick("Слишком много событий", "Too many events"),
            };
            cv.text(msg, Rect::new(x, r.y + 164.0, w, 14.0), &f.small, status_pink());
        }
        let by = r.bottom() - 32.0;
        if form.id != 0 && ui.chip(cv, id(TODAY, DELETE), Rect::new(x, by, 84.0, 24.0), pick("Удалить", "Delete"), false) {
            self.remove_event(form.id);
            keep = false;
        }
        if ui.chip(cv, id(TODAY, SAVE), Rect::new(r.right() - 9.0 - 100.0, by, 100.0, 24.0), pick("Сохранить", "Save"), true) {
            save = true;
        }

        if keep && save {
            match Self::read_event(ui, &form) {
                Ok(ev) => {
                    if self.save_event(ev) {
                        keep = false;
                    } else {
                        form.problem = Some(Problem::Limit);
                    }
                }
                Err(p) => form.problem = Some(p),
            }
        }
        if keep {
            self.ev_form = Some(form);
        } else {
            ui.blur();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_time_variants() {
        assert_eq!(parse_hm("9:30"), Some(570));
        assert_eq!(parse_hm("09:30"), Some(570));
        assert_eq!(parse_hm("9.05"), Some(545));
        assert_eq!(parse_hm("930"), Some(570));
        assert_eq!(parse_hm("0930"), Some(570));
        assert_eq!(parse_hm("14"), Some(840));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("10:75"), None);
        assert_eq!(parse_hm(""), None);
        assert_eq!(parse_hm("ab"), None);
        assert_eq!(parse_hm("-1:30"), None);
    }
}
