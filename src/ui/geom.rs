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
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains() {
        let r = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert!(r.contains((10.0, 10.0)));
        assert!(!r.contains((30.0, 15.0)));
    }
}
