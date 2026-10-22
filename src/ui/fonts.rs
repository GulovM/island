//! Шрифты и текстовые форматы DirectWrite, кэш раскладок.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use windows::core::{w, Interface, Result, BOOL};
use windows::Win32::Graphics::DirectWrite::*;

use crate::ios::{self, TextRole};

/// Вес Semibold (590) согласно спецификации iOS 26 для ролей Headline и Emphasized (запасной путь
/// без вариативного шрифта: DirectWrite берёт ближайшее начертание, Semibold 600).
#[cfg(test)]
pub const WEIGHT_EMPHASIZED: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(590);

/// Веса ролей по оси `wght` Segoe UI Variable. Иерархия держится весом, как в SF Pro: основной
/// текст средний (не полужирный — иначе всё одинаково «кричит»), заголовки и кнопки полужирные,
/// вторичный текст чуть плотнее Regular, чтобы не таял на размытом стекле.
pub mod weight {
    /// Основной текст: строки списков, подписи настроек, поля ввода.
    pub const BODY: u32 = 540;
    /// Акцентный текст: заголовки плиток, кнопки, подписи по центру.
    pub const EMPHASIZED: u32 = 600;
    /// Вторичный текст: подзаголовки, время, пояснения.
    pub const SECONDARY: u32 = 430;
    /// Мелкие заголовки секций прописными.
    pub const CAPS: u32 = 660;
    /// Абзац задач и превью.
    pub const PARAGRAPH: u32 = 500;
    /// Заголовок трека и крупные числа.
    pub const TITLE: u32 = 690;
    /// Цифры часов: крупный кегль Display легче — как часы экрана блокировки iOS.
    pub const CLOCK: u32 = 620;
}

/// Таблица соответствия форматов острова шкале типографики Apple iOS 26 (в масштабе острова 9/17).
/// Вес — значение оси `wght` (на Windows 10 без вариативного шрифта — ближайшее начертание).
/// Трекинг (px) в кегле острова; «tnum» — табличные цифры, чтобы меняющиеся числа не дрожали.
///
/// | Имя формата | Роль iOS              | Размер (px) | Вес | Трекинг     | Примечание                          |
/// |-------------|-----------------------|-------------|-----|-------------|-------------------------------------|
/// | body        | Body                  | 9.0         | 540 | Body        | Основной текст                      |
/// | body_c      | Headline              | 9.0         | 600 | Headline    | Основной по центру, кнопки          |
/// | small       | Subheadline           | 8.0         | 430 | Subheadline | Вторичный текст                     |
/// | small_c     | Subheadline (emp)     | 8.0         | 600 | Subheadline | Чипы и кнопки                       |
/// | small_b     | Subheadline (emp)     | 8.0         | 600 | Subheadline | Мелкий жирный слева                 |
/// | small_r     | Subheadline           | 8.0         | 430 | Subheadline | Вторичный справа, tnum              |
/// | body_r      | Callout (emp)         | 8.5         | 600 | Callout     | Процент громкости, tnum             |
/// | caps        | Footnote (emp)        | 7.0         | 660 | +0.04 em    | Заголовки секций прописными         |
/// | caps_c      | Caption1 (emp)        | 6.5         | 600 | +0.02 em    | Мелкие подписи по центру, tnum      |
/// | num         | Title1 (emp)          | 15.0        | 690 | 0           | Крупные числа слева, tnum           |
/// | num_c       | Title1 (emp)          | 15.0        | 690 | 0           | Крупные числа по центру, tnum       |
/// | wrap_b      | Body                  | 9.0         | 540 | Body        | Абзац по центру с переносом         |
/// | wrap_l      | Subheadline           | 8.0         | 500 | Subheadline | Абзац слева с переносом (задачи)    |
/// | tab         | Callout (emp)         | 8.5         | 600 | Callout     | Метки вкладок (gfx::make_text)      |
/// | title       | Title2 (emp)          | 12.5        | 690 | Title2      | Заголовок трека и метрик            |
/// | title_c     | Title2 (emp)          | 12.5        | 690 | Title2      | Время таймера по центру, tnum       |
/// | big_c       | Display               | 26.0        | 620 | −0.012 em   | Цифры часов, tnum                   |
/// | icon_*      | Не текст              | 7.5–12      | 400 | 0           | Запасные глифы Segoe Fluent         |
#[cfg(test)]
pub const FORMAT_ROLE_MAP: &[(&str, Option<TextRole>, f32, u32)] = &[
    ("body", Some(TextRole::Body), 9.0, weight::BODY),
    ("body_c", Some(TextRole::Headline), 9.0, weight::EMPHASIZED),
    ("small", Some(TextRole::Subheadline), 8.0, weight::SECONDARY),
    ("small_c", Some(TextRole::Subheadline), 8.0, weight::EMPHASIZED),
    ("small_b", Some(TextRole::Subheadline), 8.0, weight::EMPHASIZED),
    ("small_r", Some(TextRole::Subheadline), 8.0, weight::SECONDARY),
    ("body_r", Some(TextRole::Callout), 8.5, weight::EMPHASIZED),
    ("caps", Some(TextRole::Footnote), 7.0, weight::CAPS),
    ("caps_c", Some(TextRole::Caption1), 6.5, weight::EMPHASIZED),
    ("num", Some(TextRole::Title1), 15.0, weight::TITLE),
    ("num_c", Some(TextRole::Title1), 15.0, weight::TITLE),
    ("wrap_b", Some(TextRole::Body), 9.0, weight::BODY),
    ("wrap_l", Some(TextRole::Subheadline), 8.0, weight::PARAGRAPH),
    ("tab", Some(TextRole::Callout), 8.5, weight::EMPHASIZED),
    ("title", None, 12.5, weight::TITLE),
    ("title_c", None, 12.5, weight::TITLE),
    ("big_c", None, 26.0, weight::CLOCK),
    ("icon_small", None, 7.5, 400),
    ("icon", None, 9.0, 400),
    ("icon_big", None, 12.0, 400),
];

