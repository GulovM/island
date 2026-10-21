//! Сетка виджетов вкладки: упаковка по `layout`, пружинная анимация перестановок и режим правки
//! в духе iOS. В режиме правки виджеты покачиваются, на углах появляются «−» и кнопка размера,
//! виджет можно поднять и перенести (остальные плавно перетекают на новые места), а скрытые
//! виджеты лежат в лотке.
//!
//! Вкладка отдаёт сетке функцию рисования одного виджета (`paint`). Ввод в режиме правки сетка
//! забирает себе (`Ui::mute`), поэтому сами виджеты не знают, что их правят.

use std::f32::consts::TAU;

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows_numerics::Matrix3x2;

use super::layout::{self, Layout, Placed, Side, TabSpec, Window, CELL};
use super::{id as tab_id};
use crate::anim::Spring;
use crate::i18n::tr;
use crate::ui::{self, Canvas, Rect, Ui};

/// На сколько остров выше в режиме правки: под сеткой появляется полоса с кнопками.
pub const EXTRA: f32 = 32.0;

const BAR_H: f32 = 22.0;
/// Сдвиг курсора с нажатой кнопкой, после которого нажатие становится перетаскиванием.
const MOVE_MIN: f32 = 4.0;
const NOTE_SECS: f32 = 2.4;
const BADGE_R: f32 = 8.0;
const CHIP_H: f32 = 22.0;
const TRAY_PAD: f32 = 8.0;
const TRAY_GAP: f32 = 6.0;

const BLACK: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

fn fade(c: D2D1_COLOR_F, k: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: c.a * k.clamp(0.0, 1.0), ..c }
}

/// Один виджет, который просят нарисовать: номер в `TabSpec`, индекс размера и прямоугольник
/// (он анимируется, поэтому в момент перестановки отличается от «паспортного» размера).
pub struct Item {
    pub idx: usize,
    pub size: usize,
    pub rect: Rect,
}

/// Пружины положения и размера одного виджета и его «присутствие» (скрытие и появление).
struct Slot {
    x: Spring,
    y: Spring,
    w: Spring,
    h: Spring,
    a: Spring,
    init: bool,
}

impl Slot {
    fn new() -> Self {
        let sp = || Spring::new(0.0, 460.0, 36.0);
        Self { x: sp(), y: sp(), w: sp(), h: sp(), a: Spring::unit(0.0, 420.0, 34.0), init: false }
    }

    fn rect(&self) -> Rect {
        Rect::new(self.x.value, self.y.value, self.w.value.max(0.0), self.h.value.max(0.0))
    }

    fn set(&mut self, r: Rect, snap: bool) {
        self.x.target = r.x;
        self.y.target = r.y;
        self.w.target = r.w;
        self.h.target = r.h;
        if snap {
            self.snap_rect();
        }
    }

    fn snap_rect(&mut self) {
        self.x.snap();
        self.y.snap();
        self.w.snap();
        self.h.snap();
    }

    /// Жёсткие пружины положения: поднятый виджет идёт за курсором почти без отставания.
    fn follow(&mut self, stiff: bool) {
        let (k, c) = if stiff { (900.0, 52.0) } else { (460.0, 36.0) };
        self.x.tune(k, c);
        self.y.tune(k, c);
    }

    fn settled(&self) -> bool {
        self.x.settled() && self.y.settled() && self.w.settled() && self.h.settled() && self.a.settled()
    }

    fn step(&mut self, dt: f32) {
        self.x.step(dt);
        self.y.step(dt);
        self.w.step(dt);
        self.h.step(dt);
        self.a.step(dt);
    }
}

/// Нажатие на виджете в режиме правки.
struct Drag {
    idx: usize,
    press: (f32, f32),
    /// Смещение курсора внутри виджета в момент нажатия.
    grab: (f32, f32),
    live: bool,
    /// Где сейчас «плавающий» прямоугольник виджета (левый верх).
    float: (f32, f32),
    /// Целевая ячейка сетки (тень-приёмник).
    target: Option<(u8, u8)>,
}

/// Нажатие на угловой ручке: короткое — смена размера по кругу, с движением — подбор размера.
struct Resize {
    idx: usize,
    press: (f32, f32),
    /// Размер виджета в пикселях в момент нажатия.
    from: (f32, f32),
    live: bool,
    before: Layout,
}

/// Нажатие/перетаскивание ручки растягивания окна сетки.
struct StretchDrag {
    side: Side,
    press_pos: (f32, f32),
    initial_window: Window,
    live: bool,
}

/// Кнопки полосы под сеткой.
struct Bar {
    tray: Rect,
    reset: Rect,
    done: Rect,
    note: Rect,
    show_tray: bool,
    show_reset: bool,
}

/// Лоток скрытых виджетов: карточка и чипы по одному на виджет.
struct Tray {
    card: Rect,
    chips: Vec<Rect>,
}

pub struct Grid {
    tab: usize,
    spec: &'static TabSpec,
    lay: Layout,
    /// Раскладка на начало перетаскивания: прерванный перенос возвращает её.
    before: Option<Layout>,
    slots: Vec<Slot>,
    /// Прямоугольник, куда виджет встанет, если сейчас отпустить (тень-приёмник).
    ghost: [Spring; 4],
    edit: Spring,
    lift: Spring,
    tray: Spring,
    note: Spring,
    empty: Spring,
    editing: bool,
    drag: Option<Drag>,
    resizing: Option<Resize>,
    stretching: Option<StretchDrag>,
    window: Window,
    min_cols: u8,
    overlay_rows: u8,
    last_skip: u32,
    /// Виджет, который рисуется поверх остальных, пока он «летит» на место.
    top: Option<usize>,
    tray_open: bool,
    note_text: &'static str,
    note_left: f32,
    phase: f32,
    pending: Option<String>,
    saved: Option<String>,
    resize: bool,
    snap: bool,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            tab: 0,
            spec: &layout::EMPTY,
            lay: Layout { entries: Vec::new() },
            before: None,
            slots: Vec::new(),
            ghost: std::array::from_fn(|_| Spring::new(0.0, 520.0, 38.0)),
            edit: Spring::unit(0.0, 380.0, 24.0),
            lift: Spring::unit(0.0, 420.0, 30.0),
            tray: Spring::unit(0.0, 460.0, 34.0),
            note: Spring::unit(0.0, 380.0, 32.0),
            empty: Spring::unit(0.0, 380.0, 34.0),
            editing: false,
            drag: None,
            resizing: None,
            stretching: None,
            window: Window { c0: 0, r0: 0, c1: 1, r1: 1 },
            min_cols: 1,
            overlay_rows: 0,
            last_skip: 0,
            top: None,
            tray_open: false,
            note_text: "",
            note_left: 0.0,
            phase: 0.0,
            pending: None,
            saved: None,
            resize: false,
            snap: true,
        }
    }
}

impl Grid {
    /// Привязывает сетку к вкладке и описанию её виджетов, `text` — раскладка из конфига.
    pub fn configure(&mut self, tab: usize, spec: &'static TabSpec, text: &str) {
        self.tab = tab;
        self.spec = spec;
        self.lay = Layout::default_for(spec);
        self.slots = spec.specs.iter().map(|_| Slot::new()).collect();
        self.window = layout::fit_window(&self.lay.entries, spec, 0, self.min_cols.max(spec.min_cols), self.eff_min_rows());
        self.pending = Some(text.to_string());
        self.snap = true;
    }

    /// Текущее окно сетки вкладки (в целых ячейках).
    #[inline]
    pub fn window(&self) -> Window {
        self.window
    }

    /// Число рядов окна, запрошенное открытым оверлеем (список устройств, месячный календарь).
    #[cfg(test)]
    #[inline]
    pub fn overlay_rows(&self) -> u8 {
        self.overlay_rows
    }

