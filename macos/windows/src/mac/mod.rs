#![cfg(target_os = "macos")]
#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::too_many_arguments)]

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use windows_numerics::Matrix3x2;

use crate::d2d::{
    BrushData, GeometryData, PathCmd, PixelSurface, StrokeStyleData, D2D1_COLOR_F,
    D2D1_DRAW_TEXT_OPTIONS, D2D1_FILL_MODE_ALTERNATE, D2D_RECT_F,
};
use crate::dwrite::{
    ComputedLayout, GlyphPos, LayoutLineInfo, TextFormatData, TextLayoutData,
    DWRITE_HIT_TEST_METRICS, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use crate::foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use crate::win32_ui::Win32::Graphics::Gdi::{HMONITOR, MONITORINFO, RgnShape};
use crate::win32_ui::Win32::UI::Shell::{NOTIFYICONDATAW, NIM_ADD, NIM_DELETE, NIM_MODIFY};
use crate::win32_ui::Win32::UI::WindowsAndMessaging::{
    MSG, WNDPROC, MF_CHECKED, MF_SEPARATOR, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, WM_ACTIVATE, WM_CHAR, WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSELEAVE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONUP, WM_TIMER,
};

// ---- External macOS Framework FFI ----

pub type Id = *mut c_void;
pub type Sel = *mut c_void;
pub type Class = *mut c_void;
pub type CGFloat = f64;
pub type CFIndex = isize;
pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFDataRef = *const c_void;
pub type CFArrayRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFMutableDictionaryRef = *mut c_void;
pub type CFAttributedStringRef = *const c_void;
pub type CGContextRef = *mut c_void;
pub type CGColorSpaceRef = *const c_void;
pub type CGImageRef = *const c_void;
pub type CGGradientRef = *const c_void;
pub type CGPathRef = *mut c_void;
pub type CTFontRef = *const c_void;
pub type CTLineRef = *const c_void;
pub type CTTypesetterRef = *const c_void;
pub type CGImageSourceRef = *const c_void;
pub type CFRunLoopTimerRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGSize {
    pub width: CGFloat,
    pub height: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSEdgeInsets {
    pub top: CGFloat,
    pub left: CGFloat,
    pub bottom: CGFloat,
    pub right: CGFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CFRange {
    pub location: CFIndex,
    pub length: CFIndex,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGAffineTransform {
    pub a: CGFloat,
    pub b: CGFloat,
    pub c: CGFloat,
    pub d: CGFloat,
    pub tx: CGFloat,
    pub ty: CGFloat,
}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_fpret();
    fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra_bytes: usize) -> Class;
    fn objc_registerClassPair(cls: Class);
    fn class_addMethod(cls: Class, name: Sel, imp: *const c_void, types: *const c_char) -> bool;
    fn class_respondsToSelector(cls: Class, sel: Sel) -> bool;
    fn object_getClass(obj: Id) -> Class;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRelease(cf: CFTypeRef);
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFStringCreateWithCString(
        alloc: CFTypeRef,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFDictionaryCreateMutable(
        alloc: CFTypeRef,
        capacity: CFIndex,
        kcb: *const c_void,
        vcb: *const c_void,
    ) -> CFMutableDictionaryRef;
    fn CFDictionarySetValue(dict: CFMutableDictionaryRef, key: *const c_void, val: *const c_void);
    fn CFAttributedStringCreate(
        alloc: CFTypeRef,
        str: CFStringRef,
        attrs: CFDictionaryRef,
    ) -> CFAttributedStringRef;
    fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, length: CFIndex) -> CFDataRef;
    fn CFArrayGetCount(arr: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: CFIndex) -> *const c_void;
    fn CFStringGetCString(
        s: CFStringRef,
        buf: *mut c_char,
        buf_size: CFIndex,
        encoding: u32,
    ) -> bool;
    fn CFNumberCreate(alloc: CFTypeRef, the_type: i32, val: *const c_void) -> CFTypeRef;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopWakeUp(rl: *mut c_void);
    fn CFRunLoopTimerCreate(
        alloc: CFTypeRef,
        fire_date: f64,
        interval: f64,
        flags: usize,
        order: CFIndex,
        callout: extern "C" fn(CFRunLoopTimerRef, *mut c_void),
        ctx: *const c_void,
    ) -> CFRunLoopTimerRef;
    fn CFRunLoopAddTimer(rl: *mut c_void, timer: CFRunLoopTimerRef, mode: CFStringRef);
    fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);
    fn CFAbsoluteTimeGetCurrent() -> f64;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef;
    fn CGColorSpaceRelease(cs: CGColorSpaceRef);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        bitmap_info: u32,
    ) -> CGContextRef;
    fn CGBitmapContextCreateImage(ctx: CGContextRef) -> CGImageRef;
    fn CGContextRelease(ctx: CGContextRef);
    fn CGContextSaveGState(ctx: CGContextRef);
    fn CGContextRestoreGState(ctx: CGContextRef);
    fn CGContextSetRGBFillColor(ctx: CGContextRef, r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat);
    fn CGContextSetRGBStrokeColor(ctx: CGContextRef, r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat);
    fn CGContextSetLineWidth(ctx: CGContextRef, w: CGFloat);
    fn CGContextSetLineCap(ctx: CGContextRef, cap: i32);
    fn CGContextSetLineJoin(ctx: CGContextRef, join: i32);
    fn CGContextSetMiterLimit(ctx: CGContextRef, limit: CGFloat);
    fn CGContextBeginPath(ctx: CGContextRef);
    fn CGContextMoveToPoint(ctx: CGContextRef, x: CGFloat, y: CGFloat);
    fn CGContextAddLineToPoint(ctx: CGContextRef, x: CGFloat, y: CGFloat);
    fn CGContextAddQuadCurveToPoint(ctx: CGContextRef, cpx: CGFloat, cpy: CGFloat, x: CGFloat, y: CGFloat);
    fn CGContextAddCurveToPoint(
        ctx: CGContextRef,
        cp1x: CGFloat,
        cp1y: CGFloat,
        cp2x: CGFloat,
        cp2y: CGFloat,
        x: CGFloat,
        y: CGFloat,
    );
    fn CGContextClosePath(ctx: CGContextRef);
    fn CGContextFillPath(ctx: CGContextRef);
    fn CGContextEOFillPath(ctx: CGContextRef);
    fn CGContextStrokePath(ctx: CGContextRef);
    fn CGContextClip(ctx: CGContextRef);
    fn CGContextEOClip(ctx: CGContextRef);
    fn CGContextClipToRect(ctx: CGContextRef, rect: CGRect);
    fn CGContextConcatCTM(ctx: CGContextRef, transform: CGAffineTransform);
    fn CGContextTranslateCTM(ctx: CGContextRef, tx: CGFloat, ty: CGFloat);
    fn CGContextScaleCTM(ctx: CGContextRef, sx: CGFloat, sy: CGFloat);
    fn CGContextSetTextMatrix(ctx: CGContextRef, t: CGAffineTransform);
    fn CGContextSetTextPosition(ctx: CGContextRef, x: CGFloat, y: CGFloat);
    fn CGContextSetAlpha(ctx: CGContextRef, alpha: CGFloat);
    fn CGContextDrawImage(ctx: CGContextRef, rect: CGRect, img: CGImageRef);
    fn CGContextDrawLinearGradient(
        ctx: CGContextRef,
        grad: CGGradientRef,
        start: CGPoint,
        end: CGPoint,
        options: u32,
    );
    fn CGContextDrawRadialGradient(
        ctx: CGContextRef,
        grad: CGGradientRef,
        start_center: CGPoint,
        start_radius: CGFloat,
        end_center: CGPoint,
        end_radius: CGFloat,
        options: u32,
    );
    fn CGContextBeginTransparencyLayer(ctx: CGContextRef, aux: CFDictionaryRef);
    fn CGContextEndTransparencyLayer(ctx: CGContextRef);
    fn CGGradientCreateWithColorComponents(
        space: CGColorSpaceRef,
        components: *const CGFloat,
        locations: *const CGFloat,
        count: usize,
    ) -> CGGradientRef;
    fn CGGradientRelease(grad: CGGradientRef);
    fn CGImageRelease(img: CGImageRef);
    fn CGImageGetWidth(img: CGImageRef) -> usize;
    fn CGImageGetHeight(img: CGImageRef) -> usize;
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

#[link(name = "CoreText", kind = "framework")]
extern "C" {
    static kCTFontAttributeName: CFStringRef;
    static kCTForegroundColorAttributeName: CFStringRef;
    static kCTKernAttributeName: CFStringRef;
    fn CTFontCreateUIFontForLanguage(
        ui_type: u32,
        size: CGFloat,
        language: CFStringRef,
    ) -> CTFontRef;
    fn CTFontCreateWithName(
        name: CFStringRef,
        size: CGFloat,
        matrix: *const CGAffineTransform,
    ) -> CTFontRef;
    fn CTFontCreateCopyWithSymbolicTraits(
        font: CTFontRef,
        size: CGFloat,
        matrix: *const CGAffineTransform,
        sym_trait_value: u32,
        sym_trait_mask: u32,
    ) -> CTFontRef;
    fn CTFontManagerCopyAvailableFontFamilyNames() -> CFArrayRef;
    fn CTLineCreateWithAttributedString(attr_str: CFAttributedStringRef) -> CTLineRef;
    fn CTLineDraw(line: CTLineRef, ctx: CGContextRef);
    fn CTLineGetTypographicBounds(
        line: CTLineRef,
        ascent: *mut CGFloat,
        descent: *mut CGFloat,
        leading: *mut CGFloat,
    ) -> CGFloat;
    fn CTLineGetOffsetForStringIndex(
        line: CTLineRef,
        char_index: CFIndex,
        secondary_offset: *mut CGFloat,
    ) -> CGFloat;
    fn CTLineGetStringIndexForPosition(line: CTLineRef, position: CGPoint) -> CFIndex;
    fn CTTypesetterCreateWithAttributedString(attr_str: CFAttributedStringRef) -> CTTypesetterRef;
    fn CTTypesetterSuggestLineBreak(
        ts: CTTypesetterRef,
        start_index: CFIndex,
        width: CGFloat,
    ) -> CFIndex;
    fn CTTypesetterCreateLine(ts: CTTypesetterRef, string_range: CFRange) -> CTLineRef;
    fn CGColorCreate(space: CGColorSpaceRef, components: *const CGFloat) -> *const c_void;
}

#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    fn CGImageSourceCreateWithData(data: CFDataRef, options: CFDictionaryRef) -> CGImageSourceRef;
    fn CGImageSourceCreateImageAtIndex(
        isrc: CGImageSourceRef,
        index: usize,
        options: CFDictionaryRef,
    ) -> CGImageRef;
}

