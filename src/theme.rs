//! Тема острова. Вся палитра выводится из двух цветов — фона и акцента: поверхности (карточки,
//! дорожки, плашки) получаются подмешиванием «цвета текста» к фону, поэтому на светлом фоне
//! интерфейс остаётся читаемым, а при стандартных цветах совпадает с исходным тёмным оформлением.
//! Палитра хранится в потоке интерфейса; виджеты читают её функциями из `ui` (`card()`, `txt()`…).

use std::cell::Cell;

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::ios;

pub type Rgb = [f32; 3];

/// Стиль цветовой палитры интерфейса острова.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PaletteStyle {
    /// Системные цвета Windows или пользовательские `bg` и `accent`.
    #[default]
    Windows,
    /// Системная палитра iOS 26 Liquid Glass: акцент Blue, системные метки, заливки и разделители.
    Ios,
}

impl PaletteStyle {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Ios => "ios",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ios" => Self::Ios,
            _ => Self::Windows,
        }
    }
}

/// Стандартные цвета: золотой акцент на почти чёрном.
pub const DEFAULT_ACCENT: Rgb = [0.83, 0.69, 0.37];
pub const DEFAULT_BG: Rgb = [0.05, 0.05, 0.05];
const WHITE: Rgb = [1.0; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub style: PaletteStyle,
    pub accent: D2D1_COLOR_F,
    pub bg: D2D1_COLOR_F,
    /// Цвет, к которому подмешиваются поверхности и подсветка: белый на тёмном фоне, чёрный на светлом.
    pub fg: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub dim: D2D1_COLOR_F,
    pub card: D2D1_COLOR_F,
    pub track: D2D1_COLOR_F,
    pub chip: D2D1_COLOR_F,
    pub inset: D2D1_COLOR_F,
    /// Надписи и значки на заливке акцентом.
    pub ink: D2D1_COLOR_F,
    /// Кромка стекла: блик слева-сверху, середина, правый-нижний край. Блик белый в обеих темах.
    pub rim_hi: D2D1_COLOR_F,
    pub rim_mid: D2D1_COLOR_F,
    pub rim_lo: D2D1_COLOR_F,
    /// Осветление сверху тела острова.
    pub sheen: D2D1_COLOR_F,
    /// Нейтральная капля выбранной вкладки.
    pub pill: D2D1_COLOR_F,
    /// Волосяная граница карточек.
    pub hairline: D2D1_COLOR_F,
}

fn col(c: Rgb) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a: 1.0 }
}

fn rgba(c: Rgb, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a }
}

