//! Вкладка Today: часы, погода (Open-Meteo), неделя и ближайшее событие календаря (iCal),
//! таймер (часы/минуты, пресеты, кольцо, сигнал по окончании), список задач.
//! Раскладка снята с макета 1:1 (панель 550×275).

mod alarm;
mod calendar;
mod clock;
mod timer;

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::anim::Spring;
use crate::config::Config;
use crate::online::{self, days_from_civil, Event, Online};
use crate::ui::{icon, mix, Canvas, Rect, Submit, Ui, chip_bg, dim, GREEN, ink, inset, txt, track, fg};

use super::{id, read_lines, write_lines, PAD, TODAY, TOP};
use alarm::play_alarm;
use calendar::MonthCache;

const MAX_TODO: usize = 100;
/// Число циферблатов часов.
const CLOCK_STYLES: usize = 5;
/// Через сколько без действий выключается режим выбора циферблата.
const CLOCK_PICK_TIMEOUT: Duration = Duration::from_secs(5);
const TODO_FILE: &str = "todo.txt";

/// Дни недели с понедельника.
const SUN_COLOR: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.96, g: 0.75, b: 0.19, a: 1.0 };

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
