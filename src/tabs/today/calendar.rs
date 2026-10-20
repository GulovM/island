//! Неделя, ближайшее событие и месячный календарь, ввод ссылки на календарь (iCal).

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::Today;
use crate::online::{self, civil_from_days, days_from_civil, weekday, DayEvent, Event};
use crate::tabs::{id, TODAY};
use crate::ui::{dim, fg, icon, ink, inset, mix, txt, Canvas, Rect, Submit, Ui, PINK};

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
    let when = if ev.all_day { crate::i18n::t("весь день").to_string() } else { format!("{:02}:{:02}", tod / 60, tod % 60) };
    if day == today {
        format!("{} • {when}", crate::i18n::pick("Далее", "Next"))
    } else if day == today + 1 {
        format!("{} • {when}", crate::i18n::pick("Завтра", "Tomorrow"))
    } else {
        let (_, m, d) = civil_from_days(day);
        format!("{} {} {} • {when}", crate::i18n::weekdays()[weekday(day) as usize], d, crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1])
    }
}

/// Сокращает URL календаря для показа в списке: показывает хост и короткий хвост (имя файла),
/// скрывая токены авторизации и приватные параметры.
pub fn abbreviate_url(url: &str) -> String {
    let u = url.trim();
    if u.is_empty() {
        return String::new();
    }
    let rest = if let Some(r) = u.strip_prefix("https://") {
        r
    } else if let Some(r) = u.strip_prefix("http://") {
        r
    } else if let Some(r) = u.strip_prefix("webcal://") {
        r
    } else {
        u
    };
    let (hostport, path_and_query) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = hostport.split(':').next().unwrap_or(hostport);
    if host.is_empty() {
        return String::new();
    }
    let path = path_and_query.split('?').next().unwrap_or("");
    let last_seg = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    if last_seg.is_empty() {
        return host.to_string();
    }
    let tail = if last_seg.chars().count() > 14 {
        let mut s: String = last_seg.chars().take(12).collect();
        s.push('…');
        s
    } else {
        last_seg.to_string()
    };
    format!("{host}/…/{tail}")
}

/// События показанного месяца: (месяц, версия календаря, дни сетки).
pub(super) type MonthCache = ((i32, u32), u64, Vec<Vec<DayEvent>>);

impl Today {
    /// Открывает месячный календарь на месяце дня `day` и выбирает этот день.
    fn open_month(&mut self, day: i64) {
        let (y, m, _) = civil_from_days(day);
        self.view = (y, m);
        self.sel = day;
        self.month_open = true;
    }

    /// Сдвиг показанного месяца; выбирается сегодняшний день, если он в этом месяце, иначе 1-е число.
    fn shift_month(&mut self, delta: i32, today: i64) {
        let n = self.view.0 * 12 + self.view.1 as i32 - 1 + delta;
        self.view = (n.div_euclid(12), n.rem_euclid(12) as u32 + 1);
        let (ty, tm, _) = civil_from_days(today);
        self.sel = if self.view == (ty, tm) { today } else { days_from_civil(self.view.0, self.view.1, 1) };
    }

