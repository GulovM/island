//! Список задач: хранение в `todo.txt`, анимация строк, карточка задач.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::Today;
use crate::anim::Spring;
use crate::tabs::{id, read_lines, write_lines, TODAY};
use crate::ui::{dim, fg, icon, inset, mix, status_red, track, txt, Align, Canvas, Corner, Rect, Submit, Ui};

const MAX_TODO: usize = 100;

// ID задач на вкладке: поле ввода и кнопки, прокрутка обеих колонок и по `MAX_TODO` на строки, удаление и правку.
const ID_FIELD: u32 = 30;
const ID_ADD: u32 = 31;
const ID_CANCEL: u32 = 32;
const ID_SORT: u32 = 33;
const ID_CLEAR: u32 = 34;
const ID_LIST: u32 = 100;
const ID_LIST_DONE: u32 = 35;
const ID_ROW: u32 = 200;
const ID_DEL: u32 = 400;
const ID_EDIT: u32 = 600;

/// Занятые задачами ID вкладки (полуинтервалы) для теста непересечения в `mod.rs`.
#[cfg(test)]
pub(super) const ID_RANGES: &[(u32, u32)] = &[
    (ID_FIELD, ID_LIST_DONE + 1),
    (ID_LIST, ID_LIST + 1),
    (ID_ROW, ID_ROW + MAX_TODO as u32),
    (ID_DEL, ID_DEL + MAX_TODO as u32),
    (ID_EDIT, ID_EDIT + MAX_TODO as u32),
];

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

    fn label(self) -> &'static str {
        match self {
            Self::Manual => crate::i18n::pick("Вручную", "Manual"),
            Self::Date => crate::i18n::pick("По дате", "By date"),
            Self::Status => crate::i18n::pick("По статусу", "By status"),
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

    /// Сбрасывает признак «строка на месте»: следующая отрисовка ставит строки без движения.
    #[cfg(feature = "gallery")]
    pub(super) fn unplace_rows(&mut self) {
        for a in &mut self.anim {
            a.placed = false;
        }
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

    /// Отрисовка поля ввода задачи (пилюля) и кнопок отправки/отмены внутри него.
    unsafe fn paint_field(&mut self, ui: &mut Ui, cv: &Canvas, field: Rect, is_compact: bool) -> Option<String> {
        let fid = id(TODAY, ID_FIELD);
        let editing = self.edit.is_some_and(|i| i < self.todo.len());
        let (hint, glyph, left) = if editing {
            (crate::i18n::pick("Изменить задачу…", "Edit task…"), icon::EDIT, 28.0)
        } else {
            (crate::i18n::pick("Новая задача…", "New task…"), "", 12.0)
        };
        let bs = (field.h - 8.0).max(16.0);
        let add = Rect::new(field.right() - 4.0 - bs, field.cy() - bs / 2.0, bs, bs);
        let cancel = Rect::new(add.x - 3.0 - bs, add.y, bs, bs);
        let (_, add_clicked) = ui.click_area(id(TODAY, ID_ADD), add);
        let cancel_clicked = editing && ui.click_area(id(TODAY, ID_CANCEL), cancel).1;
        let right = if editing { 2.0 * bs + 13.0 } else { bs + 10.0 };

        ui.text_field_at(cv, fid, field, hint, glyph, Submit::Clear, left, right);
        let focus = ui.on_of(fid);
        cv.rr_line(field, field.h / 2.0, mix(track(), cv.accent, if editing { 1.0 } else { focus }), 1.0);

        let hv = ui.hover_of(id(TODAY, ID_ADD));
        cv.circle(add.cx(), add.cy(), bs / 2.0, D2D1_COLOR_F { a: 0.16 + 0.84 * hv, ..cv.accent });
        let arrow = if editing { icon::CHECK } else { icon::ADD };
        cv.glyph(arrow, add.inset(2.0), false, mix(cv.accent, crate::ui::on_color(cv.accent), hv));

        if editing {
            let c_hv = ui.hover_of(id(TODAY, ID_CANCEL));
            cv.circle(cancel.cx(), cancel.cy(), bs / 2.0, D2D1_COLOR_F { a: 0.10 * c_hv, ..fg() });
            cv.glyph(icon::CLOSE, cancel.inset(2.0), false, mix(dim(), txt(), c_hv));
        }

        if cancel_clicked {
            let _ = ui.submit_field(fid);
            self.edit = None;
            if is_compact {
                ui.focus = None;
            }
        }

        let mut submitted = ui.take_submitted(fid);
        if add_clicked {
            submitted = ui.submit_field(fid);
        }
        submitted
    }

    /// Отрисовка одной строки задачи: плоская подложка, круглый чекбокс, текст и кнопки действий при наведении.
    #[allow(clippy::too_many_arguments)]
    unsafe fn render_row(
        &mut self,
        ui: &mut Ui,
        cv: &Canvas,
        i: usize,
        view: Rect,
        off: f32,
        row_info: (f32, f32, usize, f32),
        text_w: f32,
        row_h: f32,
        max_lines: usize,
        is_dragged: bool,
        can_drag: bool,
        live_drag_idx: Option<usize>,
    ) -> (bool, bool, bool) {
        let is_done = self.todo[i].0;
        let text = &self.todo[i].1;
        let (_, rh, lines, lh) = row_info;
        let (fade, dn) = (self.anim[i].fade.value.clamp(0.0, 1.0), self.anim[i].done.value.clamp(0.0, 1.0));
        let rcv = Canvas { g: cv.g, alpha: cv.alpha * fade, accent: cv.accent };
        let cv = &rcv;
        let y = view.y + self.anim[i].pos.value - off;
        if !is_dragged && (y + rh < view.y || y > view.bottom()) {
            return (false, false, false);
        }
        let r = Rect::new(view.x, y, view.w - 4.0, rh);
        let rid = id(TODAY, ID_ROW + i as u32);
        let (eid, did) = (id(TODAY, ID_EDIT + i as u32), id(TODAY, ID_DEL + i as u32));
        let del = Rect::new(r.right() - 21.0, r.y + row_h / 2.0 - 9.0, 18.0, 18.0);
        let edit = Rect::new(del.x - 20.0, del.y, 18.0, 18.0);
        let show = live_drag_idx.is_none() && (ui.hover_of(rid) > 0.3 || ui.hover_of(did) > 0.3 || ui.hover_of(eid) > 0.3);
        let mut edit_clicked = false;
        let mut del_clicked = false;
        if show {
            edit_clicked = ui.click_area(eid, edit).1;
            del_clicked = ui.click_area(did, del).1;
        }
        let (_, clicked) = ui.click_area(rid, r);
        if can_drag && self.todo.len() > 1 && self.todo_drag.is_none() && ui.is_dragging(rid) {
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
            // Плоская строка: подложка только при наведении и контур при правке
            if hv > 0.001 {
                cv.rr(r, 8.0, mix(inset(), fg(), 0.035 * hv));
            }
            if self.edit == Some(i) {
                cv.rr_line(r, 8.0, crate::ui::hue(crate::ios::SysColor::Blue, cv.accent), 1.0);
            }
        }

        // Круглый чекбокс: кольцо dim при невыполненном, залитый cv.accent круг с галочкой при выполнении
        let (cx, cy) = (r.x + 14.0, r.y + row_h / 2.0);
        if dn < 0.99 {
            let ring = D2D1_COLOR_F { a: dim().a * (1.0 - dn), ..dim() };
            cv.rr_line(Rect::new(cx - 7.0, cy - 7.0, 14.0, 14.0), 7.0, ring, 1.2);
        }
        if dn > 0.01 {
            cv.circle(cx, cy, 7.0 * (0.5 + 0.5 * dn), D2D1_COLOR_F { a: dn, ..cv.accent });
            let mark = crate::ui::on_color(cv.accent);
            cv.symbol(icon::CHECK, Rect::new(cx - 7.0, cy - 7.0, 14.0, 14.0), 10.0 * (0.6 + 0.4 * dn), D2D1_COLOR_F { a: dn, ..mark });
        }

        let f = &cv.g.fonts;
        let th = lines as f32 * lh;
        let tr = Rect::new(r.x + 27.0, r.y + (r.h - th) / 2.0, text_w, th + 2.0);
        let shown = cv.wrap_clip(text, &f.wrap_l, text_w, max_lines);
        cv.text_strike(&shown, tr, &f.wrap_l, mix(txt(), dim(), dn), is_done);

        if show {
            let patch = Rect::new(edit.x - 4.0, edit.y - 1.0, r.right() - edit.x + 2.0, 20.0);
            cv.rr(patch, 8.0, mix(inset(), fg(), 0.035 * hv));
            let rem = crate::ui::hue(crate::ios::SysColor::Blue, cv.accent);
            let hot = [txt(), status_red()];
            for (k, (bid, rect, glyph, active)) in [(eid, edit, icon::EDIT, self.edit == Some(i)), (did, del, icon::DELETE, false)].into_iter().enumerate() {
                let h = ui.hover_of(bid);
                cv.rr(rect, 9.0, D2D1_COLOR_F { a: 0.10 * h, ..fg() });
                cv.glyph(glyph, rect, false, if active { rem } else { mix(dim(), hot[k], h) });
            }
        }

        (clicked, del_clicked, edit_clicked)
    }

    pub(super) unsafe fn todo_card(&mut self, ui: &mut Ui, cv: &Canvas, card: Rect) {
        self.roll_day();
        cv.card(card);
        cv.glow(card, 14.0, cv.accent, Corner::TopRight);

        let f = &cv.g.fonts;
        let done = self.todo.iter().filter(|(d, _)| *d).count();
        let fid = id(TODAY, ID_FIELD);
        let editing = self.edit.is_some_and(|i| i < self.todo.len());
        if !editing {
            self.edit = None;
        }

        // Определение размера по геометрии карточки:
        // 4x2 = h < 130, 4x3 = h < 190, 4x4 = w < 260 и h >= 190, 6x4 = w >= 260.
        let is_wide = card.w >= 260.0;
        let is_compact = !is_wide && card.h < 130.0;
        let is_tall = !is_wide && card.h >= 190.0;

        // 1. Отрисовка шапки и поля ввода в зависимости от размера
        let mut submitted = None;

        if is_compact {
            // 4x2 (216x104): шапка "ЗАДАЧИ" + "2 из 7", справа кнопка "+".
            // Поле ввода появляется только по клику на "+" или в режиме правки, заменяя шапку.
            let show_field = self.edit.is_some() || ui.focus == Some(fid);
            if show_field {
                let field = Rect::new(card.x + 10.0, card.y + 9.0, card.w - 20.0, 24.0);
                submitted = self.paint_field(ui, cv, field, true);
            } else {
                let title = crate::i18n::pick("ЗАДАЧИ", "TASKS");
                let tw = cv.text_w(title, &f.caps);
                cv.line(title, &f.caps, card.x + 12.0, tw + 2.0, card.y + 21.0, Align::Leading, dim());

                let count_s = format!("{done} {} {}", crate::i18n::pick("из", "of"), self.todo.len());
                let count_w = cv.text_w(&count_s, &f.small);
                cv.line(&count_s, &f.small, card.x + 12.0 + tw + 8.0, count_w + 2.0, card.y + 21.0, Align::Leading, dim());

                let btn_sz = 22.0;
                let btn_r = Rect::new(card.right() - 12.0 - btn_sz, card.y + 10.0, btn_sz, btn_sz);

                // Чип сортировки (как в 4×3 и 4×4): если места в шапке не хватает, не рисуется.
                let sort_label = self.todo_sort.label();
                let sort_w = cv.chip_w(sort_label);
                let sort_r = Rect::new(card.x + 12.0 + tw + 8.0 + count_w + 8.0, card.y + 12.5, sort_w, 17.0);
                if sort_r.right() < btn_r.x - 6.0 && ui.chip(cv, id(TODAY, ID_SORT), sort_r, sort_label, self.todo_sort != Sort::Manual) {
                    self.todo_sort = self.todo_sort.next();
                    self.resort();
                    self.save();
                }
                let (_, add_clicked) = ui.click_area(id(TODAY, ID_ADD), btn_r);
                let hv = ui.hover_of(id(TODAY, ID_ADD));
                cv.circle(btn_r.cx(), btn_r.cy(), 11.0, D2D1_COLOR_F { a: 0.16 + 0.84 * hv, ..cv.accent });
                cv.glyph(icon::ADD, btn_r.inset(3.0), false, mix(cv.accent, crate::ui::on_color(cv.accent), hv));
                if add_clicked {
                    ui.focus = Some(fid);
                    ui.caret_on = true;
                }
            }
        } else if is_wide {
            // 6x4 (328x216): слева три строки (ЗАДАЧИ, счётчик 16px semibold, прогресс 120x4),
            // справа вверху поле ввода-пилюля ~130px.
            let title = crate::i18n::pick("ЗАДАЧИ", "TASKS");
            cv.line(title, &f.caps, card.x + 14.0, 100.0, card.y + 16.0, Align::Leading, dim());

            let done_str = format!(
                "{} {} {} {}",
                done,
                crate::i18n::pick("из", "of"),
                self.todo.len(),
                crate::i18n::pick("выполнено", "done")
            );
            cv.digits(&done_str, card.x + 14.0, 160.0, card.y + 35.0, 16.0, 600, Align::Leading, txt());

            let frac = if self.todo.is_empty() { 0.0 } else { done as f32 / self.todo.len() as f32 };
            cv.progress(Rect::new(card.x + 14.0, card.y + 49.0, 120.0, 4.0), frac, cv.accent);

            let fw = 130.0;
            let field = Rect::new(card.right() - 14.0 - fw, card.y + 14.0, fw, 26.0);
            submitted = self.paint_field(ui, cv, field, false);
        } else {
            // 4x3 и 4x4: шапка "ЗАДАЧИ" слева, "2 из 7" справа, чип сортировки между ними.
            let title = crate::i18n::pick("ЗАДАЧИ", "TASKS");
            let tw = cv.text_w(title, &f.caps);
            cv.line(title, &f.caps, card.x + 12.0, tw + 2.0, card.y + 19.0, Align::Leading, dim());

            let count_s = format!("{done} {} {}", crate::i18n::pick("из", "of"), self.todo.len());
            let count_w = cv.text_w(&count_s, &f.small);
            let count_x = card.right() - 12.0 - count_w;
            cv.line(&count_s, &f.small, count_x, count_w + 2.0, card.y + 19.0, Align::Trailing, dim());

            let sort_label = self.todo_sort.label();
            let sort_w = cv.chip_w(sort_label);
            let sort_r = Rect::new(card.x + 12.0 + tw + 8.0, card.y + 10.5, sort_w, 17.0);
            if sort_r.right() < count_x - 6.0 && ui.chip(cv, id(TODAY, ID_SORT), sort_r, sort_label, self.todo_sort != Sort::Manual) {
                self.todo_sort = self.todo_sort.next();
                self.resort();
                self.save();
            }

            // В 4x3 мелкий чип "Убрать выполненные" при наличии выполненных и свободного места
            if !is_tall && done > 0 {
                let clear_lbl = crate::i18n::pick("Убрать выполненные", "Clear done");
                let clear_w = cv.chip_w(clear_lbl);
                let clear_r = Rect::new(sort_r.right() + 6.0, card.y + 10.5, clear_w, 17.0);
                if clear_r.right() < count_x - 6.0 && ui.chip(cv, id(TODAY, ID_CLEAR), clear_r, clear_lbl, false) {
                    self.clear_done();
                }
            }

            // Поле ввода-пилюля под шапкой
            let field = Rect::new(card.x + 10.0, card.y + 32.0, card.w - 20.0, 26.0);
            submitted = self.paint_field(ui, cv, field, false);
        }

        // 2. Обработка отправки текста из поля ввода
        if let Some(text) = submitted {
            let text = text.trim();
            match self.edit.take() {
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
            if is_compact {
                ui.focus = None;
            }
        }

        // 3. Футер в 4x4 (слева "Очистить выполненные", справа progress)
        if is_tall {
            let footer_cy = card.bottom() - 13.0;
            let clear_lbl = crate::i18n::pick("Очистить выполненные", "Clear completed");
            let cw = cv.text_w(clear_lbl, &f.small);
            let cr = Rect::new(card.x + 12.0, footer_cy - 8.0, cw + 4.0, 16.0);
            let (_, c_clicked) = ui.click_area(id(TODAY, ID_CLEAR), cr);
            let c_hv = ui.hover_of(id(TODAY, ID_CLEAR));
            let c_col = if done > 0 { mix(dim(), txt(), c_hv) } else { D2D1_COLOR_F { a: dim().a * 0.4, ..dim() } };
            cv.line(clear_lbl, &f.small, cr.x, cr.w, footer_cy, Align::Leading, c_col);
            if c_clicked && done > 0 {
                self.clear_done();
            }

            let prog_w = 70.0;
            let prog_r = Rect::new(card.right() - 12.0 - prog_w, footer_cy - 2.0, prog_w, 4.0);
            let frac = if self.todo.is_empty() { 0.0 } else { done as f32 / self.todo.len() as f32 };
            cv.progress(prog_r, frac, cv.accent);
        }

        // 4. Списки задач: двухколоночный в 6x4 и одноколоночный в остальных размерах
        let mut toggled = None;
        let mut removed = None;
        let mut edit_req = None;
        const GAP: f32 = 3.0;

        if is_wide {
            // --- 6x4: ДВЕ КОЛОНКИ ---
            let content_w = card.w - 28.0;
            let left_w = (content_w * 0.62).round();
            let gap = 12.0;
            let div_x = card.x + 14.0 + left_w + gap / 2.0;
            let right_x = div_x + gap / 2.0;
            let right_w = (card.right() - 14.0 - right_x).max(10.0);

            // Тонкая разделительная линия между колонками
            cv.stroke_line((div_x, card.y + 64.0), (div_x, card.bottom() - 10.0), 1.0, track());

            // Подзаголовки колонок
            let col_head_cy = card.y + 72.0;
            let active_count = self.todo.len() - done;
            let act_txt = format!("{} · {}", crate::i18n::pick("АКТИВНЫЕ", "ACTIVE"), active_count);
            let act_w = cv.text_w(&act_txt, &f.caps);
            cv.line(&act_txt, &f.caps, card.x + 14.0, act_w + 2.0, col_head_cy, Align::Leading, dim());

            let sort_label = self.todo_sort.label();
            let sort_w = cv.chip_w(sort_label);
            let sort_r = Rect::new(div_x - gap / 2.0 - sort_w, col_head_cy - 8.5, sort_w, 17.0);
            if ui.chip(cv, id(TODAY, ID_SORT), sort_r, sort_label, self.todo_sort != Sort::Manual) {
                self.todo_sort = self.todo_sort.next();
                self.resort();
                self.save();
            }

            let done_txt = format!("{} · {}", crate::i18n::pick("ГОТОВО", "DONE"), done);
            let done_w = cv.text_w(&done_txt, &f.caps);
            cv.line(&done_txt, &f.caps, right_x, done_w + 2.0, col_head_cy, Align::Leading, dim());

            if done > 0 {
                let clr_s = crate::i18n::pick("Очистить", "Clear");
                let clr_w = cv.text_w(clr_s, &f.small);
                let clr_r = Rect::new(card.right() - 14.0 - clr_w, col_head_cy - 7.0, clr_w, 14.0);
                let (_, clicked) = ui.click_area(id(TODAY, ID_CLEAR), clr_r);
                let clr_hv = ui.hover_of(id(TODAY, ID_CLEAR));
                cv.line(clr_s, &f.small, clr_r.x, clr_r.w, col_head_cy, Align::Trailing, mix(dim(), txt(), clr_hv));
                if clicked {
                    self.clear_done();
                }
            }

            let list_top = card.y + 86.0;
            let list_h = (card.bottom() - 10.0 - list_top).max(20.0);
            let view_left = Rect::new(card.x + 14.0, list_top, left_w, list_h);
            let view_right = Rect::new(right_x, list_top, right_w, list_h);

            let text_w_left = (view_left.w - 4.0 - 27.0 - 4.0).max(20.0);
            let text_w_right = (view_right.w - 4.0 - 27.0 - 4.0).max(20.0);
            let row_h = 24.0;

            let mut rows: Vec<(f32, f32, usize, f32)> = vec![(0.0, 0.0, 0, 0.0); self.todo.len()];
            let mut total_left = 0.0f32;
            let mut total_right = 0.0f32;

            for (i, (d, text)) in self.todo.iter().enumerate() {
                if !*d {
                    let (lines, lh) = cv.wrapped(text, &f.wrap_l, text_w_left);
                    let lines = lines.clamp(1, 2);
                    let h = (lines as f32 * lh + 11.0).max(row_h);
                    rows[i] = (total_left, h, lines, lh);
                    total_left += h + GAP;
                } else {
                    let (lines, lh) = cv.wrapped(text, &f.wrap_l, text_w_right);
                    let lines = lines.clamp(1, 2);
                    let h = (lines as f32 * lh + 11.0).max(row_h);
                    rows[i] = (total_right, h, lines, lh);
                    total_right += h + GAP;
                }
            }

            self.anim.resize_with(self.todo.len(), || RowAnim::new(false, false));
            for (a, row) in self.anim.iter_mut().zip(&rows) {
                a.pos.target = row.0;
                if !a.placed {
                    a.pos.snap();
                    a.placed = true;
                }
            }

            let sid_left = id(TODAY, ID_LIST);
            let off_left = ui.scroll_begin(cv, sid_left, view_left, total_left);

            let active_indices: Vec<usize> = self.todo.iter().enumerate().filter(|(_, (d, _))| !*d).map(|(j, _)| j).collect();

            // Перетаскивание только среди активных задач левой колонки
            let mut dropped = None;
            if let Some(mut d) = self.todo_drag.take() {
                if d.idx < self.todo.len() && self.todo_sort == Sort::Manual && !self.todo[d.idx].0 {
                    let drag_id = id(TODAY, ID_ROW + d.idx as u32);
                    let ptr = ui.pointer();
                    if ui.is_dragging(drag_id) {
                        if !d.live && (ptr.0 - d.press_pos.0).hypot(ptr.1 - d.press_pos.1) > 4.0 {
                            d.live = true;
                        }
                        if d.live {
                            const SCROLL_ZONE: f32 = 16.0;
                            const SCROLL_SPEED: f32 = 3.0;
                            if ptr.1 < view_left.y + SCROLL_ZONE {
                                ui.scroll_jump(sid_left, (off_left - SCROLL_SPEED).max(0.0));
                            } else if ptr.1 > view_left.bottom() - SCROLL_ZONE {
                                ui.scroll_jump(sid_left, off_left + SCROLL_SPEED);
                            }

                            let drag_pos = (ptr.1 - d.grab_y - view_left.y + off_left).clamp(0.0, (total_left - rows[d.idx].1).max(0.0));
                            self.anim[d.idx].pos.value = drag_pos;
                            self.anim[d.idx].pos.target = drag_pos;
                            self.anim[d.idx].pos.snap();

                            let drag_cy = drag_pos + rows[d.idx].1 / 2.0;
                            let mut target_slot = 0;
                            let mut acc = 0.0f32;
                            for (k, &other_idx) in active_indices.iter().filter(|&&j| j != d.idx).enumerate() {
                                let h = rows[other_idx].1;
                                let mid = acc + h / 2.0;
                                if drag_cy > mid {
                                    target_slot = k + 1;
                                }
                                acc += h + GAP;
                            }
                            d.target = target_slot;

                            if let Some(cur_pos) = active_indices.iter().position(|&j| j == d.idx) {
                                let mut virt_active = active_indices.clone();
                                virt_active.remove(cur_pos);
                                virt_active.insert(d.target, d.idx);
                                let mut y = 0.0f32;
                                for &item_idx in &virt_active {
                                    let h = rows[item_idx].1;
                                    if item_idx != d.idx {
                                        self.anim[item_idx].pos.target = y;
                                    }
                                    y += h + GAP;
                                }
                            }
                        }
                        self.todo_drag = Some(d);
                    } else if d.live {
                        if let Some(cur_pos) = active_indices.iter().position(|&j| j == d.idx) {
                            let target_slot = d.target.min(active_indices.len() - 1);
                            let to_todo_idx = active_indices[target_slot];
                            dropped = Some((d.idx, to_todo_idx));
                            let mut virt_active = active_indices.clone();
                            virt_active.remove(cur_pos);
                            virt_active.insert(d.target, d.idx);
                            let mut y = 0.0f32;
                            for &item_idx in &virt_active {
                                let h = rows[item_idx].1;
                                self.anim[item_idx].pos.target = y;
                                y += h + GAP;
                            }
                        }
                    }
                }
            }

            let mut live_drag_idx = self.todo_drag.as_ref().filter(|d| d.live).map(|d| d.idx);
            if live_drag_idx.is_none() {
                live_drag_idx = dropped.map(|(from, _)| from);
            }

            let mut draw_order_left = active_indices;
            if let Some(drag_idx) = live_drag_idx {
                if let Some(pos) = draw_order_left.iter().position(|&j| j == drag_idx) {
                    draw_order_left.remove(pos);
                    draw_order_left.push(drag_idx);
                }
            }

            for i in draw_order_left {
                let (c, d, e) = self.render_row(ui, cv, i, view_left, off_left, rows[i], text_w_left, row_h, 2, live_drag_idx == Some(i), self.todo_sort == Sort::Manual, live_drag_idx);
                if c { toggled = Some(i); }
                if d { removed = Some(i); }
                if e { edit_req = Some(i); }
            }
            if active_count == 0 {
                cv.text(crate::i18n::pick("Нет активных", "No active tasks"), Rect::new(view_left.x + 4.0, view_left.y + 6.0, view_left.w - 8.0, 14.0), &f.small, dim());
            }
            ui.scroll_end(cv, sid_left);

            let sid_right = id(TODAY, ID_LIST_DONE);
            let off_right = ui.scroll_begin(cv, sid_right, view_right, total_right);
            let draw_order_right: Vec<usize> = self.todo.iter().enumerate().filter(|(_, (d, _))| *d).map(|(j, _)| j).collect();

            for i in draw_order_right {
                let (c, d, e) = self.render_row(ui, cv, i, view_right, off_right, rows[i], text_w_right, row_h, 2, false, false, live_drag_idx);
                if c { toggled = Some(i); }
                if d { removed = Some(i); }
                if e { edit_req = Some(i); }
            }
            if done == 0 {
                cv.text(crate::i18n::pick("Пока ничего", "Nothing yet"), Rect::new(view_right.x + 4.0, view_right.y + 6.0, view_right.w - 8.0, 14.0), &f.small, dim());
            }
            ui.scroll_end(cv, sid_right);

            if let Some((from, to)) = dropped {
                if from != to {
                    self.reorder(from, to);
                    self.save();
                }
            }
        } else {
            // --- ОДНА КОЛОНКА (4x2, 4x3, 4x4) ---
            let (row_h, max_lines) = if is_compact { (25.0, 2) } else { (22.0, 3) };

            let view_y = if is_compact {
                card.y + 36.0
            } else {
                card.y + 64.0
            };
            let bottom_limit = if is_tall {
                card.bottom() - 24.0
            } else {
                card.bottom() - 6.0
            };
            let avail = (bottom_limit - view_y).max(row_h);
            let whole = ((avail + GAP) / (row_h + GAP)).floor().max(1.0);
            let view_h = whole * (row_h + GAP) - GAP;
            let view = Rect::new(card.x + 10.0, view_y, card.w - 20.0, view_h);
            let text_w = (view.w - 4.0 - 27.0 - 4.0).max(20.0);

            let mut rows: Vec<(f32, f32, usize, f32)> = Vec::with_capacity(self.todo.len());
            let mut total = 0.0f32;
            for (_, text) in &self.todo {
                let (lines, lh) = cv.wrapped(text, &f.wrap_l, text_w);
                let lines = lines.clamp(1, max_lines);
                let h = (lines as f32 * lh + 11.0).max(row_h);
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

            let sid = id(TODAY, ID_LIST);
            let off = ui.scroll_begin(cv, sid, view, total);

            let mut dropped = None;
            if let Some(mut d) = self.todo_drag.take() {
                if d.idx < self.todo.len() && self.todo_sort == Sort::Manual {
                    let drag_id = id(TODAY, ID_ROW + d.idx as u32);
                    let ptr = ui.pointer();
                    if ui.is_dragging(drag_id) {
                        if !d.live && (ptr.0 - d.press_pos.0).hypot(ptr.1 - d.press_pos.1) > 4.0 {
                            d.live = true;
                        }
                        if d.live {
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

            for i in draw_order {
                let (c, d, e) = self.render_row(ui, cv, i, view, off, rows[i], text_w, row_h, max_lines, live_drag_idx == Some(i), self.todo_sort == Sort::Manual, live_drag_idx);
                if c { toggled = Some(i); }
                if d { removed = Some(i); }
                if e { edit_req = Some(i); }
            }
            if self.todo.is_empty() {
                cv.text(crate::i18n::pick("Список пуст", "List is empty"), Rect::new(view.x + 4.0, view.y + 6.0, view.w - 8.0, 14.0), &f.small, dim());
            }
            ui.scroll_end(cv, sid);

            if let Some((from, to)) = dropped {
                if from != to {
                    self.reorder(from, to);
                    self.save();
                }
            }
        }

        // 5. Применение изменений по строкам
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
                    if is_compact {
                        ui.focus = None;
                    }
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
        } else if let Some(i) = toggled {
            self.todo[i].0 = !self.todo[i].0;
            if let Some(a) = self.anim.get_mut(i) {
                a.done.target = self.todo[i].0 as u8 as f32;
            }
            self.resort();
            self.save();
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
