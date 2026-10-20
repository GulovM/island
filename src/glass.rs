//! Стекло острова без Win32: прозрачность тела и точный контур окна-подложки с размытием.
//! Всё — чистые функции, их проверяют тесты.

use std::f32::consts::{FRAC_PI_2, PI};

/// Самое прозрачное тело при настоящем размытии под островом (0% на ползунке): чёрная подкладка 41%,
/// как у материала «Regular» (тёмный) в iOS 26 UI Kit — `backdrop-blur 50px` + `rgba(0, 0, 0, 0.41)`.
pub const MIN_BODY_ALPHA: f32 = 0.41;
/// Без размытия (в Windows выключены «Эффекты прозрачности» или blur-behind недоступен) фон за телом
/// остаётся чётким, поэтому предел плотнее: иначе текст не читается. Подбирается на экране.
pub const MIN_BODY_ALPHA_NO_BLUR: f32 = 0.60;

/// Фактическая непрозрачность тела острова. `translucent` — стекло включено (`cfg.glass`), `blur` —
/// под телом настоящее размытие, `opacity` — ползунок 0–100 (0 → предел прозрачности, 100 → сплошное),
/// `p` — прогресс раскрытия, `k` — 1 − «полоска». Свёрнутый остров и полоска всегда сплошные.
pub fn body_alpha(translucent: bool, blur: bool, opacity: f32, p: f32, k: f32) -> f32 {
    if !translucent || p <= 0.001 {
        return 1.0;
    }
    let min = if blur { MIN_BODY_ALPHA } else { MIN_BODY_ALPHA_NO_BLUR };
    let base = min + (1.0 - min) * (opacity / 100.0).clamp(0.0, 1.0);
    1.0 - (1.0 - base) * p.clamp(0.0, 1.0) * k.clamp(0.0, 1.0)
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
        assert_eq!(body_alpha(false, true, 0.0, 1.0, 1.0), 1.0);
        assert_eq!(body_alpha(true, true, 0.0, 0.0, 1.0), 1.0);
        // С размытием 0% — подкладка 41% как в Figma, 100% — сплошное тело.
        assert!((body_alpha(true, true, 0.0, 1.0, 1.0) - MIN_BODY_ALPHA).abs() < 1e-6);
        assert!((body_alpha(true, true, 100.0, 1.0, 1.0) - 1.0).abs() < 1e-6);
        // Без размытия предел плотнее, ползунок по-прежнему тянется до сплошного.
        assert!((body_alpha(true, false, 0.0, 1.0, 1.0) - MIN_BODY_ALPHA_NO_BLUR).abs() < 1e-6);
        assert!((body_alpha(true, false, 100.0, 1.0, 1.0) - 1.0).abs() < 1e-6);
        for blur in [true, false] {
            let min = if blur { MIN_BODY_ALPHA } else { MIN_BODY_ALPHA_NO_BLUR };
            let mut prev = 0.0;
            for o in 0..=100 {
                let a = body_alpha(true, blur, o as f32, 1.0, 1.0);
                assert!(a >= min - 1e-6 && a > prev, "{blur} {o}%: {a}");
                prev = a;
            }
            assert!(body_alpha(true, blur, -50.0, 1.0, 1.0) >= min - 1e-6);
            // В полоске (k = 0) тело сплошное, в середине раскрытия — плотнее, чем раскрытое.
            assert_eq!(body_alpha(true, blur, 0.0, 1.0, 0.0), 1.0);
            assert!(body_alpha(true, blur, 0.0, 0.5, 1.0) > body_alpha(true, blur, 0.0, 1.0, 1.0));
        }
    }
}
