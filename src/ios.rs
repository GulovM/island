//! Дизайн-система Apple iOS 26 Liquid Glass: токены материалов, цветов, типографики и геометрии.
//!
//! Источник параметров: выгрузка Figma «iOS and iPadOS 26 Community» (см. `docs/figma/`).
//! Все функции модуля — чистые данные и вычисления без вызовов Win32 API.

#![allow(dead_code)] // Убрать в T7, когда токены будут использованы.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

pub use crate::theme::Rgb;

/// Вспомогательный конструктор цвета D2D1_COLOR_F.
pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

/// Создание RGB из hex-числа вида `0xRRGGBB`.
pub const fn hex(rgb_hex: u32) -> Rgb {
    [
        ((rgb_hex >> 16) & 0xFF) as f32 / 255.0,
        ((rgb_hex >> 8) & 0xFF) as f32 / 255.0,
        (rgb_hex & 0xFF) as f32 / 255.0,
    ]
}

/// Создание цвета D2D1_COLOR_F из hex-числа `0xRRGGBB` и прозрачности `alpha`.
pub const fn hex_a(rgb_hex: u32, a: f32) -> D2D1_COLOR_F {
    let [r, g, b] = hex(rgb_hex);
    rgba(r, g, b, a)
}

/// Перевод `D2D1_COLOR_F` в пару `(Rgb, alpha)`.
pub const fn to_rgb_alpha(c: D2D1_COLOR_F) -> (Rgb, f32) {
    ([c.r, c.g, c.b], c.a)
}

/// Перевод пары `(Rgb, alpha)` в `D2D1_COLOR_F`.
pub const fn to_d2d(rgb: Rgb, a: f32) -> D2D1_COLOR_F {
    rgba(rgb[0], rgb[1], rgb[2], a)
}

// ---------------------------------------------------------------------------
// 1. Материалы (Materials)
// ---------------------------------------------------------------------------

/// Пять системных материалов iOS 26 (раздел `_Materials/Mode Options`, `docs/figma/materials.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Ultrathin,
    Thin,
    Regular,
    Thick,
    Chrome,
}

/// Числовая спецификация материала: радиус размытия фона и слои заливки.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialSpec {
    /// Радиус размытия подложки (backdrop-blur) в пикселях (25 или 50 px).
    pub blur_px: f32,
    /// Слои заливок в порядке Figma (индекс 0 — базовый слой, индекс 1 — верхний).
    pub fills: &'static [D2D1_COLOR_F],
}

// Заливки материалов из docs/figma/materials.json / materials.md (раздел 1):
// Light:
const FILLS_ULTRATHIN_LIGHT: [D2D1_COLOR_F; 2] = [rgba(1.0, 1.0, 1.0, 0.03), rgba(1.0, 1.0, 1.0, 0.07)];
const FILLS_THIN_LIGHT: [D2D1_COLOR_F; 2] = [rgba(1.0, 1.0, 1.0, 0.4), rgba(1.0, 1.0, 1.0, 0.05)];
const FILLS_REGULAR_LIGHT: [D2D1_COLOR_F; 2] = [rgba(1.0, 1.0, 1.0, 0.6), rgba(1.0, 1.0, 1.0, 0.25)];
const FILLS_THICK_LIGHT: [D2D1_COLOR_F; 2] = [rgba(1.0, 1.0, 1.0, 0.84), rgba(1.0, 1.0, 1.0, 0.34)];
const FILLS_CHROME_LIGHT: [D2D1_COLOR_F; 1] = [rgba(1.0, 1.0, 1.0, 0.75)];

// Dark:
const FILLS_ULTRATHIN_DARK: [D2D1_COLOR_F; 1] = [rgba(0.0, 0.0, 0.0, 0.02)];
const FILLS_THIN_DARK: [D2D1_COLOR_F; 1] = [rgba(0.0, 0.0, 0.0, 0.26)];
const FILLS_REGULAR_DARK: [D2D1_COLOR_F; 1] = [rgba(0.0, 0.0, 0.0, 0.41)];
const FILLS_THICK_DARK: [D2D1_COLOR_F; 1] = [rgba(0.0, 0.0, 0.0, 0.6)];
const FILLS_CHROME_DARK: [D2D1_COLOR_F; 2] = [hex_a(0x7C7C7C, 1.0), rgba(28.0 / 255.0, 28.0 / 255.0, 28.0 / 255.0, 0.9)];

