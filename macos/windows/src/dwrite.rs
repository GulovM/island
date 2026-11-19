#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::too_many_arguments)]

use std::ffi::c_void;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use windows_core::{CastFrom, Error, Interface, PCWSTR, Result, BOOL};

use crate::foundation::E_FAIL;

// ---- Constants & Enums ----

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_FACTORY_TYPE(pub i32);
pub const DWRITE_FACTORY_TYPE_SHARED: DWRITE_FACTORY_TYPE = DWRITE_FACTORY_TYPE(0);
pub const DWRITE_FACTORY_TYPE_ISOLATED: DWRITE_FACTORY_TYPE = DWRITE_FACTORY_TYPE(1);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_MEASURING_MODE(pub i32);
pub const DWRITE_MEASURING_MODE_NATURAL: DWRITE_MEASURING_MODE = DWRITE_MEASURING_MODE(0);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_RENDERING_MODE(pub i32);
pub const DWRITE_RENDERING_MODE_DEFAULT: DWRITE_RENDERING_MODE = DWRITE_RENDERING_MODE(0);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DWRITE_FONT_WEIGHT(pub i32);
pub const DWRITE_FONT_WEIGHT_THIN: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(100);
pub const DWRITE_FONT_WEIGHT_EXTRA_LIGHT: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(200);
pub const DWRITE_FONT_WEIGHT_LIGHT: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(300);
pub const DWRITE_FONT_WEIGHT_NORMAL: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(400);
pub const DWRITE_FONT_WEIGHT_REGULAR: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(400);
pub const DWRITE_FONT_WEIGHT_MEDIUM: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(500);
pub const DWRITE_FONT_WEIGHT_SEMI_BOLD: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(600);
pub const DWRITE_FONT_WEIGHT_BOLD: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(700);
pub const DWRITE_FONT_WEIGHT_EXTRA_BOLD: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(800);
pub const DWRITE_FONT_WEIGHT_BLACK: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(900);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_FONT_STYLE(pub i32);
pub const DWRITE_FONT_STYLE_NORMAL: DWRITE_FONT_STYLE = DWRITE_FONT_STYLE(0);
pub const DWRITE_FONT_STYLE_OBLIQUE: DWRITE_FONT_STYLE = DWRITE_FONT_STYLE(1);
pub const DWRITE_FONT_STYLE_ITALIC: DWRITE_FONT_STYLE = DWRITE_FONT_STYLE(2);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_FONT_STRETCH(pub i32);
pub const DWRITE_FONT_STRETCH_NORMAL: DWRITE_FONT_STRETCH = DWRITE_FONT_STRETCH(5);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_TEXT_ALIGNMENT(pub i32);
pub const DWRITE_TEXT_ALIGNMENT_LEADING: DWRITE_TEXT_ALIGNMENT = DWRITE_TEXT_ALIGNMENT(0);
pub const DWRITE_TEXT_ALIGNMENT_TRAILING: DWRITE_TEXT_ALIGNMENT = DWRITE_TEXT_ALIGNMENT(1);
pub const DWRITE_TEXT_ALIGNMENT_CENTER: DWRITE_TEXT_ALIGNMENT = DWRITE_TEXT_ALIGNMENT(2);
pub const DWRITE_TEXT_ALIGNMENT_JUSTIFIED: DWRITE_TEXT_ALIGNMENT = DWRITE_TEXT_ALIGNMENT(3);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_PARAGRAPH_ALIGNMENT(pub i32);
pub const DWRITE_PARAGRAPH_ALIGNMENT_NEAR: DWRITE_PARAGRAPH_ALIGNMENT =
    DWRITE_PARAGRAPH_ALIGNMENT(0);
pub const DWRITE_PARAGRAPH_ALIGNMENT_FAR: DWRITE_PARAGRAPH_ALIGNMENT = DWRITE_PARAGRAPH_ALIGNMENT(1);
pub const DWRITE_PARAGRAPH_ALIGNMENT_CENTER: DWRITE_PARAGRAPH_ALIGNMENT =
    DWRITE_PARAGRAPH_ALIGNMENT(2);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_WORD_WRAPPING(pub i32);
pub const DWRITE_WORD_WRAPPING_WRAP: DWRITE_WORD_WRAPPING = DWRITE_WORD_WRAPPING(0);
pub const DWRITE_WORD_WRAPPING_NO_WRAP: DWRITE_WORD_WRAPPING = DWRITE_WORD_WRAPPING(1);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_TRIMMING_GRANULARITY(pub i32);
pub const DWRITE_TRIMMING_GRANULARITY_NONE: DWRITE_TRIMMING_GRANULARITY =
    DWRITE_TRIMMING_GRANULARITY(0);
