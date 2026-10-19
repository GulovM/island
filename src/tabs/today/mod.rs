//! Вкладка Today: часы, погода (Open-Meteo), неделя и ближайшее событие календаря (iCal),
//! таймер (часы/минуты, пресеты, кольцо, сигнал по окончании), список задач.
//! Раскладка снята с макета 1:1 (панель 550×275).

mod alarm;

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::anim::Spring;
use crate::config::Config;
use crate::online::{self, civil_from_days, days_from_civil, weekday, DayEvent, Event, Online};
use crate::ui::{icon, mix, Canvas, Rect, Submit, Ui, chip_bg, dim, GREEN, ink, inset, ORANGE, PINK, txt, track, fg};

use super::{id, read_lines, write_lines, PAD, TODAY, TOP};
use alarm::play_alarm;

const PRESETS: [u32; 5] = [1, 5, 10, 15, 20];
const MAX_TODO: usize = 100;
/// Число циферблатов часов.
const CLOCK_STYLES: usize = 5;
/// Через сколько без действий выключается режим выбора циферблата.
const CLOCK_PICK_TIMEOUT: Duration = Duration::from_secs(5);
const TODO_FILE: &str = "todo.txt";

/// Дни недели с понедельника.
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

/// Доля круга секунд 0..1: на :59 круг замкнут, дальше счёт идёт заново (+1 минута).
fn second_frac(sec: u16) -> f32 {
    (f32::from(sec.min(59)) + 1.0) / 60.0
}

/// Число из текста поля: берутся только цифры, пусто — 0.
fn parse_num(s: &str) -> u32 {
    s.chars().filter_map(|c| c.to_digit(10)).fold(0u32, |a, d| a.saturating_mul(10).saturating_add(d)).min(99_999)
}

/// Ссылка на календарь: пусто (отключить) или http(s):// / webcal://.
fn valid_calendar_url(url: &str) -> bool {
    url.is_empty() || ["https://", "http://", "webcal://"].iter().any(|p| url.starts_with(p))
}

/// События показанного месяца: (месяц, версия календаря, дни сетки).
type MonthCache = ((i32, u32), u64, Vec<Vec<DayEvent>>);

/// Плавность строки задачи: смещение при вставке/удалении соседей, появление и отметка «выполнено».
struct RowAnim {
    pos: Spring,
    fade: Spring,
    done: Spring,
    /// Положение уже выставлено (первая отрисовка ставит строку на место без движения).
    placed: bool,
}

impl RowAnim {
    fn new(done: bool, entering: bool) -> Self {
        let mut fade = Spring::unit(if entering { 0.0 } else { 1.0 }, 220.0, 24.0);
        fade.target = 1.0;
        let mut d = Spring::unit(done as u8 as f32, 260.0, 26.0);
        d.target = done as u8 as f32;
        Self { pos: Spring::new(0.0, 300.0, 28.0), fade, done: d, placed: entering }
    }

    fn settled(&self) -> bool {
        self.pos.settled() && self.fade.settled() && self.done.settled()
    }

