//! UI-тулкит этапа 2: иконки, примитивы (карточка, кнопка, тоггл, слайдер, кольцо, прогресс,
//! чип, строка списка), hover/pressed-пружины, скролл с инерцией, текстовое поле, кэш битмапов.
//!
//! Режим «immediate»: вкладка в каждом кадре вызывает виджеты; состояние (пружины наведения,
//! прокрутка, текст поля) живёт в `Ui` и ищется по числовому `id`. Координаты — логические
//! пиксели относительно левого края панели (масштаб DPI применяет трансформация холста).

use std::collections::HashMap;

use windows::core::BOOL;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

use crate::anim::Spring;
mod color;
pub use color::*;

mod glyphs;
pub use glyphs::icon;

mod geom;
pub use geom::*;
mod bitmap_cache;
pub use bitmap_cache::*;

mod fonts;
pub use fonts::*;

mod canvas;
pub use canvas::*;

// ---------- Состояния виджетов ----------

struct Widget {
    hover: Spring,
    press: Spring,
    on: Spring,
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

struct Scroll {
    off: f32,
    vel: f32,
    max: f32,
    content: f32,
    view: Rect,
    vis: Spring,
    /// Курсор над полосой или она перетаскивается: полоса остаётся видимой.
    hot: bool,
    /// Расстояние от верха бегунка до курсора в момент захвата.
    grab: f32,
}

/// Ширина зоны захвата полосы прокрутки у правого края области.
const SCROLLBAR_GRAB_W: f32 = 12.0;

/// Импульс прокрутки на одну «ступеньку» колеса (пикселей/с) и затухание инерции (1/с).
const WHEEL_IMPULSE: f32 = 620.0;
const SCROLL_FRICTION: f32 = 7.0;

#[derive(Default)]
struct Field {
    chars: Vec<char>,
    caret: usize,
    anchor: usize,
    scroll: f32,
    submit: Submit,
    /// Поле пароля: символы показываются точками, копировать нельзя.
    secret: bool,
    /// Только цифры (не больше 5 знаков): поля времени.
    digits: bool,
}

/// Стрелка справа на широкой плитке.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Chevron {
    None,
    Plain,
    /// С тонкой вертикальной чертой перед стрелкой (выпадающий список).
    Divided,
}

/// Результат нажатия на широкую плитку.
#[derive(Default, Clone, Copy)]
pub struct TileHit {
    pub toggled: bool,
    pub chevron: bool,
}

/// Что делает Enter в текстовом поле.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Submit {
    /// Снимает фокус (поиск).
    #[default]
    None,
    /// Сообщает текст, очищает поле, фокус остаётся (добавление задачи).
    Clear,
    /// Сообщает текст, поле и фокус остаются (поиск с действием по Enter).
    Keep,
}

/// Навигация по списку с клавиатуры, пока в поле поиска есть фокус.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    Up,
    Down,
    Pin,
    /// Ctrl+C без выделенного текста в поле: копировать выбранную запись списка.
    Copy,
    /// Delete при пустом поле: удалить выбранную запись.
    Delete,
    /// Tab: следующий фильтр.
    Tab,
}

impl Field {
    fn range(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }
    fn has_sel(&self) -> bool {
        self.caret != self.anchor
    }
    fn delete_sel(&mut self) -> bool {
        if !self.has_sel() {
            return false;
        }
        let (lo, hi) = self.range();
        self.chars.drain(lo..hi);
        self.caret = lo;
        self.anchor = lo;
        true
    }
    fn selected(&self) -> String {
        let (lo, hi) = self.range();
        self.chars[lo..hi].iter().collect()
    }
    fn insert(&mut self, s: &str) {
        self.delete_sel();
        let digits = self.digits;
        for c in s.chars().filter(|c| !c.is_control() && (!digits || c.is_ascii_digit())) {
            if self.chars.len() >= if digits { 5 } else { FIELD_MAX } {
                break;
            }
            self.chars.insert(self.caret, c);
            self.caret += 1;
        }
        self.anchor = self.caret;
    }
}

const FIELD_MAX: usize = 256;

/// Клавиатурное событие для сфокусированного текстового поля.
pub enum KeyEv {
    Char(char),
    Left(bool),
    Right(bool),
    Home(bool),
    End(bool),
    Backspace,
    Delete,
    SelectAll,
    Copy,
    Cut,
    Paste(String),
    Enter,
    Escape,
    Up,
    Down,
    Pin,
    Tab,
}

