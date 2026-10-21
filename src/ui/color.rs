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

/// Цвет переключателя во включённом состоянии:
/// в стиле палитры iOS — системный зелёный `ios::sys(Green, dark)`,
/// в стиле Windows — текущий цвет акцента.
pub fn switch_on_color(accent: D2D1_COLOR_F) -> D2D1_COLOR_F {
    let p = crate::theme::get();
    if is_ios_palette() {
        let dark = !crate::theme::is_light([p.bg.r, p.bg.g, p.bg.b]);
        let green = crate::ios::sys(crate::ios::SysColor::Green, dark);
        crate::ios::to_d2d(green, 1.0)
    } else {
        accent
    }
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
}
