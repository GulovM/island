//! Раскладка виджетов вкладки: чистая модель квадратной сетки без рисования.
//!
//! Как в Пункте управления iOS, вкладка делится на крупные ячейки 1×1 с шагом `CELL` (56 px) и
//! зазором `GAP` (8 px), а виджет занимает целое число таких ячеек стандартного размера
//! (1×1, 2×1, 1×2, 2×2, 4×1, 4×2, 4×4 и т. д., см. `Size::is_standard`). Виджет размером `w`×`h`
//! ячеек имеет пиксельный размер `w*CELL - GAP` на `h*CELL - GAP` (то есть `56w - 8` на `56h - 8` px).
//! Позиция каждого виджета — ячейка `(col, row)`; режим уплотнения (`compact`) убирает пустоты,
//! «свободная раскладка» допускает их.
//!
//! Раскладка в `config.toml` — строка токенов `v2 [-]ключ[:размер]@col,row` через пробел, например:
//! `v2 wifi@0,0 bluetooth:1@2,0 -night:2@0,1`. Минус — виджет скрыт в лоток, номер размера — индекс
//! в `Spec::sizes`. Пустая строка означает раскладку по умолчанию. Строка без `v2` — сохранённая
//! старой версией на мелкой сетке (шаг 16 px): размеры и позиции приводятся к ближайшим стандартным.

use super::{PAD, TOP};

/// Шаг крупной ячейки 1×1, логических пикселей.
pub const CELL: f32 = 56.0;

/// Зазор между виджетами по горизонтали и вертикали, логических пикселей.
pub const GAP: f32 = 8.0;

/// Метка формата раскладки на крупной сетке в начале строки конфига.
const VERSION: &str = "v2";

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

/// Значение `Spec::row`: виджет по умолчанию лежит в лотке скрытых и не занимает места на вкладке
/// (так добавляют новые виджеты, не меняя размер острова у тех, кто уже настроил вкладку).
pub const HIDDEN: u8 = u8::MAX;

/// Размер виджета в целых ячейках сетки.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Size {
    pub w: u8,
    pub h: u8,
}

pub const fn sz(w: u8, h: u8) -> Size {
    Size { w, h }
}

/// Ширина «на весь остров» (вместо числа в `sz`): равна числу колонок острова, то есть ряду вкладок.
/// В статических спецификациях это метка; сетка подставляет настоящую ширину (`resolve_full`),
/// поэтому все расчёты раскладки видят уже число ячеек.
pub const FULL: u8 = 0;

/// Метки диапазонов в статических спецификациях. `sz(WIDE + a, h)` — все ширины от `a` ячеек до
/// ширины сетки вкладки при высоте `h` (длина виджета произвольная, толщина фиксирована);
/// `sz(w, TALL + a)` — все высоты от `a` до высоты сетки при ширине `w`. Верхней границы, кроме
/// самой сетки, нет: что не помещается в окно, просто не ставится. `resolve_full` раскрывает
/// метки в обычные размеры по порядку.
pub const WIDE: u8 = 100;
pub const TALL: u8 = 100;

impl Size {
    /// Стандартный размер виджета: ширина 1, 2, 3, 4, 6, 8 или 10 ячеек (или `FULL` — на весь
    /// остров), высота 1, 2 или 4. Спецификации виджетов берут размеры только из этого набора
    /// (проверяется тестом).
    #[cfg(test)]
    pub const fn is_standard(&self) -> bool {
        // Диапазоны (`WIDE`, `TALL`) раскрываются в произвольные длины и в стандартный набор не входят.
        (self.w >= WIDE || self.h >= TALL) || (matches!(self.w, FULL | 1 | 2 | 3 | 4 | 6 | 8 | 10) && matches!(self.h, 1 | 2 | 4))
    }

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

/// Подставляет вместо метки `FULL` ширину `cols` ячеек (не больше `spec.cols`) и раскрывает диапазоны
/// `WIDE` и `TALL` в обычные размеры. Спецификация без таких размеров возвращается как есть. Результаты кэшируются, поэтому одна и та же пара
/// (спецификация, ширина) создаёт данные один раз.
pub fn resolve_full(spec: &'static TabSpec, cols: u8) -> &'static TabSpec {
    use std::sync::Mutex;
    static CACHE: Mutex<Vec<(usize, u8, &'static TabSpec)>> = Mutex::new(Vec::new());
    if !spec.specs.iter().any(|s| s.sizes.iter().any(|z| z.w == FULL || z.w >= WIDE || z.h >= TALL)) {
        return spec;
    }
    // Меньше 4 колонок не бывает: размеры по умолчанию (3×1, 3×2) должны помещаться.
    let cols = cols.clamp(4.min(spec.cols), spec.cols);
    let key = spec as *const TabSpec as usize;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(_, _, r)) = cache.iter().find(|&&(k, c, _)| k == key && c == cols) {
        return r;
    }
    let specs: Vec<Spec> = spec
        .specs
        .iter()
        .map(|s| {
            let mut sizes: Vec<Size> = Vec::new();
            for z in s.sizes {
                if z.w == FULL {
                    sizes.push(sz(cols, z.h));
                } else if z.w >= WIDE {
                    let from = z.w - WIDE;
                    sizes.extend((from..=spec.cols.max(from)).map(|w| sz(w, z.h)));
                } else if z.h >= TALL {
                    let from = z.h - TALL;
                    sizes.extend((from..=spec.rows.max(from)).map(|h| sz(z.w, h)));
                } else {
                    sizes.push(*z);
                }
            }
            Spec { key: s.key, title: s.title, sizes: Box::leak(sizes.into_boxed_slice()), size: s.size, col: s.col, row: s.row }
        })
        .collect();
    let resolved: &'static TabSpec = Box::leak(Box::new(TabSpec {
        cols: spec.cols,
        rows: spec.rows,
        min_cols: spec.min_cols,
        min_rows: spec.min_rows,
        specs: Box::leak(specs.into_boxed_slice()),
    }));
    cache.push((key, cols, resolved));
    resolved
}