pub fn mix3(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Яркость цвета 0..1 (по весам Rec. 709, без гамма-коррекции: нужна только для выбора светлый/тёмный).
pub fn luminance(c: Rgb) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn is_light(c: Rgb) -> bool {
    luminance(c) > 0.5
}

/// RGB (0..1) → HSV: оттенок 0..360, насыщенность и значение 0..1.
pub fn rgb_to_hsv(c: Rgb) -> [f32; 3] {
    let [r, g, b] = c;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= f32::EPSILON {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= f32::EPSILON { 0.0 } else { d / max };
    [h.rem_euclid(360.0), s, max]
}

pub fn hsv_to_rgb(hsv: [f32; 3]) -> Rgb {
    let h = hsv[0].rem_euclid(360.0);
    let (s, v) = (hsv[1].clamp(0.0, 1.0), hsv[2].clamp(0.0, 1.0));
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// `#RRGGBB` → цвет.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some([ch(0)?, ch(2)?, ch(4)?])
}

pub fn to_hex(c: Rgb) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", b(c[0]), b(c[1]), b(c[2]))
}

/// Палитра из фона, акцента и выбранного стиля.
pub fn derive(bg: Rgb, accent: Rgb, style: PaletteStyle) -> Palette {
    let light = is_light(bg);
    let dark = !light;
    let fg: Rgb = if light { [0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0] };
    let on = |t: f32| col(mix3(bg, fg, t));

    // Тёмная надпись на светлом акценте, светлая — на тёмном.
    let ink_for = |acc: Rgb| if is_light(acc) { [0.10, 0.085, 0.05] } else { [0.97, 0.97, 0.97] };

    let (rim_hi_a, rim_mid_a, rim_lo_col, rim_lo_a) = crate::glass::rim_alphas(dark);
    let sheen_a = crate::glass::sheen_alpha(dark);

    // Базовая палитра (стиль Windows).
    let mut p = Palette {
        style,
        accent: col(accent),
        bg: col(bg),
        fg: col(fg),
        text: on(if light { 0.90 } else { 0.945 }),
        dim: on(0.78),
        card: on(0.058),
        track: on(0.128),
        chip: on(0.105),
        inset: on(0.095),
        ink: col(ink_for(accent)),
        rim_hi: rgba(WHITE, rim_hi_a),
        rim_mid: rgba(WHITE, rim_mid_a),
        rim_lo: rgba(rim_lo_col, rim_lo_a),
        sheen: rgba(WHITE, sheen_a),
        pill: on(if light { 0.08 } else { 0.16 }),
        hairline: D2D1_COLOR_F { a: if light { 0.08 } else { 0.07 }, ..col(fg) },
    };

    if style == PaletteStyle::Ios {
        // Системный акцент Blue iOS 26 для текущей темы (тёмная/светлая).
        let ios_accent = ios::sys(ios::SysColor::Blue, dark);
        p.accent = col(ios_accent);
        p.ink = col(ink_for(ios_accent));

        // Текст: основной — Label Primary (непрозрачный).
        p.text = ios::label_primary(dark);

        // Вторичный текст (dim): токен Label Secondary (с альфой 0.6) смешивается с фоном,
        // чтобы поле оставалось непрозрачным (альфа 1.0) как и у стиля Windows.
        let sec = ios::label_secondary(dark);
        let dim_rgb = mix3(bg, [sec.r, sec.g, sec.b], sec.a);
        p.dim = col(dim_rgb);

        // Поверхности по смыслу и визуальной иерархии iOS:
        // card — Quaternary Fill (самая нежная заливка подложки крупных карточек);
        // track — Primary Fill (наиболее контрастная заливка для тонких дорожек слайдеров и прогресса);
        // chip — Secondary Fill (интерактивные плашки/чипы и кнопки);
        // inset — Tertiary Fill (утопленные поля ввода и внутренние группировки).
        p.card = ios::fill_quaternary(dark);
        p.track = ios::fill_primary(dark);
        p.chip = ios::fill_secondary(dark);
        p.inset = ios::fill_tertiary(dark);

        // Волосяная граница: полупрозрачный системный разделитель iOS (Non-Opaque).
        p.hairline = ios::separator_non_opaque(dark);
    }

    p
}

/// Цвета системы Windows → (фон острова, акцент). Тёмная тема: фон — самый тёмный оттенок
/// системного акцента, сам акцент осветляется до читаемого на тёмном. Светлая тема — наоборот.
pub fn from_system(sys: SystemColors, dark: bool) -> (Rgb, Rgb) {
    let hsv = rgb_to_hsv(sys.accent);
    if dark {
        // Серый акцент (насыщенность ≈ 0) остаётся серым; цветной — не бледнеет.
        let accent = hsv_to_rgb([hsv[0], if hsv[1] < 0.08 { hsv[1] } else { hsv[1].max(0.35) }, hsv[2].max(0.72)]);
        (sys.darkest, accent)
    } else {
        let accent = hsv_to_rgb([hsv[0], hsv[1], hsv[2].min(0.62)]);
        (mix3(sys.lightest, [1.0, 1.0, 1.0], 0.62), accent)
    }
}

/// Цвета акцента Windows: сам акцент, самый светлый и самый тёмный оттенки палитры.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemColors {
    pub accent: Rgb,
    pub lightest: Rgb,
    pub darkest: Rgb,
}

impl SystemColors {
    /// Палитра из одного цвета (если `AccentPalette` в реестре нет): оттенки считаем сами.
    pub fn from_accent(accent: Rgb) -> Self {
        Self { accent, lightest: mix3(accent, [1.0, 1.0, 1.0], 0.45), darkest: mix3(accent, [0.0, 0.0, 0.0], 0.82) }
    }
}

/// Разбор значения `AccentPalette` (8 цветов по 4 байта RGBA: Light3, Light2, Light1, Accent,
/// Dark1, Dark2, Dark3, служебный).
pub fn parse_accent_palette(bytes: &[u8]) -> Option<SystemColors> {
    if bytes.len() < 28 {
        return None;
    }
    let at = |i: usize| [bytes[i * 4] as f32 / 255.0, bytes[i * 4 + 1] as f32 / 255.0, bytes[i * 4 + 2] as f32 / 255.0];
    Some(SystemColors { accent: at(3), lightest: at(0), darkest: at(6) })
}

thread_local! {
    static PALETTE: Cell<Palette> = Cell::new(derive(DEFAULT_BG, DEFAULT_ACCENT, PaletteStyle::Windows));
    static GLASS: Cell<bool> = const { Cell::new(true) };
}

