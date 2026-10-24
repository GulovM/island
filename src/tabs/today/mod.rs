//! Вкладка Today: часы, погода (Open-Meteo), неделя и ближайшее событие календаря (iCal),
//! таймер (часы/минуты, пресеты, кольцо, сигнал по окончании), список задач.
//! Раскладка снята с макета 1:1 (панель 550×275).

mod alarm;
mod calendar;
mod clock;
mod events;
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
use super::{id, TODAY};
use alarm::play_alarm;
use calendar::MonthCache;
pub use events::Notice;
use todo::RowAnim;

/// Число циферблатов часов.
pub const CLOCK_STYLES: usize = 5;
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

/// Часы: карточка, высокая и широкая — у каждого циферблата своя композиция под каждый размер.
static CLOCK_SIZES: [Size; 3] = [sz(2, 2), sz(2, 4), sz(4, 2)];
static WEATHER_SIZES: [Size; 3] = [sz(2, 1), sz(2, 2), sz(4, 1)];
/// Неделя: с ближайшим событием, шире, и расширенная со списком событий на сегодня.
static WEEK_SIZES: [Size; 3] = [sz(4, 2), sz(6, 2), sz(4, 4)];
static TIMER_SIZES: [Size; 3] = [sz(4, 1), sz(4, 2), sz(6, 2)];
static TODO_SIZES: [Size; 3] = [sz(4, 2), sz(4, 4), sz(6, 4)];

/// Сетка вкладки: 10 колонок и 4 ряда крупных ячеек (шаг CELL=56, зазор GAP=8; рядов до 5 — под месячный календарь). Часы и погода стоят
/// столбцом шириной 2 ячейки, неделя и таймер — 4×2 рядом, задачи — 4×4: исходные пропорции макета.
///
/// Статический `min_rows = 1` задаёт компактный размер при скрытых виджетах или пустой вкладке.
/// Месячный календарь (MONTH_H = 252 px) запрашивает место динамически через `overlay_rows`.
pub static SPEC: TabSpec = TabSpec {
    cols: 12,
    rows: 5,
    min_cols: 4,
    min_rows: 1,
    specs: &[
        Spec { key: "clock", title: "Часы", sizes: &CLOCK_SIZES, size: 0, col: 0, row: 0 },
        Spec { key: "weather", title: "Погода", sizes: &WEATHER_SIZES, size: 1, col: 0, row: 2 },
        Spec { key: "week", title: "Эта неделя", sizes: &WEEK_SIZES, size: 0, col: 2, row: 0 },
        Spec { key: "timer", title: "Таймер", sizes: &TIMER_SIZES, size: 1, col: 2, row: 2 },
        Spec { key: "todo", title: "Задачи", sizes: &TODO_SIZES, size: 1, col: 6, row: 0 },
    ],
};

