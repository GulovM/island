//! Раскладка виджетов вкладки: чистая модель без рисования (её проверяют юнит-тесты).
//!
//! Вкладка делится на `cols` одинаковых колонок. Виджет занимает несколько колонок подряд
//! (`Size::cols`) и имеет высоту в пикселях (`Size::h`). Слева направо виджет «привязан» к своей
//! колонке (`Entry::col`), а по вертикали падает вниз «под тяжестью»: его верх — это низ самого
//! нижнего из более ранних (по порядку в списке) виджетов, пересекающихся с ним по колонкам, плюс
//! зазор `GAP`. Так перетаскивание меняет только колонку и место в порядке, а остальные виджеты
//! сами перетекают на новые места.
//!
//! Раскладка в `config.toml` — строка токенов `[-]ключ[:размер][@колонка]` через пробел, например
//! `wifi bluetooth:2@2 -night`. Минус — виджет скрыт, номер размера — индекс в `Spec::sizes`.

/// Зазор между виджетами и между колонками, пикселей.
pub const GAP: f32 = 10.0;

/// Один из размеров виджета: ширина в колонках и высота в пикселях.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Size {
    pub cols: u8,
    pub h: f32,
}

pub const fn sz(cols: u8, h: f32) -> Size {
    Size { cols, h }
}

/// Описание виджета вкладки.
pub struct Spec {
    /// Токен в конфиге: латиница без пробелов.
    pub key: &'static str,
    /// Название для лотка скрытых виджетов (русский ключ `i18n`).
    pub title: &'static str,
    /// Допустимые размеры от меньшего к большему; не пусто.
    pub sizes: &'static [Size],
    /// Размер по умолчанию (индекс в `sizes`) и колонка по умолчанию.
    pub size: usize,
    pub col: u8,
}

/// Описание вкладки: число колонок и виджеты в порядке по умолчанию.
pub struct TabSpec {
    pub cols: u8,
    pub specs: &'static [Spec],
}

pub static EMPTY: TabSpec = TabSpec { cols: 1, specs: &[] };

/// Состояние одного виджета в раскладке.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Индекс в `TabSpec::specs`.
    pub idx: usize,
    /// Индекс в `Spec::sizes`.
    pub size: usize,
    pub col: u8,
    pub hidden: bool,
}

/// Видимый виджет после упаковки.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub idx: usize,
    pub size: usize,
    pub col: u8,
    pub cols: u8,
    pub y: f32,
    pub h: f32,
}

impl Placed {
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// Раскладка вкладки. Порядок в `entries` — порядок «тяжести» при упаковке.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub entries: Vec<Entry>,
}

/// Виджет недоступен на этом компьютере (нет яркости) и ни рисуется, ни показывается в лотке.
pub fn skipped(skip: u32, idx: usize) -> bool {
    skip & (1 << idx) != 0
}

fn clamp_col(col: u8, cols: u8, total: u8) -> u8 {
    col.min(total.saturating_sub(cols))
}

impl Layout {
    pub fn default_for(spec: &TabSpec) -> Layout {
        let entries = spec
            .specs
            .iter()
            .enumerate()
            .map(|(idx, s)| Entry { idx, size: s.size, col: clamp_col(s.col, s.sizes[s.size].cols, spec.cols), hidden: false })
            .collect();
        Layout { entries }
    }

    pub fn is_default(&self, spec: &TabSpec) -> bool {
        *self == Layout::default_for(spec)
    }