/// Возвращает спецификацию системного материала (размытие и слои заливки) по выгрузке Figma.
pub fn material(m: Material, dark: bool) -> MaterialSpec {
    match (m, dark) {
        (Material::Ultrathin, false) => MaterialSpec { blur_px: 50.0, fills: &FILLS_ULTRATHIN_LIGHT },
        (Material::Thin, false)      => MaterialSpec { blur_px: 50.0, fills: &FILLS_THIN_LIGHT },
        (Material::Regular, false)   => MaterialSpec { blur_px: 50.0, fills: &FILLS_REGULAR_LIGHT },
        (Material::Thick, false)     => MaterialSpec { blur_px: 50.0, fills: &FILLS_THICK_LIGHT },
        (Material::Chrome, false)    => MaterialSpec { blur_px: 25.0, fills: &FILLS_CHROME_LIGHT },
        (Material::Ultrathin, true)  => MaterialSpec { blur_px: 50.0, fills: &FILLS_ULTRATHIN_DARK },
        (Material::Thin, true)       => MaterialSpec { blur_px: 50.0, fills: &FILLS_THIN_DARK },
        (Material::Regular, true)    => MaterialSpec { blur_px: 50.0, fills: &FILLS_REGULAR_DARK },
        (Material::Thick, true)      => MaterialSpec { blur_px: 50.0, fills: &FILLS_THICK_DARK },
        (Material::Chrome, true)     => MaterialSpec { blur_px: 50.0, fills: &FILLS_CHROME_DARK },
    }
}

/// Композиция «поверх» (Porter-Duff source-over) для двух заливок: `top` поверх `bottom`.
///
/// Формула:
/// `alpha_out = alpha_top + alpha_bottom * (1.0 - alpha_top)`
/// `color_out = (color_top * alpha_top + color_bottom * alpha_bottom * (1.0 - alpha_top)) / alpha_out`
/// При `alpha_out == 0` возвращается нулевой чёрный цвет.
pub fn composite_over(bottom: (Rgb, f32), top: (Rgb, f32)) -> (Rgb, f32) {
    let (c_b, a_b) = bottom;
    let (c_t, a_t) = top;
    let a_out = a_t + a_b * (1.0 - a_t);
    if a_out <= 0.0 {
        return ([0.0, 0.0, 0.0], 0.0);
    }
    let weight_t = a_t;
    let weight_b = a_b * (1.0 - a_t);
    let r = (c_t[0] * weight_t + c_b[0] * weight_b) / a_out;
    let g = (c_t[1] * weight_t + c_b[1] * weight_b) / a_out;
    let b = (c_t[2] * weight_t + c_b[2] * weight_b) / a_out;
    ([r, g, b], a_out)
}

/// Единая итоговая заливка тела острова `(rgb, alpha)` для выбранного материала и темы.
/// Если в таблице материала несколько заливок — они последовательно компонуются операцией «поверх».
pub fn body_tint(m: Material, dark: bool) -> (Rgb, f32) {
    let spec = material(m, dark);
    match spec.fills {
        [] => ([0.0, 0.0, 0.0], 0.0),
        [single] => to_rgb_alpha(*single),
        [bottom, top] => composite_over(to_rgb_alpha(*bottom), to_rgb_alpha(*top)),
        [first, rest @ ..] => {
            let mut acc = to_rgb_alpha(*first);
            for next in rest {
                acc = composite_over(acc, to_rgb_alpha(*next));
            }
            acc
        }
    }
}

/// Итоговая заливка тела острова в виде `D2D1_COLOR_F`.
pub fn body_tint_color(m: Material, dark: bool) -> D2D1_COLOR_F {
    let (c, a) = body_tint(m, dark);
    to_d2d(c, a)
}

// ---------------------------------------------------------------------------
// 2. Системные цвета (System Colors)
// ---------------------------------------------------------------------------

/// 12 системных цветов iOS 26 (`docs/figma/colors.md`, раздел 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SysColor {
    Red,
    Orange,
    Yellow,
    Green,
    Mint,
    Teal,
    Cyan,
    Blue,
    Indigo,
    Purple,
    Pink,
    Brown,
}

