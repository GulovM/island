use std::ffi::c_void;
use std::time::Instant;

use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

use crate::sys;

pub(super) struct Resp {
    pub(super) status: u32,
    pub(super) body: Vec<u8>,
    pub(super) total: u64,
    pub(super) secs: f64,
}

/// Хэндл WinHTTP, закрывается при выходе из области видимости.
struct H(*mut c_void);

impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

pub(super) fn split_url(url: &str) -> Result<(bool, String, u16, String), String> {
    let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("webcal://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("неподдерживаемый адрес: {url}"));
    };
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().unwrap_or(443)),
        _ => (hostport, if secure { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(format!("нет хоста: {url}"));
    }
    Ok((secure, host.to_string(), port, path.to_string()))
}

/// Запрос GET (или POST с `upload` байт нулей). Тело сохраняется до `keep` байт, но считается целиком.
pub(super) unsafe fn http(url: &str, upload: Option<usize>, keep: usize) -> Result<Resp, String> {
    let (secure, host, port, path) = split_url(url)?;
    let session = H(WinHttpOpen(w!("Island/0.1"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
    if session.0.is_null() {
        return Err("WinHttpOpen".into());
    }
    let _ = WinHttpSetTimeouts(session.0, 8000, 8000, 20_000, 20_000);
    let hostw = sys::wz(&host);
    let conn = H(WinHttpConnect(session.0, PCWSTR(hostw.as_ptr()), port, 0));
    if conn.0.is_null() {
        return Err(format!("нет соединения с {host}"));
    }
    let pathw = sys::wz(&path);
    let verb = if upload.is_some() { w!("POST") } else { w!("GET") };
    let flags = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
    let req = H(WinHttpOpenRequest(conn.0, verb, PCWSTR(pathw.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
    if req.0.is_null() {
        return Err("WinHttpOpenRequest".into());
    }
    let t0 = Instant::now();
    match upload {
        None => WinHttpSendRequest(req.0, None, None, 0, 0, 0).map_err(|e| e.to_string())?,
        Some(n) => {
            WinHttpSendRequest(req.0, None, None, 0, n as u32, 0).map_err(|e| e.to_string())?;
            let chunk = vec![0u8; 64 * 1024];
            let mut sent = 0usize;
            while sent < n {
                let k = (n - sent).min(chunk.len());
                let mut wr = 0u32;
                WinHttpWriteData(req.0, Some(chunk.as_ptr() as *const c_void), k as u32, &mut wr).map_err(|e| e.to_string())?;
                if wr == 0 {
                    return Err("отправка оборвалась".into());
                }
                sent += wr as usize;
            }
        }
    }
    WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|e| e.to_string())?;
    let (mut status, mut len) = (0u32, 4u32);
    let _ = WinHttpQueryHeaders(
        req.0,
        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
        PCWSTR::null(),
        Some(&mut status as *mut u32 as *mut c_void),
        &mut len,
        std::ptr::null_mut(),
    );
    let mut body = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let mut avail = 0u32;
        WinHttpQueryDataAvailable(req.0, &mut avail).map_err(|e| e.to_string())?;
        if avail == 0 {
            break;
        }
        let want = (avail as usize).min(buf.len());
        let mut got = 0u32;
        WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, want as u32, &mut got).map_err(|e| e.to_string())?;
        if got == 0 {
            break;
        }
        total += got as u64;
        if body.len() < keep {
            let room = keep - body.len();
            body.extend_from_slice(&buf[..(got as usize).min(room)]);
        }
    }
    Ok(Resp { status, body, total, secs: t0.elapsed().as_secs_f64() })
}

pub(crate) fn get_text(url: &str) -> Result<String, String> {
    let r = unsafe { http(url, None, 4 << 20)? };
    if r.status >= 400 {
        return Err(format!("HTTP {} для {url}", r.status));
    }
    Ok(String::from_utf8_lossy(&r.body).into_owned())
}

/// Скачивает файл целиком в память; больше `max` байт — ошибка.
pub(crate) fn get_bytes(url: &str, max: usize) -> Result<Vec<u8>, String> {
    let r = unsafe { http(url, None, max)? };
    if r.status >= 400 {
        return Err(format!("HTTP {} для {url}", r.status));
    }
    if r.total as usize > max {
        return Err(format!("файл больше {max} байт"));
    }
    Ok(r.body)
}

pub(super) fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Число по ключу `"key":` (первое вхождение).
pub(super) fn json_num(text: &str, key: &str) -> Option<f64> {
    let i = text.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = text[i..].trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Строка по ключу `"key":"…"` (первое вхождение).
pub(crate) fn json_str(text: &str, key: &str) -> Option<String> {
    let i = text.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = text[i..].trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut it = rest.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return Some(out),
            '\\' => {
                if let Some(e) = it.next() {
                    out.push(if e == 'n' { ' ' } else { e });
                }
            }
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_json_helpers() {
        assert_eq!(urlenc("Москва"), "%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0");
        assert_eq!(split_url("https://a.b:8443/x?y").unwrap(), (true, "a.b".into(), 8443, "/x?y".into()));
        assert!(split_url("webcal://cal.example/p.ics").unwrap().0);
        let j = r#"{"results":[{"id":1,"name":"Москва","latitude":55.75,"longitude":37.61}]}"#;
        assert_eq!(json_num(j, "latitude"), Some(55.75));
        assert_eq!(json_str(j, "name").as_deref(), Some("Москва"));
        let fc = r#"{"current_units":{"temperature_2m":"°C"},"current":{"temperature_2m":-3.5,"weather_code":71}}"#;
        let cur = &fc[fc.find("\"current\":{").unwrap()..];
        assert_eq!(json_num(cur, "temperature_2m"), Some(-3.5));
    }
}