    /// Неделя. `size`: 0 — только дни недели, 1 и 2 — ещё и блок ближайшего события (2 шире:
    /// дни расходятся по ширине карточки).
    pub(super) unsafe fn week_card(&mut self, ui: &mut Ui, cv: &Canvas, wk: Rect, size: usize, today: i64, dow: usize) {
        let f = &cv.g.fonts;
        cv.card(wk);
        if self.cal_editing {
            self.calendar_form(ui, cv, wk);
            return;
        }
        let (y, m, d) = civil_from_days(today);
        let dates = week_dates(y, m, d);
        let monday = today - dow as i64;
        cv.text("Эта неделя", Rect::new(wk.x + 9.0, wk.y + 10.0, 110.0, 14.0), &f.body, txt());
        // Раскрыть месяц.
        if ui.icon_button(cv, id(TODAY, 90), Rect::new(wk.x + 80.0, wk.y + 8.0, 18.0, 18.0), icon::DOWN, false) {
            self.open_month(today);
        }
        let range = format!(
            "{} {} – {} {}",
            dates[0].0,
            crate::i18n::months_short()[(dates[0].1 as usize).clamp(1, 12) - 1],
            dates[6].0,
            crate::i18n::months_short()[(dates[6].1 as usize).clamp(1, 12) - 1]
        );
        cv.text(&range, Rect::new(wk.right() - 111.0, wk.y + 10.0, 100.0, 14.0), &f.small_r, dim());
        for (i, (day, _)) in dates.iter().enumerate() {
            let cx = wk.x + 23.0 + i as f32 * (wk.w - 46.0) / 6.0;
            cv.text(crate::i18n::weekdays()[i], Rect::new(cx - 14.0, wk.y + 28.0, 28.0, 12.0), &f.small_c, dim());
            let b = Rect::new(cx - 12.5, wk.y + 43.0, 25.0, 23.0);
            let is_today = i == dow;
            // Клик по дню открывает месяц с выбранным днём.
            let cid = id(TODAY, 80 + i as u32);
            let (_, clicked) = ui.click_area(cid, b);
            let hv = ui.hover_of(cid);
            if is_today {
                cv.rr(b, 5.0, cv.accent);
            } else if hv > 0.01 {
                cv.rr(b, 5.0, D2D1_COLOR_F { a: 0.07 * hv, ..fg() });
            }
            cv.text(&day.to_string(), b, &f.body_c, if is_today { ink() } else { txt() });
            if clicked {
                self.open_month(monday + i as i64);
            }
        }

        // Ближайшее событие календаря (в низкой карточке его нет).
        if size == 0 {
            return;
        }
        let ev = Rect::new(wk.x + 8.0, wk.bottom() - 46.0, wk.w - 16.0, 38.0);
        cv.rr(ev, 8.0, inset());
        cv.rr(Rect::new(ev.x + 8.0, ev.y + 8.0, 3.0, 20.0), 1.5, PINK);
        let (l1, l2) = match (&self.next, self.online.calendar_set) {
            (Some(e), _) => (event_when(e, online::now_local_minutes()), e.title.clone()),
            (None, true) if self.online.calendar_failed => (
                crate::i18n::pick("Не удалось загрузить", "Couldn't load").to_string(),
                if self.ics.len() > 1 {
                    crate::i18n::pick("Проверьте ссылки календаря", "Check calendar links").to_string()
                } else {
                    crate::i18n::t("Проверьте ссылку calendar_ics").to_string()
                },
            ),
            (None, true) => (
                crate::i18n::pick("Событий нет", "No events").to_string(),
                crate::i18n::pick("Календарь пуст на ближайшее время", "The calendar is empty for now").to_string(),
            ),
            (None, false) => (
                crate::i18n::pick("Календарь не подключён", "Calendar not connected").to_string(),
                crate::i18n::pick("Нажмите, чтобы добавить ссылку", "Click to add a link").to_string(),
            ),
        };
        cv.text(&l1, Rect::new(ev.x + 17.0, ev.y + 5.0, ev.w - 24.0, 13.0), &f.small_b, txt());
        cv.text(&l2, Rect::new(ev.x + 17.0, ev.y + 18.0, ev.w - 24.0, 13.0), &f.small, dim());
        // Клик по блоку — ссылка на календарь (подключить, сменить или убрать).
        let (_, clicked) = ui.click_area(id(TODAY, 63), ev);
        let hv = ui.hover_of(id(TODAY, 63));
        if hv > 0.01 {
            cv.rr(ev, 8.0, D2D1_COLOR_F { a: 0.05 * hv, ..fg() });
            cv.glyph(icon::EDIT, Rect::new(ev.right() - 22.0, ev.y + 10.0, 16.0, 16.0), false, D2D1_COLOR_F { a: hv, ..dim() });
        } else if self.online.calendar_failed {
            // Предупреждение об ошибке загрузки при наличии событий
            cv.circle(ev.right() - 14.0, ev.y + 19.0, 2.5, PINK);
        }
        if clicked {
            self.begin_calendar_edit(ui);
        }
    }

