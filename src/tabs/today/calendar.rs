//! Неделя, ближайшее событие и месячный календарь, ввод ссылки на календарь (iCal).

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::Today;
use crate::online::{self, civil_from_days, days_from_civil, weekday, DayEvent, Event};
use crate::tabs::{id, TODAY};
use crate::ui::{dim, fg, icon, inset, mix, status_pink, txt, Canvas, Rect, Submit, Ui};

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
    let hm = |t: i64| format!("{:02}:{:02}", t.rem_euclid(1440) / 60, t.rem_euclid(1440) % 60);
    // Конец у события на весь день — начало дня после последнего, поэтому «до» считаем от предыдущей минуты.
    let last_day = if ev.end > ev.start { (ev.end - 1).div_euclid(1440) } else { day };
    let until = || {
        let (_, m, d) = civil_from_days(last_day);
        format!("{d} {}", crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1])
    };
    let when = if ev.all_day {
        let all = crate::i18n::t("весь день");
        if last_day > day { format!("{all} → {}", until()) } else { all.to_string() }
    } else if ev.end <= ev.start {
        hm(ev.start)
    } else if last_day == day {
        format!("{}–{}", hm(ev.start), hm(ev.end))
    } else {
        format!("{} → {} {}", hm(ev.start), until(), hm(ev.end))
    };
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

/// Статус события на сегодня относительно текущего времени.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventStatus {
    /// Событие уже закончилось.
    Past,
    /// Событие идёт прямо сейчас (или длится весь день).
    Current,
    /// Событие ещё не началось.
    Future,
}

/// Определяет статус события на сегодня по времени начала, окончания и текущему времени (в минутах от полуночи).
pub fn event_status(start: Option<i64>, end: Option<i64>, now: i64) -> EventStatus {
    match start {
        None => EventStatus::Current,
        Some(s) => {
            let e = match end {
                Some(end_m) => end_m,
                None => s,
            };
            if s == e {
                if now < s {
                    EventStatus::Future
                } else if now == s {
                    EventStatus::Current
                } else {
                    EventStatus::Past
                }
            } else if now < s {
                EventStatus::Future
            } else if now < e {
                EventStatus::Current
            } else {
                EventStatus::Past
            }
        }
    }
}

/// Подготовленный элемент списка событий на сегодня для отображения в расширенной карточке.
#[derive(Clone, Debug, PartialEq)]
pub struct TodayEventItem {
    pub minute: Option<i64>,
    pub end_minute: Option<i64>,
    pub title: String,
    pub status: EventStatus,
    /// Подсвечено ли событие (текущее событие либо ближайшее будущее).
    pub highlighted: bool,
    /// Номер события, созданного в острове.
    pub local_id: Option<u64>,
}

/// Выбирает и подготавливает события на сегодня: вычисляет статус каждого события
/// и подсвечивает текущее (или ближайшее будущее, если ни одно событие сейчас не идёт).
pub fn select_today_events(events: &[DayEvent], now_minute: i64) -> Vec<TodayEventItem> {
    if events.is_empty() {
        return Vec::new();
    }
    let mut items: Vec<TodayEventItem> = events
        .iter()
        .map(|e| {
            let status = event_status(e.minute, e.end_minute, now_minute);
            TodayEventItem {
                minute: e.minute,
                end_minute: e.end_minute,
                title: e.title.clone(),
                status,
                highlighted: false,
                local_id: e.local_id,
            }
        })
        .collect();

    // Подсветка: сначала текущее событие со временем, затем любое текущее (весь день),
    // а если текущих нет — ближайшее будущее событие.
    let highlight_idx = items
        .iter()
        .position(|it| it.status == EventStatus::Current && it.minute.is_some())
        .or_else(|| items.iter().position(|it| it.status == EventStatus::Current))
        .or_else(|| items.iter().position(|it| it.status == EventStatus::Future));

    if let Some(idx) = highlight_idx {
        items[idx].highlighted = true;
    }

    items
}

/// Раскладка списка событий: сколько строк влезает и сколько осталось за кадром («+N ещё»).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventListLayout {
    pub visible_count: usize,
    pub more_count: usize,
}

/// Вычисляет, сколько строк событий помещается в заданную высоту `avail_h`.
/// Если помещаются все события, `more_count` = 0.
/// Если не все, резервируется место под плашку «+N ещё».
/// Если не влезает даже 1 строка (или 1 строка + плашка), возвращает `None` (деградация).
pub fn calculate_event_list_layout(
    total_events: usize,
    avail_h: f32,
    row_h: f32,
    more_h: f32,
) -> Option<EventListLayout> {
    if total_events == 0 || avail_h <= 0.0 || row_h <= 0.0 || more_h <= 0.0 {
        return None;
    }
    let all_h = total_events as f32 * row_h;
    if all_h <= avail_h + 0.01 {
        return Some(EventListLayout {
            visible_count: total_events,
            more_count: 0,
        });
    }
    if avail_h < row_h + more_h - 0.01 {
        return None;
    }
    let max_events = ((avail_h - more_h) / row_h).floor() as usize;
    let visible = max_events.min(total_events.saturating_sub(1));
    if visible == 0 {
        return None;
    }
    Some(EventListLayout {
        visible_count: visible,
        more_count: total_events - visible,
    })
}