#[link(name = "Metal", kind = "framework")]
extern "C" {
    fn MTLCreateSystemDefaultDevice() -> Id;
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: *const c_void,
        num_types: u32,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hk: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: u32,
        actual_size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

// ---- ObjC ABI Helpers ----

const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const KCF_NUMBER_FLOAT64_TYPE: i32 = 6;
const KCG_BITMAP_BGRA_PREMUL: u32 = (2 << 12) | 2; // kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst

#[inline]
unsafe fn cls(name: &[u8]) -> Class {
    objc_getClass(name.as_ptr() as *const c_char)
}

#[inline]
unsafe fn sel(name: &[u8]) -> Sel {
    sel_registerName(name.as_ptr() as *const c_char)
}

#[inline]
unsafe fn msg0<R>(obj: Id, s: Sel) -> R {
    let f: unsafe extern "C" fn(Id, Sel) -> R = std::mem::transmute(objc_msgSend as *const c_void);
    f(obj, s)
}

#[inline]
unsafe fn msg0_f64(obj: Id, s: Sel) -> f64 {
    #[cfg(target_arch = "x86_64")]
    {
        let f: unsafe extern "C" fn(Id, Sel) -> f64 =
            std::mem::transmute(objc_msgSend_fpret as *const c_void);
        f(obj, s)
    }
    #[cfg(not(target_arch = "x86_64"))]
    msg0::<f64>(obj, s)
}

#[inline]
unsafe fn msg0_rect(obj: Id, s: Sel) -> CGRect {
    #[cfg(target_arch = "x86_64")]
    {
        let f: unsafe extern "C" fn(Id, Sel) -> CGRect =
            std::mem::transmute(objc_msgSend_stret as *const c_void);
        f(obj, s)
    }
    #[cfg(not(target_arch = "x86_64"))]
    msg0::<CGRect>(obj, s)
}

#[inline]
unsafe fn msg0_insets(obj: Id, s: Sel) -> NSEdgeInsets {
    #[cfg(target_arch = "x86_64")]
    {
        let f: unsafe extern "C" fn(Id, Sel) -> NSEdgeInsets =
            std::mem::transmute(objc_msgSend_stret as *const c_void);
        f(obj, s)
    }
    #[cfg(not(target_arch = "x86_64"))]
    msg0::<NSEdgeInsets>(obj, s)
}

#[inline]
unsafe fn msg1<A, R>(obj: Id, s: Sel, a: A) -> R {
    let f: unsafe extern "C" fn(Id, Sel, A) -> R = std::mem::transmute(objc_msgSend as *const c_void);
    f(obj, s, a)
}

#[inline]
unsafe fn msg2<A, B, R>(obj: Id, s: Sel, a: A, b: B) -> R {
    let f: unsafe extern "C" fn(Id, Sel, A, B) -> R =
        std::mem::transmute(objc_msgSend as *const c_void);
    f(obj, s, a, b)
}

#[inline]
unsafe fn msg3<A, B, C, R>(obj: Id, s: Sel, a: A, b: B, c: C) -> R {
    let f: unsafe extern "C" fn(Id, Sel, A, B, C) -> R =
        std::mem::transmute(objc_msgSend as *const c_void);
    f(obj, s, a, b, c)
}

#[inline]
unsafe fn msg4<A, B, C, D, R>(obj: Id, s: Sel, a: A, b: B, c: C, d: D) -> R {
    let f: unsafe extern "C" fn(Id, Sel, A, B, C, D) -> R =
        std::mem::transmute(objc_msgSend as *const c_void);
    f(obj, s, a, b, c, d)
}

unsafe fn nsstring(text: &str) -> Id {
    let c = CString::new(text).unwrap_or_default();
    let cf = CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), KCF_STRING_ENCODING_UTF8);
    cf as Id
}

unsafe fn nsstring_to_rust(ns: Id) -> String {
    if ns.is_null() {
        return String::new();
    }
    let ptr: *const c_char = msg0(ns, sel(b"UTF8String\0"));
    if ptr.is_null() {
        return String::new();
    }
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

// ---- AppKit Bootstrap & Screens / Hardware Notch ----

static APP_INIT: OnceLock<()> = OnceLock::new();
static QUIT_FLAG: AtomicBool = AtomicBool::new(false);

unsafe fn ensure_nsapp() {
    APP_INIT.get_or_init(|| {
        let app_cls = cls(b"NSApplication\0");
        let app: Id = msg0(app_cls as Id, sel(b"sharedApplication\0"));
        // NSApplicationActivationPolicyAccessory = 1 (menu bar / overlay app without Dock icon)
        let _: bool = msg1(app, sel(b"setActivationPolicy:\0"), 1isize);
        let _: () = msg0(app, sel(b"finishLaunching\0"));
    });
}

#[inline]
unsafe fn primary_screen_height() -> f64 {
    let screens: Id = msg0(cls(b"NSScreen\0") as Id, sel(b"screens\0"));
    let count: usize = msg0(screens, sel(b"count\0"));
    if count == 0 {
        return 900.0;
    }
    let first: Id = msg1(screens, sel(b"objectAtIndex:\0"), 0usize);
    let fr = msg0_rect(first, sel(b"frame\0"));
    fr.size.height
}

unsafe fn find_screen(mon: HMONITOR) -> Id {
    ensure_nsapp();
    let screens: Id = msg0(cls(b"NSScreen\0") as Id, sel(b"screens\0"));
    let count: usize = msg0(screens, sel(b"count\0"));
    if count == 0 {
        return std::ptr::null_mut();
    }
    for i in 0..count {
        let sc: Id = msg1(screens, sel(b"objectAtIndex:\0"), i);
        let num = screen_display_id(sc);
        if num == mon.0 {
            return sc;
        }
    }
    msg1(screens, sel(b"objectAtIndex:\0"), 0usize)
}

unsafe fn screen_display_id(sc: Id) -> isize {
    if sc.is_null() {
        return 1;
    }
    let desc: Id = msg0(sc, sel(b"deviceDescription\0"));
    let key = nsstring("NSScreenNumber");
    let num: Id = msg1(desc, sel(b"objectForKey:\0"), key);
    CFRelease(key as CFTypeRef);
    if num.is_null() {
        1
    } else {
        let v: usize = msg0(num, sel(b"unsignedIntegerValue\0"));
        v as isize
    }
}

unsafe fn screen_scale(sc: Id) -> f64 {
    if sc.is_null() {
        return 2.0;
    }
    let s = msg0_f64(sc, sel(b"backingScaleFactor\0"));
    if s > 0.5 { s } else { 2.0 }
}

pub unsafe fn enum_monitors() -> Vec<(HMONITOR, RECT)> {
    ensure_nsapp();
    let h0 = primary_screen_height();
    let screens: Id = msg0(cls(b"NSScreen\0") as Id, sel(b"screens\0"));
    let count: usize = msg0(screens, sel(b"count\0"));
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let sc: Id = msg1(screens, sel(b"objectAtIndex:\0"), i);
        let id = screen_display_id(sc);
        let s = screen_scale(sc);
        let fr = msg0_rect(sc, sel(b"frame\0"));
        let top = h0 - (fr.origin.y + fr.size.height);
        out.push((
            HMONITOR(id),
            RECT {
                left: (fr.origin.x * s).round() as i32,
                top: (top * s).round() as i32,
                right: ((fr.origin.x + fr.size.width) * s).round() as i32,
                bottom: ((top + fr.size.height) * s).round() as i32,
            },
        ));
    }
    out
}