/// Выравнивание строки в формате `Fonts::display`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Leading,
    Center,
    Trailing,
}

/// Накладывает добавочный трекинг (letter-spacing) на текстовую раскладку DirectWrite.
/// Для сужения текста передаётся отрицательный `trailingSpacing`.
pub unsafe fn apply_tracking(layout: &IDWriteTextLayout, tracking: f32, len: u32) {
    if tracking != 0.0 && len > 0 {
        if let Ok(layout1) = layout.cast::<IDWriteTextLayout1>() {
            let _ = layout1.SetCharacterSpacing(
                0.0,
                tracking,
                0.0,
                DWRITE_TEXT_RANGE { startPosition: 0, length: len },
            );
        }
    }
}
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
    /// Заголовочный кегль по центру с табличными цифрами (время в кольце таймера).
    pub title_c: IDWriteTextFormat,
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
    /// Добавочный трекинг (letter-spacing) в пикселях по указателю формата (`as_raw() as usize`).
    tracking: RefCell<HashMap<usize, f32>>,
    /// Форматы с табличными цифрами (`tnum`) по указателю.
    tabular: RefCell<std::collections::HashSet<usize>>,
    /// Семейство крупных цифр (Display) и сделанные по месту форматы `display` по (кегль×4, вес, выравнивание).
    display_family: windows::core::PCWSTR,
    sized: RefCell<HashMap<(u32, u32, u8), IDWriteTextFormat>>,
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

/// Оптический размер (ось `opsz` Segoe UI Variable, в пунктах) для кегля в логических пикселях:
/// холст масштабируется DPI и масштабом интерфейса (~1,5 при 125 %), поэтому реальный кегль
/// крупнее логического. Мелкие подписи получают начертание Small (шире, крупнее очко),
/// цифры часов — Display (плотнее), как оптические размеры SF Pro Text/Display.
pub fn optical_size(size: f32) -> f32 {
    (size * 1.15).clamp(8.0, 36.0)
}

/// Формат на вариативном Segoe UI Variable: точный вес по оси `wght` (не ближайшее именованное
/// начертание) и оптический размер по кеглю. Без вариативного шрифта (Windows 10) — `fmt` с
/// семейством `legacy` и ближайшим весом.
pub(crate) unsafe fn fmt_var(
    f: &IDWriteFactory,
    legacy: windows::core::PCWSTR,
    weight: u32,
    size: f32,
    center: bool,
) -> Result<IDWriteTextFormat> {
    if font_exists(f, w!("Segoe UI Variable Text")) {
        if let Ok(f6) = f.cast::<IDWriteFactory6>() {
            let axes = [
                DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT, value: weight as f32 },
                DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_OPTICAL_SIZE, value: optical_size(size) },
            ];
            if let Ok(t) = f6.CreateTextFormat(w!("Segoe UI Variable"), None, &axes, size, w!("ru-RU")) {
                let t: IDWriteTextFormat = t.cast()?;
                setup(f, &t, center)?;
                return Ok(t);
            }
        }
    }
    fmt(f, legacy, DWRITE_FONT_WEIGHT(weight as i32), size, center)
}