pub const DWRITE_TRIMMING_GRANULARITY_CHARACTER: DWRITE_TRIMMING_GRANULARITY =
    DWRITE_TRIMMING_GRANULARITY(1);
pub const DWRITE_TRIMMING_GRANULARITY_WORD: DWRITE_TRIMMING_GRANULARITY =
    DWRITE_TRIMMING_GRANULARITY(2);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_FONT_AXIS_TAG(pub u32);
pub const DWRITE_FONT_AXIS_TAG_WEIGHT: DWRITE_FONT_AXIS_TAG = DWRITE_FONT_AXIS_TAG(0x7468_6777);
pub const DWRITE_FONT_AXIS_TAG_OPTICAL_SIZE: DWRITE_FONT_AXIS_TAG =
    DWRITE_FONT_AXIS_TAG(0x7A73_706F);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_FONT_FEATURE_TAG(pub u32);
pub const DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES: DWRITE_FONT_FEATURE_TAG =
    DWRITE_FONT_FEATURE_TAG(0x6D75_6E74);

// ---- Structs ----

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DWRITE_FONT_AXIS_VALUE {
    pub axisTag: DWRITE_FONT_AXIS_TAG,
    pub value: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_FONT_FEATURE {
    pub nameTag: DWRITE_FONT_FEATURE_TAG,
    pub parameter: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_TEXT_RANGE {
    pub startPosition: u32,
    pub length: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DWRITE_TRIMMING {
    pub granularity: DWRITE_TRIMMING_GRANULARITY,
    pub delimiter: u32,
    pub delimiterCount: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DWRITE_TEXT_METRICS {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub widthIncludingTrailingWhitespace: f32,
    pub height: f32,
    pub layoutWidth: f32,
    pub layoutHeight: f32,
    pub maxBidiReorderingDepth: u32,
    pub lineCount: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DWRITE_LINE_METRICS {
    pub length: u32,
    pub trailingWhitespaceLength: u32,
    pub newlineLength: u32,
    pub height: f32,
    pub baseline: f32,
    pub isTrimmed: BOOL,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DWRITE_HIT_TEST_METRICS {
    pub textPosition: u32,
    pub length: u32,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub bidiLevel: u32,
    pub isText: BOOL,
    pub isTrimmed: BOOL,
}

// ---- Factory & Collections ----

#[derive(Clone, Debug, Default)]
pub struct IDWriteFactory;

#[derive(Clone, Debug, Default)]
pub struct IDWriteFactory6;

impl Interface for IDWriteFactory {
    fn as_raw(&self) -> *mut c_void {
        self as *const _ as *mut c_void
    }
}
impl Interface for IDWriteFactory6 {
    fn as_raw(&self) -> *mut c_void {
        self as *const _ as *mut c_void
    }
}

impl CastFrom<IDWriteFactory6> for IDWriteFactory {
    fn cast_into(&self) -> Result<IDWriteFactory6> {
        Ok(IDWriteFactory6)
    }
}

impl CastFrom<IDWriteFactory> for IDWriteFactory6 {
    fn cast_into(&self) -> Result<IDWriteFactory> {
        Ok(IDWriteFactory)
    }
}

pub unsafe fn DWriteCreateFactory<T: Default>(
    _factory_type: DWRITE_FACTORY_TYPE,
) -> Result<T> {
    Ok(T::default())
}

#[derive(Clone, Debug, Default)]
pub struct IDWriteFontCollection;

impl IDWriteFontCollection {
    pub unsafe fn FindFamilyName(
        &self,
        family_name: PCWSTR,
        index: &mut u32,
        exists: &mut BOOL,
    ) -> Result<()> {
        let name = family_name.to_string().unwrap_or_default();
        #[cfg(target_os = "macos")]
        let found = crate::mac::dwrite_font_exists(&name);
        #[cfg(not(target_os = "macos"))]
        let found = matches!(
            name.as_str(),
            "Inter"
                | "Segoe UI"
                | "Segoe UI Variable"
                | "Segoe UI Variable Text"
                | "Segoe UI Variable Display"
                | "Segoe UI Variable Small"
                | "Segoe Fluent Icons"
                | "Segoe MDL2 Assets"
                | "Consolas"
                | "SF Pro"
                | "SF Pro Text"
                | "SF Pro Display"
                | ".AppleSystemUIFont"
                | "Menlo"
                | "SF Mono"
                | "Helvetica Neue"
                | "Arial"
        );
        *index = 0;
        *exists = BOOL::from(found);
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct IDWriteInlineObject;

#[derive(Clone, Debug, Default)]
pub struct IDWriteTypography {
    pub features: Arc<Mutex<Vec<DWRITE_FONT_FEATURE>>>,
}

impl IDWriteTypography {
    pub unsafe fn AddFontFeature(&self, feature: DWRITE_FONT_FEATURE) -> Result<()> {
        if let Ok(mut f) = self.features.lock() {
            f.push(feature);
        }
        Ok(())
    }
}

// ---- TextFormat ----

#[derive(Clone, Debug)]
pub struct TextFormatData {
    pub family: String,
    pub font_size: f32,
    pub weight: DWRITE_FONT_WEIGHT,
    pub style: DWRITE_FONT_STYLE,
    pub axes: Vec<DWRITE_FONT_AXIS_VALUE>,
    pub text_align: DWRITE_TEXT_ALIGNMENT,
    pub para_align: DWRITE_PARAGRAPH_ALIGNMENT,
    pub wrapping: DWRITE_WORD_WRAPPING,
    pub trimming: DWRITE_TRIMMING,
}

#[derive(Clone, Debug)]
pub struct IDWriteTextFormat(pub Arc<Mutex<TextFormatData>>);

#[derive(Clone, Debug)]
pub struct IDWriteTextFormat3(pub Arc<Mutex<TextFormatData>>);

impl Deref for IDWriteTextFormat3 {
    type Target = IDWriteTextFormat;
    fn deref(&self) -> &Self::Target {
        // Both are single-field newtypes wrapping the identical Arc layout.
        unsafe { &*(self as *const IDWriteTextFormat3 as *const IDWriteTextFormat) }
    }
}

impl Interface for IDWriteTextFormat {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}
impl Interface for IDWriteTextFormat3 {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}

impl CastFrom<IDWriteTextFormat3> for IDWriteTextFormat {
    fn cast_into(&self) -> Result<IDWriteTextFormat3> {
        Ok(IDWriteTextFormat3(self.0.clone()))
    }
}
impl CastFrom<IDWriteTextFormat> for IDWriteTextFormat3 {
    fn cast_into(&self) -> Result<IDWriteTextFormat> {
        Ok(IDWriteTextFormat(self.0.clone()))
    }
}

pub trait OptInlineObject {
    fn into_opt(self) -> Option<IDWriteInlineObject>;
}
impl OptInlineObject for &IDWriteInlineObject {
    fn into_opt(self) -> Option<IDWriteInlineObject> {
        Some(self.clone())
    }
}
impl OptInlineObject for Option<&IDWriteInlineObject> {
    fn into_opt(self) -> Option<IDWriteInlineObject> {
        self.cloned()
    }
}

impl IDWriteTextFormat {
    pub unsafe fn SetTextAlignment(&self, align: DWRITE_TEXT_ALIGNMENT) -> Result<()> {
        if let Ok(mut d) = self.0.lock() {
            d.text_align = align;
        }
        Ok(())
    }

    pub unsafe fn SetParagraphAlignment(&self, align: DWRITE_PARAGRAPH_ALIGNMENT) -> Result<()> {
        if let Ok(mut d) = self.0.lock() {
            d.para_align = align;
        }
        Ok(())
    }

    pub unsafe fn SetWordWrapping(&self, wrap: DWRITE_WORD_WRAPPING) -> Result<()> {
        if let Ok(mut d) = self.0.lock() {
            d.wrapping = wrap;
        }
        Ok(())
    }

    pub unsafe fn GetWordWrapping(&self) -> DWRITE_WORD_WRAPPING {
        self.0.lock().map(|d| d.wrapping).unwrap_or(DWRITE_WORD_WRAPPING_WRAP)
    }

    pub unsafe fn SetTrimming<P: OptInlineObject>(
        &self,
        trimming: &DWRITE_TRIMMING,
        _sign: P,
    ) -> Result<()> {
        if let Ok(mut d) = self.0.lock() {
            d.trimming = *trimming;
        }
        Ok(())
    }

    pub unsafe fn GetFontSize(&self) -> f32 {
        self.0.lock().map(|d| d.font_size).unwrap_or(12.0)
    }

    pub unsafe fn GetFontWeight(&self) -> DWRITE_FONT_WEIGHT {
        self.0
            .lock()
            .map(|d| {
                d.axes
                    .iter()
                    .find(|a| a.axisTag == DWRITE_FONT_AXIS_TAG_WEIGHT)
                    .map(|a| DWRITE_FONT_WEIGHT(a.value.round() as i32))
                    .unwrap_or(d.weight)
            })
            .unwrap_or(DWRITE_FONT_WEIGHT_REGULAR)
    }

    pub unsafe fn GetFontFamilyName(&self, buf: &mut [u16]) -> Result<()> {
        if buf.is_empty() {
            return Err(Error::from_hresult(E_FAIL));
        }
        let d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        let mut i = 0usize;
        for u in d.family.encode_utf16() {
            if i + 1 >= buf.len() {
                break;
            }
            buf[i] = u;
            i += 1;
        }
        buf[i] = 0;
        Ok(())
    }
}

impl IDWriteTextFormat3 {
    pub unsafe fn SetFontAxisValues(&self, values: &[DWRITE_FONT_AXIS_VALUE]) -> Result<()> {
        if let Ok(mut d) = self.0.lock() {
            d.axes = values.to_vec();
            if let Some(w) = values.iter().find(|a| a.axisTag == DWRITE_FONT_AXIS_TAG_WEIGHT) {
                d.weight = DWRITE_FONT_WEIGHT(w.value.round() as i32);
            }
        }
        Ok(())
    }

    pub unsafe fn GetFontAxisValueCount(&self) -> u32 {
        self.0.lock().map(|d| d.axes.len() as u32).unwrap_or(0)
    }

    pub unsafe fn GetFontAxisValues(&self, out: &mut [DWRITE_FONT_AXIS_VALUE]) -> Result<()> {
        let d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        let n = out.len().min(d.axes.len());
        out[..n].copy_from_slice(&d.axes[..n]);
        Ok(())
    }
}

impl IDWriteFactory {
    pub unsafe fn GetSystemFontCollection(
        &self,
        collection: &mut Option<IDWriteFontCollection>,
        _check_for_updates: bool,
    ) -> Result<()> {
        *collection = Some(IDWriteFontCollection);
        Ok(())
    }

    pub unsafe fn CreateTextFormat(
        &self,
        font_family_name: PCWSTR,
        _font_collection: Option<&IDWriteFontCollection>,
        font_weight: DWRITE_FONT_WEIGHT,
        font_style: DWRITE_FONT_STYLE,
        _font_stretch: DWRITE_FONT_STRETCH,
        font_size: f32,
        _locale_name: PCWSTR,
    ) -> Result<IDWriteTextFormat> {
        let family = font_family_name.to_string().unwrap_or_else(|_| "Inter".to_string());
        let axes = vec![DWRITE_FONT_AXIS_VALUE {
            axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT,
            value: font_weight.0 as f32,
        }];
        Ok(IDWriteTextFormat(Arc::new(Mutex::new(TextFormatData {
            family,
            font_size,
            weight: font_weight,
            style: font_style,
            axes,
            text_align: DWRITE_TEXT_ALIGNMENT_LEADING,
            para_align: DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            wrapping: DWRITE_WORD_WRAPPING_WRAP,
            trimming: DWRITE_TRIMMING::default(),
        }))))
    }

    pub unsafe fn CreateEllipsisTrimmingSign(
        &self,
        _format: &IDWriteTextFormat,
    ) -> Result<IDWriteInlineObject> {
        Ok(IDWriteInlineObject)
    }

    pub unsafe fn CreateTypography(&self) -> Result<IDWriteTypography> {
        Ok(IDWriteTypography::default())
    }

    pub unsafe fn CreateTextLayout(
        &self,
        string: &[u16],
        format: &IDWriteTextFormat,
        max_width: f32,
        max_height: f32,
    ) -> Result<IDWriteTextLayout> {
        let text = String::from_utf16_lossy(string);
        let fmt = format.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?.clone();
        Ok(IDWriteTextLayout(Arc::new(Mutex::new(TextLayoutData {
            text,
            fmt,
            max_width: max_width.max(0.0),
            max_height: max_height.max(0.0),
            trailing_spacing: 0.0,
            leading_spacing: 0.0,
            weight_spans: Vec::new(),
            strike_spans: Vec::new(),
            tabular: false,
            cached_metrics: None,
        }))))
    }
}

impl IDWriteFactory6 {
    pub unsafe fn CreateTextFormat(
        &self,
        font_family_name: PCWSTR,
        _font_collection: Option<&IDWriteFontCollection>,
        axis_values: &[DWRITE_FONT_AXIS_VALUE],
        font_size: f32,
        _locale_name: PCWSTR,
    ) -> Result<IDWriteTextFormat3> {
        let family = font_family_name.to_string().unwrap_or_else(|_| "Inter".to_string());
        let weight = axis_values
            .iter()
            .find(|a| a.axisTag == DWRITE_FONT_AXIS_TAG_WEIGHT)
            .map(|a| DWRITE_FONT_WEIGHT(a.value.round() as i32))
            .unwrap_or(DWRITE_FONT_WEIGHT_REGULAR);
        Ok(IDWriteTextFormat3(Arc::new(Mutex::new(TextFormatData {
            family,
            font_size,
            weight,
            style: DWRITE_FONT_STYLE_NORMAL,
            axes: axis_values.to_vec(),
            text_align: DWRITE_TEXT_ALIGNMENT_LEADING,
            para_align: DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            wrapping: DWRITE_WORD_WRAPPING_WRAP,
            trimming: DWRITE_TRIMMING::default(),
        }))))
    }
}

// ---- TextLayout & Metrics ----

#[derive(Clone, Debug)]
pub struct WeightSpan {
    pub start: u32,
    pub len: u32,
    pub weight: DWRITE_FONT_WEIGHT,
}

#[derive(Clone, Debug)]
pub struct StrikeSpan {
    pub start: u32,
    pub len: u32,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct LayoutLineInfo {
    pub start_u16: u32,
    pub len_u16: u32,
    pub trailing_ws_u16: u32,
    pub newline_u16: u32,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

#[derive(Clone, Debug)]
pub struct GlyphPos {
    pub u16_pos: u32,
    pub u16_len: u32,
    pub line_idx: usize,
    pub x: f32,
    pub width: f32,
}

#[derive(Clone, Debug)]
pub struct ComputedLayout {
    pub metrics: DWRITE_TEXT_METRICS,
    pub lines: Vec<LayoutLineInfo>,
    pub glyphs: Vec<GlyphPos>,
}

#[derive(Clone, Debug)]
pub struct TextLayoutData {
    pub text: String,
    pub fmt: TextFormatData,
    pub max_width: f32,
    pub max_height: f32,
    pub leading_spacing: f32,
    pub trailing_spacing: f32,
    pub weight_spans: Vec<WeightSpan>,
    pub strike_spans: Vec<StrikeSpan>,
    pub tabular: bool,
    pub cached_metrics: Option<ComputedLayout>,
}

impl TextLayoutData {
    pub fn weight_at(&self, pos_u16: u32) -> DWRITE_FONT_WEIGHT {
        for s in self.weight_spans.iter().rev() {
            if pos_u16 >= s.start && pos_u16 < s.start.saturating_add(s.len) {
                return s.weight;
            }
        }
        self.fmt.weight
    }

    pub fn is_monospace(&self) -> bool {
        let f = self.fmt.family.to_ascii_lowercase();
        f.contains("consolas") || f.contains("mono") || f.contains("menlo")
    }

    /// Computes realistic per-character widths & wrapping (uses CoreText on macOS when available).
    pub fn compute(&mut self) -> ComputedLayout {
        if let Some(c) = &self.cached_metrics {
            return c.clone();
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(c) = unsafe { crate::mac::dwrite_compute_layout(self) } {
                self.cached_metrics = Some(c.clone());
                return c;
            }
        }
        let c = self.compute_fallback();
        self.cached_metrics = Some(c.clone());
        c
    }

    fn char_advance(&self, ch: char, pos_u16: u32) -> f32 {
        let sz = self.fmt.font_size.max(1.0);
        let weight = self.weight_at(pos_u16).0 as f32;
        let bold_k = if weight >= 580.0 { 1.03 } else { 1.0 };
        let base = if self.is_monospace() || (self.tabular && ch.is_ascii_digit()) {
            0.555
        } else {
            match ch {
                ' ' | '\u{00A0}' | '\u{2009}' => 0.27,
                'i' | 'l' | 'I' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' | '·' => 0.26,
                't' | 'f' | 'r' | 'j' | '(' | ')' | '[' | ']' | '-' | '/' => 0.34,
                '0'..='9' => 0.54,
                'm' | 'M' | 'w' | 'W' | 'Ж' | 'Ш' | 'Щ' | 'Ю' | 'Ф' | 'ж' | 'ш' | 'щ' | 'ю' => 0.78,
                c if c.is_ascii_uppercase() => 0.65,
                c if c as u32 >= 0x1F300 => 1.05, // emoji
                c if ('А'..='Я').contains(&c) => 0.62,
                c if ('а'..='я').contains(&c) || c == 'ё' || c == 'Ё' => 0.51,
                _ => 0.52,
            }
        };
        (base * sz * bold_k + self.leading_spacing + self.trailing_spacing).max(0.0)
    }

    fn compute_fallback(&self) -> ComputedLayout {
        let sz = self.fmt.font_size.max(1.0);
        let line_h = sz * 1.32;
        let baseline = sz * 1.04;
        let wrap = self.fmt.wrapping != DWRITE_WORD_WRAPPING_NO_WRAP && self.max_width > 0.0;

        #[derive(Clone, Copy)]
        struct Item {
            ch: char,
            u16_pos: u32,
            u16_len: u32,
            adv: f32,
        }

        let mut items = Vec::new();
        let mut u16_pos = 0u32;
        for ch in self.text.chars() {
            let u16_len = ch.len_utf16() as u32;
            let adv = if ch == '\n' || ch == '\r' { 0.0 } else { self.char_advance(ch, u16_pos) };
            items.push(Item { ch, u16_pos, u16_len, adv });
            u16_pos += u16_len;
        }

        let mut lines: Vec<LayoutLineInfo> = Vec::new();
        let mut glyphs: Vec<GlyphPos> = Vec::new();
        let mut i = 0usize;

        if items.is_empty() {
            lines.push(LayoutLineInfo {
                start_u16: 0,
                len_u16: 0,
                trailing_ws_u16: 0,
                newline_u16: 0,
                width: 0.0,
                height: line_h,
                baseline,
            });
        }

        while i < items.len() {
            let line_start = i;
            let line_start_u16 = items[i].u16_pos;
            let mut x = 0.0f32;
            let mut last_space: Option<usize> = None;
            let mut newline_len = 0u32;

            while i < items.len() {
                let it = items[i];
                if it.ch == '\r' {
                    newline_len += it.u16_len;
                    i += 1;
                    if i < items.len() && items[i].ch == '\n' {
                        newline_len += items[i].u16_len;
                        i += 1;
                    }
                    break;
                }
                if it.ch == '\n' {
                    newline_len = it.u16_len;
                    i += 1;
                    break;
                }
                if wrap && i > line_start && x + it.adv > self.max_width + 0.01 {
                    if let Some(sp) = last_space {
                        i = sp + 1;
                    }
                    break;
                }
                if it.ch == ' ' {
                    last_space = Some(i);
                }
                x += it.adv;
                i += 1;
            }

            let line_end_u16 =
                if i < items.len() { items[i].u16_pos } else { u16_pos };
            let content_end = i - if newline_len > 0 { 1 } else { 0 };
            let mut trail_idx = content_end;
            while trail_idx > line_start && items[trail_idx - 1].ch == ' ' {
                trail_idx -= 1;
            }
            let trailing_ws_u16: u32 =
                items[trail_idx..content_end].iter().map(|it| it.u16_len).sum();
            let line_w_no_ws: f32 = items[line_start..trail_idx].iter().map(|it| it.adv).sum();
            let line_w_full: f32 = items[line_start..content_end].iter().map(|it| it.adv).sum();

            let x_off = match self.fmt.text_align {
                DWRITE_TEXT_ALIGNMENT_CENTER if self.max_width > 0.0 => {
                    ((self.max_width - line_w_no_ws) * 0.5).max(0.0)
                }
                DWRITE_TEXT_ALIGNMENT_TRAILING if self.max_width > 0.0 => {
                    (self.max_width - line_w_no_ws).max(0.0)
                }
                _ => 0.0,
            };

            let lidx = lines.len();
            let mut gx = x_off;
            for it in &items[line_start..content_end] {
                glyphs.push(GlyphPos {
                    u16_pos: it.u16_pos,
                    u16_len: it.u16_len,
                    line_idx: lidx,
                    x: gx,
                    width: it.adv,
                });
                gx += it.adv;
            }

            let _ = line_w_full;
            lines.push(LayoutLineInfo {
                start_u16: line_start_u16,
                len_u16: line_end_u16 - line_start_u16,
                trailing_ws_u16,
                newline_u16: newline_len,
                width: line_w_no_ws,
                height: line_h,
                baseline,
            });
        }

        let max_line_w = lines.iter().map(|l| l.width).fold(0.0f32, f32::max);
        let full_w = if lines.len() == 1 {
            items.iter().filter(|it| it.ch != '\n' && it.ch != '\r').map(|it| it.adv).sum()
        } else {
            max_line_w
        };
        let total_h = lines.iter().map(|l| l.height).sum();
        let left = match self.fmt.text_align {
            DWRITE_TEXT_ALIGNMENT_CENTER if self.max_width > 0.0 => {
                ((self.max_width - max_line_w) * 0.5).max(0.0)
            }
            DWRITE_TEXT_ALIGNMENT_TRAILING if self.max_width > 0.0 => {
                (self.max_width - max_line_w).max(0.0)
            }
            _ => 0.0,
        };

        ComputedLayout {
            metrics: DWRITE_TEXT_METRICS {
                left,
                top: 0.0,
                width: max_line_w,
                widthIncludingTrailingWhitespace: full_w,
                height: total_h,
                layoutWidth: self.max_width,
                layoutHeight: self.max_height,
                maxBidiReorderingDepth: 1,
                lineCount: lines.len() as u32,
            },
            lines,
            glyphs,
        }
    }
}

#[derive(Clone, Debug)]
pub struct IDWriteTextLayout(pub Arc<Mutex<TextLayoutData>>);

#[derive(Clone, Debug)]
pub struct IDWriteTextLayout1(pub Arc<Mutex<TextLayoutData>>);

impl Deref for IDWriteTextLayout1 {
    type Target = IDWriteTextLayout;
    fn deref(&self) -> &Self::Target {
        unsafe { &*(self as *const IDWriteTextLayout1 as *const IDWriteTextLayout) }
    }
}

impl Interface for IDWriteTextLayout {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}
impl Interface for IDWriteTextLayout1 {
    fn as_raw(&self) -> *mut c_void {
        Arc::as_ptr(&self.0) as *mut c_void
    }
}

impl CastFrom<IDWriteTextLayout1> for IDWriteTextLayout {
    fn cast_into(&self) -> Result<IDWriteTextLayout1> {
        Ok(IDWriteTextLayout1(self.0.clone()))
    }
}
impl CastFrom<IDWriteTextLayout> for IDWriteTextLayout1 {
    fn cast_into(&self) -> Result<IDWriteTextLayout> {
        Ok(IDWriteTextLayout(self.0.clone()))
    }
}

impl IDWriteTextLayout {
    pub unsafe fn SetFontWeight(
        &self,
        weight: DWRITE_FONT_WEIGHT,
        range: DWRITE_TEXT_RANGE,
    ) -> Result<()> {
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        d.weight_spans.push(WeightSpan {
            start: range.startPosition,
            len: range.length,
            weight,
        });
        d.cached_metrics = None;
        Ok(())
    }

    pub unsafe fn GetFontWeight(
        &self,
        pos: u32,
        weight: &mut DWRITE_FONT_WEIGHT,
        range: Option<*mut DWRITE_TEXT_RANGE>,
    ) -> Result<()> {
        let d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        *weight = d.weight_at(pos);
        if let Some(rp) = range {
            if !rp.is_null() {
                *rp = DWRITE_TEXT_RANGE { startPosition: pos, length: 1 };
            }
        }
        Ok(())
    }

    pub unsafe fn SetStrikethrough(&self, has: bool, range: DWRITE_TEXT_RANGE) -> Result<()> {
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        d.strike_spans.push(StrikeSpan {
            start: range.startPosition,
            len: range.length,
            enabled: has,
        });
        Ok(())
    }

    pub unsafe fn SetTypography(
        &self,
        typo: &IDWriteTypography,
        _range: DWRITE_TEXT_RANGE,
    ) -> Result<()> {
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        if let Ok(f) = typo.features.lock() {
            if f.iter()
                .any(|x| x.nameTag == DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES && x.parameter != 0)
            {
                d.tabular = true;
                d.cached_metrics = None;
            }
        }
        Ok(())
    }

    pub unsafe fn GetMetrics(&self, metrics: &mut DWRITE_TEXT_METRICS) -> Result<()> {
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        let comp = d.compute();
        *metrics = comp.metrics;
        Ok(())
    }

    pub unsafe fn GetLineMetrics(
        &self,
        lines: Option<&mut [DWRITE_LINE_METRICS]>,
        actual_count: &mut u32,
    ) -> Result<()> {
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        let comp = d.compute();
        *actual_count = comp.lines.len() as u32;
        if let Some(out) = lines {
            for (dst, src) in out.iter_mut().zip(comp.lines.iter()) {
                *dst = DWRITE_LINE_METRICS {
                    length: src.len_u16,
                    trailingWhitespaceLength: src.trailing_ws_u16,
                    newlineLength: src.newline_u16,
                    height: src.height,
                    baseline: src.baseline,
                    isTrimmed: BOOL(0),
                };
            }
        }
        Ok(())
    }

    pub unsafe fn HitTestPoint(
        &self,
        point_x: f32,
        point_y: f32,
        is_trailing_hit: &mut BOOL,
        is_inside: &mut BOOL,
        metrics: &mut DWRITE_HIT_TEST_METRICS,
    ) -> Result<()> {
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        let comp = d.compute();
        if comp.glyphs.is_empty() {
            *is_trailing_hit = BOOL(0);
            *is_inside = BOOL(0);
            *metrics = DWRITE_HIT_TEST_METRICS::default();
            return Ok(());
        }
        let line_h = comp.lines.first().map(|l| l.height).unwrap_or(14.0);
        let line_idx = ((point_y / line_h).floor() as isize)
            .clamp(0, comp.lines.len().saturating_sub(1) as isize) as usize;
        let line_glyphs: Vec<&GlyphPos> =
            comp.glyphs.iter().filter(|g| g.line_idx == line_idx).collect();
        if line_glyphs.is_empty() {
            *is_trailing_hit = BOOL(0);
            *is_inside = BOOL(0);
            let total_u16 = d.text.encode_utf16().count() as u32;
            *metrics = DWRITE_HIT_TEST_METRICS {
                textPosition: total_u16,
                length: 0,
                height: line_h,
                ..Default::default()
            };
            return Ok(());
        }
        let first = line_glyphs[0];
        let last = line_glyphs[line_glyphs.len() - 1];
        *is_inside = BOOL::from(
            point_x >= first.x && point_x <= last.x + last.width && point_y >= 0.0,
        );
        if point_x < first.x {
            *is_trailing_hit = BOOL(0);
            *metrics = DWRITE_HIT_TEST_METRICS {
                textPosition: first.u16_pos,
                length: first.u16_len,
                left: first.x,
                top: line_idx as f32 * line_h,
                width: first.width,
                height: line_h,
                isText: BOOL(1),
                ..Default::default()
            };
            return Ok(());
        }
        for g in &line_glyphs {
            if point_x <= g.x + g.width {
                *is_trailing_hit = BOOL::from(point_x >= g.x + g.width * 0.5);
                *metrics = DWRITE_HIT_TEST_METRICS {
                    textPosition: g.u16_pos,
                    length: g.u16_len,
                    left: g.x,
                    top: line_idx as f32 * line_h,
                    width: g.width,
                    height: line_h,
                    isText: BOOL(1),
                    ..Default::default()
                };
                return Ok(());
            }
        }
        *is_trailing_hit = BOOL(1);
        *metrics = DWRITE_HIT_TEST_METRICS {
            textPosition: last.u16_pos,
            length: last.u16_len,
            left: last.x,
            top: line_idx as f32 * line_h,
            width: last.width,
            height: line_h,
            isText: BOOL(1),
            ..Default::default()
        };
        Ok(())
    }

    pub unsafe fn HitTestTextPosition(
        &self,
        text_pos: u32,
        is_trailing_hit: bool,
        point_x: &mut f32,
        point_y: &mut f32,
        metrics: &mut DWRITE_HIT_TEST_METRICS,
    ) -> Result<()> {
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        let comp = d.compute();
        let line_h = comp.lines.first().map(|l| l.height).unwrap_or(14.0);
        if let Some(g) = comp
            .glyphs
            .iter()
            .find(|g| text_pos >= g.u16_pos && text_pos < g.u16_pos + g.u16_len)
        {
            *point_x = g.x + if is_trailing_hit { g.width } else { 0.0 };
            *point_y = g.line_idx as f32 * line_h;
            *metrics = DWRITE_HIT_TEST_METRICS {
                textPosition: g.u16_pos,
                length: g.u16_len,
                left: g.x,
                top: *point_y,
                width: g.width,
                height: line_h,
                isText: BOOL(1),
                ..Default::default()
            };
        } else {
            let (end_x, lidx) = comp
                .glyphs
                .last()
                .map(|g| (g.x + g.width, g.line_idx))
                .unwrap_or((0.0, 0));
            *point_x = end_x;
            *point_y = lidx as f32 * line_h;
            *metrics = DWRITE_HIT_TEST_METRICS {
                textPosition: text_pos,
                length: 0,
                left: end_x,
                top: *point_y,
                width: 0.0,
                height: line_h,
                ..Default::default()
            };
        }
        Ok(())
    }
}

impl IDWriteTextLayout1 {
    pub unsafe fn SetCharacterSpacing(
        &self,
        leading: f32,
        trailing: f32,
        _min_advance: f32,
        _range: DWRITE_TEXT_RANGE,
    ) -> Result<()> {
        let mut d = self.0.lock().map_err(|_| Error::from_hresult(E_FAIL))?;
        d.leading_spacing = leading;
        d.trailing_spacing = trailing;
        d.cached_metrics = None;
        Ok(())
    }
}