pub unsafe fn monitor_from_point(pt: POINT) -> HMONITOR {
    ensure_nsapp();
    for (mon, rc) in enum_monitors() {
        if pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom {
            return mon;
        }
    }
    let screens: Id = msg0(cls(b"NSScreen\0") as Id, sel(b"screens\0"));
    let count: usize = msg0(screens, sel(b"count\0"));
    if count > 0 {
        screen_display_id(msg1(screens, sel(b"objectAtIndex:\0"), 0usize))
            .pipe(HMONITOR)
    } else {
        HMONITOR(1)
    }
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

pub unsafe fn monitor_from_window(hwnd: HWND) -> HMONITOR {
    if let Some(st) = windows().lock().ok().and_then(|m| m.get(&(hwnd.0 as usize)).cloned()) {
        let sc: Id = msg0(st.panel, sel(b"screen\0"));
        if !sc.is_null() {
            return HMONITOR(screen_display_id(sc));
        }
    }
    monitor_from_point(POINT::default())
}

pub unsafe fn get_monitor_info(mon: HMONITOR, mi: &mut MONITORINFO) -> bool {
    let sc = find_screen(mon);
    if sc.is_null() {
        mi.rcMonitor = RECT { left: 0, top: 0, right: 1512, bottom: 982 };
        mi.rcWork = RECT { left: 0, top: 25, right: 1512, bottom: 982 };
        mi.dwFlags = 1;
        return true;
    }
    let h0 = primary_screen_height();
    let s = screen_scale(sc);
    let fr = msg0_rect(sc, sel(b"frame\0"));
    let vf = msg0_rect(sc, sel(b"visibleFrame\0"));
    let top_m = h0 - (fr.origin.y + fr.size.height);
    let top_w = h0 - (vf.origin.y + vf.size.height);
    mi.rcMonitor = RECT {
        left: (fr.origin.x * s).round() as i32,
        top: (top_m * s).round() as i32,
        right: ((fr.origin.x + fr.size.width) * s).round() as i32,
        bottom: ((top_m + fr.size.height) * s).round() as i32,
    };
    mi.rcWork = RECT {
        left: (vf.origin.x * s).round() as i32,
        top: (top_w * s).round() as i32,
        right: ((vf.origin.x + vf.size.width) * s).round() as i32,
        bottom: ((top_w + vf.size.height) * s).round() as i32,
    };
    mi.dwFlags = 1;
    true
}

pub unsafe fn get_monitor_dpi(mon: HMONITOR) -> u32 {
    let sc = find_screen(mon);
    (screen_scale(sc) * 96.0).round() as u32
}

/// Queries physical MacBook notch `(width_px, height_px)` on the display (matching `MONITORINFO` physical pixels).
pub unsafe fn get_monitor_notch(mon: HMONITOR) -> (f32, f32) {
    let sc = find_screen(mon);
    if sc.is_null() {
        return (0.0, 0.0);
    }
    let s = screen_scale(sc) as f32;
    let sc_cls = object_getClass(sc);
    if !class_respondsToSelector(sc_cls, sel(b"safeAreaInsets\0")) {
        return (0.0, 0.0);
    }
    let insets = msg0_insets(sc, sel(b"safeAreaInsets\0"));
    if insets.top <= 0.5 {
        return (0.0, 0.0);
    }
    let notch_h_pt = insets.top as f32;
    let mut notch_w_pt = 188.0f32;
    if class_respondsToSelector(sc_cls, sel(b"auxiliaryTopLeftArea\0"))
        && class_respondsToSelector(sc_cls, sel(b"auxiliaryTopRightArea\0"))
    {
        let left_area = msg0_rect(sc, sel(b"auxiliaryTopLeftArea\0"));
        let right_area = msg0_rect(sc, sel(b"auxiliaryTopRightArea\0"));
        if left_area.size.width > 1.0 && right_area.size.width > 1.0 {
            let gap = right_area.origin.x - (left_area.origin.x + left_area.size.width);
            if gap > 40.0 && gap < 400.0 {
                notch_w_pt = gap as f32;
            }
        }
    }
    (notch_w_pt * s, notch_h_pt * s)
}

// ---- Window, Layer & Hit Region Management ----

#[derive(Clone)]
struct WinState {
    panel: Id,
    view: Id,
    blur_view: Id,
    proc_fn: WNDPROC,
    rgn: Option<RgnShape>,
    scale: f64,
    width_px: i32,
    height_px: i32,
}
unsafe impl Send for WinState {}
unsafe impl Sync for WinState {}

#[derive(Clone, Copy)]
struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

static WINDOWS: OnceLock<Mutex<HashMap<usize, WinState>>> = OnceLock::new();
static MSG_QUEUE: OnceLock<Mutex<VecDeque<MSG>>> = OnceLock::new();
static TIMERS: OnceLock<Mutex<HashMap<(usize, usize), SendPtr>>> = OnceLock::new();
static VIEW_CLS: OnceLock<SendPtr> = OnceLock::new();
static TRAY_STATE: OnceLock<Mutex<Option<SendPtr>>> = OnceLock::new();

fn windows() -> &'static Mutex<HashMap<usize, WinState>> {
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn msg_queue() -> &'static Mutex<VecDeque<MSG>> {
    MSG_QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}
fn timers() -> &'static Mutex<HashMap<(usize, usize), SendPtr>> {
    TIMERS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn tray_state() -> &'static Mutex<Option<SendPtr>> {
    TRAY_STATE.get_or_init(|| Mutex::new(None))
}

unsafe fn push_msg(hwnd: HWND, message: u32, wparam: usize, lparam: isize) {
    if let Ok(mut q) = msg_queue().lock() {
        q.push_back(MSG {
            hwnd,
            message,
            wParam: WPARAM(wparam),
            lParam: LPARAM(lparam),
            time: 0,
            pt: get_cursor_pos(),
        });
    }
    CFRunLoopWakeUp(CFRunLoopGetMain());
}

fn find_hwnd_for_view(view: Id) -> Option<(HWND, WinState)> {
    let map = windows().lock().ok()?;
    for (&k, v) in map.iter() {
        if v.view == view || v.panel == view {
            return Some((HWND(k as *mut c_void), v.clone()));
        }
    }
    None
}

unsafe fn view_event_xy_px(view: Id, event: Id, st: &WinState) -> (i32, i32) {
    let loc: CGPoint = {
        let f: unsafe extern "C" fn(Id, Sel) -> CGPoint =
            std::mem::transmute(objc_msgSend as *const c_void);
        f(event, sel(b"locationInWindow\0"))
    };
    let bounds = msg0_rect(view, sel(b"bounds\0"));
    let x = (loc.x * st.scale).round() as i32;
    let y = ((bounds.size.height - loc.y) * st.scale).round() as i32;
    (x, y)
}

#[inline]
fn pack_lparam(x: i32, y: i32) -> isize {
    (((y as i16 as u16) as isize) << 16) | ((x as i16 as u16) as isize)
}

extern "C" fn view_accepts_first_mouse(_this: Id, _cmd: Sel, _ev: Id) -> bool {
    true
}

extern "C" fn view_mouse_down(this: Id, _cmd: Sel, ev: Id) {
    unsafe {
        if let Some((hwnd, st)) = find_hwnd_for_view(this) {
            let (x, y) = view_event_xy_px(this, ev, &st);
            push_msg(hwnd, WM_LBUTTONDOWN, 1, pack_lparam(x, y));
        }
    }
}

extern "C" fn view_mouse_up(this: Id, _cmd: Sel, ev: Id) {
    unsafe {
        if let Some((hwnd, st)) = find_hwnd_for_view(this) {
            let (x, y) = view_event_xy_px(this, ev, &st);
            push_msg(hwnd, WM_LBUTTONUP, 0, pack_lparam(x, y));
        }
    }
}

extern "C" fn view_right_mouse_up(this: Id, _cmd: Sel, ev: Id) {
    unsafe {
        if let Some((hwnd, st)) = find_hwnd_for_view(this) {
            let (x, y) = view_event_xy_px(this, ev, &st);
            push_msg(hwnd, WM_RBUTTONUP, 0, pack_lparam(x, y));
        }
    }
}

extern "C" fn view_mouse_moved(this: Id, _cmd: Sel, ev: Id) {
    unsafe {
        if let Some((hwnd, st)) = find_hwnd_for_view(this) {
            let (x, y) = view_event_xy_px(this, ev, &st);
            push_msg(hwnd, WM_MOUSEMOVE, 0, pack_lparam(x, y));
        }
    }
}

extern "C" fn view_mouse_exited(this: Id, _cmd: Sel, _ev: Id) {
    unsafe {
        if let Some((hwnd, _)) = find_hwnd_for_view(this) {
            push_msg(hwnd, WM_MOUSELEAVE, 0, 0);
        }
    }
}

extern "C" fn view_scroll_wheel(this: Id, _cmd: Sel, ev: Id) {
    unsafe {
        if let Some((hwnd, st)) = find_hwnd_for_view(this) {
            let dy = msg0_f64(ev, sel(b"scrollingDeltaY\0"));
            let has_precise: bool = msg0(ev, sel(b"hasPreciseScrollingDeltas\0"));
            let delta = if has_precise { (dy * 8.0).round() as i16 } else { (dy * 120.0).round() as i16 };
            let (x, y) = view_event_xy_px(this, ev, &st);
            push_msg(hwnd, WM_MOUSEWHEEL, ((delta as i32) << 16) as usize, pack_lparam(x, y));
        }
    }
}

fn mac_keycode_to_vk(kc: u16) -> u32 {
    match kc {
        0x24 | 0x4C => 0x0D, // Return
        0x30 => 0x09,        // Tab
        0x31 => 0x20,        // Space
        0x33 => 0x08,        // Delete/Backspace
        0x35 => 0x1B,        // Escape
        0x75 => 0x2E,        // Forward Delete
        0x73 => 0x24,        // Home
        0x77 => 0x23,        // End
        0x7B => 0x25,        // Left
        0x7C => 0x27,        // Right
        0x7D => 0x28,        // Down
        0x7E => 0x26,        // Up
        _ => 0,
    }
}

extern "C" fn view_key_down(this: Id, _cmd: Sel, ev: Id) {
    unsafe {
        if let Some((hwnd, _)) = find_hwnd_for_view(this) {
            let kc: u16 = msg0(ev, sel(b"keyCode\0"));
            let vk = mac_keycode_to_vk(kc);
            if vk != 0 {
                push_msg(hwnd, WM_KEYDOWN, vk as usize, 0);
            }
            let chars_ns: Id = msg0(ev, sel(b"characters\0"));
            let s = nsstring_to_rust(chars_ns);
            for u in s.encode_utf16() {
                if u >= 0x20 && u != 0x7F {
                    push_msg(hwnd, WM_CHAR, u as usize, 0);
                }
            }
        }
    }
}

extern "C" fn view_hit_test(this: Id, _cmd: Sel, pt: CGPoint) -> Id {
    unsafe {
        if let Some((_, st)) = find_hwnd_for_view(this) {
            if let Some(shape) = &st.rgn {
                let bounds = msg0_rect(this, sel(b"bounds\0"));
                let x = (pt.x * st.scale).round() as i32;
                let y = ((bounds.size.height - pt.y) * st.scale).round() as i32;
                let hit = match shape {
                    RgnShape::Rect(r) => x >= r.left && x < r.right && y >= r.top && y < r.bottom,
                    RgnShape::Polygon(pts) => point_in_poly(x, y, pts),
                };
                if !hit {
                    return std::ptr::null_mut();
                }
            }
        }
        this
    }
}

fn point_in_poly(px: i32, py: i32, pts: &[POINT]) -> bool {
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (pts[i].x, pts[i].y);
        let (xj, yj) = (pts[j].x, pts[j].y);
        if ((yi > py) != (yj > py))
            && ((px as f64) < (xj - xi) as f64 * (py - yi) as f64 / (yj - yi) as f64 + xi as f64)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

unsafe fn get_view_class() -> Class {
    VIEW_CLS
        .get_or_init(|| {
            let super_cls = cls(b"NSView\0");
            let name = b"KiteIslandView\0";
            let existing = cls(name);
            if !existing.is_null() {
                return SendPtr(existing);
            }
            let c = objc_allocateClassPair(super_cls, name.as_ptr() as *const c_char, 0);
            class_addMethod(c, sel(b"acceptsFirstMouse:\0"), view_accepts_first_mouse as *const c_void, b"B@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"mouseDown:\0"), view_mouse_down as *const c_void, b"v@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"mouseUp:\0"), view_mouse_up as *const c_void, b"v@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"rightMouseUp:\0"), view_right_mouse_up as *const c_void, b"v@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"mouseMoved:\0"), view_mouse_moved as *const c_void, b"v@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"mouseDragged:\0"), view_mouse_moved as *const c_void, b"v@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"mouseExited:\0"), view_mouse_exited as *const c_void, b"v@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"scrollWheel:\0"), view_scroll_wheel as *const c_void, b"v@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"keyDown:\0"), view_key_down as *const c_void, b"v@:@\0".as_ptr() as *const c_char);
            class_addMethod(c, sel(b"hitTest:\0"), view_hit_test as *const c_void, b"@@:{CGPoint=dd}\0".as_ptr() as *const c_char);
            objc_registerClassPair(c);
            SendPtr(c)
        })
        .0
}

