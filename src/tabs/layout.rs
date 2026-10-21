//! Раскладка виджетов вкладки: чистая модель квадратной сетки без рисования.
//!
//! Вкладка делится на сетку из ячеек с шагом `CELL` (16 px) и зазором `GAP` (8 px).
//! Виджет размером `w`×`h` ячеек имеет пиксельный размер `w*CELL - GAP` на `h*CELL - GAP`
//! (то есть `16w - 8` на `16h - 8` px). Позиция каждого виджета задаётся явно координатами
//! ячейки `(col, row)` без вертикальной «гравитации»: пустые места между виджетами допустимы.
//!
//! Раскладка в `config.toml` — строка токенов `[-]ключ[:размер]@col,row` через пробел, например:
//! `wifi@0,0 bluetooth:1@8,0 -night:2@0,3`. Минус — виджет скрыт в лоток, номер размера — индекс
//! в `Spec::sizes`. Пустая строка означает раскладку по умолчанию.

use super::{PAD, TOP};

/// Шаг ячейки сетки, логических пикселей.
pub const CELL: f32 = 16.0;

/// Зазор между виджетами по горизонтали и вертикали, логических пикселей.
pub const GAP: f32 = 8.0;

/// «Окно» сетки в целых ячейках внутри максимальной сетки `TabSpec`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Window {
    pub c0: u8,
    pub r0: u8,
    pub c1: u8,
    pub r1: u8,
}

impl Window {
    #[inline]
    pub fn cols(&self) -> u8 {
        self.c1.saturating_sub(self.c0)
    }

    #[inline]
    pub fn rows(&self) -> u8 {
        self.r1.saturating_sub(self.r0)
    }

    #[inline]
    pub fn content_w(&self) -> f32 {
        (self.cols() as f32 * CELL - GAP).max(0.0)
    }

    #[inline]
    pub fn content_h(&self) -> f32 {
        (self.rows() as f32 * CELL - GAP).max(0.0)
    }

    #[inline]
    pub fn island_w(&self) -> f32 {
        self.content_w() + 2.0 * PAD
    }

    #[inline]
    pub fn island_h(&self, extra: f32) -> f32 {
        TOP + self.content_h() + PAD + extra
    }

    #[inline]
    pub fn cell_to_pixel(&self, col: u8, row: u8) -> (f32, f32) {
        (
            (col as f32 - self.c0 as f32) * CELL,
            (row as f32 - self.r0 as f32) * CELL,
        )
    }

    #[cfg(test)]
    #[inline]
    pub fn pixel_to_cell(&self, x: f32, y: f32) -> (u8, u8) {
        let c = (self.c0 as f32 + (x / CELL).floor()).max(0.0) as u32;
        let r = (self.r0 as f32 + (y / CELL).floor()).max(0.0) as u32;
        (c.min(255) as u8, r.min(255) as u8)
    }

    #[inline]
    pub fn contains_cell(&self, col: u8, row: u8, w: u8, h: u8) -> bool {
        col >= self.c0 && row >= self.r0 && col + w <= self.c1 && row + h <= self.r1
    }
}

/// Сторона окна сетки для растягивания.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Top,
    Right,
    Bottom,
}

/// Растягивает окно на `delta` ячеек в направлении `side` в пределах максимальной сетки `spec`.
pub fn expand_side(win: Window, spec: &TabSpec, side: Side, delta: u8) -> Window {
    let mut w = win;
    match side {
        Side::Left => w.c0 = w.c0.saturating_sub(delta),
        Side::Top => w.r0 = w.r0.saturating_sub(delta),
        Side::Right => w.c1 = (w.c1 + delta).min(spec.cols),
        Side::Bottom => w.r1 = (w.r1 + delta).min(spec.rows),
    }
    w
}

/// Можно ли расширить окно в направлении `side` (есть ли свободное место до максимальной сетки).
#[inline]
pub fn can_expand_side(win: Window, spec: &TabSpec, side: Side) -> bool {
    match side {
        Side::Left => win.c0 > 0,
        Side::Top => win.r0 > 0,
        Side::Right => win.c1 < spec.cols,
        Side::Bottom => win.r1 < spec.rows,
    }
}

/// Вычисляет новые границы окна сетки при перетаскивании ручки растягивания.
/// По горизонтали расчёт ведётся относительно центра острова `x_c = pointer.0 - layer_w / 2.0`
/// (остров центрирован и растёт симметрично в обе стороны: желаемая ширина в ячейках
/// `round((2 * |x_c| - 2 * PAD + GAP) / CELL)` распределяется на выбранную сторону).
/// По вертикали верх зафиксирован у края экрана: для нижней ручки смещение идёт вниз,
/// для верхней — смещение вверх добавляет ряды над окном без сдвига верха.
pub fn stretch_window(
    side: Side,
    pointer: (f32, f32),
    press_pos: (f32, f32),
    initial_win: Window,
    spec: &TabSpec,
    layer_w: f32,
) -> Window {
    match side {
        Side::Right => {
            let x_c = pointer.0 - layer_w / 2.0;
            let desired_cols = if x_c > 0.0 {
                ((2.0 * x_c - 2.0 * PAD + GAP) / CELL).round().max(1.0) as u8
            } else {
                0
            };
            let initial_cols = initial_win.cols();
            let delta = desired_cols.saturating_sub(initial_cols);
            let new_c1 = (initial_win.c1 + delta).min(spec.cols);
            Window { c0: initial_win.c0, c1: new_c1, r0: initial_win.r0, r1: initial_win.r1 }
        }
        Side::Left => {
            let x_c = pointer.0 - layer_w / 2.0;
            let desired_cols = if x_c < 0.0 {
                ((2.0 * (-x_c) - 2.0 * PAD + GAP) / CELL).round().max(1.0) as u8
            } else {
                0
            };
            let initial_cols = initial_win.cols();
            let delta = desired_cols.saturating_sub(initial_cols);
            let new_c0 = initial_win.c0.saturating_sub(delta);
            Window { c0: new_c0, c1: initial_win.c1, r0: initial_win.r0, r1: initial_win.r1 }
        }
        Side::Bottom => {
            let delta_px = pointer.1 - press_pos.1;
            let cells = (delta_px / CELL).round().max(0.0) as u8;
            let new_r1 = (initial_win.r1 + cells).min(spec.rows);
            Window { c0: initial_win.c0, c1: initial_win.c1, r0: initial_win.r0, r1: new_r1 }
        }
        Side::Top => {
            let delta_px = press_pos.1 - pointer.1;
            let cells = (delta_px / CELL).round().max(0.0) as u8;
            let new_r0 = initial_win.r0.saturating_sub(cells);
            Window { c0: initial_win.c0, c1: initial_win.c1, r0: new_r0, r1: initial_win.r1 }
        }
    }
}

/// Плотное окно сетки вокруг всех видимых (не скрытых и не пропущенных) виджетов,
/// расширенное до `min_cols` и `min_rows` в пределах максимальной сетки `spec`.
pub fn fit_window(
    entries: &[Entry],
    spec: &TabSpec,
    skip: u32,
    min_cols: u8,
    min_rows: u8,
) -> Window {
    let mut min_c = u8::MAX;
    let mut min_r = u8::MAX;
    let mut max_c = 0u8;
    let mut max_r = 0u8;
    let mut any_visible = false;

    for e in entries {
        if e.hidden || skipped(skip, e.idx) {
            continue;
        }
        any_visible = true;
        let sz = spec.specs[e.idx].sizes[e.size];
        min_c = min_c.min(e.col);
        min_r = min_r.min(e.row);
        max_c = max_c.max(e.col + sz.w);
        max_r = max_r.max(e.row + sz.h);
    }

    if !any_visible {
        let c1 = min_cols.min(spec.cols);
        let r1 = min_rows.min(spec.rows);
        return Window { c0: 0, r0: 0, c1, r1 };
    }

    let mut c0 = min_c.min(spec.cols);
    let mut c1 = max_c.min(spec.cols).max(c0);
    let mut r0 = min_r.min(spec.rows);
    let mut r1 = max_r.min(spec.rows).max(r0);

    expand_range(&mut c0, &mut c1, min_cols, spec.cols);
    expand_range(&mut r0, &mut r1, min_rows, spec.rows);

    Window { c0, r0, c1, r1 }
}