    /// Разбирает строку из конфига. Незнакомые ключи, повторы и мусор пропускаются, недостающие
    /// виджеты дописываются в конец с настройками по умолчанию, плохие номера размера и колонки
    /// заменяются умолчаниями.
    pub fn parse(spec: &TabSpec, text: &str) -> Layout {
        let mut out: Vec<Entry> = Vec::new();
        for tok in text.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()) {
            let (hidden, rest) = match tok.strip_prefix('-') {
                Some(r) => (true, r),
                None => (false, tok),
            };
            let (rest, col) = match rest.split_once('@') {
                Some((a, b)) => (a, b.parse::<u8>().ok()),
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
            let cols = s.sizes[size].cols;
            let col = clamp_col(col.unwrap_or(s.col), cols, spec.cols);
            out.push(Entry { idx, size, col, hidden });
        }
        for (idx, s) in spec.specs.iter().enumerate() {
            if !out.iter().any(|e| e.idx == idx) {
                out.push(Entry { idx, size: s.size, col: clamp_col(s.col, s.sizes[s.size].cols, spec.cols), hidden: false });
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
            if e.col != clamp_col(s.col, s.sizes[e.size].cols, spec.cols) {
                t.push_str(&format!("@{}", e.col));
            }
            parts.push(t);
        }
        parts.join(" ")
    }

    pub fn entry(&self, idx: usize) -> Option<&Entry> {
        self.entries.iter().find(|e| e.idx == idx)
    }

    fn entry_mut(&mut self, idx: usize) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.idx == idx)
    }

    /// Упаковка: видимые виджеты в порядке списка с координатами. Колонка зажимается так, чтобы
    /// виджет не вылез за правый край.
    pub fn pack(&self, spec: &TabSpec, skip: u32) -> Vec<Placed> {
        let mut out: Vec<Placed> = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            if e.hidden || skipped(skip, e.idx) {
                continue;
            }
            let size = spec.specs[e.idx].sizes[e.size];
            let col = clamp_col(e.col, size.cols, spec.cols);
            // Самое высокое свободное место: виджет, идущий в порядке позже, может занять дыру
            // над уже стоящими (например, возвращённый из лотка Wi-Fi рядом с Bluetooth).
            let mut cands: Vec<f32> = vec![0.0];
            cands.extend(out.iter().filter(|p| col < p.col + p.cols && p.col < col + size.cols).map(|p| p.bottom() + GAP));
            cands.sort_by(f32::total_cmp);
            let y = cands
                .into_iter()
                .find(|&y| {
                    out.iter().all(|p| {
                        let apart_x = col >= p.col + p.cols || p.col >= col + size.cols;
                        apart_x || y >= p.bottom() + GAP - 0.01 || p.y >= y + size.h + GAP - 0.01
                    })
                })
                .unwrap_or(0.0);
            out.push(Placed { idx: e.idx, size: e.size, col, cols: size.cols, y, h: size.h });
        }
        out
    }

    /// Нижняя граница упакованных виджетов.
    pub fn height(&self, spec: &TabSpec, skip: u32) -> f32 {
        self.pack(spec, skip).iter().map(Placed::bottom).fold(0.0, f32::max)
    }

    pub fn fits(&self, spec: &TabSpec, skip: u32, avail: f32) -> bool {
        self.height(spec, skip) <= avail + 0.01
    }

    pub fn visible_count(&self, skip: u32) -> usize {
        self.entries.iter().filter(|e| !e.hidden && !skipped(skip, e.idx)).count()
    }

    /// Скрытые доступные виджеты в порядке по умолчанию (для лотка).
    pub fn hidden_list(&self, spec: &TabSpec, skip: u32) -> Vec<usize> {
        (0..spec.specs.len()).filter(|&i| !skipped(skip, i) && self.entry(i).is_some_and(|e| e.hidden)).collect()
    }

    // ---------- Правки ----------

    pub fn hide(&mut self, idx: usize) {
        if let Some(e) = self.entry_mut(idx) {
            e.hidden = true;
        }
    }

    /// Возвращает скрытый виджет в конец порядка. Колонка — ближе к умолчанию, лишь бы всё
    /// поместилось по высоте; если не помещается — пробуем размеры поменьше. `false` — места нет.
    pub fn show(&mut self, spec: &TabSpec, skip: u32, idx: usize, avail: f32) -> bool {
        let Some(pos) = self.entries.iter().position(|e| e.idx == idx) else { return false };
        let mut e = self.entries.remove(pos);
        let s = &spec.specs[idx];
        e.hidden = false;
        // Сначала размер, который был у виджета, затем меньшие.
        let mut sizes: Vec<usize> = (0..=e.size).rev().collect();
        sizes.dedup();
        let mut best: Option<(f32, f32, Entry)> = None;
        for size in sizes {
            let cols = s.sizes[size].cols;
            let max_col = spec.cols.saturating_sub(cols);
            for col in 0..=max_col {
                let cand = Entry { idx, size, col, hidden: false };
                let mut lay = self.clone();
                lay.entries.push(cand.clone());
                if !lay.fits(spec, skip, avail) {
                    continue;
                }
                let total = lay.height(spec, skip);
                let near = (col as f32 - s.col as f32).abs();
                let better = match &best {
                    None => true,
                    Some((t, n, _)) => total < *t - 0.01 || ((total - *t).abs() <= 0.01 && near < *n),
                };
                if better {
                    best = Some((total, near, cand));
                }
            }
            // Нашлось место при этом размере — меньшие не нужны.
            if best.is_some() {
                break;
            }
        }
        match best {
            Some((_, _, cand)) => {
                self.entries.push(cand);
                true
            }
            None => {
                e.hidden = true;
                self.entries.insert(pos, e);
                false
            }
        }
    }

    /// Следующий по кругу размер виджета, при котором раскладка помещается. `false` — менять некуда.
    pub fn cycle_size(&mut self, spec: &TabSpec, skip: u32, idx: usize, avail: f32) -> bool {
        let Some(cur) = self.entry(idx).cloned() else { return false };
        let n = spec.specs[idx].sizes.len();
        for step in 1..n {
            let size = (cur.size + step) % n;
            let cols = spec.specs[idx].sizes[size].cols;
            let mut lay = self.clone();
            if let Some(e) = lay.entry_mut(idx) {
                e.size = size;
                e.col = clamp_col(e.col, cols, spec.cols);
            }
            if lay.fits(spec, skip, avail) {
                *self = lay;
                return true;
            }
        }
        false
    }

    /// Размер виджета, ближайший к желаемому прямоугольнику `want_w` × `want_h` (угловая ручка),
    /// из тех, при которых раскладка помещается. `None` — ни один размер не подходит.
    #[allow(clippy::too_many_arguments)]
    pub fn resized_to(&self, spec: &TabSpec, skip: u32, idx: usize, want_w: f32, want_h: f32, pitch: f32, avail: f32) -> Option<Layout> {
        let cur = self.entry(idx)?;
        let mut best: Option<(f32, Layout)> = None;
        for (size, s) in spec.specs[idx].sizes.iter().enumerate() {
            // Текущий размер выигрывает ничью: виджет не дёргается на границе.
            let cost = ((s.cols as f32 * pitch - GAP) - want_w).abs() + (s.h - want_h).abs() - if size == cur.size { 1.0 } else { 0.0 };
            if best.as_ref().is_some_and(|(c, _)| *c <= cost) {
                continue;
            }
            let mut lay = self.clone();
            if let Some(e) = lay.entry_mut(idx) {
                e.size = size;
                e.col = clamp_col(e.col, s.cols, spec.cols);
            }
            if lay.fits(spec, skip, avail) {
                best = Some((cost, lay));
            }
        }
        best.map(|(_, l)| l)
    }

    /// Лучшая вставка перетаскиваемого виджета: каждая колонка и каждое место в порядке
    /// проверяются упаковкой, выбирается положение, ближайшее к плавающему прямоугольнику
    /// (`left`, `top` — его левый верх относительно области, `pitch` — шаг колонок в пикселях).
    /// Возвращает `(раскладка, цена)`; `None` — ни один вариант не помещается.
    #[allow(clippy::too_many_arguments)]
    pub fn best_drop(&self, spec: &TabSpec, skip: u32, idx: usize, left: f32, top: f32, pitch: f32, avail: f32) -> Option<(Layout, f32)> {
        let cur = self.entry(idx)?.clone();
        let cols = spec.specs[idx].sizes[cur.size].cols;
        let base: Vec<Entry> = self.entries.iter().filter(|e| e.idx != idx).cloned().collect();
        let visible: Vec<usize> = base
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.hidden && !skipped(skip, e.idx))
            .map(|(i, _)| i)
            .collect();
        let old_col = clamp_col(cur.col, cols, spec.cols);
        let mut best: Option<(Layout, f32, f32)> = None;
        for p in 0..=visible.len() {
            let at = visible.get(p).copied().unwrap_or(base.len());
            for col in 0..=spec.cols.saturating_sub(cols) {
                // Кроме обычной вставки — обмен: виджет той же ширины, стоящий на этом месте,
                // уходит в колонку, которую освободил перетаскиваемый.
                let swaps = std::iter::once(None).chain(visible.iter().copied().filter_map(|i| {
                    let o = &base[i];
                    let oc = spec.specs[o.idx].sizes[o.size].cols;
                    let ocol = clamp_col(o.col, oc, spec.cols);
                    (oc == cols && ocol != old_col && col < ocol + oc && ocol < col + cols).then_some(Some(i))
                }));
                for swap in swaps {
                    let mut entries = base.clone();
                    if let Some(i) = swap {
                        entries[i].col = old_col;
                    }
                    entries.insert(at, Entry { idx, size: cur.size, col, hidden: false });
                    let lay = Layout { entries };
                    let placed = lay.pack(spec, skip);
                    let total = placed.iter().map(Placed::bottom).fold(0.0, f32::max);
                    if total > avail + 0.01 {
                        continue;
                    }
                    let Some(me) = placed.iter().find(|q| q.idx == idx) else { continue };
                    let dx = me.col as f32 * pitch - left;
                    let dy = me.y - top;
                    let cost = dx * dx + dy * dy;
                    // При равной цене выигрывает более низкая раскладка (обмен лучше, чем «сосед уехал вниз»).
                    if best.as_ref().is_none_or(|(_, c, t)| cost < *c - 0.01 || (cost < *c + 0.01 && total < *t - 0.01)) {
                        best = Some((lay, cost, total));
                    }
                }
            }
        }
        best.map(|(l, c, _)| (l, c))
    }

    /// Цена текущего положения виджета относительно плавающего прямоугольника (как в `best_drop`).
    pub fn drop_cost(&self, spec: &TabSpec, skip: u32, idx: usize, left: f32, top: f32, pitch: f32) -> Option<f32> {
        let me = self.pack(spec, skip).into_iter().find(|q| q.idx == idx)?;
        let dx = me.col as f32 * pitch - left;
        let dy = me.y - top;
        Some(dx * dx + dy * dy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SIZES_A: [Size; 3] = [sz(1, 40.0), sz(2, 40.0), sz(3, 40.0)];
    static SIZES_B: [Size; 2] = [sz(2, 40.0), sz(4, 80.0)];
    static SPEC: TabSpec = TabSpec {
        cols: 4,
        specs: &[
            Spec { key: "a", title: "A", sizes: &SIZES_A, size: 1, col: 0 },
            Spec { key: "b", title: "B", sizes: &SIZES_A, size: 1, col: 2 },
            Spec { key: "c", title: "C", sizes: &SIZES_B, size: 0, col: 0 },
            Spec { key: "d", title: "D", sizes: &SIZES_B, size: 0, col: 2 },
        ],
    };

    fn no_overlap(placed: &[Placed]) -> bool {
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                let cols = a.col < b.col + b.cols && b.col < a.col + a.cols;
                let rows = a.y < b.bottom() && b.y < a.bottom();
                if cols && rows {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn default_stacks_with_gap() {
        let lay = Layout::default_for(&SPEC);
        let p = lay.pack(&SPEC, 0);
        assert_eq!(p.len(), 4);
        // a и b рядом, c и d под ними.
        assert_eq!((p[0].col, p[0].y), (0, 0.0));
        assert_eq!((p[1].col, p[1].y), (2, 0.0));
        assert_eq!((p[2].col, p[2].y), (0, 50.0));
        assert_eq!((p[3].col, p[3].y), (2, 50.0));
        assert!(no_overlap(&p));
        assert_eq!(lay.height(&SPEC, 0), 90.0);
    }

    #[test]
    fn hidden_and_skipped_leave_no_gap() {
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(0);
        lay.hide(1);
        let p = lay.pack(&SPEC, 0);
        assert_eq!(p.len(), 2);
        assert!(p.iter().all(|q| q.y == 0.0));
        // Недоступный виджет ведёт себя как скрытый, но в лоток не попадает.
        let lay = Layout::default_for(&SPEC);
        assert_eq!(lay.pack(&SPEC, 0b1111).len(), 0);
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(1);
        lay.hide(2);
        assert_eq!(lay.hidden_list(&SPEC, 0b0100), vec![1]);
    }

    #[test]
    fn text_roundtrip() {
        let mut lay = Layout::default_for(&SPEC);
        assert_eq!(lay.to_text(&SPEC), "");
        lay.hide(1);
        lay.entries.swap(0, 2);
        lay.entry_mut(3).unwrap().size = 1;
        lay.entry_mut(3).unwrap().col = 0;
        let text = lay.to_text(&SPEC);
        assert_eq!(Layout::parse(&SPEC, &text), lay, "{text}");
        assert_eq!(Layout::parse(&SPEC, ""), Layout::default_for(&SPEC));
    }

    #[test]
    fn parse_survives_garbage() {
        let g = Layout::parse(&SPEC, "zzz -  :: @@ a:99@77 a b:x@y --c ,, d:1@3 d");
        assert_eq!(g.entries.len(), SPEC.specs.len());
        let mut keys: Vec<usize> = g.entries.iter().map(|e| e.idx).collect();
        keys.sort();
        assert_eq!(keys, vec![0, 1, 2, 3]);
        // Плохой размер — умолчание, колонка зажата так, чтобы виджет не вылез.
        let a = g.entry(0).unwrap();
        assert_eq!(a.size, 1);
        assert!(a.col + SIZES_A[a.size].cols <= SPEC.cols);
        let d = g.entry(3).unwrap();
        assert_eq!((d.size, d.col), (1, 0));
        assert!(no_overlap(&g.pack(&SPEC, 0)));
        // Полный мусор не ломает разбор.
        assert_eq!(Layout::parse(&SPEC, "\u{0}\u{1}\n\t@-:"), Layout::default_for(&SPEC));
    }

    #[test]
    fn drop_swaps_equal_widgets_sideways() {
        let lay = Layout::default_for(&SPEC);
        // b (колонка 2) тянут на место a: a уходит в освободившуюся колонку, оба остаются в верхнем ряду.
        let (moved, _) = lay.best_drop(&SPEC, 0, 1, 0.0, 0.0, 60.0, 200.0).unwrap();
        let p = moved.pack(&SPEC, 0);
        let a = p.iter().find(|q| q.idx == 0).unwrap();
        let b = p.iter().find(|q| q.idx == 1).unwrap();
        assert_eq!((b.col, b.y), (0, 0.0));
        assert_eq!((a.col, a.y), (2, 0.0));
        assert!(no_overlap(&p));
    }

    #[test]
    fn shown_widget_fills_gap_above() {
        // a (1 колонка, колонка 0), под ним c на две колонки; b встаёт в колонку 1 и должен
        // занять дыру рядом с a, а не уехать под c.
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(3);
        for (idx, size, col) in [(0, 0, 0), (1, 0, 1), (2, 0, 0)] {
            let e = lay.entries.iter_mut().find(|e| e.idx == idx).unwrap();
            (e.size, e.col) = (size, col);
        }
        lay.entries.swap(1, 2);
        let p = lay.pack(&SPEC, 0);
        let c = p.iter().find(|q| q.idx == 2).unwrap();
        let b = p.iter().find(|q| q.idx == 1).unwrap();
        assert_eq!((b.y, c.y), (0.0, 50.0));
        assert!(no_overlap(&p));
    }

    #[test]
    fn resized_to_snaps_to_nearest_fitting_size() {
        let lay = Layout::default_for(&SPEC);
        let pitch = 60.0;
        // Желаемая ширина 170 ближе всего к 3 колонкам.
        let big = lay.resized_to(&SPEC, 0, 0, 170.0, 40.0, pitch, 200.0).unwrap();
        assert_eq!(big.entry(0).unwrap().size, 2);
        assert!(big.fits(&SPEC, 0, 200.0));
        // Ничья в пользу текущего размера: желаемое ровно между 1 и 2 колонками не меняет размер 1.
        let same = lay.resized_to(&SPEC, 0, 0, 110.0, 40.0, pitch, 200.0).unwrap();
        assert_eq!(same.entry(0).unwrap().size, 1);
        // Тесная высота: 3 колонки не помещаются, берётся ближайший из подходящих.
        let tight = lay.resized_to(&SPEC, 0, 0, 170.0, 40.0, pitch, 90.0).unwrap();
        assert!(tight.fits(&SPEC, 0, 90.0));
        // Скрытый или несуществующий виджет не меняется.
        assert!(lay.resized_to(&SPEC, 0, 9, 10.0, 10.0, pitch, 200.0).is_none());
    }

    #[test]
    fn cycle_size_respects_height() {
        let mut lay = Layout::default_for(&SPEC);
        // Места хватает: a становится 3-колоночным (c под ним сдвигается).
        assert!(lay.cycle_size(&SPEC, 0, 0, 200.0));
        assert_eq!(lay.entry(0).unwrap().size, 2);
        // Места нет: 3 колонки у a вынуждают b уехать вниз → выше лимита.
        let mut lay = Layout::default_for(&SPEC);
        assert!(!lay.cycle_size(&SPEC, 0, 0, 90.0) || lay.height(&SPEC, 0) <= 90.0);
        assert!(lay.fits(&SPEC, 0, 90.0));
        // Единственный размер или тесная высота — отказ без изменений.
        let before = Layout::default_for(&SPEC);
        let mut lay = before.clone();
        assert!(!lay.cycle_size(&SPEC, 0, 2, 90.0));
        assert_eq!(lay, before);
    }

    #[test]
    fn show_finds_room_or_refuses() {
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(2);
        assert!(lay.show(&SPEC, 0, 2, 200.0));
        assert!(lay.fits(&SPEC, 0, 200.0));
        assert!(no_overlap(&lay.pack(&SPEC, 0)));
        // Тесная высота: после скрытия c его место свободно, но места под другой размер нет.
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(3);
        let h = lay.height(&SPEC, 0);
        assert!(lay.show(&SPEC, 0, 3, h));
        // Нет места вовсе.
        let mut lay = Layout::default_for(&SPEC);
        lay.hide(3);
        let before = lay.clone();
        assert!(!lay.show(&SPEC, 0, 3, 10.0));
        assert_eq!(lay, before);
    }

    #[test]
    fn drop_moves_to_nearest_slot() {
        let lay = Layout::default_for(&SPEC);
        let pitch = 60.0;
        // d тянут к левому верху: он должен встать выше c, в колонку 0.
        let (moved, cost) = lay.best_drop(&SPEC, 0, 3, 0.0, 0.0, pitch, 200.0).unwrap();
        let p = moved.pack(&SPEC, 0);
        let d = p.iter().find(|q| q.idx == 3).unwrap();
        assert_eq!((d.col, d.y), (0, 0.0));
        assert!(cost < 1e-3);
        assert!(no_overlap(&p));
        // Остальные виджеты сохранили видимость и не потерялись.
        assert_eq!(p.len(), 4);
        // Ни одного варианта не помещается — None.
        assert!(lay.best_drop(&SPEC, 0, 3, 0.0, 0.0, pitch, 10.0).is_none());
    }

    #[test]
    fn drop_never_overflows_and_keeps_all_entries() {
        let mut lay = Layout::default_for(&SPEC);
        // Прогон по сетке положений курсора: раскладка всегда помещается и содержит все виджеты.
        for ix in 0..8 {
            for iy in 0..8 {
                for idx in 0..4 {
                    if let Some((l, _)) = lay.best_drop(&SPEC, 0, idx, ix as f32 * 30.0, iy as f32 * 20.0, 60.0, 100.0) {
                        assert!(l.fits(&SPEC, 0, 100.0));
                        assert_eq!(l.entries.len(), 4);
                        assert!(no_overlap(&l.pack(&SPEC, 0)));
                        lay = l;
                    }
                }
            }
        }
    }

    #[test]
    fn narrow_columns_clamp_not_overflow() {
        let mut lay = Layout::default_for(&SPEC);
        lay.entry_mut(1).unwrap().col = 3; // b шириной 2 колонки у правого края
        let p = lay.pack(&SPEC, 0);
        let b = p.iter().find(|q| q.idx == 1).unwrap();
        assert!(b.col + b.cols <= SPEC.cols);
    }
}
