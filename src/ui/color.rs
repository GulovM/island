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
/// Основной текст поверх свёрнутого острова (`face.rs`, `live.rs`): тело там всегда зафиксировано
/// чёрным независимо от темы/акцента (см. `glass::body_layers`), поэтому текст зафиксирован светлым.
/// Значение 0.945 (~#F1F1F1) даёт контраст WCAG ~18.6:1 против чёрного фона [0,0,0]; выбран чуть мягче
/// чистого белого 1.0, чтобы исключить светорассеяние (гало-эффект) и утомление глаз на тёмных дисплеях.
pub fn collapsed_txt() -> D2D1_COLOR_F {
    rgb(0.945, 0.945, 0.945)
}
/// Второстепенный текст и значки поверх свёрнутого острова (индикатор раскладки в `face.rs`, артист/теги
/// в `live.rs`). Значение 0.87 (~#DEDEDE) выбрано по практикам тёмных тем Apple/Windows 11
/// (TextFillColorSecondary): даёт ~83.3% линейной яркости от основного текста (в целевом диапазоне 75–90%)
/// и контраст WCAG ~15.6:1 против чёрного фона (с большим запасом превосходит норму AAA 7:1).
/// Ранее использовалось 0.79, что из-за гамма-нелинейности sRGB давало лишь ~66.5% линейной яркости,
/// из-за чего рядом с ярким временем индикатор раскладки воспринимался блёклым и расплывчатым.
/// При 0.87 размытость полностью уходит, при этом сохраняется оптическая иерархия «главный/второстепенный».
pub fn collapsed_dim() -> D2D1_COLOR_F {
    rgb(0.87, 0.87, 0.87)
}
/// Фон мелких плашек поверх свёрнутой формы (например, подложка кнопки закрытия уведомлений в `live.rs`).
/// Альфа 0.15 (15% белого на чёрном фоне) формирует видимую деликатную стеклянную подложку в состоянии
/// покоя (аналог tertiarySystemFill в iOS), согласуясь с общей шкалой плотности и оставляя достаточный
/// контрастный шаг при наведении курсора (до ~30% альфы).
pub fn collapsed_chip_bg() -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.15 }
}
/// Дорожка/«выключенный» штрих поверх свёрнутой формы (погашенные деления Wi-Fi в `face.rs`).
/// Альфа 0.25 (25% белого на чёрном фоне) критична для тонких линий толщиной 1.2 px:
/// прежнее значение 0.128 при субпиксельном размытии краёв штриха становилось неразличимым на чёрном.
/// 0.25 гарантирует чёткую считываемость неактивных дуг как направляющей шкалы, оставаясь ровно в 4 раза
/// темнее активного сигнала (контрастный шаг 4:1 относительно 1.0/0.945).
pub fn collapsed_track() -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.25 }
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

/// Яркость `Y` для APCA: простая степень 2.4 без кусочной линеаризации sRGB.
fn apca_y(c: [f32; 3]) -> f32 {
    let p = |v: f32| v.clamp(0.0, 1.0).powf(2.4);
    0.212_672_9 * p(c[0]) + 0.715_152_2 * p(c[1]) + 0.072_175 * p(c[2])
}

/// Перцептивный контраст APCA-W3 0.0.98G-4g: `Lc` текста `fg` на фоне `bg` (каналы 0..1).
/// Положительный для тёмного текста на светлом фоне, отрицательный для светлого на тёмном;
/// около ±106 у чёрного на белом и белого на чёрном.
pub fn apca_lc(fg: [f32; 3], bg: [f32; 3]) -> f32 {
    const NORM_BG: f32 = 0.56;
    const NORM_TXT: f32 = 0.57;
    const REV_TXT: f32 = 0.62;
    const REV_BG: f32 = 0.65;
    const SCALE_BOW: f32 = 1.14;
    const SCALE_WOB: f32 = -1.14;
    const LO_CLIP: f32 = 0.1;
    const LO_OFFSET: f32 = 0.027;
    const DELTA_Y_MIN: f32 = 0.0005;
    const BLK_THRS: f32 = 0.022;
    const BLK_CLMP: f32 = 1.414;
    let soft = |y: f32| if y > BLK_THRS { y } else { y + (BLK_THRS - y).powf(BLK_CLMP) };
    let (yt, yb) = (soft(apca_y(fg)), soft(apca_y(bg)));
    if (yb - yt).abs() < DELTA_Y_MIN {
        return 0.0;
    }
    let lc = if yb > yt {
        let s = (yb.powf(NORM_BG) - yt.powf(NORM_TXT)) * SCALE_BOW;
        if s < LO_CLIP { 0.0 } else { s - LO_OFFSET }
    } else {
        let s = (yt.powf(REV_TXT) - yb.powf(REV_BG)) * SCALE_WOB;
        if s > -LO_CLIP { 0.0 } else { s + LO_OFFSET }
    };
    lc * 100.0
}

const APCA_WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const APCA_DARK: [f32; 3] = [0.07, 0.07, 0.08];

