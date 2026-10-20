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

/// Шаг ячейки сетки, логических пикселей.
pub const CELL: f32 = 16.0;

/// Зазор между виджетами по горизонтали и вертикали, логических пикселей.
pub const GAP: f32 = 8.0;

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

/// Описание вкладки: максимальный размер сетки в ячейках и виджеты.
pub struct TabSpec {
    pub cols: u8,
    pub rows: u8,
    pub specs: &'static [Spec],
}

pub static EMPTY: TabSpec = TabSpec { cols: 1, rows: 1, specs: &[] };

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
/// без выхода за границы сетки и без коллизий с другими видимыми виджетами.
pub fn can_place(
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
    col: u8,
    row: u8,
) -> bool {
    let sz = spec.specs[idx].sizes[size];
    if col + sz.w > spec.cols || row + sz.h > spec.rows {
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

/// Ищет свободное место для виджета `idx` размера `size`, ближайшее к желаемой ячейке `(want_col, want_row)`.
/// При равном евклидовом расстоянии выбирается позиция в порядке чтения (меньший ряд, затем меньшая колонка).
pub fn nearest_free(
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
    want_col: u8,
    want_row: u8,
) -> Option<(u8, u8)> {
    let sz = spec.specs[idx].sizes[size];
    if sz.w > spec.cols || sz.h > spec.rows {
        return None;
    }
    let max_col = spec.cols - sz.w;
    let max_row = spec.rows - sz.h;
    let mut best: Option<(f32, u8, u8)> = None;

    for r in 0..=max_row {
        for c in 0..=max_col {
            if can_place(spec, skip, entries, idx, size, c, r) {
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

/// Первое подходящее место для виджета в порядке чтения (сверху вниз, слева направо).
pub fn first_free(
    spec: &TabSpec,
    skip: u32,
    entries: &[Entry],
    idx: usize,
    size: usize,
) -> Option<(u8, u8)> {
    let sz = spec.specs[idx].sizes[size];
    if sz.w > spec.cols || sz.h > spec.rows {
        return None;
    }
    let max_col = spec.cols - sz.w;
    let max_row = spec.rows - sz.h;
    for r in 0..=max_row {
        for c in 0..=max_col {
            if can_place(spec, skip, entries, idx, size, c, r) {
                return Some((c, r));
            }
        }
    }
    None
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

    /// Преобразует видимые виджеты в список размещённых с экранными пиксельными координатами.
    pub fn pack(&self, spec: &TabSpec, skip: u32) -> Vec<Placed> {
        let mut out = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            if e.hidden || skipped(skip, e.idx) {
                continue;
            }
            let sz = spec.specs[e.idx].sizes[e.size];
            let w_px = sz.pixel_w();
            let h_px = sz.pixel_h();
            let x = e.col as f32 * CELL;
            let y = e.row as f32 * CELL;
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

    /// Возвращает скрытый виджет из лотка: ищет первое подходящее место в порядке чтения.
    /// При нехватке места пробует меньшие размеры. `false` — места нет.
    pub fn place_new(&mut self, spec: &TabSpec, skip: u32, idx: usize) -> bool {
        let Some(pos) = self.entries.iter().position(|e| e.idx == idx) else { return false };
        let cur_size = self.entries[pos].size;
        for size in (0..=cur_size).rev() {
            if let Some((col, row)) = first_free(spec, skip, &self.entries, idx, size) {
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

    /// Следующий по кругу размер виджета, при котором он помещается на сетку (на текущем месте
    /// или со сдвигом к ближайшему свободному). `false` — менять некуда.
    pub fn cycle_size(&mut self, spec: &TabSpec, skip: u32, idx: usize, _avail: f32) -> bool {
        let Some(cur) = self.entry(idx).copied() else { return false };
        let n = spec.specs[idx].sizes.len();
        if n <= 1 {
            return false;
        }
        for step in 1..n {
            let size = (cur.size + step) % n;
            if can_place(spec, skip, &self.entries, idx, size, cur.col, cur.row) {
                if let Some(e) = self.entry_mut(idx) {
                    e.size = size;
                    return true;
                }
            }
            if let Some((col, row)) = nearest_free(spec, skip, &self.entries, idx, size, cur.col, cur.row) {
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

    /// Размер виджета, ближайший к желаемому прямоугольнику `want_w` × `want_h` (угловая ручка),
    /// из тех, при которых он помещается на сетку. `None` — ни один размер не подходит.
    pub fn resized_to(
        &self,
        spec: &TabSpec,
        skip: u32,
        idx: usize,
        want_w: f32,
        want_h: f32,
    ) -> Option<Layout> {
        let cur = self.entry(idx)?;
        let mut best: Option<(f32, Layout)> = None;
        for (size, s) in spec.specs[idx].sizes.iter().enumerate() {
            let pw = s.pixel_w();
            let ph = s.pixel_h();
            let mut cost = (pw - want_w).abs() + (ph - want_h).abs();
            if size == cur.size {
                cost -= 2.0; // Гистерезис в пользу текущего размера
            }
            if best.as_ref().is_some_and(|(c, _)| *c <= cost) {
                continue;
            }
            if can_place(spec, skip, &self.entries, idx, size, cur.col, cur.row) {
                let mut lay = self.clone();
                if let Some(e) = lay.entry_mut(idx) {
                    e.size = size;
                }
                best = Some((cost, lay));
            } else if let Some((col, row)) = nearest_free(spec, skip, &self.entries, idx, size, cur.col, cur.row) {
                // Если сдвиг небольшой (до 1 ячейки по каждой оси), разрешаем его при ресайзе
                if (col as i32 - cur.col as i32).abs() <= 1 && (row as i32 - cur.row as i32).abs() <= 1 {
                    let mut lay = self.clone();
                    if let Some(e) = lay.entry_mut(idx) {
                        e.size = size;
                        e.col = col;
                        e.row = row;
                    }
                    best = Some((cost + 4.0, lay));
                }
            }
        }
        best.map(|(_, l)| l)
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
}
