//! UI-тулкит этапа 2: иконки, примитивы (карточка, кнопка, тоггл, слайдер, кольцо, прогресс,
//! чип, строка списка), hover/pressed-пружины, скролл с инерцией, текстовое поле, кэш битмапов.
//!
//! Режим «immediate»: вкладка в каждом кадре вызывает виджеты; состояние (пружины наведения,
//! прокрутка, текст поля) живёт в `Ui` и ищется по числовому `id`. Координаты — логические
//! пиксели относительно левого края панели (масштаб DPI применяет трансформация холста).

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};

use windows::core::{w, Interface, Result, BOOL};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows_numerics::{Matrix3x2, Vector2};

use crate::anim::Spring;
use crate::gfx::Gfx;

// ---------- Цвета ----------

const fn rgb(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}

pub const WHITE: D2D1_COLOR_F = rgb(1.0, 1.0, 1.0);
pub const GREEN: D2D1_COLOR_F = rgb(0.61, 0.80, 0.40);
pub const BAR_GREEN: D2D1_COLOR_F = rgb(0.55, 0.76, 0.29);
pub const VIOLET: D2D1_COLOR_F = rgb(0.78, 0.61, 0.91);
pub const ORANGE: D2D1_COLOR_F = rgb(0.95, 0.63, 0.14);
pub const PINK: D2D1_COLOR_F = rgb(0.91, 0.42, 0.60);
pub const CREAM: D2D1_COLOR_F = rgb(0.93, 0.89, 0.78);

// Цвета темы (см. `theme`): меняются на лету, поэтому это функции, а не константы.

/// Основной текст.
pub fn txt() -> D2D1_COLOR_F {
    crate::theme::get().text
}
/// Приглушённый текст и значки.
pub fn dim() -> D2D1_COLOR_F {
    crate::theme::get().dim
}
/// Карточки (нейтральная поверхность чуть светлее фона).
pub fn card() -> D2D1_COLOR_F {
    crate::theme::get().card
}
pub fn track() -> D2D1_COLOR_F {
    crate::theme::get().track
}
/// Надписи и значки на акцентной заливке.
pub fn ink() -> D2D1_COLOR_F {
    crate::theme::get().ink
}
/// Фон мелких плашек (буква приложения, клавиша, бейдж плитки).
pub fn chip_bg() -> D2D1_COLOR_F {
    crate::theme::get().chip
}
/// Поля внутри карточек (ввод часов/минут, блок события).
pub fn inset() -> D2D1_COLOR_F {
    crate::theme::get().inset
}
/// Цвет подсветки при наведении: белый на тёмном фоне, чёрный на светлом.
pub fn fg() -> D2D1_COLOR_F {
    crate::theme::get().fg
}

/// Подсветка выбранной строки: акцент, приглушённый до тёмно-золотого.
pub fn selection(accent: D2D1_COLOR_F) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: 0.27, ..accent }
}

