//! Яркость встроенного дисплея Mac через `DisplayServices` (`dlopen`/`dlsym`).

use std::sync::atomic::{AtomicBool, Ordering};

static OK: AtomicBool = AtomicBool::new(false);

pub fn available() -> bool {
    OK.load(Ordering::Relaxed)
}

pub fn probe() {
    OK.store(get().is_some(), Ordering::Relaxed);
}

pub fn get() -> Option<f32> {
    #[cfg(target_os = "macos")]
    {
        mac::get()
    }
    #[cfg(not(target_os = "macos"))]
    None
}

pub fn get_panel() -> Option<f32> {
    if !available() {
        return None;
    }
    get()
}

pub fn set(v: f32) {
    let clamped = v.clamp(0.0, 1.0);
    #[cfg(target_os = "macos")]
    mac::set(clamped);
    #[cfg(not(target_os = "macos"))]
    let _ = clamped;
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> u32;
    }

    extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, sym: *const c_char) -> *mut c_void;
    }

    type FnGet = unsafe extern "C" fn(u32, *mut f32) -> i32;
    type FnSet = unsafe extern "C" fn(u32, f32) -> i32;

    struct Api {
        get: Option<FnGet>,
        set: Option<FnSet>,
    }
    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    fn api() -> &'static Api {
        static API: OnceLock<Api> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let path = b"/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices\0";
            let h = dlopen(path.as_ptr() as *const c_char, 1);
            if h.is_null() {
                return Api { get: None, set: None };
            }
            let g = dlsym(h, c"DisplayServicesGetBrightness".as_ptr());
            let s = dlsym(h, c"DisplayServicesSetBrightness".as_ptr());
            Api {
                get: (!g.is_null()).then(|| std::mem::transmute::<*mut c_void, FnGet>(g)),
                set: (!s.is_null()).then(|| std::mem::transmute::<*mut c_void, FnSet>(s)),
            }
        })
    }

    pub fn get() -> Option<f32> {
        unsafe {
            let f = api().get?;
            let mut val = 0.0f32;
            if f(CGMainDisplayID(), &mut val) == 0 && (0.0..=1.0).contains(&val) {
                Some(val)
            } else {
                None
            }
        }
    }

    pub fn set(v: f32) {
        unsafe {
            if let Some(f) = api().set {
                let _ = f(CGMainDisplayID(), v);
            }
        }
    }
}
