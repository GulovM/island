#![allow(non_snake_case)]

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    pub X: f32,
    pub Y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector3 {
    pub X: f32,
    pub Y: f32,
    pub Z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Matrix3x2 {
    pub M11: f32,
    pub M12: f32,
    pub M21: f32,
    pub M22: f32,
    pub M31: f32,
    pub M32: f32,
}

impl Matrix3x2 {
    pub const fn identity() -> Self {
        Self { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 }
    }
}