pub static EMPTY: TabSpec =TabSpec { cols: 1, rows: 1, min_cols: 1, min_rows: 1, specs: &[] };

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

/// Ручка размера виджета: угловая (правый нижний угол) и «вертикальная» (правый верхний угол,
/// только у виджетов, которые умеют стоять и лежать, — ползунков).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    Corner,
    Top,
}

/// Новый размер виджета `idx` под ручкой `handle`: размеры перебираются по близости к желаемому
/// прямоугольнику `want_w` × `want_h` (с гистерезисом в пользу `prefer`, см. `resize_order`).
/// Угловая ручка держит левый верхний угол на месте, верхняя — левый нижний (виджет растёт вверх);
/// якорь сдвигается, только если иначе виджет вылезает за окно. Соседи расталкиваются. Первый
/// размер, который удаётся разместить, и есть ответ. Считается от `base` на начало жеста.
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
    handle: Handle,
) -> Option<Resolved> {
    let cur = *base.iter().find(|e| e.idx == idx)?;
    let cur_h = spec.specs[idx].sizes[cur.size].h;
    resize_order(spec, idx, cur.size, handle, want_w, want_h, prefer).into_iter().find_map(|size| {
        let new_h = spec.specs[idx].sizes[size].h;
        let row = match handle {
            Handle::Corner => cur.row,
            Handle::Top => (i32::from(cur.row) + i32::from(cur_h) - i32::from(new_h)).max(0) as u8,
        };
        push_at(win, spec, skip, base, idx, size, cur.col, row)
    })
}

/// Ориентация размера: 1 — горизонтальный, −1 — вертикальный, 0 — квадратный.
fn orientation(s: &Size) -> i8 {
    match s.w.cmp(&s.h) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
    }
}

/// Виджет умеет и лежать, и стоять (у него есть горизонтальные и вертикальные размеры): у такого
/// две ручки, и ориентацию меняет только явный жест.
pub fn is_dual(spec: &TabSpec, idx: usize) -> bool {
    let sizes = spec.specs[idx].sizes;
    sizes.iter().any(|s| orientation(s) > 0) && sizes.iter().any(|s| orientation(s) < 0)
}