pub unsafe fn create_window(
    cname: &str,
    _title: &str,
    ex_style: u32,
    _style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    proc_fn: WNDPROC,
) -> HWND {
    ensure_nsapp();
    let h0 = primary_screen_height();
    let screens: Id = msg0(cls(b"NSScreen\0") as Id, sel(b"screens\0"));
    let first_sc: Id = if msg0::<usize>(screens, sel(b"count\0")) > 0 {
        msg1(screens, sel(b"objectAtIndex:\0"), 0usize)
    } else {
        std::ptr::null_mut()
    };
    let s = screen_scale(first_sc);
    let rw = (w.max(1) as f64) / s;
    let rh = (h.max(1) as f64) / s;
    let rx = (x as f64) / s;
    let ry = h0 - ((y as f64) / s + rh);
    let rect = CGRect { origin: CGPoint { x: rx, y: ry }, size: CGSize { width: rw, height: rh } };

    // NSWindowStyleMaskBorderless (0) | NSWindowStyleMaskNonactivatingPanel (1 << 7)
    let mask = 1usize << 7;
    let panel_alloc: Id = msg0(cls(b"NSPanel\0") as Id, sel(b"alloc\0"));
    let f_init: unsafe extern "C" fn(Id, Sel, CGRect, usize, usize, bool) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let panel = f_init(
        panel_alloc,
        sel(b"initWithContentRect:styleMask:backing:defer:\0"),
        rect,
        mask,
        2, // NSBackingStoreBuffered
        false,
    );
    let clear_col: Id = msg0(cls(b"NSColor\0") as Id, sel(b"clearColor\0"));
    let _: () = msg1(panel, sel(b"setBackgroundColor:\0"), clear_col);
    let _: () = msg1(panel, sel(b"setOpaque:\0"), false);
    let _: () = msg1(panel, sel(b"setHasShadow:\0"), false);
    // Above macOS menu bar & notch cutout (NSMainMenuWindowLevel = 24 -> level = 26)
    let _: () = msg1(panel, sel(b"setLevel:\0"), 26isize);
    let _: () = msg1(panel, sel(b"setHidesOnDeactivate:\0"), false);
    let _: () = msg1(panel, sel(b"setAcceptsMouseMovedEvents:\0"), true);
    let ignores_mouse = (ex_style & 0x20) != 0; // WS_EX_TRANSPARENT
    let _: () = msg1(panel, sel(b"setIgnoresMouseEvents:\0"), ignores_mouse);
    // CanJoinAllSpaces (1) | Stationary (16) | IgnoresCycle (64) | FullScreenAuxiliary (256)
    let _: () = msg1(panel, sel(b"setCollectionBehavior:\0"), 1usize | 16 | 64 | 256);

    let view_cls = get_view_class();
    let v_alloc: Id = msg0(view_cls as Id, sel(b"alloc\0"));
    let v_frame = CGRect { origin: CGPoint::default(), size: rect.size };
    let view: Id = msg1(v_alloc, sel(b"initWithFrame:\0"), v_frame);
    let _: () = msg1(view, sel(b"setWantsLayer:\0"), true);

    if cname == "IslandGlass" {
        let ve_alloc: Id = msg0(cls(b"NSVisualEffectView\0") as Id, sel(b"alloc\0"));
        let ve: Id = msg1(ve_alloc, sel(b"initWithFrame:\0"), v_frame);
        let _: () = msg1(ve, sel(b"setMaterial:\0"), 12isize); // NSVisualEffectMaterialHUDWindow
        let _: () = msg1(ve, sel(b"setBlendingMode:\0"), 0isize); // BehindWindow
        let _: () = msg1(ve, sel(b"setState:\0"), 1isize); // Active
        let _: () = msg1(ve, sel(b"setAutoresizingMask:\0"), 18usize);
        let _: () = msg1(panel, sel(b"setContentView:\0"), ve);
        let hwnd = HWND(panel);
        if let Ok(mut m) = windows().lock() {
            m.insert(panel as usize, WinState { panel, view: ve, blur_view: ve, proc_fn, rgn: None, scale: s, width_px: w, height_px: h });
        }
        return hwnd;
    }

    // Add NSTrackingArea so hover & MouseLeft events arrive even when inactive
    let ta_alloc: Id = msg0(cls(b"NSTrackingArea\0") as Id, sel(b"alloc\0"));
    let f_ta: unsafe extern "C" fn(Id, Sel, CGRect, usize, Id, Id) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    // MouseEnteredAndExited (0x01) | MouseMoved (0x02) | ActiveAlways (0x80) | InVisibleRect (0x200)
    let ta = f_ta(
        ta_alloc,
        sel(b"initWithRect:options:owner:userInfo:\0"),
        v_frame,
        0x01 | 0x02 | 0x80 | 0x200,
        view,
        std::ptr::null_mut(),
    );
    let _: () = msg1(view, sel(b"addTrackingArea:\0"), ta);
    let _: () = msg1(panel, sel(b"setContentView:\0"), view);

    let hwnd = HWND(panel);
    if let Ok(mut m) = windows().lock() {
        m.insert(panel as usize, WinState { panel, view, blur_view: std::ptr::null_mut(), proc_fn, rgn: None, scale: s, width_px: w, height_px: h });
    }
    hwnd
}

pub unsafe fn destroy_window(hwnd: HWND) {
    let removed = windows().lock().ok().and_then(|mut m| m.remove(&(hwnd.0 as usize)));
    if let Some(st) = removed {
        let _: () = msg0(st.panel, sel(b"close\0"));
    }
}

pub unsafe fn show_window(hwnd: HWND, cmd: i32) {
    if let Some(st) = windows().lock().ok().and_then(|m| m.get(&(hwnd.0 as usize)).cloned()) {
        if cmd == SW_HIDE.0 {
            let _: () = msg1(st.panel, sel(b"orderOut:\0"), std::ptr::null_mut::<c_void>());
        } else {
            let _: () = msg0(st.panel, sel(b"orderFrontRegardless\0"));
        }
    }
}

pub unsafe fn set_window_pos(hwnd: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) {
    let Some(mut st) = windows().lock().ok().and_then(|m| m.get(&(hwnd.0 as usize)).cloned()) else {
        return;
    };
    let h0 = primary_screen_height();
    let sc: Id = msg0(st.panel, sel(b"screen\0"));
    let s = screen_scale(sc);
    let cur = msg0_rect(st.panel, sel(b"frame\0"));
    let new_w = if (flags & SWP_NOSIZE.0) != 0 { cur.size.width } else { (cx.max(1) as f64) / s };
    let new_h = if (flags & SWP_NOSIZE.0) != 0 { cur.size.height } else { (cy.max(1) as f64) / s };
    let (new_x, new_y) = if (flags & SWP_NOMOVE.0) != 0 {
        (cur.origin.x, cur.origin.y)
    } else {
        ((x as f64) / s, h0 - ((y as f64) / s + new_h))
    };
    let fr = CGRect { origin: CGPoint { x: new_x, y: new_y }, size: CGSize { width: new_w, height: new_h } };
    let _: () = msg2(st.panel, sel(b"setFrame:display:\0"), fr, false);
    if (flags & SWP_SHOWWINDOW.0) != 0 && (flags & SWP_NOACTIVATE.0) != 0 {
        let _: () = msg0(st.panel, sel(b"orderFrontRegardless\0"));
    }
    st.scale = s;
    if (flags & SWP_NOSIZE.0) == 0 {
        st.width_px = cx;
        st.height_px = cy;
    }
    if let Ok(mut m) = windows().lock() {
        m.insert(hwnd.0 as usize, st);
    }
}

pub unsafe fn get_window_rect(hwnd: HWND, rect: &mut RECT) -> bool {
    let Some(st) = windows().lock().ok().and_then(|m| m.get(&(hwnd.0 as usize)).cloned()) else {
        return false;
    };
    let h0 = primary_screen_height();
    let fr = msg0_rect(st.panel, sel(b"frame\0"));
    let top = h0 - (fr.origin.y + fr.size.height);
    *rect = RECT {
        left: (fr.origin.x * st.scale).round() as i32,
        top: (top * st.scale).round() as i32,
        right: ((fr.origin.x + fr.size.width) * st.scale).round() as i32,
        bottom: ((top + fr.size.height) * st.scale).round() as i32,
    };
    true
}

pub unsafe fn get_cursor_pos() -> POINT {
    let pt: CGPoint = {
        let f: unsafe extern "C" fn(Id, Sel) -> CGPoint =
            std::mem::transmute(objc_msgSend as *const c_void);
        f(cls(b"NSEvent\0") as Id, sel(b"mouseLocation\0"))
    };
    let h0 = primary_screen_height();
    let screens: Id = msg0(cls(b"NSScreen\0") as Id, sel(b"screens\0"));
    let sc: Id = if msg0::<usize>(screens, sel(b"count\0")) > 0 {
        msg1(screens, sel(b"objectAtIndex:\0"), 0usize)
    } else {
        std::ptr::null_mut()
    };
    let s = screen_scale(sc);
    POINT {
        x: (pt.x * s).round() as i32,
        y: ((h0 - pt.y) * s).round() as i32,
    }
}

pub unsafe fn set_window_rgn(hwnd: HWND, shape: Option<RgnShape>) {
    if let Ok(mut map) = windows().lock() {
        if let Some(st) = map.get_mut(&(hwnd.0 as usize)) {
            st.rgn = shape.clone();
            if !st.blur_view.is_null() {
                let layer: Id = msg0(st.blur_view, sel(b"layer\0"));
                if !layer.is_null() {
                    if let Some(RgnShape::Rect(r)) = shape {
                        let rad = ((r.bottom - r.top).min(r.right - r.left) as f64) / (2.0 * st.scale);
                        let _: () = msg1(layer, sel(b"setCornerRadius:\0"), rad.min(26.0));
                        let _: () = msg1(layer, sel(b"setMasksToBounds:\0"), true);
                    }
                }
            }
        }
    }
}

