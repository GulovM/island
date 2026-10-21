//! Цветовые константы, служебные цвета темы и смешивание цветов.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

pub(super) const fn rgb(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}

pub const WHITE: D2D1_COLOR_F = rgb(1.0, 1.0, 1.0);
pub const GREEN: D2D1_COLOR_F = rgb(0.61, 0.80, 0.40);
pub const BAR_GREEN: D2D1_COLOR_F = rgb(0.55, 0.76, 0.29);
pub const VIOLET: D2D1_COLOR_F = rgb(0.78, 0.61, 0.91);
pub const ORANGE: D2D1_COLOR_F = rgb(0.95, 0.63, 0.14);
pub const PINK: D2D1_COLOR_F = rgb(0.91, 0.42, 0.60);
pub const RED: D2D1_COLOR_F = rgb(0.93, 0.36, 0.33);

// Цвета темы (см. `theme`): меняются на лету, поэтому это функции, а не константы.

/// Основной текст.
pub fn txt() -> D2D1_COLOR_F {
    crate::theme::get().text
}
/// Приглушённый текст и значки.
pub fn dim() -> D2D1_COLOR_F {
    crate::theme::get().dim
}
/// Текст прямо на теле острова, без карточки или плашки под ним: сквозь полупрозрачное тело виден фон,
/// и серый там теряется. Белый на тёмном острове, чёрный на светлом.
pub fn bare() -> D2D1_COLOR_F {
    crate::theme::get().fg
}
/// Карточки (нейтральная поверхность чуть светлее фона).
pub fn card() -> D2D1_COLOR_F {
    crate::theme::get().card
}
pub fn track() -> D2D1_COLOR_F {
    crate::theme::get().track
}
/// Волосяная граница карточек (прозрачная, когда стекло выключено).
pub fn hairline() -> D2D1_COLOR_F {
    let mut c = crate::theme::get().hairline;
    if !crate::theme::glass() {
        c.a = 0.0;
    }
    c
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

/// Проверяет, активен ли стиль палитры iOS.
pub fn is_ios_palette() -> bool {
    crate::theme::get().style == crate::theme::PaletteStyle::Ios
}

fn status_color(sys_color: crate::ios::SysColor, fallback: D2D1_COLOR_F) -> D2D1_COLOR_F {
    if is_ios_palette() {
        let p = crate::theme::get();
        let dark = !crate::theme::is_light([p.bg.r, p.bg.g, p.bg.b]);
        crate::ios::to_d2d(crate::ios::sys(sys_color, dark), 1.0)
    } else {
        fallback
    }
}

/// Смысловой цвет функции: у каждой своя краска, как у модулей Пункта управления и приложений iOS
/// (Wi-Fi и Bluetooth — синие, звук — розовый, ночной свет — оранжевый, календарь — красный…).
/// В стиле Windows всё красится акцентом `fallback`, как раньше.
pub fn hue(c: crate::ios::SysColor, fallback: D2D1_COLOR_F) -> D2D1_COLOR_F {
    status_color(c, fallback)
}

/// Относительная яркость по WCAG (с линеаризацией sRGB).
fn rel_luminance(c: D2D1_COLOR_F) -> f32 {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// Надпись на сплошной заливке `fill`: белая или почти чёрная — что контрастнее. Насыщенным
/// системным цветам iOS (синий, красный, фиолетовый) достаётся белая, как в iOS; светлым
/// (жёлтый, мятный, белая заливка ползунка) — тёмная.
pub fn on_color(fill: D2D1_COLOR_F) -> D2D1_COLOR_F {
    let l = rel_luminance(fill);
    // Порог чуть сдвинут в пользу белого (как в iOS): чёрный только если он заметно контрастнее.
    if (l + 0.05) / 0.05 > 1.35 * (1.05 / (l + 0.05)) {
        rgb(0.11, 0.11, 0.12)
    } else {
        WHITE
    }
}

/// Заливка ползунков громкости, яркости и перемотки: в стиле iOS на тёмном острове — белая, как
/// в Пункте управления; на светлом и в стиле Windows — акцент.
pub fn slider_fill(accent: D2D1_COLOR_F) -> D2D1_COLOR_F {
    let p = crate::theme::get();
    if is_ios_palette() && !crate::theme::is_light([p.bg.r, p.bg.g, p.bg.b]) {
        D2D1_COLOR_F { a: 0.96, ..WHITE }
    } else {
        accent
    }
}

/// Статусный зелёный (батарея, задачи):
/// в стиле Windows — константа `GREEN`,
/// в стиле iOS — `ios::sys(Green, dark)`.
pub fn status_green() -> D2D1_COLOR_F {
    status_color(crate::ios::SysColor::Green, GREEN)
}

/// Статусный зелёный полосы (график в stats):
/// в стиле Windows — константа `BAR_GREEN`,
/// в стиле iOS — `ios::sys(Green, dark)`.
pub fn status_bar_green() -> D2D1_COLOR_F {
    status_color(crate::ios::SysColor::Green, BAR_GREEN)
}

/// Статусный фиолетовый (GPU в stats):
/// в стиле Windows — константа `VIOLET`,
/// в стиле iOS — `ios::sys(Purple, dark)`.
pub fn status_violet() -> D2D1_COLOR_F {
    status_color(crate::ios::SysColor::Purple, VIOLET)
}

/// Статусный оранжевый (таймер, предупреждение о батарее, заметка в grid):
/// в стиле Windows — константа `ORANGE`,
/// в стиле iOS — `ios::sys(Orange, dark)`.
pub fn status_orange() -> D2D1_COLOR_F {
    status_color(crate::ios::SysColor::Orange, ORANGE)
}

/// Статусный розовый/красный (события календаря):
/// в стиле Windows — константа `PINK`,
/// в стиле iOS — `ios::sys(Red, dark)` (календарь в iOS красный).
pub fn status_pink() -> D2D1_COLOR_F {
    status_color(crate::ios::SysColor::Red, PINK)
}

/// Статусный красный (критический заряд батареи, индикатор удаления в grid):
/// в стиле Windows — константа `RED`,
/// в стиле iOS — `ios::sys(Red, dark)`.
pub fn status_red() -> D2D1_COLOR_F {
    status_color(crate::ios::SysColor::Red, RED)
}

/// Цвет переключателя во включённом состоянии:
/// в стиле палитры iOS — системный зелёный `ios::sys(Green, dark)`,
/// в стиле Windows — текущий цвет акцента.
pub fn switch_on_color(accent: D2D1_COLOR_F) -> D2D1_COLOR_F {
    status_color(crate::ios::SysColor::Green, accent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_endpoints() {
        let c = mix(track(), fg(), 1.0);
        assert!((c.r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_switch_on_color_styles() {
        use crate::theme::{derive, DEFAULT_ACCENT, DEFAULT_BG, PaletteStyle};
        // Для палитры Windows: возвращается переданный акцент
        let p_win = derive(DEFAULT_BG, DEFAULT_ACCENT, PaletteStyle::Windows);
        crate::theme::set(p_win);
        assert!(!is_ios_palette());
        let on_win = switch_on_color(p_win.accent);
        assert_eq!(on_win, p_win.accent);

        // Для палитры iOS: возвращается системный зелёный iOS
        let p_ios = derive(DEFAULT_BG, DEFAULT_ACCENT, PaletteStyle::Ios);
        crate::theme::set(p_ios);
        assert!(is_ios_palette());
        let on_ios = switch_on_color(p_ios.accent);
        let expected_green = crate::ios::to_d2d(crate::ios::sys(crate::ios::SysColor::Green, true), 1.0);
        assert_eq!(on_ios, expected_green);

        // Восстанавливаем палитру по умолчанию
        crate::theme::set(p_win);
    }

    #[test]
    fn test_status_colors_styles_and_themes() {
        use crate::theme::{derive, DEFAULT_ACCENT, DEFAULT_BG, PaletteStyle};
        let bg_dark = DEFAULT_BG;
        let bg_light = [0.95, 0.95, 0.95];

        // 1. Windows стиль, тёмная тема: константы
        let p_win_dark = derive(bg_dark, DEFAULT_ACCENT, PaletteStyle::Windows);
        crate::theme::set(p_win_dark);
        assert!(!is_ios_palette());
        assert_eq!(status_green(), GREEN);
        assert_eq!(status_bar_green(), BAR_GREEN);
        assert_eq!(status_violet(), VIOLET);
        assert_eq!(status_orange(), ORANGE);
        assert_eq!(status_pink(), PINK);
        assert_eq!(status_red(), RED);

        // 2. Windows стиль, светлая тема: константы
        let p_win_light = derive(bg_light, DEFAULT_ACCENT, PaletteStyle::Windows);
        crate::theme::set(p_win_light);
        assert!(!is_ios_palette());
        assert_eq!(status_green(), GREEN);
        assert_eq!(status_bar_green(), BAR_GREEN);
        assert_eq!(status_violet(), VIOLET);
        assert_eq!(status_orange(), ORANGE);
        assert_eq!(status_pink(), PINK);
        assert_eq!(status_red(), RED);

        // 3. iOS стиль, тёмная тема: ios::sys(..., true)
        let p_ios_dark = derive(bg_dark, DEFAULT_ACCENT, PaletteStyle::Ios);
        crate::theme::set(p_ios_dark);
        assert!(is_ios_palette());
        let to_d2d = |c| crate::ios::to_d2d(crate::ios::sys(c, true), 1.0);
        assert_eq!(status_green(), to_d2d(crate::ios::SysColor::Green));
        assert_eq!(status_bar_green(), to_d2d(crate::ios::SysColor::Green));
        assert_eq!(status_violet(), to_d2d(crate::ios::SysColor::Purple));
        assert_eq!(status_orange(), to_d2d(crate::ios::SysColor::Orange));
        assert_eq!(status_pink(), to_d2d(crate::ios::SysColor::Red));
        assert_eq!(status_red(), to_d2d(crate::ios::SysColor::Red));

        // 4. iOS стиль, светлая тема: ios::sys(..., false)
        let p_ios_light = derive(bg_light, DEFAULT_ACCENT, PaletteStyle::Ios);
        crate::theme::set(p_ios_light);
        assert!(is_ios_palette());
        let to_d2d_l = |c| crate::ios::to_d2d(crate::ios::sys(c, false), 1.0);
        assert_eq!(status_green(), to_d2d_l(crate::ios::SysColor::Green));
        assert_eq!(status_bar_green(), to_d2d_l(crate::ios::SysColor::Green));
        assert_eq!(status_violet(), to_d2d_l(crate::ios::SysColor::Purple));
        assert_eq!(status_orange(), to_d2d_l(crate::ios::SysColor::Orange));
        assert_eq!(status_pink(), to_d2d_l(crate::ios::SysColor::Red));
        assert_eq!(status_red(), to_d2d_l(crate::ios::SysColor::Red));

        // Восстанавливаем палитру по умолчанию
        crate::theme::set(p_win_dark);
    }
}