    fn begin_calendar_edit(&mut self, ui: &mut Ui) {
        let field = id(TODAY, 62);
        self.cal_editing = true;
        self.cal_bad = false;
        ui.set_field_text(field, "");
        ui.focus = Some(field);
        ui.caret_on = true;
    }

    /// Форма управления ссылками календаря (iCal). Позволяет добавлять новые ссылки и удалять
    /// существующие. Список ссылок отображается в сокращённом виде без секретных токенов.
    unsafe fn calendar_form(&mut self, ui: &mut Ui, cv: &Canvas, wk: Rect) {
        let f = &cv.g.fonts;
        let field = id(TODAY, 62);

        // Обработка ввода новой ссылки в поле
        if let Some(url) = ui.take_submitted(field) {
            let url = url.trim().to_string();
            if url.is_empty() {
                self.cal_editing = false;
                ui.blur();
                return;
            }
            if crate::config::valid_calendar_url(&url) {
                if !self.ics.iter().any(|u| u == &url) && self.ics.len() < crate::config::MAX_CALENDAR_URLS {
                    self.ics.push(url);
                    crate::config::save_calendar_ics(&self.ics);
                    online::refresh_calendar(&self.ics);
                    self.sync();
                }
                self.cal_bad = false;
                ui.set_field_text(field, "");
                ui.focus = Some(field);
                ui.caret_on = true;
            } else {
                self.cal_bad = true;
                ui.set_field_text(field, &url);
                ui.focus = Some(field);
                ui.caret_on = true;
            }
        }

        // Клик вне карточки или закрытие
        let ptr = ui.pointer();
        let clicked_outside = ui.pressed && !wk.contains(ptr);
        if clicked_outside || (!ui.has_focus(field) && !wk.contains(ptr)) {
            self.cal_editing = false;
            return;
        }

        // Заголовок и кнопка закрытия
        cv.text(
            crate::i18n::pick("Календари (iCal)", "Calendars (iCal)"),
            Rect::new(wk.x + 9.0, wk.y + 7.0, wk.w - 36.0, 14.0),
            &f.body,
            txt(),
        );
        let close_btn = Rect::new(wk.right() - 24.0, wk.y + 6.0, 16.0, 16.0);
        if ui.icon_button(cv, id(TODAY, 64), close_btn, icon::CLOSE, false) {
            self.cal_editing = false;
            ui.blur();
            return;
        }

        // Поле ввода для добавления новой ссылки
        let at_limit = self.ics.len() >= crate::config::MAX_CALENDAR_URLS;
        let placeholder = if at_limit {
            crate::i18n::pick("Лимит: макс. 8 ссылок", "Limit: max 8 links")
        } else {
            crate::i18n::pick("https://… или webcal://…", "https://… or webcal://…")
        };
        let field_y = if wk.h > 100.0 { wk.y + 25.0 } else { wk.y + 22.0 };
        let field_h = if wk.h > 100.0 { 24.0 } else { 22.0 };
        ui.text_field_ex(
            cv,
            field,
            Rect::new(wk.x + 7.0, field_y, wk.w - 14.0, field_h),
            placeholder,
            icon::ADD,
            Submit::Keep,
        );

        // Сообщение об ошибке
        let mut list_top = field_y + field_h + 3.0;
        if self.cal_bad {
            let msg = crate::i18n::pick(
                "Нужна ссылка вида https:// или webcal://",
                "A link like https:// or webcal:// is required",
            );
            cv.text(msg, Rect::new(wk.x + 9.0, list_top, wk.w - 18.0, 12.0), &f.small, PINK);
            list_top += 14.0;
        }

        if self.ics.is_empty() {
            if !self.cal_bad {
                cv.text(
                    crate::i18n::pick("Enter — добавить, Esc — закрыть", "Enter — add, Esc — close"),
                    Rect::new(wk.x + 9.0, list_top, wk.w - 18.0, 12.0),
                    &f.small,
                    dim(),
                );
                if wk.h > 100.0 {
                    cv.text(
                        crate::i18n::pick(
                            "Google/Яндекс/Outlook: «секретный адрес iCal»",
                            "Google/Yandex/Outlook: “secret iCal address”",
                        ),
                        Rect::new(wk.x + 9.0, list_top + 16.0, wk.w - 18.0, 12.0),
                        &f.small,
                        mix(dim(), crate::ui::card(), 0.35),
                    );
                }
            }
            return;
        }

        // Список добавленных ссылок в сокращённом виде
        let view_h = (wk.bottom() - 4.0 - list_top).max(18.0);
        let view = Rect::new(wk.x + 7.0, list_top, wk.w - 14.0, view_h);
        const ROW_H: f32 = 19.0;
        let sid = id(TODAY, 65);
        let off = ui.scroll_begin(cv, sid, view, self.ics.len() as f32 * ROW_H);
        let mut remove_idx = None;

        for (i, url) in self.ics.iter().enumerate() {
            let ry = view.y + i as f32 * ROW_H - off;
            if ry + ROW_H < view.y || ry > view.bottom() {
                continue;
            }
            let row = Rect::new(view.x, ry, view.w, ROW_H - 1.0);
            let row_id = id(TODAY, 600 + i as u32);
            let hv = ui.hover_of(row_id);
            if hv > 0.01 {
                cv.rr(row, 3.0, D2D1_COLOR_F { a: 0.04 * hv, ..fg() });
            }
            cv.glyph(icon::LINK, Rect::new(row.x + 2.0, row.y + 2.5, 12.0, 12.0), false, dim());
            let abbrev = abbreviate_url(url);
            cv.text(&abbrev, Rect::new(row.x + 17.0, row.y + 1.0, row.w - 35.0, 13.0), &f.small, txt());
            let del_btn = Rect::new(row.right() - 17.0, row.y + 1.0, 14.0, 14.0);
            if ui.icon_button(cv, id(TODAY, 700 + i as u32), del_btn, icon::CLOSE, false) {
                remove_idx = Some(i);
            }
        }
        ui.scroll_end(cv, sid);

        if let Some(i) = remove_idx {
            self.ics.remove(i);
            crate::config::save_calendar_ics(&self.ics);
            online::refresh_calendar(&self.ics);
            self.sync();
            ui.focus = Some(field);
            ui.caret_on = true;
        }
    }