    /// Устанавливает динамическую потребность в рядах под активный оверлей.
    pub fn set_overlay_rows(&mut self, rows: u8) {
        if self.overlay_rows == rows {
            return;
        }
        self.overlay_rows = rows;
        if !self.editing {
            let eff = self.eff_min_rows();
            let new_win = layout::fit_window(&self.lay.entries, self.spec, self.last_skip, self.min_cols, eff);
            if new_win != self.window {
                self.window = new_win;
                self.resize = true;
            }
        }
    }

    #[inline]
    fn eff_min_rows(&self) -> u8 {
        self.spec.min_rows.max(self.overlay_rows)
    }

    #[inline]
    pub fn layout(&self) -> &Layout {
        &self.lay
    }

    /// Минимальная ширина в ячейках для согласования с рядом вкладок острова.
    pub fn set_min_cols(&mut self, min_cols: u8) {
        let eff = min_cols.max(self.spec.min_cols);
        if self.min_cols == eff {
            return;
        }
        self.min_cols = eff;
        if !self.editing {
            self.window = layout::fit_window(&self.lay.entries, self.spec, self.last_skip, self.min_cols, self.eff_min_rows());
        }
    }

    /// Раскладка из конфига изменилась снаружи (файл отредактировали). Свою же запись игнорируем.
    pub fn set_text(&mut self, text: &str) {
        if self.slots.is_empty() || text == self.lay.to_text(self.spec) {
            return;
        }
        self.pending = Some(text.to_string());
    }

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    pub fn can_edit(&self) -> bool {
        !self.spec.specs.is_empty()
    }

    /// Насколько вырастает остров, пока вкладку правят.
    pub fn extra(&self) -> f32 {
        if self.editing {
            EXTRA
        } else {
            0.0
        }
    }

    pub fn begin_edit(&mut self) {
        if self.editing || !self.can_edit() {
            return;
        }
        self.editing = true;
        self.edit.target = 1.0;
        self.tray_open = false;
        self.stretching = None;
    }

    pub fn end_edit(&mut self) {
        if !self.editing {
            return;
        }
        self.abort_drag();
        self.resizing = None;
        self.stretching = None;
        self.editing = false;
        self.edit.target = 0.0;
        self.tray_open = false;
        // По «Готово» окно сбрасывает ручные расширения и плотно обтягивает оставшиеся виджеты
        self.window = layout::fit_window(&self.lay.entries, self.spec, self.last_skip, self.min_cols, self.eff_min_rows());
        self.resize = true;
    }

    /// Вкладка стала видимой: пружины встают на конечные значения, а положения виджетов
    /// выставляются сразу, без «полёта» из нуля или со старых мест.
    pub fn show(&mut self) {
        if !self.editing {
            self.window = layout::fit_window(&self.lay.entries, self.spec, self.last_skip, self.min_cols, self.eff_min_rows());
        }
        for sp in [&mut self.edit, &mut self.lift, &mut self.tray, &mut self.note, &mut self.empty] {
            sp.snap();
        }
        for g in &mut self.ghost {
            g.snap();
        }
        self.top = None;
        self.note_left = 0.0;
        self.snap = true;
    }

    /// Вкладка скрыта или остров свернулся: правка закрывается (недоведённый перенос откатывается).
    /// Затухание рамок доигрывает пружина, пока старая вкладка видна при смене.
    pub fn hide(&mut self) {
        self.end_edit();
        self.overlay_rows = 0;
    }

    /// Раскладка для записи в `config.toml`, если она изменилась с прошлого вызова.
    pub fn take_saved(&mut self) -> Option<String> {
        self.saved.take()
    }

    /// Просьба пересчитать размер острова (кнопка «Готово»).
    pub fn take_resize(&mut self) -> bool {
        std::mem::take(&mut self.resize)
    }

    pub fn step(&mut self, dt: f32) {
        self.edit.step(dt);
        self.lift.step(dt);
        self.tray.step(dt);
        self.note.step(dt);
        self.empty.step(dt);
        for s in &mut self.slots {
            s.step(dt);
        }
        for g in &mut self.ghost {
            g.step(dt);
        }
        if self.editing || self.edit.value > 0.002 {
            self.phase = (self.phase + dt * 16.0) % TAU;
        }
        if self.note_left > 0.0 {
            self.note_left -= dt;
            if self.note_left <= 0.0 {
                self.note.target = 0.0;
            }
        }
        if self.drag.is_none() && self.lift.value < 0.01 && self.lift.target == 0.0 {
            self.top = None;
        }
    }

    pub fn animating(&self) -> bool {
        self.editing
            || self.drag.is_some()
            || self.stretching.is_some()
            || !(self.edit.settled() && self.lift.settled() && self.tray.settled() && self.note.settled() && self.empty.settled())
            || self.slots.iter().any(|s| !s.settled())
            || self.ghost.iter().any(|g| !g.settled())
    }

    // ---------- Служебное ----------

    fn wid(&self, idx: usize, k: u32) -> u32 {
        tab_id(self.tab, 0x1000 + idx as u32 * 8 + k)
    }

    fn hid(&self, side: Side) -> u32 {
        let k = match side {
            Side::Left => 0,
            Side::Top => 1,
            Side::Right => 2,
            Side::Bottom => 3,
        };
        tab_id(self.tab, 0x1400 + k)
    }

    fn bid(&self, k: u32) -> u32 {
        tab_id(self.tab, 0x1200 + k)
    }

    fn commit(&mut self) {
        self.saved = Some(self.lay.to_text(self.spec));
    }

    fn say(&mut self, text: &'static str) {
        self.note_text = text;
        self.note.target = 1.0;
        self.note_left = NOTE_SECS;
    }

    fn abort_drag(&mut self) {
        if let Some(before) = self.before.take() {
            self.lay = before;
        }
        if let Some(d) = self.drag.take() {
            self.slots[d.idx].follow(false);
        }
        self.stretching = None;
        self.lift.target = 0.0;
    }

    fn visible(&self, skip: u32) -> Vec<usize> {
        self.lay.entries.iter().filter(|e| !e.hidden && !layout::skipped(skip, e.idx)).map(|e| e.idx).collect()
    }

    // ---------- Полоса и лоток ----------

    fn tray_label(n: usize) -> String {
        format!("+ {} · {}", tr("Виджеты"), n)
    }

    unsafe fn bar(&self, cv: &Canvas, area: Rect, hidden: usize) -> Bar {
        let f = &cv.g.fonts.small_c;
        let y = area.bottom() + 8.0;
        let done_w = cv.text_w(&tr("Готово"), f) + 28.0;
        let reset_w = cv.text_w(&tr("Сбросить"), f) + 22.0;
        let done = Rect::new(area.right() - done_w, y, done_w, BAR_H);
        let show_reset = !self.lay.is_default(self.spec);
        let reset = Rect::new(done.x - 6.0 - reset_w, y, reset_w, BAR_H);
        let show_tray = hidden > 0;
        let tray_w = if show_tray { cv.text_w(&Self::tray_label(hidden), f) + 22.0 } else { 0.0 };
        let tray = Rect::new(area.x, y, tray_w, BAR_H);
        let left = if show_tray { tray.right() + 8.0 } else { area.x };
        let right = if show_reset { reset.x } else { done.x } - 8.0;
        Bar { tray, reset, done, note: Rect::new(left, y, (right - left).max(0.0), BAR_H), show_tray, show_reset }
    }