/// Возвращает системный цвет в стандартном режиме (Default Light/Dark) из спецификации.
pub fn sys(c: SysColor, dark: bool) -> Rgb {
    match (c, dark) {
        (SysColor::Red, false)    => hex(0xFF383C),
        (SysColor::Orange, false) => hex(0xFF8D28),
        (SysColor::Yellow, false) => hex(0xFFCC00),
        (SysColor::Green, false)  => hex(0x34C759),
        (SysColor::Mint, false)   => hex(0x00C8B3),
        (SysColor::Teal, false)   => hex(0x00C3D0),
        (SysColor::Cyan, false)   => hex(0x00C0E8),
        (SysColor::Blue, false)   => hex(0x0088FF),
        (SysColor::Indigo, false) => hex(0x6155F5),
        (SysColor::Purple, false) => hex(0xCB30E0),
        (SysColor::Pink, false)   => hex(0xFF2D55),
        (SysColor::Brown, false)  => hex(0xAC7F5E),

        (SysColor::Red, true)     => hex(0xFF4245),
        (SysColor::Orange, true)  => hex(0xFF9230),
        (SysColor::Yellow, true)  => hex(0xFFD600),
        (SysColor::Green, true)   => hex(0x30D158),
        (SysColor::Mint, true)    => hex(0x00DAC3),
        (SysColor::Teal, true)    => hex(0x00D2E0),
        (SysColor::Cyan, true)    => hex(0x3CD3FE),
        (SysColor::Blue, true)    => hex(0x0091FF),
        (SysColor::Indigo, true)  => hex(0x6D7CFF),
        (SysColor::Purple, true)  => hex(0xDB34F2),
        (SysColor::Pink, true)    => hex(0xFF375F),
        (SysColor::Brown, true)   => hex(0xB78A66),
    }
}

// ---------------------------------------------------------------------------
// 3. Оттенки серого (Grays)
// ---------------------------------------------------------------------------

/// 8 оттенков серого шкалы iOS 26 (`docs/figma/colors.md`, раздел 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrayToken {
    Black,
    Gray,
    Gray2,
    Gray3,
    Gray4,
    Gray5,
    Gray6,
    White,
}

/// Возвращает системный оттенок серого для заданной темы.
pub fn gray(g: GrayToken, dark: bool) -> Rgb {
    match (g, dark) {
        (GrayToken::Black, _)     => hex(0x000000),
        (GrayToken::Gray, _)      => hex(0x8E8E93),
        (GrayToken::Gray2, false) => hex(0xAEAEB2),
        (GrayToken::Gray2, true)  => hex(0x636366),
        (GrayToken::Gray3, false) => hex(0xC7C7CC),
        (GrayToken::Gray3, true)  => hex(0x48484A),
        (GrayToken::Gray4, false) => hex(0xD1D1D6),
        (GrayToken::Gray4, true)  => hex(0x3A3A3C),
        (GrayToken::Gray5, false) => hex(0xE5E5EA),
        (GrayToken::Gray5, true)  => hex(0x2C2C2E),
        (GrayToken::Gray6, false) => hex(0xF2F2F7),
        (GrayToken::Gray6, true)  => hex(0x1C1C1E),
        (GrayToken::White, _)     => hex(0xFFFFFF),
    }
}

// ---------------------------------------------------------------------------
// 4. Метки, заливки, разделители, фоны (Labels, Fills, Separators, Backgrounds)
// ---------------------------------------------------------------------------

/// Уровни контрастности меток и текста (Labels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelToken {
    Primary,
    Secondary,
    Tertiary,
    Quaternary,
}

/// Цвет метки (текста) по спецификации `docs/figma/colors.md` (раздел 3).
pub fn label(l: LabelToken, dark: bool) -> D2D1_COLOR_F {
    match (l, dark) {
        (LabelToken::Primary, false)    => hex_a(0x000000, 1.0),
        (LabelToken::Primary, true)     => hex_a(0xFFFFFF, 1.0),
        (LabelToken::Secondary, false)  => rgba(60.0 / 255.0, 60.0 / 255.0, 67.0 / 255.0, 0.6),
        (LabelToken::Secondary, true)   => rgba(235.0 / 255.0, 235.0 / 255.0, 245.0 / 255.0, 0.6),
        (LabelToken::Tertiary, false)   => rgba(60.0 / 255.0, 60.0 / 255.0, 67.0 / 255.0, 0.3),
        (LabelToken::Tertiary, true)    => rgba(235.0 / 255.0, 235.0 / 255.0, 245.0 / 255.0, 0.3),
        (LabelToken::Quaternary, false) => rgba(60.0 / 255.0, 60.0 / 255.0, 67.0 / 255.0, 0.18),
        (LabelToken::Quaternary, true)  => rgba(235.0 / 255.0, 235.0 / 255.0, 245.0 / 255.0, 0.16),
    }
}

pub fn label_primary(dark: bool) -> D2D1_COLOR_F { label(LabelToken::Primary, dark) }
pub fn label_secondary(dark: bool) -> D2D1_COLOR_F { label(LabelToken::Secondary, dark) }
pub fn label_tertiary(dark: bool) -> D2D1_COLOR_F { label(LabelToken::Tertiary, dark) }
pub fn label_quaternary(dark: bool) -> D2D1_COLOR_F { label(LabelToken::Quaternary, dark) }

/// Уровни заливок поверхностей (Fills).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillToken {
    Primary,
    Secondary,
    Tertiary,
    Quaternary,
}

