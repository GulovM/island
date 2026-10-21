//! Стекло острова без Win32: слои тела острова (тинт, осветление, кромка, тень) и
//! точный контур окна-подложки с размытием. Всё — чистые функции, их проверяют тесты.

use std::f32::consts::{FRAC_PI_2, PI};

use crate::config::Material;
use crate::ios;
use crate::theme;

/// Самое прозрачное тело при настоящем размытии под островом (0% на ползунке): чёрная подкладка 41%,
/// как у материала «Regular» (тёмный) в iOS 26 UI Kit — `backdrop-blur 50px` + `rgba(0, 0, 0, 0.41)`.
#[cfg(test)]
pub const MIN_BODY_ALPHA: f32 = 0.41;
/// Без размытия (в Windows выключены «Эффекты прозрачности» или blur-behind недоступен) фон за телом
/// остаётся чётким, поэтому предел плотнее: иначе текст не читается. Подбирается на экране.
pub const MIN_BODY_ALPHA_NO_BLUR: f32 = 0.60;

/// Фактическая непрозрачность тела острова. `translucent` — стекло включено (`cfg.glass`),
/// `min_alpha` — нижний предел непрозрачности (из спецификации материала и наличия размытия),
/// `opacity` — ползунок 0–100 (0 → min_alpha, 100 → 1.0),
/// `p` — прогресс раскрытия, `k` — 1 − «полоска». Свёрнутый остров и полоска всегда сплошные.
pub fn body_alpha(translucent: bool, min_alpha: f32, opacity: f32, p: f32, k: f32) -> f32 {
    if !translucent || p <= 0.001 {
        return 1.0;
    }
    let min = min_alpha.clamp(0.0, 1.0);
    let base = min + (1.0 - min) * (opacity / 100.0).clamp(0.0, 1.0);
    1.0 - (1.0 - base) * p.clamp(0.0, 1.0) * k.clamp(0.0, 1.0)
}

/// Параметры осветления сверху (sheen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheenLayer {
    /// Цвет осветления (обычно белый).
    pub color: [f32; 3],
    /// Прозрачность у верхнего края острова (0.0..1.0).
    pub alpha: f32,
    /// Доля высоты острова, на которую распространяется осветление (0.0..1.0).
    pub height_fraction: f32,
}

/// Параметры зеркальной кромки и бликов (rim).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RimLayer {
    /// Толщина линии кромки в логических пикселях (подобрано).
    pub width: f32,
    /// Цвет блика слева-сверху (подобрано).
    pub hi_color: [f32; 3],
    /// Прозрачность блика слева-сверху (подобрано).
    pub hi_alpha: f32,
    /// Цвет средней части кромки (подобрано).
    pub mid_color: [f32; 3],
    /// Прозрачность средней части кромки (подобрано).
    pub mid_alpha: f32,
    /// Цвет дуги справа-снизу (подобрано).
    pub lo_color: [f32; 3],
    /// Прозрачность дуги справа-снизу (подобрано).
    pub lo_alpha: f32,
    /// Тонкая внутренняя подсветка по верхней половине (подобрано).
    pub inner_highlight_alpha: f32,
}

/// Параметры внешней тени стеклянного тела (shadow).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowLayer {
    /// Цвет тени (чёрный).
    pub color: [f32; 3],
    /// Прозрачность тени (с учётом p · k и включённого размытия).
    pub alpha: f32,
    /// Смещение вниз в логических пикселях (из ios::GLASS_SHADOW).
    pub offset_y: f32,
    /// Радиус размытия в логических пикселях (из ios::GLASS_SHADOW).
    pub blur_radius: f32,
}

/// Полный набор визуальных слоёв стеклянного тела острова.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyLayers {
    /// RGB тинта тела острова.
    pub tint_color: [f32; 3],
    /// Итоговая альфа тинта тела острова.
    pub tint_alpha: f32,
    /// Минимальная альфа для данного материала и режима размытия.
    pub min_alpha: f32,
    /// Слой осветления сверху (sheen).
    pub sheen: SheenLayer,
    /// Слой кромки и бликов (rim).
    pub rim: RimLayer,
    /// Слой мягкой тени (shadow).
    pub shadow: ShadowLayer,
}

/// Базовые параметры кромки `(hi_alpha, mid_alpha, lo_color, lo_alpha)` для тёмной/светлой темы.
pub fn rim_alphas(dark: bool) -> (f32, f32, [f32; 3], f32) {
    if dark {
        (0.50, 0.05, [1.0, 1.0, 1.0], 0.22) // подобрано
    } else {
        (0.70, 0.08, [0.0, 0.0, 0.0], 0.10) // подобрано
    }
}

/// Базовая непрозрачность осветления сверху (sheen) для тёмной/светлой темы.
pub fn sheen_alpha(dark: bool) -> f32 {
    if dark { 0.07 } else { 0.30 } // подобрано
}