pub fn mix(a: D2D1_COLOR_F, b: D2D1_COLOR_F, t: f32) -> D2D1_COLOR_F {
    let t = t.clamp(0.0, 1.0);
    D2D1_COLOR_F {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

// ---------- Иконки (Segoe Fluent Icons / Segoe MDL2 Assets) ----------

#[allow(dead_code)] // набор глифов для вкладок этапа 3
pub mod icon {
    pub const WIFI: &str = "\u{E701}";
    pub const BLUETOOTH: &str = "\u{E702}";
    pub const BRIGHTNESS: &str = "\u{E706}";
    pub const MOON: &str = "\u{E708}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const LOCK: &str = "\u{E72E}";
    pub const SEARCH: &str = "\u{E721}";
    pub const VOLUME: &str = "\u{E767}";
    pub const MUTE: &str = "\u{E74F}";
    pub const VOLUME1: &str = "\u{E992}";
    pub const VOLUME2: &str = "\u{E993}";
    pub const VOLUME3: &str = "\u{E994}";
    pub const PLAY: &str = "\u{E768}";
    pub const PLAY_SOLID: &str = "\u{F5B0}";
    pub const PAUSE: &str = "\u{E769}";
    pub const POWER: &str = "\u{E7E8}";
    pub const RESTART: &str = "\u{E777}";
    pub const COPY: &str = "\u{E8C8}";
    pub const LINK: &str = "\u{E71B}";
    pub const PICTURE: &str = "\u{E8B9}";
    pub const PIN: &str = "\u{E718}";
    pub const DELETE: &str = "\u{E74D}";
    pub const ADD: &str = "\u{E710}";
    pub const CHECK: &str = "\u{E73E}";
    pub const CLOCK: &str = "\u{E121}";
    pub const CALENDAR: &str = "\u{E787}";
    pub const FOLDER: &str = "\u{E8B7}";
    pub const APPS: &str = "\u{E71D}";
    pub const PREV: &str = "\u{E892}";
    pub const NEXT: &str = "\u{E893}";
    pub const MUSIC: &str = "\u{E8D6}";
    pub const CLOSE: &str = "\u{E711}";
    pub const PASTE: &str = "\u{E77F}";
    pub const LEFT: &str = "\u{E76B}";
    pub const RIGHT: &str = "\u{E76C}";
    pub const STAR: &str = "\u{E734}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const DOWN: &str = "\u{E70D}";
    pub const UP: &str = "\u{E70E}";
    pub const EDIT: &str = "\u{E70F}";
    pub const STAR_FILL: &str = "\u{E735}";
    pub const SETTINGS_GEAR: &str = "\u{E713}";
    pub const RINGER: &str = "\u{EA8F}";
    pub const SPEAKER: &str = "\u{E7F5}";
    pub const CONTRAST: &str = "\u{E793}";
    pub const SUN: &str = "\u{E706}";
    pub const CLOUD: &str = "\u{E753}";
    pub const FORWARD: &str = "\u{E72A}";
    pub const TEXT_DOC: &str = "\u{E8A5}";
}

// ---------- Геометрия ----------

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.right() && p.1 >= self.y && p.1 < self.bottom()
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
}

fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// Матрица «масштаб DPI + сдвиг» для содержимого вкладки.
pub fn matrix(scale: f32, tx: f32, ty: f32) -> Matrix3x2 {
    Matrix3x2 { M11: scale, M12: 0.0, M21: 0.0, M22: scale, M31: tx, M32: ty }
}

/// Масштаб DPI и поворот на `deg` градусов вокруг точки `(cx, cy)` (логические пиксели): шестерёнка.
pub fn matrix_rot(scale: f32, cx: f32, cy: f32, deg: f32) -> Matrix3x2 {
    let (sin, cos) = deg.to_radians().sin_cos();
    Matrix3x2 {
        M11: scale * cos,
        M12: scale * sin,
        M21: -scale * sin,
        M22: scale * cos,
        M31: scale * (cx - cx * cos + cy * sin),
        M32: scale * (cy - cx * sin - cy * cos),
    }
}

// ---------- Шрифты ----------

/// Форматы текста в логических пикселях (холст масштабируется трансформацией). Размеры сняты
/// с макета: подписи 8 px, основной текст 9.5 px, заголовок трека 13 px, цифры часов 23 px.
pub struct Fonts {
    pub factory: IDWriteFactory,
    pub body: IDWriteTextFormat,
    pub body_c: IDWriteTextFormat,
    pub small: IDWriteTextFormat,
    pub small_c: IDWriteTextFormat,
    /// Мелкий жирный по левому краю.
    pub small_b: IDWriteTextFormat,
    pub small_r: IDWriteTextFormat,
    /// Жирный по правому краю (процент громкости).
    pub body_r: IDWriteTextFormat,
    /// Мелкие заголовки секций («ЗАКРЕПЛЕНО», «СЕТЬ»).
    pub caps: IDWriteTextFormat,
    pub caps_c: IDWriteTextFormat,
    /// Маленькие иконки (галочка в кружке задачи).
    pub icon_small: IDWriteTextFormat,
    /// Заголовок трека.
    pub title: IDWriteTextFormat,
    /// Крупные числа: скорость интернета (слева) и температура (по центру).
    pub num: IDWriteTextFormat,
    pub num_c: IDWriteTextFormat,
    /// Цифры часов по центру.
    pub big_c: IDWriteTextFormat,
    /// Жирный абзац по центру с переносом (превью буфера обмена).
    pub wrap_b: IDWriteTextFormat,
    /// Полужирный абзац слева с переносом по словам (текст задач).
    pub wrap_l: IDWriteTextFormat,
    pub icon: IDWriteTextFormat,
    pub icon_big: IDWriteTextFormat,
    /// Готовые раскладки текста: `DrawText` строит её заново при каждом вызове, а в кадре
    /// десятки строк, которые не меняются от кадра к кадру.
    layouts: RefCell<HashMap<u64, (String, IDWriteTextLayout)>>,
}

/// Предел кэша раскладок; при переполнении он сбрасывается целиком (меняющиеся строки —
/// проценты, часы — иначе копились бы бесконечно).
const LAYOUT_CACHE_MAX: usize = 160;

pub unsafe fn font_exists(f: &IDWriteFactory, name: windows::core::PCWSTR) -> bool {
    let mut col: Option<IDWriteFontCollection> = None;
    if f.GetSystemFontCollection(&mut col, false).is_err() {
        return false;
    }
    let Some(col) = col else { return false };
    let (mut idx, mut exists) = (0u32, BOOL(0));
    col.FindFamilyName(name, &mut idx, &mut exists).is_ok() && exists.as_bool()
}

unsafe fn fmt(
    f: &IDWriteFactory,
    family: windows::core::PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
    center: bool,
) -> Result<IDWriteTextFormat> {
    let t = f.CreateTextFormat(
        family,
        None,
        weight,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STRETCH_NORMAL,
        size,
        w!("ru-RU"),
    )?;
    t.SetTextAlignment(if center { DWRITE_TEXT_ALIGNMENT_CENTER } else { DWRITE_TEXT_ALIGNMENT_LEADING })?;
    t.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    t.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    // Не влезающий текст заканчивается многоточием, как в макете.
    let sign = f.CreateEllipsisTrimmingSign(&t)?;
    let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
    t.SetTrimming(&trim, &sign)?;
    Ok(t)
}

/// Обрезает `s` до `units` единиц UTF-16 (границы строк DirectWrite) и ставит «…» вместо
/// последних символов, чтобы многоточие поместилось в ту же строку.
/// Масштаб и сдвиг, чтобы картинка `bw`×`bh` закрыла прямоугольник `w`×`h` целиком
/// (режим «cover»): излишек обрезается поровну с обеих сторон.
fn cover_transform(w: f32, h: f32, bw: f32, bh: f32) -> (f32, f32, f32) {
    let s = (w / bw).max(h / bh);
    (s, (w - bw * s) / 2.0, (h - bh * s) / 2.0)
}

fn ellipsize_units(s: &str, units: usize) -> String {
    let (mut out, mut used) = (String::new(), 0usize);
    for c in s.chars() {
        used += c.len_utf16();
        if used > units {
            break;
        }
        out.push(c);
    }
    for _ in 0..3 {
        out.pop();
    }
    while out.ends_with(char::is_whitespace) {
        out.pop();
    }
    out.push('…');
    out
}

impl Fonts {
    pub unsafe fn new(f: &IDWriteFactory) -> Result<Self> {
        // Segoe UI Variable Text (Windows 11) — оптимизирован для мелкого размера; запасной — Segoe UI.
        let ui = if font_exists(f, w!("Segoe UI Variable Text")) { w!("Segoe UI Variable Text") } else { w!("Segoe UI") };
        let icons = if font_exists(f, w!("Segoe Fluent Icons")) { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") };
        let reg = DWRITE_FONT_WEIGHT_REGULAR;
        let semi = DWRITE_FONT_WEIGHT_SEMI_BOLD;
        let bold = DWRITE_FONT_WEIGHT_BOLD;
        let right = |t: IDWriteTextFormat| -> Result<IDWriteTextFormat> {
            t.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            Ok(t)
        };
        Ok(Fonts {
            factory: f.clone(),
            body: fmt(f, ui, semi, 9.0, false)?,
            body_c: fmt(f, ui, semi, 9.0, true)?,
            small: fmt(f, ui, reg, 8.0, false)?,
            small_c: fmt(f, ui, semi, 8.0, true)?,
            small_b: fmt(f, ui, semi, 8.0, false)?,
            small_r: right(fmt(f, ui, reg, 8.0, false)?)?,
            body_r: right(fmt(f, ui, semi, 8.5, false)?)?,
            caps: fmt(f, ui, semi, 7.0, false)?,
            caps_c: fmt(f, ui, semi, 6.5, true)?,
            icon_small: fmt(f, icons, reg, 7.5, true)?,
            title: fmt(f, ui, bold, 12.5, false)?,
            num: fmt(f, ui, bold, 15.0, false)?,
            num_c: fmt(f, ui, bold, 15.0, true)?,
            big_c: fmt(f, ui, bold, 26.0, true)?,
            wrap_b: {
                let t = fmt(f, ui, semi, 9.0, true)?;
                t.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                t.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
                t
            },
            wrap_l: {
                let t = fmt(f, ui, semi, 8.0, false)?;
                t.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                t.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
                t
            },
            icon: fmt(f, icons, reg, 9.0, true)?,
            icon_big: fmt(f, icons, reg, 12.0, true)?,
            layouts: RefCell::new(HashMap::new()),
        })
    }

    /// Раскладка строки `s` в рамке `w`×`h` по формату `f` (из кэша, если такая уже была).
    /// `None` — рамка пустая или DirectWrite отказал; вызывающий рисует обычным `DrawText`.
    pub unsafe fn layout(&self, s: &str, f: &IDWriteTextFormat, w: f32, h: f32) -> Option<IDWriteTextLayout> {
        if !(w > 0.0 && h > 0.0) {
            return None;
        }
        let s = &*crate::i18n::tr(s);
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        (f.as_raw() as usize).hash(&mut hasher);
        w.to_bits().hash(&mut hasher);
        h.to_bits().hash(&mut hasher);
        let key = hasher.finish();
        let mut map = self.layouts.borrow_mut();
        if let Some((text, layout)) = map.get(&key) {
            if text == s {
                return Some(layout.clone());
            }
        }
        if map.len() >= LAYOUT_CACHE_MAX {
            map.clear();
        }
        let layout = crate::sys::with_utf16(s, |wide| self.factory.CreateTextLayout(wide, f, w, h)).ok()?;
        map.insert(key, (s.to_string(), layout.clone()));
        Some(layout)
    }

    /// Нужно после пересоздания форматов (смена DPI): ключ содержит адрес формата.
    pub fn clear_layouts(&self) {
        self.layouts.borrow_mut().clear();
    }
}

// ---------- Холст ----------

/// Рисование одного слоя содержимого: общая прозрачность и акцентный цвет.
pub struct Canvas<'a> {
    pub g: &'a Gfx,
    pub alpha: f32,
    pub accent: D2D1_COLOR_F,
}

impl Canvas<'_> {
    fn col(&self, mut c: D2D1_COLOR_F) -> D2D1_COLOR_F {
        c.a *= self.alpha;
        c
    }

    pub unsafe fn rr(&self, r: Rect, radius: f32, c: D2D1_COLOR_F) {
        self.g.brush.SetColor(&self.col(c));
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        self.g.dc.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad },
            &self.g.brush,
        );
    }

    pub unsafe fn rr_line(&self, r: Rect, radius: f32, c: D2D1_COLOR_F, width: f32) {
        self.g.brush.SetColor(&self.col(c));
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        self.g.dc.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT { rect: r.inset(width / 2.0).d2d(), radiusX: rad, radiusY: rad },
            &self.g.brush,
            width,
            None,
        );
    }

    pub unsafe fn circle(&self, cx: f32, cy: f32, radius: f32, c: D2D1_COLOR_F) {
        self.g.brush.SetColor(&self.col(c));
        self.g.dc.FillEllipse(
            &D2D1_ELLIPSE { point: pt(cx, cy), radiusX: radius, radiusY: radius },
            &self.g.brush,
        );
    }

    pub unsafe fn text(&self, s: &str, r: Rect, f: &IDWriteTextFormat, c: D2D1_COLOR_F) {
        if s.is_empty() {
            return;
        }
        self.g.brush.SetColor(&self.col(c));
        if let Some(layout) = self.g.fonts.layout(s, f, r.w, r.h) {
            self.g.dc.DrawTextLayout(pt(r.x, r.y), &layout, &self.g.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            return;
        }
        crate::sys::with_utf16(s, |wide| {
            self.g.dc.DrawText(
                wide,
                f,
                &r.d2d(),
                &self.g.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        });
    }

    /// Жирный абзац по центру с переносом (лишнее обрезается).
    pub unsafe fn text_wrap_c(&self, s: &str, r: Rect, c: D2D1_COLOR_F) {
        self.text(s, r, &self.g.fonts.wrap_b, c);
    }

    /// Скруглённый прямоугольник с вертикальным градиентом (обложка трека в плеере).
    pub unsafe fn gradient_rr(&self, r: Rect, radius: f32, top: D2D1_COLOR_F, bottom: D2D1_COLOR_F) {
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color: self.col(top) },
            D2D1_GRADIENT_STOP { position: 1.0, color: self.col(bottom) },
        ];
        let Ok(coll) = ID2D1RenderTarget::CreateGradientStopCollection(&self.g.dc, &stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(r.x, r.y), endPoint: pt(r.right(), r.bottom()) };
        let Ok(br) = self.g.dc.CreateLinearGradientBrush(&props, None, &coll) else { return };
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        self.g.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad }, &br);
    }

    /// Скруглённый прямоугольник с линейным градиентом из нескольких остановок (палитра цвета):
    /// `horizontal` — слева направо, иначе сверху вниз. Цвета могут быть полупрозрачными.
    pub unsafe fn gradient_stops(&self, r: Rect, radius: f32, stops: &[(f32, D2D1_COLOR_F)], horizontal: bool) {
        let list: Vec<D2D1_GRADIENT_STOP> =
            stops.iter().map(|(p, c)| D2D1_GRADIENT_STOP { position: *p, color: self.col(*c) }).collect();
        let Ok(coll) = ID2D1RenderTarget::CreateGradientStopCollection(&self.g.dc, &list, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
        let (a, b) = if horizontal { (pt(r.x, r.y), pt(r.right(), r.y)) } else { (pt(r.x, r.y), pt(r.x, r.bottom())) };
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: a, endPoint: b };
        let Ok(br) = self.g.dc.CreateLinearGradientBrush(&props, None, &coll) else { return };
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        self.g.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad }, &br);
    }

    /// Ширина строки в логических пикселях по формату `f` (раскладка из кэша); при отказе DirectWrite —
    /// грубая оценка по числу символов.
    /// Сколько строк займёт `s` при переносе по словам в ширину `w` и какова высота строки.
    pub unsafe fn wrapped(&self, s: &str, f: &IDWriteTextFormat, w: f32) -> (usize, f32) {
        if let Some(l) = self.g.fonts.layout(s, f, w, 10_000.0) {
            let mut m = DWRITE_TEXT_METRICS::default();
            if l.GetMetrics(&mut m).is_ok() && m.lineCount > 0 {
                return (m.lineCount as usize, m.height / m.lineCount as f32);
            }
        }
        (1, 11.0)
    }

    /// Текст, не длиннее `max_lines` строк при переносе в ширину `w`: лишнее заменяется «…».
    pub unsafe fn wrap_clip<'s>(&self, s: &'s str, f: &IDWriteTextFormat, w: f32, max_lines: usize) -> std::borrow::Cow<'s, str> {
        let Some(l) = self.g.fonts.layout(s, f, w, 10_000.0) else { return s.into() };
        let mut m = DWRITE_TEXT_METRICS::default();
        if l.GetMetrics(&mut m).is_err() || (m.lineCount as usize) <= max_lines {
            return s.into();
        }
        let mut lines = vec![DWRITE_LINE_METRICS::default(); m.lineCount as usize];
        let mut n = 0u32;
        if l.GetLineMetrics(Some(&mut lines), &mut n).is_err() {
            return s.into();
        }
        let units: u32 = lines.iter().take(max_lines).map(|x| x.length).sum();
        ellipsize_units(s, units as usize).into()
    }

    /// Как `text`, но с переключаемым зачёркиванием (для выполненных задач).
    pub unsafe fn text_strike(&self, s: &str, r: Rect, f: &IDWriteTextFormat, c: D2D1_COLOR_F, strike: bool) {
        if s.is_empty() {
            return;
        }
        self.g.brush.SetColor(&self.col(c));
        if let Some(layout) = self.g.fonts.layout(s, f, r.w, r.h) {
            // Раскладка общая из кэша: признак ставится явно при каждой отрисовке.
            let range = DWRITE_TEXT_RANGE { startPosition: 0, length: s.encode_utf16().count() as u32 };
            let _ = layout.SetStrikethrough(strike, range);
            self.g.dc.DrawTextLayout(pt(r.x, r.y), &layout, &self.g.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
    }

    pub unsafe fn text_w(&self, s: &str, f: &IDWriteTextFormat) -> f32 {
        if let Some(l) = self.g.fonts.layout(s, f, 10_000.0, 100.0) {
            let mut m = DWRITE_TEXT_METRICS::default();
            if l.GetMetrics(&mut m).is_ok() {
                return m.widthIncludingTrailingWhitespace.ceil();
            }
        }
        s.chars().count() as f32 * 4.6
    }

    /// Плашка клавиши («Ctrl P») и подпись после неё. Возвращает общую ширину.
    pub unsafe fn keycap(&self, x: f32, cy: f32, key: &str, label: &str) -> f32 {
        let kw = self.text_w(key, &self.g.fonts.small_c) + 9.0;
        let k = Rect::new(x, cy - 7.0, kw, 14.0);
        self.rr(k, 3.5, chip_bg());
        self.text(key, k, &self.g.fonts.small_c, txt());
        let lw = self.text_w(label, &self.g.fonts.small) + 4.0;
        self.text(label, Rect::new(k.right() + 4.0, cy - 7.0, lw + 10.0, 14.0), &self.g.fonts.small, dim());
        kw + 4.0 + lw
    }

    /// Картинка без обрезки в прямоугольнике `r` (иконки приложений).
    pub unsafe fn bitmap(&self, bmp: &ID2D1Bitmap1, r: Rect, opacity: f32) {
        self.g.dc.DrawBitmap(
            bmp,
            Some(&r.d2d()),
            opacity * self.alpha,
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            None,
            None,
        );
    }

    /// Картинка в параллелограмме со скруглёнными углами: верхняя грань сдвинута вправо на
    /// `slant` (карусель обоев).
    pub unsafe fn bitmap_quad(&self, bmp: Option<&ID2D1Bitmap1>, r: Rect, slant: f32, opacity: f32) {
        let Ok(geo) = self.g.d2d.CreatePathGeometry() else { return };
        let Ok(sink) = geo.Open() else { return };
        let corners = [
            (r.x + slant, r.y),
            (r.right() + slant, r.y),
            (r.right(), r.bottom()),
            (r.x, r.bottom()),
        ];
        let rad = 8.0f32.min(r.w / 3.0).min(r.h / 3.0);
        // Точка на отрезке a→b на расстоянии `d` от a.
        let toward = |a: (f32, f32), b: (f32, f32), d: f32| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            pt(a.0 + dx / len * d, a.1 + dy / len * d)
        };
        for k in 0..4 {
            let c = corners[k];
            let prev = corners[(k + 3) % 4];
            let next = corners[(k + 1) % 4];
            let a = toward(c, prev, rad);
            let b = toward(c, next, rad);
            if k == 0 {
                sink.BeginFigure(a, D2D1_FIGURE_BEGIN_FILLED);
            } else {
                sink.AddLine(a);
            }
            sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: pt(c.0, c.1), point2: b });
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        if sink.Close().is_err() {
            return;
        }
        let size = bmp.map(|b| b.GetPixelSize());
        match (bmp, size) {
            (Some(b), Some(sz)) if sz.width > 0 && sz.height > 0 => {
                let Ok(br) = self.g.dc.CreateBitmapBrush(b, None, None) else { return };
                // Кубическая фильтрация: линейная при сильном уменьшении (узкие карточки карусели) рябит.
                br.SetInterpolationMode1(D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC);
                br.SetOpacity(opacity * self.alpha);
                // Картинка не сжимается по одной оси: масштаб «cover», лишнее по краям обрезается.
                let (s, dx, dy) = cover_transform(r.w + slant, r.h, sz.width as f32, sz.height as f32);
                br.SetTransform(&Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: r.x + dx, M32: r.y + dy });
                self.g.dc.FillGeometry(&geo, &br, None);
            }
            _ => {
                let mut c = card();
                c.a *= opacity * self.alpha;
                self.g.brush.SetColor(&c);
                self.g.dc.FillGeometry(&geo, &self.g.brush, None);
            }
        }
    }

    /// Иконка-глиф по центру прямоугольника.
    pub unsafe fn glyph(&self, g: &str, r: Rect, big: bool, c: D2D1_COLOR_F) {
        let f = if big { &self.g.fonts.icon_big } else { &self.g.fonts.icon };
        self.text(g, r, f, c);
    }

    /// Дуга-кольцо: `frac` 0..1 по часовой стрелке от верхней точки, поверх серой дорожки.
    pub unsafe fn ring(&self, cx: f32, cy: f32, radius: f32, thick: f32, frac: f32, c: D2D1_COLOR_F) {
        let frac = frac.clamp(0.0, 1.0);
        let ell = D2D1_ELLIPSE { point: pt(cx, cy), radiusX: radius, radiusY: radius };
        self.g.brush.SetColor(&self.col(track()));
        self.g.dc.DrawEllipse(&ell, &self.g.brush, thick, None);
        if frac < 0.004 {
            return;
        }
        self.g.brush.SetColor(&self.col(c));
        if frac > 0.996 {
            self.g.dc.DrawEllipse(&ell, &self.g.brush, thick, None);
            return;
        }
        let a = frac * std::f32::consts::TAU;
        let end = pt(cx + radius * a.sin(), cy - radius * a.cos());
        let Ok(geo) = self.g.d2d.CreatePathGeometry() else { return };
        let Ok(sink) = geo.Open() else { return };
        sink.BeginFigure(pt(cx, cy - radius), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: end,
            size: D2D_SIZE_F { width: radius, height: radius },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: if frac > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
        });
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        if sink.Close().is_ok() {
            self.g.dc.DrawGeometry(&geo, &self.g.brush, thick, &self.g.round);
        }
    }

    pub unsafe fn card(&self, r: Rect) {
        self.rr(r, 11.0, card());
    }

    /// Обводка дуги окружности: углы в градусах от верхней точки по часовой стрелке
    /// (как у `ring`), концы круглые. Дуга короче 360°.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn stroke_arc(&self, cx: f32, cy: f32, radius: f32, from_deg: f32, to_deg: f32, thick: f32, c: D2D1_COLOR_F) {
        let at = |deg: f32| {
            let a = deg.to_radians();
            pt(cx + radius * a.sin(), cy - radius * a.cos())
        };
        let Ok(geo) = self.g.d2d.CreatePathGeometry() else { return };
        let Ok(sink) = geo.Open() else { return };
        sink.BeginFigure(at(from_deg), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: at(to_deg),
            size: D2D_SIZE_F { width: radius, height: radius },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: if to_deg - from_deg > 180.0 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
        });
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        if sink.Close().is_ok() {
            self.g.brush.SetColor(&self.col(c));
            self.g.dc.DrawGeometry(&geo, &self.g.brush, thick, &self.g.round);
        }
    }

    /// Линия с круглыми концами.
    pub unsafe fn stroke_line(&self, a: (f32, f32), b: (f32, f32), thick: f32, c: D2D1_COLOR_F) {
        self.g.brush.SetColor(&self.col(c));
        self.g.dc.DrawLine(pt(a.0, a.1), pt(b.0, b.1), &self.g.brush, thick, &self.g.round);
    }

    pub unsafe fn progress(&self, r: Rect, frac: f32, c: D2D1_COLOR_F) {
        self.rr(r, r.h / 2.0, track());
        let fw = (r.w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { r.h } else { 0.0 });
        if fw > 0.0 {
            self.rr(Rect::new(r.x, r.y, fw, r.h), r.h / 2.0, c);
        }
    }
}