/// Форматирует строку «+N ещё» / «+N more».
pub fn format_more_events(more: usize) -> String {
    format!("+{} {}", more, crate::i18n::pick("ещё", "more"))
}

/// Форматирует время начала события для строки списка.
pub fn format_event_time(minute: Option<i64>) -> String {
    match minute {
        Some(m) => format!("{:02}:{:02}", m / 60, m % 60),
        None => crate::i18n::t("весь день").to_string(),
    }
}

/// Время в списке дня: «10:00–11:30», если окончание в тот же день позже начала. Без окончания —
/// только начало; если событие уходит за полночь (`end` = 1440), — «10:00–…».
pub fn format_event_range(minute: Option<i64>, end: Option<i64>) -> String {
    let start = format_event_time(minute);
    match (minute, end) {
        (Some(_), Some(e)) if e >= 1440 => format!("{start}–…"),
        (Some(s), Some(e)) if e > s => format!("{start}–{}", format_event_time(Some(e))),
        _ => start,
    }
}

/// Ширина кнопки-метки «ICS» рядом с кнопкой добавления события.
const ICS_W: f32 = 26.0;

/// Кнопка-метка «ICS»: только текст, без подложки; при наведении подсвечивается. `true` — нажата.
unsafe fn ics_label(ui: &mut Ui, cv: &Canvas, r: Rect) -> bool {
    let cid = id(TODAY, 99);
    let (_, clicked) = ui.click_area(cid, r);
    let hv = ui.hover_of(cid);
    cv.text("ICS", r, &cv.g.fonts.small_c, mix(dim(), cv.accent, hv));
    ui.tip(cid, r, tip_ics());
    clicked
}

fn tip_add_event() -> &'static str {
    crate::i18n::pick("Добавить событие", "Add event")
}

fn tip_ics() -> &'static str {
    crate::i18n::pick("Подключить календарь по ссылке (iCal)", "Connect a calendar by link (iCal)")
}