/// Вычисляет все визуальные слои тела острова: тинт, осветление, кромку и тень.
/// Чистая функция без вызовов Win32 API.
#[allow(clippy::too_many_arguments)]
pub fn body_layers(
    material: Material,
    bg: [f32; 3],
    translucent: bool,
    blur: bool,
    opacity: f32,
    p: f32,
    k: f32,
) -> BodyLayers {
    let dark = !theme::is_light(bg);
    let (_mat_rgb, base_alpha) = ios::body_tint(material, dark);

    // Правило альфы: минимум = итоговая альфа материала; без blur — минимум не ниже MIN_BODY_ALPHA_NO_BLUR.
    let min_alpha = if blur {
        base_alpha
    } else {
        base_alpha.max(MIN_BODY_ALPHA_NO_BLUR)
    };

    let tint_alpha = body_alpha(translucent, min_alpha, opacity, p, k);

    // Цвет RGB тинта: всегда цвет фона острова (bg) для обоих стилей.
    // При сжатии в полоску (k < 1) фон пропорционально темнеет к чистому чёрному.
    let tint_color = [bg[0] * k, bg[1] * k, bg[2] * k];

    // Множитель стеклянных эффектов (осветление сверху, кромка, тень).
    // Полоска (k = 0), свёрнутый остров (p = 0) или выключенное стекло гасят эффекты в ноль,
    // сохраняя прежний вид. Без «Эффектов прозрачности» (blur = false) стекло остаётся стеклом
    // (кромка, блик, тень сохраняются, но тело плотнее).
    let glass_factor = if translucent {
        (p * k).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // 1. Осветление сверху (sheen)
    let base_sheen_a = sheen_alpha(dark);
    let sheen = SheenLayer {
        color: [1.0, 1.0, 1.0],
        alpha: base_sheen_a * glass_factor,
        height_fraction: 0.45, // подобрано
    };

    // 2. Зеркальная кромка (rim): блик слева-сверху и дуга справа-снизу
    let (rim_hi_a, rim_mid_a, rim_lo_col, rim_lo_a) = rim_alphas(dark);
    let rim = RimLayer {
        width: 1.2, // подобрано
        hi_color: [1.0, 1.0, 1.0],
        hi_alpha: rim_hi_a * glass_factor,
        mid_color: [1.0, 1.0, 1.0],
        mid_alpha: rim_mid_a * glass_factor,
        lo_color: rim_lo_col,
        lo_alpha: rim_lo_a * glass_factor,
        inner_highlight_alpha: (if dark { 0.12 } else { 0.20 }) * glass_factor, // подобрано
    };

    // 3. Тень стекла: ios::GLASS_SHADOW (0 8 40 rgba(0,0,0,0.12))
    let shadow = ShadowLayer {
        color: [ios::GLASS_SHADOW.color.r, ios::GLASS_SHADOW.color.g, ios::GLASS_SHADOW.color.b],
        alpha: ios::GLASS_SHADOW.color.a * glass_factor,
        offset_y: ios::GLASS_SHADOW.offset_y,
        blur_radius: ios::GLASS_SHADOW.blur_radius,
    };

    BodyLayers {
        tint_color,
        tint_alpha,
        min_alpha,
        sheen,
        rim,
        shadow,
    }
}

/// Приближение дополнительной функции ошибок erfc(x) = 1 - erf(x)
/// по формуле Абрамовица–Стегуна 7.1.26 с максимальной абсолютной погрешностью <= 1.5e-7.
/// Не требует внешних крейтов.
#[allow(clippy::excessive_precision)]
pub fn erfc(x: f32) -> f32 {
    if x < 0.0 {
        return 2.0 - erfc(-x);
    }
    const P: f32 = 0.3275911;
    const A1: f32 = 0.254829592;
    const A2: f32 = -0.284496736;
    const A3: f32 = 1.421413741;
    const A4: f32 = -1.453152027;
    const A5: f32 = 1.061405429;

    let t = 1.0 / (1.0 + P * x);
    let poly = ((((A5 * t + A4) * t + A3) * t + A2) * t + A1) * t;
    poly * (-x * x).exp()
}

/// Вычисляет кольца мягкой тени тела острова с плавным гауссовым профилем (Задача T11).
/// Возвращает вектор пар `(grow_px, alpha)` от тела наружу:
/// `grow_px` равномерно от `blur_px / n` до `blur_px`, `n >= 24`.
///
/// Целевая накопленная непрозрачность на расстоянии `d` от тела:
/// гауссов профиль размытия полуплоскости (erfc) со `sigma = blur_px / 2.5`.
/// Профиль нормирован так, чтобы на границе тела (d = 0) накопленная непрозрачность
/// составляла `peak_alpha` (как в ios::GLASS_SHADOW):
/// `A(d) = peak_alpha · erfc(d / (sigma · sqrt(2)))`.
///
/// Так как заливки в Direct2D накладываются мультипликативно (1 - prod(1 - a_i)),
/// альфы вычисляются изнутри наружу с конца:
/// `a_N = A(d_N)` для самого внешнего кольца,
/// `a_k = 1 - (1 - A(d_k)) / (1 - A(d_{k+1}))` для внутренних колец,
/// а для внутреннего края у тела (d = 0) используется `A(0) = peak_alpha`,
/// что обеспечивает точное совпадение накопленной непрозрачности у тела с `peak_alpha`.
pub fn shadow_rings(blur_px: f32, peak_alpha: f32, n: usize) -> Vec<(f32, f32)> {
    if n == 0 || blur_px <= 0.0 || peak_alpha <= 0.0 {
        return Vec::new();
    }
    let peak_alpha = peak_alpha.clamp(0.0, 1.0);
    let sigma = blur_px / 2.5;
    let inv_sigma_sqrt2 = 1.0 / (sigma * std::f32::consts::SQRT_2);

    let a_target = |d: f32| -> f32 {
        (peak_alpha * erfc(d * inv_sigma_sqrt2)).clamp(0.0, 1.0)
    };

    let mut rings = Vec::with_capacity(n);
    let mut alphas = vec![0.0f32; n];

    let d = |k: usize| -> f32 {
        k as f32 * blur_px / n as f32
    };

    // Внешнее кольцо (k = n, индекс n - 1)
    alphas[n - 1] = a_target(d(n));

    // Внутренние кольца с конца от n - 2 вниз до 0.
    // При k = 0 (самое первое кольцо у тела) целевая непрозрачность берётся при d = 0,
    // чтобы накопленная непрозрачность на теле в точности равнялась peak_alpha.
    for k in (0..n - 1).rev() {
        let a_curr = if k == 0 { a_target(0.0) } else { a_target(d(k)) };
        let a_next = a_target(d(k + 1));
        let one_minus_next = (1.0 - a_next).max(1e-7);
        let ratio = ((1.0 - a_curr) / one_minus_next).clamp(0.0, 1.0);
        alphas[k] = (1.0 - ratio).clamp(0.0, 1.0);
    }

    for (i, &alpha) in alphas.iter().enumerate() {
        let grow = d(i + 1);
        rings.push((grow, alpha));
    }

    rings
}

/// Контур тела острова (как в `body_path`: плоский верх на краю экрана, вогнутые «ушки» радиуса `ear`,
/// скруглённые снизу углы радиуса `r`), сдвинутый внутрь на `inset` px; точки по часовой стрелке в
/// координатах окна острова. Подложка с размытием берёт именно этот контур, поэтому форма и углы у неё
/// те же, что у тела (прежняя подложка была прямоугольником и не повторяла «ушки»).
/// Верх не сдвигается: он упирается в край экрана. Дуги разбиты на отрезки по радиусу.
pub fn body_outline(x0: f32, aw: f32, ah: f32, ear: f32, r: f32, inset: f32) -> Vec<(f32, f32)> {
    let i = inset.max(0.0);
    let (l, rt, bt) = (x0 + i, x0 + aw - i, ah - i);
    let rr = (r - i).max(0.0).min((rt - l) / 2.0).min(bt.max(0.0));
    let ear = ear.max(0.0);
    let ring = ear + i;
    // Угол, на котором дуга «ушка» (радиус `ring` вокруг (x0 − ear, ear)) выходит на верхний край.
    let a0 = if ring > 0.0 { -(ear / ring).clamp(0.0, 1.0).asin() } else { 0.0 };
    let steps = |radius: f32| ((radius * 1.5).ceil() as usize).clamp(4, 32);
    let mut pts = Vec::new();
    let arc = |pts: &mut Vec<(f32, f32)>, cx: f32, cy: f32, radius: f32, from: f32, to: f32| {
        let n = steps(radius);
        for s in 0..=n {
            let a = from + (to - from) * s as f32 / n as f32;
            pts.push((cx + radius * a.cos(), cy + radius * a.sin()));
        }
    };

    if ear > 0.0 {
        arc(&mut pts, x0 - ear, ear, ring, a0, 0.0);
    } else {
        pts.push((l, 0.0));
    }
    if rr > 0.0 {
        arc(&mut pts, l + rr, bt - rr, rr, PI, FRAC_PI_2);
        arc(&mut pts, rt - rr, bt - rr, rr, FRAC_PI_2, 0.0);
    } else {
        pts.push((l, bt));
        pts.push((rt, bt));
    }
    if ear > 0.0 {
        arc(&mut pts, x0 + aw + ear, ear, ring, PI, PI - a0);
    } else {
        pts.push((rt, 0.0));
    }
    pts
}

/// Прямоугольник подложки размытия для Windows 11 (DWM-скругление без `SetWindowRgn`).
/// Подложка целиком вписана в тело острова: без «ушек», со сдвигом внутрь на `inset` физических px,
/// чтобы скруглённый угол радиуса `r_dwm` не выпирал за нижние углы тела радиуса `r_body`.
/// Верх упирается в верхний край окна (`y = 0`), без отступа.
/// Возвращает `(left_off, width, height, inset)` в целых пикселях окна:
/// `left_off` — смещение левого края от начала окна острова,
/// `width` — ширина подложки,
/// `height` — высота подложки от верхнего края окна (y = 0),
/// `inset` — отступ с краёв.
pub fn blur_plate(x0: f32, aw: f32, ah: f32, r_body: f32, r_dwm: f32) -> (i32, i32, i32, i32) {
    let diff = (r_body - r_dwm).max(0.0);
    let inset = (0.2929 * diff).ceil() as i32 + 1;
    let left_off = (x0 + inset as f32).round() as i32;
    let width = ((aw - 2.0 * inset as f32).round() as i32).max(1);
    let height = ((ah - inset as f32).round() as i32).max(1);
    (left_off, width, height, inset)
}

/// Контур подложки в целых физических пикселях: рамка окна и точки региона относительно её левого верха.
#[derive(Clone, PartialEq, Debug)]
pub struct BlurShape {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub points: Vec<(i32, i32)>,
}

impl BlurShape {
    /// Окружение контура в пикселях: ширина и высота окна, точки региона от его левого верха.
    /// Округление до целых делает форму устойчивой к дробным колебаниям пружин.
    pub fn from_outline(outline: &[(f32, f32)]) -> Self {
        let ints: Vec<(i32, i32)> = outline.iter().map(|&(x, y)| (x.round() as i32, y.round() as i32)).collect();
        let (mut l, mut t, mut rt, mut bt) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(x, y) in &ints {
            l = l.min(x);
            t = t.min(y);
            rt = rt.max(x);
            bt = bt.max(y);
        }
        if ints.is_empty() {
            return Self { left: 0, top: 0, width: 1, height: 1, points: Vec::new() };
        }
        let points = ints.iter().map(|&(x, y)| (x - l, y - t)).collect();
        Self { left: l, top: t, width: (rt - l).max(1), height: (bt - t).max(1), points }
    }

    /// Отпечаток формы для `blur_key`: три числа вместо сравнения всех точек.
    pub fn signature(&self) -> i32 {
        self.points.iter().fold(self.points.len() as u32, |h, &(x, y)| {
            h.wrapping_mul(0x0100_0193) ^ (x as u32).wrapping_mul(31).wrapping_add(y as u32)
        }) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Точка внутри тела: плоский верх, «ушки» по бокам и скруглённые снизу углы.
    fn inside_body(px: f32, py: f32, x0: f32, aw: f32, ah: f32, ear: f32, r: f32) -> bool {
        const TOL: f32 = 0.05;
        if py < -TOL || py > ah + TOL {
            return false;
        }
        if px < x0 - TOL {
            return px >= x0 - ear - TOL && py <= ear + TOL && (px - (x0 - ear)).hypot(py - ear) >= ear - TOL;
        }
        if px > x0 + aw + TOL {
            return px <= x0 + aw + ear + TOL && py <= ear + TOL && (px - (x0 + aw + ear)).hypot(py - ear) >= ear - TOL;
        }
        let cx = px.clamp(x0 + r, x0 + aw - r);
        let cy = py.min(ah - r);
        (px - cx).hypot(py - cy) <= r + TOL
    }

    fn cases() -> Vec<(f32, f32, f32, f32, f32)> {
        let mut v = Vec::new();
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            for (w, h) in [(180.0f32, 24.0f32), (200.0, 40.0), (420.0, 112.0), (430.0, 282.0), (620.0, 476.0), (333.3, 151.7)] {
                for r in [0.0f32, 1.0, 5.0, 10.08, 14.0, 22.5, 30.0] {
                    for x0 in [0.0f32, 17.3, 170.5] {
                        let (aw, ah) = (w * scale, h * scale);
                        let ear = (8.0 * scale * (ah / (24.0 * scale)).min(1.0)).min(ah);
                        let ear = if ear < 0.5 { 0.0 } else { ear };
                        let r = (r * scale).min(aw / 2.0).min(ah - ear).max(0.0);
                        v.push((x0 + ear, aw, ah, ear, r));
                    }
                }
            }
        }
        v
    }

    #[test]
    fn outline_lies_inside_the_body() {
        for (x0, aw, ah, ear, r) in cases() {
            for inset in [0.0f32, 1.0, 2.0] {
                let pts = body_outline(x0, aw, ah, ear, r, inset);
                assert!(pts.len() >= 4);
                for (px, py) in pts {
                    assert!(
                        inside_body(px, py, x0, aw, ah, ear, r),
                        "x0={x0} aw={aw} ah={ah} ear={ear} r={r} inset={inset}: точка ({px}, {py})"
                    );
                }
            }
        }
    }

    #[test]
    fn outline_follows_the_ears_and_the_corners() {
        // Верх контура упирается в край экрана и доходит до внешней границы «ушек».
        let (x0, aw, ah, ear, r) = (50.0, 400.0, 200.0, 8.0, 14.0);
        let pts = body_outline(x0, aw, ah, ear, r, 0.0);
        let min_x = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min);
        let max_x = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        let min_y = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        let max_y = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        assert!((min_x - (x0 - ear)).abs() < 1e-3 && (max_x - (x0 + aw + ear)).abs() < 1e-3);
        assert!(min_y.abs() < 1e-3 && (max_y - ah).abs() < 1e-3);
        // В нижних углах нет острых точек: ближайшая к углу точка контура отстоит от него на радиус дуги.
        let corner = pts.iter().map(|p| (p.0 - x0).hypot(p.1 - ah)).fold(f32::MAX, f32::min);
        assert!(corner > r * 0.3, "угол острый: {corner}");
        // Со сдвигом внутрь контур строго уже: боковые стороны уходят внутрь, а не наружу.
        let inner = body_outline(x0, aw, ah, ear, r, 2.0);
        let (ix, ax) = (
            inner.iter().map(|p| p.0).fold(f32::MAX, f32::min),
            inner.iter().map(|p| p.0).fold(f32::MIN, f32::max),
        );
        assert!(ix >= x0 - ear - 1e-3 && ax <= x0 + aw + ear + 1e-3);
        assert!(inner.iter().map(|p| p.1).fold(f32::MIN, f32::max) <= ah - 2.0 + 1e-3);
    }

    #[test]
    fn zero_radius_and_no_ears_work() {
        let pts = body_outline(10.0, 300.0, 150.0, 0.0, 0.0, 1.0);
        assert!(pts.contains(&(11.0, 0.0)) && pts.contains(&(309.0, 149.0)));
        // Всё нулевое не падает и не даёт пустого контура.
        assert!(body_outline(0.0, 1.0, 1.0, 0.0, 0.0, 0.0).len() >= 2);
    }

    #[test]
    fn shape_is_stable_against_spring_jitter() {
        let a = BlurShape::from_outline(&body_outline(80.0, 600.0, 400.0, 8.0, 16.8, 1.0));
        for d in [-0.004f32, -0.0005, 0.0005, 0.004] {
            let b = BlurShape::from_outline(&body_outline(80.0 + d, 600.0 + d, 400.0 + d, 8.0, 16.8 + d, 1.0));
            assert_eq!(a, b, "сдвиг {d}");
            assert_eq!(a.signature(), b.signature());
        }
        // Другая форма — другой отпечаток; точки лежат внутри рамки окна.
        let c = BlurShape::from_outline(&body_outline(80.0, 600.0, 400.0, 8.0, 30.0, 1.0));
        assert_ne!(a.signature(), c.signature());
        assert!(a.points.iter().all(|&(x, y)| (0..=a.width).contains(&x) && (0..=a.height).contains(&y)));
    }

    #[test]
    fn body_alpha_rules() {
        // Без стекла и в свёрнутом виде — сплошное тело.
        assert_eq!(body_alpha(false, MIN_BODY_ALPHA, 0.0, 1.0, 1.0), 1.0);
        assert_eq!(body_alpha(true, MIN_BODY_ALPHA, 0.0, 0.0, 1.0), 1.0);
        // С размытием 0% — подкладка 41% как в Figma, 100% — сплошное тело.
        assert!((body_alpha(true, MIN_BODY_ALPHA, 0.0, 1.0, 1.0) - MIN_BODY_ALPHA).abs() < 1e-6);
        assert!((body_alpha(true, MIN_BODY_ALPHA, 100.0, 1.0, 1.0) - 1.0).abs() < 1e-6);
        // Без размытия предел плотнее, ползунок по-прежнему тянется до сплошного.
        assert!((body_alpha(true, MIN_BODY_ALPHA_NO_BLUR, 0.0, 1.0, 1.0) - MIN_BODY_ALPHA_NO_BLUR).abs() < 1e-6);
        assert!((body_alpha(true, MIN_BODY_ALPHA_NO_BLUR, 100.0, 1.0, 1.0) - 1.0).abs() < 1e-6);
        for min in [MIN_BODY_ALPHA, MIN_BODY_ALPHA_NO_BLUR] {
            let mut prev = 0.0;
            for o in 0..=100 {
                let a = body_alpha(true, min, o as f32, 1.0, 1.0);
                assert!(a >= min - 1e-6 && a > prev, "{min} {o}%: {a}");
                prev = a;
            }
            assert!(body_alpha(true, min, -50.0, 1.0, 1.0) >= min - 1e-6);
            // В полоске (k = 0) тело сплошное, в середине раскрытия — плотнее, чем раскрытое.
            assert_eq!(body_alpha(true, min, 0.0, 1.0, 0.0), 1.0);
            assert!(body_alpha(true, min, 0.0, 0.5, 1.0) > body_alpha(true, min, 0.0, 1.0, 1.0));
        }
    }

    #[test]
    fn test_all_materials_themes_styles_blur_matrix() {
        let materials = [
            Material::Ultrathin,
            Material::Thin,
            Material::Regular,
            Material::Thick,
            Material::Chrome,
        ];
        let themes = [
            (true, [0.05, 0.05, 0.05]),  // dark
            (false, [0.95, 0.95, 0.95]), // light
        ];

        for m in materials {
            for (dark, bg) in themes {
                let (_mat_rgb, mat_alpha) = ios::body_tint(m, dark);

                for blur in [true, false] {
                    let layers = body_layers(m, bg, true, blur, 0.0, 1.0, 1.0);

                    // Проверка минимальной альфы:
                    // Regular + dark + blur == 0.41 (MIN_BODY_ALPHA)
                    if m == Material::Regular && dark && blur {
                        assert!((layers.min_alpha - MIN_BODY_ALPHA).abs() < 1e-6, "Regular dark blur must be 0.41");
                    }
                    if m == Material::Regular && dark && !blur {
                        assert!((layers.min_alpha - MIN_BODY_ALPHA_NO_BLUR).abs() < 1e-6, "Regular dark no-blur must be 0.60");
                    }

                    if blur {
                        assert!((layers.min_alpha - mat_alpha).abs() < 1e-6);
                    } else {
                        assert!((layers.min_alpha - mat_alpha.max(MIN_BODY_ALPHA_NO_BLUR)).abs() < 1e-6);
                    }

                    // При opacity = 0, p = 1, k = 1: tint_alpha == min_alpha
                    assert!((layers.tint_alpha - layers.min_alpha).abs() < 1e-6);

                    // Проверка цвета тинта: всегда равен bg острова
                    assert_eq!(layers.tint_color, bg);

                    // При translucent слои активны независимо от blur (без blur стекло плотнее, но со слоями)
                    assert!(layers.sheen.alpha > 0.0);
                    assert!(layers.rim.hi_alpha > 0.0);
                    assert!(layers.shadow.alpha > 0.0);
                    assert_eq!(layers.shadow.offset_y, 8.0);
                    assert_eq!(layers.shadow.blur_radius, 40.0);

                    // При выключенном стекле (translucent = false) слои выключены
                    let off = body_layers(m, bg, false, blur, 0.0, 1.0, 1.0);
                    assert_eq!(off.sheen.alpha, 0.0);
                    assert_eq!(off.rim.hi_alpha, 0.0);
                    assert_eq!(off.shadow.alpha, 0.0);
                    assert_eq!(off.tint_color, bg);
                }
            }
        }
    }

    #[test]
    fn test_opacity_slider_monotonicity() {
        let materials = [
            Material::Ultrathin,
            Material::Thin,
            Material::Regular,
            Material::Thick,
            Material::Chrome,
        ];
        for m in materials {
            for dark in [true, false] {
                let bg = if dark { [0.05; 3] } else { [0.95; 3] };
                for blur in [true, false] {
                    let mut prev = 0.0;
                    for op in 0..=100 {
                        let layers = body_layers(m, bg, true, blur, op as f32, 1.0, 1.0);
                        assert!(layers.tint_alpha >= layers.min_alpha - 1e-6);
                        assert!(layers.tint_alpha <= 1.0 + 1e-6);
                        assert!(layers.tint_alpha >= prev, "opacity {}% not monotonic: {} < {}", op, layers.tint_alpha, prev);
                        prev = layers.tint_alpha;
                    }
                    // 100% всегда даёт 1.0
                    let full = body_layers(m, bg, true, blur, 100.0, 1.0, 1.0);
                    assert!((full.tint_alpha - 1.0).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn test_strip_and_collapsed_states() {
        let bg = [0.1, 0.1, 0.1];
        // Полоска (k = 0): сплошное тело (alpha = 1.0), эффекты погашены, цвет затемнён в 0
        let strip = body_layers(Material::Regular, bg, true, true, 50.0, 1.0, 0.0);
        assert_eq!(strip.tint_alpha, 1.0);
        assert_eq!(strip.sheen.alpha, 0.0);
        assert_eq!(strip.rim.hi_alpha, 0.0);
        assert_eq!(strip.shadow.alpha, 0.0);
        assert_eq!(strip.tint_color, [0.0, 0.0, 0.0]);

        // Свёрнутое состояние (p = 0): сплошное тело (alpha = 1.0), эффекты погашены
        let collapsed = body_layers(Material::Regular, bg, true, true, 50.0, 0.0, 1.0);
        assert_eq!(collapsed.tint_alpha, 1.0);
        assert_eq!(collapsed.sheen.alpha, 0.0);
        assert_eq!(collapsed.rim.hi_alpha, 0.0);
        assert_eq!(collapsed.shadow.alpha, 0.0);
        assert_eq!(collapsed.tint_color, bg);

        // Выключенное стекло (translucent = false): плоское тело (alpha = 1.0), эффекты погашены
        let disabled = body_layers(Material::Regular, bg, false, true, 50.0, 1.0, 1.0);
        assert_eq!(disabled.tint_alpha, 1.0);
        assert_eq!(disabled.sheen.alpha, 0.0);
        assert_eq!(disabled.rim.hi_alpha, 0.0);
        assert_eq!(disabled.shadow.alpha, 0.0);
        assert_eq!(disabled.tint_color, bg);
    }

    #[test]
    fn test_rim_alphas_and_sheen_alpha() {
        // Тёмная тема:
        assert_eq!(sheen_alpha(true), 0.07);
        let (hi_d, mid_d, lo_col_d, lo_a_d) = rim_alphas(true);
        assert_eq!(hi_d, 0.50);
        assert_eq!(mid_d, 0.05);
        assert_eq!(lo_col_d, [1.0, 1.0, 1.0]);
        assert_eq!(lo_a_d, 0.22);

        // Светлая тема:
        assert_eq!(sheen_alpha(false), 0.30);
        let (hi_l, mid_l, lo_col_l, lo_a_l) = rim_alphas(false);
        assert_eq!(hi_l, 0.70);
        assert_eq!(mid_l, 0.08);
        assert_eq!(lo_col_l, [0.0, 0.0, 0.0]);
        assert_eq!(lo_a_l, 0.10);
    }

    #[test]
    fn blur_plate_inset_rules() {
        for r_dwm in [8.0f32, 10.0, 12.0, 16.0] {
            // При r_body <= r_dwm inset всегда равен 1.
            for r_body in [0.0f32, 1.0, 4.0, r_dwm - 0.01, r_dwm] {
                let (_, _, _, inset) = blur_plate(10.0, 200.0, 100.0, r_body, r_dwm);
                assert_eq!(inset, 1, "r_body={r_body}, r_dwm={r_dwm}: inset должен быть 1, получен {inset}");
            }

            // inset монотонен по r_body.
            let mut prev_inset = 1;
            let mut r = 0.0f32;
            while r <= 100.0 {
                let (_, _, _, inset) = blur_plate(10.0, 200.0, 100.0, r, r_dwm);
                assert!(
                    inset >= prev_inset,
                    "r_body={r}, r_dwm={r_dwm}: нарушение монотонности: {prev_inset} -> {inset}"
                );
                let expected = (0.2929 * (r - r_dwm).max(0.0)).ceil() as i32 + 1;
                assert_eq!(inset, expected);
                prev_inset = inset;
                r += 0.25;
            }
        }
    }

    #[test]
    fn blur_plate_points_lie_inside_body() {
        let mut test_cases = cases();
        // Добавляем специфичные случаи с нулевым радиусом и r_body < r_dwm
        for x0 in [0.0f32, 25.0] {
            for (aw, ah) in [(200.0f32, 40.0f32), (400.0, 120.0), (333.3, 150.0)] {
                for ear in [0.0f32, 8.0] {
                    for r in [0.0f32, 2.0, 4.0, 6.0, 8.0, 15.0, 28.0] {
                        test_cases.push((x0 + ear, aw, ah, ear, r));
                    }
                }
            }
        }

        for &(x0, aw, ah, ear, r_body) in &test_cases {
            for r_dwm in [8.0f32, 10.0, 12.0, 16.0] {
                let (left_off, width, height, _inset) = blur_plate(x0, aw, ah, r_body, r_dwm);
                let pl = left_off as f32;
                let pr = (left_off + width) as f32;
                let pt = 0.0f32;
                let pb = height as f32;

                let mut sample_pts = vec![
                    // 1. Верхний край и верхние углы
                    (pl, pt),
                    (pr, pt),
                    ((pl + pr) * 0.5, pt),
                    // 2. Боковые кромки
                    (pl, (pt + pb) * 0.5),
                    (pr, (pt + pb) * 0.5),
                    // 3. Нижний край между скруглениями
                    ((pl + pr) * 0.5, pb),
                ];

                // 4. Точки скруглённого левого нижнего угла плиты с радиусом r_dwm
                let clx = pl + r_dwm;
                let cly = pb - r_dwm;
                for step in 0..=16 {
                    let a = PI * 0.5 + (PI * 0.5) * (step as f32 / 16.0);
                    sample_pts.push((clx + r_dwm * a.cos(), cly + r_dwm * a.sin()));
                }

                // 5. Точки скруглённого правого нижнего угла плиты с радиусом r_dwm
                let crx = pr - r_dwm;
                let cry = pb - r_dwm;
                for step in 0..=16 {
                    let a = (PI * 0.5) * (step as f32 / 16.0);
                    sample_pts.push((crx + r_dwm * a.cos(), cry + r_dwm * a.sin()));
                }

                // Проверка, что каждая точка скруглённой плиты лежит строго внутри тела
                for (px, py) in sample_pts {
                    assert!(
                        inside_body(px, py, x0, aw, ah, ear, r_body),
                        "Плита выпирает из тела! x0={x0} aw={aw} ah={ah} ear={ear} r_body={r_body} r_dwm={r_dwm}: точка ({px}, {py}), плита: pl={pl} pr={pr} pb={pb}"
                    );
                }
            }
        }
    }

    #[test]
    fn erfc_abramowitz_stegun_values() {
        assert!((erfc(0.0) - 1.0).abs() < 1e-6);
        assert!((erfc(1.0) - 0.1572992).abs() < 1e-5);
        assert!((erfc(2.0) - 0.0046777).abs() < 1e-5);
        assert!((erfc(3.0) - 0.00002209).abs() < 1e-6);
        assert!((erfc(-1.0) - (2.0 - 0.1572992)).abs() < 1e-5);
    }

    #[test]
    fn shadow_rings_invariants_and_tolerances() {
        // Проверка для различных параметров (включая стандартный ios::GLASS_SHADOW 0 8 40 rgba(0,0,0,0.12))
        for blur in [20.0f32, 40.0, 50.0] {
            for peak in [0.05f32, 0.12, 0.25] {
                for n in [24usize, 32, 48, 64] {
                    let rings = shadow_rings(blur, peak, n);

                    // 1. n колец
                    assert_eq!(rings.len(), n, "blur={blur}, peak={peak}, n={n}: число колец");

                    // 2. Монотонность grow: строго возрастает от blur/n до blur
                    let mut prev_grow = 0.0f32;
                    for (i, &(grow, _alpha)) in rings.iter().enumerate() {
                        assert!(
                            grow > prev_grow,
                            "blur={blur}, peak={peak}, n={n}, ring={i}: grow={grow} <= prev={prev_grow}"
                        );
                        let expected = (i + 1) as f32 * blur / n as f32;
                        assert!((grow - expected).abs() < 1e-5);
                        prev_grow = grow;
                    }
                    assert!((rings[0].0 - blur / n as f32).abs() < 1e-5);
                    assert!((rings[n - 1].0 - blur).abs() < 1e-5);

                    // 3. Все альфы в 0..1
                    for (i, &(_grow, alpha)) in rings.iter().enumerate() {
                        assert!(
                            (0.0..=1.0).contains(&alpha),
                            "blur={blur}, peak={peak}, n={n}, ring={i}: alpha={alpha} вне 0..1"
                        );
                    }

                    // 4. Внешнее кольцо должно иметь альфу < 0.004 (иначе виден резкий край)
                    let outer_alpha = rings[n - 1].1;
                    assert!(
                        outer_alpha < 0.004,
                        "blur={blur}, peak={peak}, n={n}: внешняя альфа {outer_alpha} >= 0.004"
                    );

                    // 5. Накопленная непрозрачность на нулевом расстоянии ≈ peak_alpha (допуск 3%)
                    let acc_zero = 1.0 - rings.iter().fold(1.0f32, |prod, &(_, a)| prod * (1.0 - a));
                    let diff_pct = (acc_zero - peak).abs() / peak;
                    assert!(
                        diff_pct <= 0.03,
                        "blur={blur}, peak={peak}, n={n}: накопленная альфа на теле {acc_zero} отличается от {peak} на {:.2}% (> 3%)",
                        diff_pct * 100.0
                    );

                    // Сумма альф близка к peak_alpha
                    let sum_alpha: f32 = rings.iter().map(|&(_, a)| a).sum();
                    assert!(sum_alpha >= peak * 0.95 && sum_alpha <= peak * 1.20);
                }
            }
        }
    }

    #[test]
    fn shadow_rings_edge_cases() {
        assert!(shadow_rings(0.0, 0.12, 24).is_empty());
        assert!(shadow_rings(40.0, 0.0, 24).is_empty());
        assert!(shadow_rings(40.0, -0.5, 24).is_empty());
        assert!(shadow_rings(40.0, 0.12, 0).is_empty());
    }
}