    unsafe fn tray_layout(&self, cv: &Canvas, area: Rect, bar: &Bar, hidden: &[usize]) -> Tray {
        let f = &cv.g.fonts.small_c;
        let widths: Vec<f32> =
            hidden.iter().map(|&i| (cv.text_w(&tr(self.spec.specs[i].title), f) + 22.0).max(44.0)).collect();
        let max_w = area.w.min(420.0) - 2.0 * TRAY_PAD;
        let (pos, w, h) = flow(&widths, max_w, CHIP_H, TRAY_GAP);
        let dy = (1.0 - self.tray.value) * 8.0;
        let card_h = if pos.is_empty() { 0.0 } else { h + 2.0 * TRAY_PAD };
        let card = Rect::new(area.x, bar.tray.y - 6.0 - card_h + dy, w + 2.0 * TRAY_PAD, card_h);
        let chips = pos
            .iter()
            .zip(&widths)
            .map(|(&(x, y), &cw)| Rect::new(card.x + TRAY_PAD + x, card.y + TRAY_PAD + y, cw, CHIP_H))
            .collect();
        Tray { card, chips }
    }

    // ---------- Ввод режима правки ----------

    /// Опрашивает кнопки и тела виджетов. Порядок — приоритет: что опрошено раньше, то и получит
    /// нажатие (лоток, потом значки на углах, потом сами виджеты).
    fn input(&mut self, ui: &mut Ui, area: Rect, skip: u32, hidden: &[usize], tray: &Tray) {
        let spec = self.spec;
        let ptr = ui.pointer();
        let vis = self.visible(skip);

        if vis.is_empty() && !hidden.is_empty() {
            self.tray_open = true;
        }
        if hidden.is_empty() {
            self.tray_open = false;
        }

        let mut shown: Option<usize> = None;
        if self.tray_open {
            // Чипы опрашиваются раньше карточки: кто опрошен первым, тот и берёт нажатие.
            for (k, &idx) in hidden.iter().enumerate() {
                let (_, clicked) = ui.click_area(tab_id(self.tab, 0x1300 + idx as u32), tray.chips[k]);
                if clicked {
                    shown = Some(idx);
                }
            }
            let _ = ui.click_area(self.bid(0x50), tray.card);
        }

        let mut to_hide = None;
        let mut to_cycle = None;
        for &idx in &vis {
            let r = self.slots[idx].rect();
            let (_, hide) = ui.click_area(self.wid(idx, 0), badge_hit(r));
            if hide {
                to_hide = Some(idx);
            }
            if spec.specs[idx].sizes.len() > 1 {
                let id = self.wid(idx, 1);
                let (_, grow) = ui.click_area(id, size_hit(r));
                // Отпускание после перетаскивания ручки не считается нажатием на кнопку размера.
                let dragged = self.resizing.as_ref().is_some_and(|rs| rs.idx == idx && rs.live);
                if grow && !dragged {
                    to_cycle = Some(idx);
                }
                if self.resizing.is_none() && self.drag.is_none() && ui.is_dragging(id) {
                    let (ptr_x, ptr_y) = ptr;
                    self.resizing = Some(Resize { idx, press: (ptr_x, ptr_y), from: (r.w, r.h), live: false, before: self.lay.clone() });
                }
            }
        }
        for &idx in &vis {
            let id = self.wid(idx, 2);
            let r = self.slots[idx].rect();
            let _ = ui.click_area(id, r);
            if self.drag.is_none() && self.resizing.is_none() && ui.is_dragging(id) {
                let target = self.lay.entry(idx).map(|e| (e.col, e.row));
                self.drag = Some(Drag { idx, press: ptr, grab: (ptr.0 - r.x, ptr.1 - r.y), live: false, float: (r.x, r.y), target });
            }
        }

        // Ручки растягивания окна сетки
        const SIDES: [Side; 4] = [Side::Left, Side::Top, Side::Right, Side::Bottom];
        for &side in &SIDES {
            if !layout::can_expand_side(self.window, spec, side) {
                continue;
            }
            let id = self.hid(side);
            let (_bar, hit) = Self::handle_geom(side, area);
            let (_, clicked) = ui.click_area(id, hit);
            let dragged = self.stretching.as_ref().is_some_and(|st| st.side == side && st.live);
            if clicked && !dragged {
                // Клик без движения: +4 ячейки (но не больше максимума)
                self.window = layout::expand_side(self.window, spec, side, 4);
                self.resize = true;
            }
            if self.stretching.is_none() && self.drag.is_none() && self.resizing.is_none() && ui.is_dragging(id) {
                self.stretching = Some(StretchDrag {
                    side,
                    press_pos: ptr,
                    initial_window: self.window,
                    live: false,
                });
            }
        }

        if let Some(idx) = shown {
            if self.lay.place_new_in_window(self.window, spec, skip, idx) {
                self.commit();
                if self.lay.hidden_list(spec, skip).is_empty() {
                    self.tray_open = false;
                }
            } else {
                self.say("Не хватает места");
            }
        }
        if let Some(idx) = to_hide {
            self.lay.hide(idx);
            self.commit();
            if self.lay.visible_count(skip) == 0 {
                self.tray_open = true;
            }
        }
        if let Some(idx) = to_cycle {
            if self.lay.cycle_size_in_window(self.window, spec, skip, idx) {
                self.commit();
            } else {
                self.say("Не хватает места");
            }
        }

        self.stretch_step(ui, spec, area);
        self.resize_step(ui, area, skip);
        self.drag_step(ui, area, skip);

        // Нажатие мимо лотка закрывает его (кроме случая, когда на вкладке ничего не осталось).
        if self.tray_open && ui.pressed && self.lay.visible_count(skip) > 0 {
            let on_toggle = ptr.1 >= area.bottom() && ptr.1 < area.bottom() + 40.0 && ptr.0 < area.x + 160.0;
            if !tray.card.contains(ptr) && !on_toggle {
                self.tray_open = false;
            }
        }
    }

    /// Геометрия ручки растягивания окна: (полоса для рисования, область нажатия).
    pub fn handle_geom(side: Side, area: Rect) -> (Rect, Rect) {
        let span = 0.5;
        let thick = 3.0;
        match side {
            Side::Left => {
                let len = area.h * span;
                let cy = area.cy();
                let x = area.x - 4.5;
                let bar = Rect::new(x - thick / 2.0, cy - len / 2.0, thick, len);
                let hit = Rect::new(0.0, cy - len / 2.0, area.x + 2.0, len);
                (bar, hit)
            }
            Side::Right => {
                let len = area.h * span;
                let cy = area.cy();
                let x = area.right() + 4.5;
                let bar = Rect::new(x - thick / 2.0, cy - len / 2.0, thick, len);
                let hit = Rect::new(area.right() - 2.0, cy - len / 2.0, super::PAD + 8.0, len);
                (bar, hit)
            }
            Side::Top => {
                let len = area.w * span;
                let cx = area.cx();
                let y = area.y - 4.5;
                let bar = Rect::new(cx - len / 2.0, y - thick / 2.0, len, thick);
                let hit = Rect::new(cx - len / 2.0, area.y - 12.0, len, 14.0);
                (bar, hit)
            }
            Side::Bottom => {
                let len = area.w * span;
                let cx = area.cx();
                let y = area.bottom() + 4.0;
                let bar = Rect::new(cx - len / 2.0, y - thick / 2.0, len, thick);
                let hit = Rect::new(cx - len / 2.0, area.bottom() - 2.0, len, 9.0);
                (bar, hit)
            }
        }
    }

    /// Растягивание окна сетки перетаскиванием ручки.
    fn stretch_step(&mut self, ui: &mut Ui, spec: &TabSpec, area: Rect) {
        let Some(mut st) = self.stretching.take() else { return };
        if !ui.is_dragging(self.hid(st.side)) {
            return;
        }
        let (px, py) = ui.pointer();
        if !st.live && (px - st.press_pos.0).hypot(py - st.press_pos.1) > MOVE_MIN {
            st.live = true;
            self.tray_open = false;
        }
        if st.live {
            let layer_w = area.w + 2.0 * super::PAD;
            let new_win = layout::stretch_window(st.side, (px, py), st.press_pos, st.initial_window, spec, layer_w);
            if new_win != self.window {
                self.window = new_win;
                self.resize = true;
            }
        }
        self.stretching = Some(st);
    }