fn utf16_len(chars: &[char]) -> u32 {
    chars.iter().map(|c| c.len_utf16() as u32).sum()
}

fn char_at_utf16(chars: &[char], pos16: u32) -> usize {
    let mut acc = 0;
    for (i, c) in chars.iter().enumerate() {
        if acc >= pos16 {
            return i;
        }
        acc += c.len_utf16() as u32;
    }
    chars.len()
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

    fn widget(&mut self, id: u32) -> &mut Widget {
        self.widgets.entry(id).or_insert_with(Widget::new)
    }

    /// Пружина наведения для элемента, который рисует сам вызывающий (ряд вкладок).
    pub fn hover_amount(&mut self, id: u32, hot: bool) -> f32 {
        let hot = hot && self.drag.is_none();
        let w = self.widget(id);
        w.hover.target = if hot { 1.0 } else { 0.0 };
        w.hover.value
    }

    /// Hit-testing: (наведён, клик). Клик — отпускание над тем же элементом, где нажали.
    fn interact(&mut self, id: u32, r: Rect) -> (bool, bool) {
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

    // --- примитивы ---

    /// Плитка-кнопка (Power): иконка сверху, жирная подпись под ней.
    pub unsafe fn button(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, label: &str) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let rect = r.inset(1.0 * pr);
        cv.rr(rect, 10.0, mix(card(), fg(), 0.055 * hv + 0.03 * pr));
        if label.is_empty() {
            cv.glyph(glyph, rect, false, txt());
        } else if glyph.is_empty() {
            cv.text(label, rect, &cv.g.fonts.body_c, txt());
        } else {
            cv.glyph(glyph, Rect::new(rect.x, rect.cy() - 18.0, rect.w, 18.0), false, txt());
            cv.text(label, Rect::new(rect.x, rect.cy() + 1.0, rect.w, 16.0), &cv.g.fonts.body_c, txt());
        }
        clicked
    }

    /// Широкая плитка-тоггл как в макете: круглый бейдж с иконкой слева, заголовок и подпись по
    /// центру, справа может быть стрелка «⌄» (с разделителем или без). При «вкл» плитка залита
    /// акцентом с тёмным текстом.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn tile_wide(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        glyph: &str,
        title: &str,
        sub: &str,
        on: &mut bool,
        chevron: Chevron,
    ) -> TileHit {
        let chev_w = if chevron == Chevron::None { 0.0 } else { 24.0 };
        let body = Rect::new(r.x, r.y, r.w - chev_w, r.h);
        let chev = Rect::new(r.right() - chev_w, r.y, chev_w, r.h);
        let mut hit = TileHit::default();
        if chev_w > 0.0 {
            let (_, c) = self.interact(id | 0x8000, chev);
            hit.chevron = c;
        }
        let (_, clicked) = self.interact(id, body);
        if clicked {
            *on = !*on;
            hit.toggled = true;
        }
        let chev_hv = if chev_w > 0.0 { self.widget(id | 0x8000).hover.value } else { 0.0 };
        let w = self.widget(id);
        w.on.target = if *on { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);
        let rect = r.inset(0.8 * pr);
        let fill = mix(mix(card(), fg(), 0.05 * hv.max(chev_hv)), mix(cv.accent, fg(), 0.06 * hv), ov);
        cv.rr(rect, 10.0, fill);
        let fg = mix(txt(), ink(), ov);
        let sub_c = mix(dim(), D2D1_COLOR_F { a: 1.0, ..mix(ink(), cv.accent, 0.35) }, ov);
        // Бейдж.
        let (bx, by) = (rect.x + 19.0, rect.cy());
        cv.circle(bx, by, 11.0, mix(chip_bg(), mix(cv.accent, ink(), 0.16), ov));
        cv.glyph(glyph, Rect::new(bx - 11.0, by - 11.0, 22.0, 22.0), false, fg);
        // Текст по центру между бейджем и стрелкой.
        let tx = rect.x + 32.0;
        let tw = (rect.w - 32.0 - chev_w - 4.0).max(10.0);
        cv.text(title, Rect::new(tx, rect.cy() - 12.0, tw, 13.0), &cv.g.fonts.body_c, fg);
        cv.text(sub, Rect::new(tx, rect.cy() + 1.0, tw, 11.0), &cv.g.fonts.small_c, sub_c);
        if chevron != Chevron::None {
            if chevron == Chevron::Divided {
                let mut c = fg;
                c.a = 0.22;
                cv.rr(Rect::new(chev.x, rect.y + 8.0, 1.0, rect.h - 16.0), 0.5, c);
            }
            let mut c = fg;
            c.a = 0.75 + 0.25 * chev_hv;
            cv.glyph(icon::DOWN, Rect::new(chev.x, rect.y, chev_w - 2.0, rect.h), false, c);
        }
        hit
    }

    /// Полоса-слайдер из макета: тёмная дорожка, акцентная заливка от левого края с иконкой,
    /// процент справа.
    pub unsafe fn slider_bar(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, value: &mut f32) -> bool {
        let hit = Rect::new(r.x, r.y - 4.0, r.w, r.h + 8.0);
        let _ = self.interact(id, hit);
        let mut changed = false;
        if self.is_dragging(id) {
            let v = ((self.mouse.0 - r.x) / r.w.max(1.0)).clamp(0.0, 1.0);
            if (v - *value).abs() > f32::EPSILON {
                *value = v;
                changed = true;
            }
        }
        let w = self.widget(id);
        let hv = w.hover.value;
        let v = value.clamp(0.0, 1.0);
        cv.rr(r, r.h / 2.0, mix(card(), fg(), 0.04 * hv));
        if v > 0.0 {
            cv.rr(Rect::new(r.x, r.y, (r.w * v).max(r.h), r.h), r.h / 2.0, cv.accent);
        }
        cv.glyph(glyph, Rect::new(r.x + 6.0, r.y, 18.0, r.h), false, if v > 0.04 { ink() } else { txt() });
        let col = if v > 0.93 { ink() } else { txt() };
        cv.text(&format!("{}%", (v * 100.0).round() as i32), Rect::new(r.right() - 52.0, r.y, 42.0, r.h), &cv.g.fonts.body_r, col);
        changed
    }

    /// Прозрачная кнопка-иконка (звезда, крестик): `active` красит её акцентом.
    pub unsafe fn icon_button(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, active: bool) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let mut bg = fg();
        bg.a = 0.10 * hv;
        cv.rr(r, r.h / 2.0, bg);
        let col = if active { cv.accent } else { mix(rgb(0.45, 0.45, 0.48), txt(), hv) };
        cv.glyph(glyph, r.inset(1.0 * pr), false, col);
        clicked
    }

    /// Компактная строка списка (Apps, Clipboard, задачи): квадратик с буквой или иконкой,
    /// заголовок (по левому краю или по центру), справа короткая подпись. `right_pad` — место
    /// под кнопки справа.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn row_compact(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        lead: &str,
        lead_is_glyph: bool,
        title: &str,
        right: &str,
        right_pad: f32,
        selected: bool,
        centered: bool,
    ) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        w.on.target = if selected { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);
        let mut bg = fg();
        bg.a = 0.045 * hv + 0.03 * pr;
        cv.rr(r, 8.0, bg);
        if ov > 0.01 {
            let mut c = selection(cv.accent);
            c.a *= ov;
            cv.rr(r, 8.0, c);
        }
        let ts = (r.h - 7.0).clamp(12.0, 17.0);
        let tile = Rect::new(r.x + 6.0, r.cy() - ts / 2.0, ts, ts);
        cv.rr(tile, 4.0, mix(chip_bg(), mix(cv.accent, chip_bg(), 0.45), ov));
        if lead_is_glyph {
            cv.glyph(lead, tile, false, mix(dim(), txt(), ov));
        } else {
            cv.text(lead, tile, &cv.g.fonts.small_c, txt());
        }
        let tx = tile.right() + 8.0;
        let rw = if right.is_empty() { 0.0 } else { cv.text_w(right, &cv.g.fonts.small_r) + 6.0 };
        let tw = (r.right() - tx - right_pad - rw - 8.0).max(10.0);
        let font = if centered { &cv.g.fonts.body_c } else { &cv.g.fonts.body };
        cv.text(title, Rect::new(tx, r.y, tw, r.h), font, txt());
        cv.text(right, Rect::new(r.right() - right_pad - rw - 6.0, r.y, rw, r.h), &cv.g.fonts.small_r, dim());
        clicked
    }

    /// Сплошная кнопка с подписью (Старт/Сброс, «Замерить скорость»).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn solid_button(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        label: &str,
        bg: D2D1_COLOR_F,
        fg: D2D1_COLOR_F,
        radius: f32,
    ) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let rect = r.inset(0.8 * pr);
        cv.rr(rect, radius, mix(bg, crate::ui::fg(), 0.08 * hv));
        cv.text(label, rect, &cv.g.fonts.small_c, fg);
        clicked
    }

    /// Забирает текст поля (как по Enter) — для кнопки «добавить» рядом с полем.
    pub fn submit_field(&mut self, id: u32) -> Option<String> {
        let f = self.fields.get_mut(&id)?;
        let text: String = f.chars.iter().collect();
        f.chars.clear();
        f.caret = 0;
        f.anchor = 0;
        f.scroll = 0.0;
        Some(text)
    }

    /// Область для самостоятельно нарисованных элементов: (наведён, нажат) с общей логикой
    /// «клик = отпускание над тем же элементом»; пружина наведения доступна по `hover_of`.
    pub fn click_area(&mut self, id: u32, r: Rect) -> (bool, bool) {
        self.interact(id, r)
    }

    /// Курсор над прямоугольником (без захвата нажатия, в отличие от `click_area`).
    pub fn hovering(&self, r: Rect) -> bool {
        self.interactive && r.contains(self.mouse)
    }

    pub fn hover_of(&mut self, id: u32) -> f32 {
        self.widget(id).hover.value
    }

    /// Чип-фильтр. Выбранный залит акцентом. `flat` — невыбранный без фона (фильтры в строке
    /// поиска). `true` — нажат.
    pub unsafe fn chip(&mut self, cv: &Canvas, id: u32, r: Rect, label: &str, selected: bool) -> bool {
        self.chip_ex(cv, id, r, label, selected, false, None)
    }

    /// Чип с выбором стиля: `tint` — цвет выбранного вместо сплошного акцента (выбранный пресет
    /// таймера: тёмно-оранжевый фон и оранжевый текст).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn chip_ex(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        label: &str,
        selected: bool,
        flat: bool,
        tint: Option<D2D1_COLOR_F>,
    ) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        w.on.target = if selected { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);
        let rect = r.inset(0.8 * pr);
        let mut base = if flat { card() } else { chip_bg() };
        base = mix(base, fg(), 0.06 * hv);
        if flat && ov < 0.01 && hv < 0.01 {
            // Плоский невыбранный: только текст.
        } else {
            let mut bg = match tint {
                Some(t) => mix(base, D2D1_COLOR_F { a: 1.0, ..mix(card(), t, 0.28) }, ov),
                None => mix(base, cv.accent, ov),
            };
            if flat {
                bg.a = bg.a.max(0.0) * (ov + hv * 0.6).min(1.0);
            }
            cv.rr(rect, rect.h / 2.0, bg);
        }
        let fg = match tint {
            Some(t) => mix(mix(dim(), txt(), hv), t, ov),
            None => mix(mix(rgb(0.78, 0.78, 0.78), txt(), hv), ink(), ov),
        };
        cv.text(label, rect, &cv.g.fonts.small_c, fg);
        clicked
    }

    /// Переключатель-«пилюля» 30×16 в прямоугольнике `r`. `enabled == false` — серый и неактивный.
    /// Возвращает `true` в кадре, когда значение переключили.
    pub unsafe fn switch(&mut self, cv: &Canvas, id: u32, r: Rect, on: &mut bool, enabled: bool) -> bool {
        let clicked = if enabled { self.interact(id, r).1 } else { false };
        if clicked {
            *on = !*on;
        }
        let w = self.widget(id);
        w.on.target = if *on { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);
        let mut bg = mix(mix(chip_bg(), fg(), 0.05 * hv), cv.accent, ov);
        let mut knob = mix(dim(), ink(), ov);
        if !enabled {
            bg.a = 0.45;
            knob.a = 0.5;
        }
        cv.rr(r, r.h / 2.0, bg);
        let kr = r.h / 2.0 - 3.0 + 0.6 * pr;
        let kx = r.x + r.h / 2.0 + (r.w - r.h) * ov;
        cv.circle(kx, r.cy(), kr, knob);
        clicked
    }

    /// Тонкий ползунок 0..1: дорожка, акцентная заливка и круглая ручка. Возвращает `true`, пока значение меняется.
    pub unsafe fn slider_thin(&mut self, cv: &Canvas, id: u32, r: Rect, value: &mut f32) -> bool {
        let hit = Rect::new(r.x - 8.0, r.y - 8.0, r.w + 16.0, r.h + 16.0);
        let _ = self.interact(id, hit);
        let mut changed = false;
        if self.is_dragging(id) {
            let v = ((self.mouse.0 - r.x) / r.w.max(1.0)).clamp(0.0, 1.0);
            if (v - *value).abs() > f32::EPSILON {
                *value = v;
                changed = true;
            }
        }
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let v = value.clamp(0.0, 1.0);
        let cy = r.cy();
        cv.rr(Rect::new(r.x, cy - 2.0, r.w, 4.0), 2.0, track());
        if v > 0.0 {
            cv.rr(Rect::new(r.x, cy - 2.0, (r.w * v).max(4.0), 4.0), 2.0, cv.accent);
        }
        let kx = r.x + r.w * v;
        cv.circle(kx, cy, 8.0 + 2.0 * hv + 1.5 * pr, D2D1_COLOR_F { a: 0.16 * (hv + pr).min(1.0), ..cv.accent });
        cv.circle(kx, cy, 5.5 + 0.8 * hv, txt());
        cv.circle(kx, cy, 2.4, cv.accent);
        changed
    }

    /// Область, которую можно тянуть мышью (палитра цвета): пока кнопка зажата, возвращает положение
    /// курсора внутри `r` в долях 0..1 (за пределами — прижато к краю).
    pub fn drag_area(&mut self, id: u32, r: Rect) -> Option<(f32, f32)> {
        let _ = self.interact(id, r);
        if self.is_dragging(id) {
            Some((
                ((self.mouse.0 - r.x) / r.w.max(1.0)).clamp(0.0, 1.0),
                ((self.mouse.1 - r.y) / r.h.max(1.0)).clamp(0.0, 1.0),
            ))
        } else {
            None
        }
    }

    /// Круглая плашка темы: фон, внутри точка акцента; выбранная обведена акцентом. `true` — нажата.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn swatch(&mut self, cv: &Canvas, id: u32, cx: f32, cy: f32, radius: f32, bg: D2D1_COLOR_F, accent: D2D1_COLOR_F, selected: bool) -> bool {
        let hit = Rect::new(cx - radius - 2.0, cy - radius - 2.0, 2.0 * radius + 4.0, 2.0 * radius + 4.0);
        let (_, clicked) = self.interact(id, hit);
        let w = self.widget(id);
        w.on.target = if selected { 1.0 } else { 0.0 };
        let (hv, ov) = (w.hover.value, w.on.value);
        let rad = radius + 1.0 * hv;
        cv.circle(cx, cy, rad + 2.0, D2D1_COLOR_F { a: ov, ..cv.accent });
        cv.circle(cx, cy, rad, chip_bg());
        cv.circle(cx, cy, rad - 1.2, bg);
        cv.circle(cx, cy, (rad - 1.2) * 0.5, accent);
        clicked
    }

    /// Задаёт текст поля программно (поле не должно быть в фокусе, иначе оно перебьёт ввод).
    pub fn set_field_text(&mut self, id: u32, s: &str) {
        let f = self.fields.entry(id).or_default();
        f.chars = s.chars().take(FIELD_MAX).collect();
        f.caret = f.chars.len();
        f.anchor = f.caret;
        f.scroll = 0.0;
    }

    /// Поле принимает только цифры.
    pub fn set_digits(&mut self, id: u32, on: bool) {
        self.fields.entry(id).or_default().digits = on;
    }

    /// Делает поле полем пароля (точки вместо символов, без Ctrl+C/X).
    pub fn set_secret(&mut self, id: u32, on: bool) {
        self.fields.entry(id).or_default().secret = on;
    }

    /// Выделяет весь текст поля (ввод заменит его).
    pub fn select_all(&mut self, id: u32) {
        let f = self.fields.entry(id).or_default();
        f.anchor = 0;
        f.caret = f.chars.len();
    }

    pub fn has_focus(&self, id: u32) -> bool {
        self.focus == Some(id)
    }

    // --- скролл ---

    /// Начинает область прокрутки; возвращает смещение содержимого (пиксели вниз).
    /// Элементы рисуются с `y - offset`; `scroll_end` закрывает область.
    pub unsafe fn scroll_begin(&mut self, cv: &Canvas, id: u32, view: Rect, content_h: f32) -> f32 {
        let hov = self.interactive && view.contains(self.mouse);
        let wheel = self.wheel;
        let s = self.scrolls.entry(id).or_insert_with(|| Scroll {
            off: 0.0,
            vel: 0.0,
            max: 0.0,
            content: 0.0,
            view,
            vis: Spring::unit(0.0, 300.0, 30.0),
            hot: false,
            grab: 0.0,
        });
        s.max = (content_h - view.h).max(0.0);
        s.content = content_h;
        s.view = view;
        s.off = s.off.clamp(0.0, s.max);

        // Перетаскивание полосы: нажатие в зоне у правого края захватывает мышь раньше
        // содержимого (область рисуется до строк), дальше бегунок следует за курсором.
        let (interactive, mouse, pressed, down) = (self.interactive, self.mouse, self.pressed, self.down);
        let mut dragging = self.drag == Some(id) && down;
        s.hot = dragging;
        if s.max > 0.0 {
            let th = (view.h * view.h / content_h).max(24.0).min(view.h);
            let track = (view.h - th).max(1.0);
            let zone = Rect::new(view.right() - SCROLLBAR_GRAB_W, view.y, SCROLLBAR_GRAB_W, view.h);
            let over = interactive && zone.contains(mouse);
            if over && self.drag.is_none() {
                s.hot = true;
                if pressed {
                    let ty = view.y + track * (s.off / s.max);
                    s.grab = if mouse.1 >= ty && mouse.1 <= ty + th { mouse.1 - ty } else { th / 2.0 };
                    self.drag = Some(id);
                    dragging = true;
                }
            }
            if dragging {
                s.off = ((mouse.1 - s.grab - view.y) / track).clamp(0.0, 1.0) * s.max;
                s.vel = 0.0;
                s.vis.target = 1.0;
            }
        }
        let hov = hov && !dragging;
        if hov && wheel != 0.0 {
            s.vel -= wheel * WHEEL_IMPULSE;
            self.wheel = 0.0;
        }
        let off = s.off;
        cv.g.dc.PushAxisAlignedClip(&view.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
        self.clip = Some(view);
        off
    }

    pub unsafe fn scroll_end(&mut self, cv: &Canvas, id: u32) {
        cv.g.dc.PopAxisAlignedClip();
        self.clip = None;
        let Some(s) = self.scrolls.get(&id) else { return };
        if s.max <= 0.0 || s.vis.value < 0.01 {
            return;
        }
        let th = (s.view.h * s.view.h / s.content).max(24.0).min(s.view.h);
        let ty = s.view.y + (s.view.h - th) * (s.off / s.max);
        let mut c = dim();
        c.a = 0.7 * s.vis.value;
        cv.rr(Rect::new(s.view.right() - 4.0, ty, 3.0, th), 1.5, c);
    }

    // --- текстовое поле ---

    /// Текст, отправленный клавишей Enter из поля `id` (однократно).
    pub fn take_submitted(&mut self, id: u32) -> Option<String> {
        match &self.submitted {
            Some((i, _)) if *i == id => self.submitted.take().map(|(_, t)| t),
            _ => None,
        }
    }

    /// Забирает клавиши навигации; не у интерактивного слоя (уходящая вкладка) их нет.
    pub fn take_nav(&mut self) -> Vec<Nav> {
        if self.interactive {
            std::mem::take(&mut self.nav)
        } else {
            Vec::new()
        }
    }

    /// Колесо мыши над прямоугольником `r` (в координатах слоя): забирает накопленное значение.
    pub fn wheel_in(&mut self, r: Rect) -> f32 {
        if self.interactive && r.contains(self.mouse) && self.wheel != 0.0 {
            std::mem::take(&mut self.wheel)
        } else {
            0.0
        }
    }

    pub fn field_text(&self, id: u32) -> String {
        self.fields.get(&id).map(|f| f.chars.iter().collect()).unwrap_or_default()
    }

    /// Однострочное поле ввода: курсор, выделение мышью и клавишами, прокрутка по caret.
    /// Поле ввода с иконкой и режимом Enter.
    pub unsafe fn text_field_ex(&mut self, cv: &Canvas, id: u32, r: Rect, placeholder: &str, glyph: &str, submit: Submit) {
        self.text_field_pad(cv, id, r, placeholder, glyph, submit, 10.0);
    }

    /// То же, но с заданным отступом справа: текст не заезжает под кнопки поверх поля.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn text_field_pad(&mut self, cv: &Canvas, id: u32, r: Rect, placeholder: &str, glyph: &str, submit: Submit, right: f32) {
        self.text_field_at(cv, id, r, placeholder, glyph, submit, 30.0, right);
    }

    /// Поле с заданными отступами слева (под значок) и справа (под кнопки или единицу измерения).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn text_field_at(&mut self, cv: &Canvas, id: u32, r: Rect, placeholder: &str, glyph: &str, submit: Submit, left: f32, right: f32) {
        self.fields.entry(id).or_default().submit = submit;
        let (hover, _) = self.interact(id, r);
        let inner = Rect::new(r.x + left, r.y, (r.w - left - right).max(1.0), r.h);
        let w = self.widget(id);
        let (hv, ov) = (w.hover.value, w.on.value);
        let focused_now = self.focus == Some(id);
        self.widget(id).on.target = if focused_now { 1.0 } else { 0.0 };

        let mine = self.drag == Some(id);
        let (pressed, down, mouse_x, origin_x) = (self.pressed, self.down, self.mouse.0, self.origin_x);
        if pressed && hover {
            self.focus = Some(id);
            self.focus_claimed = true;
            self.caret_on = true;
        }
        let focused = self.focus == Some(id);
        let caret_on = self.caret_on;

        let f = self.fields.entry(id).or_default();
        // Точка «•» — одна единица UTF-16, поэтому позиции курсора совпадают с символами.
        let shown: String = if f.secret { "•".repeat(f.chars.len()) } else { f.chars.iter().collect() };
        let text: Vec<u16> = shown.encode_utf16().collect();
        let fonts = &cv.g.fonts;
        let layout = fonts.factory.CreateTextLayout(&text, &fonts.body, 100000.0, r.h).ok();

        // Позиция курсора по мышке (нажатие и протяжка).
        if mine && down {
            if let Some(l) = &layout {
                let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
                let mut m = DWRITE_HIT_TEST_METRICS::default();
                let x = mouse_x - inner.x + f.scroll;
                if l.HitTestPoint(x, r.h / 2.0, &mut trailing, &mut inside, &mut m).is_ok() {
                    let p16 = m.textPosition + if trailing.as_bool() { m.length } else { 0 };
                    f.caret = if f.secret { (p16 as usize).min(f.chars.len()) } else { char_at_utf16(&f.chars, p16) };
                    if pressed && !shift_down() {
                        f.anchor = f.caret;
                    }
                }
            }
        }

        // Положение курсора и прокрутка, чтобы он оставался видимым.
        let caret16 = if f.secret { f.caret.min(f.chars.len()) as u32 } else { utf16_len(&f.chars[..f.caret.min(f.chars.len())]) };
        let mut caret_x = 0.0f32;
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        let mut cy = 0.0f32;
        if let Some(l) = &layout {
            let mut cx = 0.0f32;
            if l.HitTestTextPosition(caret16, false, &mut cx, &mut cy, &mut m).is_ok() {
                caret_x = cx;
            }
        }
        if caret_x - f.scroll > inner.w - 2.0 {
            f.scroll = caret_x - inner.w + 2.0;
        }
        if caret_x - f.scroll < 0.0 {
            f.scroll = caret_x;
        }
        if f.scroll < 0.0 || text.is_empty() {
            f.scroll = 0.0;
        }

        // Фон и рамка.
        cv.rr(r, 10.0, mix(card(), fg(), 0.035 * hv));
        cv.glyph(glyph, Rect::new(r.x + 4.0, r.y, 26.0, r.h), r.h >= 27.0, mix(rgb(0.7, 0.7, 0.7), cv.accent, ov));

        // Текст, выделение и курсор — внутри клипа.
        cv.g.dc.PushAxisAlignedClip(&inner.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
        if text.is_empty() {
            cv.text(placeholder, inner, &fonts.body, rgb(0.5, 0.5, 0.5));
        } else if let Some(l) = &layout {
            if f.has_sel() {
                let (lo, hi) = f.range();
                let (mut x0, mut x1) = (0.0f32, 0.0f32);
                let (p0, p1) = if f.secret { (lo as u32, hi as u32) } else { (utf16_len(&f.chars[..lo]), utf16_len(&f.chars[..hi])) };
                if l.HitTestTextPosition(p0, false, &mut x0, &mut cy, &mut m).is_ok()
                    && l.HitTestTextPosition(p1, false, &mut x1, &mut cy, &mut m).is_ok()
                {
                    let mut c = cv.accent;
                    c.a = 0.35;
                    cv.rr(
                        Rect::new(inner.x + x0 - f.scroll, r.cy() - 7.0, (x1 - x0).max(1.0), 14.0),
                        3.0,
                        c,
                    );
                }
            }
            let tr = Rect::new(inner.x - f.scroll, r.y, caret_x.max(inner.w) + 200.0, r.h);
            cv.text(&shown, tr, &fonts.body, txt());
        }
        if focused && caret_on {
            cv.rr(Rect::new(inner.x + caret_x - f.scroll, r.cy() - 6.0, 1.2, 12.0), 0.6, txt());
        }
        cv.g.dc.PopAxisAlignedClip();

        if focused {
            self.ime_pos = Some((origin_x + inner.x + caret_x - f.scroll, r.bottom()));
        }
    }

    /// Клавиатурное событие в сфокусированное поле.
    pub fn key(&mut self, ev: KeyEv) {
        let Some(id) = self.focus else { return };
        let f = self.fields.entry(id).or_default();
        let n = f.chars.len();
        self.caret_on = true;
        match ev {
            KeyEv::Char(c) => {
                let mut b = [0u8; 4];
                f.insert(c.encode_utf8(&mut b));
            }
            KeyEv::Paste(s) => f.insert(&s),
            KeyEv::Left(sh) => {
                if !sh && f.has_sel() {
                    f.caret = f.range().0;
                } else {
                    f.caret = f.caret.saturating_sub(1);
                }
                if !sh {
                    f.anchor = f.caret;
                }
            }
            KeyEv::Right(sh) => {
                if !sh && f.has_sel() {
                    f.caret = f.range().1;
                } else {
                    f.caret = (f.caret + 1).min(n);
                }
                if !sh {
                    f.anchor = f.caret;
                }
            }
            KeyEv::Home(sh) => {
                f.caret = 0;
                if !sh {
                    f.anchor = 0;
                }
            }
            KeyEv::End(sh) => {
                f.caret = n;
                if !sh {
                    f.anchor = n;
                }
            }
            KeyEv::Backspace => {
                if !f.delete_sel() && f.caret > 0 {
                    f.chars.remove(f.caret - 1);
                    f.caret -= 1;
                    f.anchor = f.caret;
                }
            }
            KeyEv::Delete => {
                if n == 0 {
                    self.nav.push(Nav::Delete);
                } else if !f.delete_sel() && f.caret < n {
                    f.chars.remove(f.caret);
                }
            }
            KeyEv::SelectAll => {
                f.anchor = 0;
                f.caret = n;
            }
            KeyEv::Copy => {
                if f.secret {
                } else if f.has_sel() {
                    self.clipboard_out = Some(f.selected());
                } else {
                    self.nav.push(Nav::Copy);
                }
            }
            KeyEv::Cut => {
                if f.has_sel() && !f.secret {
                    self.clipboard_out = Some(f.selected());
                    f.delete_sel();
                }
            }
            KeyEv::Enter => match f.submit {
                Submit::None => self.focus = None,
                Submit::Keep => self.submitted = Some((id, f.chars.iter().collect())),
                Submit::Clear => {
                    self.submitted = Some((id, f.chars.iter().collect()));
                    f.chars.clear();
                    f.caret = 0;
                    f.anchor = 0;
                    f.scroll = 0.0;
                }
            },
            KeyEv::Escape => self.focus = None,
            KeyEv::Up => self.nav.push(Nav::Up),
            KeyEv::Down => self.nav.push(Nav::Down),
            KeyEv::Pin => self.nav.push(Nav::Pin),
            KeyEv::Tab => self.nav.push(Nav::Tab),
        }
    }
}

fn shift_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_edit() {
        let mut f = Field::default();
        f.insert("привет");
        assert_eq!(f.chars.len(), 6);
        f.anchor = 0;
        assert_eq!(f.selected(), "привет");
        f.insert("мир");
        assert_eq!(f.chars.iter().collect::<String>(), "мир");
        assert_eq!(f.caret, 3);
    }

    #[test]
    fn utf16_positions() {
        let chars: Vec<char> = "a😀b".chars().collect();
        assert_eq!(utf16_len(&chars), 4);
        assert_eq!(char_at_utf16(&chars, 3), 2);
        assert_eq!(char_at_utf16(&chars, 4), 3);
    }
}
