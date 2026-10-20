//! Вкладка Today: часы, погода (Open-Meteo), неделя и ближайшее событие календаря (iCal),
//! таймер (часы/минуты, пресеты, кольцо, сигнал по окончании), список задач.
//! Раскладка снята с макета 1:1 (панель 550×275).

mod alarm;
mod calendar;
mod clock;
mod timer;
mod todo;
mod weather;

use std::time::{Duration, Instant};

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::anim::Spring;
use crate::config::Config;
use crate::online::{self, days_from_civil, Event, Online};
use crate::ui::{dim, icon, Canvas, Rect, Ui};

use super::grid::{Grid, Item};
use super::layout::{sz, Size, Spec, TabSpec};
use super::{id, PAD, TODAY, TOP};
use alarm::play_alarm;
use calendar::MonthCache;
use todo::RowAnim;

/// Число циферблатов часов.
const CLOCK_STYLES: usize = 5;
/// Через сколько без действий выключается режим выбора циферблата.
const CLOCK_PICK_TIMEOUT: Duration = Duration::from_secs(5);

/// Цвет солнца на карточке погоды.
const SUN_COLOR: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.96, g: 0.75, b: 0.19, a: 1.0 };

// Номера виджетов в `SPEC`.
const CLOCK: usize = 0;
const WEATHER: usize = 1;
const WEEK: usize = 2;
const TIMER: usize = 3;
const TODO: usize = 4;

/// Месячный календарь раскрывается поверх сетки: минимальная ширина и высота.
const MONTH_W: f32 = 230.0;
const MONTH_H: f32 = 252.0;

static CLOCK_SIZES: [Size; 2] = [sz(2, 76.0), sz(2, 140.0)];
static WEATHER_SIZES: [Size; 3] = [sz(2, 62.0), sz(2, 102.0), sz(4, 62.0)];
static WEEK_SIZES: [Size; 4] = [sz(4, 76.0), sz(4, 122.0), sz(6, 122.0), sz(4, 252.0)];
static TIMER_SIZES: [Size; 3] = [sz(4, 64.0), sz(4, 120.0), sz(6, 120.0)];
static TODO_SIZES: [Size; 3] = [sz(4, 122.0), sz(4, 252.0), sz(6, 252.0)];

/// Сетка вкладки: 10 колонок по ~64 px (632 px внутри полей). Часы и погода занимают 2 колонки,
/// неделя, таймер и задачи — по 4: исходные пропорции макета.
pub static SPEC: TabSpec = TabSpec {
    cols: 10,
    specs: &[
        Spec { key: "clock", title: "Часы", sizes: &CLOCK_SIZES, size: 1, col: 0 },
        Spec { key: "weather", title: "Погода", sizes: &WEATHER_SIZES, size: 1, col: 0 },
        Spec { key: "week", title: "Эта неделя", sizes: &WEEK_SIZES, size: 1, col: 2 },
        Spec { key: "timer", title: "Таймер", sizes: &TIMER_SIZES, size: 1, col: 2 },
        Spec { key: "todo", title: "Задачи", sizes: &TODO_SIZES, size: 1, col: 6 },
    ],
};

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
    ics: Vec<String>,
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
    /// Где в прошлом кадре лежала карточка недели: месячный календарь раскрывается на её месте.
    week_rect: Rect,
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
            ics: Vec::new(),
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
            week_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        };
        t.set_preset(1);
        t
    }
}