    /// Рисует подсвеченные ручки растягивания окна с пульсирующей альфой и шевронами наружу.
    unsafe fn draw_handles(&self, ui: &mut Ui, cv: &Canvas, area: Rect) {
        let e = self.edit.value.clamp(0.0, 1.0);
        if e < 0.002 {
            return;
        }
        let pulse = (self.phase * 0.25).sin() * 0.5 + 0.5;
        let base_a = 0.40 + 0.25 * pulse;

        const SIDES: [Side; 4] = [Side::Left, Side::Top, Side::Right, Side::Bottom];
        for &side in &SIDES {
            if !layout::can_expand_side(self.window, self.spec, side) {
                continue;
            }
            let id = self.hid(side);
            let hv = ui.hover_of(id);
            let a = ((base_a + 0.35 * hv) * e).clamp(0.0, 1.0);
            let col = fade(cv.accent, a);
            let (bar, _) = Self::handle_geom(side, area);

            let rad = (bar.w.min(bar.h) / 2.0).min(1.5);
            cv.rr(bar, rad, col);

            let stroke_col = fade(cv.accent, (a + 0.2 * e).clamp(0.0, 1.0));
            let t = 1.3;
            match side {
                Side::Left => {
                    let cx = bar.cx() - 1.0;
                    let cy = bar.cy();
                    cv.stroke_line((cx + 2.5, cy - 3.5), (cx - 1.5, cy), t, stroke_col);
                    cv.stroke_line((cx - 1.5, cy), (cx + 2.5, cy + 3.5), t, stroke_col);
                }
                Side::Right => {
                    let cx = bar.cx() + 1.0;
                    let cy = bar.cy();
                    cv.stroke_line((cx - 2.5, cy - 3.5), (cx + 1.5, cy), t, stroke_col);
                    cv.stroke_line((cx + 1.5, cy), (cx - 2.5, cy + 3.5), t, stroke_col);
                }
                Side::Top => {
                    let cx = bar.cx();
                    let cy = bar.cy() - 1.0;
                    cv.stroke_line((cx - 3.5, cy + 2.5), (cx, cy - 1.5), t, stroke_col);
                    cv.stroke_line((cx, cy - 1.5), (cx + 3.5, cy + 2.5), t, stroke_col);
                }
                Side::Bottom => {
                    let cx = bar.cx();
                    let cy = bar.cy() + 1.0;
                    cv.stroke_line((cx - 3.5, cy - 2.5), (cx, cy + 1.5), t, stroke_col);
                    cv.stroke_line((cx, cy + 1.5), (cx + 3.5, cy - 2.5), t, stroke_col);
                }
            }
        }
    }

    /// Угловая ручка: пока кнопка зажата и курсор двигается, виджет выбирает ближайший
    /// допустимый размер из `Spec::sizes`.
    fn resize_step(&mut self, ui: &mut Ui, area: Rect, skip: u32) {
        let spec = self.spec;
        let Some(mut rs) = self.resizing.take() else { return };
        if !ui.is_dragging(self.wid(rs.idx, 1)) {
            if rs.live && rs.before != self.lay {
                self.commit();
            }
            return;
        }
        let (px, py) = ui.pointer();
        if !rs.live && (px - rs.press.0).hypot(py - rs.press.1) > MOVE_MIN {
            rs.live = true;
            self.tray_open = false;
            if let Some(e) = self.lay.entry(rs.idx) {
                let sz = spec.specs[rs.idx].sizes[e.size];
                let gx = area.x + (e.col - self.window.c0) as f32 * CELL;
                let gy = area.y + (e.row - self.window.r0) as f32 * CELL;
                let gw = sz.pixel_w();
                let gh = sz.pixel_h();
                for (g, v) in self.ghost.iter_mut().zip([gx, gy, gw, gh]) {
                    g.value = v;
                    g.target = v;
                    g.snap();
                }
            }
        }
        if rs.live {
            let (w, h) = (rs.from.0 + px - rs.press.0, rs.from.1 + py - rs.press.1);
            if let Some(lay) = self.lay.resized_to_in_window(self.window, spec, skip, rs.idx, w, h) {
                self.lay = lay;
            }
            if let Some(e) = self.lay.entry(rs.idx) {
                let sz = spec.specs[rs.idx].sizes[e.size];
                let gx = area.x + (e.col - self.window.c0) as f32 * CELL;
                let gy = area.y + (e.row - self.window.r0) as f32 * CELL;
                let gw = sz.pixel_w();
                let gh = sz.pixel_h();
                for (g, v) in self.ghost.iter_mut().zip([gx, gy, gw, gh]) {
                    g.target = v;
                }
            }
        }
        self.resizing = Some(rs);
    }

    fn drag_step(&mut self, ui: &mut Ui, area: Rect, skip: u32) {
        let spec = self.spec;
        let Some(mut d) = self.drag.take() else { return };
        if !ui.is_dragging(self.wid(d.idx, 2)) {
            // Кнопку отпустили: виджет встаёт на место цели тени-приёмника.
            self.slots[d.idx].follow(false);
            self.lift.target = 0.0;
            if d.live {
                if let Some((col, row)) = d.target {
                    if let Some(e) = self.lay.entry_mut(d.idx) {
                        e.col = col;
                        e.row = row;
                    }
                    if let Some(before) = self.before.take() {
                        if before != self.lay {
                            self.commit();
                        }
                    }
                } else {
                    self.abort_drag();
                    self.say("Не хватает места");
                }
            }
            return;
        }
        let (px, py) = ui.pointer();
        if !d.live && (px - d.press.0).hypot(py - d.press.1) > MOVE_MIN {
            d.live = true;
            self.before = Some(self.lay.clone());
            self.top = Some(d.idx);
            self.lift.target = 1.0;
            self.tray_open = false;
            let s = &mut self.slots[d.idx];
            s.follow(true);
            let (x, y, w, h) = (s.x.value, s.y.value, s.w.value, s.h.value);
            for (g, v) in self.ghost.iter_mut().zip([x, y, w, h]) {
                g.value = v;
                g.target = v;
                g.snap();
            }
        }
        if d.live {
            let (w, h) = (self.slots[d.idx].w.target, self.slots[d.idx].h.target);
            let fx = (px - d.grab.0).clamp(area.x - 8.0, (area.right() - w + 8.0).max(area.x - 8.0));
            let fy = (py - d.grab.1).clamp(area.y - 8.0, (area.bottom() - h + 8.0).max(area.y - 8.0));
            d.float = (fx, fy);
            let (want_col, want_row) = self.window.pixel_to_cell(fx - area.x + CELL / 2.0, fy - area.y + CELL / 2.0);
            let cur_size = self.lay.entry(d.idx).map_or(0, |e| e.size);
            let target = if layout::can_place_in_window(self.window, spec, skip, &self.lay.entries, d.idx, cur_size, want_col, want_row) {
                Some((want_col, want_row))
            } else {
                None
            };
            d.target = target;
            let sz = spec.specs[d.idx].sizes[cur_size];
            let gw = sz.pixel_w();
            let gh = sz.pixel_h();
            if let Some((tc, tr)) = target {
                let gx = area.x + (tc - self.window.c0) as f32 * CELL;
                let gy = area.y + (tr - self.window.r0) as f32 * CELL;
                for (g, v) in self.ghost.iter_mut().zip([gx, gy, gw, gh]) {
                    g.target = v;
                }
            } else {
                let max_c = self.window.c1.saturating_sub(sz.w);
                let max_r = self.window.r1.saturating_sub(sz.h);
                let clamp_c = want_col.clamp(self.window.c0, max_c.max(self.window.c0));
                let clamp_r = want_row.clamp(self.window.r0, max_r.max(self.window.r0));
                let gx = area.x + (clamp_c - self.window.c0) as f32 * CELL;
                let gy = area.y + (clamp_r - self.window.r0) as f32 * CELL;
                for (g, v) in self.ghost.iter_mut().zip([gx, gy, gw, gh]) {
                    g.target = v;
                }
            }
        }
        self.drag = Some(d);
    }

