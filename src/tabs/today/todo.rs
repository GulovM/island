//! Список задач: хранение в `todo.txt`, анимация строк, карточка задач.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::Today;
use crate::anim::Spring;
use crate::tabs::{id, read_lines, write_lines, TODAY};
use crate::ui::{chip_bg, dim, fg, icon, inset, mix, status_green, track, txt, Canvas, Rect, Submit, Ui};

const MAX_TODO: usize = 100;

const TODO_FILE: &str = "todo.txt";

/// Сортировка списка задач. «Вручную» порядок не трогает (новые добавляются сверху).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Sort {
    #[default]
    Manual,
    /// Новые сверху; задачи без даты (созданные в старых версиях) в конце.
    Date,
    /// Невыполненные сверху, внутри групп порядок сохраняется.
    Status,
}

impl Sort {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Date => "date",
            Self::Status => "status",
        }
    }

    fn parse(s: &str) -> Self {
        match s.trim() {
            "date" => Self::Date,
            "status" => Self::Status,
            _ => Self::Manual,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Manual => Self::Date,
            Self::Date => Self::Status,
            Self::Status => Self::Manual,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Manual => "Вручную",
            Self::Date => "По дате",
            Self::Status => "По статусу",
        }
    }
}

/// Новый порядок индексов для `items` (выполнено, время создания) по сортировке; сортировка устойчивая.
fn sorted_order(sort: Sort, items: &[(bool, u64)]) -> Vec<usize> {
    let mut ord: Vec<usize> = (0..items.len()).collect();
    match sort {
        Sort::Manual => {}
        // Ключ — убывание времени создания; задачи без даты (0) оказываются в конце.
        Sort::Date => ord.sort_by_key(|&i| std::cmp::Reverse(items[i].1)),
        Sort::Status => ord.sort_by_key(|&i| items[i].0),
    }
    ord
}

/// Разбор `todo.txt`: необязательные строки `#sort=…` и `#day=…` (день последней проверки),
/// затем `выполнено[@время]|текст`.
fn parse_todo(lines: &[String]) -> (Sort, i64, Vec<(bool, String, u64)>) {
    let sort = lines.iter().find_map(|l| l.strip_prefix("#sort=")).map_or(Sort::Manual, Sort::parse);
    let day = lines.iter().find_map(|l| l.strip_prefix("#day=")).and_then(|d| d.trim().parse().ok()).unwrap_or(0);
    let items = lines
        .iter()
        .filter_map(|l| l.split_once('|'))
        .map(|(head, t)| {
            let (d, ts) = head.split_once('@').unwrap_or((head, ""));
            (d == "1", t.to_string(), ts.parse().unwrap_or(0))
        })
        .take(MAX_TODO)
        .collect();
    (sort, day, items)
}

/// Плавность строки задачи: смещение при вставке/удалении соседей, появление и отметка «выполнено».
pub(super) struct RowAnim {
    pos: Spring,
    fade: Spring,
    done: Spring,
    /// Положение уже выставлено (первая отрисовка ставит строку на место без движения).
    placed: bool,
}

impl RowAnim {
    pub(super) fn new(done: bool, entering: bool) -> Self {
        let mut fade = Spring::unit(if entering { 0.0 } else { 1.0 }, 220.0, 24.0);
        fade.target = 1.0;
        let mut d = Spring::unit(done as u8 as f32, 260.0, 26.0);
        d.target = done as u8 as f32;
        Self { pos: Spring::new(0.0, 300.0, 28.0), fade, done: d, placed: entering }
    }

    pub(super) fn settled(&self) -> bool {
        self.pos.settled() && self.fade.settled() && self.done.settled()
    }

    pub(super) fn step(&mut self, dt: f32) {
        self.pos.step(dt);
        self.fade.step(dt);
        self.done.step(dt);
    }
}

/// Состояние перетаскивания задачи мышью внутри списка.
#[derive(Clone, Copy, Debug)]
pub(super) struct TodoDrag {
    /// Исходный индекс перетаскиваемой задачи в `todo`.
    pub(super) idx: usize,
    /// Координаты мыши в момент нажатия.
    pub(super) press_pos: (f32, f32),
    /// Смещение курсора от верхнего края строки в момент нажатия.
    pub(super) grab_y: f32,
    /// Движение превысило порог гистерезиса: жест стал перетаскиванием, а не кликом.
    pub(super) live: bool,
    /// Целевой индекс вставки, под который освобождено место.
    pub(super) target: usize,
}

