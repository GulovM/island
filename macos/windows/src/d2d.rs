#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::too_many_arguments)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ops::{BitAnd, BitOr, Deref};
use std::sync::{Arc, Mutex};

use windows_core::{CastFrom, Error, Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

use crate::dwrite::{IDWriteTextFormat, IDWriteTextLayout, DWRITE_MEASURING_MODE};
use crate::dxgi_d3d::Dxgi::{DXGI_FORMAT, IDXGIDevice, IDXGISurface};
use crate::foundation::E_FAIL;

pub const D2D1_DEFAULT_FLATTENING_TOLERANCE: f32 = 0.25;

pub mod Common {
    use windows_numerics::Vector2;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct D2D1_COLOR_F {
        pub r: f32,
        pub g: f32,
        pub b: f32,
        pub a: f32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct D2D_RECT_F {
        pub left: f32,
        pub top: f32,
        pub right: f32,
        pub bottom: f32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct D2D_POINT_2F {
        pub x: f32,
        pub y: f32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct D2D_SIZE_F {
        pub width: f32,
        pub height: f32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D_SIZE_U {
        pub width: u32,
        pub height: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D_POINT_2U {
        pub x: u32,
        pub y: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D_RECT_U {
        pub left: u32,
        pub top: u32,
        pub right: u32,
        pub bottom: u32,
    }

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D1_ALPHA_MODE(pub i32);
    pub const D2D1_ALPHA_MODE_UNKNOWN: D2D1_ALPHA_MODE = D2D1_ALPHA_MODE(0);
    pub const D2D1_ALPHA_MODE_PREMULTIPLIED: D2D1_ALPHA_MODE = D2D1_ALPHA_MODE(1);
    pub const D2D1_ALPHA_MODE_STRAIGHT: D2D1_ALPHA_MODE = D2D1_ALPHA_MODE(2);
    pub const D2D1_ALPHA_MODE_IGNORE: D2D1_ALPHA_MODE = D2D1_ALPHA_MODE(3);

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D1_PIXEL_FORMAT {
        pub format: super::DXGI_FORMAT,
        pub alphaMode: D2D1_ALPHA_MODE,
    }

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D1_FIGURE_BEGIN(pub i32);
    pub const D2D1_FIGURE_BEGIN_FILLED: D2D1_FIGURE_BEGIN = D2D1_FIGURE_BEGIN(0);
    pub const D2D1_FIGURE_BEGIN_HOLLOW: D2D1_FIGURE_BEGIN = D2D1_FIGURE_BEGIN(1);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D1_FIGURE_END(pub i32);
    pub const D2D1_FIGURE_END_OPEN: D2D1_FIGURE_END = D2D1_FIGURE_END(0);
    pub const D2D1_FIGURE_END_CLOSED: D2D1_FIGURE_END = D2D1_FIGURE_END(1);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D1_FILL_MODE(pub i32);
    pub const D2D1_FILL_MODE_ALTERNATE: D2D1_FILL_MODE = D2D1_FILL_MODE(0);
    pub const D2D1_FILL_MODE_WINDING: D2D1_FILL_MODE = D2D1_FILL_MODE(1);

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct D2D1_BEZIER_SEGMENT {
        pub point1: Vector2,
        pub point2: Vector2,
        pub point3: Vector2,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct D2D1_QUADRATIC_BEZIER_SEGMENT {
        pub point1: Vector2,
        pub point2: Vector2,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct D2D1_GRADIENT_STOP {
        pub position: f32,
        pub color: D2D1_COLOR_F,
    }

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D1_GAMMA(pub i32);
    pub const D2D1_GAMMA_2_2: D2D1_GAMMA = D2D1_GAMMA(0);
    pub const D2D1_GAMMA_1_0: D2D1_GAMMA = D2D1_GAMMA(1);

    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct D2D1_BORDER_MODE(pub i32);
    pub const D2D1_BORDER_MODE_SOFT: D2D1_BORDER_MODE = D2D1_BORDER_MODE(0);
    pub const D2D1_BORDER_MODE_HARD: D2D1_BORDER_MODE = D2D1_BORDER_MODE(1);
}

pub use Common::*;

// ---- Constants & Enums ----

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_FACTORY_TYPE(pub i32);
pub const D2D1_FACTORY_TYPE_SINGLE_THREADED: D2D1_FACTORY_TYPE = D2D1_FACTORY_TYPE(0);
pub const D2D1_FACTORY_TYPE_MULTI_THREADED: D2D1_FACTORY_TYPE = D2D1_FACTORY_TYPE(1);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_DEVICE_CONTEXT_OPTIONS(pub i32);
pub const D2D1_DEVICE_CONTEXT_OPTIONS_NONE: D2D1_DEVICE_CONTEXT_OPTIONS =
    D2D1_DEVICE_CONTEXT_OPTIONS(0);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_TEXT_ANTIALIAS_MODE(pub i32);
pub const D2D1_TEXT_ANTIALIAS_MODE_DEFAULT: D2D1_TEXT_ANTIALIAS_MODE = D2D1_TEXT_ANTIALIAS_MODE(0);
pub const D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE: D2D1_TEXT_ANTIALIAS_MODE =
    D2D1_TEXT_ANTIALIAS_MODE(1);
pub const D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE: D2D1_TEXT_ANTIALIAS_MODE =
    D2D1_TEXT_ANTIALIAS_MODE(2);
pub const D2D1_TEXT_ANTIALIAS_MODE_ALIASED: D2D1_TEXT_ANTIALIAS_MODE = D2D1_TEXT_ANTIALIAS_MODE(3);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_ANTIALIAS_MODE(pub i32);
pub const D2D1_ANTIALIAS_MODE_PER_PRIMITIVE: D2D1_ANTIALIAS_MODE = D2D1_ANTIALIAS_MODE(0);
pub const D2D1_ANTIALIAS_MODE_ALIASED: D2D1_ANTIALIAS_MODE = D2D1_ANTIALIAS_MODE(1);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_CAP_STYLE(pub i32);
pub const D2D1_CAP_STYLE_FLAT: D2D1_CAP_STYLE = D2D1_CAP_STYLE(0);
pub const D2D1_CAP_STYLE_SQUARE: D2D1_CAP_STYLE = D2D1_CAP_STYLE(1);
pub const D2D1_CAP_STYLE_ROUND: D2D1_CAP_STYLE = D2D1_CAP_STYLE(2);
pub const D2D1_CAP_STYLE_TRIANGLE: D2D1_CAP_STYLE = D2D1_CAP_STYLE(3);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_LINE_JOIN(pub i32);
pub const D2D1_LINE_JOIN_MITER: D2D1_LINE_JOIN = D2D1_LINE_JOIN(0);
pub const D2D1_LINE_JOIN_BEVEL: D2D1_LINE_JOIN = D2D1_LINE_JOIN(1);
pub const D2D1_LINE_JOIN_ROUND: D2D1_LINE_JOIN = D2D1_LINE_JOIN(2);
pub const D2D1_LINE_JOIN_MITER_OR_BEVEL: D2D1_LINE_JOIN = D2D1_LINE_JOIN(3);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_DASH_STYLE(pub i32);
pub const D2D1_DASH_STYLE_SOLID: D2D1_DASH_STYLE = D2D1_DASH_STYLE(0);
pub const D2D1_DASH_STYLE_DASH: D2D1_DASH_STYLE = D2D1_DASH_STYLE(1);
pub const D2D1_DASH_STYLE_DOT: D2D1_DASH_STYLE = D2D1_DASH_STYLE(2);
pub const D2D1_DASH_STYLE_DASH_DOT: D2D1_DASH_STYLE = D2D1_DASH_STYLE(3);
pub const D2D1_DASH_STYLE_DASH_DOT_DOT: D2D1_DASH_STYLE = D2D1_DASH_STYLE(4);
pub const D2D1_DASH_STYLE_CUSTOM: D2D1_DASH_STYLE = D2D1_DASH_STYLE(5);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_STROKE_TRANSFORM_TYPE(pub i32);
pub const D2D1_STROKE_TRANSFORM_TYPE_NORMAL: D2D1_STROKE_TRANSFORM_TYPE =
    D2D1_STROKE_TRANSFORM_TYPE(0);
pub const D2D1_STROKE_TRANSFORM_TYPE_FIXED: D2D1_STROKE_TRANSFORM_TYPE =
    D2D1_STROKE_TRANSFORM_TYPE(1);
pub const D2D1_STROKE_TRANSFORM_TYPE_HAIRLINE: D2D1_STROKE_TRANSFORM_TYPE =
    D2D1_STROKE_TRANSFORM_TYPE(2);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_EXTEND_MODE(pub i32);
pub const D2D1_EXTEND_MODE_CLAMP: D2D1_EXTEND_MODE = D2D1_EXTEND_MODE(0);
pub const D2D1_EXTEND_MODE_WRAP: D2D1_EXTEND_MODE = D2D1_EXTEND_MODE(1);
pub const D2D1_EXTEND_MODE_MIRROR: D2D1_EXTEND_MODE = D2D1_EXTEND_MODE(2);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_BITMAP_OPTIONS(pub u32);
pub const D2D1_BITMAP_OPTIONS_NONE: D2D1_BITMAP_OPTIONS = D2D1_BITMAP_OPTIONS(0);
pub const D2D1_BITMAP_OPTIONS_TARGET: D2D1_BITMAP_OPTIONS = D2D1_BITMAP_OPTIONS(1);
pub const D2D1_BITMAP_OPTIONS_CANNOT_DRAW: D2D1_BITMAP_OPTIONS = D2D1_BITMAP_OPTIONS(2);
pub const D2D1_BITMAP_OPTIONS_CPU_READ: D2D1_BITMAP_OPTIONS = D2D1_BITMAP_OPTIONS(4);

impl BitOr for D2D1_BITMAP_OPTIONS {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl BitAnd for D2D1_BITMAP_OPTIONS {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_MAP_OPTIONS(pub u32);
pub const D2D1_MAP_OPTIONS_READ: D2D1_MAP_OPTIONS = D2D1_MAP_OPTIONS(1);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_DRAW_TEXT_OPTIONS(pub u32);
pub const D2D1_DRAW_TEXT_OPTIONS_NONE: D2D1_DRAW_TEXT_OPTIONS = D2D1_DRAW_TEXT_OPTIONS(0);
pub const D2D1_DRAW_TEXT_OPTIONS_CLIP: D2D1_DRAW_TEXT_OPTIONS = D2D1_DRAW_TEXT_OPTIONS(2);
pub const D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT: D2D1_DRAW_TEXT_OPTIONS =
    D2D1_DRAW_TEXT_OPTIONS(4);

impl BitOr for D2D1_DRAW_TEXT_OPTIONS {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_INTERPOLATION_MODE(pub i32);
pub const D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR: D2D1_INTERPOLATION_MODE =
    D2D1_INTERPOLATION_MODE(0);
pub const D2D1_INTERPOLATION_MODE_LINEAR: D2D1_INTERPOLATION_MODE = D2D1_INTERPOLATION_MODE(1);
pub const D2D1_INTERPOLATION_MODE_CUBIC: D2D1_INTERPOLATION_MODE = D2D1_INTERPOLATION_MODE(2);
pub const D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC: D2D1_INTERPOLATION_MODE =
    D2D1_INTERPOLATION_MODE(5);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_ARC_SIZE(pub i32);
pub const D2D1_ARC_SIZE_SMALL: D2D1_ARC_SIZE = D2D1_ARC_SIZE(0);
pub const D2D1_ARC_SIZE_LARGE: D2D1_ARC_SIZE = D2D1_ARC_SIZE(1);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_SWEEP_DIRECTION(pub i32);
pub const D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE: D2D1_SWEEP_DIRECTION = D2D1_SWEEP_DIRECTION(0);
pub const D2D1_SWEEP_DIRECTION_CLOCKWISE: D2D1_SWEEP_DIRECTION = D2D1_SWEEP_DIRECTION(1);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_COMBINE_MODE(pub i32);
pub const D2D1_COMBINE_MODE_UNION: D2D1_COMBINE_MODE = D2D1_COMBINE_MODE(0);
pub const D2D1_COMBINE_MODE_INTERSECT: D2D1_COMBINE_MODE = D2D1_COMBINE_MODE(1);
pub const D2D1_COMBINE_MODE_XOR: D2D1_COMBINE_MODE = D2D1_COMBINE_MODE(2);
pub const D2D1_COMBINE_MODE_EXCLUDE: D2D1_COMBINE_MODE = D2D1_COMBINE_MODE(3);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_LAYER_OPTIONS1(pub u32);
pub const D2D1_LAYER_OPTIONS1_NONE: D2D1_LAYER_OPTIONS1 = D2D1_LAYER_OPTIONS1(0);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_RENDER_TARGET_TYPE(pub i32);
pub const D2D1_RENDER_TARGET_TYPE_DEFAULT: D2D1_RENDER_TARGET_TYPE = D2D1_RENDER_TARGET_TYPE(0);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_RENDER_TARGET_USAGE(pub u32);
pub const D2D1_RENDER_TARGET_USAGE_NONE: D2D1_RENDER_TARGET_USAGE = D2D1_RENDER_TARGET_USAGE(0);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_FEATURE_LEVEL(pub i32);
pub const D2D1_FEATURE_LEVEL_DEFAULT: D2D1_FEATURE_LEVEL = D2D1_FEATURE_LEVEL(0);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_GAUSSIANBLUR_PROP(pub u32);
pub const D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION: D2D1_GAUSSIANBLUR_PROP =
    D2D1_GAUSSIANBLUR_PROP(0);
pub const D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION: D2D1_GAUSSIANBLUR_PROP = D2D1_GAUSSIANBLUR_PROP(1);
pub const D2D1_GAUSSIANBLUR_PROP_BORDER_MODE: D2D1_GAUSSIANBLUR_PROP = D2D1_GAUSSIANBLUR_PROP(2);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D2D1_GAUSSIANBLUR_OPTIMIZATION(pub u32);
pub const D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED: D2D1_GAUSSIANBLUR_OPTIMIZATION =
    D2D1_GAUSSIANBLUR_OPTIMIZATION(1);

// ---- Structs ----

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_ROUNDED_RECT {
    pub rect: D2D_RECT_F,
    pub radiusX: f32,
    pub radiusY: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_ELLIPSE {
    pub point: Vector2,
    pub radiusX: f32,
    pub radiusY: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_ARC_SEGMENT {
    pub point: Vector2,
    pub size: D2D_SIZE_F,
    pub rotationAngle: f32,
    pub sweepDirection: D2D1_SWEEP_DIRECTION,
    pub arcSize: D2D1_ARC_SIZE,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct D2D1_STROKE_STYLE_PROPERTIES1 {
    pub startCap: D2D1_CAP_STYLE,
    pub endCap: D2D1_CAP_STYLE,
    pub dashCap: D2D1_CAP_STYLE,
    pub lineJoin: D2D1_LINE_JOIN,
    pub miterLimit: f32,
    pub dashStyle: D2D1_DASH_STYLE,
    pub dashOffset: f32,
    pub transformType: D2D1_STROKE_TRANSFORM_TYPE,
}

impl Default for D2D1_STROKE_STYLE_PROPERTIES1 {
    fn default() -> Self {
        Self {
            startCap: D2D1_CAP_STYLE_FLAT,
            endCap: D2D1_CAP_STYLE_FLAT,
            dashCap: D2D1_CAP_STYLE_FLAT,
            lineJoin: D2D1_LINE_JOIN_MITER,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
            transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_BITMAP_PROPERTIES1 {
    pub pixelFormat: D2D1_PIXEL_FORMAT,
    pub dpiX: f32,
    pub dpiY: f32,
    pub bitmapOptions: D2D1_BITMAP_OPTIONS,
    pub colorContext: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
    pub startPoint: Vector2,
    pub endPoint: Vector2,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
    pub center: Vector2,
    pub gradientOriginOffset: Vector2,
    pub radiusX: f32,
    pub radiusY: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_BRUSH_PROPERTIES {
    pub opacity: f32,
    pub transform: Matrix3x2,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_BITMAP_BRUSH_PROPERTIES1 {
    pub extendModeX: D2D1_EXTEND_MODE,
    pub extendModeY: D2D1_EXTEND_MODE,
    pub interpolationMode: D2D1_INTERPOLATION_MODE,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct D2D1_MAPPED_RECT {
    pub pitch: u32,
    pub bits: *mut u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D2D1_RENDER_TARGET_PROPERTIES {
    pub r#type: D2D1_RENDER_TARGET_TYPE,
    pub pixelFormat: D2D1_PIXEL_FORMAT,
    pub dpiX: f32,
    pub dpiY: f32,
    pub usage: D2D1_RENDER_TARGET_USAGE,
    pub minLevel: D2D1_FEATURE_LEVEL,
}

#[derive(Clone, Debug)]
pub struct D2D1_LAYER_PARAMETERS1 {
    pub contentBounds: D2D_RECT_F,
    pub geometricMask: ManuallyDrop<Option<ID2D1Geometry>>,
    pub maskAntialiasMode: D2D1_ANTIALIAS_MODE,
    pub maskTransform: Matrix3x2,
    pub opacity: f32,
    pub opacityBrush: ManuallyDrop<Option<ID2D1Brush>>,
    pub layerOptions: D2D1_LAYER_OPTIONS1,
}

// ---- Geometry Engine ----

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt2 {
    pub x: f64,
    pub y: f64,
}

impl Pt2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PathCmd {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Arc {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
        rot_deg: f64,
        end_x: f64,
        end_y: f64,
        large: bool,
        cw: bool,
    },
    Close,
}

#[derive(Clone, Debug)]
pub struct PolyFigure {
    pub pts: Vec<Pt2>,
    pub closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GeometryData {
    pub cmds: Vec<PathCmd>,
    pub fill_mode: D2D1_FILL_MODE,
}

fn transform_pt(m: Option<&Matrix3x2>, x: f64, y: f64) -> Pt2 {
    if let Some(m) = m {
        Pt2::new(
            x * m.M11 as f64 + y * m.M21 as f64 + m.M31 as f64,
            x * m.M12 as f64 + y * m.M22 as f64 + m.M32 as f64,
        )
    } else {
        Pt2::new(x, y)
    }
}

fn push_unique_pt(pts: &mut Vec<Pt2>, p: Pt2) {
    if let Some(last) = pts.last() {
        if (last.x - p.x).hypot(last.y - p.y) < 1e-6 {
            return;
        }
    }
    pts.push(p);
}

fn flatten_quad(p0: Pt2, p1: Pt2, p2: Pt2, tol: f64, depth: u8, out: &mut Vec<Pt2>) {
    let mx = 0.5 * (p0.x + p2.x);
    let my = 0.5 * (p0.y + p2.y);
    let d = (p1.x - mx).hypot(p1.y - my);
    if depth >= 10 || d <= tol {
        push_unique_pt(out, p2);
        return;
    }
    let q0 = Pt2::new(0.5 * (p0.x + p1.x), 0.5 * (p0.y + p1.y));
    let q1 = Pt2::new(0.5 * (p1.x + p2.x), 0.5 * (p1.y + p2.y));
    let r = Pt2::new(0.5 * (q0.x + q1.x), 0.5 * (q0.y + q1.y));
    flatten_quad(p0, q0, r, tol, depth + 1, out);
    flatten_quad(r, q1, p2, tol, depth + 1, out);
}

fn flatten_cubic(p0: Pt2, p1: Pt2, p2: Pt2, p3: Pt2, tol: f64, depth: u8, out: &mut Vec<Pt2>) {
    let dx = p3.x - p0.x;
    let dy = p3.y - p0.y;
    let len = dx.hypot(dy);
    let (d1, d2) = if len > 1e-9 {
        let d1 = ((p1.x - p0.x) * dy - (p1.y - p0.y) * dx).abs() / len;
        let d2 = ((p2.x - p0.x) * dy - (p2.y - p0.y) * dx).abs() / len;
        (d1, d2)
    } else {
        ((p1.x - p0.x).hypot(p1.y - p0.y), (p2.x - p0.x).hypot(p2.y - p0.y))
    };
    if depth >= 10 || d1.max(d2) <= tol {
        push_unique_pt(out, p3);
        return;
    }
    let m01 = Pt2::new(0.5 * (p0.x + p1.x), 0.5 * (p0.y + p1.y));
    let m12 = Pt2::new(0.5 * (p1.x + p2.x), 0.5 * (p1.y + p2.y));
    let m23 = Pt2::new(0.5 * (p2.x + p3.x), 0.5 * (p2.y + p3.y));
    let l012 = Pt2::new(0.5 * (m01.x + m12.x), 0.5 * (m01.y + m12.y));
    let l123 = Pt2::new(0.5 * (m12.x + m23.x), 0.5 * (m12.y + m23.y));
    let mid = Pt2::new(0.5 * (l012.x + l123.x), 0.5 * (l012.y + l123.y));
    flatten_cubic(p0, m01, l012, mid, tol, depth + 1, out);
    flatten_cubic(mid, l123, m23, p3, tol, depth + 1, out);
}

/// Converts a Direct2D/SVG elliptical arc segment into center parameterization and flattens it.
pub fn arc_to_points(
    p0: Pt2,
    end: Pt2,
    mut rx: f64,
    mut ry: f64,
    rot_deg: f64,
    large: bool,
    cw: bool,
    tol: f64,
    out: &mut Vec<Pt2>,
) {
    if (p0.x - end.x).hypot(p0.y - end.y) < 1e-7 {
        return;
    }
    rx = rx.abs();
    ry = ry.abs();
    if rx < 1e-7 || ry < 1e-7 {
        push_unique_pt(out, end);
        return;
    }
    let phi = rot_deg.to_radians();
    let (sin_p, cos_p) = phi.sin_cos();
    let dx2 = 0.5 * (p0.x - end.x);
    let dy2 = 0.5 * (p0.y - end.y);
    let x1p = cos_p * dx2 + sin_p * dy2;
    let y1p = -sin_p * dx2 + cos_p * dy2;

    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let x1p2 = x1p * x1p;
    let y1p2 = y1p * y1p;
    let num = (rx2 * ry2 - rx2 * y1p2 - ry2 * x1p2).max(0.0);
    let den = (rx2 * y1p2 + ry2 * x1p2).max(1e-12);
    let sign = if large == cw { -1.0 } else { 1.0 };
    let coef = sign * (num / den).sqrt();
    let cxp = coef * (rx * y1p) / ry;
    let cyp = -coef * (ry * x1p) / rx;

    let cx = cos_p * cxp - sin_p * cyp + 0.5 * (p0.x + end.x);
    let cy = sin_p * cxp + cos_p * cyp + 0.5 * (p0.y + end.y);

    let vec_angle = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let cos_a = if len > 1e-12 { (dot / len).clamp(-1.0, 1.0) } else { 1.0 };
        let mut ang = cos_a.acos();
        if ux * vy - uy * vx < 0.0 {
            ang = -ang;
        }
        ang
    };

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;

    let theta1 = vec_angle(1.0, 0.0, ux, uy);
    let mut dtheta = vec_angle(ux, uy, vx, vy);
    if !cw && dtheta > 0.0 {
        dtheta -= std::f64::consts::TAU;
    } else if cw && dtheta < 0.0 {
        dtheta += std::f64::consts::TAU;
    }

    let max_r = rx.max(ry);
    let max_step = if max_r > tol {
        let c = (1.0 - tol / max_r).clamp(-1.0, 1.0);
        (2.0 * c.acos()).clamp(0.04, 0.4)
    } else {
        0.35
    };
    let steps = ((dtheta.abs() / max_step).ceil() as usize).clamp(4, 96);
    for i in 1..steps {
        let t = theta1 + dtheta * (i as f64 / steps as f64);
        let (st, ct) = t.sin_cos();
        let px = cos_p * (rx * ct) - sin_p * (ry * st) + cx;
        let py = sin_p * (rx * ct) + cos_p * (ry * st) + cy;
        push_unique_pt(out, Pt2::new(px, py));
    }
    push_unique_pt(out, end);
}

impl GeometryData {
    pub fn flatten(&self, m: Option<&Matrix3x2>, tolerance: f32) -> Vec<PolyFigure> {
        let tol = (tolerance as f64).clamp(0.02, 2.0);
        let mut figs = Vec::new();
        let mut cur_pts: Vec<Pt2> = Vec::new();
        let mut cur_raw = Pt2::new(0.0, 0.0);
        let mut start_raw = cur_raw;

        for &cmd in &self.cmds {
            match cmd {
                PathCmd::Move(x, y) => {
                    if cur_pts.len() >= 2 {
                        figs.push(PolyFigure { pts: cur_pts, closed: false });
                    }
                    cur_raw = Pt2::new(x, y);
                    start_raw = cur_raw;
                    cur_pts = vec![transform_pt(m, x, y)];
                }
                PathCmd::Line(x, y) => {
                    cur_raw = Pt2::new(x, y);
                    push_unique_pt(&mut cur_pts, transform_pt(m, x, y));
                }
                PathCmd::Quad(ax, ay, x, y) => {
                    let p0 = transform_pt(m, cur_raw.x, cur_raw.y);
                    let p1 = transform_pt(m, ax, ay);
                    let p2 = transform_pt(m, x, y);
                    flatten_quad(p0, p1, p2, tol, 0, &mut cur_pts);
                    cur_raw = Pt2::new(x, y);
                }
                PathCmd::Cubic(ax, ay, bx, by, x, y) => {
                    let p0 = transform_pt(m, cur_raw.x, cur_raw.y);
                    let p1 = transform_pt(m, ax, ay);
                    let p2 = transform_pt(m, bx, by);
                    let p3 = transform_pt(m, x, y);
                    flatten_cubic(p0, p1, p2, p3, tol, 0, &mut cur_pts);
                    cur_raw = Pt2::new(x, y);
                }
                PathCmd::Arc { rx, ry, rot_deg, end_x, end_y, large, cw, .. } => {
                    // Flatten in local space then transform so skewed matrices stay exact.
                    let mut arc_pts = Vec::new();
                    arc_to_points(
                        cur_raw,
                        Pt2::new(end_x, end_y),
                        rx,
                        ry,
                        rot_deg,
                        large,
                        cw,
                        tol,
                        &mut arc_pts,
                    );
                    for pt in arc_pts {
                        push_unique_pt(&mut cur_pts, transform_pt(m, pt.x, pt.y));
                    }
                    cur_raw = Pt2::new(end_x, end_y);
                }
                PathCmd::Close => {
                    if cur_pts.len() >= 2 {
                        if let Some(first) = cur_pts.first().copied() {
                            if let Some(last) = cur_pts.last() {
                                if (first.x - last.x).hypot(first.y - last.y) < 1e-6 {
                                    cur_pts.pop();
                                }
                            }
                        }
                        if cur_pts.len() >= 2 {
                            figs.push(PolyFigure { pts: cur_pts, closed: true });
                        }
                    }
                    cur_pts = Vec::new();
                    cur_raw = start_raw;
                }
            }
        }
        if cur_pts.len() >= 2 {
            figs.push(PolyFigure { pts: cur_pts, closed: false });
        }
        figs
    }

    pub fn bounds(&self, m: Option<&Matrix3x2>) -> D2D_RECT_F {
        let figs = self.flatten(m, 0.15);
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for f in &figs {
            for p in &f.pts {
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
        }
        if !min_x.is_finite() {
            return D2D_RECT_F::default();
        }
        D2D_RECT_F {
            left: min_x as f32,
            top: min_y as f32,
            right: max_x as f32,
            bottom: max_y as f32,
        }
    }
}

// ---- Planar Slab Polygon Boolean Engine ----

const Q_SCALE: f64 = 4096.0;

#[derive(Clone, Copy)]
struct Seg {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    dir: i32, // +1 if original y0 < y1 (downward), -1 if upward
    from_b: bool,
}

#[inline]
fn quant(v: f64) -> i64 {
    (v * Q_SCALE).round() as i64
}

fn collect_segments(figs: &[PolyFigure], from_b: bool, out: &mut Vec<Seg>) {
    for fig in figs {
        if fig.pts.len() < 2 {
            continue;
        }
        let n = fig.pts.len();
        let edge_count = if fig.closed { n } else { n - 1 };
        for i in 0..edge_count {
            let a = fig.pts[i];
            let b = fig.pts[(i + 1) % n];
            let dy = b.y - a.y;
            if dy.abs() < 1e-8 {
                continue;
            }
            if dy > 0.0 {
                out.push(Seg { x0: a.x, y0: a.y, x1: b.x, y1: b.y, dir: 1, from_b });
            } else {
                out.push(Seg { x0: b.x, y0: b.y, x1: a.x, y1: a.y, dir: -1, from_b });
            }
        }
    }
}

fn is_inside(wind: i32, mode: D2D1_FILL_MODE) -> bool {
    if mode == D2D1_FILL_MODE_ALTERNATE {
        (wind & 1) != 0
    } else {
        wind != 0
    }
}

/// Evaluates planar boolean combination (`UNION`, `INTERSECT`, `XOR`, `EXCLUDE`) via exact horizontal slabs.
pub fn boolean_polygons(
    figs_a: &[PolyFigure],
    mode_a: D2D1_FILL_MODE,
    figs_b: &[PolyFigure],
    mode_b: D2D1_FILL_MODE,
    combine: D2D1_COMBINE_MODE,
) -> Vec<PolyFigure> {
    let mut segs = Vec::new();
    collect_segments(figs_a, false, &mut segs);
    collect_segments(figs_b, true, &mut segs);
    if segs.is_empty() {
        return Vec::new();
    }

    let mut ys: Vec<f64> = Vec::with_capacity(segs.len() * 2 + 16);
    for s in &segs {
        ys.push(s.y0);
        ys.push(s.y1);
    }

    // All pairwise non-parallel segment intersections inside shared y-spans.
    for i in 0..segs.len() {
        let a = &segs[i];
        let dax = a.x1 - a.x0;
        let day = a.y1 - a.y0;
        for b in &segs[i + 1..] {
            if a.y1 <= b.y0 || b.y1 <= a.y0 {
                continue;
            }
            let dbx = b.x1 - b.x0;
            let dby = b.y1 - b.y0;
            let det = dax * dby - day * dbx;
            if det.abs() < 1e-10 {
                continue;
            }
            let rx = b.x0 - a.x0;
            let ry = b.y0 - a.y0;
            let t = (rx * dby - ry * dbx) / det;
            let u = (rx * day - ry * dax) / det;
            if t > 1e-7 && t < 1.0 - 1e-7 && u > 1e-7 && u < 1.0 - 1e-7 {
                ys.push(a.y0 + t * day);
            }
        }
    }

    ys.sort_by(|a, b| a.total_cmp(b));
    let mut unique_ys: Vec<f64> = Vec::with_capacity(ys.len());
    for y in ys {
        if let Some(&prev) = unique_ys.last() {
            if (y - prev).abs() < 1e-5 {
                continue;
            }
        }
        unique_ys.push(y);
    }
    if unique_ys.len() < 2 {
        return Vec::new();
    }

    let mut edges: Vec<((i64, i64), (i64, i64))> = Vec::new();
    #[derive(Clone, Copy)]
    struct BoundSpan {
        qx_top_start: i64,
        qx_bot_start: i64,
        qx_top_end: i64,
        qx_bot_end: i64,
    }
    let mut prev_spans: Vec<BoundSpan> = Vec::new();
    let mut prev_qy_bot: i64 = 0;

    for k in 0..unique_ys.len() {
        let mut cur_spans: Vec<BoundSpan> = Vec::new();
        let qy0 = quant(unique_ys[k]);

        if k + 1 < unique_ys.len() {
            let y0 = unique_ys[k];
            let y1 = unique_ys[k + 1];
            let qy1 = quant(y1);
            let y_mid = 0.5 * (y0 + y1);

            struct Active {
                x_mid: f64,
                slope: f64,
                qx0: i64,
                qx1: i64,
                dir: i32,
                from_b: bool,
            }
            let mut active = Vec::new();
            for s in &segs {
                if s.y0 <= y0 + 1e-6 && s.y1 >= y1 - 1e-6 {
                    let dy = s.y1 - s.y0;
                    let slope = (s.x1 - s.x0) / dy;
                    active.push(Active {
                        x_mid: s.x0 + (y_mid - s.y0) * slope,
                        slope,
                        qx0: quant(s.x0 + (y0 - s.y0) * slope),
                        qx1: quant(s.x0 + (y1 - s.y0) * slope),
                        dir: s.dir,
                        from_b: s.from_b,
                    });
                }
            }
            active.sort_by(|a, b| {
                a.x_mid.total_cmp(&b.x_mid).then_with(|| a.slope.total_cmp(&b.slope))
            });

            let mut wa = 0i32;
            let mut wb = 0i32;
            let mut prev_in = false;
            let mut span_start_t = 0i64;
            let mut span_start_b = 0i64;
            let mut idx = 0;

            while idx < active.len() {
                let base = &active[idx];
                let mut end_idx = idx + 1;
                while end_idx < active.len()
                    && active[end_idx].qx0 == base.qx0
                    && active[end_idx].qx1 == base.qx1
                {
                    end_idx += 1;
                }
                for a in &active[idx..end_idx] {
                    if a.from_b {
                        wb += a.dir;
                    } else {
                        wa += a.dir;
                    }
                }
                let in_a = is_inside(wa, mode_a);
                let in_b = is_inside(wb, mode_b);
                let next_in = match combine {
                    D2D1_COMBINE_MODE_UNION => in_a || in_b,
                    D2D1_COMBINE_MODE_INTERSECT => in_a && in_b,
                    D2D1_COMBINE_MODE_XOR => in_a ^ in_b,
                    D2D1_COMBINE_MODE_EXCLUDE => in_a && !in_b,
                    _ => in_a || in_b,
                };

                if !prev_in && next_in {
                    span_start_t = base.qx0;
                    span_start_b = base.qx1;
                    edges.push(((base.qx1, qy1), (base.qx0, qy0)));
                } else if prev_in && !next_in {
                    edges.push(((base.qx0, qy0), (base.qx1, qy1)));
                    if span_start_t < base.qx0 || span_start_b < base.qx1 {
                        cur_spans.push(BoundSpan {
                            qx_top_start: span_start_t,
                            qx_bot_start: span_start_b,
                            qx_top_end: base.qx0,
                            qx_bot_end: base.qx1,
                        });
                    }
                }
                prev_in = next_in;
                idx = end_idx;
            }
        }

        // Emit horizontal edges along qy0 comparing prev_spans bottom vs cur_spans top!
        let mut xs: Vec<i64> = Vec::with_capacity((prev_spans.len() + cur_spans.len()) * 2);
        for sp in &prev_spans {
            xs.push(sp.qx_bot_start);
            xs.push(sp.qx_bot_end);
        }
        for sp in &cur_spans {
            xs.push(sp.qx_top_start);
            xs.push(sp.qx_top_end);
        }
        xs.sort_unstable();
        xs.dedup();

        let y_line = if k == 0 { qy0 } else { prev_qy_bot };
        for w in xs.windows(2) {
            let x0 = w[0];
            let x1 = w[1];
            let in_above = prev_spans.iter().any(|sp| sp.qx_bot_start <= x0 && x1 <= sp.qx_bot_end);
            let in_below = cur_spans.iter().any(|sp| sp.qx_top_start <= x0 && x1 <= sp.qx_top_end);
            if in_above && !in_below {
                // Interior is above -> boundary goes right-to-left (x1 -> x0)
                edges.push(((x1, y_line), (x0, y_line)));
            } else if !in_above && in_below {
                // Interior is below -> boundary goes left-to-right (x0 -> x1)
                edges.push(((x0, y_line), (x1, y_line)));
            }
        }

        if k + 1 < unique_ys.len() {
            prev_qy_bot = quant(unique_ys[k + 1]);
        }
        prev_spans = cur_spans;
    }

    stitch_edges(edges)
}

fn stitch_edges(raw_edges: Vec<((i64, i64), (i64, i64))>) -> Vec<PolyFigure> {
    let mut adj: HashMap<(i64, i64), Vec<(i64, i64)>> = HashMap::new();
    for (a, b) in raw_edges {
        if a == b {
            continue;
        }
        adj.entry(a).or_default().push(b);
    }

    let mut keys: Vec<(i64, i64)> = adj.keys().copied().collect();
    keys.sort_unstable();

    let mut figs = Vec::new();
    for start in keys {
        loop {
            let Some(next) = adj.get_mut(&start).and_then(|v| v.pop()) else {
                break;
            };
            let mut ring = vec![start, next];
            let mut cur = next;
            let mut steps = 0usize;
            while cur != start && steps < 100_000 {
                steps += 1;
                let prev = ring[ring.len() - 2];
                let Some(cands) = adj.get_mut(&cur) else {
                    break;
                };
                if cands.is_empty() {
                    break;
                }
                // Prefer continuation without 180-deg reversal
                let pos = cands.iter().position(|&c| c != prev).unwrap_or(0);
                let nxt = cands.swap_remove(pos);
                if nxt == start {
                    cur = start;
                    break;
                }
                ring.push(nxt);
                cur = nxt;
            }
            if ring.len() >= 3 && cur == start {
                // Remove collinear middle points
                let mut cleaned: Vec<Pt2> = Vec::with_capacity(ring.len());
                for i in 0..ring.len() {
                    let a = ring[(i + ring.len() - 1) % ring.len()];
                    let b = ring[i];
                    let c = ring[(i + 1) % ring.len()];
                    let cross = (b.0 - a.0) as i128 * (c.1 - b.1) as i128
                        - (b.1 - a.1) as i128 * (c.0 - b.0) as i128;
                    if cross != 0 {
                        cleaned.push(Pt2::new(b.0 as f64 / Q_SCALE, b.1 as f64 / Q_SCALE));
                    }
                }
                if cleaned.len() >= 3 {
                    figs.push(PolyFigure { pts: cleaned, closed: true });
                }
            }
        }
    }
    figs
}

/// Widens each path figure by `stroke_width` with round/flat/square caps & joins, then resolves overlaps.
fn widen_geometry(
    src: &GeometryData,
    stroke_width: f32,
    style: Option<&StrokeStyleData>,
    m: Option<&Matrix3x2>,
    tolerance: f32,
) -> Vec<PolyFigure> {
    let hw = (stroke_width as f64 * 0.5).max(0.0);
    if hw <= 0.0 {
        return Vec::new();
    }
    let st = style.cloned().unwrap_or_default();
    let figs = src.flatten(m, tolerance);
    let mut parts: Vec<PolyFigure> = Vec::new();

    let push_disc = |cx: f64, cy: f64, r: f64, parts: &mut Vec<PolyFigure>| {
        let mut pts = Vec::with_capacity(16);
        for i in 0..16 {
            let a = (i as f64 / 16.0) * std::f64::consts::TAU;
            pts.push(Pt2::new(cx + r * a.cos(), cy + r * a.sin()));
        }
        parts.push(PolyFigure { pts, closed: true });
    };

    for fig in &figs {
        if fig.pts.len() < 2 {
            continue;
        }
        let n = fig.pts.len();
        let segs = if fig.closed { n } else { n - 1 };
        for i in 0..segs {
            let p0 = fig.pts[i];
            let p1 = fig.pts[(i + 1) % n];
            let dx = p1.x - p0.x;
            let dy = p1.y - p0.y;
            let len = dx.hypot(dy);
            if len < 1e-8 {
                continue;
            }
            let ux = dx / len;
            let uy = dy / len;
            let nx = -uy * hw;
            let ny = ux * hw;

            let ext0 = if !fig.closed && i == 0 && st.start_cap == D2D1_CAP_STYLE_SQUARE {
                hw
            } else {
                0.0
            };
            let ext1 = if !fig.closed && i + 1 == segs && st.end_cap == D2D1_CAP_STYLE_SQUARE {
                hw
            } else {
                0.0
            };
            let a = Pt2::new(p0.x - ux * ext0, p0.y - uy * ext0);
            let b = Pt2::new(p1.x + ux * ext1, p1.y + uy * ext1);

            parts.push(PolyFigure {
                pts: vec![
                    Pt2::new(a.x + nx, a.y + ny),
                    Pt2::new(b.x + nx, b.y + ny),
                    Pt2::new(b.x - nx, b.y - ny),
                    Pt2::new(a.x - nx, a.y - ny),
                ],
                closed: true,
            });

            if i > 0 || fig.closed {
                if st.line_join == D2D1_LINE_JOIN_ROUND {
                    push_disc(p0.x, p0.y, hw, &mut parts);
                } else {
                    push_disc(p0.x, p0.y, hw * 0.999, &mut parts);
                }
            }
        }
        if !fig.closed {
            if st.start_cap == D2D1_CAP_STYLE_ROUND {
                push_disc(fig.pts[0].x, fig.pts[0].y, hw, &mut parts);
            }
            if st.end_cap == D2D1_CAP_STYLE_ROUND {
                push_disc(fig.pts[n - 1].x, fig.pts[n - 1].y, hw, &mut parts);
            }
        }
    }

    if parts.is_empty() {
        return Vec::new();
    }
    boolean_polygons(
        &parts,
        D2D1_FILL_MODE_WINDING,
        &[],
        D2D1_FILL_MODE_WINDING,
        D2D1_COMBINE_MODE_UNION,
    )
}

fn write_figs_to_sink(figs: &[PolyFigure], sink: &ID2D1GeometrySink) {
    unsafe {
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        for f in figs {
            if f.pts.len() < 3 {
                continue;
            }
            sink.BeginFigure(
                Vector2 { X: f.pts[0].x as f32, Y: f.pts[0].y as f32 },
                D2D1_FIGURE_BEGIN_FILLED,
            );
            for p in &f.pts[1..] {
                sink.AddLine(Vector2 { X: p.x as f32, Y: p.y as f32 });
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
    }
}

// ---- Software / Native Backing Pixel Surface ----

#[derive(Clone, Debug)]
pub struct PixelSurface {
    pub width: u32,
    pub height: u32,
    /// Premultiplied BGRA bytes (`stride = width * 4`).
    pub pixels: Vec<u8>,
}

impl PixelSurface {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0u8; (width as usize) * (height as usize) * 4],
        }
    }

    pub fn clear(&mut self, color: Option<&D2D1_COLOR_F>) {
        if let Some(c) = color {
            let a = c.a.clamp(0.0, 1.0);
            let b = ((c.b.clamp(0.0, 1.0) * a) * 255.0).round() as u8;
            let g = ((c.g.clamp(0.0, 1.0) * a) * 255.0).round() as u8;
            let r = ((c.r.clamp(0.0, 1.0) * a) * 255.0).round() as u8;
            let aa = (a * 255.0).round() as u8;
            for px in self.pixels.chunks_exact_mut(4) {
                px[0] = b;
                px[1] = g;
                px[2] = r;
                px[3] = aa;
            }
        } else {
            self.pixels.fill(0);
        }
    }

    fn blend_pixel(&mut self, x: i32, y: i32, r: f32, g: f32, b: f32, a: f32, cov: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let sa = (a * cov).clamp(0.0, 1.0);
        if sa <= 0.0 {
            return;
        }
        let idx = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        let db = self.pixels[idx] as f32 / 255.0;
        let dg = self.pixels[idx + 1] as f32 / 255.0;
        let dr = self.pixels[idx + 2] as f32 / 255.0;
        let da = self.pixels[idx + 3] as f32 / 255.0;
        let inv = 1.0 - sa;
        self.pixels[idx] = ((b * sa + db * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
        self.pixels[idx + 1] = ((g * sa + dg * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
        self.pixels[idx + 2] = ((r * sa + dr * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
        self.pixels[idx + 3] = ((sa + da * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
    }

    pub fn fill_polygons(
        &mut self,
        figs: &[PolyFigure],
        fill_mode: D2D1_FILL_MODE,
        brush: &BrushData,
        transform: &Matrix3x2,
        clip: Option<D2D_RECT_F>,
    ) {
        if figs.is_empty() || self.width == 0 || self.height == 0 {
            return;
        }
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for f in figs {
            for p in &f.pts {
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
        }
        if !min_x.is_finite() {
            return;
        }
        let mut x0 = (min_x.floor() as i32).max(0);
        let mut y0 = (min_y.floor() as i32).max(0);
        let mut x1 = (max_x.ceil() as i32).min(self.width as i32);
        let mut y1 = (max_y.ceil() as i32).min(self.height as i32);
        if let Some(c) = clip {
            let p0 = transform_pt(Some(transform), c.left as f64, c.top as f64);
            let p1 = transform_pt(Some(transform), c.right as f64, c.bottom as f64);
            x0 = x0.max(p0.x.min(p1.x).floor() as i32);
            y0 = y0.max(p0.y.min(p1.y).floor() as i32);
            x1 = x1.min(p0.x.max(p1.x).ceil() as i32);
            y1 = y1.min(p0.y.max(p1.y).ceil() as i32);
        }
        if x0 >= x1 || y0 >= y1 {
            return;
        }

        // 2x2 subpixel supersampling for crisp anti-aliased edges in software mode
        const SUB: [f64; 2] = [0.25, 0.75];
        for py in y0..y1 {
            let mut row_hits: Vec<Vec<(f64, i32)>> = vec![Vec::new(); 2];
            for (sy_idx, &sy_off) in SUB.iter().enumerate() {
                let sy = py as f64 + sy_off;
                for f in figs {
                    if f.pts.len() < 3 {
                        continue;
                    }
                    let n = f.pts.len();
                    for i in 0..n {
                        let a = f.pts[i];
                        let b = f.pts[(i + 1) % n];
                        let (y_min, y_max, dir) =
                            if a.y <= b.y { (a.y, b.y, 1) } else { (b.y, a.y, -1) };
                        if sy >= y_min && sy < y_max {
                            let x = a.x + (sy - a.y) * (b.x - a.x) / (b.y - a.y);
                            row_hits[sy_idx].push((x, dir));
                        }
                    }
                }
                row_hits[sy_idx].sort_by(|a, b| a.0.total_cmp(&b.0));
            }
            for px in x0..x1 {
                let mut inside_count = 0u32;
                for sy_idx in 0..2 {
                    for &sx_off in &SUB {
                        let sx = px as f64 + sx_off;
                        let mut wind = 0i32;
                        for &(hx, dir) in &row_hits[sy_idx] {
                            if hx > sx {
                                break;
                            }
                            wind += dir;
                        }
                        if is_inside(wind, fill_mode) {
                            inside_count += 1;
                        }
                    }
                }
                if inside_count > 0 {
                    let cov = inside_count as f32 / 4.0;
                    let col = brush.sample(px as f32 + 0.5, py as f32 + 0.5);
                    self.blend_pixel(px, py, col.r, col.g, col.b, col.a, cov);
                }
            }
        }
    }
}

// ---- Interfaces & COM Objects ----

#[derive(Clone, Debug)]
pub struct ID2D1Factory1 {
    _tag: (),
}

impl Default for ID2D1Factory1 {
    fn default() -> Self {
        Self { _tag: () }
    }
}

impl Interface for ID2D1Factory1 {
    fn as_raw(&self) -> *mut c_void {
        self as *const _ as *mut c_void
    }
}

pub unsafe fn D2D1CreateFactory<T: Default>(
    _factory_type: D2D1_FACTORY_TYPE,
    _options: Option<*const c_void>,
) -> Result<T> {
    Ok(T::default())
}

impl ID2D1Factory1 {
    pub unsafe fn CreateDevice(&self, dxgi: &IDXGIDevice) -> Result<ID2D1Device> {
        Ok(ID2D1Device { _dxgi: dxgi.clone() })
    }

    pub unsafe fn CreateStrokeStyle(
        &self,
        props: &D2D1_STROKE_STYLE_PROPERTIES1,
        _dashes: Option<&[f32]>,
    ) -> Result<ID2D1StrokeStyle1> {
        Ok(ID2D1StrokeStyle1(Arc::new(StrokeStyleData {
            start_cap: props.startCap,
            end_cap: props.endCap,
            line_join: props.lineJoin,
            miter_limit: props.miterLimit,
        })))
    }

    pub unsafe fn CreatePathGeometry(&self) -> Result<ID2D1PathGeometry1> {
        Ok(ID2D1PathGeometry1(Arc::new(Mutex::new(GeometryData::default()))))
    }

    pub unsafe fn CreateRoundedRectangleGeometry(
        &self,
        rr: &D2D1_ROUNDED_RECT,
    ) -> Result<ID2D1RoundedRectangleGeometry> {
        Ok(ID2D1RoundedRectangleGeometry(Arc::new(Mutex::new(rounded_rect_geometry(rr)))))
    }

    pub unsafe fn CreateWicBitmapRenderTarget(
        &self,
        target: &crate::win32_ui::Win32::Graphics::Imaging::IWICBitmap,
        _props: &D2D1_RENDER_TARGET_PROPERTIES,
    ) -> Result<ID2D1RenderTarget> {
        Ok(ID2D1RenderTarget::from_surface(target.surface()))
    }
}

fn rounded_rect_geometry(rr: &D2D1_ROUNDED_RECT) -> GeometryData {
    let l = rr.rect.left as f64;
    let t = rr.rect.top as f64;
    let r = rr.rect.right as f64;
    let b = rr.rect.bottom as f64;
    let w = (r - l).max(0.0);
    let h = (b - t).max(0.0);
    let rx = (rr.radiusX as f64).clamp(0.0, w * 0.5);
    let ry = (rr.radiusY as f64).clamp(0.0, h * 0.5);
    let mut cmds = Vec::new();
    if rx <= 1e-4 || ry <= 1e-4 {
        cmds.push(PathCmd::Move(l, t));
        cmds.push(PathCmd::Line(r, t));
        cmds.push(PathCmd::Line(r, b));
        cmds.push(PathCmd::Line(l, b));
        cmds.push(PathCmd::Close);
    } else {
        let arc = |ex, ey| PathCmd::Arc {
            cx: ex,
            cy: ey,
            rx,
            ry,
            rot_deg: 0.0,
            end_x: ex,
            end_y: ey,
            large: false,
            cw: true,
        };
        cmds.push(PathCmd::Move(l + rx, t));
        cmds.push(PathCmd::Line(r - rx, t));
        cmds.push(arc(r, t + ry));
        cmds.push(PathCmd::Line(r, b - ry));
        cmds.push(arc(r - rx, b));
        cmds.push(PathCmd::Line(l + rx, b));
        cmds.push(arc(l, b - ry));
        cmds.push(PathCmd::Line(l, t + ry));
        cmds.push(arc(l + rx, t));
        cmds.push(PathCmd::Close);
    }
    GeometryData { cmds, fill_mode: D2D1_FILL_MODE_WINDING }
}

#[derive(Clone, Debug)]
pub struct ID2D1Device {
    _dxgi: IDXGIDevice,
}

impl ID2D1Device {
    pub unsafe fn CreateDeviceContext(
        &self,
        _opts: D2D1_DEVICE_CONTEXT_OPTIONS,
    ) -> Result<ID2D1DeviceContext> {
        Ok(ID2D1DeviceContext(ID2D1RenderTarget::new_empty()))
    }
}

#[derive(Clone, Debug)]
pub struct StrokeStyleData {
    pub start_cap: D2D1_CAP_STYLE,
    pub end_cap: D2D1_CAP_STYLE,
    pub line_join: D2D1_LINE_JOIN,
    pub miter_limit: f32,
}

impl Default for StrokeStyleData {
    fn default() -> Self {
        Self {
            start_cap: D2D1_CAP_STYLE_FLAT,
            end_cap: D2D1_CAP_STYLE_FLAT,
            line_join: D2D1_LINE_JOIN_MITER,
            miter_limit: 10.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ID2D1StrokeStyle1(pub Arc<StrokeStyleData>);

#[derive(Clone, Debug)]
pub struct ID2D1StrokeStyle(pub Arc<StrokeStyleData>);

impl Interface for ID2D1StrokeStyle1 {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}

impl CastFrom<ID2D1StrokeStyle> for ID2D1StrokeStyle1 {
    fn cast_into(&self) -> Result<ID2D1StrokeStyle> {
        Ok(ID2D1StrokeStyle(self.0.clone()))
    }
}

pub enum OptStrokeStyle<'a> {
    None,
    Some(&'a StrokeStyleData),
}

impl<'a> From<Option<&'a ID2D1StrokeStyle>> for OptStrokeStyle<'a> {
    fn from(v: Option<&'a ID2D1StrokeStyle>) -> Self {
        match v {
            Some(s) => Self::Some(&s.0),
            None => Self::None,
        }
    }
}

impl<'a> From<&'a ID2D1StrokeStyle1> for OptStrokeStyle<'a> {
    fn from(v: &'a ID2D1StrokeStyle1) -> Self {
        Self::Some(&v.0)
    }
}

impl<'a> From<&'a ID2D1StrokeStyle> for OptStrokeStyle<'a> {
    fn from(v: &'a ID2D1StrokeStyle) -> Self {
        Self::Some(&v.0)
    }
}

// ---- Geometry Types ----

#[derive(Clone, Debug)]
pub struct ID2D1Geometry(pub Arc<Mutex<GeometryData>>);

#[derive(Clone, Debug)]
pub struct ID2D1PathGeometry1(pub Arc<Mutex<GeometryData>>);

#[derive(Clone, Debug)]
pub struct ID2D1RoundedRectangleGeometry(pub Arc<Mutex<GeometryData>>);

impl Interface for ID2D1Geometry {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}
impl Interface for ID2D1PathGeometry1 {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}
impl Interface for ID2D1RoundedRectangleGeometry {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}

impl CastFrom<ID2D1Geometry> for ID2D1PathGeometry1 {
    fn cast_into(&self) -> Result<ID2D1Geometry> {
        Ok(ID2D1Geometry(self.0.clone()))
    }
}

impl CastFrom<ID2D1Geometry> for ID2D1RoundedRectangleGeometry {
    fn cast_into(&self) -> Result<ID2D1Geometry> {
        Ok(ID2D1Geometry(self.0.clone()))
    }
}

pub trait IntoGeometry {
    fn geo_data(&self) -> &Arc<Mutex<GeometryData>>;
}

impl IntoGeometry for ID2D1Geometry {
    fn geo_data(&self) -> &Arc<Mutex<GeometryData>> {
        &self.0
    }
}
impl IntoGeometry for ID2D1PathGeometry1 {
    fn geo_data(&self) -> &Arc<Mutex<GeometryData>> {
        &self.0
    }
}
impl IntoGeometry for ID2D1RoundedRectangleGeometry {
    fn geo_data(&self) -> &Arc<Mutex<GeometryData>> {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ID2D1GeometrySink {
    target: Arc<Mutex<GeometryData>>,
    buf: Arc<Mutex<GeometryData>>,
    in_figure: Arc<Mutex<bool>>,
}

impl ID2D1GeometrySink {
    pub unsafe fn SetFillMode(&self, mode: D2D1_FILL_MODE) {
        if let Ok(mut b) = self.buf.lock() {
            b.fill_mode = mode;
        }
    }

    pub unsafe fn BeginFigure(&self, start: Vector2, _begin: D2D1_FIGURE_BEGIN) {
        if let (Ok(mut b), Ok(mut inf)) = (self.buf.lock(), self.in_figure.lock()) {
            b.cmds.push(PathCmd::Move(start.X as f64, start.Y as f64));
            *inf = true;
        }
    }

    pub unsafe fn AddLine(&self, point: Vector2) {
        if let Ok(mut b) = self.buf.lock() {
            b.cmds.push(PathCmd::Line(point.X as f64, point.Y as f64));
        }
    }

    pub unsafe fn AddQuadraticBezier(&self, seg: &D2D1_QUADRATIC_BEZIER_SEGMENT) {
        if let Ok(mut b) = self.buf.lock() {
            b.cmds.push(PathCmd::Quad(
                seg.point1.X as f64,
                seg.point1.Y as f64,
                seg.point2.X as f64,
                seg.point2.Y as f64,
            ));
        }
    }

    pub unsafe fn AddBezier(&self, seg: &D2D1_BEZIER_SEGMENT) {
        if let Ok(mut b) = self.buf.lock() {
            b.cmds.push(PathCmd::Cubic(
                seg.point1.X as f64,
                seg.point1.Y as f64,
                seg.point2.X as f64,
                seg.point2.Y as f64,
                seg.point3.X as f64,
                seg.point3.Y as f64,
            ));
        }
    }

    pub unsafe fn AddArc(&self, arc: &D2D1_ARC_SEGMENT) {
        if let Ok(mut b) = self.buf.lock() {
            b.cmds.push(PathCmd::Arc {
                cx: arc.point.X as f64,
                cy: arc.point.Y as f64,
                rx: arc.size.width as f64,
                ry: arc.size.height as f64,
                rot_deg: arc.rotationAngle as f64,
                end_x: arc.point.X as f64,
                end_y: arc.point.Y as f64,
                large: arc.arcSize == D2D1_ARC_SIZE_LARGE,
                cw: arc.sweepDirection == D2D1_SWEEP_DIRECTION_CLOCKWISE,
            });
        }
    }

    pub unsafe fn EndFigure(&self, end: D2D1_FIGURE_END) {
        if let (Ok(mut b), Ok(mut inf)) = (self.buf.lock(), self.in_figure.lock()) {
            if *inf && end == D2D1_FIGURE_END_CLOSED {
                b.cmds.push(PathCmd::Close);
            }
            *inf = false;
        }
    }

    pub unsafe fn Close(&self) -> Result<()> {
        let data = self.buf.lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
        *self.target.lock().map_err(|_| Error::from_hresult(E_FAIL))? = data;
        Ok(())
    }
}

macro_rules! impl_geometry_methods {
    ($ty:ty) => {
        impl $ty {
            pub unsafe fn GetBounds(&self, transform: Option<&Matrix3x2>) -> Result<D2D_RECT_F> {
                let g = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
                Ok(g.bounds(transform))
            }

            pub unsafe fn Widen<'a>(
                &self,
                stroke_width: f32,
                stroke_style: impl Into<OptStrokeStyle<'a>>,
                world_transform: Option<&Matrix3x2>,
                flattening_tolerance: f32,
                sink: &ID2D1GeometrySink,
            ) -> Result<()> {
                let g = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
                let style = match stroke_style.into() {
                    OptStrokeStyle::Some(s) => Some(s),
                    OptStrokeStyle::None => None,
                };
                let figs = widen_geometry(&g, stroke_width, style, world_transform, flattening_tolerance);
                write_figs_to_sink(&figs, sink);
                Ok(())
            }

            pub unsafe fn CombineWithGeometry(
                &self,
                input: &impl IntoGeometry,
                mode: D2D1_COMBINE_MODE,
                input_transform: Option<&Matrix3x2>,
                flattening_tolerance: f32,
                sink: &ID2D1GeometrySink,
            ) -> Result<()> {
                let a = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
                let b = input.geo_data().lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
                let figs_a = a.flatten(None, flattening_tolerance);
                let figs_b = b.flatten(input_transform, flattening_tolerance);
                let out = boolean_polygons(&figs_a, a.fill_mode, &figs_b, b.fill_mode, mode);
                write_figs_to_sink(&out, sink);
                Ok(())
            }
        }
    };
}

impl_geometry_methods!(ID2D1Geometry);
impl_geometry_methods!(ID2D1PathGeometry1);
impl_geometry_methods!(ID2D1RoundedRectangleGeometry);

impl ID2D1PathGeometry1 {
    pub unsafe fn Open(&self) -> Result<ID2D1GeometrySink> {
        Ok(ID2D1GeometrySink {
            target: self.0.clone(),
            buf: Arc::new(Mutex::new(GeometryData::default())),
            in_figure: Arc::new(Mutex::new(false)),
        })
    }
}

// ---- Brushes & Bitmaps ----

#[derive(Clone, Debug)]
pub enum BrushData {
    Solid(D2D1_COLOR_F),
    Linear {
        start: Vector2,
        end: Vector2,
        stops: Vec<D2D1_GRADIENT_STOP>,
        opacity: f32,
    },
    Radial {
        center: Vector2,
        origin_offset: Vector2,
        rx: f32,
        ry: f32,
        stops: Vec<D2D1_GRADIENT_STOP>,
        opacity: f32,
    },
    Bitmap {
        surface: Arc<Mutex<PixelSurface>>,
        transform: Matrix3x2,
        opacity: f32,
    },
}

fn sample_stops(stops: &[D2D1_GRADIENT_STOP], mut t: f32, opacity: f32) -> D2D1_COLOR_F {
    if stops.is_empty() {
        return D2D1_COLOR_F::default();
    }
    t = t.clamp(0.0, 1.0);
    if t <= stops[0].position {
        let mut c = stops[0].color;
        c.a *= opacity;
        return c;
    }
    for w in stops.windows(2) {
        if t <= w[1].position {
            let span = (w[1].position - w[0].position).max(1e-6);
            let k = (t - w[0].position) / span;
            let (a, b) = (w[0].color, w[1].color);
            return D2D1_COLOR_F {
                r: a.r + (b.r - a.r) * k,
                g: a.g + (b.g - a.g) * k,
                b: a.b + (b.b - a.b) * k,
                a: (a.a + (b.a - a.a) * k) * opacity,
            };
        }
    }
    let mut c = stops[stops.len() - 1].color;
    c.a *= opacity;
    c
}

impl BrushData {
    pub fn sample(&self, x: f32, y: f32) -> D2D1_COLOR_F {
        match self {
            BrushData::Solid(c) => *c,
            BrushData::Linear { start, end, stops, opacity } => {
                let dx = end.X - start.X;
                let dy = end.Y - start.Y;
                let len2 = (dx * dx + dy * dy).max(1e-6);
                let t = ((x - start.X) * dx + (y - start.Y) * dy) / len2;
                sample_stops(stops, t, *opacity)
            }
            BrushData::Radial { center, rx, ry, stops, opacity, .. } => {
                let nx = (x - center.X) / rx.max(1e-6);
                let ny = (y - center.Y) / ry.max(1e-6);
                let t = (nx * nx + ny * ny).sqrt();
                sample_stops(stops, t, *opacity)
            }
            BrushData::Bitmap { surface, transform, opacity } => {
                let Ok(s) = surface.lock() else { return D2D1_COLOR_F::default() };
                if s.width == 0 || s.height == 0 {
                    return D2D1_COLOR_F::default();
                }
                let det = transform.M11 * transform.M22 - transform.M12 * transform.M21;
                let (sx, sy) = if det.abs() > 1e-8 {
                    let tx = x - transform.M31;
                    let ty = y - transform.M32;
                    (
                        (tx * transform.M22 - ty * transform.M21) / det,
                        (ty * transform.M11 - tx * transform.M12) / det,
                    )
                } else {
                    (x, y)
                };
                let ix = (sx.floor() as i32).clamp(0, s.width as i32 - 1) as usize;
                let iy = (sy.floor() as i32).clamp(0, s.height as i32 - 1) as usize;
                let idx = (iy * s.width as usize + ix) * 4;
                let a = s.pixels[idx + 3] as f32 / 255.0;
                if a <= 1e-5 {
                    return D2D1_COLOR_F::default();
                }
                D2D1_COLOR_F {
                    b: (s.pixels[idx] as f32 / 255.0) / a,
                    g: (s.pixels[idx + 1] as f32 / 255.0) / a,
                    r: (s.pixels[idx + 2] as f32 / 255.0) / a,
                    a: a * *opacity,
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ID2D1Brush(pub Arc<Mutex<BrushData>>);

#[derive(Clone, Debug)]
pub struct ID2D1SolidColorBrush(pub Arc<Mutex<BrushData>>);

#[derive(Clone, Debug)]
pub struct ID2D1LinearGradientBrush(pub Arc<Mutex<BrushData>>);

#[derive(Clone, Debug)]
pub struct ID2D1RadialGradientBrush(pub Arc<Mutex<BrushData>>);

#[derive(Clone, Debug)]
pub struct ID2D1BitmapBrush1(pub Arc<Mutex<BrushData>>);

#[derive(Clone, Debug)]
pub struct ID2D1GradientStopCollection(pub Arc<Vec<D2D1_GRADIENT_STOP>>);

impl ID2D1SolidColorBrush {
    pub unsafe fn SetColor(&self, color: &D2D1_COLOR_F) {
        if let Ok(mut b) = self.0.lock() {
            *b = BrushData::Solid(*color);
        }
    }
}

impl ID2D1LinearGradientBrush {
    pub unsafe fn SetStartPoint(&self, p: Vector2) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Linear { start, .. } = &mut *b {
                *start = p;
            }
        }
    }
    pub unsafe fn SetEndPoint(&self, p: Vector2) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Linear { end, .. } = &mut *b {
                *end = p;
            }
        }
    }
    pub unsafe fn SetOpacity(&self, op: f32) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Linear { opacity, .. } = &mut *b {
                *opacity = op;
            }
        }
    }
}

impl ID2D1RadialGradientBrush {
    pub unsafe fn SetCenter(&self, p: Vector2) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Radial { center, .. } = &mut *b {
                *center = p;
            }
        }
    }
    pub unsafe fn SetRadiusX(&self, r: f32) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Radial { rx, .. } = &mut *b {
                *rx = r;
            }
        }
    }
    pub unsafe fn SetRadiusY(&self, r: f32) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Radial { ry, .. } = &mut *b {
                *ry = r;
            }
        }
    }
    pub unsafe fn SetOpacity(&self, op: f32) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Radial { opacity, .. } = &mut *b {
                *opacity = op;
            }
        }
    }
}

impl ID2D1BitmapBrush1 {
    pub unsafe fn SetInterpolationMode1(&self, _mode: D2D1_INTERPOLATION_MODE) {}
    pub unsafe fn SetOpacity(&self, op: f32) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Bitmap { opacity, .. } = &mut *b {
                *opacity = op;
            }
        }
    }
    pub unsafe fn SetTransform(&self, m: &Matrix3x2) {
        if let Ok(mut b) = self.0.lock() {
            if let BrushData::Bitmap { transform, .. } = &mut *b {
                *transform = *m;
            }
        }
    }
}

pub trait IntoBrush {
    fn brush_data(&self) -> BrushData;
}

macro_rules! impl_into_brush {
    ($($t:ty),*) => {
        $(impl IntoBrush for $t {
            fn brush_data(&self) -> BrushData {
                self.0.lock().map(|g| g.clone()).unwrap_or(BrushData::Solid(D2D1_COLOR_F::default()))
            }
        })*
    };
}

impl_into_brush!(
    ID2D1Brush,
    ID2D1SolidColorBrush,
    ID2D1LinearGradientBrush,
    ID2D1RadialGradientBrush,
    ID2D1BitmapBrush1
);

#[derive(Clone)]
pub struct ID2D1Image(pub Arc<Mutex<PixelSurface>>);

#[derive(Clone)]
pub struct ID2D1Bitmap1(pub Arc<Mutex<PixelSurface>>);

impl Interface for ID2D1Image {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}
impl Interface for ID2D1Bitmap1 {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}

impl CastFrom<ID2D1Image> for ID2D1Bitmap1 {
    fn cast_into(&self) -> Result<ID2D1Image> {
        Ok(ID2D1Image(self.0.clone()))
    }
}

pub enum OptTarget<'a> {
    None,
    Surface(&'a Arc<Mutex<PixelSurface>>),
}

impl<'a> From<Option<&'a ID2D1Image>> for OptTarget<'a> {
    fn from(v: Option<&'a ID2D1Image>) -> Self {
        match v {
            Some(i) => Self::Surface(&i.0),
            None => Self::None,
        }
    }
}

impl<'a> From<&'a ID2D1Bitmap1> for OptTarget<'a> {
    fn from(v: &'a ID2D1Bitmap1) -> Self {
        Self::Surface(&v.0)
    }
}

impl<'a> From<&'a ID2D1Image> for OptTarget<'a> {
    fn from(v: &'a ID2D1Image) -> Self {
        Self::Surface(&v.0)
    }
}

impl ID2D1Bitmap1 {
    pub unsafe fn GetPixelSize(&self) -> D2D_SIZE_U {
        self.0
            .lock()
            .map(|s| D2D_SIZE_U { width: s.width, height: s.height })
            .unwrap_or_default()
    }

    pub unsafe fn GetSize(&self) -> D2D_SIZE_F {
        self.0
            .lock()
            .map(|s| D2D_SIZE_F { width: s.width as f32, height: s.height as f32 })
            .unwrap_or_default()
    }

    pub unsafe fn CopyFromBitmap(
        &self,
        _dest_point: Option<*const D2D_POINT_2U>,
        src: &ID2D1Bitmap1,
        _src_rect: Option<*const D2D_RECT_U>,
    ) -> Result<()> {
        if Arc::ptr_eq(&self.0, &src.0) {
            return Ok(());
        }
        let s = src.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?.pixels.clone();
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        let n = d.pixels.len().min(s.len());
        d.pixels[..n].copy_from_slice(&s[..n]);
        Ok(())
    }

    pub unsafe fn Map(&self, _opts: D2D1_MAP_OPTIONS) -> Result<D2D1_MAPPED_RECT> {
        let mut s = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        let pitch = s.width * 4;
        let bits = s.pixels.as_mut_ptr();
        Ok(D2D1_MAPPED_RECT { pitch, bits })
    }

    pub unsafe fn Unmap(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct ID2D1Layer;

// ---- RenderTarget & DeviceContext ----

struct RtState {
    target: Option<Arc<Mutex<PixelSurface>>>,
    transform: Matrix3x2,
    clips: Vec<D2D_RECT_F>,
    text_aa: D2D1_TEXT_ANTIALIAS_MODE,
    drawing: bool,
    layers: Vec<()>,
}

#[derive(Clone)]
pub struct ID2D1RenderTarget(Arc<Mutex<RtState>>);

#[derive(Clone)]
pub struct ID2D1DeviceContext(pub ID2D1RenderTarget);

impl Deref for ID2D1DeviceContext {
    type Target = ID2D1RenderTarget;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ID2D1RenderTarget {
    fn new_empty() -> Self {
        Self(Arc::new(Mutex::new(RtState {
            target: None,
            transform: Matrix3x2::identity(),
            clips: Vec::new(),
            text_aa: D2D1_TEXT_ANTIALIAS_MODE_DEFAULT,
            drawing: false,
            layers: Vec::new(),
        })))
    }

    pub fn from_surface(surface: Arc<Mutex<PixelSurface>>) -> Self {
        Self(Arc::new(Mutex::new(RtState {
            target: Some(surface),
            transform: Matrix3x2::identity(),
            clips: Vec::new(),
            text_aa: D2D1_TEXT_ANTIALIAS_MODE_DEFAULT,
            drawing: false,
            layers: Vec::new(),
        })))
    }

    pub unsafe fn SetTextAntialiasMode(&self, mode: D2D1_TEXT_ANTIALIAS_MODE) {
        if let Ok(mut st) = self.0.lock() {
            st.text_aa = mode;
        }
    }

    pub unsafe fn CreateSolidColorBrush(
        &self,
        color: &D2D1_COLOR_F,
        _props: Option<&D2D1_BRUSH_PROPERTIES>,
    ) -> Result<ID2D1SolidColorBrush> {
        Ok(ID2D1SolidColorBrush(Arc::new(Mutex::new(BrushData::Solid(*color)))))
    }

    pub unsafe fn CreateGradientStopCollection(
        &self,
        stops: &[D2D1_GRADIENT_STOP],
        _gamma: D2D1_GAMMA,
        _extend: D2D1_EXTEND_MODE,
    ) -> Result<ID2D1GradientStopCollection> {
        Ok(ID2D1GradientStopCollection(Arc::new(stops.to_vec())))
    }

    pub unsafe fn CreateLinearGradientBrush(
        &self,
        props: &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
        brush_props: Option<&D2D1_BRUSH_PROPERTIES>,
        stops: &ID2D1GradientStopCollection,
    ) -> Result<ID2D1LinearGradientBrush> {
        Ok(ID2D1LinearGradientBrush(Arc::new(Mutex::new(BrushData::Linear {
            start: props.startPoint,
            end: props.endPoint,
            stops: stops.0.as_ref().clone(),
            opacity: brush_props.map(|p| p.opacity).unwrap_or(1.0),
        }))))
    }

    pub unsafe fn CreateRadialGradientBrush(
        &self,
        props: &D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES,
        brush_props: Option<&D2D1_BRUSH_PROPERTIES>,
        stops: &ID2D1GradientStopCollection,
    ) -> Result<ID2D1RadialGradientBrush> {
        Ok(ID2D1RadialGradientBrush(Arc::new(Mutex::new(BrushData::Radial {
            center: props.center,
            origin_offset: props.gradientOriginOffset,
            rx: props.radiusX,
            ry: props.radiusY,
            stops: stops.0.as_ref().clone(),
            opacity: brush_props.map(|p| p.opacity).unwrap_or(1.0),
        }))))
    }

    pub unsafe fn BeginDraw(&self) {
        if let Ok(mut st) = self.0.lock() {
            st.drawing = true;
        }
    }

    pub unsafe fn EndDraw(&self, _t1: Option<*mut u64>, _t2: Option<*mut u64>) -> Result<()> {
        if let Ok(mut st) = self.0.lock() {
            st.drawing = false;
            st.clips.clear();
        }
        Ok(())
    }

    pub unsafe fn Clear(&self, color: Option<&D2D1_COLOR_F>) {
        if let Ok(st) = self.0.lock() {
            if let Some(t) = &st.target {
                if let Ok(mut surf) = t.lock() {
                    surf.clear(color);
                }
            }
        }
    }

    pub unsafe fn SetTransform(&self, transform: &Matrix3x2) {
        if let Ok(mut st) = self.0.lock() {
            st.transform = *transform;
        }
    }

    pub unsafe fn GetTransform(&self, transform: &mut Matrix3x2) {
        if let Ok(st) = self.0.lock() {
            *transform = st.transform;
        }
    }

    pub unsafe fn PushAxisAlignedClip(&self, clip_rect: &D2D_RECT_F, _aa: D2D1_ANTIALIAS_MODE) {
        if let Ok(mut st) = self.0.lock() {
            st.clips.push(*clip_rect);
        }
    }

    pub unsafe fn PopAxisAlignedClip(&self) {
        if let Ok(mut st) = self.0.lock() {
            st.clips.pop();
        }
    }

    fn current_clip(st: &RtState) -> Option<D2D_RECT_F> {
        let mut iter = st.clips.iter().copied();
        let first = iter.next()?;
        Some(iter.fold(first, |a, b| D2D_RECT_F {
            left: a.left.max(b.left),
            top: a.top.max(b.top),
            right: a.right.min(b.right),
            bottom: a.bottom.min(b.bottom),
        }))
    }

    pub unsafe fn FillGeometry(
        &self,
        geometry: &impl IntoGeometry,
        brush: &impl IntoBrush,
        _opacity_brush: Option<&ID2D1Brush>,
    ) {
        let (target, clip, xform) = {
            let Ok(st) = self.0.lock() else { return };
            let Some(t) = st.target.clone() else { return };
            (t, Self::current_clip(&st), st.transform)
        };
        let Ok(geo) = geometry.geo_data().lock() else { return };
        let br = brush.brush_data();

        #[cfg(target_os = "macos")]
        {
            if let Ok(mut surf) = target.lock() {
                crate::mac::d2d_fill_geometry(&mut surf, &geo, &br, &xform, clip);
                return;
            }
        }
        let figs = geo.flatten(Some(&xform), D2D1_DEFAULT_FLATTENING_TOLERANCE);
        if let Ok(mut surf) = target.lock() {
            surf.fill_polygons(&figs, geo.fill_mode, &br, &xform, clip);
        };
    }

    pub unsafe fn DrawGeometry<'a>(
        &self,
        geometry: &impl IntoGeometry,
        brush: &impl IntoBrush,
        stroke_width: f32,
        stroke_style: impl Into<OptStrokeStyle<'a>>,
    ) {
        let (target, clip, xform) = {
            let Ok(st) = self.0.lock() else { return };
            let Some(t) = st.target.clone() else { return };
            (t, Self::current_clip(&st), st.transform)
        };
        let Ok(geo) = geometry.geo_data().lock() else { return };
        let br = brush.brush_data();
        let style_opt = match stroke_style.into() {
            OptStrokeStyle::Some(s) => Some(s.clone()),
            OptStrokeStyle::None => None,
        };

        #[cfg(target_os = "macos")]
        {
            if let Ok(mut surf) = target.lock() {
                crate::mac::d2d_draw_geometry(
                    &mut surf,
                    &geo,
                    &br,
                    stroke_width,
                    style_opt.as_ref(),
                    &xform,
                    clip,
                );
                return;
            }
        }
        let figs = widen_geometry(&geo, stroke_width, style_opt.as_ref(), Some(&xform), 0.25);
        if let Ok(mut surf) = target.lock() {
            surf.fill_polygons(&figs, D2D1_FILL_MODE_WINDING, &br, &xform, clip);
        };
    }

    pub unsafe fn FillRectangle(&self, rect: &D2D_RECT_F, brush: &impl IntoBrush) {
        let rr = D2D1_ROUNDED_RECT { rect: *rect, radiusX: 0.0, radiusY: 0.0 };
        self.FillRoundedRectangle(&rr, brush);
    }

    pub unsafe fn FillRoundedRectangle(&self, rr: &D2D1_ROUNDED_RECT, brush: &impl IntoBrush) {
        let geo = ID2D1Geometry(Arc::new(Mutex::new(rounded_rect_geometry(rr))));
        self.FillGeometry(&geo, brush, None);
    }

    pub unsafe fn DrawRoundedRectangle<'a>(
        &self,
        rr: &D2D1_ROUNDED_RECT,
        brush: &impl IntoBrush,
        stroke_width: f32,
        stroke_style: impl Into<OptStrokeStyle<'a>>,
    ) {
        let geo = ID2D1Geometry(Arc::new(Mutex::new(rounded_rect_geometry(rr))));
        self.DrawGeometry(&geo, brush, stroke_width, stroke_style);
    }

    pub unsafe fn FillEllipse(&self, ell: &D2D1_ELLIPSE, brush: &impl IntoBrush) {
        let rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: ell.point.X - ell.radiusX,
                top: ell.point.Y - ell.radiusY,
                right: ell.point.X + ell.radiusX,
                bottom: ell.point.Y + ell.radiusY,
            },
            radiusX: ell.radiusX,
            radiusY: ell.radiusY,
        };
        self.FillRoundedRectangle(&rr, brush);
    }

    pub unsafe fn DrawEllipse<'a>(
        &self,
        ell: &D2D1_ELLIPSE,
        brush: &impl IntoBrush,
        stroke_width: f32,
        stroke_style: impl Into<OptStrokeStyle<'a>>,
    ) {
        let rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: ell.point.X - ell.radiusX,
                top: ell.point.Y - ell.radiusY,
                right: ell.point.X + ell.radiusX,
                bottom: ell.point.Y + ell.radiusY,
            },
            radiusX: ell.radiusX,
            radiusY: ell.radiusY,
        };
        self.DrawRoundedRectangle(&rr, brush, stroke_width, stroke_style);
    }

    pub unsafe fn DrawLine<'a>(
        &self,
        p0: Vector2,
        p1: Vector2,
        brush: &impl IntoBrush,
        stroke_width: f32,
        stroke_style: impl Into<OptStrokeStyle<'a>>,
    ) {
        let geo = ID2D1Geometry(Arc::new(Mutex::new(GeometryData {
            cmds: vec![
                PathCmd::Move(p0.X as f64, p0.Y as f64),
                PathCmd::Line(p1.X as f64, p1.Y as f64),
            ],
            fill_mode: D2D1_FILL_MODE_WINDING,
        })));
        self.DrawGeometry(&geo, brush, stroke_width, stroke_style);
    }

    pub unsafe fn DrawTextLayout(
        &self,
        origin: Vector2,
        layout: &IDWriteTextLayout,
        brush: &impl IntoBrush,
        opts: D2D1_DRAW_TEXT_OPTIONS,
    ) {
        let (target, clip, xform) = {
            let Ok(st) = self.0.lock() else { return };
            let Some(t) = st.target.clone() else { return };
            (t, Self::current_clip(&st), st.transform)
        };
        let color = brush.brush_data().sample(origin.X, origin.Y);
        #[cfg(target_os = "macos")]
        {
            if let Ok(mut surf) = target.lock() {
                crate::mac::d2d_draw_text_layout(
                    &mut surf,
                    origin,
                    layout,
                    color,
                    opts,
                    &xform,
                    clip,
                );
            };
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (target, clip, color, opts, origin, layout, xform);
    }

    pub unsafe fn DrawText(
        &self,
        string: &[u16],
        format: &IDWriteTextFormat,
        rect: &D2D_RECT_F,
        brush: &impl IntoBrush,
        opts: D2D1_DRAW_TEXT_OPTIONS,
        _measuring: DWRITE_MEASURING_MODE,
    ) {
        let (target, clip, xform) = {
            let Ok(st) = self.0.lock() else { return };
            let Some(t) = st.target.clone() else { return };
            (t, Self::current_clip(&st), st.transform)
        };
        let color = brush.brush_data().sample(rect.left, rect.top);
        #[cfg(target_os = "macos")]
        {
            if let Ok(mut surf) = target.lock() {
                crate::mac::d2d_draw_text(
                    &mut surf,
                    string,
                    format,
                    rect,
                    color,
                    opts,
                    &xform,
                    clip,
                );
            };
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (target, clip, color, string, format, rect, opts, xform);
    }
}

impl ID2D1DeviceContext {
    pub unsafe fn SetTarget<'a>(&self, target: impl Into<OptTarget<'a>>) {
        if let Ok(mut st) = (self.0).0.lock() {
            st.target = match target.into() {
                OptTarget::Surface(s) => Some(s.clone()),
                OptTarget::None => None,
            };
        }
    }

    pub unsafe fn CreateBitmap(
        &self,
        size: D2D_SIZE_U,
        src_data: Option<*const c_void>,
        pitch: u32,
        _props: &D2D1_BITMAP_PROPERTIES1,
    ) -> Result<ID2D1Bitmap1> {
        let mut surf = PixelSurface::new(size.width, size.height);
        if let Some(ptr) = src_data {
            if !ptr.is_null() && size.width > 0 && size.height > 0 {
                let dst_stride = size.width as usize * 4;
                let src_stride = (pitch as usize).max(dst_stride);
                let src_bytes = ptr as *const u8;
                for y in 0..size.height as usize {
                    std::ptr::copy_nonoverlapping(
                        src_bytes.add(y * src_stride),
                        surf.pixels.as_mut_ptr().add(y * dst_stride),
                        dst_stride,
                    );
                }
            }
        }
        Ok(ID2D1Bitmap1(Arc::new(Mutex::new(surf))))
    }

    pub unsafe fn CreateBitmapFromDxgiSurface(
        &self,
        surface: &IDXGISurface,
        _props: Option<&D2D1_BITMAP_PROPERTIES1>,
    ) -> Result<ID2D1Bitmap1> {
        Ok(ID2D1Bitmap1(surface.surface()))
    }

    pub unsafe fn CreateBitmapBrush(
        &self,
        bitmap: &ID2D1Bitmap1,
        _bb_props: Option<&D2D1_BITMAP_BRUSH_PROPERTIES1>,
        brush_props: Option<&D2D1_BRUSH_PROPERTIES>,
    ) -> Result<ID2D1BitmapBrush1> {
        Ok(ID2D1BitmapBrush1(Arc::new(Mutex::new(BrushData::Bitmap {
            surface: bitmap.0.clone(),
            transform: brush_props.map(|p| p.transform).unwrap_or_else(Matrix3x2::identity),
            opacity: brush_props.map(|p| p.opacity).unwrap_or(1.0),
        }))))
    }

    pub unsafe fn DrawBitmap(
        &self,
        bmp: &ID2D1Bitmap1,
        dst_rect: Option<&D2D_RECT_F>,
        opacity: f32,
        _interp: D2D1_INTERPOLATION_MODE,
        src_rect: Option<&D2D_RECT_F>,
        _perspective: Option<&[f32; 16]>,
    ) {
        let (target, clip, xform) = {
            let Ok(st) = (self.0).0.lock() else { return };
            let Some(t) = st.target.clone() else { return };
            (t, ID2D1RenderTarget::current_clip(&st), st.transform)
        };
        let Ok(src_surf) = bmp.0.lock() else { return };
        if src_surf.width == 0 || src_surf.height == 0 {
            return;
        }
        let dst = dst_rect.copied().unwrap_or(D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: src_surf.width as f32,
            bottom: src_surf.height as f32,
        });

        #[cfg(target_os = "macos")]
        {
            if let Ok(mut dst_surf) = target.lock() {
                crate::mac::d2d_draw_bitmap(
                    &mut dst_surf,
                    &src_surf,
                    dst,
                    opacity,
                    src_rect,
                    &xform,
                    clip,
                );
                return;
            }
        }
        let _ = src_rect;
        if let Ok(mut dst_surf) = target.lock() {
            let p0 = transform_pt(Some(&xform), dst.left as f64, dst.top as f64);
            let p1 = transform_pt(Some(&xform), dst.right as f64, dst.bottom as f64);
            let w = (p1.x - p0.x).max(1.0);
            let h = (p1.y - p0.y).max(1.0);
            let mut x0 = (p0.x.floor() as i32).max(0);
            let mut y0 = (p0.y.floor() as i32).max(0);
            let mut x1 = (p1.x.ceil() as i32).min(dst_surf.width as i32);
            let mut y1 = (p1.y.ceil() as i32).min(dst_surf.height as i32);
            if let Some(c) = clip {
                let cp0 = transform_pt(Some(&xform), c.left as f64, c.top as f64);
                let cp1 = transform_pt(Some(&xform), c.right as f64, c.bottom as f64);
                x0 = x0.max(cp0.x.min(cp1.x).floor() as i32);
                y0 = y0.max(cp0.y.min(cp1.y).floor() as i32);
                x1 = x1.min(cp0.x.max(cp1.x).ceil() as i32);
                y1 = y1.min(cp0.y.max(cp1.y).ceil() as i32);
            }
            let op = opacity.clamp(0.0, 1.0);
            for py in y0..y1 {
                let v = (((py as f64 + 0.5 - p0.y) / h) * src_surf.height as f64)
                    .clamp(0.0, (src_surf.height - 1) as f64) as usize;
                for px in x0..x1 {
                    let u = (((px as f64 + 0.5 - p0.x) / w) * src_surf.width as f64)
                        .clamp(0.0, (src_surf.width - 1) as f64) as usize;
                    let sidx = (v * src_surf.width as usize + u) * 4;
                    let sa = (src_surf.pixels[sidx + 3] as f32 / 255.0) * op;
                    if sa <= 0.0 {
                        continue;
                    }
                    let sb = (src_surf.pixels[sidx] as f32 / 255.0) * op;
                    let sg = (src_surf.pixels[sidx + 1] as f32 / 255.0) * op;
                    let sr = (src_surf.pixels[sidx + 2] as f32 / 255.0) * op;
                    let didx = ((py as usize) * (dst_surf.width as usize) + (px as usize)) * 4;
                    let inv = 1.0 - sa;
                    dst_surf.pixels[didx] =
                        ((sb + (dst_surf.pixels[didx] as f32 / 255.0) * inv).clamp(0.0, 1.0) * 255.0)
                            .round() as u8;
                    dst_surf.pixels[didx + 1] =
                        ((sg + (dst_surf.pixels[didx + 1] as f32 / 255.0) * inv).clamp(0.0, 1.0)
                            * 255.0)
                            .round() as u8;
                    dst_surf.pixels[didx + 2] =
                        ((sr + (dst_surf.pixels[didx + 2] as f32 / 255.0) * inv).clamp(0.0, 1.0)
                            * 255.0)
                            .round() as u8;
                    dst_surf.pixels[didx + 3] =
                        ((sa + (dst_surf.pixels[didx + 3] as f32 / 255.0) * inv).clamp(0.0, 1.0)
                            * 255.0)
                            .round() as u8;
                }
            }
        };
    }

    pub unsafe fn PushLayer(&self, params: &D2D1_LAYER_PARAMETERS1, _layer: Option<&ID2D1Layer>) {
        if let Ok(mut st) = (self.0).0.lock() {
            #[cfg(target_os = "macos")]
            if let Some(t) = &st.target {
                if let Ok(mut surf) = t.lock() {
                    let mask = params.geometricMask.as_ref().and_then(|g| g.0.lock().ok().map(|d| d.clone()));
                    crate::mac::d2d_push_layer(&mut surf, params.contentBounds, mask.as_ref(), &params.maskTransform, params.opacity);
                }
            }
            let _ = params;
            st.layers.push(());
        }
    }

    pub unsafe fn PopLayer(&self) {
        if let Ok(mut st) = (self.0).0.lock() {
            #[cfg(target_os = "macos")]
            if let Some(t) = &st.target {
                if let Ok(mut surf) = t.lock() {
                    crate::mac::d2d_pop_layer(&mut surf);
                }
            }
            st.layers.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_fig(x: f64, y: f64, w: f64, h: f64) -> PolyFigure {
        PolyFigure {
            pts: vec![Pt2::new(x, y), Pt2::new(x + w, y), Pt2::new(x + w, y + h), Pt2::new(x, y + h)],
            closed: true,
        }
    }

    #[test]
    fn boolean_intersect_and_exclude_produce_valid_bounds() {
        let a = vec![rect_fig(0.0, 0.0, 20.0, 20.0)];
        let b = vec![rect_fig(10.0, 5.0, 20.0, 10.0)];

        let inter = boolean_polygons(
            &a,
            D2D1_FILL_MODE_WINDING,
            &b,
            D2D1_FILL_MODE_WINDING,
            D2D1_COMBINE_MODE_INTERSECT,
        );
        assert_eq!(inter.len(), 1);
        let inter_geo = GeometryData {
            cmds: vec![
                PathCmd::Move(inter[0].pts[0].x, inter[0].pts[0].y),
                PathCmd::Line(inter[0].pts[1].x, inter[0].pts[1].y),
                PathCmd::Line(inter[0].pts[2].x, inter[0].pts[2].y),
                PathCmd::Line(inter[0].pts[3].x, inter[0].pts[3].y),
                PathCmd::Close,
            ],
            fill_mode: D2D1_FILL_MODE_WINDING,
        };
        let bounds = inter_geo.bounds(None);
        assert!((bounds.left - 10.0).abs() < 0.01);
        assert!((bounds.top - 5.0).abs() < 0.01);
        assert!((bounds.right - 20.0).abs() < 0.01);
        assert!((bounds.bottom - 15.0).abs() < 0.01);

        let excl = boolean_polygons(
            &a,
            D2D1_FILL_MODE_WINDING,
            &b,
            D2D1_FILL_MODE_WINDING,
            D2D1_COMBINE_MODE_EXCLUDE,
        );
        assert!(!excl.is_empty());
    }
}