/// Симметрично (или туда, где есть место) расширяет диапазон `[min_val, max_val]`
/// до целевой длины `target_len` в пределах `0..limit`.
pub fn expand_range(min_val: &mut u8, max_val: &mut u8, target_len: u8, limit: u8) {
    let cur_len = max_val.saturating_sub(*min_val);
    let target = target_len.min(limit);
    if cur_len >= target {
        return;
    }
    let needed = target - cur_len;
    let space_before = *min_val;
    let space_after = limit.saturating_sub(*max_val);

    let half = needed / 2;
    let mut add_after = (needed - half).min(space_after);
    let mut add_before = half.min(space_before);
    if add_before + add_after < needed {
        let rem = needed - (add_before + add_after);
        let extra_before = rem.min(space_before - add_before);
        add_before += extra_before;
        let rem2 = rem - extra_before;
        add_after += rem2.min(space_after - add_after);
    }
    *min_val -= add_before;
    *max_val += add_after;
}

/// Размер виджета в целых ячейках сетки.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Size {
    pub w: u8,
    pub h: u8,
}

pub const fn sz(w: u8, h: u8) -> Size {
    Size { w, h }
}

impl Size {
    #[inline]
    pub fn pixel_w(&self) -> f32 {
        self.w as f32 * CELL - GAP
    }

    #[inline]
    pub fn pixel_h(&self) -> f32 {
        self.h as f32 * CELL - GAP
    }
}

/// Описание виджета вкладки.
pub struct Spec {
    /// Токен в конфиге: латиница без пробелов.
    pub key: &'static str,
    /// Название для лотка скрытых виджетов (русский ключ `i18n`).
    pub title: &'static str,
    /// Допустимые размеры от меньшего к большему; не пусто.
    pub sizes: &'static [Size],
    /// Размер по умолчанию (индекс в `sizes`) и начальная ячейка (колонка и ряд).
    pub size: usize,
    pub col: u8,
    pub row: u8,
}

/// Описание вкладки: максимальный размер сетки в ячейках, минимальные размеры и виджеты.
pub struct TabSpec {
    pub cols: u8,
    pub rows: u8,
    pub min_cols: u8,
    pub min_rows: u8,
    pub specs: &'static [Spec],
}

pub static EMPTY: TabSpec = TabSpec { cols: 1, rows: 1, min_cols: 1, min_rows: 1, specs: &[] };

/// Состояние одного виджета в раскладке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Индекс в `TabSpec::specs`.
    pub idx: usize,
    /// Индекс в `Spec::sizes`.
    pub size: usize,
    pub col: u8,
    pub row: u8,
    pub hidden: bool,
}

/// Размещённый виджет с координатами в ячейках и пикселях.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub idx: usize,
    pub size: usize,
    pub col: u8,
    pub row: u8,
    pub w: u8,
    pub h: u8,
    pub x: f32,
    pub y: f32,
    pub w_px: f32,
    pub h_px: f32,
}

/// Раскладка вкладки.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub entries: Vec<Entry>,
}

/// Виджет недоступен на этом компьютере (нет яркости) и ни рисуется, ни показывается в лотке.
#[inline]
pub fn skipped(skip: u32, idx: usize) -> bool {
    skip & (1 << idx) != 0
}

/// Проверяет, можно ли поместить виджет `idx` размера `size` в ячейку `(col, row)`
/// внутри окна `win` без выхода за границы и без коллизий с другими видимыми виджетами.
#[allow(clippy::too_many_arguments)]
pub fn can_place_in_window(
    win: Window,
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
    col: u8,
    row: u8,
) -> bool {
    let sz = spec.specs[idx].sizes[size];
    if !win.contains_cell(col, row, sz.w, sz.h) {
        return false;
    }
    for e in entries {
        if e.idx == idx || e.hidden || skipped(skip, e.idx) {
            continue;
        }
        let esz = spec.specs[e.idx].sizes[e.size];
        let apart_x = col >= e.col + esz.w || e.col >= col + sz.w;
        let apart_y = row >= e.row + esz.h || e.row >= row + sz.h;
        if !apart_x && !apart_y {
            return false;
        }
    }
    true
}

/// Проверяет, можно ли поместить виджет `idx` размера `size` в ячейку `(col, row)`
/// без выхода за границы сетки и без коллизий с другими видимыми виджетами.
#[inline]
pub fn can_place(
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
    col: u8,
    row: u8,
) -> bool {
    can_place_in_window(
        Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows },
        spec,
        skip,
        entries,
        idx,
        size,
        col,
        row,
    )
}

/// Ищет свободное место для виджета `idx` размера `size` внутри окна `win`,
/// ближайшее к желаемой ячейке `(want_col, want_row)`.
/// При равном евклидовом расстоянии выбирается позиция в порядке чтения (меньший ряд, затем меньшая колонка).
#[allow(clippy::too_many_arguments)]
pub fn nearest_free_in_window(
    win: Window,
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
    want_col: u8,
    want_row: u8,
) -> Option<(u8, u8)> {
    let sz = spec.specs[idx].sizes[size];
    if sz.w > win.cols() || sz.h > win.rows() {
        return None;
    }
    let max_col = win.c1 - sz.w;
    let max_row = win.r1 - sz.h;
    let mut best: Option<(f32, u8, u8)> = None;

    for r in win.r0..=max_row {
        for c in win.c0..=max_col {
            if can_place_in_window(win, spec, skip, entries, idx, size, c, r) {
                let dc = c as f32 - want_col as f32;
                let dr = r as f32 - want_row as f32;
                let dist = dc * dc + dr * dr;
                let better = match best {
                    None => true,
                    Some((b_dist, b_r, b_c)) => {
                        dist < b_dist - 1e-4
                            || ((dist - b_dist).abs() <= 1e-4 && (r < b_r || (r == b_r && c < b_c)))
                    }
                };
                if better {
                    best = Some((dist, r, c));
                }
            }
        }
    }

    best.map(|(_, r, c)| (c, r))
}

/// Ищет свободное место для виджета `idx` размера `size`, ближайшее к желаемой ячейке `(want_col, want_row)`.
/// При равном евклидовом расстоянии выбирается позиция в порядке чтения (меньший ряд, затем меньшая колонка).
#[inline]
pub fn nearest_free(
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
    want_col: u8,
    want_row: u8,
) -> Option<(u8, u8)> {
    nearest_free_in_window(
        Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows },
        spec,
        skip,
        entries,
        idx,
        size,
        want_col,
        want_row,
    )
}

/// Первое подходящее место для виджета в порядке чтения внутри окна `win`.
pub fn first_free_in_window(
    win: Window,
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
) -> Option<(u8, u8)> {
    let sz = spec.specs[idx].sizes[size];
    if sz.w > win.cols() || sz.h > win.rows() {
        return None;
    }
    let max_col = win.c1 - sz.w;
    let max_row = win.r1 - sz.h;
    for r in win.r0..=max_row {
        for c in win.c0..=max_col {
            if can_place_in_window(win, spec, skip, entries, idx, size, c, r) {
                return Some((c, r));
            }
        }
    }
    None
}

/// Первое подходящее место для виджета в порядке чтения (сверху вниз, слева направо).
#[inline]
pub fn first_free(
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
) -> Option<(u8, u8)> {
    first_free_in_window(
        Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows },
        spec,
        skip,
        entries,
        idx,
        size,
    )
}

// ---------- Вытеснение соседей ----------

/// Цена каждой ячейки расстояния от виджета до желаемой точки.
const NEAR: f32 = 1.5;
/// Штраф за каждого сдвинутого соседа.
const PUSH_EACH: f32 = 1.0;
/// Штраф за каждую ячейку сдвига соседа (по Манхэттену).
const PUSH_CELL: f32 = 0.05;

/// Итог постановки виджета с вытеснением соседей.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// Новая раскладка целиком (скрытые и пропущенные виджеты не тронуты).
    pub entries: Vec<Entry>,
    /// Где встал сам виджет.
    pub col: u8,
    pub row: u8,
    /// Сколько соседей сдвинуто.
    pub pushed: u8,
    /// Стоимость: взвешенное расстояние до желаемой ячейки плюс штрафы за вытеснение.
    pub cost: f32,
}

/// Прямоугольник виджета в ячейках со знаком (для сдвигов за край без переполнения).
#[derive(Clone, Copy)]
struct Bx {
    c: i32,
    r: i32,
    w: i32,
    h: i32,
}

impl Bx {
    fn of(spec: &TabSpec, e: &Entry) -> Bx {
        let s = spec.specs[e.idx].sizes[e.size];
        Bx { c: e.col as i32, r: e.row as i32, w: s.w as i32, h: s.h as i32 }
    }

    fn hits(&self, o: &Bx) -> bool {
        self.c < o.c + o.w && o.c < self.c + self.w && self.r < o.r + o.h && o.r < self.r + self.h
    }

    fn inside(&self, win: Window) -> bool {
        self.c >= win.c0 as i32 && self.r >= win.r0 as i32 && self.c + self.w <= win.c1 as i32 && self.r + self.h <= win.r1 as i32
    }
}

