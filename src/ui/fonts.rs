//! Шрифты и текстовые форматы DirectWrite, кэш раскладок.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use windows::core::{w, Interface, Result, BOOL};
use windows::Win32::Graphics::DirectWrite::*;

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
pub(super) fn cover_transform(w: f32, h: f32, bw: f32, bh: f32) -> (f32, f32, f32) {
    let s = (w / bw).max(h / bh);
    (s, (w - bw * s) / 2.0, (h - bh * s) / 2.0)
}

pub(super) fn ellipsize_units(s: &str, units: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crops_instead_of_squeezing() {
        // Узкая карточка 40×100 для кадра 16:9 384×216: масштаб по высоте, обрезка по бокам.
        let (s, dx, dy) = cover_transform(40.0, 100.0, 384.0, 216.0);
        assert!((s - 100.0 / 216.0).abs() < 1e-5 && dy.abs() < 1e-4);
        assert!(dx < 0.0 && ((384.0 * s + 2.0 * dx) - 40.0).abs() < 1e-3);
        // Широкая карточка: масштаб по ширине, обрезка сверху и снизу.
        let (s, dx, dy) = cover_transform(384.0, 100.0, 384.0, 216.0);
        assert!((s - 1.0).abs() < 1e-5 && dx.abs() < 1e-4 && dy < 0.0);
    }

    #[test]
    fn ellipsize_cuts_on_utf16_units() {
        assert_eq!(ellipsize_units("абвгдежзик", 8), "абвгд…");
        assert_eq!(ellipsize_units("ab", 10), "…");
        // Символ вне BMP занимает две единицы.
        assert_eq!(ellipsize_units("😀😀😀😀😀", 6), "…");
        assert_eq!(ellipsize_units("слово слово слово", 12), "слово сло…");
    }
}