/// Заливка поверхности по спецификации `docs/figma/colors.md` (раздел 4).
pub fn fill(f: FillToken, dark: bool) -> D2D1_COLOR_F {
    match (f, dark) {
        (FillToken::Primary, false)    => rgba(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.20),
        (FillToken::Primary, true)     => rgba(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.36),
        (FillToken::Secondary, false)  => rgba(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.16),
        (FillToken::Secondary, true)   => rgba(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.32),
        (FillToken::Tertiary, false)   => rgba(118.0 / 255.0, 118.0 / 255.0, 128.0 / 255.0, 0.12),
        (FillToken::Tertiary, true)    => rgba(118.0 / 255.0, 118.0 / 255.0, 128.0 / 255.0, 0.24),
        (FillToken::Quaternary, false) => rgba(116.0 / 255.0, 116.0 / 255.0, 128.0 / 255.0, 0.08),
        (FillToken::Quaternary, true)  => rgba(118.0 / 255.0, 118.0 / 255.0, 128.0 / 255.0, 0.18),
    }
}

pub fn fill_primary(dark: bool) -> D2D1_COLOR_F { fill(FillToken::Primary, dark) }
pub fn fill_secondary(dark: bool) -> D2D1_COLOR_F { fill(FillToken::Secondary, dark) }
pub fn fill_tertiary(dark: bool) -> D2D1_COLOR_F { fill(FillToken::Tertiary, dark) }
pub fn fill_quaternary(dark: bool) -> D2D1_COLOR_F { fill(FillToken::Quaternary, dark) }

/// Типы разделителей (Separators).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeparatorToken {
    Opaque,
    NonOpaque,
}

/// Цвет разделителя по спецификации `docs/figma/colors.md` (раздел 5).
pub fn separator(s: SeparatorToken, dark: bool) -> D2D1_COLOR_F {
    match (s, dark) {
        (SeparatorToken::Opaque, false)    => hex_a(0xC6C6C8, 1.0),
        (SeparatorToken::Opaque, true)     => hex_a(0x38383A, 1.0),
        (SeparatorToken::NonOpaque, false) => rgba(0.0, 0.0, 0.0, 0.12),
        (SeparatorToken::NonOpaque, true)  => rgba(1.0, 1.0, 1.0, 0.17),
    }
}

pub fn separator_opaque(dark: bool) -> D2D1_COLOR_F { separator(SeparatorToken::Opaque, dark) }
pub fn separator_non_opaque(dark: bool) -> D2D1_COLOR_F { separator(SeparatorToken::NonOpaque, dark) }

/// Системные фоны (System Backgrounds: Light и Dark Base) по `docs/figma/colors.md` (раздел 6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BgToken {
    Primary,
    Secondary,
    Tertiary,
}

pub fn bg(b: BgToken, dark: bool) -> D2D1_COLOR_F {
    match (b, dark) {
        (BgToken::Primary, false)   => hex_a(0xFFFFFF, 1.0),
        (BgToken::Primary, true)    => hex_a(0x000000, 1.0),
        (BgToken::Secondary, false) => hex_a(0xF2F2F7, 1.0),
        (BgToken::Secondary, true)  => hex_a(0x1C1C1E, 1.0),
        (BgToken::Tertiary, false)  => hex_a(0xFFFFFF, 1.0),
        (BgToken::Tertiary, true)   => hex_a(0x2C2C2E, 1.0),
    }
}

pub fn bg_primary(dark: bool) -> D2D1_COLOR_F { bg(BgToken::Primary, dark) }
pub fn bg_secondary(dark: bool) -> D2D1_COLOR_F { bg(BgToken::Secondary, dark) }
pub fn bg_tertiary(dark: bool) -> D2D1_COLOR_F { bg(BgToken::Tertiary, dark) }

// ---------------------------------------------------------------------------
// 5. Радиусы, тени и метрики геометрии
// ---------------------------------------------------------------------------

/// Радиус скругления крупных стеклянных контейнеров Liquid Glass Large/Medium (34 px).
pub const R_LARGE: f32 = 34.0;
/// Радиус скругления секций, списков Inset Grouped, меню и уведомлений (26 px).
pub const R_MEDIUM: f32 = 26.0;
/// Капсульный радиус для кнопок и плашек (1000 px).
pub const R_SMALL_CAPSULE: f32 = 1000.0;
/// Радиус круглой подложки малого эффекта Liquid Glass Small (296 px).
pub const R_SMALL_CIRCLE: f32 = 296.0;
/// Радиус скругления Liquid Glass Clear (24 px).
pub const R_CLEAR: f32 = 24.0;

/// Стандартная толщина волосяного разделителя iOS (0.5 px).
pub const SEPARATOR_THICKNESS: f32 = 0.5;