/// Размеры виджета `idx` по порядку предпочтения для жеста за ручку `handle` (первый — ответ).
/// `start` — размер на начало жеста, `prefer` — нынешний (с гистерезисом в его пользу).
/// У обычных виджетов это близость к прямоугольнику `want_w` × `want_h`. У виджетов, умеющих
/// и лежать, и стоять, угловая ручка задаёт длину лёжа, верхняя — длину стоя, а сменить ориентацию
/// можно, только протянув ручку не меньше чем на ячейку: пока короче — размер остаётся прежним.
pub fn resize_order(spec: &TabSpec, idx: usize, start: usize, handle: Handle, want_w: f32, want_h: f32, prefer: usize) -> Vec<usize> {
    if !is_dual(spec, idx) {
        return size_order(spec, idx, want_w, want_h, prefer);
    }
    let sizes = spec.specs[idx].sizes;
    let target = if handle == Handle::Corner { 1 } else { -1 };
    let from = sizes[start];
    if orientation(&from) != target {
        let moved = if handle == Handle::Corner { want_w - from.pixel_w() } else { want_h - from.pixel_h() };
        if moved < CELL {
            return vec![start];
        }
    }
    let mut ranked: Vec<(f32, usize)> = sizes
        .iter()
        .enumerate()
        .filter(|(_, s)| orientation(s) == target)
        .map(|(size, s)| {
            let cost = if handle == Handle::Corner { (s.pixel_w() - want_w).abs() } else { (s.pixel_h() - want_h).abs() };
            (if size == prefer { cost - 2.0 } else { cost }, size)
        })
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().map(|(_, size)| size).collect()
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

/// Сколько раз уплотнение повторяется, пока раскладка не перестанет меняться.
const COMPACT_PASSES: usize = 8;

/// Один проход уплотнения: видимые виджеты в порядке чтения заново занимают первое свободное место.
fn compact_pass(win: Window, spec: &TabSpec, skip: u32, entries: &[Entry]) -> Option<Vec<Entry>> {
    let mut order: Vec<usize> = (0..entries.len()).filter(|&i| active(skip, &entries[i])).collect();
    order.sort_by_key(|&i| (entries[i].row, entries[i].col, entries[i].idx));
    let mut out = entries.to_vec();
    for &i in &order {
        out[i].hidden = true;
    }
    for &i in &order {
        let e = out[i];
        let (col, row) = first_free_in_window(win, spec, skip, &out, e.idx, e.size)?;
        out[i].col = col;
        out[i].row = row;
        out[i].hidden = false;
    }
    Some(out)
}

/// Уплотнение раскладки, как в Пункте управления iOS: пустоты между виджетами схлопываются, а
/// порядок чтения (сверху вниз, слева направо) сохраняется. Виджеты сдвигаются в любую сторону,
/// но только по узлам сетки. Размеры не меняются, скрытые и недоступные виджеты не трогаются.
/// `None` — виджеты не удалось уложить в окно `win` (тогда раскладку лучше оставить как есть).
pub fn compact(win: Window, spec: &TabSpec, skip: u32, entries: &[Entry]) -> Option<Vec<Entry>> {
    let mut cur = entries.to_vec();
    for _ in 0..COMPACT_PASSES {
        let next = compact_pass(win, spec, skip, &cur)?;
        if next == cur {
            break;
        }
        cur = next;
    }
    Some(cur)
}

/// Порядок размеров для щелчка по ручке у виджета со многими размерами: сперва «следующие шаги»
/// той же ориентации (площадь не меньше чем в полтора раза больше), затем самый малый, затем остальные.
fn preset_order(spec: &TabSpec, idx: usize, cur: usize) -> Vec<usize> {
    let sizes = spec.specs[idx].sizes;
    let area = |s: &Size| u32::from(s.w) * u32::from(s.h);
    let o = orientation(&sizes[cur]);
    let same: Vec<usize> = (0..sizes.len()).filter(|&i| orientation(&sizes[i]) == o).collect();
    let mut steps: Vec<usize> = same.iter().copied().filter(|&i| area(&sizes[i]) * 2 >= area(&sizes[cur]) * 3).collect();
    if let Some(&smallest) = same.first() {
        if smallest != cur {
            steps.push(smallest);
        }
    }
    for i in (0..sizes.len()).filter(|&i| i != cur) {
        if !steps.contains(&i) {
            steps.push(i);
        }
    }
    steps
}

/// Размер из токена раскладки: номер в `Spec::sizes` (старая запись) или `WxH` в ячейках.
#[derive(Clone, Copy)]
enum SizeTok {
    Index(usize),
    Dims(u8, u8),
}

impl SizeTok {
    fn parse(text: &str) -> Option<SizeTok> {
        match text.split_once('x') {
            Some((w, h)) => Some(SizeTok::Dims(w.parse().ok()?, h.parse().ok()?)),
            None => text.parse().ok().map(SizeTok::Index),
        }
    }

    fn index(self) -> Option<usize> {
        match self {
            SizeTok::Index(n) => Some(n),
            SizeTok::Dims(..) => None,
        }
    }

    /// Индекс размера в `sizes`: номер как есть (если он есть), ячейки — точное совпадение или
    /// ближайший размер (ширина острова могла измениться); непонятное — `default`.
    fn resolve(self, sizes: &[Size], default: usize) -> usize {
        match self {
            SizeTok::Index(n) if n < sizes.len() => n,
            SizeTok::Index(_) => default,
            SizeTok::Dims(w, h) => sizes
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| (i32::from(s.w) - i32::from(w)).abs() + (i32::from(s.h) - i32::from(h)).abs())
                .map_or(default, |(i, _)| i),
        }
    }
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
                col: if s.row == HIDDEN { 0 } else { s.col },
                row: if s.row == HIDDEN { 0 } else { s.row },
                hidden: s.row == HIDDEN,
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

        let mut tokens = text.split(|c: char| c.is_whitespace() || c == ';').filter(|t| !t.is_empty()).peekable();
        // Без метки версии строка записана старой версией на мелкой сетке: приводим её к крупной.
        let migrate = tokens.next_if_eq(&VERSION).is_none();

        let mut out: Vec<Entry> = Vec::new();
        for tok in tokens {
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
                Some((a, b)) => (a, SizeTok::parse(b)),
                None => (rest, None),
            };
            let Some(idx) = spec.specs.iter().position(|s| s.key == key) else { continue };
            if out.iter().any(|e| e.idx == idx) {
                continue;
            }
            let s = &spec.specs[idx];
            let (size, coords) = if migrate {
                (
                    super::legacy::migrate_size(key, s.sizes, size.and_then(SizeTok::index))
                        .unwrap_or_else(|| size.map_or(s.size, |t| t.resolve(s.sizes, s.size))),
                    coords.map(|(c, r)| (super::legacy::migrate_cell(c), super::legacy::migrate_cell(r))),
                )
            } else {
                (size.map_or(s.size, |t| t.resolve(s.sizes, s.size)), coords)
            };
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
            if s.row == HIDDEN {
                out.push(Entry { idx, size: s.size, col: 0, row: 0, hidden: true });
            } else if can_place(spec, 0, &out, idx, s.size, s.col, s.row) {
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
        let mut parts = Vec::with_capacity(self.entries.len() + 1);
        parts.push(VERSION.to_string());
        for e in &self.entries {
            let s = &spec.specs[e.idx];
            // Лежит в лотке по умолчанию и не тронут: разбор вернёт его туда же, место в строке не нужно.
            if e.hidden && s.row == HIDDEN && e.size == s.size {
                continue;
            }
            let mut t = String::new();
            if e.hidden {
                t.push('-');
            }
            t.push_str(s.key);
            if e.size != s.size {
                // Размер записывается ячейками, а не номером: номера плавают, когда меняется
                // ширина острова и вместе с ней список размеров «на весь остров».
                let z = s.sizes[e.size];
                t.push_str(&format!(":{}x{}", z.w, z.h));
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
        // У виджетов с произвольной длиной размеров много: щелчок по ручке перебирает лишь «шаги»,
        // каждый заметно больше прежнего, а после самого большого возвращается к самому малому.
        let order: Vec<usize> = if n > 6 { preset_order(spec, idx, cur.size) } else { (1..n).map(|step| (cur.size + step) % n).collect() };
        for size in order {
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

    /// Щелчок по вертикальной ручке: виджет ложится или встаёт, сохраняя длину (в ячейках, сколько
    /// позволяют размеры). Растёт вверх от нижнего края, когда встаёт, и вправо от левого верха,
    /// когда ложится. `false` — места нет.
    pub fn flip_orientation_in_window(&mut self, win: Window, spec: &TabSpec, skip: u32, idx: usize) -> bool {
        let Some(cur) = self.entry(idx).copied() else { return false };
        let sizes = spec.specs[idx].sizes;
        let from = sizes[cur.size];
        let target = -orientation(&from);
        let len = i32::from(from.w.max(from.h));
        let mut cands: Vec<usize> = (0..sizes.len()).filter(|&i| orientation(&sizes[i]) == target).collect();
        cands.sort_by_key(|&i| ((i32::from(sizes[i].w.max(sizes[i].h)) - len).abs(), i));
        for size in cands {
            let new = sizes[size];
            let row = if target < 0 { (i32::from(cur.row) + i32::from(from.h) - i32::from(new.h)).max(0) as u8 } else { cur.row };
            if let Some(res) = push_at(win, spec, skip, &self.entries, idx, size, cur.col, row) {
                self.entries = res.entries;
                return true;
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
        resolve_resize(win, spec, skip, &self.entries, idx, want_w, want_h, prefer, Handle::Corner).map(|r| Layout { entries: r.entries })
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // в тестах описания вкладок названы как константы: C, T
mod tests {
    use super::*;

    /// Описание вкладки с раскрытыми размерами «на весь остров» и диапазонами (остров в 8 колонок).
    fn real(spec: &'static TabSpec) -> &'static TabSpec {
        resolve_full(spec, 8)
    }

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
        let C = real(&crate::tabs::controls::SPEC);
        // «Управление» по умолчанию занимает всё окно: Wi-Fi чуть правее своего места
        // не может растолкать соседей и остаётся где был.
        let base = Layout::default_for(C).entries;
        let win = fit_window(&base, C, 0, C.min_cols, C.min_rows);
        let r = resolve_drop(win, C, 0, &base, 0, 1, (1, 0)).unwrap();
        assert_eq!(pos(&r, 0), (0, 0));
        assert_eq!(r.pushed, 0);
        assert_eq!(r.entries, base);
    }

    #[test]
    fn drop_over_neighbour_swaps() {
        let C = real(&crate::tabs::controls::SPEC);
        let base = Layout::default_for(C).entries;
        let win = fit_window(&base, C, 0, C.min_cols, C.min_rows);
        // Wi-Fi прямо над Bluetooth: меняются местами
        let r = resolve_drop(win, C, 0, &base, 0, 1, (2, 0)).unwrap();
        assert_eq!(pos(&r, 0), (2, 0));
        assert_eq!(pos(&r, 1), (0, 0));
        assert_eq!(r.pushed, 1);
        check(win, C, 0, &base, 0, &r);
    }

    #[test]
    fn drop_big_widget_reflows_neighbours() {
        let C = real(&crate::tabs::controls::SPEC);
        let base = Layout::default_for(C).entries;
        let win = fit_window(&base, C, 0, C.min_cols, C.min_rows);
        // Плеер 3×2 наверх: плитки и ползунки уезжают вниз, в окно 6×4 влезают все
        let media = 7;
        let size = base[media].size;
        let r = resolve_drop(win, C, 0, &base, media, size, (0, 0)).unwrap();
        // Окно занято целиком, и самая дешёвая расстановка ставит плеер не выше второго ряда
        assert_eq!(pos(&r, media).0, 0);
        assert!(pos(&r, media).1 <= 1);
        assert!(r.pushed > 0);
        check(win, C, 0, &base, media, &r);
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
        let C = real(&crate::tabs::controls::SPEC);
        let base = Layout::default_for(C).entries;
        let win = fit_window(&base, C, 0, C.min_cols, C.min_rows);
        let first = resolve_drop(win, C, 0, &base, 0, 1, (2, 0));
        let _ = resolve_drop(win, C, 0, &base, 0, 1, (4, 1));
        let _ = resolve_drop(win, C, 0, &base, 0, 1, (0, 3));
        assert_eq!(resolve_drop(win, C, 0, &base, 0, 1, (2, 0)), first);
        // Обратно в исходную ячейку — исходная раскладка
        let home = resolve_drop(win, C, 0, &base, 0, 1, (0, 0)).unwrap();
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
        let C = real(&crate::tabs::controls::SPEC);
        let base = Layout::default_for(C).entries;
        let win = fit_window(&base, C, 0, C.min_cols, C.min_rows);
        let r = resolve_drop(win, C, 0, &base, 7, base[7].size, (0, 0)).unwrap();
        let lay = Layout { entries: r.entries };
        assert_eq!(Layout::parse(C, &lay.to_text(C)), lay);
    }

    /// Все виджеты настоящих вкладок над каждой ячейкой окна: виджет всегда куда-то встаёт,
    /// раскладка корректна, паники нет. Раскладка по умолчанию занимает окно целиком, поэтому
    /// вторая — с половиной скрытых виджетов, где есть куда вытеснять.
    #[test]
    fn real_tabs_always_resolve() {
        use crate::tabs::{controls, stats, today};
        for spec in [real(&controls::SPEC), real(&today::SPEC), real(&stats::SPEC)] {
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
        let r = resolve_resize(full(&S), &S, 0, &base, 0, GROW[1].pixel_w(), GROW[1].pixel_h(), 0, Handle::Corner).unwrap();
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
        let r = resolve_resize(full(&S), &S, 0, &base, 0, GROW[1].pixel_w(), GROW[1].pixel_h(), 0, Handle::Corner).unwrap();
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
        let r = resolve_resize(full(&S), &S, 0, &base, 0, GROW[1].pixel_w(), GROW[1].pixel_h(), 0, Handle::Corner).unwrap();
        assert_eq!(r.entries, base);
    }

    #[test]
    fn cycle_size_pushes_neighbours() {
        let C = real(&crate::tabs::controls::SPEC);
        let mut lay = Layout::default_for(C);
        let win = Window { c0: 0, r0: 0, c1: C.cols, r1: C.rows };
        // Окно занято целиком; спрячем плеер, чтобы соседям было куда уйти
        lay.hide(7);
        let before = lay.clone();
        // Wi-Fi 8×3 → 12×3 растёт на месте: Bluetooth и Аудио расталкиваются, Wi-Fi не переезжает
        assert!(lay.cycle_size_in_window(win, C, 0, 0));
        let wifi = lay.entry(0).unwrap();
        assert_eq!((wifi.size, wifi.col, wifi.row), (2, 0, 0));
        assert_ne!(lay, before);
        assert!(no_overlap(&lay.pack(C, 0)));
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
        // win.cols() = 4. content_w = 4*56 - 8 = 216. layer_w = 216 + 28 = 244.
        let layer_w = win.island_w();
        let center = layer_w / 2.0;
        assert_eq!(layer_w, 244.0);
        assert_eq!(center, 122.0);

        // Правая ручка: курсор правее центра, тянем вправо
        let press_pos = (240.0, 50.0);
        // Смещение x=244.0 -> x_c=122.0. desired_cols = round((244 - 28 + 8)/56) = round(4.0) = 4 (delta = 0)
        let w_r0 = stretch_window(Side::Right, (244.0, 50.0), press_pos, win, &SPEC, layer_w);
        assert_eq!(w_r0, win);

        // Смещение x=272.0 -> x_c=150.0. desired_cols = round((300 - 28 + 8)/56) = round(5.0) = 5 (delta = 1)
        let w_r1 = stretch_window(Side::Right, (272.0, 50.0), press_pos, win, &SPEC, layer_w);
        assert_eq!(w_r1, Window { c0: 2, r0: 2, c1: 7, r1: 5 });

        // Ограничение по spec.cols (8)
        let w_r_max = stretch_window(Side::Right, (700.0, 50.0), press_pos, win, &SPEC, layer_w);
        assert_eq!(w_r_max, Window { c0: 2, r0: 2, c1: 8, r1: 5 });

        // Движение внутрь (влево): не сжимается меньше initial_win
        let w_r_in = stretch_window(Side::Right, (150.0, 50.0), press_pos, win, &SPEC, layer_w);
        assert_eq!(w_r_in, win);

        // Левая ручка: курсор левее центра, тянем влево (x = -28.0) -> x_c = -150.0. desired_cols = 5 (delta = 1)
        let w_l1 = stretch_window(Side::Left, (-28.0, 50.0), (9.5, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_l1, Window { c0: 1, r0: 2, c1: 6, r1: 5 });

        // Левая ручка ограничена c0 = 0
        let w_l_max = stretch_window(Side::Left, (-400.0, 50.0), (9.5, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_l_max, Window { c0: 0, r0: 2, c1: 6, r1: 5 });

        // Нижняя ручка: смещение вниз на 56 px (1 ячейка)
        let w_b1 = stretch_window(Side::Bottom, (50.0, 106.0), (50.0, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_b1, Window { c0: 2, r0: 2, c1: 6, r1: 6 });

        // Нижняя ручка ограничена spec.rows (6)
        let w_b_max = stretch_window(Side::Bottom, (50.0, 400.0), (50.0, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_b_max, Window { c0: 2, r0: 2, c1: 6, r1: 6 });

        // Верхняя ручка: смещение вверх на 56 px (1 ячейка) -> r0 = 2 - 1 = 1
        let w_t1 = stretch_window(Side::Top, (50.0, -6.0), (50.0, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_t1, Window { c0: 2, r0: 1, c1: 6, r1: 5 });

        // Верхняя ручка ограничена r0 = 0
        let w_t_max = stretch_window(Side::Top, (50.0, -300.0), (50.0, 50.0), win, &SPEC, layer_w);
        assert_eq!(w_t_max, Window { c0: 2, r0: 0, c1: 6, r1: 5 });
    }

    // ---------- Ползунки: произвольная длина и две ручки ----------

    static DUAL: [Size; 2] = [sz(WIDE + 2, 1), sz(1, TALL + 2)];
    static DUAL_SPEC: TabSpec = TabSpec {
        cols: 8,
        rows: 4,
        min_cols: 2,
        min_rows: 1,
        specs: &[Spec { key: "s", title: "S", sizes: &DUAL, size: 1, col: 0, row: 0 }],
    };

    fn dual() -> &'static TabSpec {
        resolve_full(&DUAL_SPEC, 8)
    }

    fn sz_of(spec: &TabSpec, i: usize) -> Size {
        spec.specs[0].sizes[i]
    }

    #[test]
    fn ranges_expand_into_every_length() {
        let d = dual();
        let ws: Vec<u8> = d.specs[0].sizes.iter().filter(|z| z.h == 1).map(|z| z.w).collect();
        assert_eq!(ws, (2..=8).collect::<Vec<u8>>());
        let hs: Vec<u8> = d.specs[0].sizes.iter().filter(|z| z.w == 1).map(|z| z.h).collect();
        assert_eq!(hs, (2..=4).collect::<Vec<u8>>());
        // По умолчанию (индекс 1) — лёжа 3×1
        assert_eq!(sz_of(d, 1), sz(3, 1));
        assert!(is_dual(d, 0));
        // Длину ограничивает только сетка вкладки, а не ширина острова
        assert_eq!(resolve_full(&DUAL_SPEC, 6).specs[0].sizes.iter().map(|z| z.w).max(), Some(8));
        assert_eq!(resolve_full(&DUAL_SPEC, 40).specs[0].sizes.iter().map(|z| z.w).max(), Some(8));
    }

    #[test]
    fn corner_handle_sets_horizontal_length_freely() {
        let d = dual();
        let start = 1; // 3×1
        for (want, w) in [(104.0, 2), (160.0, 3), (270.0, 5), (500.0, 8)] {
            let order = resize_order(d, 0, start, Handle::Corner, want, 48.0, start);
            assert_eq!((sz_of(d, order[0]).w, sz_of(d, order[0]).h), (w, 1), "want {want}");
        }
    }

    #[test]
    fn orientation_flip_needs_a_deliberate_drag() {
        let d = dual();
        let start = 1; // лёжа 3×1 (160×48)
        // Верхняя ручка чуть вверх: размер прежний
        for up in [0.0, 10.0, 30.0, 50.0] {
            let order = resize_order(d, 0, start, Handle::Top, 160.0, 48.0 + up, start);
            assert_eq!(order, vec![start], "up = {up}");
        }
        // Протянули на ячейку и дальше: встаёт, длина следует за ручкой
        let order = resize_order(d, 0, start, Handle::Top, 160.0, 48.0 + 60.0, start);
        assert_eq!(sz_of(d, order[0]), sz(1, 2));
        let order = resize_order(d, 0, start, Handle::Top, 160.0, 48.0 + 180.0, start);
        assert_eq!(sz_of(d, order[0]), sz(1, 4));
        // Стоящий 1×3: угловая ручка чуть вправо ничего не меняет, на ячейку и дальше — ложится
        let tall = d.specs[0].sizes.iter().position(|z| *z == sz(1, 3)).unwrap();
        let order = resize_order(d, 0, tall, Handle::Corner, 48.0 + 40.0, 160.0, tall);
        assert_eq!(order, vec![tall]);
        let order = resize_order(d, 0, tall, Handle::Corner, 48.0 + 120.0, 160.0, tall);
        assert_eq!(sz_of(d, order[0]).h, 1);
        assert_eq!(sz_of(d, order[0]).w, 3);
    }

    #[test]
    fn top_handle_keeps_the_bottom_edge() {
        let d = dual();
        let base = vec![Entry { idx: 0, size: 1, col: 2, row: 3, hidden: false }];
        let win = Window { c0: 0, r0: 0, c1: 8, r1: 4 };
        let r = resolve_resize(win, d, 0, &base, 0, 160.0, 48.0 + 180.0, 1, Handle::Top).unwrap();
        let e = r.entries[0];
        assert_eq!(sz_of(d, e.size), sz(1, 4));
        // Нижний край стоял в ряду 3 и остался там: виджет вытянулся вверх
        assert_eq!((e.col, e.row), (2, 0));
    }

    #[test]
    fn flip_click_keeps_length_and_fits_the_window() {
        let d = dual();
        let win = Window { c0: 0, r0: 0, c1: 8, r1: 4 };
        let mut lay = Layout { entries: vec![Entry { idx: 0, size: 1, col: 0, row: 3, hidden: false }] };
        assert!(lay.flip_orientation_in_window(win, d, 0, 0));
        let z = sz_of(d, lay.entries[0].size);
        assert!(z.h > z.w, "{}×{}", z.w, z.h);
        assert_eq!(lay.entries[0].row + z.h, 4);
        assert!(lay.flip_orientation_in_window(win, d, 0, 0));
        assert!(sz_of(d, lay.entries[0].size).w > 1);
    }

    #[test]
    fn click_cycles_through_a_few_steps_for_long_lists() {
        let d = dual();
        let win = Window { c0: 0, r0: 0, c1: 8, r1: 4 };
        let mut lay = Layout { entries: vec![Entry { idx: 0, size: 0, col: 0, row: 0, hidden: false }] };
        let mut seen = vec![];
        for _ in 0..8 {
            assert!(lay.cycle_size_in_window(win, d, 0, 0));
            seen.push(lay.entries[0].size);
        }
        // Шаги заметно больше прежнего, а после самого большого — снова самый малый
        let w: Vec<u8> = seen.iter().map(|&i| sz_of(d, i).w).collect();
        assert!(w.contains(&2) && w.contains(&8), "{w:?}");
        assert!(w.windows(2).all(|p| p[1] == 2 || p[1] > p[0]), "{w:?}");
    }

    #[test]
    fn size_text_survives_changing_island_width() {
        let d = dual();
        let text = Layout { entries: vec![Entry { idx: 0, size: d.specs[0].sizes.iter().position(|z| *z == sz(6, 1)).unwrap(), col: 1, row: 2, hidden: false }] }.to_text(d);
        assert!(text.contains(":6x1"), "{text}");
        // Ширина острова длину не меняет
        let narrow = resolve_full(&DUAL_SPEC, 4);
        let lay = Layout::parse(narrow, &text);
        assert_eq!(sz_of(narrow, lay.entries[0].size).w, 6);
        // А номер по-старому тоже читается
        assert_eq!(Layout::parse(d, "v2 s:2").entries[0].size, 2);
    }

    #[test]
    fn real_sliders_have_vertical_sizes_one_cell_wide() {
        let c = real(&crate::tabs::controls::SPEC);
        for key in ["volume", "brightness"] {
            let s = c.specs.iter().find(|s| s.key == key).unwrap();
            let vertical: Vec<_> = s.sizes.iter().filter(|z| z.h > z.w).collect();
            assert!(!vertical.is_empty(), "{key}");
            assert!(vertical.iter().all(|z| z.w == 1), "{key}: вертикальный ползунок шире одной ячейки");
            assert!(vertical.iter().any(|z| z.h == c.rows), "{key}: нет ползунка на всю высоту сетки");
        }
        let media = c.specs.iter().find(|s| s.key == "media").unwrap();
        assert!(media.sizes.iter().any(|z| z.h == 2 && z.w == c.cols), "плеер на всю ширину сетки");
        assert!(!is_dual(c, 7), "у плеера одна ручка");
    }

    // ---------- Крупная сетка: стандартные размеры, уплотнение, миграция ----------

    #[test]
    fn cell_is_a_large_square_unit() {
        assert_eq!(CELL, 56.0);
        // Плитка 1×1 — квадрат 48 px, 2×1 вдвое шире с зазором между ячейками.
        assert_eq!((sz(1, 1).pixel_w(), sz(1, 1).pixel_h()), (48.0, 48.0));
        assert_eq!(sz(2, 1).pixel_w(), 104.0);
        assert_eq!(sz(4, 4).pixel_w(), sz(4, 4).pixel_h());
    }

    #[test]
    fn standard_sizes_are_recognised() {
        for (w, h) in [(1, 1), (2, 1), (1, 2), (2, 2), (4, 1), (1, 4), (4, 2), (2, 4), (4, 4), (6, 2), (10, 4), (FULL, 1), (FULL, 2)] {
            assert!(sz(w, h).is_standard(), "{w}×{h}");
        }
        for (w, h) in [(5, 2), (7, 1), (2, 3), (2, 5), (11, 4), (1, 0)] {
            assert!(!sz(w, h).is_standard(), "{w}×{h}");
        }
    }

    /// Раскладка для проверок уплотнения: на сетке 4×4 стоят виджеты A–D (по sizes `SIZES_A`/`SIZES_B`)
    /// не плотно, с пустотами.
    static PACK_SIZES: [Size; 2] = [sz(2, 1), sz(2, 2)];
    static PACK: TabSpec = TabSpec {
        cols: 4,
        rows: 6,
        min_cols: 2,
        min_rows: 1,
        specs: &[
            Spec { key: "a", title: "A", sizes: &PACK_SIZES, size: 0, col: 0, row: 0 },
            Spec { key: "b", title: "B", sizes: &PACK_SIZES, size: 0, col: 2, row: 0 },
            Spec { key: "c", title: "C", sizes: &PACK_SIZES, size: 1, col: 0, row: 1 },
            Spec { key: "d", title: "D", sizes: &PACK_SIZES, size: 0, col: 2, row: 1 },
        ],
    };

    fn pack_win() -> Window {
        Window { c0: 0, r0: 0, c1: 4, r1: 6 }
    }

    #[test]
    fn compact_closes_gaps_in_reading_order() {
        // A(0,0) B(2,0) C(0,3) D(2,5): между рядами пустоты
        let base = vec![at(0, 0, 0), at(1, 2, 0), Entry { size: 1, ..at(2, 0, 3) }, at(3, 2, 5)];
        let out = compact(pack_win(), &PACK, 0, &base).unwrap();
        let place = |i: usize| {
            let e = out.iter().find(|e| e.idx == i).unwrap();
            (e.col, e.row)
        };
        assert_eq!(place(0), (0, 0));
        assert_eq!(place(1), (2, 0));
        // C (2×2) поднимается к A и B, D встаёт рядом с C
        assert_eq!(place(2), (0, 1));
        assert_eq!(place(3), (2, 1));
        assert!(no_overlap(&Layout { entries: out }.pack(&PACK, 0)));
    }

    #[test]
    fn compact_is_idempotent_and_keeps_sizes_and_hidden() {
        let mut base = vec![at(0, 2, 4), at(1, 0, 2), Entry { size: 1, ..at(2, 2, 2) }, at(3, 0, 0)];
        base[1].hidden = true;
        let once = compact(pack_win(), &PACK, 0, &base).unwrap();
        let twice = compact(pack_win(), &PACK, 0, &once).unwrap();
        assert_eq!(once, twice);
        for (b, o) in base.iter().zip(&once) {
            assert_eq!((b.idx, b.size, b.hidden), (o.idx, o.size, o.hidden));
        }
        // Скрытый виджет остался на своём месте
        assert_eq!((once[1].col, once[1].row), (0, 2));
    }

    #[test]
    fn compact_skips_unavailable_widgets() {
        let base = vec![at(0, 0, 0), at(1, 2, 3), Entry { size: 1, ..at(2, 0, 3) }, at(3, 2, 5)];
        // Виджет B недоступен (маска): остальные уплотняются как будто его нет, он не двигается
        let out = compact(pack_win(), &PACK, 0b0010, &base).unwrap();
        assert_eq!((out[1].col, out[1].row), (2, 3));
        let p = Layout { entries: out }.pack(&PACK, 0b0010);
        assert!(no_overlap(&p));
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn compact_reports_when_nothing_fits() {
        // Окно 2×1 вмещает один виджет 2×1, два — нет
        let tiny = Window { c0: 0, r0: 0, c1: 2, r1: 1 };
        let base = vec![at(0, 0, 0), at(1, 0, 0)];
        assert_eq!(compact(tiny, &PACK, 0b1100, &base), None);
    }

    #[test]
    fn compact_default_layouts_are_already_dense() {
        use crate::tabs::{controls, stats, today};
        for spec in [real(&controls::SPEC), real(&today::SPEC), real(&stats::SPEC)] {
            let base = Layout::default_for(spec).entries;
            let win = fit_window(&base, spec, 0, spec.min_cols, spec.min_rows);
            assert_eq!(compact(win, spec, 0, &base).as_deref(), Some(base.as_slice()), "{}", spec.specs[0].key);
        }
    }

    #[test]
    fn compact_keeps_everything_inside_the_window_for_real_tabs() {
        use crate::tabs::{controls, stats, today};
        for spec in [real(&controls::SPEC), real(&today::SPEC), real(&stats::SPEC)] {
            // Половина виджетов скрыта, остальные разбросаны по краям: после уплотнения тесно и без пересечений
            let mut lay = Layout::default_for(spec);
            for i in (1..spec.specs.len()).step_by(2) {
                lay.hide(i);
            }
            let win = Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows };
            let out = compact(win, spec, 0, &lay.entries).unwrap();
            let placed = Layout { entries: out }.pack(spec, 0);
            assert!(no_overlap(&placed));
            for p in &placed {
                assert!(win.contains_cell(p.col, p.row, p.w, p.h));
            }
        }
    }

    #[test]
    fn v2_text_roundtrip_and_marker() {
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(1);
        lay.entry_mut(0).unwrap().row = 4;
        let text = lay.to_text(&SPEC);
        assert!(text.starts_with("v2 "), "{text}");
        assert_eq!(Layout::parse(&SPEC, &text), lay);
        // Одна метка без виджетов — раскладка по умолчанию
        assert_eq!(Layout::parse(&SPEC, "v2"), Layout::default_for(&SPEC));
    }

    #[test]
    fn v2_text_is_not_migrated() {
        // Новый формат читается как есть: координаты — уже крупные ячейки
        let lay = Layout::parse(&PACK, "v2 a@2,5 b@0,0");
        assert_eq!((lay.entry(0).unwrap().col, lay.entry(0).unwrap().row), (2, 5));
    }

    #[test]
    fn legacy_text_is_migrated_to_large_cells() {
        let C = real(&crate::tabs::controls::SPEC);
        // Старый конфиг: Wi-Fi 8×3 (индекс 1) в (0,0), Bluetooth 12×3 (индекс 2) в (8,0), Яркость 24×2 в (0,14)
        let lay = Layout::parse(C, "wifi@0,0 bluetooth:2@8,0 brightness:1@0,14 -night");
        let wifi = lay.entry(0).unwrap();
        // 120×40 → ближайший стандартный 2×1 (120×56)
        assert_eq!(C.specs[0].sizes[wifi.size], sz(2, 1));
        assert_eq!((wifi.col, wifi.row), (0, 0));
        // 184×40 → 3×1; позиция (8,0) → колонка 2
        let bt = lay.entry(1).unwrap();
        assert_eq!(C.specs[1].sizes[bt.size], sz(3, 1));
        assert_eq!(bt.col, 2);
        // Скрытый виджет остаётся скрытым
        assert!(lay.entry(3).unwrap().hidden);
        // Все виджеты на месте и не пересекаются
        assert_eq!(lay.entries.len(), C.specs.len());
        assert!(no_overlap(&lay.pack(C, 0)));
        assert!(lay.fits(C, 0, 0.0));
        // После записи формат уже новый
        assert!(lay.to_text(C).starts_with("v2 "));
    }

    #[test]
    fn legacy_default_size_without_index_is_migrated() {
        let T = real(&crate::tabs::today::SPEC);
        // «clock» без индекса в старой версии значило размер по умолчанию (8×10): высокие часы → 2×4 или 2×2
        let lay = Layout::parse(T, "clock@0,0");
        let clock = lay.entry(0).unwrap();
        let z = T.specs[0].sizes[clock.size];
        assert!(z.is_standard() && z.w == 2, "{}×{}", z.w, z.h);
    }

    #[test]
    fn real_tab_layouts_survive_text_roundtrip_after_compaction() {
        use crate::tabs::{controls, stats, today};
        for spec in [real(&controls::SPEC), real(&today::SPEC), real(&stats::SPEC)] {
            let mut lay = Layout::default_for(spec);
            lay.hide(0);
            let win = Window { c0: 0, r0: 0, c1: spec.cols, r1: spec.rows };
            lay.entries = compact(win, spec, 0, &lay.entries).unwrap();
            assert_eq!(Layout::parse(spec, &lay.to_text(spec)), lay);
        }
    }
}
