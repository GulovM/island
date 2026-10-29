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

/// Бегущая строка: заголовок, не влезший в плитку, один раз прокручивается и дальше стоит на месте.
pub(super) struct Marquee {
    /// Время с начала прокрутки, секунды.
    pub(super) t: f32,
    /// Полная длительность (пауза, прокрутка, пауза); пока `t` меньше, кадры нужны.
    pub(super) total: f32,
    /// Рисовалась ли строка в текущем кадре: пропавшая из кадра начнёт заново.
    drawn: bool,
}

// ---------- Ui ----------

#[derive(Default)]
pub struct Ui {
    marquees: HashMap<u32, Marquee>,
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
    /// Поле или виджет с клавиатурным фокусом.
    pub focus: Option<u32>,
    /// Поле, которое возвращает себе фокус после нажатия мимо полей (поиск в буфере обмена).
    pub sticky_focus: Option<u32>,
    pub caret_on: bool,
    focus_claimed: bool,
    /// Все интерактивные id, прошедшие через `interact()` в этом кадре, в порядке отрисовки —
    /// по ним идёт `Tab`/`Shift+Tab`, когда фокус не на текстовом поле.
    focusables: Vec<u32>,
    /// Последний прямоугольник сфокусированного виджета в этом кадре (для кольца фокуса).
    focus_rect: Option<Rect>,
    /// Дикторское имя сфокусированного виджета в этом кадре, если оно было зарегистрировано
    /// (`tip()` на icon-only кнопках) — для `report_focus` (UI Automation).
    focus_name: Option<String>,
    /// Фокус пришёл клавишей `Tab`, а не кликом мыши: кольцо фокуса рисуется только в этом случае
    /// (`:focus-visible`), не при обычном наведении или клике.
    kbd_nav: bool,
    /// Enter/Space на сфокусированном нетекстовом виджете — активирует его в этом кадре, как клик.
    key_activate: bool,
    /// Куда поставить окно IME: (x, y) в логических пикселях окна.
    pub ime_pos: Option<(f32, f32)>,
    /// Текст, который нужно положить в буфер обмена (Ctrl+C / Ctrl+X).
    pub clipboard_out: Option<String>,
    /// Навигационные клавиши для сфокусированного поля (стрелки, Ctrl+P).
    pub nav: Vec<Nav>,
    /// Поле, в котором нажали Enter (только для полей с `Submit != None`).
    submitted: Option<(u32, String)>,
    /// Подсказка под курсором: прямоугольник кнопки, текст и степень наведения; рисуется в конце кадра.
    tip: Option<(Rect, String, f32)>,
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
        self.focusables.clear();
        self.focus_rect = None;
        self.focus_name = None;
        // Строка, которой не было в прошлом кадре (другая вкладка, остров был закрыт), прокрутится заново.
        self.marquees.retain(|_, m| m.drawn);
        for m in self.marquees.values_mut() {
            m.drawn = false;
        }
    }

    /// Все бегущие строки начнут прокрутку заново (остров открыт, вкладка переключена).
    pub fn reset_marquees(&mut self) {
        self.marquees.clear();
    }

    /// Прокрутка заголовка `id`: смещение текста в пикселях при ширине текста `text_w` в окне `view_w`.
    pub(super) fn marquee_offset(&mut self, id: u32, text_w: f32, view_w: f32) -> Option<f32> {
        const PAUSE: f32 = 0.9;
        const SPEED: f32 = 20.0;
        let dist = text_w - view_w + 4.0;
        let m = self.marquees.entry(id).or_insert(Marquee { t: 0.0, total: 0.0, drawn: false });
        m.drawn = true;
        m.total = 2.0 * PAUSE + dist / SPEED;
        if m.t >= m.total {
            return None;
        }
        Some(-((m.t - PAUSE) * SPEED).clamp(0.0, dist))
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
        self.key_activate = false;
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

    /// `id` сейчас в фокусе ради ввода текста (а не ради клавиатурной активации клика).
    pub fn is_field(&self, id: u32) -> bool {
        self.fields.contains_key(&id)
    }

    /// `Tab`/`Shift+Tab` вне текстового поля: следующий/предыдущий виджет из прошлого кадра
    /// (`focusables` заполняется заново каждый кадр, так что к моменту вызова он уже готов).
    /// Без фокуса идёт к первому (или последнему при `shift`) виджету.
    pub fn request_tab(&mut self, shift: bool) {
        if self.focusables.is_empty() {
            return;
        }
        let last = self.focusables.len() - 1;
        let cur = self.focus.and_then(|id| self.focusables.iter().position(|&x| x == id));
        let next = match cur {
            Some(i) if shift => if i == 0 { last } else { i - 1 },
            Some(i) => if i == last { 0 } else { i + 1 },
            None if shift => last,
            None => 0,
        };
        self.focus = Some(self.focusables[next]);
        self.kbd_nav = true;
    }

    /// `Enter`/`Space` на сфокусированном нетекстовом виджете — засчитывается как клик по нему
    /// в этом кадре (одноразово, сбрасывается в `end_frame`).
    pub fn request_activate(&mut self) {
        self.key_activate = true;
    }

    /// Виджет, до которого дошли клавиатурой (`Tab`) в этом кадре, и его дикторское имя, если оно
    /// было зарегистрировано (`tip()`) — для сообщения экранному диктору (UI Automation).
    pub fn kbd_focus(&self) -> Option<(u32, Option<&str>)> {
        if !self.kbd_nav {
            return None;
        }
        self.focus.map(|id| (id, self.focus_name.as_deref()))
    }

    /// Кольцо вокруг виджета, к которому дошли клавиатурой (`Tab`), а не мышью: рисуется поверх
    /// всего в конце кадра, как подсказка — само решает, нужно ли показываться.
    pub unsafe fn paint_focus_ring(&self, cv: &Canvas) {
        if !self.kbd_nav {
            return;
        }
        let Some(r) = self.focus_rect else { return };
        cv.rr_line(r.inset(-2.0), 8.0, cv.accent, 1.5);
    }

    pub fn step(&mut self, dt: f32) {
        for w in self.widgets.values_mut() {
            w.hover.step(dt);
            w.press.step(dt);
            w.on.step(dt);
        }
        for m in self.marquees.values_mut().filter(|m| m.t < m.total) {
            m.t += dt;
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
            || self.marquees.values().any(|m| m.drawn && m.t < m.total)
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

    /// Плавно ведёт значение элемента (0..1) к `target`; возвращает текущее.
    pub fn anim_to(&mut self, id: u32, target: f32) -> f32 {
        let w = self.widget(id);
        w.on.target = target;
        w.on.value
    }

    /// Нажатая кнопка мыши не досталась ни одному виджету (долгое нажатие по пустому месту).
    pub fn press_free(&self) -> bool {
        self.drag.is_none()
    }

    /// Hit-testing: (наведён, клик). Клик — отпускание над тем же элементом, где нажали, или
    /// `Enter`/`Space`, пока `id` в клавиатурном фокусе (`request_activate`).
    pub(super) fn interact(&mut self, id: u32, r: Rect) -> (bool, bool) {
        if self.interactive {
            self.focusables.push(id);
            if self.focus == Some(id) {
                self.focus_rect = Some(r);
            }
        }
        let inside = self.interactive
            && r.contains(self.mouse)
            && self.clip.is_none_or(|c| c.contains(self.mouse));
        let hover = inside && (self.drag.is_none() || self.drag == Some(id));
        if hover && self.pressed {
            self.drag = Some(id);
            // Настоящий клик мышью переключает управление обратно на неё: кольцо фокуса,
            // показанное клавиатурной навигацией, больше не нужно.
            self.kbd_nav = false;
        }
        let mine = self.drag == Some(id);
        let key_clicked = self.interactive && self.key_activate && self.focus == Some(id);
        let clicked = (mine && self.released && inside) || key_clicked;
        let down = self.down;
        let w = self.widget(id);
        w.hover.target = if hover || (mine && down) { 1.0 } else { 0.0 };
        w.press.target = if (mine && down && inside) || key_clicked { 1.0 } else { 0.0 };
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

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };

    /// Один кадр интерактивного слоя: курсор заведомо вне `R`, чтобы мышь не мешала проверке клавиатуры.
    fn frame(ui: &mut Ui) {
        ui.begin_frame((9000.0, 9000.0), true);
        ui.begin_content(0.0, true);
        ui.pressed = false;
        ui.released = false;
        ui.down = false;
    }

    #[test]
    fn tab_cycles_through_focusables_in_draw_order() {
        let mut ui = Ui::default();
        frame(&mut ui);
        let _ = ui.interact(1, R);
        let _ = ui.interact(2, R);
        let _ = ui.interact(3, R);
        ui.end_frame();

        ui.request_tab(false);
        assert_eq!(ui.focus, Some(1));
        ui.request_tab(false);
        assert_eq!(ui.focus, Some(2));
        ui.request_tab(true);
        assert_eq!(ui.focus, Some(1));
        // С конца назад — заворачивает на последний.
        ui.request_tab(true);
        assert_eq!(ui.focus, Some(3));
    }

    #[test]
    fn enter_activates_focused_widget_without_mouse() {
        let mut ui = Ui::default();
        frame(&mut ui);
        let _ = ui.interact(1, R);
        let _ = ui.interact(2, R);
        ui.end_frame();

        ui.request_tab(false); // фокус -> 1
        ui.request_activate();

        frame(&mut ui);
        let (_, clicked1) = ui.interact(1, R);
        let (_, clicked2) = ui.interact(2, R);
        assert!(clicked1, "Enter/Space на виджете 1 должен засчитаться как клик по нему");
        assert!(!clicked2, "а не по соседнему виджету 2");
    }

    #[test]
    fn real_mouse_press_clears_keyboard_focus_ring() {
        let mut ui = Ui::default();
        frame(&mut ui);
        let _ = ui.interact(1, R);
        ui.end_frame();
        ui.request_tab(false);
        assert!(ui.kbd_nav, "после Tab кольцо фокуса должно быть видимым");

        // Настоящий клик мышью по тому же виджету.
        ui.begin_frame((5.0, 5.0), true);
        ui.begin_content(0.0, true);
        ui.pressed = true;
        ui.down = true;
        let _ = ui.interact(1, R);
        assert!(!ui.kbd_nav, "клик мышью возвращает управление ей — кольцо больше не нужно");
    }

    #[test]
    fn is_field_distinguishes_text_fields_from_ordinary_widgets() {
        let mut ui = Ui::default();
        frame(&mut ui);
        let _ = ui.interact(1, R);
        ui.set_field_text(2, "текст");
        ui.end_frame();

        assert!(!ui.is_field(1));
        assert!(ui.is_field(2));
    }
}