/// Эффект стекла включён (ключ `glass`): виджеты рисуют волосяные границы и блики.
pub fn set_glass(on: bool) {
    GLASS.with(|c| c.set(on));
}

pub fn glass() -> bool {
    GLASS.with(|c| c.get())
}

pub fn set(p: Palette) {
    PALETTE.with(|c| c.set(p));
}

pub fn get() -> Palette {
    PALETTE.with(|c| c.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_matches_original_look() {
        let p = derive(DEFAULT_BG, DEFAULT_ACCENT, PaletteStyle::Windows);
        // Исходные константы: CARD 0.105, TRACK 0.17, CHIP 0.15, INSET 0.14, TEXT 0.95, DIM 0.58 (с тех пор осветлён: серый текст терялся на стекле).
        for (got, want) in [(p.card.r, 0.105), (p.track.r, 0.17), (p.chip.r, 0.15), (p.inset.r, 0.14), (p.text.r, 0.95), (p.dim.r, 0.79)] {
            assert!((got - want).abs() < 0.01, "{got} против {want}");
        }
        // Тёмная надпись на золотом акценте.
        assert!(p.ink.r < 0.2);
    }

    #[test]
    fn pill_is_darker_in_light_theme_and_lighter_in_dark_theme() {
        // Тёмная тема: капля светлее фона острова.
        let dark = derive(DEFAULT_BG, DEFAULT_ACCENT, PaletteStyle::Windows);
        assert!(!is_light(DEFAULT_BG));
        assert!(luminance([dark.pill.r, dark.pill.g, dark.pill.b]) > luminance(DEFAULT_BG));

        // Светлая тема: капля темнее фона острова.
        let light_bg = [0.95, 0.95, 0.96];
        let light = derive(light_bg, DEFAULT_ACCENT, PaletteStyle::Windows);
        assert!(is_light(light_bg));
        assert!(luminance([light.pill.r, light.pill.g, light.pill.b]) < luminance(light_bg));
    }

    #[test]
    fn glass_tokens_are_sane_on_any_background() {
        for bg in [DEFAULT_BG, [0.95, 0.95, 0.96], [0.8, 0.1, 0.1], [0.1, 0.2, 0.9], [0.1, 0.6, 0.2], [0.5, 0.5, 0.5]] {
            let p = derive(bg, DEFAULT_ACCENT, PaletteStyle::Windows);
            for c in [p.rim_hi, p.rim_mid, p.rim_lo, p.sheen, p.hairline] {
                assert!((0.0..=1.0).contains(&c.a), "{bg:?}: alpha {}", c.a);
            }
            // Блик сильнее середины кромки; граница карточек едва заметна.
            assert!(p.rim_hi.a > p.rim_mid.a && p.hairline.a < 0.15);
            // Блик белый в обеих темах.
            assert!(p.rim_hi.r == 1.0 && p.rim_hi.g == 1.0 && p.rim_hi.b == 1.0);
            // Капля отличается от фона в сторону текста: светлее на тёмном, темнее на светлом.
            let (pl, bl) = (luminance([p.pill.r, p.pill.g, p.pill.b]), luminance(bg));
            if is_light(bg) {
                assert!(pl < bl, "{bg:?}");
            } else {
                assert!(pl > bl, "{bg:?}");
            }
        }
    }

    #[test]
    fn light_background_flips_text() {
        let p = derive([0.95, 0.95, 0.96], [0.1, 0.4, 0.8], PaletteStyle::Windows);
        assert!(p.text.r < 0.3 && p.card.r < 0.95);
        // Светлая надпись на тёмном акценте.
        assert!(p.ink.r > 0.9);
    }

    #[test]
    fn ios_palette_dark_and_light() {
        // Тёмный фон: акцент Blue Dark (#0091FF), Label Primary Dark (#FFFFFF)
        let dark_p = derive(DEFAULT_BG, DEFAULT_ACCENT, PaletteStyle::Ios);
        let blue_dark = ios::sys(ios::SysColor::Blue, true);
        assert_eq!([dark_p.accent.r, dark_p.accent.g, dark_p.accent.b], blue_dark);
        assert_eq!(dark_p.text, ios::label_primary(true));
        assert!(dark_p.text.r > 0.9, "текст на тёмном фоне должен быть светлым");

        // Светлый фон: акцент Blue Light (#0088FF), Label Primary Light (#000000)
        let light_bg = [0.95, 0.95, 0.96];
        let light_p = derive(light_bg, DEFAULT_ACCENT, PaletteStyle::Ios);
        let blue_light = ios::sys(ios::SysColor::Blue, false);
        assert_eq!([light_p.accent.r, light_p.accent.g, light_p.accent.b], blue_light);
        assert_eq!(light_p.text, ios::label_primary(false));
        assert!(light_p.text.r < 0.1, "текст на светлом фоне должен быть тёмным");

        // Все поля должны иметь валидные альфы в диапазоне 0..=1
        for p in [dark_p, light_p] {
            for (name, c) in [
                ("accent", p.accent),
                ("bg", p.bg),
                ("fg", p.fg),
                ("text", p.text),
                ("dim", p.dim),
                ("card", p.card),
                ("track", p.track),
                ("chip", p.chip),
                ("inset", p.inset),
                ("ink", p.ink),
                ("rim_hi", p.rim_hi),
                ("rim_mid", p.rim_mid),
                ("rim_lo", p.rim_lo),
                ("sheen", p.sheen),
                ("pill", p.pill),
                ("hairline", p.hairline),
            ] {
                assert!((0.0..=1.0).contains(&c.a), "поле {name} имеет недопустимую альфу {}", c.a);
            }
            // dim непрозрачный
            assert_eq!(p.dim.a, 1.0);
        }

        // Проверка соответствия токенов заливки iOS
        assert_eq!(dark_p.card, ios::fill_quaternary(true));
        assert_eq!(dark_p.track, ios::fill_primary(true));
        assert_eq!(dark_p.chip, ios::fill_secondary(true));
        assert_eq!(dark_p.inset, ios::fill_tertiary(true));
        assert_eq!(dark_p.hairline, ios::separator_non_opaque(true));

        assert_eq!(light_p.card, ios::fill_quaternary(false));
        assert_eq!(light_p.track, ios::fill_primary(false));
        assert_eq!(light_p.chip, ios::fill_secondary(false));
        assert_eq!(light_p.inset, ios::fill_tertiary(false));
        assert_eq!(light_p.hairline, ios::separator_non_opaque(false));
    }

    #[test]
    fn style_change_with_same_colors_yields_different_palette() {
        let p_win = derive(DEFAULT_BG, DEFAULT_ACCENT, PaletteStyle::Windows);
        let p_ios = derive(DEFAULT_BG, DEFAULT_ACCENT, PaletteStyle::Ios);
        assert_ne!(p_win, p_ios);
        assert_eq!(p_win.style, PaletteStyle::Windows);
        assert_eq!(p_ios.style, PaletteStyle::Ios);
    }

    #[test]
    fn hsv_roundtrip() {
        for c in [[0.83, 0.69, 0.37], [0.1, 0.4, 0.8], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.2, 0.2, 0.2]] {
            let back = hsv_to_rgb(rgb_to_hsv(c));
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 0.002, "{c:?} → {back:?}");
            }
        }
    }

    #[test]
    fn hex_parse_and_format() {
        assert_eq!(to_hex(parse_hex("#d4af5e").unwrap()), "#D4AF5E");
        assert_eq!(parse_hex("D4AF5E"), parse_hex("#D4AF5E"));
        assert!(parse_hex("#12345").is_none() && parse_hex("#GGGGGG").is_none() && parse_hex("").is_none());
    }

    #[test]
    fn accent_palette_parsing() {
        // Палитра этого ПК (оливковая): Light3 768452 … Accent 464E30 … Dark3 16190F.
        let raw: Vec<u8> = (0..8)
            .flat_map(|i| match i {
                0 => [0x76, 0x84, 0x52, 0xFF],
                3 => [0x46, 0x4E, 0x30, 0xFF],
                6 => [0x16, 0x19, 0x0F, 0xFF],
                _ => [0x40, 0x40, 0x40, 0xFF],
            })
            .collect();
        let s = parse_accent_palette(&raw).unwrap();
        assert_eq!(to_hex(s.accent), "#464E30");
        assert_eq!(to_hex(s.darkest), "#16190F");
        assert!(parse_accent_palette(&raw[..8]).is_none());
    }

    #[test]
    fn system_colors_stay_legible() {
        let s = SystemColors { accent: [0.27, 0.31, 0.19], lightest: [0.46, 0.52, 0.32], darkest: [0.09, 0.10, 0.06] };
        let (bg, accent) = from_system(s, true);
        assert!(!is_light(bg), "фон острова в тёмной теме должен быть тёмным");
        assert!(luminance(accent) > 0.4, "акцент должен читаться на тёмном: {accent:?}");
        let (bg, accent) = from_system(s, false);
        assert!(is_light(bg));
        assert!(luminance(accent) < 0.6);
    }
}