impl Today {
    pub fn configure(&mut self, cfg: &Config) {
        self.city = cfg.weather_city.clone();
        self.ics = cfg.calendar_ics.clone();
        self.clock_style = cfg.clock_style.min(CLOCK_STYLES - 1);
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

    /// Вкладку начали править: месячный календарь и поля ввода закрываются.
    pub fn hide_overlays(&mut self) {
        self.month_open = false;
        self.month_t.target = 0.0;
        self.month_t.snap();
        self.editing = false;
        self.cal_editing = false;
        self.edit = None;
        self.clock_pick = None;
    }

    pub fn sync(&mut self) -> bool {
        self.online = online::snapshot();
        self.next_min = online::now_local_minutes();
        self.next = online::next_of(&self.online.cal, self.next_min, online::utc_offset_minutes());
        true
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

    /// Тик: вышло время таймера — сигнал и сброс. Всегда просит кадр (часы).
    pub fn poll(&mut self) -> bool {
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            self.deadline = None;
            self.left = self.total;
            self.fired = true;
            if self.sound {
                play_alarm();
            }
            self.save_timer();
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


    /// Рисует вкладку по сетке; месячный календарь раскрывается поверх неё. `h` — высота панели
    /// без полосы правки.
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, grid: &mut Grid, w: f32, h: f32) {
        let area = Rect::new(PAD, TOP, w - 2.0 * PAD, h - TOP - PAD);
        let t = GetLocalTime();
        let dow = (t.wDayOfWeek as usize + 6) % 7;
        let today = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        self.month_t.target = if self.month_open { 1.0 } else { 0.0 };
        // Виджеты под календарём гаснут и не принимают нажатий (иначе они перехватят их у дней месяца).
        let cover = (self.month_t.value > 0.004).then(|| self.month_rect(area));
        grid.draw(ui, cv, area, 0, |ui, cv, it| self.widget(ui, cv, it, cover, &t, today, dow));
        let mt = self.month_t.value.clamp(0.0, 1.0);
        if mt > 0.01 {
            let mut r = self.month_rect(area);
            r.y += (1.0 - mt) * 6.0;
            let mcv = Canvas { g: cv.g, alpha: cv.alpha * mt, accent: cv.accent };
            // Закрывающийся календарь уже не принимает нажатий.
            let prev = if self.month_open { None } else { Some(ui.mute()) };
            self.month_card(ui, &mcv, r, today);
            if let Some(p) = prev {
                ui.unmute(p);
            }
        }
    }

    /// Где раскрывается месячный календарь: на месте карточки недели (она не меньше нужной
    /// ширины), но не выходя за область вкладки.
    fn month_rect(&self, area: Rect) -> Rect {
        let wk = self.week_rect;
        let w = wk.w.max(MONTH_W).min(area.w);
        let h = MONTH_H.min(area.h);
        Rect::new(wk.x.min(area.right() - w).max(area.x), wk.y.min(area.bottom() - h).max(area.y), w, h)
    }

    /// Один виджет сетки; если его закрывает месячный календарь, он гаснет и становится немым.
    #[allow(clippy::too_many_arguments)]
    unsafe fn widget(&mut self, ui: &mut Ui, cv: &Canvas, it: &Item, cover: Option<Rect>, t: &SYSTEMTIME, today: i64, dow: usize) {
        if it.idx == WEEK {
            self.week_rect = it.rect;
        }
        let covered = cover.is_some_and(|c| c.intersects(&it.rect));
        let k = if covered { 1.0 - self.month_t.value.clamp(0.0, 1.0) } else { 1.0 };
        if k < 0.004 {
            return;
        }
        let sub = Canvas { g: cv.g, alpha: cv.alpha * k, accent: cv.accent };
        let prev = if covered { Some(ui.mute()) } else { None };
        let r = it.rect;
        match it.idx {
            CLOCK => self.clock_card(ui, &sub, r, it.size, t, dow),
            WEATHER => self.weather_card(ui, &sub, r, it.size),
            WEEK => self.week_card(ui, &sub, r, it.size, today, dow),
            TIMER => self.timer_card(ui, &sub, r, it.size),
            TODO => self.todo_card(ui, &sub, r),
            _ => {}
        }
        if let Some(p) = prev {
            ui.unmute(p);
        }
    }

    /// Часы: 5 циферблатов. Двойной клик включает режим выбора (подсветка и стрелки по бокам),
    /// стрелки и колесо мыши листают по кругу; клик мимо или бездействие выключают режим.
    /// `size` 0 — компактная карточка (время в одну строку).
    unsafe fn clock_card(&mut self, ui: &mut Ui, cv: &Canvas, c: Rect, size: usize, t: &SYSTEMTIME, dow: usize) {
        let f = &cv.g.fonts;
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
        self.clock_face(cv, c, t, size == 0);
        let foot = format!("{} • {} {}", crate::i18n::weekdays()[dow], t.wDay, crate::i18n::months_short()[(t.wMonth as usize).clamp(1, 12) - 1]);
        cv.text(&foot, Rect::new(c.x, c.bottom() - 20.0, c.w, 13.0), &f.small_c, dim());
    }
}
