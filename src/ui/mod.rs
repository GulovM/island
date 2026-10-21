//! UI-тулкит этапа 2: иконки, примитивы (карточка, кнопка, тоггл, слайдер, кольцо, прогресс,
//! чип, строка списка), hover/pressed-пружины, скролл с инерцией, текстовое поле, кэш битмапов.
//!
//! Режим «immediate»: вкладка в каждом кадре вызывает виджеты; состояние (пружины наведения,
//! прокрутка, текст поля) живёт в `Ui` и ищется по числовому `id`. Координаты — логические
//! пиксели относительно левого края панели (масштаб DPI применяет трансформация холста).

use std::collections::HashMap;

use crate::anim::Spring;

mod color;
pub use color::*;

mod glyphs;
pub use glyphs::icon;

mod symbols;
pub(crate) use symbols::{ICON, ICON_BIG};

mod geom;
pub use geom::*;

mod bitmap_cache;
pub use bitmap_cache::*;

mod fonts;
pub use fonts::*;

mod canvas;
pub use canvas::*;

mod scroll;
use scroll::{Scroll, SCROLL_FRICTION};

mod field;
pub use field::{KeyEv, Nav, Submit};
use field::Field;

mod widgets;
pub use widgets::*;

// ---------- Состояния виджетов ----------

pub(super) struct Widget {
    pub(super) hover: Spring,
    pub(super) press: Spring,
    pub(super) on: Spring,
}

impl Widget {
    fn new() -> Self {
        Self {
            hover: Spring::unit(0.0, 520.0, 38.0),
            press: Spring::unit(0.0, 700.0, 44.0),
            on: Spring::unit(0.0, 420.0, 34.0),
        }
    }
    fn settled(&self) -> bool {
        self.hover.settled() && self.press.settled() && self.on.settled()
    }
}

// ---------- Ui ----------

#[derive(Default)]
pub struct Ui {
    /// Курсор в логических пикселях окна (не панели).
    cursor: (f32, f32),
    /// Курсор в координатах текущего слоя содержимого.
    mouse: (f32, f32),
    origin_x: f32,
    on_island: bool,
    interactive: bool,
    /// Кнопка удерживается / только что нажата / только что отпущена (в этом кадре).
    pub down: bool,
    pub pressed: bool,
    pub released: bool,
    /// Накопленные «ступеньки» колеса (положительное — вверх).
    pub wheel: f32,
    clip: Option<Rect>,
    drag: Option<u32>,
    widgets: HashMap<u32, Widget>,
    scrolls: HashMap<u32, Scroll>,
    fields: HashMap<u32, Field>,
    /// Поле с клавиатурным фокусом.
    pub focus: Option<u32>,
    /// Поле, которое возвращает себе фокус после нажатия мимо полей (поиск в буфере обмена).
    pub sticky_focus: Option<u32>,
    pub caret_on: bool,
    focus_claimed: bool,
    /// Куда поставить окно IME: (x, y) в логических пикселях окна.
    pub ime_pos: Option<(f32, f32)>,
    /// Текст, который нужно положить в буфер обмена (Ctrl+C / Ctrl+X).
    pub clipboard_out: Option<String>,
    /// Навигационные клавиши для сфокусированного поля (стрелки, Ctrl+P).
    pub nav: Vec<Nav>,
    /// Поле, в котором нажали Enter (только для полей с `Submit != None`).
    submitted: Option<(u32, String)>,
    pub cache: BitmapCache,
}

impl Ui {
    // --- кадр ---

    pub fn begin_frame(&mut self, cursor: (f32, f32), on_island: bool) {
        self.cursor = cursor;
        self.on_island = on_island;
        if !self.down && !self.pressed && !self.released {
            self.drag = None;
        }
        self.focus_claimed = false;
        self.sticky_focus = None;
        self.ime_pos = None;
    }

    pub fn end_frame(&mut self) {
        // Нажатие мимо текстового поля снимает фокус ввода.
        if self.pressed && !self.focus_claimed {
            self.focus = self.sticky_focus;
        }
        if self.released {
            self.drag = None;
        }
        self.pressed = false;
        self.released = false;
        self.wheel = 0.0;
    }