    // ---------- Рисование ----------

    /// Рисует сетку. `area` — область под виджеты (логические пиксели слоя), `skip` — маска
    /// недоступных виджетов. `paint` рисует один виджет в переданном прямоугольнике.
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, area: Rect, skip: u32, mut paint: impl FnMut(&mut Ui, &Canvas, &Item)) {
        let spec = self.spec;
        if spec.specs.is_empty() || self.slots.len() != spec.specs.len() {
            return;
        }
        self.last_skip = skip;
        if let Some(text) = self.pending.take() {
            let lay = Layout::parse(spec, &text);
            self.lay = if lay.fits(spec, skip, area.h) { lay } else { Layout::default_for(spec) };
            if !self.editing {
                self.window = layout::fit_window(&self.lay.entries, spec, skip, self.min_cols, self.eff_min_rows());
            }
        }
        if self.editing && ui.focus.is_some() {
            ui.blur();
        }

        if self.editing {
            self.tray.target = if self.tray_open { 1.0 } else { 0.0 };
            let hidden = self.lay.hidden_list(spec, skip);
            let bar = self.bar(cv, area, hidden.len());
            let tray = self.tray_layout(cv, area, &bar, &hidden);
            self.input(ui, area, skip, &hidden, &tray);
        }
        self.tray.target = if self.editing && self.tray_open { 1.0 } else { 0.0 };

        // Раскладка → цели пружин с учётом смещения текущего окна.
        let mut placed = self.lay.pack_window(spec, skip, self.window);
        if !self.editing && self.drag.is_none() && !self.lay.is_default(spec) && !self.lay.fits(spec, skip, area.h) {
            self.lay = Layout::default_for(spec);
            self.window = layout::fit_window(&self.lay.entries, spec, skip, self.min_cols, self.eff_min_rows());
            placed = self.lay.pack_window(spec, skip, self.window);
        }
        self.retarget(area, &placed);
        let live = self.drag.as_ref().filter(|d| d.live).map(|d| d.idx);
        self.empty.target = if placed.is_empty() { 1.0 } else { 0.0 };

        // Вершины сетки и подсветка узлов в режиме правки (рисуются под виджетами).
        let focus = if let Some(d) = self.drag.as_ref().filter(|d| d.live) {
            if let Some((tc, tr)) = d.target {
                let cur_size = self.lay.entry(d.idx).map_or(0, |e| e.size);
                let sz = spec.specs[d.idx].sizes[cur_size];
                Some((tc, tr, sz.w, sz.h))
            } else {
                None
            }
        } else if let Some(rs) = self.resizing.as_ref().filter(|rs| rs.live) {
            if let Some(e) = self.lay.entry(rs.idx) {
                let sz = spec.specs[rs.idx].sizes[e.size];
                Some((e.col + sz.w, e.row + sz.h, 0, 0))
            } else {
                None
            }
        } else {
            None
        };
        self.draw_grid_vertices(cv, area, focus);

        // Тень-приёмник под поднятым виджетом или тень будущего размера при ресайзе.
        if let (Some(_idx), true) = (live, self.lift.value > 0.01) {
            let r = Rect::new(self.ghost[0].value, self.ghost[1].value, self.ghost[2].value.max(0.0), self.ghost[3].value.max(0.0));
            let k = self.lift.value.clamp(0.0, 1.0);
            let valid = self.drag.as_ref().is_some_and(|d| d.target.is_some());
            let ghost_color = if valid { cv.accent } else { ui::status_red() };
            cv.rr(r, 11.0, fade(ghost_color, 0.12 * k));
            cv.rr_line(r, 11.0, fade(ghost_color, 0.55 * k), 1.2);
        } else if self.resizing.as_ref().is_some_and(|rs| rs.live) {
            let r = Rect::new(self.ghost[0].value, self.ghost[1].value, self.ghost[2].value.max(0.0), self.ghost[3].value.max(0.0));
            cv.rr(r, 11.0, fade(cv.accent, 0.12));
            cv.rr_line(r, 11.0, fade(cv.accent, 0.55), 1.2);
        }

        // Порядок рисования: как в раскладке, а поднятый (и летящий на место) виджет — последним.
        let mut order: Vec<usize> = self.lay.entries.iter().map(|e| e.idx).filter(|&i| Some(i) != self.top).collect();
        order.extend(self.top);
        let mut m = Matrix3x2::default();
        cv.g.dc.GetTransform(&mut m);
        for idx in order {
            self.draw_one(ui, cv, &m, idx, &mut paint);
        }
        cv.g.dc.SetTransform(&m);

        self.draw_empty(ui, cv, area, skip);
        if self.edit.value > 0.002 {
            self.draw_handles(ui, cv, area);
            self.draw_chrome(ui, cv, area, skip);
        }
    }

    /// Выставляет цели пружин по упакованной раскладке.
    fn retarget(&mut self, area: Rect, placed: &[Placed]) {
        let snap = std::mem::take(&mut self.snap);
        let float = self.drag.as_ref().filter(|d| d.live).map(|d| (d.idx, d.float));
        let mut present = vec![false; self.slots.len()];
        for p in placed {
            present[p.idx] = true;
            let r = Rect::new(area.x + p.x, area.y + p.y, p.w_px, p.h_px);
            let s = &mut self.slots[p.idx];
            // Виджет, который только что вернули, появляется на новом месте, а не прилетает со старого.
            let fresh = !s.init || (s.a.target < 0.5 && s.a.value < 0.02);
            s.init = true;
            s.set(r, snap || fresh);
            s.a.target = 1.0;
            if snap {
                s.a.snap();
            }
            if let Some((fi, (fx, fy))) = float {
                if fi == p.idx {
                    s.x.target = fx;
                    s.y.target = fy;
                }
            }
        }
        for (i, s) in self.slots.iter_mut().enumerate() {
            if !present[i] {
                s.a.target = 0.0;
                if !s.init || snap {
                    s.init = true;
                    s.a.snap();
                }
            }
        }
    }

    unsafe fn draw_one(&self, ui: &mut Ui, cv: &Canvas, base: &Matrix3x2, idx: usize, paint: &mut impl FnMut(&mut Ui, &Canvas, &Item)) {
        let s = &self.slots[idx];
        let a = s.a.value.clamp(0.0, 1.0);
        if a < 0.004 {
            return;
        }
        let r = s.rect();
        let e = self.edit.value;
        let lift = if Some(idx) == self.top { self.lift.value } else { 0.0 };
        let scale = (0.88 + 0.12 * a) * (1.0 + 0.045 * lift);
        let deg = if e > 0.001 { (self.phase + idx as f32 * 1.9).sin() * 0.5 * e * (1.0 - lift.clamp(0.0, 1.0)) } else { 0.0 };
        if (scale - 1.0).abs() > 1e-4 || deg.abs() > 1e-3 {
            cv.g.dc.SetTransform(&ui::compose(&ui::matrix_about(scale, deg, r.cx(), r.cy()), base));
        } else {
            cv.g.dc.SetTransform(base);
        }
        if lift > 0.01 {
            shadow(cv, r, lift);
        }

        let size = self.lay.entry(idx).map_or(self.spec.specs[idx].size, |e| e.size);
        let sub = Canvas { g: cv.g, alpha: cv.alpha * a, accent: cv.accent };
        let mute = self.editing || e > 0.002 || s.a.target < 0.5;
        let prev = if mute { Some(ui.mute()) } else { None };
        paint(ui, &sub, &Item { idx, size, rect: r });
        if let Some(p) = prev {
            ui.unmute(p);
        }

        if e > 0.002 && a > 0.3 {
            let k = (e * a * a).clamp(0.0, 1.0);
            let hv = ui.hover_of(self.wid(idx, 2));
            let dcv = Canvas { g: cv.g, alpha: cv.alpha * k, accent: cv.accent };
            dcv.rr_line(r, 11.0, fade(cv.accent, 0.30 + 0.35 * hv.clamp(0.0, 1.0) + 0.2 * lift), 1.0);
            if lift < 0.5 {
                let kb = (1.0 - lift * 2.0).clamp(0.0, 1.0);
                let bcv = Canvas { g: cv.g, alpha: dcv.alpha * kb, accent: cv.accent };
                let bh = ui.hover_of(self.wid(idx, 0));
                let (bx, by) = (r.x + 2.0, r.y + 2.0);
                let br = BADGE_R * e.max(0.0) * (1.0 + 0.1 * bh);
                bcv.circle(bx, by, br, ui::status_red());
                let half = 3.2 * e.max(0.0);
                bcv.stroke_line((bx - half, by), (bx + half, by), 1.7, ui::WHITE);
                if self.spec.specs[idx].sizes.len() > 1 {
                    let held = self.resizing.as_ref().is_some_and(|rs| rs.idx == idx && rs.live);
                    let sh = ui.hover_of(self.wid(idx, 1)).max(if held { 2.0 } else { 0.0 });
                    let (sx, sy) = (r.right() - 2.0, r.bottom() - 2.0);
                    let sr = BADGE_R * e.max(0.0) * (1.0 + 0.1 * sh);
                    bcv.circle(sx, sy, sr, cv.accent);
                    let q = 3.0 * e.max(0.0);
                    let t = ui::ink();
                    bcv.stroke_line((sx - q, sy + q), (sx + q, sy - q), 1.5, t);
                    bcv.stroke_line((sx + q, sy - q), (sx + q, sy - q + 2.8), 1.5, t);
                    bcv.stroke_line((sx + q, sy - q), (sx + q - 2.8, sy - q), 1.5, t);
                    bcv.stroke_line((sx - q, sy + q), (sx - q, sy + q - 2.8), 1.5, t);
                    bcv.stroke_line((sx - q, sy + q), (sx - q + 2.8, sy + q), 1.5, t);
                }
            }
        }
    }

    /// Заглушка пустой вкладки: все виджеты скрыты.
    unsafe fn draw_empty(&mut self, ui: &mut Ui, cv: &Canvas, area: Rect, skip: u32) {
        let k = self.empty.value.clamp(0.0, 1.0);
        if k < 0.01 {
            return;
        }
        let ecv = Canvas { g: cv.g, alpha: cv.alpha * k, accent: cv.accent };
        let f = &cv.g.fonts;
        let cy = area.y + area.h * 0.42;
        ecv.text(&tr("Нет виджетов"), Rect::new(area.x, cy - 16.0, area.w, 16.0), &f.body_c, ui::txt());
        let hidden = self.lay.hidden_list(self.spec, skip);
        if hidden.is_empty() {
            return;
        }
        if self.editing {
            ecv.text(&tr("Верните их из списка внизу"), Rect::new(area.x, cy + 4.0, area.w, 14.0), &f.small_c, ui::bare());
        } else if ui.solid_button(&ecv, self.bid(0x60), Rect::new(area.cx() - 46.0, cy + 6.0, 92.0, 22.0), &tr("Настроить"), ui::chip_bg(), ui::txt(), 11.0) {
            self.begin_edit();
        }
    }

    /// Полоса с кнопками и лоток скрытых виджетов (поверх всего остального).
    unsafe fn draw_chrome(&mut self, ui: &mut Ui, cv: &Canvas, area: Rect, skip: u32) {
        let spec = self.spec;
        let e = self.edit.value.clamp(0.0, 1.0);
        let hidden = self.lay.hidden_list(spec, skip);
        let bar = self.bar(cv, area, hidden.len());
        let f = &cv.g.fonts;

        // Кнопки опрашиваются только пока идёт правка; при затухании полосы они уже неактивны.
        let prev = if self.editing { None } else { Some(ui.mute()) };
        let bcv = Canvas { g: cv.g, alpha: cv.alpha * e, accent: cv.accent };
        if bar.show_tray && ui.chip(&bcv, self.bid(1), bar.tray, &Self::tray_label(hidden.len()), self.tray_open) {
            self.tray_open = !self.tray_open;
        }
        if bar.show_reset && ui.chip(&bcv, self.bid(2), bar.reset, &tr("Сбросить"), false) {
            self.lay = Layout::default_for(spec);
            self.commit();
            self.tray_open = false;
            let def_win = layout::fit_window(&self.lay.entries, spec, skip, self.min_cols, self.eff_min_rows());
            self.window.c0 = self.window.c0.min(def_win.c0);
            self.window.r0 = self.window.r0.min(def_win.r0);
            self.window.c1 = self.window.c1.max(def_win.c1);
            self.window.r1 = self.window.r1.max(def_win.r1);
            self.resize = true;
        }
        if ui.solid_button(&bcv, self.bid(3), bar.done, &tr("Готово"), cv.accent, ui::ink(), BAR_H / 2.0) {
            self.end_edit();
            self.resize = true;
        }
        if let Some(p) = prev {
            ui.unmute(p);
        }

        let nk = self.note.value.clamp(0.0, 1.0);
        if nk > 0.01 && !self.note_text.is_empty() {
            let ncv = Canvas { g: cv.g, alpha: bcv.alpha * nk, accent: cv.accent };
            ncv.text(&tr(self.note_text), bar.note, &f.small_c, ui::status_orange());
        }

        // Лоток: карточка с чипами, по одному на скрытый виджет.
        let tk = self.tray.value.clamp(0.0, 1.0);
        if tk > 0.01 && !hidden.is_empty() {
            let tray = self.tray_layout(cv, area, &bar, &hidden);
            let tcv = Canvas { g: cv.g, alpha: cv.alpha * tk, accent: cv.accent };
            let bg = D2D1_COLOR_F { a: 1.0, ..ui::mix(crate::theme::get().bg, ui::fg(), 0.10) };
            shadow(&tcv, tray.card, 0.7);
            tcv.rr(tray.card, 11.0, bg);
            tcv.rr_line(tray.card, 11.0, ui::track(), 1.0);
            for (k, &idx) in hidden.iter().enumerate() {
                let r = tray.chips[k];
                let hv = ui.hover_of(tab_id(self.tab, 0x1300 + idx as u32)).clamp(0.0, 1.0);
                tcv.rr(r, r.h / 2.0, ui::mix(ui::chip_bg(), ui::fg(), 0.08 * hv));
                tcv.text(&tr(spec.specs[idx].title), r, &f.small_c, ui::mix(ui::dim(), ui::txt(), hv));
            }
        }
    }

    /// Рисует вершины сетки точками и «+» на углах под виджетами в режиме правки внутри текущего окна.
    unsafe fn draw_grid_vertices(&self, cv: &Canvas, area: Rect, focus: Option<(u8, u8, u8, u8)>) {
        let e = self.edit.value.clamp(0.0, 1.0);
        if e < 0.002 {
            return;
        }
        let fg = ui::fg();
        let base_inner = fade(fg, 0.22 * e);
        let base_edge = fade(fg, 0.38 * e);
        let base_corner = fade(fg, 0.45 * e);
        let max_dist = 2.5;

        let win = self.window;
        let cols = win.cols();
        let rows = win.rows();

        for r_rel in 0..=rows {
            for c_rel in 0..=cols {
                let (nx, ny) = node_pos(area, c_rel, r_rel);
                let kind = classify_node(c_rel, r_rel, cols, rows);
                let abs_c = win.c0 + c_rel;
                let abs_r = win.r0 + r_rel;
                let att = highlight_factor(abs_c, abs_r, focus, max_dist);

                match kind {
                    NodeKind::Corner => {
                        let col = if att > 0.0 {
                            ui::mix(base_corner, fade(cv.accent, 0.85 * e), att)
                        } else {
                            base_corner
                        };
                        let arm = 3.2;
                        cv.stroke_line((nx - arm, ny), (nx + arm, ny), 1.2, col);
                        cv.stroke_line((nx, ny - arm), (nx, ny + arm), 1.2, col);
                    }
                    NodeKind::Perimeter => {
                        let col = if att > 0.0 {
                            ui::mix(base_edge, fade(cv.accent, 0.85 * e), att)
                        } else {
                            base_edge
                        };
                        let rad = 1.1 + 0.5 * att;
                        cv.circle(nx, ny, rad, col);
                    }
                    NodeKind::Interior => {
                        let col = if att > 0.0 {
                            ui::mix(base_inner, fade(cv.accent, 0.85 * e), att)
                        } else {
                            base_inner
                        };
                        let rad = 1.0 + 0.6 * att;
                        cv.circle(nx, ny, rad, col);
                    }
                }
            }
        }
    }
}

