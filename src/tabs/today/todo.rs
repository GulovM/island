//! Список задач: хранение в `todo.txt`, анимация строк, карточка задач.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

use super::Today;
use crate::anim::Spring;
use crate::tabs::{id, read_lines, write_lines, TODAY};
use crate::ui::{
    dim, fg, icon, inset, mix, status_blue, status_orange, status_red, track, txt,
    Align, Canvas, Corner, Rect, Submit, Ui,
};

const MAX_TODO: usize = 100;

// ID задач на вкладке: поле ввода и кнопки, прокрутка обеих колонок и по `MAX_TODO` на строки, удаление, правку и важность.
const ID_FIELD: u32 = 30;
const ID_ADD: u32 = 31;
const ID_CANCEL: u32 = 32;
const ID_SORT: u32 = 33;
const ID_CLEAR: u32 = 34;
const ID_LIST_DONE: u32 = 35;
const ID_FIELD_PRIO: u32 = 36;
const ID_LIST: u32 = 100;
const ID_ROW: u32 = 200;
const ID_DEL: u32 = 400;
const ID_EDIT: u32 = 600;
const ID_PRIO: u32 = 1100;
const ID_PRIO_BADGE: u32 = 1200;

/// Занятые задачами ID вкладки (полуинтервалы) для теста непересечения в `mod.rs`.
#[cfg(test)]
pub(super) const ID_RANGES: &[(u32, u32)] = &[
    (ID_FIELD, ID_FIELD_PRIO + 1),
    (ID_LIST, ID_LIST + 1),
    (ID_ROW, ID_ROW + MAX_TODO as u32),
    (ID_DEL, ID_DEL + MAX_TODO as u32),
    (ID_EDIT, ID_EDIT + MAX_TODO as u32),
    (ID_PRIO, ID_PRIO + MAX_TODO as u32),
    (ID_PRIO_BADGE, ID_PRIO_BADGE + MAX_TODO as u32),
];

const TODO_FILE: &str = "todo.txt";

/// Уровень важности задачи.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Priority {
    #[default]
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

impl Priority {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Low,
            2 => Self::Medium,
            3 => Self::High,
            _ => Self::None,
        }
    }

    pub(super) fn next(self) -> Self {
        match self {
            Self::None => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::None,
        }
    }

    fn glyph(self) -> &'static str {
        if self == Self::None {
            icon::FLAG
        } else {
            icon::FLAG_FILL
        }
    }

    fn color(self) -> D2D1_COLOR_F {
        match self {
            Self::None => dim(),
            Self::Low => status_blue(),
            Self::Medium => status_orange(),
            Self::High => status_red(),
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Self::None => crate::i18n::pick("Важность: обычная", "Importance: normal"),
            Self::Low => crate::i18n::pick("Важность: низкая", "Importance: low"),
            Self::Medium => crate::i18n::pick("Важность: средняя", "Importance: medium"),
            Self::High => crate::i18n::pick("Важность: высокая", "Importance: high"),
        }
    }
}

/// Сортировка списка задач. «Вручную» порядок не трогает (новые добавляются сверху).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Sort {
    #[default]
    Manual,
    /// Новые сверху; задачи без даты (созданные в старых версиях) в конце.
    Date,
    /// Невыполненные сверху, внутри групп порядок сохраняется.
    Status,
    /// Невыполненные важные сверху (Высокая → Средняя → Низкая → Обычная).
    Priority,
}

impl Sort {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Date => "date",
            Self::Status => "status",
            Self::Priority => "priority",
        }
    }

    fn parse(s: &str) -> Self {
        match s.trim() {
            "date" => Self::Date,
            "status" => Self::Status,
            "priority" => Self::Priority,
            _ => Self::Manual,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Manual => Self::Date,
            Self::Date => Self::Status,
            Self::Status => Self::Priority,
            Self::Priority => Self::Manual,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Manual => crate::i18n::pick("Вручную", "Manual"),
            Self::Date => crate::i18n::pick("По дате", "By date"),
            Self::Status => crate::i18n::pick("По статусу", "By status"),
            Self::Priority => crate::i18n::pick("По важности", "By priority"),
        }
    }
}

/// Новый порядок индексов для `items` (выполнено, время создания, важность) по сортировке; сортировка устойчивая.
fn sorted_order(sort: Sort, items: &[(bool, u64, Priority)]) -> Vec<usize> {
    let mut ord: Vec<usize> = (0..items.len()).collect();
    match sort {
        Sort::Manual => {}
        // Ключ — убывание времени создания; задачи без даты (0) оказываются в конце.
        Sort::Date => ord.sort_by_key(|&i| std::cmp::Reverse(items[i].1)),
        Sort::Status => ord.sort_by_key(|&i| items[i].0),
        Sort::Priority => ord.sort_by_key(|&i| (items[i].0, std::cmp::Reverse(items[i].2))),
    }
    ord
}