pub unsafe fn present_surface(hwnd: HWND, surf: &PixelSurface) {
    let Some(st) = windows().lock().ok().and_then(|m| m.get(&(hwnd.0 as usize)).cloned()) else {
        return;
    };
    if surf.width == 0 || surf.height == 0 || surf.pixels.is_empty() {
        return;
    }
    let layer: Id = msg0(st.view, sel(b"layer\0"));
    if layer.is_null() {
        return;
    }
    let cs = CGColorSpaceCreateDeviceRGB();
    let ctx = CGBitmapContextCreate(
        surf.pixels.as_ptr() as *mut c_void,
        surf.width as usize,
        surf.height as usize,
        8,
        (surf.width * 4) as usize,
        cs,
        KCG_BITMAP_BGRA_PREMUL,
    );
    CGColorSpaceRelease(cs);
    if ctx.is_null() {
        return;
    }
    let img = CGBitmapContextCreateImage(ctx);
    CGContextRelease(ctx);
    if img.is_null() {
        return;
    }
    let cat_cls = cls(b"CATransaction\0");
    let _: () = msg0(cat_cls as Id, sel(b"begin\0"));
    let _: () = msg1(cat_cls as Id, sel(b"setDisableActions:\0"), true);
    let _: () = msg1(layer, sel(b"setContentsScale:\0"), st.scale);
    let _: () = msg1(layer, sel(b"setContents:\0"), img as Id);
    let _: () = msg0(cat_cls as Id, sel(b"commit\0"));
    CGImageRelease(img);
}

pub unsafe fn activate_window(hwnd: HWND) {
    if let Some(st) = windows().lock().ok().and_then(|m| m.get(&(hwnd.0 as usize)).cloned()) {
        let _: () = msg1(st.panel, sel(b"makeKeyAndOrderFront:\0"), std::ptr::null_mut::<c_void>());
        push_msg(hwnd, WM_ACTIVATE, 1, 0);
    }
}

pub unsafe fn post_message(hwnd: Option<HWND>, msg: u32, wp: WPARAM, lp: LPARAM) {
    let h = hwnd.unwrap_or_default();
    push_msg(h, msg, wp.0, lp.0);
}

pub unsafe fn post_quit(_code: i32) {
    QUIT_FLAG.store(true, Ordering::Relaxed);
    CFRunLoopWakeUp(CFRunLoopGetMain());
}

extern "C" fn timer_callout(_timer: CFRunLoopTimerRef, info: *mut c_void) {
    let id = (info as usize) & 0xFFFF_FFFF;
    unsafe {
        let target_hwnd = windows()
            .lock()
            .ok()
            .and_then(|m| m.keys().next().copied())
            .unwrap_or(0);
        if target_hwnd != 0 {
            push_msg(HWND(target_hwnd as *mut c_void), WM_TIMER, id, 0);
        }
    }
}

pub unsafe fn set_timer(hwnd: Option<HWND>, id: usize, elapse_ms: u32) {
    kill_timer(hwnd, id);
    let interval = (elapse_ms.max(10) as f64) / 1000.0;
    let fire = CFAbsoluteTimeGetCurrent() + interval;
    let ctx_ptr = (id & 0xFFFF_FFFF) as *mut c_void;
    #[repr(C)]
    struct CfTimerCtx {
        version: CFIndex,
        info: *mut c_void,
        retain: *const c_void,
        release: *const c_void,
        copy_desc: *const c_void,
    }
    let ctx = CfTimerCtx {
        version: 0,
        info: ctx_ptr,
        retain: std::ptr::null(),
        release: std::ptr::null(),
        copy_desc: std::ptr::null(),
    };
    let timer = CFRunLoopTimerCreate(
        std::ptr::null(),
        fire,
        interval,
        0,
        0,
        timer_callout,
        &ctx as *const CfTimerCtx as *const c_void,
    );
    if !timer.is_null() {
        CFRunLoopAddTimer(CFRunLoopGetMain(), timer, kCFRunLoopDefaultMode);
        if let Ok(mut t) = timers().lock() {
            t.insert((hwnd.map(|h| h.0 as usize).unwrap_or(0), id), SendPtr(timer));
        }
    }
}

pub unsafe fn kill_timer(hwnd: Option<HWND>, id: usize) {
    if let Ok(mut t) = timers().lock() {
        if let Some(timer) = t.remove(&(hwnd.map(|h| h.0 as usize).unwrap_or(0), id)) {
            CFRunLoopTimerInvalidate(timer.0);
            CFRelease(timer.0 as CFTypeRef);
        }
    }
}

pub unsafe fn next_message(blocking: bool) -> Option<MSG> {
    ensure_nsapp();
    loop {
        if let Ok(mut q) = msg_queue().lock() {
            if let Some(m) = q.pop_front() {
                return Some(m);
            }
        }
        if QUIT_FLAG.load(Ordering::Relaxed) {
            return Some(MSG { message: WM_QUIT, ..Default::default() });
        }
        let app: Id = msg0(cls(b"NSApplication\0") as Id, sel(b"sharedApplication\0"));
        let until_date: Id = if blocking {
            msg0(cls(b"NSDate\0") as Id, sel(b"distantFuture\0"))
        } else {
            msg0(cls(b"NSDate\0") as Id, sel(b"distantPast\0"))
        };
        let mode = kCFRunLoopDefaultMode as Id;
        let ev: Id = msg4(
            app,
            sel(b"nextEventMatchingMask:untilDate:inMode:dequeue:\0"),
            u64::MAX,
            until_date,
            mode,
            true,
        );
        if !ev.is_null() {
            let _: () = msg1(app, sel(b"sendEvent:\0"), ev);
            let _: () = msg0(app, sel(b"updateWindows\0"));
        } else if !blocking {
            return None;
        }
        if let Ok(mut q) = msg_queue().lock() {
            if let Some(m) = q.pop_front() {
                return Some(m);
            }
        }
        if !blocking {
            return None;
        }
    }
}

// ---- Hotkeys, Keys, Tray, Fullscreen & Shell ----

fn vk_to_carbon_kc(vk: u32) -> Option<u32> {
    match vk {
        0x20 => Some(0x31), // Space
        0x41 => Some(0x00), // A
        0x42 => Some(0x0B), // B
        0x43 => Some(0x08), // C
        0x44 => Some(0x02), // D
        0x45 => Some(0x0E), // E
        0x46 => Some(0x03), // F
        0x47 => Some(0x05), // G
        0x48 => Some(0x04), // H
        0x49 => Some(0x22), // I
        0x4A => Some(0x26), // J
        0x4B => Some(0x28), // K
        0x4C => Some(0x25), // L
        0x4D => Some(0x2E), // M
        0x4E => Some(0x2D), // N
        0x4F => Some(0x1F), // O
        0x50 => Some(0x23), // P
        0x51 => Some(0x0C), // Q
        0x52 => Some(0x0F), // R
        0x53 => Some(0x01), // S
        0x54 => Some(0x11), // T
        0x55 => Some(0x20), // U
        0x56 => Some(0x09), // V
        0x57 => Some(0x0D), // W
        0x58 => Some(0x07), // X
        0x59 => Some(0x10), // Y
        0x5A => Some(0x06), // Z
        _ => None,
    }
}

static HOTKEY_REFS: OnceLock<Mutex<HashMap<i32, SendPtr>>> = OnceLock::new();

extern "C" fn carbon_hk_handler(_next: *mut c_void, event: *mut c_void, _ud: *mut c_void) -> i32 {
    unsafe {
        let mut hk_id = EventHotKeyID { signature: 0, id: 0 };
        let err = GetEventParameter(
            event,
            u32::from_be_bytes(*b"hkid"),
            u32::from_be_bytes(*b"hkid"),
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>() as u32,
            std::ptr::null_mut(),
            &mut hk_id as *mut _ as *mut c_void,
        );
        if err == 0 {
            if let Some(h) = windows().lock().ok().and_then(|m| m.keys().next().copied()) {
                push_msg(HWND(h as *mut c_void), WM_HOTKEY, hk_id.id as usize, 0);
            }
        }
    }
    0
}

pub unsafe fn register_hotkey(_hwnd: Option<HWND>, id: i32, mods: u32, vk: u32) {
    static HANDLER_INSTALLED: OnceLock<()> = OnceLock::new();
    HANDLER_INSTALLED.get_or_init(|| {
        let spec = EventTypeSpec {
            event_class: u32::from_be_bytes(*b"keyb"),
            event_kind: 6, // kEventHotKeyReleased
        };
        let mut out_ref = std::ptr::null_mut();
        InstallEventHandler(
            GetApplicationEventTarget(),
            carbon_hk_handler as *const c_void,
            1,
            &spec,
            std::ptr::null_mut(),
            &mut out_ref,
        );
    });
    let Some(kc) = vk_to_carbon_kc(vk) else { return };
    let mut c_mods = 0u32;
    if (mods & 1) != 0 { c_mods |= 1 << 11; } // optionKey
    if (mods & 2) != 0 { c_mods |= 1 << 12; } // controlKey
    if (mods & 4) != 0 { c_mods |= 1 << 9; }  // shiftKey
    if (mods & 8) != 0 { c_mods |= 1 << 8; }  // cmdKey
    let mut out_hk = std::ptr::null_mut();
    let hk_id = EventHotKeyID { signature: u32::from_be_bytes(*b"ISLD"), id: id as u32 };
    if RegisterEventHotKey(kc, c_mods, hk_id, GetApplicationEventTarget(), 0, &mut out_hk) == 0 && !out_hk.is_null() {
        let map = HOTKEY_REFS.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(mut g) = map.lock() {
            g.insert(id, SendPtr(out_hk));
        }
    }
}

pub unsafe fn unregister_hotkey(_hwnd: Option<HWND>, id: i32) {
    if let Some(map) = HOTKEY_REFS.get() {
        if let Ok(mut g) = map.lock() {
            if let Some(hk) = g.remove(&id) {
                UnregisterEventHotKey(hk.0);
            }
        }
    }
}