/// Минимальная высота и ширина контейнера списка событий для показа расширенной формы.
const MIN_LIST_BOX_H: f32 = 70.0;
const MIN_LIST_BOX_W: f32 = 140.0;
const EVENT_ROW_H: f32 = 22.0;
/// Ширина колонки времени в списках событий: хватает на «10:00–11:30».
const TIME_COL_W: f32 = 74.0;
const EVENT_MORE_H: f32 = 20.0;

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

    /// Неделя. `size`: 0 и 1 — дни недели и блок ближайшего события (1 шире: дни расходятся
    /// по ширине карточки), 2 — расширенная форма со списком нескольких событий на сегодня.
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
        // В низкой карточке (только дни) ряды разнесены по всей высоте, иначе под ними — событие.
        let low = wk.h < 90.0;
        // Карточка 4×2 (104 px) ниже макета в 120: шапка и ряды дней подтянуты вверх, чтобы событие поместилось.
        let tight = !low && wk.h < 112.0;
        let head_cy = wk.y + if low { 17.0 } else if tight { 14.0 } else { 16.0 };
        let tw = cv.line("Эта неделя", &f.body, wk.x + 12.0, 120.0, head_cy, crate::ui::Align::Leading, txt());
        // Раскрыть месяц.
        if ui.icon_button(cv, id(TODAY, 90), Rect::new(wk.x + 12.0 + tw + 1.0, head_cy - 9.0, 18.0, 18.0), icon::DOWN, false, crate::i18n::pick("Месяц", "Month")) {
            self.open_month(today);
        }
        // Новое событие: месячный календарь раскрывается сразу с формой на сегодня.
        let add_r = Rect::new(wk.x + 12.0 + tw + 21.0, head_cy - 9.0, 18.0, 18.0);
        if ui.icon_button(cv, id(TODAY, 98), add_r, icon::ADD, false, tip_add_event()) {
            self.open_month(today);
            self.begin_event(ui, today);
        }
        let ics_r = Rect::new(add_r.right() + 2.0, head_cy - 9.0, ICS_W, 18.0);
        if ics_label(ui, cv, ics_r) {
            self.begin_calendar_edit(ui);
        }
        // Диапазон недели: полный, короткий или (в узкой карточке) без него, если не хватает места.
        let months = crate::i18n::months_short();
        let month_of = |m: u32| months[(m as usize).clamp(1, 12) - 1];
        let full = format!("{} {} – {} {}", dates[0].0, month_of(dates[0].1), dates[6].0, month_of(dates[6].1));
        let short = (dates[0].1 == dates[6].1).then(|| format!("{}–{} {}", dates[0].0, dates[6].0, month_of(dates[6].1)));
        let room = wk.right() - 12.0 - (ics_r.right() + 8.0);
        if let Some(range) = [Some(full), short].into_iter().flatten().find(|s| cv.text_w(s, &f.small_r) <= room) {
            cv.text(&range, Rect::new(wk.right() - 12.0 - room, head_cy - 7.0, room, 14.0), &f.small_r, dim());
        }
        let (label_cy, day_cy) = if low {
            (wk.y + 41.0, wk.y + 64.0)
        } else if tight {
            (wk.y + 30.0, wk.y + 46.0)
        } else {
            (wk.y + 34.0, wk.y + 54.0)
        };
        for (i, (day, _)) in dates.iter().enumerate() {
            let cx = wk.x + 24.0 + i as f32 * (wk.w - 48.0) / 6.0;
            // Выходные приглушены сильнее будней, как в Календаре iOS.
            let wd = if i >= 5 { mix(dim(), crate::ui::card(), 0.3) } else { dim() };
            cv.text(crate::i18n::weekdays()[i], Rect::new(cx - 14.0, label_cy - 6.0, 28.0, 12.0), &f.small_c, wd);
            let b = Rect::new(cx - 11.5, day_cy - 11.5, 23.0, 23.0);
            let is_today = i == dow;
            // Клик по дню открывает месяц с выбранным днём.
            let cid = id(TODAY, 80 + i as u32);
            let (_, clicked) = ui.click_area(cid, b);
            let hv = ui.hover_of(cid);
            // Сегодня — акцентом: красный календаря iOS на «Сегодня» лишний (#10), он остаётся в месяце.
            let cal = cv.accent;
            if is_today {
                cv.circle(b.cx(), b.cy(), b.h / 2.0, cal);
            } else if hv > 0.01 {
                cv.circle(b.cx(), b.cy(), b.h / 2.0, D2D1_COLOR_F { a: 0.07 * hv, ..fg() });
            }
            cv.text(&day.to_string(), b, &f.body_c, if is_today { crate::ui::on_color(cal) } else { txt() });
            if clicked {
                self.open_month(monday + i as i64);
            }
        }

        // Ближайшее событие или расширенный список (в низкой карточке компактного размера их нет).
        if low {
            return;
        }

        // В расширенном размере (size >= 2), если хватает места по высоте и ширине,
        // показываем СПИСОК событий на сегодня.
        if size >= 2 {
            let events_top = wk.y + 72.0;
            let events_bottom = wk.bottom() - 8.0;
            let events_h = events_bottom - events_top;
            let events_w = wk.w - 16.0;

            if events_h >= MIN_LIST_BOX_H && events_w >= MIN_LIST_BOX_W {
                let box_r = Rect::new(wk.x + 8.0, events_top, events_w, events_h);
                if self.draw_expanded_events(ui, cv, box_r, today) {
                    return;
                }
            }
            // Если места для списка не хватило или деградировали — переходим к обычному виду с одним событием.
        }

        self.draw_single_event(ui, cv, wk);
    }

    /// Обычный компактный вид с одним ближайшим событием (для стандартных размеров карточки
    /// или при деградации, когда расширенный список не влезает).
    unsafe fn draw_single_event(&mut self, ui: &mut Ui, cv: &Canvas, wk: Rect) {
        let f = &cv.g.fonts;
        let ev_y = if wk.h > 140.0 { wk.y + 72.0 } else if wk.h < 112.0 { wk.bottom() - 43.0 } else { wk.bottom() - 46.0 };
        let ev = Rect::new(wk.x + 8.0, ev_y, wk.w - 16.0, 38.0);
        cv.rr(ev, 8.0, inset());
        cv.rr(Rect::new(ev.x + 8.0, ev.y + 8.0, 3.0, 20.0), 1.5, cv.accent);
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
        // Событие острова правится кликом по блоку (карандаш); пока календарь не подключён, клик
        // ведёт к вводу ссылки; у событий из подключённого календаря кнопки нет.
        let local = self.next.as_ref().and_then(|e| e.local_id.map(|id| (id, e.start.div_euclid(1440))));
        let connect = self.next.is_none() && !self.online.calendar_set;
        let (_, clicked) = ui.click_area(id(TODAY, 63), ev);
        let hv = if local.is_some() || connect { ui.hover_of(id(TODAY, 63)) } else { 0.0 };
        if hv > 0.01 {
            cv.rr(ev, 8.0, D2D1_COLOR_F { a: 0.05 * hv, ..fg() });
            if local.is_some() {
                cv.glyph(icon::EDIT, Rect::new(ev.right() - 22.0, ev.y + 10.0, 16.0, 16.0), false, D2D1_COLOR_F { a: hv, ..dim() });
            }
        } else if self.online.calendar_failed {
            // Предупреждение об ошибке загрузки при наличии событий: нейтральная точка, без красного.
            cv.circle(ev.right() - 14.0, ev.y + 19.0, 2.5, dim());
        }
        if clicked {
            if let Some((lid, day)) = local {
                self.open_month(day);
                self.edit_event(ui, lid);
            } else if connect {
                self.begin_calendar_edit(ui);
            }
        }
    }

    /// Расширенная форма: список нескольких событий на сегодня.
    /// Возвращает `true`, если список успешно отрисован, или `false`, если места не хватило
    /// и карточка должна деградировать к одному событию.
    unsafe fn draw_expanded_events(&mut self, ui: &mut Ui, cv: &Canvas, box_r: Rect, today: i64) -> bool {
        let f = &cv.g.fonts;
        let today_days = online::events_by_day(&self.online.cal, today, 1, online::utc_offset_minutes());
        let raw_events = today_days.into_iter().next().unwrap_or_default();
        let now_min = online::now_local_minutes();
        let today_minute = now_min.rem_euclid(1440);
        let items = select_today_events(&raw_events, today_minute);

        cv.rr(box_r, 8.0, inset());

        // Шапка карточки событий на сегодня
        let head_r = Rect::new(box_r.x + 8.0, box_r.y + 6.0, box_r.w - 36.0, 14.0);
        cv.text(crate::i18n::pick("Сегодня", "Today"), head_r, &f.small_b, txt());

        if self.online.calendar_failed {
            cv.circle(box_r.right() - 28.0, box_r.y + 12.0, 2.5, dim());
        }

        let content_top = box_r.y + 24.0;
        let content_h = box_r.bottom() - 6.0 - content_top;

        if items.is_empty() {
            // Если на сегодня событий нет — аккуратная заглушка и ближайшее будущее событие
            let note = if !self.online.calendar_set {
                crate::i18n::pick("Календарь не подключён", "Calendar not connected")
            } else if self.online.calendar_failed {
                crate::i18n::pick("Не удалось загрузить", "Couldn't load")
            } else {
                crate::i18n::pick("На сегодня событий нет", "No events for today")
            };
            cv.text(note, Rect::new(box_r.x + 8.0, content_top, box_r.w - 16.0, 14.0), &f.small, dim());

            if let Some(ref e) = self.next {
                let next_y = content_top + 18.0;
                let next_h = (box_r.bottom() - 6.0 - next_y).min(44.0);
                if next_h >= 32.0 {
                    let next_r = Rect::new(box_r.x + 6.0, next_y, box_r.w - 12.0, next_h);
                    cv.rr(next_r, 6.0, D2D1_COLOR_F { a: 0.04, ..fg() });
                    cv.rr(Rect::new(next_r.x + 6.0, next_r.y + 7.0, 2.5, next_r.h - 14.0), 1.25, cv.accent);
                    let l1 = event_when(e, now_min);
                    cv.text(&l1, Rect::new(next_r.x + 14.0, next_r.y + 4.0, next_r.w - 20.0, 13.0), &f.small_b, txt());
                    cv.text(&e.title, Rect::new(next_r.x + 14.0, next_r.y + 17.0, next_r.w - 20.0, 13.0), &f.small, dim());
                }
            } else {
                let sub = if !self.online.calendar_set {
                    crate::i18n::pick("Нажмите, чтобы добавить ссылку", "Click to add a link")
                } else if self.online.calendar_failed {
                    if self.ics.len() > 1 {
                        crate::i18n::pick("Проверьте ссылки календаря", "Check calendar links")
                    } else {
                        crate::i18n::t("Проверьте ссылку calendar_ics")
                    }
                } else {
                    crate::i18n::pick("Календарь пуст на ближайшее время", "The calendar is empty for now")
                };
                cv.text(sub, Rect::new(box_r.x + 8.0, content_top + 18.0, box_r.w - 16.0, 14.0), &f.small, mix(dim(), crate::ui::card(), 0.35));
            }

            if !self.online.calendar_set && ui.click_area(id(TODAY, 95), box_r).1 {
                self.begin_calendar_edit(ui);
            }
            return true;
        }

        let layout = calculate_event_list_layout(items.len(), content_h, EVENT_ROW_H, EVENT_MORE_H);
        let Some(layout) = layout else {
            // Места не хватает даже для минимального списка без обрезки строк -> деградация
            return false;
        };

        let mut edit_id = None;
        for (i, item) in items[..layout.visible_count].iter().enumerate() {
            let ry = content_top + i as f32 * EVENT_ROW_H;
            let row_r = Rect::new(box_r.x + 4.0, ry, box_r.w - 8.0, EVENT_ROW_H - 2.0);

            let (strip_color, time_color, title_color) = if item.highlighted {
                let cal = cv.accent;
                cv.rr(row_r, 4.0, D2D1_COLOR_F { a: 0.10, ..cal });
                (cal, cal, txt())
            } else if item.status == EventStatus::Past {
                (D2D1_COLOR_F { a: 0.35, ..cv.accent }, dim(), mix(dim(), crate::ui::card(), 0.3))
            } else {
                (cv.accent, dim(), txt())
            };

            // Цветная полоска
            cv.rr(Rect::new(row_r.x + 3.0, row_r.y + 2.5, 3.0, row_r.h - 5.0), 1.5, strip_color);

            // Время начала или «весь день»
            // Событие острова: клик по строке открывает правку, при наведении виден карандаш.
            if let Some(lid) = item.local_id {
                let rid = id(TODAY, 910 + i as u32);
                if ui.click_area(rid, row_r).1 {
                    edit_id = Some(lid);
                }
                let hv = ui.hover_of(rid);
                if hv > 0.01 {
                    cv.rr(row_r, 4.0, D2D1_COLOR_F { a: 0.06 * hv, ..fg() });
                    cv.glyph(icon::EDIT, Rect::new(row_r.right() - 18.0, row_r.y + 3.0, 14.0, 14.0), false, D2D1_COLOR_F { a: hv, ..dim() });
                }
            }
            let when = format_event_range(item.minute, item.end_minute);
            cv.text(&when, Rect::new(row_r.x + 10.0, row_r.y + 2.0, TIME_COL_W, 14.0), &f.small_b, time_color);

            // Название события
            let title_w = (row_r.w - TIME_COL_W - 16.0).max(10.0);
            cv.text(&item.title, Rect::new(row_r.x + 14.0 + TIME_COL_W, row_r.y + 2.0, title_w, 14.0), &f.small, title_color);
        }

        if let Some(lid) = edit_id {
            self.open_month(today);
            self.edit_event(ui, lid);
        }

        if layout.more_count > 0 {
            let my = content_top + layout.visible_count as f32 * EVENT_ROW_H;
            let more_r = Rect::new(box_r.x + 4.0, my, box_r.w - 8.0, EVENT_MORE_H - 2.0);
            let cid = id(TODAY, 96);
            let (_, clicked) = ui.click_area(cid, more_r);
            let hv = ui.hover_of(cid);
            if hv > 0.01 {
                cv.rr(more_r, 4.0, D2D1_COLOR_F { a: 0.05 * hv, ..fg() });
            }
            let more_text = format_more_events(layout.more_count);
            cv.text(&more_text, Rect::new(more_r.x + 10.0, more_r.y + 2.0, more_r.w - 20.0, 14.0), &f.small_b, if hv > 0.01 { cv.accent } else { dim() });
            if clicked {
                self.open_month(today);
            }
        }

        true
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
        if ui.icon_button(cv, id(TODAY, 64), close_btn, icon::CLOSE, false, crate::i18n::pick("Закрыть", "Close")) {
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
            cv.text(msg, Rect::new(wk.x + 9.0, list_top, wk.w - 18.0, 12.0), &f.small, status_pink());
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
            if ui.icon_button(cv, id(TODAY, 700 + i as u32), del_btn, icon::CLOSE, false, crate::i18n::pick("Удалить подписку", "Remove subscription")) {
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
        const ROW: f32 = 20.0;
        const PITCH: f32 = 14.0;
        let f = &cv.g.fonts;
        cv.card(r);
        if self.ev_form.is_some() {
            self.event_form(ui, cv, r);
            return;
        }
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
        let name = format!("{} {}", crate::i18n::months_full()[vm as usize - 1], vy);
        let name_w = cv.text_w(&name, &f.body).ceil() + 2.0;
        let title = Rect::new(r.x + 9.0, r.y + 7.0, name_w, 18.0);
        let (_, jump) = ui.click_area(id(TODAY, 91), title);
        let hv = ui.hover_of(id(TODAY, 91));
        cv.text(&name, title, &f.body, mix(txt(), cv.accent, hv));
        // Новое событие и подключение календаря по ссылке — те же кнопки, что в карточке недели.
        let add_r = Rect::new(title.right() + 4.0, r.y + 7.0, 16.0, 18.0);
        let add = ui.icon_button(cv, id(TODAY, 97), add_r, icon::ADD, false, tip_add_event());
        if ics_label(ui, cv, Rect::new(add_r.right() + 2.0, r.y + 7.0, ICS_W, 18.0)) {
            self.month_open = false;
            self.begin_calendar_edit(ui);
        }
        if self.online.calendar_failed {
            cv.circle(r.right() - 62.0, r.y + 16.0, 2.5, dim());
        }
        let mut shift = 0;
        if ui.icon_button(cv, id(TODAY, 92), Rect::new(r.right() - 58.0, r.y + 7.0, 16.0, 18.0), icon::LEFT, false, crate::i18n::pick("Предыдущий месяц", "Previous month")) {
            shift = -1;
        }
        if ui.icon_button(cv, id(TODAY, 93), Rect::new(r.right() - 40.0, r.y + 7.0, 16.0, 18.0), icon::RIGHT, false, crate::i18n::pick("Следующий месяц", "Next month")) {
            shift = 1;
        }
        let close = ui.icon_button(cv, id(TODAY, 94), Rect::new(r.right() - 22.0, r.y + 7.0, 16.0, 18.0), icon::UP, false, crate::i18n::pick("Свернуть", "Collapse"));

        // Сетка месяца.
        let cw = (r.w - 14.0) / 7.0;
        for (c, name) in crate::i18n::weekdays().iter().enumerate() {
            cv.text(name, Rect::new(r.x + 7.0 + c as f32 * cw, r.y + 26.0, cw, 12.0), &f.small_c, dim());
        }
        let gy = r.y + 39.0;
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
            let cal = cv.accent;
            if is_today {
                cv.rr(b, 5.0, cal);
            } else {
                if hv > 0.01 {
                    cv.rr(b, 5.0, D2D1_COLOR_F { a: 0.07 * hv, ..fg() });
                }
                if day == self.sel {
                    cv.rr_line(b, 5.0, cal, 1.2);
                }
            }
            let col = if is_today { crate::ui::on_color(cal) } else if mm == vm { txt() } else { mix(dim(), crate::ui::card(), 0.3) };
            cv.text(&dd.to_string(), Rect::new(b.x, b.y - 1.0, b.w, 15.0), &f.body_c, col);
            let n = evs[idx].len().min(3);
            for k in 0..n {
                let dx = (k as f32 - (n as f32 - 1.0) / 2.0) * 4.5;
                cv.circle(b.cx() + dx, b.y + 15.5, 1.5, if is_today { crate::ui::on_color(cal) } else { cv.accent });
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
        let mut edit_id = None;
        let off = ui.scroll_begin(cv, sid, view, list.len() as f32 * PITCH);
        for (i, e) in list.iter().enumerate() {
            let y = view.y + i as f32 * PITCH - off;
            if y + PITCH < view.y || y > view.bottom() {
                continue;
            }
            cv.rr(Rect::new(view.x + 3.0, y + 2.5, 2.5, PITCH - 5.0), 1.25, cv.accent);
            if let Some(lid) = e.local_id {
                // Событие острова: клик по строке открывает правку.
                let row = Rect::new(view.x, y, view.w, PITCH);
                let rid = id(TODAY, 900 + i as u32);
                if ui.click_area(rid, row).1 {
                    edit_id = Some(lid);
                }
                let hv = ui.hover_of(rid);
                if hv > 0.01 {
                    cv.rr(row, 3.0, D2D1_COLOR_F { a: 0.05 * hv, ..fg() });
                }
            }
            let when = format_event_range(e.minute, e.end_minute);
            cv.text(&when, Rect::new(view.x + 10.0, y, TIME_COL_W, PITCH), &f.small_b, dim());
            cv.text(&e.title, Rect::new(view.x + 14.0 + TIME_COL_W, y, view.w - TIME_COL_W - 22.0, PITCH), &f.small, txt());
        }
        ui.scroll_end(cv, sid);

        self.cache = Some((key.0, key.1, evs));
        if let Some(lid) = edit_id {
            self.edit_event(ui, lid);
        } else if add {
            self.begin_event(ui, self.sel);
        } else if close {
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
        let ev = |day: i64, tod: i64, all_day| Event { start: day * 1440 + tod, end: 0, all_day, title: String::new(), local_id: None };
        let today = now / 1440;
        assert_eq!(event_when(&ev(today, 11 * 60, false), now), "Далее • 11:00");
        assert_eq!(event_when(&ev(today + 1, 9 * 60 + 30, false), now), "Завтра • 09:30");
        assert_eq!(event_when(&ev(today + 2, 0, true), now), "Сб 17 окт • весь день");
    }

    #[test]
    fn event_labels_with_end() {
        let now = days_from_civil(2026, 10, 15) * 1440 + 9 * 60;
        let today = now / 1440;
        let ev = |start: i64, end: i64, all_day| Event { start, end, all_day, title: String::new(), local_id: None };
        // Тот же день: диапазон.
        assert_eq!(event_when(&ev(today * 1440 + 600, today * 1440 + 690, false), now), "Далее • 10:00–11:30");
        // Окончание на другой день: дата и время конца.
        assert_eq!(
            event_when(&ev(today * 1440 + 22 * 60, (today + 1) * 1440 + 90, false), now),
            "Далее • 22:00 → 16 окт 01:30"
        );
        // Весь день на несколько дней: до последнего дня (конец — начало следующего).
        assert_eq!(event_when(&ev((today + 1) * 1440, (today + 4) * 1440, true), now), "Завтра • весь день → 18 окт");
        // Весь день на один день: без «до».
        assert_eq!(event_when(&ev((today + 1) * 1440, (today + 2) * 1440, true), now), "Завтра • весь день");
    }

    #[test]
    fn event_range_in_lists() {
        assert_eq!(format_event_range(Some(600), Some(690)), "10:00–11:30");
        assert_eq!(format_event_range(Some(600), None), "10:00");
        assert_eq!(format_event_range(Some(600), Some(600)), "10:00");
        assert_eq!(format_event_range(Some(1320), Some(1440)), "22:00–…");
        assert_eq!(format_event_range(None, None), "весь день");
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

    #[test]
    fn event_status_past_current_future() {
        // Событие на весь день: сегодня всегда Current
        assert_eq!(event_status(None, None, 0), EventStatus::Current);
        assert_eq!(event_status(None, None, 720), EventStatus::Current);
        assert_eq!(event_status(None, None, 1439), EventStatus::Current);

        // Событие с диапазоном 10:00 (600) — 11:00 (660)
        let s = Some(600);
        let e = Some(660);
        assert_eq!(event_status(s, e, 599), EventStatus::Future);
        assert_eq!(event_status(s, e, 600), EventStatus::Current);
        assert_eq!(event_status(s, e, 630), EventStatus::Current);
        assert_eq!(event_status(s, e, 659), EventStatus::Current);
        assert_eq!(event_status(s, e, 660), EventStatus::Past);
        assert_eq!(event_status(s, e, 700), EventStatus::Past);

        // Точечное событие без окончания (start == end)
        assert_eq!(event_status(s, s, 599), EventStatus::Future);
        assert_eq!(event_status(s, s, 600), EventStatus::Current);
        assert_eq!(event_status(s, s, 601), EventStatus::Past);
        assert_eq!(event_status(s, None, 599), EventStatus::Future);
        assert_eq!(event_status(s, None, 600), EventStatus::Current);
        assert_eq!(event_status(s, None, 601), EventStatus::Past);
    }

    #[test]
    fn select_today_events_and_highlighting() {
        let ev = |m: Option<i64>, end: Option<i64>, title: &str| DayEvent {
            minute: m,
            end_minute: end,
            title: title.to_string(),
            local_id: None,
        };

        // Пустой список
        assert!(select_today_events(&[], 600).is_empty());

        // События: прошлое (9:00-10:00), будущее-1 (12:00-13:00), будущее-2 (15:00-16:00)
        // Текущее время 11:00 (660). Ни одно не идёт -> первое будущее (12:00) подсвечено.
        let list = vec![
            ev(Some(540), Some(600), "Morning"),
            ev(Some(720), Some(780), "Lunch"),
            ev(Some(900), Some(960), "Evening"),
        ];
        let items = select_today_events(&list, 660);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].status, EventStatus::Past);
        assert!(!items[0].highlighted);
        assert_eq!(items[1].status, EventStatus::Future);
        assert!(items[1].highlighted); // ближайшее подсвечено
        assert_eq!(items[2].status, EventStatus::Future);
        assert!(!items[2].highlighted);

        // Время 12:30 (750) — Lunch идёт прямо сейчас.
        let items = select_today_events(&list, 750);
        assert_eq!(items[0].status, EventStatus::Past);
        assert!(!items[0].highlighted);
        assert_eq!(items[1].status, EventStatus::Current);
        assert!(items[1].highlighted); // текущее подсвечено
        assert_eq!(items[2].status, EventStatus::Future);
        assert!(!items[2].highlighted);

        // Время 17:00 (1020) — все события закончились. Ни одно не подсвечено, все приглушены.
        let items = select_today_events(&list, 1020);
        assert_eq!(items[0].status, EventStatus::Past);
        assert_eq!(items[1].status, EventStatus::Past);
        assert_eq!(items[2].status, EventStatus::Past);
        assert!(!items.iter().any(|it| it.highlighted));

        // Событие на весь день и встреча в 14:00. При 10:00 — активен весь день.
        let list2 = vec![
            ev(None, None, "Holiday"),
            ev(Some(840), Some(900), "Call"),
        ];
        let items = select_today_events(&list2, 600);
        assert_eq!(items[0].status, EventStatus::Current);
        assert!(items[0].highlighted);
        assert_eq!(items[1].status, EventStatus::Future);
        assert!(!items[1].highlighted);

        // При 14:15 — встреча со временем приоритетнее для подсветки, чем событие на весь день.
        let items = select_today_events(&list2, 855);
        assert_eq!(items[0].status, EventStatus::Current);
        assert!(!items[0].highlighted);
        assert_eq!(items[1].status, EventStatus::Current);
        assert!(items[1].highlighted);
    }

    #[test]
    fn calculate_event_list_layout_fits_and_more() {
        let row_h = 22.0;
        let more_h = 20.0;

        // Места хватает на все события
        let lay = calculate_event_list_layout(3, 100.0, row_h, more_h).unwrap();
        assert_eq!(lay.visible_count, 3);
        assert_eq!(lay.more_count, 0);

        // Точно на грани: 3 * 22 = 66
        let lay = calculate_event_list_layout(3, 66.0, row_h, more_h).unwrap();
        assert_eq!(lay.visible_count, 3);
        assert_eq!(lay.more_count, 0);

        // 6 событий в 70px: все (132px) не влезают.
        // Место под «+N ещё» резервируется: (70 - 20) / 22 = 2.
        let lay = calculate_event_list_layout(6, 70.0, row_h, more_h).unwrap();
        assert_eq!(lay.visible_count, 2);
        assert_eq!(lay.more_count, 4);

        // 2 события в 35px: 2 * 22 = 44 > 35, а для 1 события + «ещё» нужно 22 + 20 = 42 > 35.
        // Возвращает None -> деградация к одиночному событию без обрезанных строк.
        assert_eq!(calculate_event_list_layout(2, 35.0, row_h, more_h), None);

        // Тесная высота, меньше 1 строки
        assert_eq!(calculate_event_list_layout(5, 15.0, row_h, more_h), None);

        // 0 событий или некорректные размеры
        assert_eq!(calculate_event_list_layout(0, 100.0, row_h, more_h), None);
        assert_eq!(calculate_event_list_layout(3, 0.0, row_h, more_h), None);
        assert_eq!(calculate_event_list_layout(3, 100.0, 0.0, more_h), None);
    }

    #[test]
    fn format_more_and_event_time() {
        assert_eq!(format_more_events(3), "+3 ещё");
        assert_eq!(format_event_time(Some(600)), "10:00");
        assert_eq!(format_event_time(Some(545)), "09:05");
        assert_eq!(format_event_time(None), "весь день");
    }

    #[test]
    fn week_widget_sizes_and_layout_compatibility() {
        use crate::tabs::today::SPEC;
        use crate::tabs::layout::{sz, Layout};

        // Размеры виджета "week" — стандартные, в крупных ячейках
        let week_spec = &SPEC.specs[2];
        assert_eq!(week_spec.key, "week");
        assert_eq!(week_spec.sizes.len(), 3);
        assert_eq!(week_spec.sizes[0], sz(4, 2));
        assert_eq!(week_spec.sizes[1], sz(6, 2));
        assert_eq!(week_spec.sizes[2], sz(4, 4));

        // Размер по умолчанию — индекс 0 (248×120 px)
        assert_eq!(week_spec.size, 0);

        // Раскладка по умолчанию не сломалась
        let def = Layout::default_for(&SPEC);
        let week_entry = def.entry(2).unwrap();
        assert_eq!(week_entry.size, 0);

        // Разбор из конфига: индексы 0, 1, 2 распознаются корректно
        for n in 0..3 {
            let l = Layout::parse(&SPEC, &format!("v2 week:{n}"));
            assert_eq!(l.entry(2).unwrap().size, n);
        }

        // Невалидный индекс размера откатывается на размер по умолчанию (0)
        let linv = Layout::parse(&SPEC, "v2 week:99");
        assert_eq!(linv.entry(2).unwrap().size, 0);

        // Старая раскладка на мелкой сетке: 16×8 → 4×2, 24×8 → 6×2, 16×16 → 4×4
        for (old, new) in [(1, 0), (2, 1), (3, 2)] {
            let l = Layout::parse(&SPEC, &format!("week:{old}@8,0"));
            assert_eq!(l.entry(2).unwrap().size, new, "старый размер {old}");
            assert_eq!((l.entry(2).unwrap().col, l.entry(2).unwrap().row), (2, 0));
        }
    }
}