    /// Начало слоя содержимого: `ox` — левый край панели в логических пикселях окна.
    pub fn begin_content(&mut self, ox: f32, interactive: bool) {
        self.origin_x = ox;
        self.mouse = (self.cursor.0 - ox, self.cursor.1);
        self.interactive = interactive && self.on_island;
        self.clip = None;
    }

    /// Мышь «захвачена» перетаскиванием или фокусом ввода: сворачивать остров нельзя.
    pub fn holds_pointer(&self) -> bool {
        (self.drag.is_some() && self.down) || self.focus.is_some()
    }

    pub fn blur(&mut self) {
        self.focus = None;
    }

    pub fn step(&mut self, dt: f32) {
        for w in self.widgets.values_mut() {
            w.hover.step(dt);
            w.press.step(dt);
            w.on.step(dt);
        }
        for s in self.scrolls.values_mut() {
            s.off += s.vel * dt;
            s.vel *= (-SCROLL_FRICTION * dt).exp();
            if s.vel.abs() < 3.0 {
                s.vel = 0.0;
            }
            if s.off < 0.0 || s.off > s.max {
                s.off = s.off.clamp(0.0, s.max);
                s.vel = 0.0;
            }
            s.vis.target = if s.vel != 0.0 || s.hot { 1.0 } else { 0.0 };
            s.vis.step(dt);
        }
    }

    pub fn animating(&self) -> bool {
        self.widgets.values().any(|w| !w.settled())
            || self.scrolls.values().any(|s| s.vel != 0.0 || !s.vis.settled())
    }

    /// Освободить устройство-зависимые ресурсы (потеря графики).
    pub fn reset_gfx(&mut self) {
        self.cache.clear();
    }

    pub(super) fn widget(&mut self, id: u32) -> &mut Widget {
        self.widgets.entry(id).or_insert_with(Widget::new)
    }

    /// Пружина наведения для элемента, который рисует сам вызывающий (ряд вкладок).
    pub fn hover_amount(&mut self, id: u32, hot: bool) -> f32 {
        let hot = hot && self.drag.is_none();
        let w = self.widget(id);
        w.hover.target = if hot { 1.0 } else { 0.0 };
        w.hover.value
    }

    /// Пружина «присутствия» (появление и исчезновение) для элемента, который рисует сам вызывающий.
    pub fn presence(&mut self, id: u32, on: bool) -> f32 {
        let w = self.widget(id);
        w.on.target = if on { 1.0 } else { 0.0 };
        w.on.value
    }

    /// Нажатая кнопка мыши не досталась ни одному виджету (долгое нажатие по пустому месту).
    pub fn press_free(&self) -> bool {
        self.drag.is_none()
    }

    /// Hit-testing: (наведён, клик). Клик — отпускание над тем же элементом, где нажали.
    pub(super) fn interact(&mut self, id: u32, r: Rect) -> (bool, bool) {
        let inside = self.interactive
            && r.contains(self.mouse)
            && self.clip.is_none_or(|c| c.contains(self.mouse));
        let hover = inside && (self.drag.is_none() || self.drag == Some(id));
        if hover && self.pressed {
            self.drag = Some(id);
        }
        let mine = self.drag == Some(id);
        let clicked = mine && self.released && inside;
        let down = self.down;
        let w = self.widget(id);
        w.hover.target = if hover || (mine && down) { 1.0 } else { 0.0 };
        w.press.target = if mine && down && inside { 1.0 } else { 0.0 };
        (hover, clicked)
    }

    pub fn is_dragging(&self, id: u32) -> bool {
        self.drag == Some(id) && self.down
    }

    /// Курсор в координатах текущего слоя (как у `Rect` виджетов этого слоя).
    pub fn pointer(&self) -> (f32, f32) {
        self.mouse
    }

    /// Выключает ввод для всего, что рисуется дальше (режим правки: виджеты только смотрят).
    /// Возвращает прежнее состояние для [`Ui::unmute`].
    pub fn mute(&mut self) -> bool {
        std::mem::replace(&mut self.interactive, false)
    }

    pub fn unmute(&mut self, prev: bool) {
        self.interactive = prev;
    }
}
