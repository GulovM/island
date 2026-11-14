//! Неделя, ближайшее событие и месячный календарь, ввод ссылки на календарь (iCal).

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::Today;
use crate::online::{self, civil_from_days, days_from_civil, weekday, DayEvent, Event};
use crate::tabs::{id, TODAY};
use crate::ui::{
    dim, fg, hairline, icon, inset, mix, readable, status_blue, status_green, status_orange,
    status_pink, txt, Canvas, Corner, Rect, Submit, Ui,
};

// Формы подключения аккаунтов. Свой блок, потому что форма рисуется на одной вкладке с таймером
// и формой события: общий ID заставлял таймер писать минуты в поле Apple ID.
const CAL_GOOGLE_BTN: u32 = 140;
const CAL_APPLE_TOGGLE_BTN: u32 = 141;
const CAL_APPLE_ID_FIELD: u32 = 142;
const CAL_APPLE_PW_FIELD: u32 = 143;
const CAL_APPLE_SUBMIT_BTN: u32 = 144;
const CAL_APPLE_CANCEL_BTN: u32 = 145;
const CAL_ICS_ADD_BTN: u32 = 146;
const CAL_EMPTY_ADD_BTN: u32 = 147;
const CAL_BACK_BTN: u32 = 148;
const CAL_MONTH_CONN_BTN: u32 = 149;
/// Кнопки удаления и строки ссылок iCal (до `MAX_CALENDAR_URLS` штук).
const URL_DEL: u32 = 700;
const URL_ROW: u32 = 720;
/// Строки событий: список выбранного дня в месяце и карточка недели. Кликабельны первые
/// `ROW_IDS` строк каждого списка: дальше ID зашли бы в чужой блок.
const DAY_ROW: u32 = 900;
const WEEK_ROW: u32 = 1000;
const ROW_IDS: usize = 100;

/// Занятые карточками календаря ID вкладки (полуинтервалы); непересечение с соседями — тест в `mod.rs`.
#[cfg(test)]
pub(super) const ID_RANGES: &[(u32, u32)] = &[
    (62, 66),
    (80, 87),
    (90, 100),
    (101, 102),
    (CAL_GOOGLE_BTN, CAL_MONTH_CONN_BTN + 1),
    (URL_DEL, URL_DEL + crate::config::MAX_CALENDAR_URLS as u32),
    (URL_ROW, URL_ROW + crate::config::MAX_CALENDAR_URLS as u32),
    (800, 842),
    (DAY_ROW, DAY_ROW + ROW_IDS as u32),
    (WEEK_ROW, WEEK_ROW + ROW_IDS as u32),
];

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

/// Строка времени события без префикса: «15:00–15:45», «11:00», «весь день», «весь день → 18 окт», «22:00 → 16 окт 01:30».
fn event_time_str(ev: &Event) -> String {
    let day = ev.start.div_euclid(1440);
    let hm = |t: i64| format!("{:02}:{:02}", t.rem_euclid(1440) / 60, t.rem_euclid(1440) % 60);
    // Конец у события на весь день — начало дня после последнего, поэтому «до» считаем от предыдущей минуты.
    let last_day = if ev.end > ev.start { (ev.end - 1).div_euclid(1440) } else { day };
    let until = || {
        let (_, m, d) = civil_from_days(last_day);
        format!("{d} {}", crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1])
    };
    if ev.all_day {
        let all = crate::i18n::t("весь день");
        if last_day > day { format!("{all} → {}", until()) } else { all.to_string() }
    } else if ev.end <= ev.start {
        hm(ev.start)
    } else if last_day == day {
        format!("{}–{}", hm(ev.start), hm(ev.end))
    } else {
        format!("{} → {} {}", hm(ev.start), until(), hm(ev.end))
    }
}

/// Первая строка блока события: «Далее • 11:00», «Завтра • 09:30», «Пт 16 окт • весь день».
fn event_when(ev: &Event, now: i64) -> String {
    let day = ev.start.div_euclid(1440);
    let today = now.div_euclid(1440);
    let when = event_time_str(ev);
    if day == today {
        format!("{} • {when}", crate::i18n::pick("Далее", "Next"))
    } else if day == today + 1 {
        format!("{} • {when}", crate::i18n::pick("Завтра", "Tomorrow"))
    } else {
        let (_, m, d) = civil_from_days(day);
        format!("{} {} {} • {when}", crate::i18n::weekdays()[weekday(day) as usize], d, crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1])
    }
}

/// Относительное время события: «через 40 мин», «сейчас», «Завтра», «Пт 16 окт».
fn event_relative_time(ev: &Event, now: i64) -> String {
    let day = ev.start.div_euclid(1440);
    let today = now.div_euclid(1440);
    if day == today {
        if now < ev.start {
            let diff = ev.start - now;
            if diff < 60 {
                crate::i18n::pick(&format!("через {diff} мин"), &format!("in {diff} min")).to_string()
            } else {
                let h = diff / 60;
                let m = diff % 60;
                if m == 0 {
                    crate::i18n::pick(&format!("через {h} ч"), &format!("in {h} h")).to_string()
                } else {
                    crate::i18n::pick(&format!("через {h} ч {m} мин"), &format!("in {h} h {m} min")).to_string()
                }
            }
        } else if now < ev.end || (ev.end <= ev.start && now == ev.start) {
            crate::i18n::pick("сейчас", "now").to_string()
        } else {
            crate::i18n::pick("завершено", "ended").to_string()
        }
    } else if day == today + 1 {
        crate::i18n::pick("Завтра", "Tomorrow").to_string()
    } else {
        let (_, m, d) = civil_from_days(day);
        format!("{} {} {}", crate::i18n::weekdays()[weekday(day) as usize], d, crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1])
    }
}

/// Относительное время события на сегодня в минутах от полуночи: «через 40 мин», «сейчас».
fn today_event_relative_time(minute: Option<i64>, end_minute: Option<i64>, today_minute: i64) -> String {
    match minute {
        None => crate::i18n::pick("сейчас", "now").to_string(),
        Some(s) => {
            if today_minute < s {
                let diff = s - today_minute;
                if diff < 60 {
                    crate::i18n::pick(&format!("через {diff} мин"), &format!("in {diff} min")).to_string()
                } else {
                    let h = diff / 60;
                    let m = diff % 60;
                    if m == 0 {
                        crate::i18n::pick(&format!("через {h} ч"), &format!("in {h} h")).to_string()
                    } else {
                        crate::i18n::pick(&format!("через {h} ч {m} мин"), &format!("in {h} h {m} min")).to_string()
                    }
                }
            } else {
                let e = end_minute.unwrap_or(s);
                if today_minute < e || (e <= s && today_minute == s) {
                    crate::i18n::pick("сейчас", "now").to_string()
                } else {
                    crate::i18n::pick("завершено", "ended").to_string()
                }
            }
        }
    }
}

/// Длительность события: «15 мин», «1 ч», «1 ч 30 мин», «весь день».
fn format_event_duration(minute: Option<i64>, end_minute: Option<i64>) -> String {
    match (minute, end_minute) {
        (None, _) => crate::i18n::t("весь день").to_string(),
        (Some(s), Some(e)) if e > s => {
            let dur = e - s;
            if dur < 60 {
                crate::i18n::pick(&format!("{dur} мин"), &format!("{dur} min")).to_string()
            } else {
                let h = dur / 60;
                let m = dur % 60;
                if m == 0 {
                    crate::i18n::pick(&format!("{h} ч"), &format!("{h} h")).to_string()
                } else {
                    crate::i18n::pick(&format!("{h} ч {m} мин"), &format!("{h} h {m} min")).to_string()
                }
            }
        }
        _ => String::new(),
    }
}