/// Вычисляет число рядов сетки, необходимое для раскрытого месячного календаря.
/// Динамический расчёт overlay_rows:
/// Месячный календарь (MONTH_H = 252 px) раскрывается поверх сетки.
/// Число рядов под высоту MONTH_H: ceil(MONTH_H / CELL) = ceil(252 / 56) = 5 рядов.
#[inline]
pub fn today_overlay_rows(month_h: f32) -> u8 {
    (month_h / super::layout::CELL).ceil().max(0.0) as u8
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
    /// Время создания каждой задачи, секунды с 1970 (та же длина, что у `todo`); 0 — даты нет.
    todo_ts: Vec<u64>,
    /// Сортировка списка задач (сохраняется в `todo.txt`).
    todo_sort: todo::Sort,
    /// Убирать выполненные задачи при смене дня (настройка `todo_clean_done`).
    clean_done: bool,
    /// День (от 1970-01-01), в который список проверялся в последний раз; 0 — ещё не записан.
    todo_day: i64,
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
    /// События, созданные в острове (`events.txt`); `local_ver` растёт при каждом изменении.
    local: Vec<crate::online::LocalEvent>,
    local_ver: u64,
    /// Календарь по ссылкам вместе со своими событиями: (версии источников, список).
    merged: Option<((u64, u64), std::sync::Arc<Vec<online::RawEvent>>)>,
    /// Минута, до которой напоминания о событиях уже проверены.
    seen_min: i64,
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
            todo_ts: Vec::new(),
            todo_sort: todo::Sort::default(),
            clean_done: false,
            todo_day: 0,
            anim: Vec::new(),
            month_t: Spring::unit(0.0, 240.0, 26.0),
            city: String::new(),
            editing: false,
            ics: Vec::new(),
            cal_editing: false,
            cal_bad: false,
            online: Online::default(),
            local: Vec::new(),
            local_ver: 0,
            merged: None,
            seen_min: 0,
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
        self.clean_done = cfg.todo_clean_done;
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
        self.merge_events();
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

    /// Обрывает сигнал окончания таймера.
    pub fn stop_alarm() {
        alarm::stop_alarm();
    }

    /// Звучит ли сейчас сигнал окончания таймера.
    pub fn alarm_playing() -> bool {
        alarm::is_playing()
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
    /// (с полосой правки).
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, grid: &mut Grid, w: f32, h: f32) {
        self.month_t.target = if self.month_open { 1.0 } else { 0.0 };
        let month_active = self.month_open || self.month_t.value > 0.0;
        let overlay_rows = if month_active { today_overlay_rows(MONTH_H) } else { 0 };
        grid.set_overlay_rows(overlay_rows);

        let area = grid.area(w, h);
        let t = GetLocalTime();
        let dow = (t.wDayOfWeek as usize + 6) % 7;
        let today = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
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
    /// `size` — индекс в `CLOCK_SIZES`: у каждого циферблата своя композиция под каждый размер.
    unsafe fn clock_card(&mut self, ui: &mut Ui, cv: &Canvas, c: Rect, size: usize, t: &SYSTEMTIME, dow: usize) {
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
            cv.rr(c, crate::ui::card_radius(), D2D1_COLOR_F { a: 0.05, ..cv.accent });
            cv.rr_line(c, crate::ui::card_radius(), cv.accent, 1.5);
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
        if self.clock_style == 2 {
            clock::aim_seconds(&mut self.sec_ring, t.wSecond);
        }
        self.clock_face(cv, c, t, size, dow);
        if picking {
            // Какой циферблат из скольких: точки на капсуле у нижнего края.
            let (n, pitch) = (CLOCK_STYLES as f32, 7.0);
            let pill = Rect::new(c.cx() - n * pitch / 2.0 - 4.0, c.bottom() - 11.0, n * pitch + 8.0, 9.0);
            cv.rr(pill, pill.h / 2.0, crate::ui::chip_bg());
            for k in 0..CLOCK_STYLES {
                let x = pill.x + 4.0 + pitch * (k as f32 + 0.5);
                let col = if k == self.clock_style { cv.accent } else { D2D1_COLOR_F { a: 0.35, ..dim() } };
                cv.circle(x, pill.cy(), 2.0, col);
            }
        }
    }
}

#[cfg(feature = "gallery")]
impl Today {
    /// Демо-данные галереи: погода, календарь на сегодня, идущий таймер, задачи.
    pub fn demo(&mut self, today: i64) {
        use std::sync::Arc;
        let at = |h: i64, m: i64| today * 1440 + h * 60 + m;
        let ev = |h: i64, m: i64, mins: i64, title: &str| online::RawEvent {
            start: at(h, m),
            end: at(h, m) + mins,
            all_day: false,
            title: title.into(),
            rrule: None,
            exdates: Vec::new(),
            uid: title.into(),
        };
        self.online.weather = Some(online::Weather { temp: 18.4, code: 1, city: "Душанбе".into() });
        self.online.calendar_set = true;
        self.online.cal = Arc::new(vec![
            ev(9, 30, 30, "Планёрка команды"),
            ev(12, 0, 60, "Обед с Алексеем"),
            ev(15, 0, 45, "Созвон с дизайнерами"),
            ev(18, 30, 90, "Тренировка"),
        ]);
        self.next = Some(online::Event { start: at(15, 0), end: at(15, 45), all_day: false, title: "Созвон с дизайнерами".into() });
        self.next_min = at(10, 9);
        self.total = Duration::from_secs(5 * 60);
        self.left = Duration::from_secs(3 * 60 + 25);
        self.deadline = Some(Instant::now() + self.left);
        self.preset = Some(1);
        self.todo = vec![
            (false, "Купить продукты на неделю".into()),
            (true, "Отправить отчёт".into()),
            (false, "Позвонить в банк насчёт новой карты и уточнить лимиты".into()),
            (false, "Забрать посылку".into()),
            (true, "Записаться к стоматологу".into()),
        ];
        self.todo_ts = vec![0; self.todo.len()];
        self.anim = self.todo.iter().map(|(d, _)| RowAnim::new(*d, false)).collect();
    }

    pub fn set_clock_style(&mut self, s: usize) {
        self.clock_style = s % CLOCK_STYLES;
    }

    /// Один виджет вне сетки: номер в `SPEC`, индекс размера и прямоугольник.
    pub unsafe fn gallery(&mut self, ui: &mut Ui, cv: &Canvas, idx: usize, size: usize, rect: Rect, t: &SYSTEMTIME) {
        let dow = (t.wDayOfWeek as usize + 6) % 7;
        let today = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        self.widget(ui, cv, &Item { idx, size, rect }, None, t, today, dow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_today_overlay_rows() {
        // Высота месячного календаря MONTH_H = 252 px -> 252 / 56 = 4.5 -> ceil = 5 рядов (вся сетка)
        assert_eq!(today_overlay_rows(MONTH_H), 5);
        assert_eq!(today_overlay_rows(252.0), 5);
        assert_eq!(today_overlay_rows(SPEC.rows as f32 * crate::tabs::layout::CELL), SPEC.rows);

        // Меньшие высоты
        assert_eq!(today_overlay_rows(56.0), 1);
        assert_eq!(today_overlay_rows(57.0), 2);
        assert_eq!(today_overlay_rows(0.0), 0);
    }
}