/// Вес формата: ось `wght` у вариативного, иначе `GetFontWeight`.
#[cfg(test)]
pub unsafe fn weight_of(t: &IDWriteTextFormat) -> u32 {
    if let Ok(t3) = t.cast::<IDWriteTextFormat3>() {
        let n = t3.GetFontAxisValueCount() as usize;
        if n > 0 {
            let mut v = vec![DWRITE_FONT_AXIS_VALUE::default(); n];
            if t3.GetFontAxisValues(&mut v).is_ok() {
                if let Some(a) = v.iter().find(|a| a.axisTag == DWRITE_FONT_AXIS_TAG_WEIGHT) {
                    return a.value.round() as u32;
                }
            }
        }
    }
    t.GetFontWeight().0 as u32
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
    setup(f, &t, center)?;
    Ok(t)
}

unsafe fn setup(f: &IDWriteFactory, t: &IDWriteTextFormat, center: bool) -> Result<()> {
    t.SetTextAlignment(if center { DWRITE_TEXT_ALIGNMENT_CENTER } else { DWRITE_TEXT_ALIGNMENT_LEADING })?;
    t.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    t.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    // Не влезающий текст заканчивается многоточием, как в макете.
    let sign = f.CreateEllipsisTrimmingSign(t)?;
    let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
    t.SetTrimming(&trim, &sign)?;
    Ok(())
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
        // Крупные цифры и заголовки — оптическое начертание Display (плотнее на крупном кегле, как SF Pro Display).
        let display = if font_exists(f, w!("Segoe UI Variable Display")) { w!("Segoe UI Variable Display") } else { ui };
        let icons = if font_exists(f, w!("Segoe Fluent Icons")) { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") };
        let reg = DWRITE_FONT_WEIGHT_REGULAR;
        let right = |t: IDWriteTextFormat| -> Result<IDWriteTextFormat> {
            t.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            Ok(t)
        };
        let wrap = |t: IDWriteTextFormat| -> Result<IDWriteTextFormat> {
            t.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            t.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            Ok(t)
        };
        use weight as wt;

        let body = fmt_var(f, ui, wt::BODY, 9.0, false)?;
        let body_c = fmt_var(f, ui, wt::EMPHASIZED, 9.0, true)?;
        let small = fmt_var(f, ui, wt::SECONDARY, 8.0, false)?;
        let small_c = fmt_var(f, ui, wt::EMPHASIZED, 8.0, true)?;
        let small_b = fmt_var(f, ui, wt::EMPHASIZED, 8.0, false)?;
        let small_r = right(fmt_var(f, ui, wt::SECONDARY, 8.0, false)?)?;
        let body_r = right(fmt_var(f, ui, wt::EMPHASIZED, 8.5, false)?)?;
        let caps = fmt_var(f, ui, wt::CAPS, 7.0, false)?;
        let caps_c = fmt_var(f, ui, wt::EMPHASIZED, 6.5, true)?;
        let icon_small = fmt(f, icons, reg, 7.5, true)?;
        let title = fmt_var(f, display, wt::TITLE, 12.5, false)?;
        let title_c = fmt_var(f, display, wt::TITLE, 12.5, true)?;
        let num = fmt_var(f, display, wt::TITLE, 15.0, false)?;
        let num_c = fmt_var(f, display, wt::TITLE, 15.0, true)?;
        let big_c = fmt_var(f, display, wt::CLOCK, 26.0, true)?;
        let wrap_b = wrap(fmt_var(f, ui, wt::BODY, 9.0, true)?)?;
        let wrap_l = wrap(fmt_var(f, ui, wt::PARAGRAPH, 8.0, false)?)?;
        let icon = fmt(f, icons, reg, 9.0, true)?;
        let icon_big = fmt(f, icons, reg, 12.0, true)?;

        let mut tracking = HashMap::new();
        let mut reg_track = |fmt: &IDWriteTextFormat, track: f32| {
            if track != 0.0 {
                tracking.insert(fmt.as_raw() as usize, track);
            }
        };

        reg_track(&body, ios::tracking_px(TextRole::Body, 9.0));
        reg_track(&body_c, ios::tracking_px(TextRole::Headline, 9.0));
        reg_track(&small, ios::tracking_px(TextRole::Subheadline, 8.0));
        reg_track(&small_c, ios::tracking_px(TextRole::Subheadline, 8.0));
        reg_track(&small_b, ios::tracking_px(TextRole::Subheadline, 8.0));
        reg_track(&small_r, ios::tracking_px(TextRole::Subheadline, 8.0));
        reg_track(&body_r, ios::tracking_px(TextRole::Callout, 8.5));
        // Прописные читаются только с разрядкой: у мелких заголовков секций трекинг положительный.
        reg_track(&caps, 0.04 * 7.0);
        reg_track(&caps_c, 0.02 * 6.5);
        reg_track(&title, ios::tracking_px(TextRole::Title2, 12.5));
        reg_track(&title_c, ios::tracking_px(TextRole::Title2, 12.5));
        reg_track(&big_c, -0.012 * 26.0);
        reg_track(&wrap_b, ios::tracking_px(TextRole::Body, 9.0));
        reg_track(&wrap_l, ios::tracking_px(TextRole::Subheadline, 8.0));

        // Табличные цифры там, где числа меняются на глазах: проценты, время, скорость, часы.
        let tabular = RefCell::new([&small_r, &body_r, &caps_c, &title_c, &num, &num_c, &big_c].iter().map(|t| t.as_raw() as usize).collect());

        Ok(Fonts {
            factory: f.clone(),
            body,
            body_c,
            small,
            small_c,
            small_b,
            small_r,
            body_r,
            caps,
            caps_c,
            icon_small,
            title,
            title_c,
            num,
            num_c,
            big_c,
            wrap_b,
            wrap_l,
            icon,
            icon_big,
            tracking: RefCell::new(tracking),
            tabular,
            display_family: display,
            sized: RefCell::new(HashMap::new()),
            layouts: RefCell::new(HashMap::new()),
        })
    }

    /// Крупные цифры виджетов (часы, температура, перекидные плитки): Display нужного кегля и веса
    /// с табличными цифрами и сужающим трекингом. Формат создаётся один раз и берётся из кэша;
    /// если DirectWrite отказал — запасной `big_c`.
    pub unsafe fn display(&self, size: f32, weight: u32, align: Align) -> IDWriteTextFormat {
        let key = ((size * 4.0).round() as u32, weight, align as u8);
        if let Some(t) = self.sized.borrow().get(&key) {
            return t.clone();
        }
        let made = fmt_var(&self.factory, self.display_family, weight, size, align == Align::Center).and_then(|t| {
            if align == Align::Trailing {
                t.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            }
            Ok(t)
        });
        let Ok(t) = made else { return self.big_c.clone() };
        // Чем крупнее цифры, тем плотнее их ставят (SF Pro Display): −1,5 % кегля.
        self.register_tracking(&t, -0.015 * size);
        self.tabular.borrow_mut().insert(t.as_raw() as usize);
        self.sized.borrow_mut().insert(key, t.clone());
        t
    }

    /// Регистрирует добавочный трекинг в пикселях для внешнего формата (например, подписей вкладок).
    pub fn register_tracking(&self, f: &IDWriteTextFormat, tracking: f32) {
        if tracking != 0.0 {
            self.tracking.borrow_mut().insert(f.as_raw() as usize, tracking);
        }
    }

    /// Удаляет регистрацию трекинга для формата по его указателю (предотвращает коллизии при пересоздании COM-объектов).
    pub fn unregister_tracking(&self, f: &IDWriteTextFormat) {
        self.tracking.borrow_mut().remove(&(f.as_raw() as usize));
    }

    /// Возвращает добавочный трекинг формата в пикселях (или 0.0, если не задан).
    pub fn tracking_for(&self, f: &IDWriteTextFormat) -> f32 {
        self.tracking.borrow().get(&(f.as_raw() as usize)).copied().unwrap_or(0.0)
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
        let tracking = self.tracking_for(f);
        let tabular = self.tabular.borrow().contains(&(f.as_raw() as usize));
        let layout = crate::sys::with_utf16(s, |wide| -> Result<IDWriteTextLayout> {
            let layout = self.factory.CreateTextLayout(wide, f, w, h)?;
            let range = DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 };
            apply_tracking(&layout, tracking, range.length);
            if tabular && s.bytes().any(|b| b.is_ascii_digit()) {
                let typo = self.factory.CreateTypography()?;
                typo.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, parameter: 1 })?;
                layout.SetTypography(&typo, range)?;
            }
            Ok(layout)
        }).ok()?;
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

    #[test]
    fn test_format_roles_scale_and_tolerance() {
        let island_scale = crate::ios::island_scale();
        let factory: Result<IDWriteFactory> = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) };
        let Ok(f) = factory else { return };
        let fonts = unsafe { Fonts::new(&f) }.expect("Fonts::new failed");
        let tab_fmt = unsafe { crate::gfx::make_text(&f, 1.0) }.expect("make_text failed");

        let mut mapped_count = 0;
        let mut unmapped_count = 0;

        for &(name, role, expected_size, expected_weight) in FORMAT_ROLE_MAP {
            let fmt: &IDWriteTextFormat = match name {
                "body" => &fonts.body,
                "body_c" => &fonts.body_c,
                "small" => &fonts.small,
                "small_c" => &fonts.small_c,
                "small_b" => &fonts.small_b,
                "small_r" => &fonts.small_r,
                "body_r" => &fonts.body_r,
                "caps" => &fonts.caps,
                "caps_c" => &fonts.caps_c,
                "num" => &fonts.num,
                "num_c" => &fonts.num_c,
                "wrap_b" => &fonts.wrap_b,
                "wrap_l" => &fonts.wrap_l,
                "tab" => &tab_fmt,
                "title" => &fonts.title,
                "title_c" => &fonts.title_c,
                "big_c" => &fonts.big_c,
                "icon_small" => &fonts.icon_small,
                "icon" => &fonts.icon,
                "icon_big" => &fonts.icon_big,
                _ => panic!("Неизвестный формат в таблице: {name}"),
            };

            // Сверяем реальный размер и вес созданного DirectWrite-формата с таблицей
            let actual_size = unsafe { fmt.GetFontSize() };
            assert!(
                (actual_size - expected_size).abs() < 1e-4,
                "Формат '{name}': GetFontSize()={actual_size} не совпадает с табличным {expected_size}"
            );

            let actual_weight = unsafe { weight_of(fmt) };
            assert_eq!(
                actual_weight, expected_weight,
                "Формат '{name}': вес {actual_weight} не совпадает с табличным {expected_weight}"
            );

            match role {
                Some(r) => {
                    mapped_count += 1;
                    let (ios_size, _, _, _) = crate::ios::type_spec(r);
                    let scaled_ios = expected_size / island_scale;
                    let diff = (scaled_ios - ios_size).abs();
                    assert!(
                        diff <= 1.5,
                        "Формат '{name}': размер {expected_size} / {island_scale} = {scaled_ios:.2} pt отличается от роли {r:?} ({ios_size} pt) более чем на 1.5 pt (diff={diff:.2})"
                    );
                }
                None => {
                    unmapped_count += 1;
                    assert!(
                        ["title", "title_c", "big_c", "icon_small", "icon", "icon_big"].contains(&name),
                        "Неожиданный не сопоставленный формат: {name}"
                    );
                }
            }
        }
        assert_eq!(mapped_count, 14);
        assert_eq!(unmapped_count, 6);
    }

    #[test]
    fn test_fallback_font_and_weight_590() {
        unsafe {
            let factory: Result<IDWriteFactory> = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED);
            let Ok(f) = factory else { return }; // В окружении без DWrite пропускаем
            // Проверяем запасной невизуализированный / невариативный шрифт Segoe UI с весом 590
            let fmt_res = fmt(&f, w!("Segoe UI"), WEIGHT_EMPHASIZED, 9.0, false);
            assert!(fmt_res.is_ok(), "Создание формата на Segoe UI с весом 590 не должно падать");
            let text_fmt = fmt_res.unwrap();
            let wide: Vec<u16> = "Проверка шрифта".encode_utf16().collect();
            let layout_res = f.CreateTextLayout(&wide, &text_fmt, 200.0, 50.0);
            assert!(layout_res.is_ok(), "Создание TextLayout на Segoe UI не должно падать");
        }
    }

    #[test]
    fn test_tracking_layout_spacing() {
        unsafe {
            let factory: Result<IDWriteFactory> = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED);
            let Ok(f) = factory else { return };
            let fonts_res = Fonts::new(&f);
            let Ok(fonts) = fonts_res else { return };

            let text = "Сегодня в острове длинный текст";
            let wide: Vec<u16> = text.encode_utf16().collect();

            // 1. Раскладка без трекинга
            let layout_normal = f.CreateTextLayout(&wide, &fonts.body, 1000.0, 100.0).unwrap();
            let mut m_normal = DWRITE_TEXT_METRICS::default();
            layout_normal.GetMetrics(&mut m_normal).unwrap();

            // 2. Раскладка с отрицательным трекингом через layout()
            let layout_tracked = fonts.layout(text, &fonts.body, 1000.0, 100.0).unwrap();
            let mut m_tracked = DWRITE_TEXT_METRICS::default();
            layout_tracked.GetMetrics(&mut m_tracked).unwrap();

            // При отрицательном trailingSpacing ширина строки должна стать меньше
            assert!(
                m_tracked.widthIncludingTrailingWhitespace < m_normal.widthIncludingTrailingWhitespace,
                "Ширина с трекингом ({}) должна быть меньше ширины без трекинга ({})",
                m_tracked.widthIncludingTrailingWhitespace,
                m_normal.widthIncludingTrailingWhitespace
            );
        }
    }
}
