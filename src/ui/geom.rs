//! Прямоугольники и аффинные трансформации холста.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows_numerics::{Matrix3x2, Vector2};

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
    /// Есть ли у прямоугольников общая площадь (касание краёв не считается).
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
}

/// Концентрический радиус: вложенный элемент на `inset` от края внешнего со скруглением `outer`.
pub fn inner_radius(outer: f32, inset: f32) -> f32 {
    (outer - inset).max(0.0)
}

pub(super) fn pt(x: f32, y: f32) -> Vector2 {
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

/// Масштаб `scale` и поворот на `deg` градусов вокруг `(cx, cy)`, при которых сама точка остаётся
/// на месте (карточка «приподнимается» или покачивается на своём месте). Без DPI: потом
/// достаточно умножить на текущую матрицу холста через [`compose`].
pub fn matrix_about(scale: f32, deg: f32, cx: f32, cy: f32) -> Matrix3x2 {
    let (sin, cos) = deg.to_radians().sin_cos();
    Matrix3x2 {
        M11: scale * cos,
        M12: scale * sin,
        M21: -scale * sin,
        M22: scale * cos,
        M31: cx - scale * (cx * cos - cy * sin),
        M32: cy - scale * (cx * sin + cy * cos),
    }
}

/// Произведение `a · b` для строк-векторов Direct2D: точка сначала проходит `a`, потом `b`.
pub fn compose(a: &Matrix3x2, b: &Matrix3x2) -> Matrix3x2 {
    Matrix3x2 {
        M11: a.M11 * b.M11 + a.M12 * b.M21,
        M12: a.M11 * b.M12 + a.M12 * b.M22,
        M21: a.M21 * b.M11 + a.M22 * b.M21,
        M22: a.M21 * b.M12 + a.M22 * b.M22,
        M31: a.M31 * b.M11 + a.M32 * b.M21 + b.M31,
        M32: a.M31 * b.M12 + a.M32 * b.M22 + b.M32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &Matrix3x2, p: (f32, f32)) -> (f32, f32) {
        (p.0 * m.M11 + p.1 * m.M21 + m.M31, p.0 * m.M12 + p.1 * m.M22 + m.M32)
    }

    #[test]
    fn about_keeps_centre_and_compose_chains() {
        let m = matrix_about(1.05, 3.0, 40.0, 25.0);
        let c = apply(&m, (40.0, 25.0));
        assert!((c.0 - 40.0).abs() < 1e-3 && (c.1 - 25.0).abs() < 1e-3, "{c:?}");
        // Покачивание, потом масштаб DPI и сдвиг вкладки: порядок важен.
        let dpi = matrix(1.5, 30.0, 0.0);
        let both = compose(&m, &dpi);
        let p = (10.0, 5.0);
        let step = apply(&dpi, apply(&m, p));
        let once = apply(&both, p);
        assert!((step.0 - once.0).abs() < 1e-3 && (step.1 - once.1).abs() < 1e-3, "{step:?} {once:?}");
    }

    #[test]
    fn rect_contains() {
        let r = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert!(r.contains((10.0, 10.0)));
        assert!(!r.contains((30.0, 15.0)));
    }

    #[test]
    fn inner_radius_is_concentric_and_never_negative() {
        assert_eq!(inner_radius(16.0, 4.0), 12.0);
        assert_eq!(inner_radius(3.0, 4.0), 0.0);
    }
}