impl Today {
    pub fn load(&mut self) {
        self.todo_drag = None;
        let (sort, day, items) = parse_todo(&read_lines(TODO_FILE));
        self.todo_sort = sort;
        self.todo_day = day;
        self.todo_ts = items.iter().map(|i| i.2).collect();
        self.todo = items.into_iter().map(|(d, t, _)| (d, t)).collect();
        self.anim = self.todo.iter().map(|(d, _)| RowAnim::new(*d, false)).collect();
        self.resort();
    }

    fn save(&self) {
        let mut lines = vec![format!("#sort={}", self.todo_sort.as_str()), format!("#day={}", self.todo_day)];
        lines.extend(self.todo.iter().zip(&self.todo_ts).map(|((d, t), ts)| format!("{}@{ts}|{t}", if *d { 1 } else { 0 })));
        write_lines(TODO_FILE, &lines);
    }

    /// Убирает выполненные задачи насовсем (без архива) и сохраняет список.
    fn clear_done(&mut self) {
        if self.remove_done() {
            self.save();
        }
    }

    /// Выбрасывает выполненные задачи из памяти; невыполненные не трогает. `true` — что-то убрано.
    fn remove_done(&mut self) -> bool {
        self.todo_drag = None;
        let keep: Vec<bool> = self.todo.iter().map(|(d, _)| !*d).collect();
        if keep.iter().all(|&k| k) {
            return false;
        }
        let new_edit = self.edit.and_then(|e| keep.get(e).copied().unwrap_or(false).then(|| keep[..e].iter().filter(|&&k| k).count()));
        let mut i = 0;
        self.todo.retain(|_| {
            i += 1;
            keep[i - 1]
        });
        let mut i = 0;
        self.todo_ts.retain(|_| {
            i += 1;
            keep.get(i - 1).copied().unwrap_or(true)
        });
        let mut i = 0;
        self.anim.retain(|_| {
            i += 1;
            keep.get(i - 1).copied().unwrap_or(true)
        });
        self.edit = new_edit;
        true
    }

    /// Смена дня: при включённой настройке выполненные задачи убираются (и при запуске после полуночи).
    /// Первая запись дня (0, файл старой версии) только запоминается.
    fn roll_day(&mut self) {
        let today = crate::online::now_local_minutes().div_euclid(1440);
        if self.todo_day == today {
            return;
        }
        let first = self.todo_day == 0;
        self.todo_day = today;
        if self.clean_done && !first {
            self.clear_done();
        } else {
            self.save();
        }
    }

    /// Переставляет задачи по выбранной сортировке; пружины строк едут на новые места сами.
    fn resort(&mut self) {
        self.todo_drag = None;
        let items: Vec<(bool, u64)> = self.todo.iter().map(|(d, _)| *d).zip(self.todo_ts.iter().copied()).collect();
        let ord = sorted_order(self.todo_sort, &items);
        if ord.iter().enumerate().all(|(n, &i)| n == i) || self.anim.len() != self.todo.len() {
            return;
        }
        let mut todo: Vec<Option<(bool, String)>> = std::mem::take(&mut self.todo).into_iter().map(Some).collect();
        let mut anim: Vec<Option<RowAnim>> = std::mem::take(&mut self.anim).into_iter().map(Some).collect();
        let ts = std::mem::take(&mut self.todo_ts);
        for &i in &ord {
            self.todo.extend(todo[i].take());
            self.anim.extend(anim[i].take());
            self.todo_ts.push(ts[i]);
        }
        self.edit = self.edit.and_then(|e| ord.iter().position(|&i| i == e));
    }

