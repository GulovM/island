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

use super::layout::{self, Layout, Placed, TabSpec, CELL};
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

const RED: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.92, g: 0.27, b: 0.27, a: 1.0 };
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
        self.pending = Some(text.to_string());
        self.snap = true;
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
    }

    pub fn end_edit(&mut self) {
        if !self.editing {
            return;
        }
        self.abort_drag();
        self.resizing = None;
        self.editing = false;
        self.edit.target = 0.0;
        self.tray_open = false;
    }

    /// Вкладка стала видимой: пружины встают на конечные значения, а положения виджетов
    /// выставляются сразу, без «полёта» из нуля или со старых мест.
    pub fn show(&mut self) {
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
            || !(self.edit.settled() && self.lift.settled() && self.tray.settled() && self.note.settled() && self.empty.settled())
            || self.slots.iter().any(|s| !s.settled())
            || self.ghost.iter().any(|g| !g.settled())
    }

    // ---------- Служебное ----------

    fn wid(&self, idx: usize, k: u32) -> u32 {
        tab_id(self.tab, 0x1000 + idx as u32 * 8 + k)
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
            if self.drag.is_none() && ui.is_dragging(id) {
                let target = self.lay.entry(idx).map(|e| (e.col, e.row));
                self.drag = Some(Drag { idx, press: ptr, grab: (ptr.0 - r.x, ptr.1 - r.y), live: false, float: (r.x, r.y), target });
            }
        }

        if let Some(idx) = shown {
            if self.lay.place_new(spec, skip, idx) {
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
            if self.lay.cycle_size(spec, skip, idx, area.h) {
                self.commit();
            } else {
                self.say("Не хватает места");
            }
        }

        self.resize_step(ui, skip);
        self.drag_step(ui, area, skip);

        // Нажатие мимо лотка закрывает его (кроме случая, когда на вкладке ничего не осталось).
        if self.tray_open && ui.pressed && self.lay.visible_count(skip) > 0 {
            let on_toggle = ptr.1 >= area.bottom() && ptr.1 < area.bottom() + 40.0 && ptr.0 < area.x + 160.0;
            if !tray.card.contains(ptr) && !on_toggle {
                self.tray_open = false;
            }
        }
    }

    /// Угловая ручка: пока кнопка зажата и курсор двигается, виджет выбирает ближайший
    /// допустимый размер из `Spec::sizes`.
    fn resize_step(&mut self, ui: &mut Ui, skip: u32) {
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
        }
        if rs.live {
            let (w, h) = (rs.from.0 + px - rs.press.0, rs.from.1 + py - rs.press.1);
            if let Some(lay) = self.lay.resized_to(spec, skip, rs.idx, w, h) {
                self.lay = lay;
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
            let want_col = ((fx - area.x + CELL / 2.0) / CELL).floor().max(0.0) as u8;
            let want_row = ((fy - area.y + CELL / 2.0) / CELL).floor().max(0.0) as u8;
            let cur_size = self.lay.entry(d.idx).map_or(0, |e| e.size);
            let target = layout::nearest_free(spec, skip, &self.lay.entries, d.idx, cur_size, want_col, want_row);
            d.target = target;
            if let Some((tc, tr)) = target {
                let sz = spec.specs[d.idx].sizes[cur_size];
                let gx = area.x + tc as f32 * CELL;
                let gy = area.y + tr as f32 * CELL;
                let gw = sz.pixel_w();
                let gh = sz.pixel_h();
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
        if let Some(text) = self.pending.take() {
            let lay = Layout::parse(spec, &text);
            self.lay = if lay.fits(spec, skip, area.h) { lay } else { Layout::default_for(spec) };
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

        // Раскладка → цели пружин.
        let mut placed = self.lay.pack(spec, skip);
        if !self.editing && self.drag.is_none() && !self.lay.is_default(spec) && !self.lay.fits(spec, skip, area.h) {
            self.lay = Layout::default_for(spec);
            placed = self.lay.pack(spec, skip);
        }
        self.retarget(area, &placed);
        let live = self.drag.as_ref().filter(|d| d.live).map(|d| d.idx);
        self.empty.target = if placed.is_empty() { 1.0 } else { 0.0 };

        // Тень-приёмник под поднятым виджетом.
        if let (Some(_idx), true) = (live, self.lift.value > 0.01) {
            let r = Rect::new(self.ghost[0].value, self.ghost[1].value, self.ghost[2].value.max(0.0), self.ghost[3].value.max(0.0));
            let k = self.lift.value.clamp(0.0, 1.0);
            cv.rr(r, 11.0, fade(cv.accent, 0.12 * k));
            cv.rr_line(r, 11.0, fade(cv.accent, 0.55 * k), 1.2);
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
                bcv.circle(bx, by, br, RED);
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
            ncv.text(&tr(self.note_text), bar.note, &f.small_c, ui::ORANGE);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