/// Направление волны вытеснения.
#[derive(Clone, Copy)]
enum Dir {
    Down,
    Right,
    Up,
    Left,
}

#[inline]
fn active(skip: u32, e: &Entry) -> bool {
    !e.hidden && !skipped(skip, e.idx)
}

/// Волна в направлении `dir`: якорь `anchor` стоит на месте, остальные видимые виджеты обходятся
/// по порядку вдоль направления, и каждый, кто упирается в уже расставленных, едет в этом
/// направлении до первого свободного места. Так сдвинутый сосед толкает следующего (каскад),
/// а кто никому не мешает, остаётся на месте. `None` — кто-то выехал за окно.
fn sweep(win: Window, spec: &TabSpec, skip: u32, entries: &[Entry], anchor: usize, dir: Dir) -> Option<Vec<Entry>> {
    let mut out = entries.to_vec();
    let a = out.iter().position(|e| e.idx == anchor)?;
    let mut placed = vec![Bx::of(spec, &out[a])];
    let mut order: Vec<usize> = (0..out.len()).filter(|&i| i != a && active(skip, &out[i])).collect();
    order.sort_by_key(|&i| {
        let b = Bx::of(spec, &out[i]);
        match dir {
            Dir::Down => (b.r, b.c),
            Dir::Up => (-(b.r + b.h), b.c),
            Dir::Right => (b.c, b.r),
            Dir::Left => (-(b.c + b.w), b.r),
        }
    });
    let (dc, dr) = match dir {
        Dir::Down => (0, 1),
        Dir::Up => (0, -1),
        Dir::Right => (1, 0),
        Dir::Left => (-1, 0),
    };
    for i in order {
        let mut b = Bx::of(spec, &out[i]);
        while placed.iter().any(|p| p.hits(&b)) {
            b.c += dc;
            b.r += dr;
            if !b.inside(win) {
                return None;
            }
        }
        out[i].col = b.c as u8;
        out[i].row = b.r as u8;
        placed.push(b);
    }
    Some(out)
}

/// Переселение: соседи, на которых стоит якорь, уходят каждый на ближайшее свободное место
/// (крупные первыми). Остальные не двигаются. Так виджеты разного размера меняются местами.
fn relocate(win: Window, spec: &TabSpec, skip: u32, entries: &[Entry], anchor: usize) -> Option<Vec<Entry>> {
    let mut out = entries.to_vec();
    let a = Bx::of(spec, out.iter().find(|e| e.idx == anchor)?);
    let mut hit: Vec<usize> = (0..out.len())
        .filter(|&i| out[i].idx != anchor && active(skip, &out[i]) && Bx::of(spec, &out[i]).hits(&a))
        .collect();
    // Свободных ячеек меньше, чем нужно вытесненным, — искать нечего.
    let area = |i: usize| {
        let b = Bx::of(spec, &out[i]);
        b.w * b.h
    };
    let busy: i32 = (0..out.len()).filter(|&i| active(skip, &out[i]) && !hit.contains(&i)).map(area).sum();
    let need: i32 = hit.iter().map(|&i| area(i)).sum();
    if need > win.cols() as i32 * win.rows() as i32 - busy {
        return None;
    }
    hit.sort_by_key(|&i| (-area(i), out[i].row, out[i].col));
    for &i in &hit {
        out[i].hidden = true;
    }
    for &i in &hit {
        let e = out[i];
        let (col, row) = nearest_free_in_window(win, spec, skip, &out, e.idx, e.size, e.col, e.row)?;
        out[i].col = col;
        out[i].row = row;
        out[i].hidden = false;
    }
    Some(out)
}

/// Сколько соседей сдвинуто и во что это обходится.
fn displacement(skip: u32, base: &[Entry], out: &[Entry], anchor: usize) -> (u8, f32) {
    let mut n = 0u8;
    let mut cost = 0.0;
    for (b, o) in base.iter().zip(out) {
        if b.idx == anchor || !active(skip, b) || (b.col, b.row) == (o.col, o.row) {
            continue;
        }
        n += 1;
        let shift = (b.col as i32 - o.col as i32).abs() + (b.row as i32 - o.row as i32).abs();
        cost += PUSH_EACH + PUSH_CELL * shift as f32;
    }
    (n, cost)
}

/// Ставит виджет `idx` размера `size` в ячейку `(col, row)` (якорь не двигается) и расталкивает
/// соседей так, чтобы никто не пересекался и все остались в окне. Из волн во все стороны и
/// переселения берётся самая дешёвая. Без помех — раскладка с одним сдвинутым виджетом и
/// нулевой стоимостью. `None` — расставить всех нельзя.
#[allow(clippy::too_many_arguments)]
pub fn push(win: Window, spec: &TabSpec, skip: u32, base: &[Entry], idx: usize, size: usize, col: u8, row: u8) -> Option<Resolved> {
    let sz = spec.specs[idx].sizes[size];
    if !win.contains_cell(col, row, sz.w, sz.h) {
        return None;
    }
    let mut entries = base.to_vec();
    let e = entries.iter_mut().find(|e| e.idx == idx)?;
    *e = Entry { idx, size, col, row, hidden: false };
    if can_place_in_window(win, spec, skip, &entries, idx, size, col, row) {
        return Some(Resolved { entries, col, row, pushed: 0, cost: 0.0 });
    }
    let mut best: Option<Resolved> = None;
    let tries = [Dir::Down, Dir::Right, Dir::Up, Dir::Left]
        .into_iter()
        .map(|d| sweep(win, spec, skip, &entries, idx, d))
        .chain(std::iter::once_with(|| relocate(win, spec, skip, &entries, idx)));
    for out in tries.flatten() {
        let (pushed, cost) = displacement(skip, base, &out, idx);
        if best.as_ref().is_none_or(|b| cost < b.cost - 1e-4) {
            best = Some(Resolved { entries: out, col, row, pushed, cost });
        }
    }
    best
}