    /// Переставляет задачу с позиции `from` на позицию `to`, синхронно сдвигая
    /// параллельные векторы `todo`, `todo_ts`, `anim` и корректируя `edit`.
    pub(super) fn reorder(&mut self, from: usize, to: usize) {
        let n = self.todo.len();
        if from >= n || to >= n || from == to {
            return;
        }
        if self.todo_ts.len() != n || self.anim.len() != n {
            return;
        }
        let item = self.todo.remove(from);
        self.todo.insert(to, item);

        let ts = self.todo_ts.remove(from);
        self.todo_ts.insert(to, ts);

        let anim = self.anim.remove(from);
        self.anim.insert(to, anim);

        if let Some(e) = self.edit {
            self.edit = Some(if e == from {
                to
            } else if from < to && e > from && e <= to {
                e - 1
            } else if from > to && e >= to && e < from {
                e + 1
            } else {
                e
            });
        }
    }

    pub(super) unsafe fn todo_card(&mut self, ui: &mut Ui, cv: &Canvas, card: Rect) {
        self.roll_day();
        cv.card(card);
        let f = &cv.g.fonts;
        let done = self.todo.iter().filter(|(d, _)| *d).count();
        // Низкая карточка: шапка и поле ввода плотнее, чтобы поместились две задачи.
        let compact = card.h < 160.0;
        let (head, field_h, row_h, pad) = if compact { (28.0, 24.0, 25.0, 8.0) } else { (38.0, 28.0, 27.0, 10.0) };
        let head_cy = card.y + head / 2.0 + 2.0;
        cv.line("Задачи", &f.body, card.x + 12.0, 90.0, head_cy, crate::ui::Align::Leading, txt());
        // Счётчик «выполнено/всего» в кружке с дугой прогресса.
        let (bx, by) = (card.right() - 21.0, head_cy);
        cv.circle(bx, by, 9.0, chip_bg());
        if !self.todo.is_empty() {
            cv.ring(bx, by, 8.0, 1.4, done as f32 / self.todo.len() as f32, status_green());
        }
        cv.text(&format!("{done}/{}", self.todo.len()), Rect::new(bx - 9.0, by - 6.0, 18.0, 12.0), &f.caps_c, txt());
        // Сортировка: чип левее счётчика, по нажатию Вручную → По дате → По статусу. Выбранная подсвечена.
        let sort_label = self.todo_sort.label();
        let sort_w = cv.chip_w(sort_label);
        let sort_r = Rect::new(bx - 9.0 - 6.0 - sort_w, by - 8.5, sort_w, 17.0);
        if ui.chip(cv, id(TODAY, 33), sort_r, sort_label, self.todo_sort != Sort::Manual) {
            self.todo_sort = self.todo_sort.next();
            self.resort();
            self.save();
        }
        // Ручная очистка: чип левее сортировки, виден, пока есть выполненные и хватает места рядом с заголовком.
        let clear_w = cv.chip_w("Убрать выполненные");
        let clear_r = Rect::new(sort_r.x - 6.0 - clear_w, by - 8.5, clear_w, 17.0);
        if done > 0 && clear_r.x >= card.x + 12.0 + 64.0 && ui.chip(cv, id(TODAY, 34), clear_r, "Убрать выполненные", false) {
            self.clear_done();
        }

        // Поле «добавить» внизу и золотая кнопка со стрелкой.
        let fid = id(TODAY, 30);
        let field = Rect::new(card.x + 10.0, card.bottom() - pad - field_h, card.w - 20.0, field_h);
        let editing = self.edit.is_some_and(|i| i < self.todo.len());
        if !editing {
            self.edit = None;
        }
        let (hint, glyph) = if editing { ("Изменить задачу…", icon::EDIT) } else { ("Добавить задачу…", icon::ADD) };
        // Нажатие достаётся первой зарегистрированной области под курсором, поэтому кнопки поверх
        // поля регистрируются раньше него, а рисуются после.
        let add = Rect::new(field.right() - 22.0, field.cy() - 8.0, 16.0, 16.0);
        let cancel = Rect::new(add.x - 19.0, add.y, 16.0, 16.0);
        let (_, add_clicked) = ui.click_area(id(TODAY, 31), add);
        let cancel_clicked = editing && ui.click_area(id(TODAY, 32), cancel).1;
        // Справа поверх поля лежат кнопки: текст до них не доходит.
        ui.text_field_pad(cv, fid, field, hint, glyph, Submit::Clear, if editing { 45.0 } else { 28.0 });
        // Задачи — синие, как Напоминания iOS (в стиле Windows — акцент).
        let rem = crate::ui::hue(crate::ios::SysColor::Blue, cv.accent);
        cv.rr_line(field, 8.0, if editing { rem } else { track() }, 1.0);
        let hv = ui.hover_of(id(TODAY, 31));
        cv.rr(add, 4.0, mix(rem, fg(), 0.12 * hv));
        cv.glyph(if editing { icon::CHECK } else { icon::FORWARD }, add, false, crate::ui::on_color(rem));
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
                    self.todo_ts.insert(0, crate::sys::now_secs());
                    self.anim.insert(0, RowAnim::new(false, true));
                    self.todo_drag = None;
                    self.resort();
                    self.save();
                }
                None => {}
            }
        }

        let row = row_h;
        const GAP: f32 = 3.0;
        /// Длинная задача переносится максимум на столько строк, дальше — «…» (целиком — в правке).
        const MAX_LINES: usize = 3;
        // Видимая область — целое число строк: край следующей задачи не торчит из-под списка.
        let avail = (field.y - pad / 2.0 - (card.y + head)).max(row);
        let whole = ((avail + GAP) / (row + GAP)).floor().max(1.0);
        let view = Rect::new(card.x + 9.0, card.y + head, card.w - 18.0, whole * (row + GAP) - GAP);
        let text_w = (view.w - 4.0 - 25.0 - 8.0).max(20.0);
        // Раскладка строк: (смещение, высота, число строк, высота строки текста).
        let mut rows: Vec<(f32, f32, usize, f32)> = Vec::with_capacity(self.todo.len());
        let mut total = 0.0f32;
        for (_, text) in &self.todo {
            let (lines, lh) = cv.wrapped(text, &f.wrap_l, text_w);
            let lines = lines.clamp(1, MAX_LINES);
            let h = (lines as f32 * lh + 11.0).max(row);
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

        // Перетаскивание строк мышью: 1:1 следование за курсором и плавный сдвиг
        // остальных строк пружинами anim[i].pos.target в ручном режиме сортировки.
        let mut dropped = None;
        if let Some(mut d) = self.todo_drag.take() {
            if d.idx < self.todo.len() && self.todo_sort == Sort::Manual {
                let drag_id = id(TODAY, 200 + d.idx as u32);
                let ptr = ui.pointer();
                if ui.is_dragging(drag_id) {
                    if !d.live && (ptr.0 - d.press_pos.0).hypot(ptr.1 - d.press_pos.1) > 4.0 {
                        d.live = true;
                    }
                    if d.live {
                        // Автопрокрутка у краёв списка
                        const SCROLL_ZONE: f32 = 16.0;
                        const SCROLL_SPEED: f32 = 3.0;
                        if ptr.1 < view.y + SCROLL_ZONE {
                            ui.scroll_jump(sid, (off - SCROLL_SPEED).max(0.0));
                        } else if ptr.1 > view.bottom() - SCROLL_ZONE {
                            ui.scroll_jump(sid, off + SCROLL_SPEED);
                        }

                        let drag_pos = (ptr.1 - d.grab_y - view.y + off).clamp(0.0, (total - rows[d.idx].1).max(0.0));
                        self.anim[d.idx].pos.value = drag_pos;
                        self.anim[d.idx].pos.target = drag_pos;
                        self.anim[d.idx].pos.snap();

                        // Целевой слот по устойчивым серединам остальных строк
                        let drag_cy = drag_pos + rows[d.idx].1 / 2.0;
                        let mut target = 0;
                        let mut acc = 0.0f32;
                        for (k, other_idx) in (0..self.todo.len()).filter(|&j| j != d.idx).enumerate() {
                            let h = rows[other_idx].1;
                            let mid = acc + h / 2.0;
                            if drag_cy > mid {
                                target = k + 1;
                            }
                            acc += h + GAP;
                        }
                        d.target = target;

                        // Раздвигаем соседей под целевой слот
                        let mut virtual_order: Vec<usize> = (0..self.todo.len()).collect();
                        virtual_order.remove(d.idx);
                        virtual_order.insert(d.target, d.idx);
                        let mut y = 0.0f32;
                        for &item_idx in &virtual_order {
                            let h = rows[item_idx].1;
                            if item_idx != d.idx {
                                self.anim[item_idx].pos.target = y;
                            }
                            y += h + GAP;
                        }
                    }
                    self.todo_drag = Some(d);
                } else if d.live {
                    dropped = Some((d.idx, d.target));
                    // Фиксируем цели пружин на момент отпускания, чтобы строки не дергались назад до перестановки
                    let mut virtual_order: Vec<usize> = (0..self.todo.len()).collect();
                    virtual_order.remove(d.idx);
                    virtual_order.insert(d.target, d.idx);
                    let mut y = 0.0f32;
                    for &item_idx in &virtual_order {
                        let h = rows[item_idx].1;
                        self.anim[item_idx].pos.target = y;
                        y += h + GAP;
                    }
                }
            }
        }

        let mut live_drag_idx = self.todo_drag.as_ref().filter(|d| d.live).map(|d| d.idx);
        if live_drag_idx.is_none() {
            live_drag_idx = dropped.map(|(from, _)| from);
        }
        let mut draw_order: Vec<usize> = (0..self.todo.len()).collect();
        if let Some(drag_idx) = live_drag_idx {
            if drag_idx < draw_order.len() {
                draw_order.remove(drag_idx);
                draw_order.push(drag_idx);
            }
        }

        let mut toggled = None;
        let mut removed = None;
        let mut edit_req = None;
        for i in draw_order {
            let is_done = &self.todo[i].0;
            let text = &self.todo[i].1;
            let is_dragged = live_drag_idx == Some(i);
            let (_, rh, lines, lh) = rows[i];
            let (fade, dn) = (self.anim[i].fade.value.clamp(0.0, 1.0), self.anim[i].done.value.clamp(0.0, 1.0));
            let rcv = Canvas { g: cv.g, alpha: cv.alpha * fade, accent: cv.accent };
            let cv = &rcv;
            let y = view.y + self.anim[i].pos.value - off;
            if !is_dragged && (y + rh < view.y || y > view.bottom()) {
                continue;
            }
            let r = Rect::new(view.x, y, view.w - 4.0, rh);
            let rid = id(TODAY, 200 + i as u32);
            // Кнопки справа («изменить», «удалить») видны при наведении; при активном перетаскивании
            // скрываются, чтобы не мешать жесту.
            let (eid, did) = (id(TODAY, 600 + i as u32), id(TODAY, 400 + i as u32));
            let del = Rect::new(r.right() - 21.0, r.y + row / 2.0 - 9.0, 18.0, 18.0);
            let edit = Rect::new(del.x - 20.0, del.y, 18.0, 18.0);
            let show = live_drag_idx.is_none() && (ui.hover_of(rid) > 0.3 || ui.hover_of(did) > 0.3 || ui.hover_of(eid) > 0.3);
            let (mut edit_clicked, mut del_clicked) = (false, false);
            if show {
                edit_clicked = ui.click_area(eid, edit).1;
                del_clicked = ui.click_area(did, del).1;
            }
            let (_, clicked) = ui.click_area(rid, r);
            if self.todo_sort == Sort::Manual && self.todo.len() > 1 && self.todo_drag.is_none() && ui.is_dragging(rid) {
                self.todo_drag = Some(TodoDrag {
                    idx: i,
                    press_pos: ui.pointer(),
                    grab_y: ui.pointer().1 - r.y,
                    live: false,
                    target: i,
                });
            }
            let hv = ui.hover_of(rid);
            if is_dragged {
                // Приподнятая карточка в руке: контрастный фон и контур акцента
                cv.rr(r, 8.0, mix(crate::ui::card(), fg(), 0.12));
                let rem = crate::ui::hue(crate::ios::SysColor::Blue, cv.accent);
                cv.rr_line(r, 8.0, if self.edit == Some(i) { rem } else { mix(fg(), cv.accent, 0.45) }, 1.2);
            } else {
                cv.rr(r, 8.0, mix(inset(), fg(), 0.035 * hv));
                if self.edit == Some(i) {
                    cv.rr_line(r, 8.0, crate::ui::hue(crate::ios::SysColor::Blue, cv.accent), 1.0);
                }
            }
            let (cx, cy) = (r.x + 13.0, r.y + row / 2.0);
            // Отметка выполнения проявляется плавно: кольцо гаснет, зелёный круг с галочкой нарастает.
            if dn < 0.99 {
                let ring = D2D1_COLOR_F { a: dim().a * (1.0 - dn), ..dim() };
                cv.rr_line(Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), 6.0, ring, 1.2);
            }
            if dn > 0.01 {
                cv.circle(cx, cy, 6.0 * (0.5 + 0.5 * dn), D2D1_COLOR_F { a: dn, ..status_green() });
                let mark = crate::ui::on_color(status_green());
                cv.symbol(icon::CHECK, Rect::new(cx - 6.0, cy - 6.0, 12.0, 12.0), 9.0 * (0.6 + 0.4 * dn), D2D1_COLOR_F { a: dn, ..mark });
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
                // Правка подсвечивается синим, удаление при наведении — красным (разрушительное действие).
                let rem = crate::ui::hue(crate::ios::SysColor::Blue, cv.accent);
                let hot = [txt(), crate::ui::status_red()];
                for (k, (bid, rect, glyph, active)) in [(eid, edit, icon::EDIT, self.edit == Some(i)), (did, del, icon::DELETE, false)].into_iter().enumerate() {
                    let h = ui.hover_of(bid);
                    cv.rr(rect, 9.0, D2D1_COLOR_F { a: 0.10 * h, ..fg() });
                    cv.glyph(glyph, rect, false, if active { rem } else { mix(dim(), hot[k], h) });
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
            if i < self.todo_ts.len() {
                self.todo_ts.remove(i);
            }
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
            self.todo_drag = None;
            self.save();
        } else if let Some(i) = edit_req {
            self.edit = Some(i);
            ui.set_field_text(fid, &self.todo[i].1);
            ui.focus = Some(fid);
            ui.caret_on = true;
        } else if let Some((from, to)) = dropped {
            if from != to {
                self.reorder(from, to);
                self.save();
            }
        } else if let Some(i) = toggled {
            self.todo[i].0 = !self.todo[i].0;
            if let Some(a) = self.anim.get_mut(i) {
                a.done.target = self.todo[i].0 as u8 as f32;
            }
            self.resort();
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
    fn parse_todo_lines() {
        let lines = [
            "#sort=status".to_string(),
            "0|Купить молоко".to_string(),
            "1@1700000000|Сдать отчёт | срочно".to_string(),
            "invalid_line".to_string(),
            "".to_string(),
        ];
        let (sort, day, items) = parse_todo(&lines);
        assert_eq!(sort, Sort::Status);
        assert_eq!(day, 0);
        assert_eq!(items, vec![
            (false, "Купить молоко".to_string(), 0),
            (true, "Сдать отчёт | срочно".to_string(), 1_700_000_000),
        ]);
        // Без заголовка (файл старой версии) — ручной порядок.
        assert_eq!(parse_todo(&["0|a".to_string()]).0, Sort::Manual);
        assert_eq!(parse_todo(&["#day=20750".to_string()]).1, 20750);
        assert_eq!(Sort::parse("мусор"), Sort::Manual);
    }

    #[test]
    fn sort_by_date_puts_undated_last() {
        // (выполнено, время): новые сверху, без даты в конце, равные не переставляются.
        let items = [(false, 0), (false, 100), (true, 300), (false, 200), (false, 0)];
        assert_eq!(sorted_order(Sort::Date, &items), vec![2, 3, 1, 0, 4]);
    }

    #[test]
    fn sort_by_status_keeps_order_inside_groups() {
        let items = [(true, 1), (false, 2), (true, 3), (false, 4)];
        assert_eq!(sorted_order(Sort::Status, &items), vec![1, 3, 0, 2]);
        assert_eq!(sorted_order(Sort::Manual, &items), vec![0, 1, 2, 3]);
    }

    #[test]
    fn remove_done_keeps_open_tasks_and_edit_target() {
        let mut t = Today::default();
        t.todo = vec![(true, "a".into()), (false, "b".into()), (true, "c".into()), (false, "d".into())];
        t.todo_ts = vec![1, 2, 3, 4];
        t.anim = t.todo.iter().map(|(d, _)| RowAnim::new(*d, false)).collect();
        t.edit = Some(3);
        assert!(t.remove_done());
        assert_eq!(t.todo, vec![(false, "b".to_string()), (false, "d".to_string())]);
        assert_eq!(t.todo_ts, vec![2, 4]);
        assert_eq!(t.anim.len(), 2);
        assert_eq!(t.edit, Some(1), "правка едет за своей задачей");
        // Нечего убирать — ничего не меняется.
        assert!(!t.remove_done());
        // Правимая выполненная задача исчезает вместе с правкой.
        t.todo[0].0 = true;
        t.edit = Some(0);
        assert!(t.remove_done());
        assert_eq!(t.edit, None);
    }

    #[test]
    fn sort_cycles_through_all_modes() {
        assert_eq!(Sort::Manual.next().next().next(), Sort::Manual);
        for s in [Sort::Manual, Sort::Date, Sort::Status] {
            assert_eq!(Sort::parse(s.as_str()), s);
        }
    }

    #[test]
    fn reorder_moves_parallel_vectors_and_tracks_edit() {
        let mut t = Today::default();
        t.todo = vec![
            (false, "a".into()),
            (true, "b".into()),
            (false, "c".into()),
            (false, "d".into()),
        ];
        t.todo_ts = vec![10, 20, 30, 40];
        t.anim = t.todo.iter().map(|(d, _)| RowAnim::new(*d, false)).collect();

        // 1. Сдвиг сверху вниз: задача 0 ("a") перемещается на позицию 2
        // Ожидаемый порядок: "b", "c", "a", "d"
        t.edit = Some(0);
        t.reorder(0, 2);
        assert_eq!(t.todo, vec![
            (true, "b".to_string()),
            (false, "c".to_string()),
            (false, "a".to_string()),
            (false, "d".to_string()),
        ]);
        assert_eq!(t.todo_ts, vec![20, 30, 10, 40]);
        assert_eq!(t.anim.len(), 4);
        assert_eq!(t.edit, Some(2), "правка следует за перемещаемой задачей при сдвиге вниз");

        // 2. Сдвиг снизу вверх: задача 2 ("a") перемещается обратно на позицию 0
        // Если при этом редактировалась задача 1 ("c"), она сдвигается вправо на позицию 2
        t.edit = Some(1);
        t.reorder(2, 0);
        assert_eq!(t.todo, vec![
            (false, "a".to_string()),
            (true, "b".to_string()),
            (false, "c".to_string()),
            (false, "d".to_string()),
        ]);
        assert_eq!(t.todo_ts, vec![10, 20, 30, 40]);
        assert_eq!(t.anim.len(), 4);
        assert_eq!(t.edit, Some(2), "правка сдвигается вправо при перемещении задачи перед ней");

        // 3. Перемещение снизу вверх самой редактируемой задачи (3 -> 1)
        t.edit = Some(3);
        t.reorder(3, 1);
        assert_eq!(t.todo, vec![
            (false, "a".to_string()),
            (false, "d".to_string()),
            (true, "b".to_string()),
            (false, "c".to_string()),
        ]);
        assert_eq!(t.todo_ts, vec![10, 40, 20, 30]);
        assert_eq!(t.edit, Some(1), "правка следует за перемещаемой задачей при сдвиге вверх");

        // 4. Задача за пределами диапазона перестановки не меняет индекс редактирования
        t.edit = Some(0);
        t.reorder(2, 3);
        assert_eq!(t.edit, Some(0));

        // 5. Некорректные индексы или перемещение на то же место — no-op
        t.edit = Some(1);
        t.reorder(1, 1);
        t.reorder(0, 100);
        t.reorder(100, 0);
        assert_eq!(t.edit, Some(1));
        assert_eq!(t.todo.len(), 4);
        assert_eq!(t.todo_ts.len(), 4);
        assert_eq!(t.anim.len(), 4);
    }
}