pub unsafe fn get_key_state(vk: i32) -> i16 {
    let down = match vk {
        0x10 => CGEventSourceKeyState(0, 56) || CGEventSourceKeyState(0, 60),
        0x11 => CGEventSourceKeyState(0, 59) || CGEventSourceKeyState(0, 62),
        0x12 => CGEventSourceKeyState(0, 58) || CGEventSourceKeyState(0, 61),
        0x5B | 0x5C => CGEventSourceKeyState(0, 55) || CGEventSourceKeyState(0, 54),
        0x01 => {
            let b: usize = msg0(cls(b"NSEvent\0") as Id, sel(b"pressedMouseButtons\0"));
            (b & 1) != 0
        }
        0x02 => {
            let b: usize = msg0(cls(b"NSEvent\0") as Id, sel(b"pressedMouseButtons\0"));
            (b & 2) != 0
        }
        _ => false,
    };
    if down { -32768i16 } else { 0 }
}

pub unsafe fn tray_notify(msg: u32, data: &NOTIFYICONDATAW) {
    ensure_nsapp();
    if msg == NIM_ADD.0 || msg == NIM_MODIFY.0 {
        let mut g = tray_state().lock().unwrap();
        let item = if let Some(it) = *g {
            it.0
        } else {
            let sb: Id = msg0(cls(b"NSStatusBar\0") as Id, sel(b"systemStatusBar\0"));
            let it: Id = msg1(sb, sel(b"statusItemWithLength:\0"), -1.0f64);
            let btn: Id = msg0(it, sel(b"button\0"));
            if !btn.is_null() {
                let title = nsstring("◐");
                let _: () = msg1(btn, sel(b"setTitle:\0"), title);
                CFRelease(title as CFTypeRef);
            }
            *g = Some(SendPtr(it));
            it
        };
        let btn: Id = msg0(item, sel(b"button\0"));
        if !btn.is_null() {
            let end = data.szTip.iter().position(|&c| c == 0).unwrap_or(data.szTip.len());
            if end > 0 {
                let tip = nsstring(&String::from_utf16_lossy(&data.szTip[..end]));
                let _: () = msg1(btn, sel(b"setToolTip:\0"), tip);
                CFRelease(tip as CFTypeRef);
            }
        }
    } else if msg == NIM_DELETE.0 {
        if let Ok(mut g) = tray_state().lock() {
            if let Some(it) = g.take() {
                let sb: Id = msg0(cls(b"NSStatusBar\0") as Id, sel(b"systemStatusBar\0"));
                let _: () = msg1(sb, sel(b"removeStatusItem:\0"), it.0);
            }
        }
    }
}

pub unsafe fn track_popup_menu(
    _hwnd: HWND,
    items: &[(u32, usize, String)],
    _x: i32,
    _y: i32,
) -> usize {
    let menu_cls = cls(b"NSMenu\0");
    let mi_cls = cls(b"NSMenuItem\0");
    let menu: Id = msg0(msg0(menu_cls as Id, sel(b"alloc\0")), sel(b"init\0"));
    for &(flags, id, ref title) in items {
        if (flags & MF_SEPARATOR.0) != 0 {
            let sep: Id = msg0(mi_cls as Id, sel(b"separatorItem\0"));
            let _: () = msg1(menu, sel(b"addItem:\0"), sep);
        } else {
            let t = nsstring(title);
            let ke = nsstring("");
            let alloc: Id = msg0(mi_cls as Id, sel(b"alloc\0"));
            let item: Id = msg3(alloc, sel(b"initWithTitle:action:keyEquivalent:\0"), t, std::ptr::null_mut::<c_void>(), ke);
            CFRelease(t as CFTypeRef);
            CFRelease(ke as CFTypeRef);
            let _: () = msg1(item, sel(b"setTag:\0"), id as isize);
            if (flags & MF_CHECKED.0) != 0 {
                let _: () = msg1(item, sel(b"setState:\0"), 1isize);
            }
            let _: () = msg1(menu, sel(b"addItem:\0"), item);
        }
    }
    let pt: CGPoint = {
        let f: unsafe extern "C" fn(Id, Sel) -> CGPoint =
            std::mem::transmute(objc_msgSend as *const c_void);
        f(cls(b"NSEvent\0") as Id, sel(b"mouseLocation\0"))
    };
    let _: bool = msg3(menu, sel(b"popUpMenuPositioningItem:atLocation:inView:\0"), std::ptr::null_mut::<c_void>(), pt, std::ptr::null_mut::<c_void>());
    0
}

pub unsafe fn is_fullscreen_app_active() -> bool {
    false
}

pub unsafe fn shell_open(target: &str, args: &str) {
    let mut cmd = std::process::Command::new("/usr/bin/open");
    cmd.arg(target);
    if !args.is_empty() {
        cmd.args(args.split_whitespace());
    }
    let _ = cmd.spawn();
}

pub unsafe fn gpu_name() -> String {
    let dev = MTLCreateSystemDefaultDevice();
    if dev.is_null() {
        return "Apple GPU".to_string();
    }
    let name: Id = msg0(dev, sel(b"name\0"));
    let s = nsstring_to_rust(name);
    if s.is_empty() { "Apple GPU".to_string() } else { s }
}

// ---- CoreGraphics Rasterization Hooks for d2d.rs ----

unsafe fn surf_cgcontext(surf: &mut PixelSurface) -> CGContextRef {
    let cs = CGColorSpaceCreateDeviceRGB();
    let ctx = CGBitmapContextCreate(
        surf.pixels.as_mut_ptr() as *mut c_void,
        surf.width as usize,
        surf.height as usize,
        8,
        (surf.width * 4) as usize,
        cs,
        KCG_BITMAP_BGRA_PREMUL,
    );
    CGColorSpaceRelease(cs);
    if !ctx.is_null() {
        // Flip Y so CoreGraphics matches Direct2D top-left origin
        CGContextTranslateCTM(ctx, 0.0, surf.height as f64);
        CGContextScaleCTM(ctx, 1.0, -1.0);
    }
    ctx
}

unsafe fn apply_d2d_matrix(ctx: CGContextRef, m: &Matrix3x2) {
    let t = CGAffineTransform {
        a: m.M11 as f64,
        b: m.M12 as f64,
        c: m.M21 as f64,
        d: m.M22 as f64,
        tx: m.M31 as f64,
        ty: m.M32 as f64,
    };
    CGContextConcatCTM(ctx, t);
}

unsafe fn emit_geometry(ctx: CGContextRef, geo: &GeometryData) {
    CGContextBeginPath(ctx);
    let mut cur = (0.0f64, 0.0f64);
    for &cmd in &geo.cmds {
        match cmd {
            PathCmd::Move(x, y) => {
                CGContextMoveToPoint(ctx, x, y);
                cur = (x, y);
            }
            PathCmd::Line(x, y) => {
                CGContextAddLineToPoint(ctx, x, y);
                cur = (x, y);
            }
            PathCmd::Quad(ax, ay, x, y) => {
                CGContextAddQuadCurveToPoint(ctx, ax, ay, x, y);
                cur = (x, y);
            }
            PathCmd::Cubic(ax, ay, bx, by, x, y) => {
                CGContextAddCurveToPoint(ctx, ax, ay, bx, by, x, y);
                cur = (x, y);
            }
            PathCmd::Arc { rx, ry, rot_deg, end_x, end_y, large, cw, .. } => {
                let mut pts = Vec::new();
                crate::d2d::arc_to_points(
                    crate::d2d::Pt2::new(cur.0, cur.1),
                    crate::d2d::Pt2::new(end_x, end_y),
                    rx,
                    ry,
                    rot_deg,
                    large,
                    cw,
                    0.15,
                    &mut pts,
                );
                for p in pts {
                    CGContextAddLineToPoint(ctx, p.x, p.y);
                }
                cur = (end_x, end_y);
            }
            PathCmd::Close => {
                CGContextClosePath(ctx);
            }
        }
    }
}

unsafe fn fill_with_brush(
    ctx: CGContextRef,
    geo: &GeometryData,
    brush: &BrushData,
    eo: bool,
) {
    match brush {
        BrushData::Solid(c) => {
            CGContextSetRGBFillColor(ctx, c.r as f64, c.g as f64, c.b as f64, c.a as f64);
            emit_geometry(ctx, geo);
            if eo { CGContextEOFillPath(ctx); } else { CGContextFillPath(ctx); }
        }
        BrushData::Linear { start, end, stops, opacity } => {
            if stops.is_empty() {
                return;
            }
            CGContextSaveGState(ctx);
            emit_geometry(ctx, geo);
            if eo { CGContextEOClip(ctx); } else { CGContextClip(ctx); }
            let cs = CGColorSpaceCreateDeviceRGB();
            let mut comps = Vec::with_capacity(stops.len() * 4);
            let mut locs = Vec::with_capacity(stops.len());
            for s in stops {
                comps.push(s.color.r as f64);
                comps.push(s.color.g as f64);
                comps.push(s.color.b as f64);
                comps.push((s.color.a * *opacity) as f64);
                locs.push(s.position as f64);
            }
            let grad = CGGradientCreateWithColorComponents(cs, comps.as_ptr(), locs.as_ptr(), stops.len());
            CGColorSpaceRelease(cs);
            if !grad.is_null() {
                CGContextDrawLinearGradient(
                    ctx,
                    grad,
                    CGPoint { x: start.X as f64, y: start.Y as f64 },
                    CGPoint { x: end.X as f64, y: end.Y as f64 },
                    3,
                );
                CGGradientRelease(grad);
            }
            CGContextRestoreGState(ctx);
        }
        BrushData::Radial { center, rx, ry, stops, opacity, .. } => {
            if stops.is_empty() {
                return;
            }
            CGContextSaveGState(ctx);
            emit_geometry(ctx, geo);
            if eo { CGContextEOClip(ctx); } else { CGContextClip(ctx); }
            let cs = CGColorSpaceCreateDeviceRGB();
            let mut comps = Vec::with_capacity(stops.len() * 4);
            let mut locs = Vec::with_capacity(stops.len());
            for s in stops {
                comps.push(s.color.r as f64);
                comps.push(s.color.g as f64);
                comps.push(s.color.b as f64);
                comps.push((s.color.a * *opacity) as f64);
                locs.push(s.position as f64);
            }
            let grad = CGGradientCreateWithColorComponents(cs, comps.as_ptr(), locs.as_ptr(), stops.len());
            CGColorSpaceRelease(cs);
            if !grad.is_null() {
                let c = CGPoint { x: center.X as f64, y: center.Y as f64 };
                CGContextDrawRadialGradient(ctx, grad, c, 0.0, c, (*rx).max(*ry) as f64, 2);
                CGGradientRelease(grad);
            }
            CGContextRestoreGState(ctx);
        }
        BrushData::Bitmap { surface, transform, opacity } => {
            let Ok(bmp) = surface.lock() else { return };
            if bmp.width == 0 || bmp.height == 0 {
                return;
            }
            let cs = CGColorSpaceCreateDeviceRGB();
            let bctx = CGBitmapContextCreate(
                bmp.pixels.as_ptr() as *mut c_void,
                bmp.width as usize,
                bmp.height as usize,
                8,
                (bmp.width * 4) as usize,
                cs,
                KCG_BITMAP_BGRA_PREMUL,
            );
            CGColorSpaceRelease(cs);
            if bctx.is_null() {
                return;
            }
            let img = CGBitmapContextCreateImage(bctx);
            CGContextRelease(bctx);
            if img.is_null() {
                return;
            }
            CGContextSaveGState(ctx);
            emit_geometry(ctx, geo);
            if eo { CGContextEOClip(ctx); } else { CGContextClip(ctx); }
            CGContextSetAlpha(ctx, *opacity as f64);
            apply_d2d_matrix(ctx, transform);
            CGContextTranslateCTM(ctx, 0.0, bmp.height as f64);
            CGContextScaleCTM(ctx, 1.0, -1.0);
            CGContextDrawImage(
                ctx,
                CGRect { origin: CGPoint::default(), size: CGSize { width: bmp.width as f64, height: bmp.height as f64 } },
                img,
            );
            CGContextRestoreGState(ctx);
            CGImageRelease(img);
        }
    }
}