/// Область нажатия на значок «−» в левом верхнем углу виджета.
fn badge_hit(r: Rect) -> Rect {
    Rect::new(r.x + 2.0 - 10.0, r.y + 2.0 - 10.0, 20.0, 20.0)
}

/// Область нажатия на кнопку размера в правом нижнем углу виджета.
fn size_hit(r: Rect) -> Rect {
    Rect::new(r.right() - 2.0 - 10.0, r.bottom() - 2.0 - 10.0, 20.0, 20.0)
}

/// Мягкая тень из нескольких полупрозрачных слоёв.
unsafe fn shadow(cv: &Canvas, r: Rect, k: f32) {
    for i in 1..=4 {
        let g = 3.0 * i as f32;
        let rr = Rect::new(r.x - g * 0.6, r.y - g * 0.6 + 5.0, r.w + g * 1.2, r.h + g * 1.2);
        cv.rr(rr, 11.0 + g * 0.6, fade(BLACK, 0.07 * k));
    }
}

/// Раскладывает чипы шириной `widths` в строки не шире `max_w`. Возвращает левые верхи чипов
/// относительно начала лотка, ширину и высоту занятого места.
fn flow(widths: &[f32], max_w: f32, row_h: f32, gap: f32) -> (Vec<(f32, f32)>, f32, f32) {
    let mut pos = Vec::with_capacity(widths.len());
    let (mut x, mut y, mut total_w) = (0.0f32, 0.0f32, 0.0f32);
    for &w in widths {
        if x > 0.0 && x + w > max_w {
            x = 0.0;
            y += row_h + gap;
        }
        pos.push((x, y));
        total_w = total_w.max(x + w);
        x += w + gap;
    }
    let h = if widths.is_empty() { 0.0 } else { y + row_h };
    (pos, total_w, h)
}