/// Спецификация внешней тени (Box Shadow).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowSpec {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: D2D1_COLOR_F,
}

/// Стандартная мягкая тень стеклянных контейнеров Liquid Glass: `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`.
pub const GLASS_SHADOW: ShadowSpec = ShadowSpec {
    offset_x: 0.0,
    offset_y: 8.0,
    blur_radius: 40.0,
    color: rgba(0.0, 0.0, 0.0, 0.12),
};

// ---------------------------------------------------------------------------
// 6. Типографика (Typography)
// ---------------------------------------------------------------------------

/// 11 текстовых ролей типографики Apple iOS 26 (`docs/figma/typography.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    LargeTitle,
    Title1,
    Title2,
    Title3,
    Headline,
    Body,
    Callout,
    Subheadline,
    Footnote,
    Caption1,
    Caption2,
}

/// Параметры текстовой роли: `(font_size_px, line_height_px, font_weight, tracking_em)`.
/// Для Headline вес по умолчанию — Semibold (590), для остальных ролей — Regular (400).
pub fn type_spec(role: TextRole) -> (f32, f32, u32, f32) {
    match role {
        TextRole::LargeTitle  => (34.0, 41.0, 400, 0.0118),
        TextRole::Title1      => (28.0, 34.0, 400, 0.0136),
        TextRole::Title2      => (22.0, 28.0, 400, -0.0118),
        TextRole::Title3      => (20.0, 25.0, 400, -0.0225),
        TextRole::Headline    => (17.0, 22.0, 590, -0.0253),
        TextRole::Body        => (17.0, 22.0, 400, -0.0253),
        TextRole::Callout     => (16.0, 21.0, 400, -0.0194),
        TextRole::Subheadline => (15.0, 20.0, 400, -0.0153),
        TextRole::Footnote    => (13.0, 18.0, 400, -0.0062),
        TextRole::Caption1    => (12.0, 16.0, 400, 0.0),
        TextRole::Caption2    => (11.0, 13.0, 400, 0.0055),
    }
}

/// Параметры усиленного начертания (Emphasized): вес 700 (Bold) для крупных заголовков,
/// 590 (Semibold) для Title 3..Caption 2.
pub fn type_spec_emphasized(role: TextRole) -> (f32, f32, u32, f32) {
    let (size, line, _, tracking) = type_spec(role);
    let weight = match role {
        TextRole::LargeTitle | TextRole::Title1 | TextRole::Title2 => 700,
        _ => 590,
    };
    (size, line, weight, tracking)
}

/// Коэффициент перевода типографических размеров iOS в размеры острова.
/// В острове основной текст равен 9 px при iOS Body 17 px -> `9.0 / 17.0`.
pub fn island_scale() -> f32 {
    9.0 / 17.0
}

/// Размер шрифта для роли в масштабе острова с округлением до 0.5 px.
pub fn island_size(role: TextRole) -> f32 {
    let (ios_size, _, _, _) = type_spec(role);
    (ios_size * island_scale() * 2.0).round() / 2.0
}