pub unsafe fn d2d_fill_geometry(
    surf: &mut PixelSurface,
    geo: &GeometryData,
    brush: &BrushData,
    xform: &Matrix3x2,
    clip: Option<D2D_RECT_F>,
) {
    let ctx = surf_cgcontext(surf);
    if ctx.is_null() {
        return;
    }
    CGContextSaveGState(ctx);
    apply_d2d_matrix(ctx, xform);
    if let Some(c) = clip {
        CGContextClipToRect(
            ctx,
            CGRect {
                origin: CGPoint { x: c.left as f64, y: c.top as f64 },
                size: CGSize { width: (c.right - c.left).max(0.0) as f64, height: (c.bottom - c.top).max(0.0) as f64 },
            },
        );
    }
    fill_with_brush(ctx, geo, brush, geo.fill_mode == D2D1_FILL_MODE_ALTERNATE);
    CGContextRestoreGState(ctx);
    CGContextRelease(ctx);
}

pub unsafe fn d2d_draw_geometry(
    surf: &mut PixelSurface,
    geo: &GeometryData,
    brush: &BrushData,
    width: f32,
    style: Option<&StrokeStyleData>,
    xform: &Matrix3x2,
    clip: Option<D2D_RECT_F>,
) {
    let ctx = surf_cgcontext(surf);
    if ctx.is_null() {
        return;
    }
    CGContextSaveGState(ctx);
    apply_d2d_matrix(ctx, xform);
    if let Some(c) = clip {
        CGContextClipToRect(
            ctx,
            CGRect {
                origin: CGPoint { x: c.left as f64, y: c.top as f64 },
                size: CGSize { width: (c.right - c.left).max(0.0) as f64, height: (c.bottom - c.top).max(0.0) as f64 },
            },
        );
    }
    let col = match brush {
        BrushData::Solid(c) => *c,
        _ => brush.sample(0.0, 0.0),
    };
    CGContextSetRGBStrokeColor(ctx, col.r as f64, col.g as f64, col.b as f64, col.a as f64);
    CGContextSetLineWidth(ctx, width as f64);
    if let Some(st) = style {
        CGContextSetLineCap(ctx, match st.start_cap.0 { 2 => 1, 1 => 2, _ => 0 });
        CGContextSetLineJoin(ctx, match st.line_join.0 { 2 => 1, 1 => 2, _ => 0 });
        CGContextSetMiterLimit(ctx, st.miter_limit as f64);
    }
    emit_geometry(ctx, geo);
    CGContextStrokePath(ctx);
    CGContextRestoreGState(ctx);
    CGContextRelease(ctx);
}

pub unsafe fn d2d_draw_bitmap(
    dst: &mut PixelSurface,
    src: &PixelSurface,
    rect: D2D_RECT_F,
    opacity: f32,
    _src_rect: Option<&D2D_RECT_F>,
    xform: &Matrix3x2,
    clip: Option<D2D_RECT_F>,
) {
    if src.width == 0 || src.height == 0 {
        return;
    }
    let cs = CGColorSpaceCreateDeviceRGB();
    let bctx = CGBitmapContextCreate(
        src.pixels.as_ptr() as *mut c_void,
        src.width as usize,
        src.height as usize,
        8,
        (src.width * 4) as usize,
        cs,
        KCG_BITMAP_BGRA_PREMUL,
    );
    CGColorSpaceRelease(cs);
    if bctx.is_null() {
        return;
    }
    let img = CGBitmapContextCreateImage(bctx);
    CGContextRelease(bctx);
    if img.is_null() {
        return;
    }
    let ctx = surf_cgcontext(dst);
    if !ctx.is_null() {
        CGContextSaveGState(ctx);
        apply_d2d_matrix(ctx, xform);
        if let Some(c) = clip {
            CGContextClipToRect(
                ctx,
                CGRect {
                    origin: CGPoint { x: c.left as f64, y: c.top as f64 },
                    size: CGSize { width: (c.right - c.left).max(0.0) as f64, height: (c.bottom - c.top).max(0.0) as f64 },
                },
            );
        }
        CGContextSetAlpha(ctx, opacity as f64);
        let h = rect.bottom - rect.top;
        CGContextTranslateCTM(ctx, rect.left as f64, (rect.top + h) as f64);
        CGContextScaleCTM(ctx, 1.0, -1.0);
        CGContextDrawImage(
            ctx,
            CGRect { origin: CGPoint::default(), size: CGSize { width: (rect.right - rect.left) as f64, height: h as f64 } },
            img,
        );
        CGContextRestoreGState(ctx);
        CGContextRelease(ctx);
    }
    CGImageRelease(img);
}

pub unsafe fn d2d_push_layer(
    _surf: &mut PixelSurface,
    _bounds: D2D_RECT_F,
    _mask: Option<&GeometryData>,
    _mask_xform: &Matrix3x2,
    _opacity: f32,
) {
}

pub unsafe fn d2d_pop_layer(_surf: &mut PixelSurface) {}

// ---- CoreText Hooks for dwrite.rs & d2d.rs ----

pub unsafe fn dwrite_font_exists(family: &str) -> bool {
    let arr = CTFontManagerCopyAvailableFontFamilyNames();
    if arr.is_null() {
        return false;
    }
    let count = CFArrayGetCount(arr);
    let mut found = false;
    let mut buf = [0i8; 256];
    for i in 0..count {
        let s = CFArrayGetValueAtIndex(arr, i) as CFStringRef;
        if CFStringGetCString(s, buf.as_mut_ptr(), 256, KCF_STRING_ENCODING_UTF8) {
            if let Ok(name) = CStr::from_ptr(buf.as_ptr()).to_str() {
                if name.eq_ignore_ascii_case(family) {
                    found = true;
                    break;
                }
            }
        }
    }
    CFRelease(arr as CFTypeRef);
    found
}

unsafe fn make_ctfont(family: &str, size: f32, weight: i32) -> CTFontRef {
    let lower = family.to_ascii_lowercase();
    let base = if lower.contains("consolas") || lower.contains("mono") || lower.contains("menlo") {
        let c = CString::new("Menlo").unwrap_or_default();
        let s = CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), KCF_STRING_ENCODING_UTF8);
        let f = CTFontCreateWithName(s, size as f64, std::ptr::null());
        CFRelease(s as CFTypeRef);
        f
    } else if lower.starts_with("segoe") || lower.starts_with(".applesystem") || lower.starts_with("sf ") {
        CTFontCreateUIFontForLanguage(2, size as f64, std::ptr::null())
    } else {
        let c = CString::new(family).unwrap_or_default();
        let s = CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), KCF_STRING_ENCODING_UTF8);
        let f = CTFontCreateWithName(s, size as f64, std::ptr::null());
        CFRelease(s as CFTypeRef);
        if f.is_null() {
            CTFontCreateUIFontForLanguage(2, size as f64, std::ptr::null())
        } else {
            f
        }
    };
    if weight >= 580 && !base.is_null() {
        let bold = CTFontCreateCopyWithSymbolicTraits(base, size as f64, std::ptr::null(), 1 << 1, 1 << 1);
        if !bold.is_null() {
            CFRelease(base as CFTypeRef);
            return bold;
        }
    }
    base
}

unsafe fn make_attr_string(text: &str, font: CTFontRef, color: Option<D2D1_COLOR_F>, kern: f32) -> CFAttributedStringRef {
    let c = CString::new(text).unwrap_or_default();
    let cf_str = CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), KCF_STRING_ENCODING_UTF8);
    let dict = CFDictionaryCreateMutable(
        std::ptr::null(),
        4,
        &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
        &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
    );
    CFDictionarySetValue(dict, kCTFontAttributeName as *const c_void, font as *const c_void);
    if let Some(col) = color {
        let cs = CGColorSpaceCreateDeviceRGB();
        let comps = [col.r as f64, col.g as f64, col.b as f64, col.a as f64];
        let cg_col = CGColorCreate(cs, comps.as_ptr());
        CGColorSpaceRelease(cs);
        if !cg_col.is_null() {
            CFDictionarySetValue(dict, kCTForegroundColorAttributeName as *const c_void, cg_col);
            CFRelease(cg_col);
        }
    }
    if kern.abs() > 1e-4 {
        let kv = kern as f64;
        let num = CFNumberCreate(std::ptr::null(), KCF_NUMBER_FLOAT64_TYPE, &kv as *const f64 as *const c_void);
        if !num.is_null() {
            CFDictionarySetValue(dict, kCTKernAttributeName as *const c_void, num);
            CFRelease(num);
        }
    }
    let out = CFAttributedStringCreate(std::ptr::null(), cf_str, dict);
    CFRelease(cf_str as CFTypeRef);
    CFRelease(dict as CFTypeRef);
    out
}