/// Классификация узла сетки по его положению на поле.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    /// Один из четырёх внешних углов области сетки (рисуется как «+»).
    Corner,
    /// Внешний периметр сетки (рисуется чуть заметнее внутренних узлов).
    Perimeter,
    /// Внутренний узел между ячейками.
    Interior,
}

/// Определяет категорию узла сетки `(c, r)` для поля размером `cols × rows`.
#[inline]
pub fn classify_node(c: u8, r: u8, cols: u8, rows: u8) -> NodeKind {
    let on_col = c == 0 || c == cols;
    let on_row = r == 0 || r == rows;
    if on_col && on_row {
        NodeKind::Corner
    } else if on_col || on_row {
        NodeKind::Perimeter
    } else {
        NodeKind::Interior
    }
}

/// Позиция узла сетки `(c, r)` в пиксельных координатах слоя.
/// Узел находится в центре зазора между ячейками: `c * CELL - GAP / 2`.
#[inline]
pub fn node_pos(area: Rect, c: u8, r: u8) -> (f32, f32) {
    (
        area.x + c as f32 * CELL - layout::GAP / 2.0,
        area.y + r as f32 * CELL - layout::GAP / 2.0,
    )
}

/// Евклидово расстояние (в ячейках) от узла `(c, r)` до прямоугольника ячеек
/// `[box_c .. box_c + box_w, box_r .. box_r + box_h]`.
/// Для отдельной точки (например, угла виджета при ресайзе) передаётся `box_w = 0, box_h = 0`.
pub fn dist_to_box(c: u8, r: u8, box_c: u8, box_r: u8, box_w: u8, box_h: u8) -> f32 {
    let dc = if c < box_c {
        (box_c - c) as f32
    } else if c > box_c + box_w {
        (c - (box_c + box_w)) as f32
    } else {
        0.0
    };
    let dr = if r < box_r {
        (box_r - r) as f32
    } else if r > box_r + box_h {
        (r - (box_r + box_h)) as f32
    } else {
        0.0
    };
    dc.hypot(dr)
}

/// Мягкое затухание подсветки по расстоянию `dist` в ячейках (гладкий переход smoothstep).
/// При `dist == 0` возвращает 1.0, при `dist >= max_dist` — 0.0.
pub fn node_attenuation(dist: f32, max_dist: f32) -> f32 {
    if dist >= max_dist || max_dist <= 0.0 {
        0.0
    } else {
        let t = 1.0 - dist / max_dist;
        t * t * (3.0 - 2.0 * t)
    }
}