fn min_lc_over(fg: [f32; 3], bgs: &[[f32; 3]]) -> f32 {
    bgs.iter().map(|&b| apca_lc(fg, b).abs()).fold(f32::INFINITY, f32::min)
}

/// Подтягивает светлоту `fg`, пока `|Lc|` на каждом из фонов `bgs` не достигнет `min_lc`: цвет
/// линейно смешивается с полюсом (белым или тёмным — тем, что контрастнее на этих фонах), минимальная
/// доля смешивания ищется за 8 шагов двоичного поиска. Если `fg` уже читается, он возвращается как есть.
pub fn ensure_apca(fg: [f32; 3], bgs: &[[f32; 3]], min_lc: f32) -> [f32; 3] {
    if bgs.is_empty() || min_lc_over(fg, bgs) >= min_lc {
        return fg;
    }
    let pole = if min_lc_over(APCA_WHITE, bgs) >= min_lc_over(APCA_DARK, bgs) { APCA_WHITE } else { APCA_DARK };
    let blend = |t: f32| [0, 1, 2].map(|i| fg[i] + (pole[i] - fg[i]) * t);
    if min_lc_over(pole, bgs) < min_lc {
        return pole;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..8 {
        let mid = (lo + hi) / 2.0;
        if min_lc_over(blend(mid), bgs) >= min_lc {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    blend(hi)
}

/// Надпись на сплошной заливке `fill`: белая или почти чёрная — что контрастнее. Насыщенным
/// системным цветам iOS (синий, красный, фиолетовый) достаётся белая, как в iOS; светлым
/// (жёлтый, мятный, белая заливка ползунка) — тёмная. При автоподборе контраста выбор делает APCA.
pub fn on_color(fill: D2D1_COLOR_F) -> D2D1_COLOR_F {
    if crate::theme::auto_contrast() {
        let bg = [fill.r, fill.g, fill.b];
        let dark = rgb(0.11, 0.11, 0.12);
        let d = apca_lc([dark.r, dark.g, dark.b], bg).abs();
        let w = apca_lc(APCA_WHITE, bg).abs();
        return if d > w { dark } else { WHITE };
    }
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
    fn apca_reference_vectors() {
        let (black, white) = ([0.0; 3], [1.0; 3]);
        let bow = apca_lc(black, white);
        let wob = apca_lc(white, black);
        assert!((bow - 106.0).abs() < 0.5, "{bow}");
        assert!((wob + 107.9).abs() < 0.5, "{wob}");
        assert_eq!(apca_lc(white, white), 0.0);
    }

    #[test]
    fn ensure_apca_reaches_thresholds() {
        // Уже читаемый цвет не меняется.
        assert_eq!(ensure_apca([1.0; 3], &[[0.05; 3]], 75.0), [1.0; 3]);
        // Серый на сером подтягивается до порога в обе стороны.
        for (fg, bg) in [([0.5; 3], [0.45; 3]), ([0.5; 3], [0.85; 3]), ([0.3, 0.4, 0.9], [0.2, 0.3, 0.8])] {
            for min in [30.0, 60.0, 75.0] {
                let out = ensure_apca(fg, &[bg], min);
                assert!(apca_lc(out, bg).abs() >= min - 0.01, "{fg:?}/{bg:?}/{min}: {}", apca_lc(out, bg));
            }
        }
    }

    #[test]
    fn on_color_picks_readable_pole() {
        assert!(on_color(rgb(0.95, 0.95, 0.5)).r < 0.2);
        assert!(on_color(rgb(0.1, 0.2, 0.7)).r > 0.9);
    }

    #[test]
    fn mix_endpoints() {
        let c = mix(track(), fg(), 1.0);
        assert!((c.r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_collapsed_colors_hierarchy_and_contrast() {
        let txt = collapsed_txt();
        let dim = collapsed_dim();
        let track = collapsed_track();
        let chip = collapsed_chip_bg();

        // Основной текст светлее второстепенного
        assert!(txt.r > dim.r && txt.g > dim.g && txt.b > dim.b);
        // Отношение линейной яркости sRGB Y(dim)/Y(txt) лежит в диапазоне 75–90%
        let lum = |c: f32| ((c + 0.055) / 1.055).powf(2.4);
        let ratio = lum(dim.r) / lum(txt.r);
        assert!((0.75..=0.90).contains(&ratio), "Luminance ratio {ratio} outside [0.75, 0.90]");
        // Контраст WCAG против чёрного фона (0.0): (Y + 0.05) / 0.05 > 7.0 (норма AAA)
        let wcag_dim = (lum(dim.r) + 0.05) / 0.05;
        assert!(wcag_dim >= 7.0, "WCAG contrast {wcag_dim} below AAA requirement");
        // Неактивная дорожка и плашка заметны, но не доминируют над текстом
        assert!((0.20..=0.30).contains(&track.a));
        assert!((0.12..=0.20).contains(&chip.a));
        assert!(track.a > chip.a);
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