type TodoEntry = (bool, String, u64, Priority);
type RowLayout = (f32, f32, usize, f32, f32, bool);

/// Разбор `todo.txt`: необязательные строки `#sort=…` и `#day=…` (день последней проверки),
/// затем `выполнено[@время][!важность]|текст`.
fn parse_todo(lines: &[String]) -> (Sort, i64, Vec<TodoEntry>) {
    let sort = lines.iter().find_map(|l| l.strip_prefix("#sort=")).map_or(Sort::Manual, Sort::parse);
    let day = lines.iter().find_map(|l| l.strip_prefix("#day=")).and_then(|d| d.trim().parse().ok()).unwrap_or(0);
    let items = lines
        .iter()
        .filter_map(|l| l.split_once('|'))
        .map(|(head, t)| {
            let (before_prio, p_str) = head.split_once('!').unwrap_or((head, ""));
            let prio = p_str.trim().parse::<u8>().map(Priority::from_u8).unwrap_or(Priority::None);
            let (d, ts) = before_prio.split_once('@').unwrap_or((before_prio, ""));
            (d == "1", t.to_string(), ts.parse().unwrap_or(0), prio)
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

fn retain_mask<T>(v: &mut Vec<T>, keep: &[bool]) {
    let mut i = 0;
    v.retain(|_| {
        let k = keep.get(i).copied().unwrap_or(true);
        i += 1;
        k
    });
}

impl Today {
    pub fn load(&mut self) {
        self.todo_drag = None;
        let (sort, day, items) = parse_todo(&read_lines(TODO_FILE));
        self.todo_sort = sort;
        self.todo_day = day;
        self.todo_ts = items.iter().map(|i| i.2).collect();
        self.todo_prio = items.iter().map(|i| i.3).collect();
        self.todo = items.into_iter().map(|(d, t, _, _)| (d, t)).collect();
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
        lines.extend(self.todo.iter().enumerate().map(|(i, (d, t))| {
            let done = if *d { 1 } else { 0 };
            let ts = self.todo_ts.get(i).copied().unwrap_or(0);
            let prio = self.todo_prio.get(i).copied().unwrap_or(Priority::None);
            if prio == Priority::None {
                format!("{done}@{ts}|{t}")
            } else {
                format!("{done}@{ts}!{}|{t}", prio as u8)
            }
        }));
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
        retain_mask(&mut self.todo, &keep);
        retain_mask(&mut self.todo_ts, &keep);
        retain_mask(&mut self.todo_prio, &keep);
        retain_mask(&mut self.anim, &keep);
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
        let items: Vec<(bool, u64, Priority)> = (0..self.todo.len())
            .map(|i| {
                (
                    self.todo[i].0,
                    self.todo_ts.get(i).copied().unwrap_or(0),
                    self.todo_prio.get(i).copied().unwrap_or(Priority::None),
                )
            })
            .collect();
        let ord = sorted_order(self.todo_sort, &items);
        if ord.iter().enumerate().all(|(n, &i)| n == i) || self.anim.len() != self.todo.len() {
            return;
        }
        let mut todo: Vec<Option<(bool, String)>> = std::mem::take(&mut self.todo).into_iter().map(Some).collect();
        let mut anim: Vec<Option<RowAnim>> = std::mem::take(&mut self.anim).into_iter().map(Some).collect();
        let ts = std::mem::take(&mut self.todo_ts);
        let prio = std::mem::take(&mut self.todo_prio);
        for &i in &ord {
            self.todo.extend(todo[i].take());
            self.anim.extend(anim[i].take());
            self.todo_ts.push(ts.get(i).copied().unwrap_or(0));
            self.todo_prio.push(prio.get(i).copied().unwrap_or(Priority::None));
        }
        self.edit = self.edit.and_then(|e| ord.iter().position(|&i| i == e));
    }

    /// Переставляет задачу с позиции `from` на позицию `to`, синхронно сдвигая
    /// параллельные векторы `todo`, `todo_ts`, `todo_prio`, `anim` и корректируя `edit`.
    pub(super) fn reorder(&mut self, from: usize, to: usize) {
        let n = self.todo.len();
        if from >= n || to >= n || from == to {
            return;
        }
        if self.todo_ts.len() != n || self.todo_prio.len() != n || self.anim.len() != n {
            return;
        }
        let item = self.todo.remove(from);
        self.todo.insert(to, item);

        let ts = self.todo_ts.remove(from);
        self.todo_ts.insert(to, ts);

        let prio = self.todo_prio.remove(from);
        self.todo_prio.insert(to, prio);

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

    /// Отрисовка поля ввода задачи (пилюля) и кнопок важности/отмены/отправки внутри него.
    unsafe fn paint_field(&mut self, ui: &mut Ui, cv: &Canvas, field: Rect) -> Option<String> {
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
        let prio_r = Rect::new(cancel.x - 3.0 - bs, add.y, bs, bs);

        let pid = id(TODAY, ID_FIELD_PRIO);
        let cid = id(TODAY, ID_CANCEL);
        let aid = id(TODAY, ID_ADD);

        let prio_clicked = ui.click_area(pid, prio_r).1;
        let cancel_clicked = ui.click_area(cid, cancel).1;
        let add_clicked = ui.click_area(aid, add).1;
        let right = 3.0 * bs + 16.0;

        // Удержание нажатия или клик по кнопке внутри пилюли не должны сбрасывать фокус поля.
        if ui.is_dragging(pid) || ui.is_dragging(cid) || ui.is_dragging(aid) || prio_clicked {
            ui.claim_focus(fid);
        }

        ui.text_field_at(cv, fid, field, hint, glyph, Submit::Clear, left, right);
        let focus = ui.on_of(fid);
        cv.rr_line(field, field.h / 2.0, mix(track(), cv.accent, if editing { 1.0 } else { focus }), 1.0);

        let p_hv = ui.hover_of(pid);
        let p_col = self.todo_draft_prio.color();
        if self.todo_draft_prio != Priority::None {
            cv.circle(prio_r.cx(), prio_r.cy(), bs / 2.0, D2D1_COLOR_F { a: 0.22 + 0.14 * p_hv, ..p_col });
            cv.glyph(self.todo_draft_prio.glyph(), prio_r.inset(2.0), false, mix(p_col, txt(), 0.12 * p_hv));
        } else {
            let idle = if crate::theme::glass() { mix(dim(), txt(), 0.28) } else { dim() };
            cv.circle(prio_r.cx(), prio_r.cy(), bs / 2.0, D2D1_COLOR_F { a: 0.10 * p_hv, ..fg() });
            cv.glyph(icon::FLAG, prio_r.inset(2.0), false, mix(idle, txt(), p_hv));
        }
        ui.tip(pid, prio_r, self.todo_draft_prio.tip());

        let c_hv = ui.hover_of(cid);
        cv.circle(cancel.cx(), cancel.cy(), bs / 2.0, D2D1_COLOR_F { a: 0.10 * c_hv, ..fg() });
        cv.glyph(icon::CLOSE, cancel.inset(2.0), false, mix(dim(), txt(), c_hv));

        let hv = ui.hover_of(aid);
        cv.circle(add.cx(), add.cy(), bs / 2.0, D2D1_COLOR_F { a: 0.16 + 0.84 * hv, ..cv.accent });
        let arrow = if editing { icon::CHECK } else { icon::ADD };
        cv.glyph(arrow, add.inset(2.0), false, mix(cv.accent, crate::ui::on_color(cv.accent), hv));

        if prio_clicked {
            self.todo_draft_prio = self.todo_draft_prio.next();
            ui.claim_focus(fid);
        }

        if cancel_clicked {
            let _ = ui.submit_field(fid);
            self.edit = None;
            self.todo_adding = false;
            self.todo_draft_prio = Priority::None;
            ui.focus = None;
            return None;
        }

        let mut submitted = ui.take_submitted(fid);
        if add_clicked {
            submitted = ui.submit_field(fid);
        }
        submitted
    }

    const ROW_LEFT_PAD: f32 = 30.0;
    const ACTION_BTN_W: f32 = 17.0;
    const ACTION_BTN_H: f32 = 18.0;
    const ACTIONS_RIGHT_PAD: f32 = 3.0;
    const TEXT_ACTIONS_GAP: f32 = 4.0;
    const MIN_SIDE_TEXT_W: f32 = 54.0;

    fn row_actions_visible(ui: &mut Ui, i: usize, has_live_drag: bool) -> bool {
        if has_live_drag {
            return false;
        }
        let rid = id(TODAY, ID_ROW + i as u32);
        let pid = id(TODAY, ID_PRIO + i as u32);
        let eid = id(TODAY, ID_EDIT + i as u32);
        let did = id(TODAY, ID_DEL + i as u32);
        ui.hover_of(rid) > 0.3 || ui.hover_of(did) > 0.3 || ui.hover_of(eid) > 0.3 || ui.hover_of(pid) > 0.3
    }

    fn text_wrap_width(base_text_w: f32, show_actions: bool) -> (f32, bool) {
        let reserve = 3.0 * Self::ACTION_BTN_W + Self::ACTIONS_RIGHT_PAD + Self::TEXT_ACTIONS_GAP - 4.0;
        let side = !show_actions || (base_text_w - reserve) >= Self::MIN_SIDE_TEXT_W;
        let wrap_w = if show_actions && side { base_text_w - reserve } else { base_text_w }.max(16.0);
        (wrap_w, side)
    }

    /// Измеряет высоту строки и ширину переноса текста с учётом контейнера иконок.
    /// Возвращает `(y_offset, row_height, lines, line_height, wrap_w, actions_side)`.
    #[allow(clippy::too_many_arguments)]
    unsafe fn measure_row(
        cv: &Canvas,
        text: &str,
        f: &IDWriteTextFormat,
        base_text_w: f32,
        row_h: f32,
        max_lines: usize,
        y_acc: f32,
        show_actions: bool,
    ) -> RowLayout {
        let (wrap_w, actions_side) = Self::text_wrap_width(base_text_w, show_actions);
        let (lines, lh) = cv.wrapped(text, f, wrap_w);
        let lines = lines.clamp(1, max_lines);
        let top_pad = ((row_h - lh) / 2.0).max(4.5);
        let text_block_h = (lines as f32 * lh + 2.0 * top_pad).max(row_h);
        let extra_h = if show_actions && !actions_side { Self::ACTION_BTN_H + 4.0 } else { 0.0 };
        (y_acc, text_block_h + extra_h, lines, lh, wrap_w, actions_side)
    }

    /// Отрисовка одной строки задачи: полоска важности, чекбокс, текст и единый контейнер кнопок.
    #[allow(clippy::too_many_arguments)]
    unsafe fn render_row(
        &mut self,
        ui: &mut Ui,
        cv: &Canvas,
        i: usize,
        view: Rect,
        off: f32,
        row_info: (f32, f32, usize, f32, f32, bool),
        row_h: f32,
        max_lines: usize,
        show_actions: bool,
        is_dragged: bool,
        can_drag: bool,
        live_drag_idx: Option<usize>,
    ) -> (bool, bool, bool, bool) {
        let is_done = self.todo[i].0;
        let text = &self.todo[i].1;
        let prio = self.todo_prio.get(i).copied().unwrap_or(Priority::None);
        let (_, rh, lines, lh, row_text_w, actions_side) = row_info;
        let (fade, dn) = (self.anim[i].fade.value.clamp(0.0, 1.0), self.anim[i].done.value.clamp(0.0, 1.0));
        let rcv = Canvas { g: cv.g, alpha: cv.alpha * fade, accent: cv.accent };
        let cv = &rcv;
        let y = view.y + self.anim[i].pos.value - off;
        if !is_dragged && (y + rh < view.y || y > view.bottom()) {
            return (false, false, false, false);
        }
        let r = Rect::new(view.x, y, view.w - 4.0, rh);
        let rid = id(TODAY, ID_ROW + i as u32);
        let (pid, eid, did) = (
            id(TODAY, ID_PRIO + i as u32),
            id(TODAY, ID_EDIT + i as u32),
            id(TODAY, ID_DEL + i as u32),
        );
        let bid = id(TODAY, ID_PRIO_BADGE + i as u32);
        let show = show_actions && live_drag_idx.is_none();

        let actions_w = 3.0 * Self::ACTION_BTN_W;
        let container_y = if actions_side {
            r.y + (row_h - Self::ACTION_BTN_H) / 2.0
        } else {
            r.bottom() - Self::ACTION_BTN_H - 3.0
        };
        let actions_r = Rect::new(
            r.right() - Self::ACTIONS_RIGHT_PAD - actions_w,
            container_y,
            actions_w,
            Self::ACTION_BTN_H,
        );
        let prio_btn = Rect::new(actions_r.x, actions_r.y, Self::ACTION_BTN_W, Self::ACTION_BTN_H);
        let edit = Rect::new(actions_r.x + Self::ACTION_BTN_W, actions_r.y, Self::ACTION_BTN_W, Self::ACTION_BTN_H);
        let del = Rect::new(actions_r.x + 2.0 * Self::ACTION_BTN_W, actions_r.y, Self::ACTION_BTN_W, Self::ACTION_BTN_H);

        let mut prio_clicked = false;
        let mut edit_clicked = false;
        let mut del_clicked = false;
        if show {
            prio_clicked = ui.click_area(pid, prio_btn).1;
            edit_clicked = ui.click_area(eid, edit).1;
            del_clicked = ui.click_area(did, del).1;
        }

        let (cx, cy) = (r.x + 17.5, r.y + row_h / 2.0);
        if prio != Priority::None && live_drag_idx.is_none() {
            let bar_hit = Rect::new(r.x + 1.5, cy - 9.0, 8.5, 18.0);
            prio_clicked |= ui.click_area(bid, bar_hit).1;
            ui.tip(bid, bar_hit, prio.tip());
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
        let hv = ui.hover_of(rid)
            .max(ui.hover_of(pid))
            .max(ui.hover_of(eid))
            .max(ui.hover_of(did));
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

        // Контрастная цветная полоска важности перед кружочком чекбокса
        if prio != Priority::None {
            let mut bar_col = prio.color();
            bar_col.a *= 1.0 - 0.35 * dn;
            cv.rr(Rect::new(r.x + 4.5, cy - 7.0, 3.0, 14.0), 1.5, bar_col);
        }

        // Круглый чекбокс: кольцо dim при невыполненном, залитый cv.accent круг с галочкой при выполнении
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
        let text_area_h = if show && !actions_side { (rh - (Self::ACTION_BTN_H + 4.0)).max(row_h) } else { rh };
        let tr = Rect::new(r.x + Self::ROW_LEFT_PAD, r.y + (text_area_h - th) / 2.0, row_text_w, th + 2.0);
        let shown = cv.wrap_clip(text, &f.wrap_l, row_text_w, max_lines);
        cv.text_strike(&shown, tr, &f.wrap_l, mix(txt(), dim(), dn), is_done);

        if show {
            // Единый контейнер-капсула для кнопок действий; форма подложки сегмента зависит от его края
            let glass = crate::theme::glass();
            let mut box_bg = mix(inset(), fg(), if glass { 0.08 } else { 0.05 });
            box_bg.a = (box_bg.a + if glass { 0.14 } else { 0.06 }) * hv.clamp(0.4, 1.0);
            let rad = Self::ACTION_BTN_H / 2.0;
            cv.rr(actions_r, rad, box_bg);

            let rem = crate::ui::hue(crate::ios::SysColor::Blue, cv.accent);
            let prio_col = prio.color();
            let (flag_base, flag_hot) = if prio != Priority::None {
                (prio_col, mix(prio_col, txt(), 0.14))
            } else {
                let idle = if glass { mix(dim(), txt(), 0.28) } else { dim() };
                (idle, txt())
            };
            let btns = [
                (pid, prio_btn, prio.glyph(), false, flag_base, flag_hot),
                (eid, edit, icon::EDIT, self.edit == Some(i), dim(), txt()),
                (did, del, icon::DELETE, false, dim(), status_red()),
            ];
            let n = btns.len();
            for (idx, (bid_btn, rect, glyph, active, base_c, hot_c)) in btns.into_iter().enumerate() {
                let h = ui.hover_of(bid_btn);
                if h > 0.001 {
                    let rl = if idx == 0 { rad } else { 0.0 };
                    let rr = if idx + 1 == n { rad } else { 0.0 };
                    cv.rr_ends(rect, rl, rr, D2D1_COLOR_F { a: if glass { 0.20 * h } else { 0.14 * h }, ..fg() });
                }
                cv.glyph(glyph, rect, false, if active { rem } else { mix(base_c, hot_c, h) });
            }
            ui.tip(pid, prio_btn, prio.tip());
        }

        (clicked && !prio_clicked && !edit_clicked && !del_clicked, del_clicked, edit_clicked, prio_clicked)
    }

    pub(super) unsafe fn todo_card(&mut self, ui: &mut Ui, cv: &Canvas, card: Rect) {
        self.roll_day();
        cv.card(card);
        cv.glow(card, 14.0, cv.accent, Corner::TopRight);

        let f = &cv.g.fonts;
        let done = self.todo.iter().filter(|(d, _)| *d).count();
        let fid = id(TODAY, ID_FIELD);
        self.todo_prio.resize(self.todo.len(), Priority::None);
        let editing = self.edit.is_some_and(|i| i < self.todo.len());
        if !editing {
            self.edit = None;
        }
        // Закрываем скрываемое поле ввода при уходе фокуса, но не посреди нажатия на строку.
        if (self.todo_adding || self.edit.is_some()) && !ui.down && !ui.released && ui.focus != Some(fid) {
            self.todo_adding = false;
            self.edit = None;
            self.todo_draft_prio = Priority::None;
            let _ = ui.submit_field(fid);
        }
        let show_field = self.todo_adding || self.edit.is_some();

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
            if show_field {
                let field = Rect::new(card.x + 10.0, card.y + 9.0, card.w - 20.0, 24.0);
                submitted = self.paint_field(ui, cv, field);
            } else {
                let title = crate::i18n::pick("ЗАДАЧИ", "TASKS");
                let tw = cv.text_w(title, &f.caps);
                cv.line(title, &f.caps, card.x + 12.0, tw + 2.0, card.y + 21.0, Align::Leading, dim());

                let count_s = format!("{done} {} {}", crate::i18n::pick("из", "of"), self.todo.len());
                let count_w = cv.text_w(&count_s, &f.small);
                cv.line(&count_s, &f.small, card.x + 12.0 + tw + 8.0, count_w + 2.0, card.y + 21.0, Align::Leading, dim());

                let btn_sz = 18.0;
                let btn_r = Rect::new(card.right() - 12.0 - btn_sz, card.y + 12.0, btn_sz, btn_sz);

                // Чип сортировки: если места в шапке не хватает, не рисуется.
                let sort_label = self.todo_sort.label();
                let sort_w = cv.chip_w(sort_label);
                let sort_r = Rect::new(card.x + 12.0 + tw + 8.0 + count_w + 8.0, card.y + 12.5, sort_w, 17.0);
                if sort_r.right() < btn_r.x - 6.0 && ui.chip(cv, id(TODAY, ID_SORT), sort_r, sort_label, self.todo_sort != Sort::Manual) {
                    self.todo_sort = self.todo_sort.next();
                    self.resort();
                    self.save();
                }
                if ui.icon_button(cv, id(TODAY, ID_ADD), btn_r, icon::ADD, false, crate::i18n::pick("Добавить задачу", "Add task")) {
                    self.todo_adding = true;
                    self.todo_draft_prio = Priority::None;
                    let _ = ui.submit_field(fid);
                    ui.claim_focus(fid);
                }
            }
        } else if is_wide {
            // 6x4 (328x216): слева три строки (ЗАДАЧИ, счётчик 16px semibold, прогресс 120x4),
            // справа вверху кнопка "+ Добавить" или поле ввода при добавлении/правке.
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

            if show_field {
                let fw = 166.0;
                let field = Rect::new(card.right() - 14.0 - fw, card.y + 14.0, fw, 26.0);
                submitted = self.paint_field(ui, cv, field);
            } else {
                let add_lbl = crate::i18n::pick("Добавить", "Add");
                let lw = cv.text_w(add_lbl, &f.small_c);
                let pw = (lw + 32.0).max(74.0);
                let pill_r = Rect::new(card.right() - 14.0 - pw, card.y + 14.0, pw, 24.0);
                let (_, add_clicked) = ui.click_area(id(TODAY, ID_ADD), pill_r);
                let hv = ui.hover_of(id(TODAY, ID_ADD));
                cv.rr(pill_r, pill_r.h / 2.0, mix(crate::ui::chip_bg(), fg(), 0.10 * hv));
                let fg_col = mix(dim(), txt(), 0.7 + 0.3 * hv);
                cv.glyph(icon::ADD, Rect::new(pill_r.x + 7.0, pill_r.y + 4.0, 16.0, 16.0), false, fg_col);
                cv.line(add_lbl, &f.small_c, pill_r.x + 24.0, pw - 30.0, pill_r.cy(), Align::Leading, fg_col);
                if add_clicked {
                    self.todo_adding = true;
                    self.todo_draft_prio = Priority::None;
                    let _ = ui.submit_field(fid);
                    ui.claim_focus(fid);
                }
            }
        } else {
            // 4x3 и 4x4: шапка "ЗАДАЧИ", чип сортировки, счётчик и кнопка "+".
            // Поле ввода раскрывается под шапкой только при добавлении или правке.
            let title = crate::i18n::pick("ЗАДАЧИ", "TASKS");
            let tw = cv.text_w(title, &f.caps);
            cv.line(title, &f.caps, card.x + 12.0, tw + 2.0, card.y + 19.0, Align::Leading, dim());

            let btn_sz = 18.0;
            let btn_r = Rect::new(card.right() - 12.0 - btn_sz, card.y + 10.0, btn_sz, btn_sz);

            let count_s = format!("{done} {} {}", crate::i18n::pick("из", "of"), self.todo.len());
            let count_w = cv.text_w(&count_s, &f.small);
            let count_right = if show_field { card.right() - 12.0 } else { btn_r.x - 6.0 };
            let count_x = count_right - count_w;
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

            if show_field {
                let field = Rect::new(card.x + 10.0, card.y + 32.0, card.w - 20.0, 26.0);
                submitted = self.paint_field(ui, cv, field);
            } else {
                if ui.icon_button(cv, id(TODAY, ID_ADD), btn_r, icon::ADD, false, crate::i18n::pick("Добавить задачу", "Add task")) {
                    self.todo_adding = true;
                    self.todo_draft_prio = Priority::None;
                    let _ = ui.submit_field(fid);
                    ui.claim_focus(fid);
                }
            }
        }

        // 2. Обработка отправки текста из поля ввода
        if let Some(text) = submitted {
            let text = text.trim();
            let draft_prio = self.todo_draft_prio;
            match self.edit.take() {
                Some(i) if !text.is_empty() && i < self.todo.len() => {
                    self.todo[i].1 = text.to_string();
                    if i < self.todo_prio.len() {
                        self.todo_prio[i] = draft_prio;
                    }
                    self.resort();
                    self.save();
                }
                Some(_) => {}
                None if !text.is_empty() && self.todo.len() < MAX_TODO => {
                    self.todo.insert(0, (false, text.to_string()));
                    self.todo_ts.insert(0, crate::sys::now_secs());
                    self.todo_prio.insert(0, draft_prio);
                    self.anim.insert(0, RowAnim::new(false, true));
                    self.todo_drag = None;
                    self.resort();
                    self.save();
                }
                None => {}
            }
            self.todo_adding = false;
            self.todo_draft_prio = Priority::None;
            ui.focus = None;
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
        let mut prio_req = None;
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

            let text_w_left = (view_left.w - 4.0 - Self::ROW_LEFT_PAD - 4.0).max(20.0);
            let text_w_right = (view_right.w - 4.0 - Self::ROW_LEFT_PAD - 4.0).max(20.0);
            let row_h = 24.0;

            let drag_live = self.todo_drag.as_ref().is_some_and(|d| d.live);
            let row_show: Vec<bool> = (0..self.todo.len())
                .map(|i| Self::row_actions_visible(ui, i, drag_live))
                .collect();
            let mut rows: Vec<RowLayout> = vec![(0.0, 0.0, 0, 0.0, 20.0, true); self.todo.len()];
            let mut total_left = 0.0f32;
            let mut total_right = 0.0f32;

            for (i, (d, text)) in self.todo.iter().enumerate() {
                if !*d {
                    let row = Self::measure_row(cv, text, &f.wrap_l, text_w_left, row_h, 2, total_left, row_show[i]);
                    total_left += row.1 + GAP;
                    rows[i] = row;
                } else {
                    let row = Self::measure_row(cv, text, &f.wrap_l, text_w_right, row_h, 2, total_right, row_show[i]);
                    total_right += row.1 + GAP;
                    rows[i] = row;
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
                let (c, d, e, p) = self.render_row(ui, cv, i, view_left, off_left, rows[i], row_h, 2, row_show[i], live_drag_idx == Some(i), self.todo_sort == Sort::Manual, live_drag_idx);
                if c { toggled = Some(i); }
                if d { removed = Some(i); }
                if e { edit_req = Some(i); }
                if p { prio_req = Some(i); }
            }
            if active_count == 0 {
                cv.text(crate::i18n::pick("Нет активных", "No active tasks"), Rect::new(view_left.x + 4.0, view_left.y + 6.0, view_left.w - 8.0, 14.0), &f.small, dim());
            }
            ui.scroll_end(cv, sid_left);

            let sid_right = id(TODAY, ID_LIST_DONE);
            let off_right = ui.scroll_begin(cv, sid_right, view_right, total_right);
            let draw_order_right: Vec<usize> = self.todo.iter().enumerate().filter(|(_, (d, _))| *d).map(|(j, _)| j).collect();

            for i in draw_order_right {
                let (c, d, e, p) = self.render_row(ui, cv, i, view_right, off_right, rows[i], row_h, 2, row_show[i], false, false, live_drag_idx);
                if c { toggled = Some(i); }
                if d { removed = Some(i); }
                if e { edit_req = Some(i); }
                if p { prio_req = Some(i); }
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

            let view_y = if is_compact || !show_field {
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
            let text_w = (view.w - 4.0 - Self::ROW_LEFT_PAD - 4.0).max(20.0);

            let drag_live = self.todo_drag.as_ref().is_some_and(|d| d.live);
            let row_show: Vec<bool> = (0..self.todo.len())
                .map(|i| Self::row_actions_visible(ui, i, drag_live))
                .collect();
            let mut rows: Vec<RowLayout> = Vec::with_capacity(self.todo.len());
            let mut total = 0.0f32;
            for (i, (_, text)) in self.todo.iter().enumerate() {
                let row = Self::measure_row(cv, text, &f.wrap_l, text_w, row_h, max_lines, total, row_show[i]);
                total += row.1 + GAP;
                rows.push(row);
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
                let (c, d, e, p) = self.render_row(ui, cv, i, view, off, rows[i], row_h, max_lines, row_show[i], live_drag_idx == Some(i), self.todo_sort == Sort::Manual, live_drag_idx);
                if c { toggled = Some(i); }
                if d { removed = Some(i); }
                if e { edit_req = Some(i); }
                if p { prio_req = Some(i); }
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
            if i < self.todo_prio.len() {
                self.todo_prio.remove(i);
            }
            if i < self.anim.len() {
                self.anim.remove(i);
            }
            match self.edit {
                Some(e) if e == i => {
                    let _ = ui.submit_field(fid);
                    self.edit = None;
                    self.todo_draft_prio = Priority::None;
                    ui.focus = None;
                }
                Some(e) if e > i => self.edit = Some(e - 1),
                _ => {}
            }
            self.todo_drag = None;
            self.save();
        } else if let Some(i) = edit_req {
            self.edit = Some(i);
            self.todo_adding = false;
            self.todo_draft_prio = self.todo_prio.get(i).copied().unwrap_or(Priority::None);
            ui.set_field_text(fid, &self.todo[i].1);
            ui.claim_focus(fid);
        } else if let Some(i) = prio_req {
            if i < self.todo_prio.len() {
                self.todo_prio[i] = self.todo_prio[i].next();
                if self.edit == Some(i) {
                    self.todo_draft_prio = self.todo_prio[i];
                }
                self.resort();
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
            "0@1700000005!3|Срочная задача".to_string(),
            "0!2|Средняя без даты".to_string(),
            "invalid_line".to_string(),
            "".to_string(),
        ];
        let (sort, day, items) = parse_todo(&lines);
        assert_eq!(sort, Sort::Status);
        assert_eq!(day, 0);
        assert_eq!(items, vec![
            (false, "Купить молоко".to_string(), 0, Priority::None),
            (true, "Сдать отчёт | срочно".to_string(), 1_700_000_000, Priority::None),
            (false, "Срочная задача".to_string(), 1_700_000_005, Priority::High),
            (false, "Средняя без даты".to_string(), 0, Priority::Medium),
        ]);
        // Без заголовка (файл старой версии) — ручной порядок.
        assert_eq!(parse_todo(&["0|a".to_string()]).0, Sort::Manual);
        assert_eq!(parse_todo(&["#day=20750".to_string()]).1, 20750);
        assert_eq!(Sort::parse("мусор"), Sort::Manual);
    }

    #[test]
    fn sort_by_date_puts_undated_last() {
        // (выполнено, время, важность): новые сверху, без даты в конце, равные не переставляются.
        let items = [
            (false, 0, Priority::None),
            (false, 100, Priority::None),
            (true, 300, Priority::None),
            (false, 200, Priority::None),
            (false, 0, Priority::None),
        ];
        assert_eq!(sorted_order(Sort::Date, &items), vec![2, 3, 1, 0, 4]);
    }

    #[test]
    fn sort_by_status_keeps_order_inside_groups() {
        let items = [
            (true, 1, Priority::None),
            (false, 2, Priority::None),
            (true, 3, Priority::None),
            (false, 4, Priority::None),
        ];
        assert_eq!(sorted_order(Sort::Status, &items), vec![1, 3, 0, 2]);
        assert_eq!(sorted_order(Sort::Manual, &items), vec![0, 1, 2, 3]);
    }

    #[test]
    fn sort_by_priority_orders_active_high_to_low() {
        let items = [
            (false, 1, Priority::Low),
            (true, 2, Priority::High),
            (false, 3, Priority::High),
            (false, 4, Priority::None),
            (false, 5, Priority::Medium),
        ];
        assert_eq!(sorted_order(Sort::Priority, &items), vec![2, 4, 0, 3, 1]);
    }

    #[test]
    fn remove_done_keeps_open_tasks_and_edit_target() {
        let mut t = Today::default();
        t.todo = vec![(true, "a".into()), (false, "b".into()), (true, "c".into()), (false, "d".into())];
        t.todo_ts = vec![1, 2, 3, 4];
        t.todo_prio = vec![Priority::Low, Priority::High, Priority::None, Priority::Medium];
        t.anim = t.todo.iter().map(|(d, _)| RowAnim::new(*d, false)).collect();
        t.edit = Some(3);
        assert!(t.remove_done());
        assert_eq!(t.todo, vec![(false, "b".to_string()), (false, "d".to_string())]);
        assert_eq!(t.todo_ts, vec![2, 4]);
        assert_eq!(t.todo_prio, vec![Priority::High, Priority::Medium]);
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
        assert_eq!(Sort::Manual.next().next().next().next(), Sort::Manual);
        for s in [Sort::Manual, Sort::Date, Sort::Status, Sort::Priority] {
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
        t.todo_prio = vec![Priority::High, Priority::Low, Priority::Medium, Priority::None];
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
        assert_eq!(t.todo_prio, vec![Priority::Low, Priority::Medium, Priority::High, Priority::None]);
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
        assert_eq!(t.todo_prio, vec![Priority::High, Priority::Low, Priority::Medium, Priority::None]);
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
        assert_eq!(t.todo_prio, vec![Priority::High, Priority::None, Priority::Low, Priority::Medium]);
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
        assert_eq!(t.todo_prio.len(), 4);
        assert_eq!(t.anim.len(), 4);
    }

    #[test]
    fn wrap_width_avoids_action_container() {
        // Обычная карточка (4x2/4x3/4x4, base_text_w = 158): при показе кнопок текст сужается перед областью иконок
        let (idle_w, idle_side) = Today::text_wrap_width(158.0, false);
        let (hot_w, hot_side) = Today::text_wrap_width(158.0, true);
        assert_eq!((idle_w, idle_side), (158.0, true));
        assert!(hot_side);
        assert!(hot_w < idle_w && hot_w >= Today::MIN_SIDE_TEXT_W);

        // Узкая правая колонка 6x4 (base_text_w = 64): кнопки уходят на нижнюю полосу, не сжимая текст в нитку
        let (narrow_w, narrow_side) = Today::text_wrap_width(64.0, true);
        assert_eq!((narrow_w, narrow_side), (64.0, false));
    }

    #[test]
    fn priority_flag_switches_between_outline_and_filled() {
        assert_eq!(Priority::None.glyph(), icon::FLAG);
        assert_eq!(Priority::Low.glyph(), icon::FLAG_FILL);
        assert_eq!(Priority::Medium.glyph(), icon::FLAG_FILL);
        assert_eq!(Priority::High.glyph(), icon::FLAG_FILL);
    }
}