/// Куда встанет перетаскиваемый виджет `idx` размера `size`, если курсор держит его над ячейкой
/// `want`. Считается всегда от раскладки `base` на начало жеста, поэтому одна и та же цель даёт
/// одну и ту же раскладку, а ушедший курсор возвращает соседей на места.
///
/// Ячейки окна перебираются по возрастанию расстояния до `want`; стоимость кандидата — расстояние
/// плюс штрафы за вытеснение. Стоимость не меньше расстояния, поэтому перебор останавливается,
/// как только расстояние догоняет лучшую найденную стоимость. Близкая прямая постановка
/// побеждает вытеснение, а курсор прямо над соседом меняет их местами.
/// `None` — в окне для виджета нет места вообще.
#[allow(clippy::too_many_arguments)]
pub fn resolve_drop(
    win: Window,
    spec: &TabSpec,
    skip: u32,
    base: &[Entry],
    idx: usize,
    size: usize,
    want: (u8, u8),
) -> Option<Resolved> {
    let sz = spec.specs[idx].sizes[size];
    if sz.w > win.cols() || sz.h > win.rows() {
        return None;
    }
    let mut cands = Vec::with_capacity((win.cols() - sz.w + 1) as usize * (win.rows() - sz.h + 1) as usize);
    for r in win.r0..=win.r1 - sz.h {
        for c in win.c0..=win.c1 - sz.w {
            let dist = NEAR * (c as f32 - want.0 as f32).hypot(r as f32 - want.1 as f32);
            cands.push((dist, r, c));
        }
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let others: Vec<Bx> = base.iter().filter(|e| e.idx != idx && active(skip, e)).map(|e| Bx::of(spec, e)).collect();
    let mut best: Option<Resolved> = None;
    for (dist, r, c) in cands {
        let bound = best.as_ref().map_or(f32::INFINITY, |b| b.cost - 1e-4);
        if dist >= bound {
            break;
        }
        // Каждого, на кого встаёт виджет, придётся сдвинуть: дешевле лучшего так не выйдет.
        let me = Bx { c: c as i32, r: r as i32, w: sz.w as i32, h: sz.h as i32 };
        let hits = others.iter().filter(|o| o.hits(&me)).count();
        if dist + hits as f32 * PUSH_EACH >= bound {
            continue;
        }
        if let Some(mut res) = push(win, spec, skip, base, idx, size, c, r) {
            res.cost += dist;
            if best.as_ref().is_none_or(|b| res.cost < b.cost - 1e-4) {
                best = Some(res);
            }
        }
    }
    best
}

/// Новый размер виджета `idx` под угловой ручкой: размеры перебираются по близости к желаемому
/// прямоугольнику `want_w` × `want_h` (с гистерезисом в пользу `prefer`), левый верхний угол
/// стоит на месте (сдвигается, только если иначе виджет вылезает за окно), соседи расталкиваются.
/// Первый размер, который удаётся разместить, и есть ответ. Считается от `base` на начало жеста.
#[allow(clippy::too_many_arguments)]
pub fn resolve_resize(
    win: Window,
    spec: &TabSpec,
    skip: u32,
    base: &[Entry],
    idx: usize,
    want_w: f32,
    want_h: f32,
    prefer: usize,
) -> Option<Resolved> {
    let cur = *base.iter().find(|e| e.idx == idx)?;
    size_order(spec, idx, want_w, want_h, prefer)
        .into_iter()
        .find_map(|size| push_at(win, spec, skip, base, idx, size, cur.col, cur.row))
}

/// Размеры виджета `idx` по близости к прямоугольнику `want_w` × `want_h` (первый — тот, что
/// под курсором), с гистерезисом в пользу `prefer`.
pub fn size_order(spec: &TabSpec, idx: usize, want_w: f32, want_h: f32, prefer: usize) -> Vec<usize> {
    let mut sizes: Vec<(f32, usize)> = spec.specs[idx]
        .sizes
        .iter()
        .enumerate()
        .map(|(size, s)| {
            let cost = (s.pixel_w() - want_w).abs() + (s.pixel_h() - want_h).abs();
            (if size == prefer { cost - 2.0 } else { cost }, size)
        })
        .collect();
    sizes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    sizes.into_iter().map(|(_, size)| size).collect()
}

/// `push` с якорем у ячейки `(col, row)`, придвинутым внутрь окна, если виджет за него вылезает.
#[allow(clippy::too_many_arguments)]
fn push_at(win: Window, spec: &TabSpec, skip: u32, base: &[Entry], idx: usize, size: usize, col: u8, row: u8) -> Option<Resolved> {
    let s = spec.specs[idx].sizes[size];
    if s.w > win.cols() || s.h > win.rows() {
        return None;
    }
    let col = col.clamp(win.c0, win.c1 - s.w);
    let row = row.clamp(win.r0, win.r1 - s.h);
    push(win, spec, skip, base, idx, size, col, row)
}

impl Layout {
    /// Раскладка по умолчанию: все виджеты видимы, стоят на позициях из `Spec`.
    pub fn default_for(spec: &TabSpec) -> Layout {
        let entries = spec
            .specs
            .iter()
            .enumerate()
            .map(|(idx, s)| Entry {
                idx,
                size: s.size,
                col: s.col,
                row: s.row,
                hidden: false,
            })
            .collect();
        Layout { entries }
    }

    pub fn is_default(&self, spec: &TabSpec) -> bool {
        *self == Layout::default_for(spec)
    }

    /// Разбирает строку из конфига. Незнакомые ключи, повторы и мусор пропускаются,
    /// недостающие виджеты дописываются в свободные места, не влезающие — скрываются в лоток.
    /// Старый формат без запятой в `@col` аккуратно обнаруживается и сбрасывается к значениям по умолчанию.
    pub fn parse(spec: &TabSpec, text: &str) -> Layout {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Layout::default_for(spec);
        }

        // Обнаружение старого формата: токен содержит '@', после которого нет запятой ','.
        let is_legacy = text.split(|c: char| c.is_whitespace() || c == ';').any(|tok| {
            if let Some((_, at)) = tok.split_once('@') {
                !at.contains(',')
            } else {
                false
            }
        });
        if is_legacy {
            crate::log::error("обнаружен устаревший формат раскладки (без запятой в @col,row), сброс к раскладке по умолчанию");
            return Layout::default_for(spec);
        }

        let mut out: Vec<Entry> = Vec::new();
        for tok in text.split(|c: char| c.is_whitespace() || c == ';').filter(|t| !t.is_empty()) {
            let (hidden, rest) = match tok.strip_prefix('-') {
                Some(r) => (true, r),
                None => (false, tok),
            };
            let (rest, coords) = match rest.split_once('@') {
                Some((a, b)) => {
                    let parts = b.split_once(',');
                    match parts {
                        Some((c_str, r_str)) => {
                            let c = c_str.parse::<u8>().ok();
                            let r = r_str.parse::<u8>().ok();
                            (a, c.zip(r))
                        }
                        None => (rest, None),
                    }
                }
                None => (rest, None),
            };
            let (key, size) = match rest.split_once(':') {
                Some((a, b)) => (a, b.parse::<usize>().ok()),
                None => (rest, None),
            };
            let Some(idx) = spec.specs.iter().position(|s| s.key == key) else { continue };
            if out.iter().any(|e| e.idx == idx) {
                continue;
            }
            let s = &spec.specs[idx];
            let size = size.filter(|&n| n < s.sizes.len()).unwrap_or(s.size);
            let (want_col, want_row) = coords.unwrap_or((s.col, s.row));

            if hidden {
                out.push(Entry { idx, size, col: want_col, row: want_row, hidden: true });
            } else if can_place(spec, 0, &out, idx, size, want_col, want_row) {
                out.push(Entry { idx, size, col: want_col, row: want_row, hidden: false });
            } else if let Some((col, row)) = nearest_free(spec, 0, &out, idx, size, want_col, want_row) {
                out.push(Entry { idx, size, col, row, hidden: false });
            } else {
                // Нет места — скрываем в лоток
                out.push(Entry { idx, size, col: want_col, row: want_row, hidden: true });
            }
        }

        // Недостающие виджеты дописываются в свободные места, не влезающие — скрываются
        for (idx, s) in spec.specs.iter().enumerate() {
            if out.iter().any(|e| e.idx == idx) {
                continue;
            }
            if can_place(spec, 0, &out, idx, s.size, s.col, s.row) {
                out.push(Entry { idx, size: s.size, col: s.col, row: s.row, hidden: false });
            } else if let Some((c, r)) = first_free(spec, 0, &out, idx, s.size) {
                out.push(Entry { idx, size: s.size, col: c, row: r, hidden: false });
            } else {
                out.push(Entry { idx, size: s.size, col: s.col, row: s.row, hidden: true });
            }
        }

        Layout { entries: out }
    }

    /// Строка для `config.toml`; для раскладки по умолчанию — пустая.
    pub fn to_text(&self, spec: &TabSpec) -> String {
        if self.is_default(spec) {
            return String::new();
        }
        let mut parts = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            let s = &spec.specs[e.idx];
            let mut t = String::new();
            if e.hidden {
                t.push('-');
            }
            t.push_str(s.key);
            if e.size != s.size {
                t.push_str(&format!(":{}", e.size));
            }
            t.push_str(&format!("@{},{}", e.col, e.row));
            parts.push(t);
        }
        parts.join(" ")
    }

    pub fn entry(&self, idx: usize) -> Option<&Entry> {
        self.entries.iter().find(|e| e.idx == idx)
    }

    pub fn entry_mut(&mut self, idx: usize) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.idx == idx)
    }

    /// Преобразует видимые виджеты в список размещённых со сдвигом на `(-win.c0, -win.r0)` ячеек.
    pub fn pack_window(&self, spec: &TabSpec, skip: u32, win: Window) -> Vec<Placed> {
        let mut out = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            if e.hidden || skipped(skip, e.idx) {
                continue;
            }
            let sz = spec.specs[e.idx].sizes[e.size];
            let w_px = sz.pixel_w();
            let h_px = sz.pixel_h();
            let (x, y) = win.cell_to_pixel(e.col, e.row);
            out.push(Placed {
                idx: e.idx,
                size: e.size,
                col: e.col,
                row: e.row,
                w: sz.w,
                h: sz.h,
                x,
                y,
                w_px,
                h_px,
            });
        }
        out
    }

    /// Преобразует видимые виджеты в список размещённых с экранными пиксельными координатами.
    #[cfg(test)]
    #[inline]
    pub fn pack(&self, spec: &TabSpec, skip: u32) -> Vec<Placed> {
        self.pack_window(spec, skip, Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows })
    }

    /// Проверяет, что все видимые непропущенные виджеты помещаются в границы сетки.
    pub fn fits(&self, spec: &TabSpec, skip: u32, _avail: f32) -> bool {
        for e in &self.entries {
            if e.hidden || skipped(skip, e.idx) {
                continue;
            }
            let sz = spec.specs[e.idx].sizes[e.size];
            if e.col + sz.w > spec.cols || e.row + sz.h > spec.rows {
                return false;
            }
        }
        true
    }

    pub fn visible_count(&self, skip: u32) -> usize {
        self.entries.iter().filter(|e| !e.hidden && !skipped(skip, e.idx)).count()
    }

    /// Скрытые доступные виджеты в порядке по умолчанию (для лотка).
    pub fn hidden_list(&self, spec: &TabSpec, skip: u32) -> Vec<usize> {
        (0..spec.specs.len())
            .filter(|&i| !skipped(skip, i) && self.entry(i).is_some_and(|e| e.hidden))
            .collect()
    }

    // ---------- Правки ----------

    pub fn hide(&mut self, idx: usize) {
        if let Some(e) = self.entry_mut(idx) {
            e.hidden = true;
        }
    }

    /// Возвращает скрытый виджет из лотка внутри окна `win`: ищет первое подходящее место в порядке чтения.
    /// При нехватке места пробует меньшие размеры. `false` — места нет.
    pub fn place_new_in_window(&mut self, win: Window, spec: &TabSpec, skip: u32, idx: usize) -> bool {
        let Some(pos) = self.entries.iter().position(|e| e.idx == idx) else { return false };
        let cur_size = self.entries[pos].size;
        for size in (0..=cur_size).rev() {
            if let Some((col, row)) = first_free_in_window(win, spec, skip, &self.entries, idx, size) {
                let e = &mut self.entries[pos];
                e.size = size;
                e.col = col;
                e.row = row;
                e.hidden = false;
                return true;
            }
        }
        false
    }

    /// Возвращает скрытый виджет из лотка: ищет первое подходящее место в порядке чтения.
    /// При нехватке места пробует меньшие размеры. `false` — места нет.
    #[cfg(test)]
    #[inline]
    pub fn place_new(&mut self, spec: &TabSpec, skip: u32, idx: usize) -> bool {
        self.place_new_in_window(Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows }, spec, skip, idx)
    }

    /// Следующий по кругу размер виджета внутри окна `win`: виджет растёт на месте и расталкивает
    /// соседей, а если так не выходит — переезжает в ближайшее свободное место. `false` — менять некуда.
    pub fn cycle_size_in_window(&mut self, win: Window, spec: &TabSpec, skip: u32, idx: usize) -> bool {
        let Some(cur) = self.entry(idx).copied() else { return false };
        let n = spec.specs[idx].sizes.len();
        if n <= 1 {
            return false;
        }
        for step in 1..n {
            let size = (cur.size + step) % n;
            if let Some(res) = push_at(win, spec, skip, &self.entries, idx, size, cur.col, cur.row) {
                self.entries = res.entries;
                return true;
            }
            if let Some((col, row)) = nearest_free_in_window(win, spec, skip, &self.entries, idx, size, cur.col, cur.row) {
                if let Some(e) = self.entry_mut(idx) {
                    e.size = size;
                    e.col = col;
                    e.row = row;
                    return true;
                }
            }
        }
        false
    }

    /// Следующий по кругу размер виджета, при котором он помещается на сетку (на текущем месте
    /// или со сдвигом к ближайшему свободному). `false` — менять некуда.
    #[cfg(test)]
    #[inline]
    pub fn cycle_size(&mut self, spec: &TabSpec, skip: u32, idx: usize, _avail: f32) -> bool {
        self.cycle_size_in_window(Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows }, spec, skip, idx)
    }

    /// Размер виджета, ближайший к желаемому прямоугольнику `want_w` × `want_h` (угловая ручка),
    /// с вытеснением соседей на всей сетке. `None` — ни один размер не подходит.
    #[cfg(test)]
    pub fn resized_to(&self, spec: &TabSpec, skip: u32, idx: usize, want_w: f32, want_h: f32) -> Option<Layout> {
        let win = Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows };
        let prefer = self.entry(idx)?.size;
        resolve_resize(win, spec, skip, &self.entries, idx, want_w, want_h, prefer).map(|r| Layout { entries: r.entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SIZES_A: [Size; 3] = [sz(2, 2), sz(4, 2), sz(6, 2)];
    static SIZES_B: [Size; 2] = [sz(4, 2), sz(8, 4)];
    static SPEC: TabSpec = TabSpec {
        cols: 8,
        rows: 6,
        min_cols: 4,
        min_rows: 3,
        specs: &[
            Spec { key: "a", title: "A", sizes: &SIZES_A, size: 1, col: 0, row: 0 },
            Spec { key: "b", title: "B", sizes: &SIZES_A, size: 1, col: 4, row: 0 },
            Spec { key: "c", title: "C", sizes: &SIZES_B, size: 0, col: 0, row: 2 },
            Spec { key: "d", title: "D", sizes: &SIZES_B, size: 0, col: 4, row: 2 },
        ],
    };

    fn no_overlap(placed: &[Placed]) -> bool {
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                let cols = a.col < b.col + b.w && b.col < a.col + a.w;
                let rows = a.row < b.row + b.h && b.row < a.row + a.h;
                if cols && rows {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn default_layout_no_overlap() {
        let lay = Layout::default_for(&SPEC);
        let p = lay.pack(&SPEC, 0);
        assert_eq!(p.len(), 4);
        assert_eq!((p[0].col, p[0].row), (0, 0));
        assert_eq!((p[1].col, p[1].row), (4, 0));
        assert_eq!((p[2].col, p[2].row), (0, 2));
        assert_eq!((p[3].col, p[3].row), (4, 2));
        assert!(no_overlap(&p));
    }

    #[test]
    fn can_place_bounds_and_collisions() {
        let lay = Layout::default_for(&SPEC);
        // Занято виджетом B
        assert!(!can_place(&SPEC, 0, &lay.entries, 0, 1, 4, 0));
        // Выход за границы по ширине: размер A size 2 имеет w=6, col=4 -> 4+6=10 > 8
        assert!(!can_place(&SPEC, 0, &lay.entries, 0, 2, 4, 0));
        // Свободное место на ряду 4 (row 4..6):
        assert!(can_place(&SPEC, 0, &lay.entries, 0, 1, 0, 4));
    }

    #[test]
    fn nearest_free_finds_closest_and_reading_order() {
        let lay = Layout::default_for(&SPEC);
        // Для виджета A размера 0 (2x2): желаемая точка (0, 4) свободна
        let pos = nearest_free(&SPEC, 0, &lay.entries, 0, 0, 0, 4);
        assert_eq!(pos, Some((0, 4)));

        // Желаемая точка (2, 0) занята виджетами A и B — ищет ближайшую свободную
        let pos = nearest_free(&SPEC, 0, &lay.entries, 0, 0, 2, 0);
        assert!(pos.is_some());
        let (c, r) = pos.unwrap();
        assert!(can_place(&SPEC, 0, &lay.entries, 0, 0, c, r));
    }

    #[test]
    fn place_new_returns_from_tray_and_tries_smaller() {
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(2); // скрыть C
        assert!(lay.entry(2).unwrap().hidden);
        // Возвращаем C: должно найти место (0, 2)
        assert!(lay.place_new(&SPEC, 0, 2));
        assert!(!lay.entry(2).unwrap().hidden);
        assert_eq!((lay.entry(2).unwrap().col, lay.entry(2).unwrap().row), (0, 2));
    }

    #[test]
    fn cycle_size_and_shift() {
        let mut lay = Layout::default_for(&SPEC);
        // A на (0, 0) с размером 1 (4x2). Следующий размер 2 (6x2) на (0, 0) пересекается с B (4x2).
        // Но на ряду 4 есть свободное место 6x2!
        assert!(lay.cycle_size(&SPEC, 0, 0, 200.0));
        let a = lay.entry(0).unwrap();
        assert_eq!(a.size, 2);
        assert!(no_overlap(&lay.pack(&SPEC, 0)));
    }

    // ---------- Вытеснение ----------

    static T42: [Size; 1] = [sz(4, 2)];
    static T43: [Size; 1] = [sz(4, 3)];
    static GROW: [Size; 2] = [sz(4, 2), sz(8, 2)];

    /// Окно на всю сетку вкладки.
    fn full(spec: &TabSpec) -> Window {
        Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows }
    }

    fn at(idx: usize, col: u8, row: u8) -> Entry {
        Entry { idx, size: 0, col, row, hidden: false }
    }

    fn pos(r: &Resolved, idx: usize) -> (u8, u8) {
        let e = r.entries.iter().find(|e| e.idx == idx).unwrap();
        (e.col, e.row)
    }

    /// Инварианты результата: нет пересечений, всё видимое в окне, скрытые и пропущенные не тронуты,
    /// размеры соседей прежние, перемещён только виджет `idx` и те, кого он вытеснил.
    fn check(win: Window, spec: &TabSpec, skip: u32, base: &[Entry], idx: usize, r: &Resolved) {
        assert_eq!(r.entries.len(), base.len());
        let act: Vec<&Entry> = r.entries.iter().filter(|e| !e.hidden && !skipped(skip, e.idx)).collect();
        for (i, a) in act.iter().enumerate() {
            let sa = spec.specs[a.idx].sizes[a.size];
            assert!(win.contains_cell(a.col, a.row, sa.w, sa.h), "{a:?} вне окна {win:?}");
            for b in &act[i + 1..] {
                assert!(!Bx::of(spec, a).hits(&Bx::of(spec, b)), "{a:?} пересекает {b:?}");
            }
        }
        let mut moved = 0;
        for (b, o) in base.iter().zip(&r.entries) {
            assert_eq!(b.idx, o.idx);
            if b.idx == idx {
                assert_eq!((o.col, o.row), (r.col, r.row));
                continue;
            }
            assert_eq!(b.size, o.size, "у соседа сменился размер");
            assert_eq!(b.hidden, o.hidden);
            if b.hidden || skipped(skip, b.idx) {
                assert_eq!(b, o, "скрытый или пропущенный виджет сдвинут");
            } else if (b.col, b.row) != (o.col, o.row) {
                moved += 1;
            }
        }
        assert_eq!(moved, r.pushed);
    }

    #[test]
    fn drop_into_free_cell_moves_nobody() {
        let base = Layout::default_for(&SPEC).entries;
        let r = resolve_drop(full(&SPEC), &SPEC, 0, &base, 0, 1, (0, 4)).unwrap();
        assert_eq!(pos(&r, 0), (0, 4));
        assert_eq!(r.pushed, 0);
        assert_eq!(r.cost, 0.0);
        check(full(&SPEC), &SPEC, 0, &base, 0, &r);
    }

    #[test]
    fn push_down_first() {
        static S: TabSpec = TabSpec {
            cols: 4,
            rows: 6,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &T42, size: 0, col: 0, row: 0 },
                Spec { key: "b", title: "B", sizes: &T42, size: 0, col: 0, row: 2 },
            ],
        };
        let base = vec![at(0, 0, 0), at(1, 0, 2)];
        // Вверх и вниз сдвиг одинаковый: побеждает «вниз»
        let r = push(full(&S), &S, 0, &base, 0, 0, 0, 2).unwrap();
        assert_eq!(pos(&r, 1), (0, 4));
        assert_eq!(r.pushed, 1);
        check(full(&S), &S, 0, &base, 0, &r);
    }

    #[test]
    fn push_right_when_down_is_blocked() {
        static S: TabSpec = TabSpec {
            cols: 12,
            rows: 2,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &T42, size: 0, col: 0, row: 0 },
                Spec { key: "b", title: "B", sizes: &T42, size: 0, col: 4, row: 0 },
            ],
        };
        let base = vec![at(0, 0, 0), at(1, 4, 0)];
        let r = push(full(&S), &S, 0, &base, 0, 0, 4, 0).unwrap();
        assert_eq!(pos(&r, 1), (8, 0));
        check(full(&S), &S, 0, &base, 0, &r);
    }

    #[test]
    fn push_left_and_up_at_window_edge() {
        static ROW: TabSpec = TabSpec {
            cols: 8,
            rows: 2,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &T42, size: 0, col: 0, row: 0 },
                Spec { key: "b", title: "B", sizes: &T42, size: 0, col: 4, row: 0 },
            ],
        };
        // Обмен местами в тесном ряду: B уходит влево, на место A
        let base = vec![at(0, 0, 0), at(1, 4, 0)];
        let r = push(full(&ROW), &ROW, 0, &base, 0, 0, 4, 0).unwrap();
        assert_eq!(pos(&r, 1), (0, 0));
        check(full(&ROW), &ROW, 0, &base, 0, &r);

        static COL: TabSpec = TabSpec {
            cols: 4,
            rows: 4,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &T42, size: 0, col: 0, row: 0 },
                Spec { key: "b", title: "B", sizes: &T42, size: 0, col: 0, row: 2 },
            ],
        };
        let base = vec![at(0, 0, 0), at(1, 0, 2)];
        let r = push(full(&COL), &COL, 0, &base, 0, 0, 0, 2).unwrap();
        assert_eq!(pos(&r, 1), (0, 0));
        check(full(&COL), &COL, 0, &base, 0, &r);
    }

    #[test]
    fn push_cascades() {
        static S: TabSpec = TabSpec {
            cols: 4,
            rows: 7,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &T42, size: 0, col: 0, row: 5 },
                Spec { key: "b", title: "B", sizes: &T43, size: 0, col: 0, row: 0 },
                Spec { key: "c", title: "C", sizes: &T42, size: 0, col: 0, row: 3 },
            ],
        };
        // A наверх: B (4×3) в освободившиеся 2 ряда не влезает, поэтому B толкает C вниз
        let base = vec![at(0, 0, 5), at(1, 0, 0), at(2, 0, 3)];
        let r = push(full(&S), &S, 0, &base, 0, 0, 0, 0).unwrap();
        assert_eq!(pos(&r, 1), (0, 2));
        assert_eq!(pos(&r, 2), (0, 5));
        assert_eq!(r.pushed, 2);
        check(full(&S), &S, 0, &base, 0, &r);
    }

    #[test]
    fn relocate_swaps_into_vacated_spot() {
        static S: TabSpec = TabSpec {
            cols: 8,
            rows: 4,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &T42, size: 0, col: 0, row: 2 },
                Spec { key: "b", title: "B", sizes: &T42, size: 0, col: 4, row: 0 },
                Spec { key: "c", title: "C", sizes: &T42, size: 0, col: 0, row: 0 },
                Spec { key: "d", title: "D", sizes: &T42, size: 0, col: 4, row: 2 },
            ],
        };
        // Окно занято целиком, волны упираются в края; B уходит на освободившееся место A
        let base = vec![at(0, 0, 2), at(1, 4, 0), at(2, 0, 0), at(3, 4, 2)];
        let r = push(full(&S), &S, 0, &base, 0, 0, 4, 0).unwrap();
        assert_eq!(pos(&r, 1), (0, 2));
        assert_eq!(r.pushed, 1);
        check(full(&S), &S, 0, &base, 0, &r);
    }

    #[test]
    fn drop_rejects_blocked_targets_and_keeps_place() {
        use crate::tabs::controls::SPEC as C;
        // «Управление» по умолчанию занимает всё окно: Wi-Fi чуть правее своего места
        // не может растолкать соседей и остаётся где был.
        let base = Layout::default_for(&C).entries;
        let win = fit_window(&base, &C, 0, C.min_cols, C.min_rows);
        let r = resolve_drop(win, &C, 0, &base, 0, 1, (2, 0)).unwrap();
        assert_eq!(pos(&r, 0), (0, 0));
        assert_eq!(r.pushed, 0);
        assert_eq!(r.entries, base);
    }

    #[test]
    fn drop_over_neighbour_swaps() {
        use crate::tabs::controls::SPEC as C;
        let base = Layout::default_for(&C).entries;
        let win = fit_window(&base, &C, 0, C.min_cols, C.min_rows);
        // Wi-Fi прямо над Bluetooth: меняются местами
        let r = resolve_drop(win, &C, 0, &base, 0, 1, (8, 0)).unwrap();
        assert_eq!(pos(&r, 0), (8, 0));
        assert_eq!(pos(&r, 1), (0, 0));
        assert_eq!(r.pushed, 1);
        check(win, &C, 0, &base, 0, &r);
    }

    #[test]
    fn drop_big_widget_reflows_neighbours() {
        use crate::tabs::controls::SPEC as C;
        let base = Layout::default_for(&C).entries;
        let win = fit_window(&base, &C, 0, C.min_cols, C.min_rows);
        // Плеер 24×6 наверх: плитки и ползунок уезжают вниз, в окно 24×16 влезают все
        let media = 7;
        let size = base[media].size;
        let r = resolve_drop(win, &C, 0, &base, media, size, (0, 0)).unwrap();
        assert_eq!(pos(&r, media), (0, 0));
        assert!(r.pushed > 0);
        check(win, &C, 0, &base, media, &r);
    }

    #[test]
    fn drop_none_when_window_is_full() {
        static S: TabSpec = TabSpec {
            cols: 4,
            rows: 2,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &T42, size: 0, col: 0, row: 0 },
                Spec { key: "b", title: "B", sizes: &T42, size: 0, col: 0, row: 0 },
            ],
        };
        let base = vec![at(0, 0, 0), Entry { hidden: true, ..at(1, 0, 0) }];
        assert_eq!(resolve_drop(full(&S), &S, 0, &base, 1, 0, (0, 0)), None);
        // Виджет больше окна
        let tiny = Window { c0: 0, r0: 0, c1: 2, r1: 2 };
        assert_eq!(resolve_drop(tiny, &S, 0, &base, 0, 0, (0, 0)), None);
    }

    #[test]
    fn drop_is_deterministic_and_returns_home() {
        use crate::tabs::controls::SPEC as C;
        let base = Layout::default_for(&C).entries;
        let win = fit_window(&base, &C, 0, C.min_cols, C.min_rows);
        let first = resolve_drop(win, &C, 0, &base, 0, 1, (8, 0));
        let _ = resolve_drop(win, &C, 0, &base, 0, 1, (16, 3));
        let _ = resolve_drop(win, &C, 0, &base, 0, 1, (0, 10));
        assert_eq!(resolve_drop(win, &C, 0, &base, 0, 1, (8, 0)), first);
        // Обратно в исходную ячейку — исходная раскладка
        let home = resolve_drop(win, &C, 0, &base, 0, 1, (0, 0)).unwrap();
        assert_eq!(home.entries, base);
        assert_eq!(home.pushed, 0);
    }

    #[test]
    fn drop_ignores_hidden_and_skipped() {
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(3); // D скрыт
        let skip = 0b0010; // B недоступен
        let base = lay.entries.clone();
        let win = full(&SPEC);
        for r in 0..SPEC.rows {
            for c in 0..SPEC.cols {
                let res = resolve_drop(win, &SPEC, skip, &base, 0, 1, (c, r)).unwrap();
                check(win, &SPEC, skip, &base, 0, &res);
            }
        }
    }

    #[test]
    fn resolved_layout_survives_text_roundtrip() {
        use crate::tabs::controls::SPEC as C;
        let base = Layout::default_for(&C).entries;
        let win = fit_window(&base, &C, 0, C.min_cols, C.min_rows);
        let r = resolve_drop(win, &C, 0, &base, 7, base[7].size, (0, 0)).unwrap();
        let lay = Layout { entries: r.entries };
        assert_eq!(Layout::parse(&C, &lay.to_text(&C)), lay);
    }

    /// Все виджеты настоящих вкладок над каждой ячейкой окна: виджет всегда куда-то встаёт,
    /// раскладка корректна, паники нет. Раскладка по умолчанию занимает окно целиком, поэтому
    /// вторая — с половиной скрытых виджетов, где есть куда вытеснять.
    #[test]
    fn real_tabs_always_resolve() {
        use crate::tabs::{controls, stats, today};
        for spec in [&controls::SPEC, &today::SPEC, &stats::SPEC] {
            let full_lay = Layout::default_for(spec);
            let mut half = full_lay.clone();
            for i in (1..spec.specs.len()).step_by(2) {
                half.hide(i);
            }
            let win = fit_window(&full_lay.entries, spec, 0, spec.min_cols, spec.min_rows);
            for base in [full_lay.entries, half.entries] {
                for e in base.iter().filter(|e| !e.hidden) {
                    for r in win.r0..win.r1 {
                        for c in win.c0..win.c1 {
                            let res = resolve_drop(win, spec, 0, &base, e.idx, e.size, (c, r))
                                .unwrap_or_else(|| panic!("{} не встал у {c},{r}", spec.specs[e.idx].key));
                            check(win, spec, 0, &base, e.idx, &res);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn resize_grow_pushes_neighbour() {
        static S: TabSpec = TabSpec {
            cols: 8,
            rows: 4,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &GROW, size: 0, col: 0, row: 0 },
                Spec { key: "b", title: "B", sizes: &T42, size: 0, col: 4, row: 0 },
            ],
        };
        let base = vec![at(0, 0, 0), at(1, 4, 0)];
        let r = resolve_resize(full(&S), &S, 0, &base, 0, GROW[1].pixel_w(), GROW[1].pixel_h(), 0).unwrap();
        assert_eq!(r.entries[0].size, 1);
        assert_eq!(pos(&r, 0), (0, 0));
        assert_eq!(pos(&r, 1), (4, 2));
        check(full(&S), &S, 0, &base, 0, &r);
    }

    #[test]
    fn resize_at_edge_shifts_widget_itself() {
        static S: TabSpec = TabSpec {
            cols: 8,
            rows: 2,
            min_cols: 4,
            min_rows: 2,
            specs: &[Spec { key: "a", title: "A", sizes: &GROW, size: 0, col: 4, row: 0 }],
        };
        let base = vec![at(0, 4, 0)];
        let r = resolve_resize(full(&S), &S, 0, &base, 0, GROW[1].pixel_w(), GROW[1].pixel_h(), 0).unwrap();
        assert_eq!(r.entries[0].size, 1);
        assert_eq!(pos(&r, 0), (0, 0));
    }

    #[test]
    fn resize_impossible_size_falls_back() {
        static S: TabSpec = TabSpec {
            cols: 8,
            rows: 2,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &GROW, size: 0, col: 0, row: 0 },
                Spec { key: "b", title: "B", sizes: &T42, size: 0, col: 4, row: 0 },
            ],
        };
        // Соседу некуда деться: остаётся прежний размер
        let base = vec![at(0, 0, 0), at(1, 4, 0)];
        let r = resolve_resize(full(&S), &S, 0, &base, 0, GROW[1].pixel_w(), GROW[1].pixel_h(), 0).unwrap();
        assert_eq!(r.entries, base);
    }

    #[test]
    fn cycle_size_pushes_neighbours() {
        use crate::tabs::controls::SPEC as C;
        let mut lay = Layout::default_for(&C);
        let win = Window { c0: 0, r0: 0, c1: C.cols, r1: C.rows };
        // Окно занято целиком; спрячем плеер, чтобы соседям было куда уйти
        lay.hide(7);
        let before = lay.clone();
        // Wi-Fi 8×3 → 12×3 растёт на месте: Bluetooth и Аудио расталкиваются, Wi-Fi не переезжает
        assert!(lay.cycle_size_in_window(win, &C, 0, 0));
        let wifi = lay.entry(0).unwrap();
        assert_eq!((wifi.size, wifi.col, wifi.row), (2, 0, 0));
        assert_ne!(lay, before);
        assert!(no_overlap(&lay.pack(&C, 0)));
    }

    #[test]
    fn resized_to_snaps_to_valid_size() {
        let lay = Layout::default_for(&SPEC);
        // Виджет C (4x2): пробуем растянуть до 8x4.
        // На (0, 2) размер 8x4 вызовет коллизию с D на (4, 2).
        // Поэтому должен остаться подходящий размер 4x2.
        let r = lay.resized_to(&SPEC, 0, 2, 8.0 * CELL - GAP, 4.0 * CELL - GAP);
        assert!(r.is_some());
        let lay2 = r.unwrap();
        assert!(no_overlap(&lay2.pack(&SPEC, 0)));
    }

    #[test]
    fn text_roundtrip() {
        let mut lay = Layout::default_for(&SPEC);
        assert_eq!(lay.to_text(&SPEC), "");
        lay.hide(1);
        lay.entry_mut(0).unwrap().col = 0;
        lay.entry_mut(0).unwrap().row = 4;
        let text = lay.to_text(&SPEC);
        assert_eq!(Layout::parse(&SPEC, &text), lay, "{text}");
        assert_eq!(Layout::parse(&SPEC, ""), Layout::default_for(&SPEC));
    }

    #[test]
    fn parse_survives_garbage_and_clamps() {
        let g = Layout::parse(&SPEC, "garbage - :: @@ a:99@77,55 a b:x@y --c ,, d:1@2,2 d");
        assert_eq!(g.entries.len(), SPEC.specs.len());
        assert!(no_overlap(&g.pack(&SPEC, 0)));
        assert_eq!(Layout::parse(&SPEC, "\u{0}\u{1}\n\t@-:"), Layout::default_for(&SPEC));
    }

    #[test]
    fn parse_detects_legacy_format_and_resets() {
        // Старый формат (@col без запятой): должен аккуратно сброситься в default_for
        let legacy = "a:0@0 b:1@2 c@0 d:0@2";
        assert_eq!(Layout::parse(&SPEC, legacy), Layout::default_for(&SPEC));

        let legacy_single = "a@2";
        assert_eq!(Layout::parse(&SPEC, legacy_single), Layout::default_for(&SPEC));
    }

    #[test]
    fn hidden_list_and_skipped() {
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(1);
        lay.hide(2);
        assert_eq!(lay.hidden_list(&SPEC, 0), vec![1, 2]);
        // Пропущенный виджет не попадает в hidden_list
        assert_eq!(lay.hidden_list(&SPEC, 0b0010), vec![2]);
    }

    #[test]
    fn window_pixel_and_cell_transforms() {
        let win = Window { c0: 2, r0: 1, c1: 6, r1: 5 };
        assert_eq!(win.cols(), 4);
        assert_eq!(win.rows(), 4);
        assert_eq!(win.content_w(), 4.0 * CELL - GAP);
        assert_eq!(win.content_h(), 4.0 * CELL - GAP);
        assert_eq!(win.island_w(), 4.0 * CELL - GAP + 2.0 * PAD);
        assert_eq!(win.island_h(0.0), TOP + 4.0 * CELL - GAP + PAD);

        // Перевод ячейки со сдвигом: ячейка (2, 1) -> (0.0, 0.0) px
        assert_eq!(win.cell_to_pixel(2, 1), (0.0, 0.0));
        assert_eq!(win.cell_to_pixel(3, 2), (CELL, CELL));

        // Обратный перевод: (0.0, 0.0) -> (2, 1)
        assert_eq!(win.pixel_to_cell(0.0, 0.0), (2, 1));
        assert_eq!(win.pixel_to_cell(CELL + 2.0, CELL + 2.0), (3, 2));

        // Принадлежность ячейки
        assert!(win.contains_cell(2, 1, 2, 2));
        assert!(win.contains_cell(4, 3, 2, 2));
        assert!(!win.contains_cell(1, 1, 2, 2)); // c0 < 2
        assert!(!win.contains_cell(2, 0, 2, 2)); // r0 < 1
        assert!(!win.contains_cell(5, 1, 2, 2)); // 5 + 2 > 6
        assert!(!win.contains_cell(2, 4, 2, 2)); // 4 + 2 > 5
    }

    #[test]
    fn fit_window_tight_and_respects_minimums() {
        // Раскладка по умолчанию: A (0..4, 0..2), B (4..8, 0..2), C (0..4, 2..4), D (4..8, 2..4)
        let lay = Layout::default_for(&SPEC);
        let win = fit_window(&lay.entries, &SPEC, 0, 4, 3);
        // Заняты колонки 0..8 и ряды 0..4 (ряд 4..6 свободен!)
        assert_eq!(win, Window { c0: 0, r0: 0, c1: 8, r1: 4 });

        // Если скрыть нижний ряд C и D: остаются только A и B на ряду 0..2
        let mut lay_top = lay.clone();
        lay_top.hide(2); // C
        lay_top.hide(3); // D
        let win_top = fit_window(&lay_top.entries, &SPEC, 0, 4, 3);
        // Ряд 0..2 имеет высоту 2, но min_rows=3 -> расширяется до 3 рядов (0..3)
        assert_eq!(win_top, Window { c0: 0, r0: 0, c1: 8, r1: 3 });

        // Если скрыть B и D (правая половина): остаётся только A (0..4, 0..2)
        let mut lay_left = lay.clone();
        lay_left.hide(1); // B
        lay_left.hide(2); // C
        lay_left.hide(3); // D
        let win_left = fit_window(&lay_left.entries, &SPEC, 0, 4, 3);
        // A занимает 0..4, 0..2. min_cols=4 (совпадает), min_rows=3 (расширяется вниз до 3)
        assert_eq!(win_left, Window { c0: 0, r0: 0, c1: 4, r1: 3 });

        // Если все виджеты скрыты: минимальное окно
        let mut lay_empty = lay.clone();
        for i in 0..4 {
            lay_empty.hide(i);
        }
        let win_empty = fit_window(&lay_empty.entries, &SPEC, 0, 4, 3);
        assert_eq!(win_empty, Window { c0: 0, r0: 0, c1: 4, r1: 3 });
    }

    #[test]
    fn fit_window_guarantees_all_visible_widgets_inside() {
        let lay = Layout::default_for(&SPEC);
        let win = fit_window(&lay.entries, &SPEC, 0, 6, 5);
        for e in &lay.entries {
            if !e.hidden {
                let sz = SPEC.specs[e.idx].sizes[e.size];
                assert!(win.contains_cell(e.col, e.row, sz.w, sz.h));
            }
        }
    }

    #[test]
    fn expand_side_and_can_expand() {
        let win = Window { c0: 2, r0: 1, c1: 6, r1: 4 };
        assert!(can_expand_side(win, &SPEC, Side::Left)); // c0=2 > 0
        assert!(can_expand_side(win, &SPEC, Side::Top)); // r0=1 > 0
        assert!(can_expand_side(win, &SPEC, Side::Right)); // c1=6 < 8
        assert!(can_expand_side(win, &SPEC, Side::Bottom)); // r1=4 < 6

        let w_left = expand_side(win, &SPEC, Side::Left, 4);
        assert_eq!(w_left.c0, 0); // saturating_sub(4) от 2 = 0
        assert!(!can_expand_side(w_left, &SPEC, Side::Left));

        let w_right = expand_side(win, &SPEC, Side::Right, 4);
        assert_eq!(w_right.c1, 8); // (6 + 4).min(8) = 8
        assert!(!can_expand_side(w_right, &SPEC, Side::Right));

        let w_top = expand_side(win, &SPEC, Side::Top, 4);
        assert_eq!(w_top.r0, 0);
        assert!(!can_expand_side(w_top, &SPEC, Side::Top));

        let w_bottom = expand_side(win, &SPEC, Side::Bottom, 4);
        assert_eq!(w_bottom.r1, 6);
        assert!(!can_expand_side(w_bottom, &SPEC, Side::Bottom));
    }

    #[test]
    fn stretch_window_all_sides_and_boundaries() {
        let win = Window { c0: 2, r0: 2, c1: 6, r1: 5 };
        // win.cols() = 4. content_w = 4*16 - 8 = 56. layer_w = 56 + 28 = 84.
        let layer_w = win.island_w();
        let center = layer_w / 2.0;
        assert_eq!(layer_w, 84.0);
        assert_eq!(center, 42.0);

        // Правая ручка: курсор правее центра, тянем вправо
        let press_pos = (74.5, 50.0);
        // Смещение x=86.0 -> x_c=44.0. desired_cols = round((88 - 28 + 8)/16) = round(4.25) = 4 (delta = 0)
        let w_r0 = stretch_window(Side::Right, (86.0, 50.0), press_pos, win, &SPEC, layer_w);
        assert_eq!(w_r0, win);

        // Смещение x=90.0 -> x_c=48.0. desired_cols = round((96 - 28 + 8)/16) = round(4.75) = 5 (delta = 1)
        let w_r1 = stretch_window(Side::Right, (90.0, 50.0), press_pos, win, &SPEC, layer_w);
        assert_eq!(w_r1, Window { c0: 2, r0: 2, c1: 7, r1: 5 });

        // Ограничение по spec.cols (8)
        let w_r_max = stretch_window(Side::Right, (150.0, 50.0), press_pos, win, &SPEC, layer_w);
        assert_eq!(w_r_max, Window { c0: 2, r0: 2, c1: 8, r1: 5 });

        // Движение внутрь (влево): не сжимается меньше initial_win
        let w_r_in = stretch_window(Side::Right, (50.0, 50.0), press_pos, win, &SPEC, layer_w);
        assert_eq!(w_r_in, win);

        // Левая ручка: курсор левее центра, тянем влево (x = -6.0) -> x_c = -48.0. desired_cols = 5 (delta = 1)
        let w_l1 = stretch_window(Side::Left, (-6.0, 50.0), (9.5, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_l1, Window { c0: 1, r0: 2, c1: 6, r1: 5 });

        // Левая ручка ограничена c0 = 0
        let w_l_max = stretch_window(Side::Left, (-50.0, 50.0), (9.5, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_l_max, Window { c0: 0, r0: 2, c1: 6, r1: 5 });

        // Нижняя ручка: смещение вниз на 16 px (1 ячейка)
        let w_b1 = stretch_window(Side::Bottom, (50.0, 66.0), (50.0, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_b1, Window { c0: 2, r0: 2, c1: 6, r1: 6 });

        // Нижняя ручка ограничена spec.rows (6)
        let w_b_max = stretch_window(Side::Bottom, (50.0, 100.0), (50.0, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_b_max, Window { c0: 2, r0: 2, c1: 6, r1: 6 });

        // Верхняя ручка: смещение вверх на 16 px (1 ячейка) -> r0 = 2 - 1 = 1
        let w_t1 = stretch_window(Side::Top, (50.0, 34.0), (50.0, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_t1, Window { c0: 2, r0: 1, c1: 6, r1: 5 });

        // Верхняя ручка ограничена r0 = 0
        let w_t_max = stretch_window(Side::Top, (50.0, 0.0), (50.0, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_t_max, Window { c0: 2, r0: 0, c1: 6, r1: 5 });
    }
}