/// Заголовок секции событий на сегодня с правильным склонением: «СЕГОДНЯ · 4 СОБЫТИЯ» / «TODAY · 4 EVENTS».
fn format_today_events_header(count: usize) -> String {
    let ru = match (count % 100, count % 10) {
        (11..=19, _) => format!("СЕГОДНЯ · {count} СОБЫТИЙ"),
        (_, 1) => format!("СЕГОДНЯ · {count} СОБЫТИЕ"),
        (_, 2..=4) => format!("СЕГОДНЯ · {count} СОБЫТИЯ"),
        _ => format!("СЕГОДНЯ · {count} СОБЫТИЙ"),
    };
    let en = if count == 1 {
        format!("TODAY · {count} EVENT")
    } else {
        format!("TODAY · {count} EVENTS")
    };
    crate::i18n::pick(&ru, &en).to_string()
}

/// Детерминированный цвет события из набора [status_blue, status_green, status_pink, status_orange].
/// Для ближайшего/текущего события — цвет темы (`accent`), одинаково во всех палитрах и размерах.
fn event_color(idx: usize, is_nearest: bool, accent: D2D1_COLOR_F) -> D2D1_COLOR_F {
    if is_nearest {
        accent
    } else {
        match idx % 4 {
            0 => status_blue(),
            1 => status_green(),
            2 => status_pink(),
            _ => status_orange(),
        }
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

/// Ширина левой секции (дни недели) в карточке 6x2 (~60% от 328 px).
const WEEK_WIDE_LEFT_W: f32 = 200.0;
/// Высота строки расписания в расширенной карточке недели (4x4).
const SCHEDULE_ROW_H: f32 = 28.0;
/// Высота плашки «+N ещё» в расписании недели.
const SCHEDULE_MORE_H: f32 = 20.0;
/// Ширина колонки времени в расписании недели.
const SCHEDULE_TIME_W: f32 = 32.0;
/// Толщина цветной полоски события.
const EVENT_STRIP_W: f32 = 3.0;
/// Радиус точки наличия событий под числом дня.
const DAY_DOT_R: f32 = 1.6;
/// Радиус точки отметки «сейчас» на временной шкале.
const NOW_MARKER_R: f32 = 2.5;

/// Кнопка «+» создания события в нейтральном сером стиле (совпадает с кнопкой настроек).
unsafe fn add_event_button(ui: &mut Ui, cv: &Canvas, cid: u32, r: Rect) -> bool {
    ui.icon_button(cv, cid, r, icon::ADD, false, crate::i18n::pick("Создать событие", "New event"))
}

/// Кнопка настроек подключений календаря (Google, Apple iCloud, iCal) с точкой статуса.
unsafe fn connections_button(
    ui: &mut Ui,
    cv: &Canvas,
    cid: u32,
    r: Rect,
    calendar_set: bool,
    calendar_failed: bool,
) -> bool {
    let tip = if calendar_failed {
        crate::i18n::pick("Ошибка календаря — проверить подключения", "Calendar error — check connections")
    } else if !calendar_set {
        crate::i18n::pick("Подключить Google, Apple или iCal", "Connect Google, Apple or iCal")
    } else {
        crate::i18n::pick("Аккаунты и календари", "Accounts & calendars")
    };
    let clicked = ui.icon_button(cv, cid, r, icon::SETTINGS, false, tip);
    if calendar_failed {
        cv.circle(r.right() - 2.5, r.y + 2.5, 2.2, status_pink());
    } else if !calendar_set {
        cv.circle(r.right() - 2.5, r.y + 2.5, 2.0, cv.accent);
    }
    clicked
}

/// Компактная пилюля-действие для пустых и ошибочных состояний.
unsafe fn compact_pill_btn(
    ui: &mut Ui,
    cv: &Canvas,
    cid: u32,
    r: Rect,
    label: &str,
    accent: bool,
) -> bool {
    let (_, clicked) = ui.click_area(cid, r);
    let hv = ui.hover_of(cid);
    let (bg, text_col) = if accent {
        let fill = D2D1_COLOR_F { a: 0.16 + 0.84 * hv, ..cv.accent };
        let fg_col = mix(readable(cv.accent, 60.0), crate::ui::on_color(cv.accent), hv);
        (fill, fg_col)
    } else {
        let fill = mix(crate::ui::chip_bg(), fg(), 0.08 * hv);
        (fill, txt())
    };
    cv.rr(r, r.h / 2.0, bg);
    cv.text(label, r, &cv.g.fonts.small_c, text_col);
    clicked
}

/// Ширина колонки времени в месячном списке событий: хватает на «10:00–11:30».
const TIME_COL_W: f32 = 74.0;

/// Параметры раскладки ряда дней недели.
struct DaysRowLayout {
    left: f32,
    margin: f32,
    step: f32,
    label_cy: f32,
    day_cy: f32,
}

/// Отрисовка шапки карточки недели: единая пилюля месяца с шевроном и кнопки «+» / подключений.
unsafe fn draw_week_header(
    today_tab: &mut Today,
    ui: &mut Ui,
    cv: &Canvas,
    left: f32,
    right: f32,
    head_cy: f32,
    today: i64,
) {
    let f = &cv.g.fonts;
    let btn_sz = 18.0;
    let conn_r = Rect::new(right - btn_sz, head_cy - btn_sz / 2.0, btn_sz, btn_sz);
    let add_r = Rect::new(conn_r.x - 4.0 - btn_sz, head_cy - btn_sz / 2.0, btn_sz, btn_sz);

    let (_, month, _) = civil_from_days(today);
    let month_name = crate::i18n::months_full()[(month as usize).clamp(1, 12) - 1].to_uppercase();
    let mw = cv.text_w(&month_name, &f.caps);
    let max_pill_w = (add_r.x - 6.0 - (left - 4.0)).max(36.0);
    let pill_w = (mw + 18.0).min(max_pill_w);
    let month_r = Rect::new(left - 4.0, head_cy - 9.0, pill_w, 18.0);
    let (_, month_clicked) = ui.click_area(id(TODAY, 90), month_r);
    let mhv = ui.hover_of(id(TODAY, 90));
    if mhv > 0.01 {
        cv.rr(month_r, 9.0, D2D1_COLOR_F { a: 0.08 * mhv, ..fg() });
    }
    let text_w = (pill_w - 16.0).max(10.0);
    cv.text(&month_name, Rect::new(left, head_cy - 5.5, text_w, 11.0), &f.caps, mix(dim(), txt(), mhv));
    cv.glyph(icon::DOWN, Rect::new(left + text_w + 1.0, head_cy - 5.0, 10.0, 10.0), false, mix(dim(), cv.accent, mhv));
    ui.tip(id(TODAY, 90), month_r, crate::i18n::pick("Открыть месяц", "Open month"));
    if month_clicked {
        today_tab.open_month(today);
    }

    if add_event_button(ui, cv, id(TODAY, 98), add_r) {
        today_tab.open_month(today);
        today_tab.begin_event(ui, today);
    }

    if connections_button(
        ui,
        cv,
        id(TODAY, 99),
        conn_r,
        today_tab.online.calendar_set,
        today_tab.online.calendar_failed,
    ) {
        today_tab.begin_calendar_edit(ui);
    }
}

/// Отрисовка ряда дней Пн..Вс: подпись дня мелко, число в капсуле для сегодня, точка событий.
unsafe fn draw_days_row(
    today_tab: &mut Today,
    ui: &mut Ui,
    cv: &Canvas,
    lay: &DaysRowLayout,
    today: i64,
    dow: usize,
    week_has_events: &[bool; 7],
) {
    let f = &cv.g.fonts;
    let (y, m, d) = civil_from_days(today);
    let dates = week_dates(y, m, d);
    let monday = today - dow as i64;
    // Сегодняшний день — цветом темы (во всех палитрах и размерах).
    let pill_fill = cv.accent;
    let on_pill = crate::ui::on_color(pill_fill);

    for (i, (day, _)) in dates.iter().enumerate() {
        let cx = lay.left + lay.margin + i as f32 * lay.step;
        let is_today = i == dow;
        // Вертикальная скруглённая плашка дня недели, числа и точки событий.
        let pill_top = lay.label_cy - 6.5;
        let dot_y = lay.day_cy + 10.5;
        let pill_h = (dot_y + 3.5) - pill_top;
        let pill_rect = Rect::new(cx - 12.0, pill_top, 24.0, pill_h);
        let cid = id(TODAY, 80 + i as u32);
        let (_, clicked) = ui.click_area(cid, pill_rect);
        let hv = ui.hover_of(cid);

        // Выходные дни приглушены сильнее будней
        let wd = if is_today { D2D1_COLOR_F { a: 0.95, ..on_pill } } else if i >= 5 { mix(dim(), crate::ui::card(), 0.3) } else { dim() };
        let num_col = if is_today { on_pill } else if i >= 5 { dim() } else { txt() };

        if is_today {
            cv.rr(pill_rect, 11.5, pill_fill);
        } else if hv > 0.01 {
            cv.rr(pill_rect, 11.5, D2D1_COLOR_F { a: 0.07 * hv, ..fg() });
        }
        cv.text(crate::i18n::weekdays()[i], Rect::new(cx - 14.0, lay.label_cy - 6.0, 28.0, 12.0), &f.small_c, wd);
        cv.text(&day.to_string(), Rect::new(cx - 10.0, lay.day_cy - 7.0, 20.0, 14.0), &f.body_c, num_col);

        // Точка наличия событий под числом дня
        if week_has_events[i] {
            let dot_col = if is_today { D2D1_COLOR_F { a: 0.9, ..on_pill } } else { dim() };
            cv.circle(cx, dot_y, DAY_DOT_R, dot_col);
        }

        if clicked {
            today_tab.open_month(monday + i as i64);
        }
    }
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

    /// Неделя. Размеры карточки:
    /// 4x2 = 216x104 (size 0) — дни недели и плашка ближайшего события;
    /// 6x2 = 328x104 (size 1) — дни слева (~60%), колонка ближайшего события справа;
    /// 4x4 = 216x216 (size 2) — дни сверху, расписание событий на сегодня снизу.
    pub(super) unsafe fn week_card(&mut self, ui: &mut Ui, cv: &Canvas, wk: Rect, size: usize, today: i64, dow: usize) {
        cv.card(wk);
        // Цветной ореол в правом верхнем углу (только в стиле Windows)
        cv.glow(wk, crate::ui::card_radius(), cv.accent, Corner::TopRight);

        let monday = today - dow as i64;
        let week_evs = online::events_by_day(&self.online.cal, monday, 7, online::utc_offset_minutes());
        let mut week_has_events = [false; 7];
        for (i, day_evs) in week_evs.iter().enumerate().take(7) {
            week_has_events[i] = !day_evs.is_empty();
        }

        let low = wk.h < 90.0;
        let is_wide = size == 1 || (size != 2 && wk.w >= 260.0);
        let is_tall = size >= 2 || wk.h >= 160.0;

        if is_wide {
            // 6x2 (size 1): левая секция ~60% ширины
            let left_w = WEEK_WIDE_LEFT_W;
            let head_cy = wk.y + 16.0;
            draw_week_header(self, ui, cv, wk.x + 12.0, wk.x + left_w - 8.0, head_cy, today);

            let lay = DaysRowLayout {
                left: wk.x,
                margin: 17.0,
                step: (left_w - 34.0) / 6.0,
                label_cy: wk.y + 40.0,
                day_cy: wk.y + 58.0,
            };
            draw_days_row(self, ui, cv, &lay, today, dow, &week_has_events);

            // Вертикальный тонкий разделитель
            let sep_x = wk.x + left_w;
            cv.rr(Rect::new(sep_x, wk.y + 10.0, 1.0, wk.h - 20.0), 0.5, hairline());

            // Правая колонка
            self.draw_single_event(ui, cv, wk);
        } else if is_tall {
            // 4x4 (size 2): шапка + дни, разделитель, расписание
            let head_cy = wk.y + 14.5;
            draw_week_header(self, ui, cv, wk.x + 12.0, wk.right() - 12.0, head_cy, today);

            let lay = DaysRowLayout {
                left: wk.x,
                margin: 24.0,
                step: (wk.w - 48.0) / 6.0,
                label_cy: wk.y + 31.5,
                day_cy: wk.y + 45.5,
            };
            draw_days_row(self, ui, cv, &lay, today, dow, &week_has_events);

            // Горизонтальный тонкий разделитель
            let sep_y = wk.y + 65.0;
            cv.rr(Rect::new(wk.x + 12.0, sep_y, wk.w - 24.0, 1.0), 0.5, hairline());

            let box_r = Rect::new(wk.x, sep_y + 1.0, wk.w, wk.bottom() - (sep_y + 1.0));
            if !self.draw_expanded_events(ui, cv, box_r, today) {
                self.draw_single_event(ui, cv, wk);
            }
        } else {
            // 4x2 (size 0) или компактная низкая карточка
            let head_cy = wk.y + if low { 17.0 } else { 14.0 };
            draw_week_header(self, ui, cv, wk.x + 12.0, wk.right() - 12.0, head_cy, today);

            let (label_cy, day_cy) = if low {
                (wk.y + 41.0, wk.y + 64.0)
            } else {
                (wk.y + 30.0, wk.y + 43.5)
            };
            let lay = DaysRowLayout {
                left: wk.x,
                margin: 24.0,
                step: (wk.w - 48.0) / 6.0,
                label_cy,
                day_cy,
            };
            draw_days_row(self, ui, cv, &lay, today, dow, &week_has_events);

            if !low {
                self.draw_single_event(ui, cv, wk);
            }
        }
    }

    /// Блок ближайшего события:
    /// в 4x2 — компактная плашка снизу под днями;
    /// в 6x2 — вертикальная колонка справа («ДАЛЕЕ», название, время, «через N мин»).
    unsafe fn draw_single_event(&mut self, ui: &mut Ui, cv: &Canvas, wk: Rect) {
        let f = &cv.g.fonts;
        let is_wide = wk.w >= 260.0;
        let ev_col = cv.accent;
        let readable_ev = readable(ev_col, 60.0);

        let now_min = if self.next_min != i64::MIN && self.next_min != 0 {
            self.next_min
        } else {
            online::now_local_minutes()
        };

        if is_wide {
            // 6x2 (size 1): правая колонка
            let col_x = wk.x + WEEK_WIDE_LEFT_W + 1.0;
            let col_w = (wk.right() - col_x).max(20.0);
            let col_r = Rect::new(col_x, wk.y, col_w, wk.h);
            let cx = col_r.x + 12.0;
            let cw = (col_r.w - 22.0).max(10.0);

            if let Some(ref e) = self.next {
                let local = e.local_id.map(|id| (id, e.start.div_euclid(1440)));
                let ev_day = e.start.div_euclid(1440);
                let (_, clicked) = ui.click_area(id(TODAY, 63), col_r);
                let hv = ui.hover_of(id(TODAY, 63));
                if hv > 0.01 {
                    cv.rr(col_r.inset(4.0), 8.0, D2D1_COLOR_F { a: 0.05 * hv, ..fg() });
                    if local.is_some() {
                        cv.glyph(icon::EDIT, Rect::new(col_r.right() - 20.0, col_r.y + 10.0, 14.0, 14.0), false, D2D1_COLOR_F { a: hv, ..dim() });
                    }
                }

                let next_label = crate::i18n::pick("ДАЛЕЕ", "NEXT");
                cv.text(next_label, Rect::new(cx, col_r.y + 14.0, cw, 11.0), &f.caps, readable_ev);

                let clipped = cv.wrap_clip(&e.title, &f.body_b, cw, 2);
                let (lines, _) = cv.wrapped(&clipped, &f.body_b, cw);
                let title_h = if lines > 1 { 28.0 } else { 14.0 };
                cv.text(&clipped, Rect::new(cx, col_r.y + 27.0, cw, title_h), &f.body_b, txt());

                let time_str = event_time_str(e);
                let time_y = col_r.y + 27.0 + title_h + 3.0;
                cv.text(&time_str, Rect::new(cx, time_y, cw, 13.0), &f.small, dim());

                let rel_str = event_relative_time(e, now_min);
                let rel_y = time_y + 14.0;
                cv.text(&rel_str, Rect::new(cx, rel_y, cw, 13.0), &f.small, readable_ev);

                if clicked {
                    if let Some((lid, day)) = local {
                        self.open_month(day);
                        self.edit_event(ui, lid);
                    } else {
                        self.open_month(ev_day);
                    }
                }
            } else {
                let (tag, l1, l2, btn_lbl) = match self.online.calendar_set {
                    true if self.online.calendar_failed => (
                        crate::i18n::pick("ОШИБКА", "ERROR"),
                        crate::i18n::pick("Сбой загрузки", "Load error"),
                        crate::i18n::pick("Проверьте подключения", "Check connections"),
                        Some(crate::i18n::pick("Проверить", "Check")),
                    ),
                    true => (
                        crate::i18n::pick("КАЛЕНДАРЬ", "CALENDAR"),
                        crate::i18n::pick("Событий нет", "No events"),
                        crate::i18n::pick("Ближайшее время свободно", "No upcoming events"),
                        None,
                    ),
                    false => (
                        crate::i18n::pick("КАЛЕНДАРЬ", "CALENDAR"),
                        crate::i18n::pick("Не подключён", "Not connected"),
                        crate::i18n::pick("Google, Apple, iCal", "Google, Apple, iCal"),
                        Some(crate::i18n::pick("Подключить", "Connect")),
                    ),
                };
                cv.text(tag, Rect::new(cx, col_r.y + 13.0, cw, 11.0), &f.caps, dim());
                cv.text(l1, Rect::new(cx, col_r.y + 27.0, cw, 13.0), &f.small_b, txt());
                cv.text(l2, Rect::new(cx, col_r.y + 42.0, cw, 13.0), &f.small, dim());
                if let Some(lbl) = btn_lbl {
                    let btn_r = Rect::new(cx, col_r.bottom() - 30.0, cw.min(96.0), 20.0);
                    if compact_pill_btn(ui, cv, id(TODAY, 95), btn_r, lbl, !self.online.calendar_failed) {
                        self.begin_calendar_edit(ui);
                    }
                }
            }
        } else {
            // 4x2 (size 0): блок снизу под рядом дней
            let ev_y = wk.bottom() - 41.0;
            let ev = Rect::new(wk.x + 8.0, ev_y, wk.w - 16.0, 33.0);
            cv.rr(ev, 8.0, inset());

            if let Some(ref e) = self.next {
                cv.rr(Rect::new(ev.x + 7.0, ev.y + 7.0, EVENT_STRIP_W, ev.h - 14.0), 1.5, ev_col);

                let local = e.local_id.map(|id| (id, e.start.div_euclid(1440)));
                let ev_day = e.start.div_euclid(1440);
                let (_, clicked) = ui.click_area(id(TODAY, 63), ev);
                let hv = ui.hover_of(id(TODAY, 63));
                if hv > 0.01 {
                    cv.rr(ev, 8.0, D2D1_COLOR_F { a: 0.05 * hv, ..fg() });
                    if local.is_some() {
                        cv.glyph(icon::EDIT, Rect::new(ev.right() - 20.0, ev.y + 9.0, 14.0, 14.0), false, D2D1_COLOR_F { a: hv, ..dim() });
                    }
                } else if self.online.calendar_failed {
                    cv.circle(ev.right() - 14.0, ev.y + 17.0, 2.5, status_pink());
                }

                let title_x = ev.x + 15.0;
                let title_w = (ev.w - 23.0).max(10.0);
                cv.text(&e.title, Rect::new(title_x, ev.y + 4.0, title_w, 13.0), &f.small_b, txt());

                let time_str = event_time_str(e);
                let rel_str = event_relative_time(e, now_min);
                let rel_w = cv.text_w(&rel_str, &f.small_r);
                let max_time_w = (ev.w - 23.0 - rel_w - 6.0).max(30.0);
                cv.text(&time_str, Rect::new(title_x, ev.y + 18.0, max_time_w, 13.0), &f.small, dim());
                cv.text(&rel_str, Rect::new(ev.right() - 8.0 - rel_w, ev.y + 18.0, rel_w, 13.0), &f.small_r, readable_ev);

                if clicked {
                    if let Some((lid, day)) = local {
                        self.open_month(day);
                        self.edit_event(ui, lid);
                    } else {
                        self.open_month(ev_day);
                    }
                }
            } else if !self.online.calendar_set || self.online.calendar_failed {
                let (l1, l2, btn_lbl, accent_btn) = if self.online.calendar_failed {
                    (
                        crate::i18n::pick("Ошибка загрузки", "Load error"),
                        crate::i18n::pick("Проверьте доступ", "Check access"),
                        crate::i18n::pick("Открыть", "Open"),
                        false,
                    )
                } else {
                    (
                        crate::i18n::pick("Нет подключений", "No calendars"),
                        crate::i18n::pick("Google, Apple, iCal", "Google, Apple, iCal"),
                        crate::i18n::pick("Подключить", "Connect"),
                        true,
                    )
                };
                let bw = (cv.text_w(btn_lbl, &f.small_c) + 16.0).clamp(58.0, 82.0);
                let btn_r = Rect::new(ev.right() - 6.0 - bw, ev.cy() - 10.0, bw, 20.0);
                let text_w = (btn_r.x - 6.0 - (ev.x + 10.0)).max(20.0);
                cv.text(l1, Rect::new(ev.x + 10.0, ev.y + 4.0, text_w, 13.0), &f.small_b, txt());
                cv.text(l2, Rect::new(ev.x + 10.0, ev.y + 18.0, text_w, 13.0), &f.small, dim());
                if compact_pill_btn(ui, cv, id(TODAY, 95), btn_r, btn_lbl, accent_btn) {
                    self.begin_calendar_edit(ui);
                }
            } else {
                cv.rr(Rect::new(ev.x + 7.0, ev.y + 7.0, EVENT_STRIP_W, ev.h - 14.0), 1.5, dim());
                cv.text(
                    crate::i18n::pick("Событий нет", "No events"),
                    Rect::new(ev.x + 15.0, ev.y + 4.0, ev.w - 23.0, 13.0),
                    &f.small_b,
                    txt(),
                );
                cv.text(
                    crate::i18n::pick("Ближайшее время свободно", "No upcoming events"),
                    Rect::new(ev.x + 15.0, ev.y + 18.0, ev.w - 23.0, 13.0),
                    &f.small,
                    dim(),
                );
            }
        }
    }

    /// Расширенная форма: расписание событий на сегодня (4x4).
    /// Возвращает `true`, если список успешно отрисован, или `false`, если места не хватило
    /// и карточка должна деградировать к одному событию.
    unsafe fn draw_expanded_events(&mut self, ui: &mut Ui, cv: &Canvas, box_r: Rect, today: i64) -> bool {
        let f = &cv.g.fonts;
        let today_days = online::events_by_day(&self.online.cal, today, 1, online::utc_offset_minutes());
        let raw_events = today_days.into_iter().next().unwrap_or_default();
        let now_min = if self.next_min != i64::MIN && self.next_min != 0 {
            self.next_min
        } else {
            online::now_local_minutes()
        };
        let today_minute = now_min.rem_euclid(1440);
        let items = select_today_events(&raw_events, today_minute);

        // Заголовок «СЕГОДНЯ · N СОБЫТИЙ»
        let head_text = format_today_events_header(items.len());
        cv.text(&head_text, Rect::new(box_r.x + 12.0, box_r.y + 7.0, box_r.w - 24.0, 11.0), &f.caps, dim());

        if self.online.calendar_failed {
            cv.circle(box_r.right() - 14.0, box_r.y + 12.0, 2.5, status_pink());
        }

        let content_top = box_r.y + 22.0;
        let content_h = box_r.bottom() - 6.0 - content_top;

        if items.is_empty() {
            if let Some(ref e) = self.next {
                cv.text(
                    crate::i18n::pick("На сегодня событий нет", "No events for today"),
                    Rect::new(box_r.x + 12.0, content_top, box_r.w - 24.0, 14.0),
                    &f.small,
                    dim(),
                );
                let next_y = content_top + 18.0;
                let next_h = (box_r.bottom() - 8.0 - next_y).min(46.0);
                if next_h >= 34.0 {
                    let next_r = Rect::new(box_r.x + 8.0, next_y, box_r.w - 16.0, next_h);
                    cv.rr(next_r, 8.0, inset());
                    cv.rr(Rect::new(next_r.x + 7.0, next_r.y + 7.0, EVENT_STRIP_W, next_r.h - 14.0), 1.5, cv.accent);
                    let local = e.local_id.map(|id| (id, e.start.div_euclid(1440)));
                    let ev_day = e.start.div_euclid(1440);
                    let (_, clicked) = ui.click_area(id(TODAY, 63), next_r);
                    let hv = ui.hover_of(id(TODAY, 63));
                    if hv > 0.01 {
                        cv.rr(next_r, 8.0, D2D1_COLOR_F { a: 0.05 * hv, ..fg() });
                    }
                    let l1 = event_when(e, now_min);
                    cv.text(&l1, Rect::new(next_r.x + 15.0, next_r.y + 5.0, next_r.w - 22.0, 13.0), &f.small_b, txt());
                    cv.text(&e.title, Rect::new(next_r.x + 15.0, next_r.y + 20.0, next_r.w - 22.0, 13.0), &f.small, dim());
                    if clicked {
                        if let Some((lid, day)) = local {
                            self.open_month(day);
                            self.edit_event(ui, lid);
                        } else {
                            self.open_month(ev_day);
                        }
                    }
                }
            } else {
                let card_h = (content_h - 4.0).min(88.0);
                let empty_r = Rect::new(box_r.x + 8.0, content_top + 2.0, box_r.w - 16.0, card_h);
                cv.rr(empty_r, 8.0, inset());
                let (title, sub) = if !self.online.calendar_set {
                    (
                        crate::i18n::pick("Календарь не подключён", "Calendar not connected"),
                        crate::i18n::pick("Google, Apple iCloud или iCal", "Google, Apple iCloud or iCal"),
                    )
                } else if self.online.calendar_failed {
                    (
                        crate::i18n::pick("Не удалось загрузить", "Couldn't load"),
                        crate::i18n::pick("Проверьте аккаунты и ссылки", "Check accounts and links"),
                    )
                } else {
                    (
                        crate::i18n::pick("На сегодня событий нет", "No events for today"),
                        crate::i18n::pick("Ближайшее время свободно", "No upcoming events"),
                    )
                };
                cv.text(title, Rect::new(empty_r.x + 10.0, empty_r.y + 10.0, empty_r.w - 20.0, 14.0), &f.small_b, txt());
                cv.text(sub, Rect::new(empty_r.x + 10.0, empty_r.y + 26.0, empty_r.w - 20.0, 14.0), &f.small, dim());

                let btn_y = empty_r.bottom() - 28.0;
                let mut bx = empty_r.x + 10.0;
                if !self.online.calendar_set || self.online.calendar_failed {
                    let lbl = if self.online.calendar_failed {
                        crate::i18n::pick("Проверить", "Check")
                    } else {
                        crate::i18n::pick("Подключить", "Connect")
                    };
                    let bw = (cv.text_w(lbl, &f.small_c) + 18.0).clamp(68.0, 96.0);
                    if compact_pill_btn(ui, cv, id(TODAY, 95), Rect::new(bx, btn_y, bw, 20.0), lbl, !self.online.calendar_failed) {
                        self.begin_calendar_edit(ui);
                    }
                    bx += bw + 6.0;
                }
                let add_lbl = crate::i18n::pick("+ Событие", "+ Event");
                let aw = (cv.text_w(add_lbl, &f.small_c) + 16.0).clamp(62.0, 88.0);
                if bx + aw <= empty_r.right() - 8.0
                    && compact_pill_btn(ui, cv, id(TODAY, CAL_EMPTY_ADD_BTN), Rect::new(bx, btn_y, aw, 20.0), add_lbl, false)
                {
                    self.open_month(today);
                    self.begin_event(ui, today);
                }
            }
            return true;
        }

        let layout = calculate_event_list_layout(items.len(), content_h, SCHEDULE_ROW_H, SCHEDULE_MORE_H);
        let Some(layout) = layout else {
            return false;
        };

        // Определение позиции отметки «сейчас» между прошедшими и будущими событиями
        let timed: Vec<&TodayEventItem> = items.iter().filter(|it| it.minute.is_some()).collect();
        let now_marker_split = if let Some(first_start) = timed.first().and_then(|it| it.minute) {
            let last_end = timed.iter().filter_map(|it| it.end_minute.or(it.minute)).max().unwrap_or(first_start);
            if today_minute >= first_start && today_minute <= last_end {
                items.iter().enumerate().position(|(idx, it)| {
                    idx > 0 && it.status == EventStatus::Future && items[idx - 1].status == EventStatus::Past
                })
            } else {
                None
            }
        } else {
            None
        };

        let mut edit_id = None;
        for (i, item) in items[..layout.visible_count].iter().enumerate() {
            let ry = content_top + i as f32 * SCHEDULE_ROW_H;
            let row_r = Rect::new(box_r.x + 6.0, ry, box_r.w - 12.0, SCHEDULE_ROW_H);

            let is_past = item.status == EventStatus::Past;
            let col = event_color(i, item.highlighted, cv.accent);
            let strip_col = if is_past { D2D1_COLOR_F { a: 0.35, ..col } } else { col };

            // Время начала слева (f.small, tabular, dim())
            let time_str = match item.minute {
                Some(m) => format!("{:02}:{:02}", m / 60, m % 60),
                None => String::new(),
            };
            let time_col = if is_past { mix(dim(), crate::ui::card(), 0.3) } else { dim() };
            cv.text(&time_str, Rect::new(row_r.x + 2.0, ry + 2.0, SCHEDULE_TIME_W, 13.0), &f.small_r, time_col);

            // Цветная вертикальная полоска
            let strip_x = row_r.x + 2.0 + SCHEDULE_TIME_W + 5.0;
            cv.rr(Rect::new(strip_x, ry + 3.0, EVENT_STRIP_W, SCHEDULE_ROW_H - 6.0), 1.5, strip_col);

            // Название события
            let title_x = strip_x + EVENT_STRIP_W + 6.0;
            let title_w = (row_r.right() - title_x - 4.0).max(10.0);
            let title_col = if is_past { mix(dim(), crate::ui::card(), 0.25) } else { txt() };
            cv.text(&item.title, Rect::new(title_x, ry + 1.0, title_w, 13.0), &f.small_b, title_col);

            // Длительность или «через N мин»
            if item.highlighted {
                let rel = today_event_relative_time(item.minute, item.end_minute, today_minute);
                let rel_col = readable(col, 60.0);
                cv.text(&rel, Rect::new(title_x, ry + 14.0, title_w, 12.0), &f.small, rel_col);
            } else {
                let dur = format_event_duration(item.minute, item.end_minute);
                let dur_col = if is_past { mix(dim(), crate::ui::card(), 0.35) } else { dim() };
                cv.text(&dur, Rect::new(title_x, ry + 14.0, title_w, 12.0), &f.small, dur_col);
            }

            // Клик по строке локального события
            if let Some(lid) = item.local_id.filter(|_| i < ROW_IDS) {
                let rid = id(TODAY, WEEK_ROW + i as u32);
                if ui.click_area(rid, row_r).1 {
                    edit_id = Some(lid);
                }
                let hv = ui.hover_of(rid);
                if hv > 0.01 {
                    cv.rr(row_r, 4.0, D2D1_COLOR_F { a: 0.06 * hv, ..fg() });
                    cv.glyph(icon::EDIT, Rect::new(row_r.right() - 16.0, ry + 7.0, 14.0, 14.0), false, D2D1_COLOR_F { a: hv, ..dim() });
                }
            }
        }

        // Горизонтальная линия «сейчас» с точкой слева — цветом темы
        if let Some(split_idx) = now_marker_split {
            if split_idx <= layout.visible_count {
                let line_y = content_top + split_idx as f32 * SCHEDULE_ROW_H;
                let circle_x = box_r.x + 12.0;
                cv.circle(circle_x, line_y, NOW_MARKER_R, cv.accent);
                let line_left = circle_x + NOW_MARKER_R + 2.0;
                let line_w = (box_r.right() - 10.0 - line_left).max(10.0);
                cv.rr(Rect::new(line_left, line_y - 0.5, line_w, 1.0), 0.5, cv.accent);
            }
        }

        if let Some(lid) = edit_id {
            self.open_month(today);
            self.edit_event(ui, lid);
        }

        if layout.more_count > 0 {
            let my = content_top + layout.visible_count as f32 * SCHEDULE_ROW_H;
            let more_r = Rect::new(box_r.x + 6.0, my, box_r.w - 12.0, SCHEDULE_MORE_H);
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
        self.ev_form = None;
        self.cal_editing = true;
        self.cal_bad = false;
        self.apple_form_open = false;
        // Ошибку входа не сбрасываем: форму открывают заново как раз после возврата из браузера,
        // и пользователь должен увидеть, почему подключение не удалось.
        ui.set_field_text(field, "");
        ui.set_secret(id(TODAY, CAL_APPLE_PW_FIELD), true);
        ui.blur();
    }

    /// Форма управления источниками календаря в оверлее 216×216:
    /// Google OAuth 2.0, Apple iCloud CalDAV и ссылки iCal.
    pub(super) unsafe fn calendar_form(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect) {
        let f = &cv.g.fonts;
        cv.card(r);
        cv.glow(r, crate::ui::card_radius(), cv.accent, Corner::TopRight);

        let field = id(TODAY, 62);
        let google_btn = id(TODAY, CAL_GOOGLE_BTN);
        let apple_toggle = id(TODAY, CAL_APPLE_TOGGLE_BTN);
        let apple_id_field = id(TODAY, CAL_APPLE_ID_FIELD);
        let apple_pw_field = id(TODAY, CAL_APPLE_PW_FIELD);
        let apple_submit = id(TODAY, CAL_APPLE_SUBMIT_BTN);
        let apple_cancel = id(TODAY, CAL_APPLE_CANCEL_BTN);
        let ics_add_btn = id(TODAY, CAL_ICS_ADD_BTN);
        ui.set_secret(apple_pw_field, true);

        // Клик вне оверлея закрывает форму подключений
        let ptr = ui.pointer();
        if self.cal_editing && ui.pressed && !r.contains(ptr) {
            self.cal_editing = false;
            self.apple_form_open = false;
            online::clear_cal_auth_error();
            ui.blur();
            return;
        }

        // Шапка оверлея
        let mut title_x = r.x + 10.0;
        if self.month_open {
            let back_r = Rect::new(r.x + 8.0, r.y + 7.0, 18.0, 18.0);
            if ui.icon_button(cv, id(TODAY, CAL_BACK_BTN), back_r, icon::LEFT, false, crate::i18n::pick("Назад к месяцу", "Back to month")) {
                self.cal_editing = false;
                self.apple_form_open = false;
                online::clear_cal_auth_error();
                ui.blur();
                return;
            }
            title_x = back_r.right() + 4.0;
        }
        cv.text(
            crate::i18n::pick("Календари", "Calendars"),
            Rect::new(title_x, r.y + 7.0, r.right() - 32.0 - title_x, 18.0),
            &f.body_b,
            txt(),
        );
        let close_btn = Rect::new(r.right() - 26.0, r.y + 7.0, 18.0, 18.0);
        if ui.icon_button(cv, id(TODAY, 64), close_btn, icon::CLOSE, false, crate::i18n::pick("Закрыть", "Close")) {
            self.cal_editing = false;
            self.month_open = false;
            self.apple_form_open = false;
            online::clear_cal_auth_error();
            ui.blur();
            return;
        }
        cv.rr(Rect::new(r.x + 10.0, r.y + 30.0, r.w - 20.0, 1.0), 0.5, hairline());

        // Обработка ввода новой ссылки iCal (Enter или кнопка «+»)
        let mut submitted_url = ui.take_submitted(field);

        let status = online::cal_account_status();
        if status.apple_id.is_some() {
            self.apple_form_open = false;
        }

        let err_msg = if self.cal_bad {
            Some(crate::i18n::pick(
                "Нужна ссылка вида https:// или webcal://",
                "A link like https:// or webcal:// is required",
            ).to_string())
        } else {
            status.error.as_ref().map(|e| crate::i18n::tr(e).into_owned())
        };

        let view_top = r.y + 35.0;
        let view = Rect::new(r.x + 8.0, view_top, r.w - 16.0, (r.bottom() - 8.0 - view_top).max(40.0));
        let row_w = view.w - 4.0;

        let err_h = if let Some(ref msg) = err_msg {
            let (lines, lh) = cv.wrapped(msg, &f.small, row_w - 4.0);
            (lines.min(2) as f32) * lh + 6.0
        } else {
            0.0
        };
        let apple_card_h = if self.apple_form_open { 106.0 } else { 36.0 };
        let ics_rows_h = self.ics.len() as f32 * 22.0;
        let total_h = 36.0 + 6.0 + apple_card_h + 10.0 + 14.0 + 24.0 + err_h + 4.0 + ics_rows_h;

        let sid = id(TODAY, 65);
        let off = ui.scroll_begin(cv, sid, view, total_h);
        let mut y = view.y - off;
        let btn_w = 68.0;
        let on_accent = crate::ui::on_color(cv.accent);

        // 1. Карточка Google Календаря
        {
            let card_r = Rect::new(view.x, y, row_w, 36.0);
            cv.rr(card_r, 8.0, inset());
            let badge_r = Rect::new(card_r.x + 7.0, card_r.cy() - 10.0, 20.0, 20.0);
            let g_col = status_blue();
            cv.rr(badge_r, 6.0, D2D1_COLOR_F { a: 0.18, ..g_col });
            cv.text("G", badge_r, &f.small_c, g_col);

            let tx = badge_r.right() + 7.0;
            let btn_r = Rect::new(card_r.right() - 7.0 - btn_w, card_r.cy() - 10.0, btn_w, 20.0);
            let tw = (btn_r.x - 6.0 - tx).max(20.0);
            cv.text(crate::i18n::pick("Google Календарь", "Google Calendar"), Rect::new(tx, card_r.y + 4.0, tw, 13.0), &f.small_b, txt());
            let (sub, sub_col) = if let Some(email) = &status.google_email {
                (email.as_str(), status_green())
            } else if status.google_busy {
                (crate::i18n::pick("Вход в браузере…", "Browser sign-in…"), readable(cv.accent, 60.0))
            } else {
                (crate::i18n::pick("Вход через браузер", "Browser sign-in"), dim())
            };
            cv.text(sub, Rect::new(tx, card_r.y + 18.0, tw, 13.0), &f.small, sub_col);

            if status.google_email.is_some() {
                if ui.solid_button(cv, google_btn, btn_r, crate::i18n::pick("Отключить", "Disconnect"), crate::ui::chip_bg(), txt(), 10.0) {
                    online::disconnect_google(&self.ics);
                    self.sync();
                }
            } else if status.google_busy {
                let _ = ui.solid_button(cv, google_btn, btn_r, crate::i18n::pick("Ожидание…", "Waiting…"), crate::ui::chip_bg(), dim(), 10.0);
            } else if online::oauth::google_available() {
                if ui.solid_button(cv, google_btn, btn_r, crate::i18n::pick("Подключить", "Connect"), cv.accent, on_accent, 10.0) {
                    online::connect_google(self.ics.clone());
                }
            } else {
                let _ = ui.solid_button(cv, google_btn, btn_r, crate::i18n::pick("Нет ключей", "No keys"), crate::ui::chip_bg(), dim(), 10.0);
            }
            y += 42.0;
        }

        // 2. Карточка Apple iCloud Календаря
        {
            let card_r = Rect::new(view.x, y, row_w, apple_card_h);
            cv.rr(card_r, 8.0, inset());
            let badge_r = Rect::new(card_r.x + 7.0, card_r.y + 8.0, 20.0, 20.0);
            let a_col = status_orange();
            cv.rr(badge_r, 6.0, D2D1_COLOR_F { a: 0.18, ..a_col });
            cv.symbol(icon::CLOUD, badge_r, 11.0, a_col);

            let tx = badge_r.right() + 7.0;
            let btn_r = Rect::new(card_r.right() - 7.0 - btn_w, card_r.y + 8.0, btn_w, 20.0);
            let tw = (btn_r.x - 6.0 - tx).max(20.0);
            cv.text(crate::i18n::pick("Apple iCloud", "Apple iCloud"), Rect::new(tx, card_r.y + 4.0, tw, 13.0), &f.small_b, txt());
            let (sub, sub_col) = if let Some(apple_id) = &status.apple_id {
                (apple_id.as_str(), status_green())
            } else {
                (crate::i18n::pick("Apple ID и пароль прил.", "Apple ID & app password"), dim())
            };
            cv.text(sub, Rect::new(tx, card_r.y + 18.0, tw, 13.0), &f.small, sub_col);

            if status.apple_id.is_some() {
                if ui.solid_button(cv, apple_toggle, btn_r, crate::i18n::pick("Отключить", "Disconnect"), crate::ui::chip_bg(), txt(), 10.0) {
                    online::disconnect_apple(&self.ics);
                    self.sync();
                }
            } else if !self.apple_form_open {
                if ui.solid_button(cv, apple_toggle, btn_r, crate::i18n::pick("Подключить", "Connect"), cv.accent, on_accent, 10.0) {
                    self.apple_form_open = true;
                    online::clear_cal_auth_error();
                    ui.focus = Some(apple_id_field);
                    ui.caret_on = true;
                }
            } else {
                if ui.solid_button(cv, apple_cancel, btn_r, crate::i18n::pick("Отмена", "Cancel"), crate::ui::chip_bg(), txt(), 10.0) {
                    self.apple_form_open = false;
                    ui.set_field_text(apple_id_field, "");
                    ui.set_field_text(apple_pw_field, "");
                    ui.blur();
                }
                let fw = row_w - 14.0;
                let fx = card_r.x + 7.0;
                ui.set_hint_marquee(apple_id_field, true);
                ui.set_hint_marquee(apple_pw_field, true);
                ui.text_field_at(
                    cv,
                    apple_id_field,
                    Rect::new(fx, card_r.y + 36.0, fw, 21.0),
                    crate::i18n::pick("Apple ID (email@icloud.com)", "Apple ID (email@icloud.com)"),
                    "",
                    Submit::Keep,
                    8.0,
                    8.0,
                );
                ui.text_field_at(
                    cv,
                    apple_pw_field,
                    Rect::new(fx, card_r.y + 60.0, fw, 21.0),
                    crate::i18n::pick("Пароль приложения", "App-specific password"),
                    "",
                    Submit::Keep,
                    8.0,
                    8.0,
                );
                let a_id = ui.field_text(apple_id_field);
                let a_pw = ui.field_text(apple_pw_field);
                let can_apple = !status.apple_busy && !a_id.trim().is_empty() && !a_pw.trim().is_empty();
                let (abg, afg) = if can_apple { (cv.accent, on_accent) } else { (crate::ui::chip_bg(), dim()) };
                let alabel = if status.apple_busy {
                    crate::i18n::pick("Проверка…", "Checking…")
                } else {
                    crate::i18n::pick("Войти", "Sign in")
                };
                let sub_r = Rect::new(card_r.right() - 7.0 - btn_w, card_r.y + 83.0, btn_w, 18.0);
                cv.text(
                    "account.apple.com",
                    Rect::new(fx, card_r.y + 85.0, (sub_r.x - 6.0 - fx).max(20.0), 13.0),
                    &f.small,
                    dim(),
                );
                let mut submit_apple = ui.take_submitted(apple_pw_field).is_some() || ui.take_submitted(apple_id_field).is_some();
                if ui.solid_button(cv, apple_submit, sub_r, alabel, abg, afg, 9.0) {
                    submit_apple = true;
                }
                if submit_apple && can_apple {
                    online::connect_apple(a_id, a_pw, self.ics.clone());
                    ui.set_field_text(apple_pw_field, "");
                }
            }
            y += apple_card_h + 10.0;
        }

        // 3. Поле добавления ссылки iCal
        cv.text(
            crate::i18n::pick("ПО ССЫЛКЕ (ICAL)", "BY LINK (ICAL)"),
            Rect::new(view.x + 2.0, y, row_w - 4.0, 11.0),
            &f.caps,
            dim(),
        );
        y += 14.0;

        let at_limit = self.ics.len() >= crate::config::MAX_CALENDAR_URLS;
        let placeholder = if at_limit {
            crate::i18n::pick("Лимит: макс. 8 ссылок", "Limit: max 8 links")
        } else {
            crate::i18n::pick("https://… или webcal://…", "https://… or webcal://…")
        };
        let field_r = Rect::new(view.x, y, row_w, 22.0);
        let add_btn_r = Rect::new(field_r.right() - 20.0, field_r.y + 2.0, 18.0, 18.0);
        let add_clicked = ui.click_area(ics_add_btn, add_btn_r).1;
        if ui.is_dragging(ics_add_btn) || add_clicked {
            ui.claim_focus(field);
        }
        ui.text_field_at(cv, field, field_r, placeholder, icon::LINK, Submit::Keep, 28.0, 24.0);
        let has_url_text = !ui.field_text(field).trim().is_empty() && !at_limit;
        let add_hv = ui.hover_of(ics_add_btn);
        let (btn_bg, btn_fg) = if has_url_text {
            (D2D1_COLOR_F { a: 0.20 + 0.80 * add_hv, ..cv.accent }, mix(cv.accent, on_accent, add_hv))
        } else {
            (D2D1_COLOR_F { a: 0.08 * add_hv, ..fg() }, mix(dim(), txt(), add_hv))
        };
        cv.circle(add_btn_r.cx(), add_btn_r.cy(), 8.5, btn_bg);
        cv.glyph(icon::ADD, add_btn_r.inset(2.0), false, btn_fg);
        ui.tip(ics_add_btn, add_btn_r, crate::i18n::pick("Добавить ссылку", "Add link"));
        if add_clicked && has_url_text {
            submitted_url = ui.submit_field(field);
        }
        y += 24.0;

        // 4. Сообщение об ошибке
        if let Some(ref msg) = err_msg {
            cv.text(msg, Rect::new(view.x + 2.0, y + 2.0, row_w - 4.0, err_h - 2.0), &f.wrap_l, status_pink());
            y += err_h;
        }
        y += 4.0;

        // 5. Список добавленных ссылок в сокращённом виде
        const ROW_H: f32 = 22.0;
        let mut remove_idx = None;
        for (i, url) in self.ics.iter().enumerate() {
            let ry = y + i as f32 * ROW_H;
            if ry + ROW_H < view.y || ry > view.bottom() {
                continue;
            }
            let row = Rect::new(view.x, ry, row_w, ROW_H - 2.0);
            let row_id = id(TODAY, URL_ROW + i as u32);
            let hv = ui.hover_of(row_id);
            cv.rr(row, 6.0, mix(inset(), fg(), 0.04 * hv));
            cv.glyph(icon::LINK, Rect::new(row.x + 5.0, row.y + 4.0, 12.0, 12.0), false, dim());
            let abbrev = abbreviate_url(url);
            cv.text(&abbrev, Rect::new(row.x + 21.0, row.y + 2.5, row.w - 42.0, 14.0), &f.small, txt());
            let del_btn = Rect::new(row.right() - 19.0, row.y + 2.0, 16.0, 16.0);
            if ui.icon_button(cv, id(TODAY, URL_DEL + i as u32), del_btn, icon::CLOSE, false, crate::i18n::pick("Удалить подписку", "Remove subscription")) {
                remove_idx = Some(i);
            }
        }
        ui.scroll_end(cv, sid);

        if let Some(url) = submitted_url {
            let url = url.trim().to_string();
            if !url.is_empty() {
                if crate::config::valid_calendar_url(&url) {
                    if !self.ics.iter().any(|u| u == &url) && self.ics.len() < crate::config::MAX_CALENDAR_URLS {
                        self.ics.push(url);
                        crate::config::save_calendar_ics(&self.ics);
                        online::refresh_calendar(&self.ics);
                        self.sync();
                    }
                    self.cal_bad = false;
                    ui.set_field_text(field, "");
                } else {
                    self.cal_bad = true;
                    ui.set_field_text(field, &url);
                    ui.focus = Some(field);
                    ui.caret_on = true;
                }
            }
        }

        if let Some(i) = remove_idx {
            self.ics.remove(i);
            crate::config::save_calendar_ics(&self.ics);
            online::refresh_calendar(&self.ics);
            self.sync();
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

        // Шапка: «< Ноябрь 2026 >» слева, справа «+», подключения и свернуть.
        let close_r = Rect::new(r.right() - 7.0 - 16.0, r.y + 7.0, 16.0, 18.0);
        let conn_r = Rect::new(close_r.x - 3.0 - 18.0, r.y + 7.0, 18.0, 18.0);
        let add_r = Rect::new(conn_r.x - 2.0 - 18.0, r.y + 7.0, 18.0, 18.0);

        let mut shift = 0;
        let prev_r = Rect::new(r.x + 6.0, r.y + 7.0, 16.0, 18.0);
        if ui.icon_button(cv, id(TODAY, 92), prev_r, icon::LEFT, false, crate::i18n::pick("Предыдущий месяц", "Previous month")) {
            shift = -1;
        }
        let name = format!("{} {}", crate::i18n::months_full()[vm as usize - 1], vy);
        let max_title_w = (add_r.x - 4.0 - 16.0 - 2.0 - (prev_r.right() + 2.0)).max(40.0);
        let name_w = (cv.text_w(&name, &f.body).ceil() + 2.0).min(max_title_w);
        let title = Rect::new(prev_r.right() + 2.0, r.y + 7.0, name_w, 18.0);
        let (_, jump) = ui.click_area(id(TODAY, 91), title);
        let hv = ui.hover_of(id(TODAY, 91));
        cv.text(&name, title, &f.body, mix(txt(), cv.accent, hv));

        let next_r = Rect::new(title.right() + 2.0, r.y + 7.0, 16.0, 18.0);
        if ui.icon_button(cv, id(TODAY, 93), next_r, icon::RIGHT, false, crate::i18n::pick("Следующий месяц", "Next month")) {
            shift = 1;
        }

        let add = add_event_button(ui, cv, id(TODAY, 97), add_r);
        if connections_button(
            ui,
            cv,
            id(TODAY, CAL_MONTH_CONN_BTN),
            conn_r,
            self.online.calendar_set,
            self.online.calendar_failed,
        ) {
            self.begin_calendar_edit(ui);
        }
        let close = ui.icon_button(cv, id(TODAY, 94), close_r, icon::UP, false, crate::i18n::pick("Свернуть", "Collapse"));

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
                crate::i18n::pick("Календарь не подключён", "Calendar not connected")
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
            if (!self.online.calendar_set || self.online.calendar_failed) && view.h >= PITCH + 22.0 {
                let lbl = if self.online.calendar_failed {
                    crate::i18n::pick("Проверить", "Check")
                } else {
                    crate::i18n::pick("Подключить", "Connect")
                };
                let bw = (cv.text_w(lbl, &f.small_c) + 18.0).clamp(68.0, 108.0);
                let br = Rect::new(view.x + 4.0, nr.bottom() + 4.0, bw, 19.0);
                if compact_pill_btn(ui, cv, id(TODAY, 95), br, lbl, !self.online.calendar_failed) {
                    self.begin_calendar_edit(ui);
                }
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
            if let Some(lid) = e.local_id.filter(|_| i < ROW_IDS) {
                // Событие острова: клик по строке открывает правку.
                let row = Rect::new(view.x, y, view.w, PITCH);
                let rid = id(TODAY, DAY_ROW + i as u32);
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

    #[test]
    fn test_event_helpers_formatting() {
        let now = days_from_civil(2026, 10, 15) * 1440 + 9 * 60;
        let today = now / 1440;
        let ev = |start: i64, end: i64, all_day| Event { start, end, all_day, title: String::new(), local_id: None };

        // event_time_str
        assert_eq!(event_time_str(&ev(today * 1440 + 600, today * 1440 + 690, false)), "10:00–11:30");
        assert_eq!(event_time_str(&ev(today * 1440 + 600, today * 1440 + 600, false)), "10:00");
        assert_eq!(event_time_str(&ev(today * 1440, today * 1440 + 1440, true)), "весь день");

        // event_relative_time
        // сейчас 9:00 (540). Событие в 9:40 (580) -> через 40 мин
        assert_eq!(event_relative_time(&ev(today * 1440 + 580, today * 1440 + 620, false), now), "через 40 мин");
        // Событие в 10:30 (630) -> через 1 ч 30 мин
        assert_eq!(event_relative_time(&ev(today * 1440 + 630, today * 1440 + 690, false), now), "через 1 ч 30 мин");
        // Событие идёт сейчас 8:30-9:30
        assert_eq!(event_relative_time(&ev(today * 1440 + 510, today * 1440 + 570, false), now), "сейчас");
        // Завтра
        assert_eq!(event_relative_time(&ev((today + 1) * 1440 + 500, (today + 1) * 1440 + 600, false), now), "Завтра");

        // format_event_duration
        assert_eq!(format_event_duration(Some(600), Some(615)), "15 мин");
        assert_eq!(format_event_duration(Some(600), Some(660)), "1 ч");
        assert_eq!(format_event_duration(Some(600), Some(690)), "1 ч 30 мин");
        assert_eq!(format_event_duration(None, None), "весь день");

        // format_today_events_header
        assert_eq!(format_today_events_header(1), "СЕГОДНЯ · 1 СОБЫТИЕ");
        assert_eq!(format_today_events_header(2), "СЕГОДНЯ · 2 СОБЫТИЯ");
        assert_eq!(format_today_events_header(4), "СЕГОДНЯ · 4 СОБЫТИЯ");
        assert_eq!(format_today_events_header(5), "СЕГОДНЯ · 5 СОБЫТИЙ");
        assert_eq!(format_today_events_header(12), "СЕГОДНЯ · 12 СОБЫТИЙ");
        assert_eq!(format_today_events_header(21), "СЕГОДНЯ · 21 СОБЫТИЕ");
    }
}