// ---------- Кэш битмапов ----------

const CACHE_CAP: usize = 128;

/// Кэш `ID2D1Bitmap1` для тяжёлых элементов (обложки, иконки, миниатюры). Привязан к устройству:
/// при потере графики очищается вместе с `Gfx`. Вытесняет самый давно использованный.
#[derive(Default)]
pub struct BitmapCache {
    items: Vec<(u64, ID2D1Bitmap1, u64)>,
    tick: u64,
}

impl BitmapCache {
    /// Возвращает битмап по ключу; если его нет — `make` отдаёт пиксели BGRA (premultiplied, w×h).
    pub unsafe fn get(
        &mut self,
        dc: &ID2D1DeviceContext,
        key: u64,
        w: u32,
        h: u32,
        make: impl FnOnce() -> Vec<u8>,
    ) -> Option<ID2D1Bitmap1> {
        self.tick += 1;
        if let Some(it) = self.items.iter_mut().find(|i| i.0 == key) {
            it.2 = self.tick;
            return Some(it.1.clone());
        }
        let pixels = make();
        let need = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4));
        if need.is_none_or(|n| w == 0 || h == 0 || pixels.len() < n) {
            return None;
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        let bmp = dc
            .CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(pixels.as_ptr() as *const c_void), w * 4, &props)
            .ok()?;
        if self.items.len() >= CACHE_CAP {
            if let Some(oldest) = self.items.iter().enumerate().min_by_key(|(_, i)| i.2).map(|(n, _)| n) {
                self.items.swap_remove(oldest);
            }
        }
        self.items.push((key, bmp.clone(), self.tick));
        Some(bmp)
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

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
}

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
        for c in s.chars().filter(|c| !c.is_control()) {
            if self.chars.len() >= FIELD_MAX {
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
            s.vis.target = if s.vel != 0.0 { 1.0 } else { 0.0 };
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

    /// Делает поле полем пароля (точки вместо символов, без Ctrl+C/X).
    pub fn set_secret(&mut self, id: u32, on: bool) {
        self.fields.entry(id).or_default().secret = on;
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
        });
        s.max = (content_h - view.h).max(0.0);
        s.content = content_h;
        s.view = view;
        s.off = s.off.clamp(0.0, s.max);
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
        self.fields.entry(id).or_default().submit = submit;
        let (hover, _) = self.interact(id, r);
        let inner = Rect::new(r.x + 30.0, r.y, (r.w - 30.0 - right).max(1.0), r.h);
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
    #[test]
    fn cover_crops_instead_of_squeezing() {
        // Узкая карточка 40×100 для кадра 16:9 384×216: масштаб по высоте, обрезка по бокам.
        let (s, dx, dy) = super::cover_transform(40.0, 100.0, 384.0, 216.0);
        assert!((s - 100.0 / 216.0).abs() < 1e-5 && dy.abs() < 1e-4);
        assert!(dx < 0.0 && ((384.0 * s + 2.0 * dx) - 40.0).abs() < 1e-3);
        // Широкая карточка: масштаб по ширине, обрезка сверху и снизу.
        let (s, dx, dy) = super::cover_transform(384.0, 100.0, 384.0, 216.0);
        assert!((s - 1.0).abs() < 1e-5 && dx.abs() < 1e-4 && dy < 0.0);
    }

    use super::*;

    #[test]
    fn ellipsize_cuts_on_utf16_units() {
        assert_eq!(ellipsize_units("абвгдежзик", 8), "абвгд…");
        assert_eq!(ellipsize_units("ab", 10), "…");
        // Символ вне BMP занимает две единицы.
        assert_eq!(ellipsize_units("😀😀😀😀😀", 6), "…");
        assert_eq!(ellipsize_units("слово слово слово", 12), "слово сло…");
    }

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

    #[test]
    fn rect_contains() {
        let r = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert!(r.contains((10.0, 10.0)));
        assert!(!r.contains((30.0, 15.0)));
    }

    #[test]
    fn mix_endpoints() {
        let c = mix(track(), fg(), 1.0);
        assert!((c.r - 1.0).abs() < 1e-6);
    }
}