pub unsafe fn dwrite_compute_layout(layout: &TextLayoutData) -> Option<ComputedLayout> {
    let font = make_ctfont(&layout.fmt.family, layout.fmt.font_size, layout.fmt.weight.0);
    if font.is_null() {
        return None;
    }
    let kern = layout.leading_spacing + layout.trailing_spacing;
    let attr = make_attr_string(&layout.text, font, None, kern);
    let ts = CTTypesetterCreateWithAttributedString(attr);
    CFRelease(attr as CFTypeRef);
    CFRelease(font as CFTypeRef);
    if ts.is_null() {
        return None;
    }

    let u16_len = layout.text.encode_utf16().count() as isize;
    let wrap_w = if layout.fmt.wrapping == DWRITE_WORD_WRAPPING_NO_WRAP || layout.max_width <= 0.0 {
        1e7f64
    } else {
        layout.max_width as f64
    };
    let mut lines = Vec::new();
    let mut glyphs = Vec::new();
    let mut start = 0isize;
    let mut max_w = 0.0f32;
    let mut total_h = 0.0f32;

    if u16_len == 0 {
        let line_h = layout.fmt.font_size * 1.32;
        lines.push(LayoutLineInfo {
            start_u16: 0,
            len_u16: 0,
            trailing_ws_u16: 0,
            newline_u16: 0,
            width: 0.0,
            height: line_h,
            baseline: layout.fmt.font_size * 1.04,
        });
        CFRelease(ts as CFTypeRef);
        return Some(ComputedLayout {
            metrics: DWRITE_TEXT_METRICS {
                left: 0.0,
                top: 0.0,
                width: 0.0,
                widthIncludingTrailingWhitespace: 0.0,
                height: line_h,
                layoutWidth: layout.max_width,
                layoutHeight: layout.max_height,
                maxBidiReorderingDepth: 1,
                lineCount: 1,
            },
            lines,
            glyphs,
        });
    }

    while start < u16_len {
        let count = CTTypesetterSuggestLineBreak(ts, start, wrap_w).max(1);
        let line = CTTypesetterCreateLine(ts, CFRange { location: start, length: count });
        let (mut asc, mut desc, mut lead) = (0.0f64, 0.0f64, 0.0f64);
        let w = if !line.is_null() {
            CTLineGetTypographicBounds(line, &mut asc, &mut desc, &mut lead) as f32
        } else {
            0.0
        };
        let line_h = ((asc + desc + lead) as f32).max(layout.fmt.font_size * 1.25);
        let base = (asc as f32).max(layout.fmt.font_size * 0.9);
        max_w = max_w.max(w);
        total_h += line_h;

        let lidx = lines.len();
        if !line.is_null() {
            for pos in start..(start + count) {
                let x0 = CTLineGetOffsetForStringIndex(line, pos, std::ptr::null_mut()) as f32;
                let x1 = CTLineGetOffsetForStringIndex(line, pos + 1, std::ptr::null_mut()) as f32;
                glyphs.push(GlyphPos {
                    u16_pos: pos as u32,
                    u16_len: 1,
                    line_idx: lidx,
                    x: x0,
                    width: (x1 - x0).max(0.0),
                });
            }
            CFRelease(line as CFTypeRef);
        }

        lines.push(LayoutLineInfo {
            start_u16: start as u32,
            len_u16: count as u32,
            trailing_ws_u16: 0,
            newline_u16: 0,
            width: w,
            height: line_h,
            baseline: base,
        });
        start += count;
    }

    CFRelease(ts as CFTypeRef);
    let left = match layout.fmt.text_align {
        DWRITE_TEXT_ALIGNMENT_CENTER if layout.max_width > 0.0 => ((layout.max_width - max_w) * 0.5).max(0.0),
        DWRITE_TEXT_ALIGNMENT_TRAILING if layout.max_width > 0.0 => (layout.max_width - max_w).max(0.0),
        _ => 0.0,
    };

    Some(ComputedLayout {
        metrics: DWRITE_TEXT_METRICS {
            left,
            top: 0.0,
            width: max_w,
            widthIncludingTrailingWhitespace: max_w,
            height: total_h,
            layoutWidth: layout.max_width,
            layoutHeight: layout.max_height,
            maxBidiReorderingDepth: 1,
            lineCount: lines.len() as u32,
        },
        lines,
        glyphs,
    })
}

unsafe fn draw_ct_text(
    surf: &mut PixelSurface,
    text: &str,
    fmt: &TextFormatData,
    origin_x: f32,
    origin_y: f32,
    max_w: f32,
    max_h: f32,
    color: D2D1_COLOR_F,
    kern: f32,
    xform: &Matrix3x2,
    clip: Option<D2D_RECT_F>,
) {
    let ctx = surf_cgcontext(surf);
    if ctx.is_null() {
        return;
    }
    CGContextSaveGState(ctx);
    apply_d2d_matrix(ctx, xform);
    if let Some(c) = clip {
        CGContextClipToRect(
            ctx,
            CGRect {
                origin: CGPoint { x: c.left as f64, y: c.top as f64 },
                size: CGSize { width: (c.right - c.left).max(0.0) as f64, height: (c.bottom - c.top).max(0.0) as f64 },
            },
        );
    }
    let font = make_ctfont(&fmt.family, fmt.font_size, fmt.weight.0);
    if !font.is_null() {
        let attr = make_attr_string(text, font, Some(color), kern);
        let ts = CTTypesetterCreateWithAttributedString(attr);
        CFRelease(attr as CFTypeRef);
        CFRelease(font as CFTypeRef);
        if !ts.is_null() {
            let wrap_w = if fmt.wrapping == DWRITE_WORD_WRAPPING_NO_WRAP || max_w <= 0.0 {
                1e7f64
            } else {
                max_w as f64
            };
            let u16_len = text.encode_utf16().count() as isize;
            let mut start = 0isize;
            let mut y_cur = origin_y as f64;
            // CoreText expects Y-up text matrix relative to unflipped axis when drawn in flipped CTM
            CGContextSetTextMatrix(ctx, CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: -1.0, tx: 0.0, ty: 0.0 });
            while start < u16_len {
                let count = CTTypesetterSuggestLineBreak(ts, start, wrap_w).max(1);
                let line = CTTypesetterCreateLine(ts, CFRange { location: start, length: count });
                if !line.is_null() {
                    let (mut asc, mut desc, mut lead) = (0.0, 0.0, 0.0);
                    let w = CTLineGetTypographicBounds(line, &mut asc, &mut desc, &mut lead);
                    let x = match fmt.text_align {
                        DWRITE_TEXT_ALIGNMENT_CENTER if max_w > 0.0 => origin_x as f64 + ((max_w as f64 - w) * 0.5).max(0.0),
                        DWRITE_TEXT_ALIGNMENT_TRAILING if max_w > 0.0 => origin_x as f64 + (max_w as f64 - w).max(0.0),
                        _ => origin_x as f64,
                    };
                    CGContextSetTextPosition(ctx, x, y_cur + asc);
                    CTLineDraw(line, ctx);
                    CFRelease(line as CFTypeRef);
                    y_cur += asc + desc + lead;
                    if max_h > 0.0 && y_cur - (origin_y as f64) > max_h as f64 {
                        break;
                    }
                }
                start += count;
            }
            CFRelease(ts as CFTypeRef);
        }
    }
    CGContextRestoreGState(ctx);
    CGContextRelease(ctx);
}

pub unsafe fn d2d_draw_text_layout(
    surf: &mut PixelSurface,
    origin: windows_numerics::Vector2,
    layout: &crate::dwrite::IDWriteTextLayout,
    color: D2D1_COLOR_F,
    _opts: D2D1_DRAW_TEXT_OPTIONS,
    xform: &Matrix3x2,
    clip: Option<D2D_RECT_F>,
) {
    let Ok(d) = layout.0.lock() else { return };
    draw_ct_text(
        surf,
        &d.text,
        &d.fmt,
        origin.X,
        origin.Y,
        d.max_width,
        d.max_height,
        color,
        d.leading_spacing + d.trailing_spacing,
        xform,
        clip,
    );
}

pub unsafe fn d2d_draw_text(
    surf: &mut PixelSurface,
    string: &[u16],
    format: &crate::dwrite::IDWriteTextFormat,
    rect: &D2D_RECT_F,
    color: D2D1_COLOR_F,
    _opts: D2D1_DRAW_TEXT_OPTIONS,
    xform: &Matrix3x2,
    clip: Option<D2D_RECT_F>,
) {
    let Ok(fmt) = format.0.lock() else { return };
    let text = String::from_utf16_lossy(string);
    draw_ct_text(
        surf,
        &text,
        &fmt,
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        color,
        0.0,
        xform,
        clip,
    );
}

// ---- ImageIO Decoding Hook for WIC ----

pub unsafe fn wic_decode_image(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if bytes.is_empty() {
        return None;
    }
    let data = CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex);
    if data.is_null() {
        return None;
    }
    let isrc = CGImageSourceCreateWithData(data, std::ptr::null());
    CFRelease(data);
    if isrc.is_null() {
        return None;
    }
    let img = CGImageSourceCreateImageAtIndex(isrc, 0, std::ptr::null());
    CFRelease(isrc);
    if img.is_null() {
        return None;
    }
    let w = CGImageGetWidth(img) as u32;
    let h = CGImageGetHeight(img) as u32;
    if w == 0 || h == 0 {
        CGImageRelease(img);
        return None;
    }
    let mut px = vec![0u8; (w as usize) * (h as usize) * 4];
    let cs = CGColorSpaceCreateDeviceRGB();
    let ctx = CGBitmapContextCreate(
        px.as_mut_ptr() as *mut c_void,
        w as usize,
        h as usize,
        8,
        (w * 4) as usize,
        cs,
        KCG_BITMAP_BGRA_PREMUL,
    );
    CGColorSpaceRelease(cs);
    if !ctx.is_null() {
        CGContextDrawImage(
            ctx,
            CGRect { origin: CGPoint::default(), size: CGSize { width: w as f64, height: h as f64 } },
            img,
        );
        CGContextRelease(ctx);
    }
    CGImageRelease(img);
    Some((w, h, px))
}

#[allow(unused_imports)]
use windows_core::{Error as _UnusedError};
const _: fn() = || {
    let _ = std::mem::size_of::<DWRITE_HIT_TEST_METRICS>();
};