    fn step(&mut self, dt: f32) {
        self.pos.step(dt);
        self.fade.step(dt);
        self.done.step(dt);
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
    /// Состояние анимации каждой задачи (та же длина, что у `todo`).
    anim: Vec<RowAnim>,
    /// 0 — неделя и таймер, 1 — месячный календарь (плавная смена).
    month_t: Spring,
    city: String,
    /// Карточка погоды в режиме ввода города.
    editing: bool,
    ics: String,
    /// Карточка недели в режиме ввода ссылки на календарь; `cal_bad` — введён не адрес.
    cal_editing: bool,
    cal_bad: bool,
    online: Online,
    /// Ближайшее событие календаря (пересчитывается раз в минуту).
    next: Option<Event>,
    next_min: i64,
    /// Раскрыт месячный календарь вместо недели и таймера.
    month_open: bool,
    /// Показанный месяц (год, месяц) и выбранный день (дни от 1970-01-01).
    view: (i32, u32),
    sel: i64,
    /// События месяца: (месяц, версия календаря, 42 дня сетки).
    cache: Option<MonthCache>,
    /// Индекс редактируемой задачи: её текст лежит в поле внизу списка.
    edit: Option<usize>,
    /// Циферблат часов (0..`CLOCK_STYLES`), хранится в конфиге.
    clock_style: usize,
    /// Режим выбора циферблата включён (время последнего действия) и время последнего клика по часам.
    clock_pick: Option<Instant>,
    clock_click: Option<Instant>,
    /// Круг секунд циферблатов с секундами.
    sec_ring: Spring,
    /// Какое поле времени (0 — часы, 1 — минуты, 2 — секунды) было в фокусе в прошлом кадре.
    time_focus: Option<usize>,
    seconds: u32,
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
            anim: Vec::new(),
            month_t: Spring::unit(0.0, 240.0, 26.0),
            city: String::new(),
            editing: false,
            ics: String::new(),
            cal_editing: false,
            cal_bad: false,
            online: Online::default(),
            next: None,
            next_min: i64::MIN,
            month_open: false,
            view: (1970, 1),
            sel: 0,
            cache: None,
            edit: None,
            clock_style: 0,
            clock_pick: None,
            clock_click: None,
            sec_ring: Spring::unit(0.0, 120.0, 20.0),
            time_focus: None,
            seconds: 0,
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
        self.anim = self.todo.iter().map(|(d, _)| RowAnim::new(*d, false)).collect();
    }

    pub fn configure(&mut self, cfg: &Config) {
        self.city = cfg.weather_city.clone();
        self.ics = cfg.calendar_ics.clone();
        self.clock_style = cfg.clock_style.min(CLOCK_STYLES - 1);
    }

    fn save(&self) {
        let lines: Vec<String> = self.todo.iter().map(|(d, t)| format!("{}|{t}", if *d { 1 } else { 0 })).collect();
        write_lines(TODO_FILE, &lines);
    }

    /// Вкладка открылась: обновить погоду и календарь, если устарели.
    pub fn show(&mut self) -> bool {
        online::refresh_weather(&self.city);
        online::refresh_calendar_age(&self.ics, Duration::from_secs(60));
        self.month_open = false;
        self.month_t.value = 0.0;
        self.month_t.target = 0.0;
        self.cal_editing = false;
        self.edit = None;
        self.sync()
    }

    pub fn sync(&mut self) -> bool {
        self.online = online::snapshot();
        self.next_min = online::now_local_minutes();
        self.next = online::next_of(&self.online.cal, self.next_min, online::utc_offset_minutes());
        true
    }

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

    pub fn step(&mut self, dt: f32) {
        self.ring.step(dt);
        self.sec_ring.step(dt);
        self.month_t.step(dt);
        for a in &mut self.anim {
            a.step(dt);
        }
    }