/// Коэффициент подсветки узла `(c, r)` относительно прямоугольника фокуса сетки.
pub fn highlight_factor(
    c: u8,
    r: u8,
    focus: Option<(u8, u8, u8, u8)>,
    max_dist: f32,
) -> f32 {
    let Some((bc, br, bw, bh)) = focus else { return 0.0 };
    let dist = dist_to_box(c, r, bc, br, bw, bh);
    node_attenuation(dist, max_dist)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_position_and_gap_centering() {
        let area = Rect::new(14.0, 38.0, 376.0, 248.0);
        let p00 = node_pos(area, 0, 0);
        assert_eq!(p00, (14.0 - 4.0, 38.0 - 4.0));

        let p10 = node_pos(area, 1, 0);
        assert_eq!(p10, (14.0 + 16.0 - 4.0, 38.0 - 4.0));
        assert_eq!(p10.0 - p00.0, CELL);

        let p01 = node_pos(area, 0, 1);
        assert_eq!(p01.1 - p00.1, CELL);

        // Узел между виджетом w=8 (x: 14..134) и виджетом w=8 (x: 142..262):
        // Зазор между 134 и 142 px, середина зазора — 138 px.
        let p80 = node_pos(area, 8, 0);
        assert_eq!(p80.0, 138.0);
    }

    #[test]
    fn classify_nodes_perimeter_and_corners() {
        let (cols, rows) = (24, 16);
        assert_eq!(classify_node(0, 0, cols, rows), NodeKind::Corner);
        assert_eq!(classify_node(cols, 0, cols, rows), NodeKind::Corner);
        assert_eq!(classify_node(0, rows, cols, rows), NodeKind::Corner);
        assert_eq!(classify_node(cols, rows, cols, rows), NodeKind::Corner);

        assert_eq!(classify_node(5, 0, cols, rows), NodeKind::Perimeter);
        assert_eq!(classify_node(0, 5, cols, rows), NodeKind::Perimeter);
        assert_eq!(classify_node(cols, 5, cols, rows), NodeKind::Perimeter);
        assert_eq!(classify_node(5, rows, cols, rows), NodeKind::Perimeter);

        assert_eq!(classify_node(5, 5, cols, rows), NodeKind::Interior);
    }

    #[test]
    fn distance_to_box_and_corner() {
        // Прямоугольник 8x3 в ячейках (0,0) .. (8,3)
        assert_eq!(dist_to_box(0, 0, 0, 0, 8, 3), 0.0);
        assert_eq!(dist_to_box(4, 2, 0, 0, 8, 3), 0.0);
        assert_eq!(dist_to_box(8, 3, 0, 0, 8, 3), 0.0);

        // На 1 ячейку вправо от правого края: (9, 2)
        assert_eq!(dist_to_box(9, 2, 0, 0, 8, 3), 1.0);
        // На 2 ячейки ниже: (4, 5)
        assert_eq!(dist_to_box(4, 5, 0, 0, 8, 3), 2.0);
        // По диагонали на 1 ячейку: (9, 4)
        let d = dist_to_box(9, 4, 0, 0, 8, 3);
        assert!((d - std::f32::consts::SQRT_2).abs() < 1e-5);

        // Отдельная точка-угол (bw = 0, bh = 0): расстояние до (10, 10)
        assert_eq!(dist_to_box(10, 10, 10, 10, 0, 0), 0.0);
        assert_eq!(dist_to_box(13, 14, 10, 10, 0, 0), 5.0); // 3-4-5 треугольник
    }

    #[test]
    fn attenuation_and_highlight_falloff() {
        assert_eq!(node_attenuation(0.0, 2.5), 1.0);
        assert_eq!(node_attenuation(2.5, 2.5), 0.0);
        assert_eq!(node_attenuation(3.0, 2.5), 0.0);

        let mid = node_attenuation(1.0, 2.5);
        assert!(mid > 0.5 && mid < 1.0);

        let far = node_attenuation(2.0, 2.5);
        assert!(far > 0.0 && far < mid);

        // Без фокуса подсветка нулевая
        assert_eq!(highlight_factor(0, 0, None, 2.5), 0.0);
        // С фокусом
        assert_eq!(highlight_factor(4, 4, Some((4, 4, 0, 0)), 2.5), 1.0);
    }

    #[test]
    fn flow_wraps_rows() {
        let (pos, w, h) = flow(&[60.0, 60.0, 60.0], 140.0, 20.0, 6.0);
        assert_eq!(pos, vec![(0.0, 0.0), (66.0, 0.0), (0.0, 26.0)]);
        assert_eq!(w, 126.0);
        assert_eq!(h, 46.0);
        let (pos, w, h) = flow(&[], 100.0, 20.0, 6.0);
        assert!(pos.is_empty() && w == 0.0 && h == 0.0);
        // Одна длинная плашка не уходит в бесконечный перенос.
        let (pos, _, h) = flow(&[300.0, 20.0], 100.0, 20.0, 6.0);
        assert_eq!(pos, vec![(0.0, 0.0), (0.0, 26.0)]);
        assert_eq!(h, 46.0);
    }

    #[test]
    fn hit_areas_sit_on_corners() {
        let r = Rect::new(20.0, 40.0, 100.0, 50.0);
        assert!(badge_hit(r).contains((22.0, 42.0)));
        assert!(!badge_hit(r).contains((60.0, 60.0)));
        assert!(size_hit(r).contains((118.0, 88.0)));
        assert!(!size_hit(r).contains((60.0, 60.0)));
    }

    #[test]
    fn unconfigured_grid_is_inert() {
        let mut g = Grid::default();
        assert!(!g.can_edit());
        g.begin_edit();
        assert!(!g.is_editing() && g.extra() == 0.0);
        assert!(!g.animating());
        g.set_text("anything");
        assert!(g.take_saved().is_none() && !g.take_resize());
    }

    #[test]
    fn handle_geometry_around_area() {
        let area = Rect::new(14.0, 38.0, 200.0, 100.0);
        let (bar_l, hit_l) = Grid::handle_geom(Side::Left, area);
        assert!(bar_l.cx() < area.x);
        assert_eq!(bar_l.cy(), area.cy());
        assert!(hit_l.x < area.x);

        let (bar_r, hit_r) = Grid::handle_geom(Side::Right, area);
        assert!(bar_r.cx() > area.right());
        assert_eq!(bar_r.cy(), area.cy());
        assert!(hit_r.right() > area.right());

        let (bar_t, hit_t) = Grid::handle_geom(Side::Top, area);
        assert!(bar_t.cy() < area.y);
        assert_eq!(bar_t.cx(), area.cx());
        assert!(hit_t.y < area.y);

        let (bar_b, hit_b) = Grid::handle_geom(Side::Bottom, area);
        assert!(bar_b.cy() > area.bottom());
        assert_eq!(bar_b.cx(), area.cx());
        assert!(hit_b.bottom() > area.bottom());
    }

    #[test]
    fn grid_stretching_and_end_edit_tightens() {
        use super::layout::{Spec, TabSpec, Size, sz};
        static SIZES: [Size; 1] = [sz(4, 2)];
        static SPEC: TabSpec = TabSpec {
            cols: 16,
            rows: 10,
            min_cols: 4,
            min_rows: 2,
            specs: &[
                Spec { key: "a", title: "A", sizes: &SIZES, size: 0, col: 0, row: 0 },
            ],
        };

        let mut g = Grid::default();
        g.configure(0, &SPEC, "a@0,0");
        // Плотное окно для одного виджета 4x2
        assert_eq!(g.window(), Window { c0: 0, r0: 0, c1: 4, r1: 2 });

        g.begin_edit();
        assert!(g.is_editing());
        // В режиме правки растягиваем вправо на 4 ячейки (клик)
        g.window = layout::expand_side(g.window, &SPEC, Side::Right, 4);
        assert_eq!(g.window, Window { c0: 0, r0: 0, c1: 8, r1: 2 });

        // Растягиваем вниз на 3 ячейки
        g.window = layout::expand_side(g.window, &SPEC, Side::Bottom, 3);
        assert_eq!(g.window, Window { c0: 0, r0: 0, c1: 8, r1: 5 });

        // По «Готово» временные расширения сбрасываются, и окно снова плотно обтягивает виджет
        g.end_edit();
        assert!(!g.is_editing());
        assert_eq!(g.window(), Window { c0: 0, r0: 0, c1: 4, r1: 2 });
        assert!(g.take_resize());
    }

    #[test]
    fn grid_overlay_rows_dynamic_expansion() {
        use super::layout::{sz, Size, Spec, TabSpec};
        static SIZES: [Size; 1] = [sz(4, 2)];
        static SPEC: TabSpec = TabSpec {
            cols: 16,
            rows: 16,
            min_cols: 4,
            min_rows: 2,
            specs: &[Spec { key: "a", title: "A", sizes: &SIZES, size: 0, col: 0, row: 0 }],
        };

        let mut g = Grid::default();
        g.configure(0, &SPEC, "a@0,0");
        assert_eq!(g.window().rows(), 2);
        assert_eq!(g.overlay_rows(), 0);

        // Оверлей требует 8 рядов: окно увеличивается
        g.set_overlay_rows(8);
        assert_eq!(g.overlay_rows(), 8);
        assert_eq!(g.window().rows(), 8);
        assert!(g.take_resize());

        // Повторная установка того же числа рядов не генерирует лишний resize
        g.set_overlay_rows(8);
        assert!(!g.take_resize());

        // Оверлей закрылся: окно возвращается к min_rows = 2
        g.set_overlay_rows(0);
        assert_eq!(g.overlay_rows(), 0);
        assert_eq!(g.window().rows(), 2);
        assert!(g.take_resize());
    }
}