    /// Месячный календарь: дни с событиями помечены точками, клик по дню показывает его события.
    pub(super) unsafe fn month_card(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, today: i64) {
        const ROW: f32 = 22.0;
        const PITCH: f32 = 14.0;
        let f = &cv.g.fonts;
        cv.card(r);
        let (vy, vm) = self.view;
        let first = days_from_civil(vy, vm, 1);
        let start = first - weekday(first);
        let in_month = civil_from_days(days_from_civil(if vm == 12 { vy + 1 } else { vy }, vm % 12 + 1, 1) - 1).2 as i64;
        let rows = ((weekday(first) + in_month + 6) / 7) as usize;

        let key = (self.view, self.online.cal_ver);
        let evs = match self.cache.take() {
            Some((v, ver, e)) if (v, ver) == key => e,
            _ => online::events_by_day(&self.online.cal, start, 42, online::utc_offset_minutes()),
        };

        // Шапка: «Октябрь 2026» (клик — к текущему месяцу), стрелки и свернуть.
        let title = Rect::new(r.x + 9.0, r.y + 7.0, 118.0, 18.0);
        let (_, jump) = ui.click_area(id(TODAY, 91), title);
        let hv = ui.hover_of(id(TODAY, 91));
        cv.text(&format!("{} {}", crate::i18n::months_full()[vm as usize - 1], vy), title, &f.body, mix(txt(), cv.accent, hv));
        if self.online.calendar_failed {
            cv.circle(r.right() - 95.0, r.y + 16.0, 2.5, PINK);
        }
        let mut shift = 0;
        if ui.icon_button(cv, id(TODAY, 92), Rect::new(r.right() - 72.0, r.y + 7.0, 18.0, 18.0), icon::LEFT, false) {
            shift = -1;
        }
        if ui.icon_button(cv, id(TODAY, 93), Rect::new(r.right() - 50.0, r.y + 7.0, 18.0, 18.0), icon::RIGHT, false) {
            shift = 1;
        }
        let close = ui.icon_button(cv, id(TODAY, 94), Rect::new(r.right() - 26.0, r.y + 7.0, 18.0, 18.0), icon::UP, false);

        // Сетка месяца.
        let cw = (r.w - 14.0) / 7.0;
        for (c, name) in crate::i18n::weekdays().iter().enumerate() {
            cv.text(name, Rect::new(r.x + 7.0 + c as f32 * cw, r.y + 28.0, cw, 12.0), &f.small_c, dim());
        }
        let gy = r.y + 42.0;
        let mut pick = None;
        #[allow(clippy::needless_range_loop)]
        for idx in 0..rows * 7 {
            let day = start + idx as i64;
            let (_, mm, dd) = civil_from_days(day);
            let cell = Rect::new(r.x + 7.0 + (idx % 7) as f32 * cw, gy + (idx / 7) as f32 * ROW, cw, ROW);
            let b = Rect::new(cell.cx() - 12.5, cell.y, 25.0, ROW - 1.0);
            let cid = id(TODAY, 800 + idx as u32);
            let (_, clicked) = ui.click_area(cid, b);
            let hv = ui.hover_of(cid);
            let is_today = day == today;
            if is_today {
                cv.rr(b, 5.0, cv.accent);
            } else {
                if hv > 0.01 {
                    cv.rr(b, 5.0, D2D1_COLOR_F { a: 0.07 * hv, ..fg() });
                }
                if day == self.sel {
                    cv.rr_line(b, 5.0, cv.accent, 1.2);
                }
            }
            let col = if is_today { ink() } else if mm == vm { txt() } else { mix(dim(), crate::ui::card(), 0.3) };
            cv.text(&dd.to_string(), Rect::new(b.x, b.y - 1.0, b.w, 15.0), &f.body_c, col);
            let n = evs[idx].len().min(3);
            for k in 0..n {
                let dx = (k as f32 - (n as f32 - 1.0) / 2.0) * 4.5;
                cv.circle(b.cx() + dx, b.y + 16.0, 1.5, if is_today { ink() } else { PINK });
            }
            if clicked {
                pick = Some(day);
            }
        }

        // События выбранного дня.
        let top = gy + rows as f32 * ROW + 4.0;
        let det = Rect::new(r.x + 7.0, top, r.w - 14.0, r.bottom() - 7.0 - top);
        cv.rr(det, 8.0, inset());
        let (_, sm, sd) = civil_from_days(self.sel);
        let mut head = format!("{} {} {}", crate::i18n::weekdays()[weekday(self.sel) as usize], sd, crate::i18n::months_short()[(sm as usize).clamp(1, 12) - 1]);
        if self.sel == today {
            head.push_str(crate::i18n::pick(" • сегодня", " • today"));
        }
        cv.text(&head, Rect::new(det.x + 8.0, det.y + 3.0, det.w - 16.0, 14.0), &f.small_b, txt());
        let list: &[DayEvent] = evs.get((self.sel - start) as usize).map(Vec::as_slice).unwrap_or(&[]);
        let view = Rect::new(det.x + 4.0, det.y + 19.0, det.w - 8.0, (det.h - 21.0).max(PITCH));
        if list.is_empty() {
            let note = if !self.online.calendar_set {
                crate::i18n::pick(
                    "Календарь не подключён — нажмите, чтобы добавить ссылку",
                    "Calendar not connected — click to add a link",
                )
            } else if self.online.calendar_failed {
                if self.ics.len() > 1 {
                    crate::i18n::pick("Не удалось загрузить часть календарей", "Couldn't load some calendars")
                } else {
                    crate::i18n::t("Не удалось загрузить календарь")
                }
            } else if self.online.cal_ver == 0 {
                crate::i18n::pick("Загрузка…", "Loading…")
            } else {
                crate::i18n::t("Событий нет")
            };
            let nr = Rect::new(view.x + 4.0, view.y, view.w - 8.0, PITCH);
            cv.text(note, nr, &f.small, dim());
            if !self.online.calendar_set && ui.click_area(id(TODAY, 95), nr).1 {
                self.month_open = false;
                self.begin_calendar_edit(ui);
            }
        }
        let sid = id(TODAY, 101);
        let off = ui.scroll_begin(cv, sid, view, list.len() as f32 * PITCH);
        for (i, e) in list.iter().enumerate() {
            let y = view.y + i as f32 * PITCH - off;
            if y + PITCH < view.y || y > view.bottom() {
                continue;
            }
            cv.rr(Rect::new(view.x + 3.0, y + 2.5, 2.5, PITCH - 5.0), 1.25, PINK);
            let when = match e.minute {
                Some(m) => format!("{:02}:{:02}", m / 60, m % 60),
                None => crate::i18n::t("весь день").to_string(),
            };
            cv.text(&when, Rect::new(view.x + 10.0, y, 50.0, PITCH), &f.small_b, dim());
            cv.text(&e.title, Rect::new(view.x + 62.0, y, view.w - 70.0, PITCH), &f.small, txt());
        }
        ui.scroll_end(cv, sid);

        self.cache = Some((key.0, key.1, evs));
        if close {
            self.month_open = false;
        } else if jump {
            let (ty, tm, _) = civil_from_days(today);
            self.view = (ty, tm);
            self.sel = today;
        } else if shift != 0 {
            self.shift_month(shift, today);
        } else if let Some(day) = pick {
            let (y, m, _) = civil_from_days(day);
            self.sel = day;
            self.view = (y, m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_url_validation() {
        assert!(crate::config::valid_calendar_url("https://calendar.google.com/x.ics"));
        assert!(crate::config::valid_calendar_url("webcal://example.com/a.ics"));
        assert!(!crate::config::valid_calendar_url("calendar.google.com/x.ics"));
        assert!(!crate::config::valid_calendar_url("ftp://x"));
        assert!(!crate::config::valid_calendar_url(""));
    }

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
    fn abbreviate_calendar_url() {
        // Google Calendar: приватный токен и email скрыты за «…»
        let google = "https://calendar.google.com/calendar/ical/my.user%40gmail.com/private-9876543210abcdef/basic.ics";
        assert_eq!(abbreviate_url(google), "calendar.google.com/…/basic.ics");

        // Yandex Calendar: токен в query-параметре отсекается
        let yandex = "https://calendar.yandex.ru/export/ics.xml?private_token=secret_123456";
        assert_eq!(abbreviate_url(yandex), "calendar.yandex.ru/…/ics.xml");

        // Webcal с портом
        let webcal = "webcal://custom.server.org:8443/feeds/work.ics";
        assert_eq!(abbreviate_url(webcal), "custom.server.org/…/work.ics");

        // Ссылка без пути (только корень)
        assert_eq!(abbreviate_url("https://cal.example.com/"), "cal.example.com");
        assert_eq!(abbreviate_url("http://cal.example.com"), "cal.example.com");

        // Длинное имя хвоста (> 14 символов) сокращается с многоточием
        let long_tail = "https://server.com/calendars/verylongcalendareventname.ics";
        assert_eq!(abbreviate_url(long_tail), "server.com/…/verylongcale…");

        // Пустая строка
        assert_eq!(abbreviate_url(""), "");
        assert_eq!(abbreviate_url("   "), "");
    }
}