// ---------------------------------------------------------------------------
// Тесты
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_color_close(actual: D2D1_COLOR_F, expected: D2D1_COLOR_F, msg: &str) {
        let eps = 1e-4;
        assert!((actual.r - expected.r).abs() < eps, "{msg} [r]: {} != {}", actual.r, expected.r);
        assert!((actual.g - expected.g).abs() < eps, "{msg} [g]: {} != {}", actual.g, expected.g);
        assert!((actual.b - expected.b).abs() < eps, "{msg} [b]: {} != {}", actual.b, expected.b);
        assert!((actual.a - expected.a).abs() < eps, "{msg} [a]: {} != {}", actual.a, expected.a);
    }

    fn assert_rgb_close(actual: Rgb, expected: Rgb, msg: &str) {
        let eps = 1e-4;
        assert!((actual[0] - expected[0]).abs() < eps, "{msg} [0]: {} != {}", actual[0], expected[0]);
        assert!((actual[1] - expected[1]).abs() < eps, "{msg} [1]: {} != {}", actual[1], expected[1]);
        assert!((actual[2] - expected[2]).abs() < eps, "{msg} [2]: {} != {}", actual[2], expected[2]);
    }

    #[test]
    fn test_all_materials_exact_spec_table() {
        // (Материал, dark, blur_px, ожидаемые заливки [(r,g,b,a)], ожидаемый body_tint ([r,g,b], a))
        struct Case {
            m: Material,
            dark: bool,
            blur: f32,
            fills: &'static [(f32, f32, f32, f32)],
            tint: (Rgb, f32),
        }

        let cases = [
            // Ultrathin Light: 50px, rgba(255,255,255,0.03) + rgba(255,255,255,0.07) -> tint a = 0.0979
            Case {
                m: Material::Ultrathin,
                dark: false,
                blur: 50.0,
                fills: &[(1.0, 1.0, 1.0, 0.03), (1.0, 1.0, 1.0, 0.07)],
                tint: ([1.0, 1.0, 1.0], 0.0979),
            },
            // Ultrathin Dark: 50px, rgba(0,0,0,0.02)
            Case {
                m: Material::Ultrathin,
                dark: true,
                blur: 50.0,
                fills: &[(0.0, 0.0, 0.0, 0.02)],
                tint: ([0.0, 0.0, 0.0], 0.02),
            },
            // Thin Light: 50px, rgba(255,255,255,0.4) + rgba(255,255,255,0.05) -> tint a = 0.43
            Case {
                m: Material::Thin,
                dark: false,
                blur: 50.0,
                fills: &[(1.0, 1.0, 1.0, 0.4), (1.0, 1.0, 1.0, 0.05)],
                tint: ([1.0, 1.0, 1.0], 0.43),
            },
            // Thin Dark: 50px, rgba(0,0,0,0.26)
            Case {
                m: Material::Thin,
                dark: true,
                blur: 50.0,
                fills: &[(0.0, 0.0, 0.0, 0.26)],
                tint: ([0.0, 0.0, 0.0], 0.26),
            },
            // Regular Light: 50px, rgba(255,255,255,0.6) + rgba(255,255,255,0.25) -> tint a = 0.70
            Case {
                m: Material::Regular,
                dark: false,
                blur: 50.0,
                fills: &[(1.0, 1.0, 1.0, 0.6), (1.0, 1.0, 1.0, 0.25)],
                tint: ([1.0, 1.0, 1.0], 0.70),
            },
            // Regular Dark: 50px, rgba(0,0,0,0.41)
            Case {
                m: Material::Regular,
                dark: true,
                blur: 50.0,
                fills: &[(0.0, 0.0, 0.0, 0.41)],
                tint: ([0.0, 0.0, 0.0], 0.41),
            },
            // Thick Light: 50px, rgba(255,255,255,0.84) + rgba(255,255,255,0.34) -> tint a = 0.8944
            Case {
                m: Material::Thick,
                dark: false,
                blur: 50.0,
                fills: &[(1.0, 1.0, 1.0, 0.84), (1.0, 1.0, 1.0, 0.34)],
                tint: ([1.0, 1.0, 1.0], 0.8944),
            },
            // Thick Dark: 50px, rgba(0,0,0,0.6)
            Case {
                m: Material::Thick,
                dark: true,
                blur: 50.0,
                fills: &[(0.0, 0.0, 0.0, 0.6)],
                tint: ([0.0, 0.0, 0.0], 0.6),
            },
            // Chrome Light: 25px, rgba(255,255,255,0.75)
            Case {
                m: Material::Chrome,
                dark: false,
                blur: 25.0,
                fills: &[(1.0, 1.0, 1.0, 0.75)],
                tint: ([1.0, 1.0, 1.0], 0.75),
            },
            // Chrome Dark: 50px, #7C7C7C + rgba(28,28,28,0.9) -> tint [37.6/255; 3], a=1.0
            Case {
                m: Material::Chrome,
                dark: true,
                blur: 50.0,
                fills: &[
                    (124.0 / 255.0, 124.0 / 255.0, 124.0 / 255.0, 1.0),
                    (28.0 / 255.0, 28.0 / 255.0, 28.0 / 255.0, 0.9),
                ],
                tint: ([37.6 / 255.0, 37.6 / 255.0, 37.6 / 255.0], 1.0),
            },
        ];

        for c in cases {
            let spec = material(c.m, c.dark);
            assert_eq!(spec.blur_px, c.blur, "{:?} dark={}", c.m, c.dark);
            assert_eq!(spec.fills.len(), c.fills.len(), "{:?} dark={} layers count", c.m, c.dark);
            for (i, &(r, g, b, a)) in c.fills.iter().enumerate() {
                assert_color_close(spec.fills[i], rgba(r, g, b, a), &format!("{:?} dark={} layer {i}", c.m, c.dark));
            }
            let (tint_rgb, tint_a) = body_tint(c.m, c.dark);
            assert_rgb_close(tint_rgb, c.tint.0, &format!("{:?} dark={} body_tint rgb", c.m, c.dark));
            assert!((tint_a - c.tint.1).abs() < 1e-4, "{:?} dark={} body_tint a: {} != {}", c.m, c.dark, tint_a, c.tint.1);
        }

        // Инвариант монотонного роста плотности
        for dark in [false, true] {
            let a_ultra = body_tint(Material::Ultrathin, dark).1;
            let a_thin = body_tint(Material::Thin, dark).1;
            let a_reg = body_tint(Material::Regular, dark).1;
            let a_thick = body_tint(Material::Thick, dark).1;

            assert!(a_ultra < a_thin);
            assert!(a_thin < a_reg);
            assert!(a_reg < a_thick);
        }
    }

    #[test]
    fn test_all_12_system_colors_exact_table() {
        let table = [
            (SysColor::Red,    0xFF383C, 0xFF4245),
            (SysColor::Orange, 0xFF8D28, 0xFF9230),
            (SysColor::Yellow, 0xFFCC00, 0xFFD600),
            (SysColor::Green,  0x34C759, 0x30D158),
            (SysColor::Mint,   0x00C8B3, 0x00DAC3),
            (SysColor::Teal,   0x00C3D0, 0x00D2E0),
            (SysColor::Cyan,   0x00C0E8, 0x3CD3FE),
            (SysColor::Blue,   0x0088FF, 0x0091FF),
            (SysColor::Indigo, 0x6155F5, 0x6D7CFF),
            (SysColor::Purple, 0xCB30E0, 0xDB34F2),
            (SysColor::Pink,   0xFF2D55, 0xFF375F),
            (SysColor::Brown,  0xAC7F5E, 0xB78A66),
        ];

        for (c, light_hex, dark_hex) in table {
            assert_eq!(sys(c, false), hex(light_hex), "{c:?} Light");
            assert_eq!(sys(c, true), hex(dark_hex), "{c:?} Dark");
        }
    }

    #[test]
    fn test_all_8_grays_exact_table() {
        let table = [
            (GrayToken::Black, 0x000000, 0x000000),
            (GrayToken::Gray,  0x8E8E93, 0x8E8E93),
            (GrayToken::Gray2, 0xAEAEB2, 0x636366),
            (GrayToken::Gray3, 0xC7C7CC, 0x48484A),
            (GrayToken::Gray4, 0xD1D1D6, 0x3A3A3C),
            (GrayToken::Gray5, 0xE5E5EA, 0x2C2C2E),
            (GrayToken::Gray6, 0xF2F2F7, 0x1C1C1E),
            (GrayToken::White, 0xFFFFFF, 0xFFFFFF),
        ];

        for (g, light_hex, dark_hex) in table {
            assert_eq!(gray(g, false), hex(light_hex), "{g:?} Light");
            assert_eq!(gray(g, true), hex(dark_hex), "{g:?} Dark");
        }
    }

    #[test]
    fn test_labels_rgb_and_alpha_exact() {
        // Labels Light:
        // Primary: #000000
        assert_color_close(label_primary(false), rgba(0.0, 0.0, 0.0, 1.0), "Label Primary Light");
        // Secondary: rgba(60, 60, 67, 0.6)
        assert_color_close(label_secondary(false), rgba(60.0 / 255.0, 60.0 / 255.0, 67.0 / 255.0, 0.6), "Label Secondary Light");
        // Tertiary: rgba(60, 60, 67, 0.3)
        assert_color_close(label_tertiary(false), rgba(60.0 / 255.0, 60.0 / 255.0, 67.0 / 255.0, 0.3), "Label Tertiary Light");
        // Quaternary: rgba(60, 60, 67, 0.18)
        assert_color_close(label_quaternary(false), rgba(60.0 / 255.0, 60.0 / 255.0, 67.0 / 255.0, 0.18), "Label Quaternary Light");

        // Labels Dark:
        // Primary: #FFFFFF
        assert_color_close(label_primary(true), rgba(1.0, 1.0, 1.0, 1.0), "Label Primary Dark");
        // Secondary: rgba(235, 235, 245, 0.6)
        assert_color_close(label_secondary(true), rgba(235.0 / 255.0, 235.0 / 255.0, 245.0 / 255.0, 0.6), "Label Secondary Dark");
        // Tertiary: rgba(235, 235, 245, 0.3)
        assert_color_close(label_tertiary(true), rgba(235.0 / 255.0, 235.0 / 255.0, 245.0 / 255.0, 0.3), "Label Tertiary Dark");
        // Quaternary: rgba(235, 235, 245, 0.16)
        assert_color_close(label_quaternary(true), rgba(235.0 / 255.0, 235.0 / 255.0, 245.0 / 255.0, 0.16), "Label Quaternary Dark");
    }

    #[test]
    fn test_fills_rgb_and_alpha_exact() {
        // Fills Light:
        // Primary: rgba(120, 120, 128, 0.20)
        assert_color_close(fill_primary(false), rgba(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.20), "Fill Primary Light");
        // Secondary: rgba(120, 120, 128, 0.16)
        assert_color_close(fill_secondary(false), rgba(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.16), "Fill Secondary Light");
        // Tertiary: rgba(118, 118, 128, 0.12)
        assert_color_close(fill_tertiary(false), rgba(118.0 / 255.0, 118.0 / 255.0, 128.0 / 255.0, 0.12), "Fill Tertiary Light");
        // Quaternary: rgba(116, 116, 128, 0.08)
        assert_color_close(fill_quaternary(false), rgba(116.0 / 255.0, 116.0 / 255.0, 128.0 / 255.0, 0.08), "Fill Quaternary Light");

        // Fills Dark:
        // Primary: rgba(120, 120, 128, 0.36)
        assert_color_close(fill_primary(true), rgba(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.36), "Fill Primary Dark");
        // Secondary: rgba(120, 120, 128, 0.32)
        assert_color_close(fill_secondary(true), rgba(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.32), "Fill Secondary Dark");
        // Tertiary: rgba(118, 118, 128, 0.24)
        assert_color_close(fill_tertiary(true), rgba(118.0 / 255.0, 118.0 / 255.0, 128.0 / 255.0, 0.24), "Fill Tertiary Dark");
        // Quaternary: rgba(118, 118, 128, 0.18)
        assert_color_close(fill_quaternary(true), rgba(118.0 / 255.0, 118.0 / 255.0, 128.0 / 255.0, 0.18), "Fill Quaternary Dark");
    }

    #[test]
    fn test_separators_and_backgrounds_rgb_and_alpha_exact() {
        // Separators:
        assert_color_close(separator_opaque(false), hex_a(0xC6C6C8, 1.0), "Separator Opaque Light");
        assert_color_close(separator_opaque(true), hex_a(0x38383A, 1.0), "Separator Opaque Dark");
        assert_color_close(separator_non_opaque(false), rgba(0.0, 0.0, 0.0, 0.12), "Separator NonOpaque Light");
        assert_color_close(separator_non_opaque(true), rgba(1.0, 1.0, 1.0, 0.17), "Separator NonOpaque Dark");

        // Backgrounds:
        assert_color_close(bg_primary(false), hex_a(0xFFFFFF, 1.0), "Bg Primary Light");
        assert_color_close(bg_primary(true), hex_a(0x000000, 1.0), "Bg Primary Dark Base");
        assert_color_close(bg_secondary(false), hex_a(0xF2F2F7, 1.0), "Bg Secondary Light");
        assert_color_close(bg_secondary(true), hex_a(0x1C1C1E, 1.0), "Bg Secondary Dark Base");
        assert_color_close(bg_tertiary(false), hex_a(0xFFFFFF, 1.0), "Bg Tertiary Light");
        assert_color_close(bg_tertiary(true), hex_a(0x2C2C2E, 1.0), "Bg Tertiary Dark Base");
    }

    #[test]
    fn test_radii_and_shadow() {
        assert_eq!(R_LARGE, 34.0);
        assert_eq!(R_MEDIUM, 26.0);
        assert_eq!(R_SMALL_CAPSULE, 1000.0);
        assert_eq!(R_SMALL_CIRCLE, 296.0);
        assert_eq!(R_CLEAR, 24.0);
        assert_eq!(SEPARATOR_THICKNESS, 0.5);

        assert_eq!(GLASS_SHADOW.offset_x, 0.0);
        assert_eq!(GLASS_SHADOW.offset_y, 8.0);
        assert_eq!(GLASS_SHADOW.blur_radius, 40.0);
        assert_eq!(GLASS_SHADOW.color, rgba(0.0, 0.0, 0.0, 0.12));
    }

    #[test]
    fn test_typography_roles_and_scale() {
        let (b_size, b_line, b_weight, b_track) = type_spec(TextRole::Body);
        assert_eq!(b_size, 17.0);
        assert_eq!(b_line, 22.0);
        assert_eq!(b_weight, 400);
        assert_eq!(b_track, -0.0253);

        let (hl_size, hl_line, hl_weight, hl_track) = type_spec(TextRole::Headline);
        assert_eq!(hl_size, 17.0);
        assert_eq!(hl_line, 22.0);
        assert_eq!(hl_weight, 590);
        assert_eq!(hl_track, -0.0253);

        let (_, _, emp_weight, _) = type_spec_emphasized(TextRole::LargeTitle);
        assert_eq!(emp_weight, 700);

        let (_, _, emp_body_weight, _) = type_spec_emphasized(TextRole::Body);
        assert_eq!(emp_body_weight, 590);

        assert!((island_scale() - (9.0 / 17.0)).abs() < 1e-6);
        assert_eq!(island_size(TextRole::Body), 9.0);
    }
}