    pub fn animating(&self) -> bool {
        !self.ring.settled() || !self.sec_ring.settled() || !self.month_t.settled() || self.anim.iter().any(|a| !a.settled())
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
                play_alarm();
            }
        }
        // Проверки «устарело ли» дешёвые: сеть дёргается раз в 15–20 минут.
        online::refresh_weather(&self.city);
        online::refresh_calendar(&self.ics);
        // «Ближайшее событие» уходит в прошлое вместе со временем.
        if online::now_local_minutes() != self.next_min {
            self.sync();
        }
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

    fn set_preset(&mut self, i: usize) {
        self.preset = Some(i);
        self.set_time(0, PRESETS[i]);
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, _h: f32) {
        let t = GetLocalTime();
        let f = &cv.g.fonts;
        let dow = (t.wDayOfWeek as usize + 6) % 7;

        // Часы: 5 циферблатов. Двойной клик включает режим выбора (подсветка и стрелки по бокам),
        // стрелки и колесо мыши листают по кругу; клик мимо или бездействие выключают режим.
        let c = Rect::new(PAD, TOP, 116.0, 140.0);
        cv.card(c);
        if self.clock_pick.is_some_and(|at| at.elapsed() >= CLOCK_PICK_TIMEOUT) || (ui.pressed && !ui.hovering(c)) {
            self.clock_pick = None;
        }
        let picking = self.clock_pick.is_some();
        let mut step = 0i32;
        let wheel = ui.wheel_in(c);
        if wheel != 0.0 {
            step = if wheel > 0.0 { -1 } else { 1 };
        }
        if picking {
            cv.rr(c, 11.0, D2D1_COLOR_F { a: 0.05, ..cv.accent });
            cv.rr_line(c, 11.0, cv.accent, 1.5);
            let ay = c.cy() - 10.0;
            if ui.icon_button(cv, id(TODAY, 71), Rect::new(c.x + 3.0, ay, 20.0, 20.0), icon::LEFT, false) {
                step = -1;
            }
            if ui.icon_button(cv, id(TODAY, 72), Rect::new(c.right() - 23.0, ay, 20.0, 20.0), icon::RIGHT, false) {
                step = 1;
            }
        }
        // Карточка опрашивается после стрелок: нажатие на стрелку не достаётся ей.
        if ui.click_area(id(TODAY, 70), c).1 {
            let double = self.clock_click.is_some_and(|at| at.elapsed() < Duration::from_millis(400));
            self.clock_click = if double { None } else { Some(Instant::now()) };
            if double {
                self.clock_pick = if picking { None } else { Some(Instant::now()) };
            }
        }
        if step != 0 {
            self.clock_style = (self.clock_style as i32 + step).rem_euclid(CLOCK_STYLES as i32) as usize;
            crate::config::save_num("clock_style", self.clock_style);
            if picking {
                self.clock_pick = Some(Instant::now());
            }
        }
        self.clock_face(cv, c, &t);
        let foot = format!("{} • {} {}", crate::i18n::weekdays()[dow], t.wDay, crate::i18n::months_short()[(t.wMonth as usize).clamp(1, 12) - 1]);
        cv.text(&foot, Rect::new(c.x, c.y + 120.0, c.w, 13.0), &f.small_c, dim());

        // Погода. Клик по карточке — поле для города (пусто — определить по IP).
        let wc = Rect::new(PAD, TOP + 150.0, 116.0, 102.0);
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
                cv.glyph(g, Rect::new(wc.x, wc.y + 15.0, wc.w, 20.0), true, col);
                cv.text(&format!("{}°C", wt.temp.round() as i32), Rect::new(wc.x, wc.y + 40.0, wc.w, 22.0), &f.num_c, txt());
                cv.text(online::weather_text(wt.code), Rect::new(wc.x + 4.0, wc.y + 65.0, wc.w - 8.0, 12.0), &f.small_c, dim());
                cv.text(&wt.city, Rect::new(wc.x + 4.0, wc.y + 79.0, wc.w - 8.0, 12.0), &f.small_c, mix(dim(), crate::ui::card(), 0.35));
            }
            None => {
                cv.glyph(icon::CLOUD, Rect::new(wc.x, wc.y + 15.0, wc.w, 20.0), true, dim());
                cv.text("—", Rect::new(wc.x, wc.y + 40.0, wc.w, 22.0), &f.num_c, dim());
                let note = self.online.weather_note.as_deref().unwrap_or("Загрузка…");
                cv.text(note, Rect::new(wc.x + 4.0, wc.y + 65.0, wc.w - 8.0, 12.0), &f.small_c, dim());
            }
        }

        let today = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        self.month_t.target = if self.month_open { 1.0 } else { 0.0 };
        let mt = self.month_t.value.clamp(0.0, 1.0);
        // Во время смены обе раскладки видны с обратной прозрачностью.
        let faded = |a: f32| Canvas { g: cv.g, alpha: cv.alpha * a, accent: cv.accent };
        if mt > 0.01 {
            self.month_card(ui, &faded(mt), Rect::new(PAD + 126.0, TOP, 230.0, 252.0), today);
        }
        if mt < 0.99 {
            let wcv = faded(1.0 - mt);
            self.week_card(ui, &wcv, Rect::new(PAD + 126.0, TOP, 230.0, 122.0), today, dow);
            self.timer_card(ui, &wcv, Rect::new(PAD + 126.0, TOP + 132.0, 230.0, 120.0));
        }
        let tx = PAD + 126.0 + 230.0 + 10.0;
        self.todo_card(ui, cv, Rect::new(tx, TOP, (w - PAD - tx).max(120.0), 252.0));
    }

    /// Циферблат: 0 — столбик ЧЧ/ММ; 1 — столбик с секундами; 2 — «ЧЧ:ММ»; 3 — «ЧЧ:ММ» с полоской секунд;
    /// 4 — круг секунд (полный оборот — +1 минута) вокруг «ЧЧ:ММ».
    unsafe fn clock_face(&mut self, cv: &Canvas, c: Rect, t: &SYSTEMTIME) {
        let f = &cv.g.fonts;
        let hm = format!("{:02}:{:02}", t.wHour, t.wMinute);
        let (hh, mm) = (format!("{:02}", t.wHour), format!("{:02}", t.wMinute));
        let ss = format!(":{:02}", t.wSecond);
        let frac = second_frac(t.wSecond);
        if self.clock_style >= 3 {
            // Новая минута: круг начинается заново, а не откатывается назад.
            if frac < self.sec_ring.target - 0.5 {
                self.sec_ring.value = 0.0;
            }
            self.sec_ring.target = frac;
        }
        match self.clock_style {
            0 => {
                cv.text(&hh, Rect::new(c.x, c.y + 24.0, c.w, 36.0), &f.big_c, txt());
                cv.text(&mm, Rect::new(c.x, c.y + 66.0, c.w, 36.0), &f.big_c, txt());
            }
            1 => {
                cv.text(&hh, Rect::new(c.x, c.y + 18.0, c.w, 36.0), &f.big_c, txt());
                cv.text(&mm, Rect::new(c.x, c.y + 52.0, c.w, 36.0), &f.big_c, txt());
                cv.text(&ss, Rect::new(c.x, c.y + 94.0, c.w, 18.0), &f.num_c, cv.accent);
            }
            2 => {
                cv.text(&hm, Rect::new(c.x, c.y + 44.0, c.w, 36.0), &f.big_c, txt());
            }
            3 => {
                cv.text(&hm, Rect::new(c.x, c.y + 32.0, c.w, 36.0), &f.big_c, txt());
                let bar = Rect::new(c.x + 18.0, c.y + 78.0, c.w - 36.0, 4.0);
                cv.rr(bar, 2.0, track());
                cv.rr(Rect::new(bar.x, bar.y, (bar.w * self.sec_ring.value.clamp(0.0, 1.0)).max(4.0), bar.h), 2.0, cv.accent);
                cv.text(&ss, Rect::new(c.x, c.y + 90.0, c.w, 18.0), &f.num_c, cv.accent);
            }
            _ => {
                let cy = c.y + 68.0;
                cv.ring(c.cx(), cy, 42.0, 4.0, self.sec_ring.value, cv.accent);
                cv.text(&hm, Rect::new(c.x, cy - 18.0, c.w, 36.0), &f.num_c, txt());
            }
        }
    }

    unsafe fn week_card(&mut self, ui: &mut Ui, cv: &Canvas, wk: Rect, today: i64, dow: usize) {
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
            let cx = wk.x + 23.0 + i as f32 * 30.7;
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

        // Ближайшее событие календаря.
        let ev = Rect::new(wk.x + 8.0, wk.y + 76.0, wk.w - 16.0, 38.0);
        cv.rr(ev, 8.0, inset());
        cv.rr(Rect::new(ev.x + 8.0, ev.y + 8.0, 3.0, 20.0), 1.5, PINK);
        let (l1, l2) = match (&self.next, self.online.calendar_set) {
            (Some(e), _) => (event_when(e, online::now_local_minutes()), e.title.clone()),
            (None, true) if self.online.calendar_failed => ("Не удалось загрузить".to_string(), "Проверьте ссылку calendar_ics".to_string()),
            (None, true) => ("Событий нет".to_string(), "Календарь пуст на ближайшее время".to_string()),
            (None, false) => ("Календарь не подключён".to_string(), "Нажмите, чтобы добавить ссылку".to_string()),
        };
        cv.text(&l1, Rect::new(ev.x + 17.0, ev.y + 5.0, ev.w - 24.0, 13.0), &f.small_b, txt());
        cv.text(&l2, Rect::new(ev.x + 17.0, ev.y + 18.0, ev.w - 24.0, 13.0), &f.small, dim());
        // Клик по блоку — ссылка на календарь (подключить, сменить или убрать).
        let (_, clicked) = ui.click_area(id(TODAY, 63), ev);
        let hv = ui.hover_of(id(TODAY, 63));
        if hv > 0.01 {
            cv.rr(ev, 8.0, D2D1_COLOR_F { a: 0.05 * hv, ..fg() });
            cv.glyph(icon::EDIT, Rect::new(ev.right() - 22.0, ev.y + 10.0, 16.0, 16.0), false, D2D1_COLOR_F { a: hv, ..dim() });
        }
        if clicked {
            self.begin_calendar_edit(ui);
        }
    }

    fn begin_calendar_edit(&mut self, ui: &mut Ui) {
        let field = id(TODAY, 62);
        self.cal_editing = true;
        self.cal_bad = false;
        ui.set_field_text(field, &self.ics);
        ui.focus = Some(field);
        ui.caret_on = true;
    }

    /// Ввод ссылки на календарь (iCal) вместо содержимого карточки недели. Enter — сохранить в
    /// `config.toml` и сразу загрузить; пусто — отключить; Esc или клик мимо — без изменений.
    unsafe fn calendar_form(&mut self, ui: &mut Ui, cv: &Canvas, wk: Rect) {
        let f = &cv.g.fonts;
        let field = id(TODAY, 62);
        if let Some(url) = ui.take_submitted(field) {
            let url = url.trim().to_string();
            if valid_calendar_url(&url) {
                self.ics = url;
                crate::config::save_str("calendar_ics", &self.ics);
                online::refresh_calendar(&self.ics);
                self.sync();
                self.cal_editing = false;
                ui.blur();
                return;
            }
            // Не адрес: остаёмся в поле, чтобы поправить.
            self.cal_bad = true;
            ui.set_field_text(field, &url);
            ui.focus = Some(field);
        } else if !ui.has_focus(field) {
            self.cal_editing = false;
            return;
        }
        cv.text("Ссылка на календарь (iCal)", Rect::new(wk.x + 9.0, wk.y + 9.0, wk.w - 18.0, 14.0), &f.body, txt());
        ui.text_field_ex(cv, field, Rect::new(wk.x + 7.0, wk.y + 30.0, wk.w - 14.0, 28.0), "https://… или webcal://…", icon::LINK, Submit::Keep);
        let (msg, col) = if self.cal_bad {
            ("Нужна ссылка вида https:// или webcal://", PINK)
        } else {
            ("Enter — сохранить, пусто — отключить, Esc — отмена", dim())
        };
        cv.text(msg, Rect::new(wk.x + 9.0, wk.y + 64.0, wk.w - 18.0, 13.0), &f.small, col);
        cv.text(
            "Google/Яндекс/Outlook: «секретный адрес в формате iCal»",
            Rect::new(wk.x + 9.0, wk.y + 80.0, wk.w - 18.0, 13.0),
            &f.small,
            mix(dim(), crate::ui::card(), 0.35),
        );
    }

    /// Месячный календарь: дни с событиями помечены точками, клик по дню показывает его события.
    unsafe fn month_card(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, today: i64) {
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
                "Календарь не подключён — нажмите, чтобы добавить ссылку"
            } else if self.online.calendar_failed {
                "Не удалось загрузить календарь"
            } else if self.online.cal_ver == 0 {
                "Загрузка…"
            } else {
                "Событий нет"
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

    unsafe fn timer_card(&mut self, ui: &mut Ui, cv: &Canvas, tm: Rect) {
        let f = &cv.g.fonts;
        cv.card(tm);
        let rem = self.remaining();
        let frac = if self.total.is_zero() { 0.0 } else { rem.as_secs_f32() / self.total.as_secs_f32() };
        self.ring.target = frac.clamp(0.0, 1.0);
        let (cx, cy) = (tm.x + 40.0, tm.y + 62.0);
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

        // Часы, минуты, секунды: поле ввода; колесо мыши меняет значение.
        let bx = tm.x + 82.0;
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

    unsafe fn todo_card(&mut self, ui: &mut Ui, cv: &Canvas, card: Rect) {
        cv.card(card);
        let f = &cv.g.fonts;
        let done = self.todo.iter().filter(|(d, _)| *d).count();
        cv.text("Задачи", Rect::new(card.x + 11.0, card.y + 12.0, 90.0, 14.0), &f.body, txt());
        // Счётчик «выполнено/всего» в кружке с дугой прогресса.
        let (bx, by) = (card.right() - 21.0, card.y + 18.0);
        cv.circle(bx, by, 9.0, chip_bg());
        if !self.todo.is_empty() {
            cv.ring(bx, by, 8.0, 1.4, done as f32 / self.todo.len() as f32, GREEN);
        }
        cv.text(&format!("{done}/{}", self.todo.len()), Rect::new(bx - 9.0, by - 6.0, 18.0, 12.0), &f.caps_c, txt());

        // Поле «добавить» внизу и золотая кнопка со стрелкой.
        let fid = id(TODAY, 30);
        let field = Rect::new(card.x + 10.0, card.bottom() - 38.0, card.w - 20.0, 28.0);
        let editing = self.edit.is_some_and(|i| i < self.todo.len());
        if !editing {
            self.edit = None;
        }
        let (hint, glyph) = if editing { ("Изменить задачу…", icon::EDIT) } else { ("Добавить задачу…", icon::ADD) };
        // Нажатие достаётся первой зарегистрированной области под курсором, поэтому кнопки поверх
        // поля регистрируются раньше него, а рисуются после.
        let add = Rect::new(field.right() - 22.0, field.y + 5.0, 16.0, 16.0);
        let cancel = Rect::new(add.x - 19.0, add.y, 16.0, 16.0);
        let (_, add_clicked) = ui.click_area(id(TODAY, 31), add);
        let cancel_clicked = editing && ui.click_area(id(TODAY, 32), cancel).1;
        // Справа поверх поля лежат кнопки: текст до них не доходит.
        ui.text_field_pad(cv, fid, field, hint, glyph, Submit::Clear, if editing { 45.0 } else { 28.0 });
        cv.rr_line(field, 8.0, if editing { cv.accent } else { track() }, 1.0);
        let hv = ui.hover_of(id(TODAY, 31));
        cv.rr(add, 4.0, mix(cv.accent, fg(), 0.12 * hv));
        cv.glyph(if editing { icon::CHECK } else { icon::FORWARD }, add, false, ink());
        if editing {
            // Отмена правки: поле очищается, задача остаётся прежней.
            let hv = ui.hover_of(id(TODAY, 32));
            cv.rr(cancel, 8.0, D2D1_COLOR_F { a: 0.10 * hv, ..fg() });
            cv.glyph(icon::CLOSE, cancel, false, mix(dim(), txt(), hv));
        }
        if cancel_clicked {
            let _ = ui.submit_field(fid);
            self.edit = None;
        }
        let mut submitted = ui.take_submitted(fid);
        if add_clicked {
            submitted = ui.submit_field(fid);
        }
        if let Some(text) = submitted {
            let text = text.trim();
            match self.edit.take() {
                // Пустой текст при правке — отмена, задача не пропадает.
                Some(i) if !text.is_empty() && i < self.todo.len() => {
                    self.todo[i].1 = text.to_string();
                    self.save();
                }
                Some(_) => {}
                None if !text.is_empty() && self.todo.len() < MAX_TODO => {
                    self.todo.insert(0, (false, text.to_string()));
                    self.anim.insert(0, RowAnim::new(false, true));
                    self.save();
                }
                None => {}
            }
        }

        const ROW: f32 = 27.0;
        const GAP: f32 = 3.0;
        /// Длинная задача переносится максимум на столько строк, дальше — «…» (целиком — в правке).
        const MAX_LINES: usize = 3;
        let view = Rect::new(card.x + 9.0, card.y + 38.0, card.w - 18.0, (card.h - 38.0 - 48.0).max(ROW));
        let text_w = (view.w - 4.0 - 25.0 - 8.0).max(20.0);
        // Раскладка строк: (смещение, высота, число строк, высота строки текста).
        let mut rows: Vec<(f32, f32, usize, f32)> = Vec::with_capacity(self.todo.len());
        let mut total = 0.0f32;
        for (_, text) in &self.todo {
            let (lines, lh) = cv.wrapped(text, &f.wrap_l, text_w);
            let lines = lines.clamp(1, MAX_LINES);
            let h = (lines as f32 * lh + 11.0).max(ROW);
            rows.push((total, h, lines, lh));
            total += h + GAP;
        }
        self.anim.resize_with(self.todo.len(), || RowAnim::new(false, false));
        for (a, row) in self.anim.iter_mut().zip(&rows) {
            a.pos.target = row.0;
            if !a.placed {
                a.pos.snap();
                a.placed = true;
            }
        }
        let sid = id(TODAY, 100);
        let off = ui.scroll_begin(cv, sid, view, total);
        let mut toggled = None;
        let mut removed = None;
        let mut edit_req = None;
        for (i, (is_done, text)) in self.todo.iter().enumerate() {
            let (_, rh, lines, lh) = rows[i];
            let (fade, dn) = (self.anim[i].fade.value.clamp(0.0, 1.0), self.anim[i].done.value.clamp(0.0, 1.0));
            let rcv = Canvas { g: cv.g, alpha: cv.alpha * fade, accent: cv.accent };
            let cv = &rcv;
            let y = view.y + self.anim[i].pos.value - off;
            if y + rh < view.y || y > view.bottom() {
                continue;
            }
            let r = Rect::new(view.x, y, view.w - 4.0, rh);
            let rid = id(TODAY, 200 + i as u32);
            // Кнопки справа («изменить», «удалить») видны при наведении; их области регистрируются
            // раньше строки, иначе строка заберёт нажатие себе.
            let (eid, did) = (id(TODAY, 600 + i as u32), id(TODAY, 400 + i as u32));
            let del = Rect::new(r.right() - 21.0, r.y + ROW / 2.0 - 9.0, 18.0, 18.0);
            let edit = Rect::new(del.x - 20.0, del.y, 18.0, 18.0);
            let show = ui.hover_of(rid) > 0.3 || ui.hover_of(did) > 0.3 || ui.hover_of(eid) > 0.3;
            let (mut edit_clicked, mut del_clicked) = (false, false);
            if show {
                edit_clicked = ui.click_area(eid, edit).1;
                del_clicked = ui.click_area(did, del).1;
            }
            let (_, clicked) = ui.click_area(rid, r);
            let hv = ui.hover_of(rid);
            cv.rr(r, 8.0, mix(inset(), fg(), 0.035 * hv));
            if self.edit == Some(i) {
                cv.rr_line(r, 8.0, cv.accent, 1.0);
            }
            let (cx, cy) = (r.x + 13.0, r.y + ROW / 2.0);
            // Отметка выполнения проявляется плавно: кольцо гаснет, зелёный круг с галочкой нарастает.
            if dn < 0.99 {
                let ring = D2D1_COLOR_F { a: dim().a * (1.0 - dn), ..dim() };
                cv.rr_line(Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), 6.0, ring, 1.2);
            }
            if dn > 0.01 {
                cv.circle(cx, cy, 6.0 * (0.5 + 0.5 * dn), D2D1_COLOR_F { a: dn, ..GREEN });
                cv.text(icon::CHECK, Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), &f.icon_small, D2D1_COLOR_F { a: ink().a * dn, ..ink() });
            }
            let th = lines as f32 * lh;
            let tr = Rect::new(r.x + 25.0, r.y + (r.h - th) / 2.0, text_w, th + 2.0);
            let shown = cv.wrap_clip(text, &f.wrap_l, text_w, MAX_LINES);
            cv.text_strike(&shown, tr, &f.wrap_l, mix(txt(), dim(), dn), *is_done);
            if show {
                // Кнопки лежат поверх конца первой строки: подложка цвета строки.
                let patch = Rect::new(edit.x - 4.0, edit.y - 1.0, r.right() - edit.x + 2.0, 20.0);
                cv.rr(patch, 8.0, mix(inset(), fg(), 0.035 * hv));
            }
            if clicked {
                toggled = Some(i);
            }
            if show {
                for (bid, rect, glyph, active) in [(eid, edit, icon::EDIT, self.edit == Some(i)), (did, del, icon::DELETE, false)] {
                    let h = ui.hover_of(bid);
                    cv.rr(rect, 9.0, D2D1_COLOR_F { a: 0.10 * h, ..fg() });
                    cv.glyph(glyph, rect, false, if active { cv.accent } else { mix(dim(), txt(), h) });
                }
            }
            if del_clicked {
                removed = Some(i);
            } else if edit_clicked {
                edit_req = Some(i);
            }
        }
        ui.scroll_end(cv, sid);
        if let Some(i) = removed {
            self.todo.remove(i);
            if i < self.anim.len() {
                self.anim.remove(i);
            }
            match self.edit {
                Some(e) if e == i => {
                    let _ = ui.submit_field(fid);
                    self.edit = None;
                }
                Some(e) if e > i => self.edit = Some(e - 1),
                _ => {}
            }
            self.save();
        } else if let Some(i) = edit_req {
            self.edit = Some(i);
            ui.set_field_text(fid, &self.todo[i].1);
            ui.focus = Some(fid);
            ui.caret_on = true;
        } else if let Some(i) = toggled {
            self.todo[i].0 = !self.todo[i].0;
            if let Some(a) = self.anim.get_mut(i) {
                a.done.target = self.todo[i].0 as u8 as f32;
            }
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
    fn calendar_url_validation() {
        assert!(valid_calendar_url(""));
        assert!(valid_calendar_url("https://calendar.google.com/x.ics"));
        assert!(valid_calendar_url("webcal://example.com/a.ics"));
        assert!(!valid_calendar_url("calendar.google.com/x.ics"));
        assert!(!valid_calendar_url("ftp://x"));
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
    fn seconds_circle() {
        assert!((second_frac(0) - 1.0 / 60.0).abs() < 1e-6);
        assert!((second_frac(59) - 1.0).abs() < 1e-6);
        assert!((second_frac(200) - 1.0).abs() < 1e-6);
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
