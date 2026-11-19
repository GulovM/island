//! Ночной свет (Night Shift на macOS + бинарный разбор для совместимости с тестами).

const CB: [u8; 4] = [0x43, 0x42, 0x01, 0x00];
const NIGHT_HEAD: [u8; 6] = [0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06];
const NIGHT_BODY: [u8; 3] = [0x2A, 0x2B, 0x0E];
const NIGHT_ON: [u8; 2] = [0x10, 0x00];
const NIGHT_NEXT: [u8; 2] = [0xD0, 0x0A];

pub fn night_light() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "com.apple.CoreBrightness"])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        if s.contains("CBBlueReductionStatus") {
            return Some(s.contains("Enabled = 1"));
        }
        Some(false)
    }
    #[cfg(not(target_os = "macos"))]
    None
}

pub fn set_night_light(on: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        if unsafe { mac::set_night_shift(on) } {
            return true;
        }
    }
    let _ = on;
    false
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void};

    extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    pub unsafe fn set_night_shift(on: bool) -> bool {
        let h = dlopen(
            c"/System/Library/PrivateFrameworks/CoreBrightness.framework/CoreBrightness".as_ptr(),
            1,
        );
        if h.is_null() {
            return false;
        }
        let cls = objc_getClass(c"CBBlueLightClient".as_ptr());
        if cls.is_null() {
            return false;
        }
        let alloc: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const c_void);
        let obj = alloc(cls, sel_registerName(c"alloc".as_ptr()));
        let obj = alloc(obj, sel_registerName(c"init".as_ptr()));
        if obj.is_null() {
            return false;
        }
        let set_enabled: unsafe extern "C" fn(*mut c_void, *mut c_void, bool) -> bool =
            std::mem::transmute(objc_msgSend as *const c_void);
        let ok = set_enabled(obj, sel_registerName(c"setEnabled:".as_ptr()), on);
        let release: unsafe extern "C" fn(*mut c_void, *mut c_void) =
            std::mem::transmute(objc_msgSend as *const c_void);
        release(obj, sel_registerName(c"release".as_ptr()));
        ok
    }
}

#[allow(dead_code)]
fn night_parse(d: &[u8]) -> Option<(bool, u64, usize, usize)> {
    if d.get(0..4)? != CB || d.get(4..10)? != NIGHT_HEAD {
        return None;
    }
    let (mut ts, mut i, mut shift) = (0u64, 10usize, 0u32);
    loop {
        let b = *d.get(i)?;
        ts |= ((b & 0x7F) as u64) << shift;
        i += 1;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 56 {
            return None;
        }
    }
    if d.get(i..i + 3)? != NIGHT_BODY {
        return None;
    }
    let len = *d.get(i + 3)? as usize;
    let p = i + 4;
    if len >= 0x80 || d.len() < p + len || d.get(p..p + 4)? != CB {
        return None;
    }
    let on = match d.get(p + 4..p + 6)? {
        x if x == NIGHT_ON && d.get(p + 6..p + 8)? == NIGHT_NEXT => true,
        x if x == NIGHT_NEXT => false,
        _ => return None,
    };
    if len < if on { 8 } else { 6 } {
        return None;
    }
    Some((on, ts, p, len))
}

#[allow(dead_code)]
fn leb128(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

#[allow(dead_code)]
fn night_build(d: &[u8], on: bool, now: u64) -> Option<Vec<u8>> {
    let (cur, ts, p, len) = night_parse(d)?;
    let mut payload = CB.to_vec();
    if on {
        payload.extend_from_slice(&NIGHT_ON);
    }
    let skip = if cur { 6 } else { 4 };
    payload.extend_from_slice(&d[p + skip..p + len]);
    let mut out = CB.to_vec();
    out.extend_from_slice(&NIGHT_HEAD);
    out.extend(leb128(now.max(ts.saturating_add(1))));
    out.extend_from_slice(&NIGHT_BODY);
    if payload.len() >= 0x80 {
        return None;
    }
    out.push(payload.len() as u8);
    out.extend(payload);
    out.extend_from_slice(&d[p + len..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: [u8; 41] = [
        0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0xDE, 0x93, 0x97, 0xCF, 0x06, 0x2A, 0x2B, 0x0E, 0x13, 0x43, 0x42,
        0x01, 0x00, 0xD0, 0x0A, 0x02, 0xC6, 0x14, 0xD7, 0xAA, 0xAA, 0x92, 0xED, 0xDE, 0x8C, 0xEE, 0x01, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn parses_real_record() {
        let (on, ts, p, len) = night_parse(&OFF).unwrap();
        assert!(!on);
        assert_eq!(ts, 1_776_667_102);
        assert_eq!((p, len), (19, 0x13));
    }

    #[test]
    fn enable_then_disable_roundtrips() {
        let on = night_build(&OFF, true, 1_800_000_000).unwrap();
        assert_eq!(on.len(), 43);
        assert_eq!(on[18], 0x15);
        assert_eq!(&on[23..27], &[0x10, 0x00, 0xD0, 0x0A]);
        assert_eq!(night_parse(&on).map(|p| p.0), Some(true));
        let off = night_build(&on, false, 1_800_000_001).unwrap();
        assert_eq!(off.len(), OFF.len());
        assert_eq!(&off[15..], &OFF[15..]);
        assert_eq!(night_parse(&off).map(|p| (p.0, p.1)), Some((false, 1_800_000_001)));
    }

    #[test]
    fn time_never_goes_back() {
        let on = night_build(&OFF, true, 5).unwrap();
        assert_eq!(night_parse(&on).unwrap().1, 1_776_667_103);
    }

    #[test]
    fn unknown_format_is_refused() {
        let mut bad = OFF;
        bad[23] = 0x77;
        assert!(night_parse(&bad).is_none() && night_build(&bad, true, 1).is_none());
        assert!(night_parse(&OFF[..20]).is_none());
        assert!(night_parse(&[]).is_none());
    }
}
