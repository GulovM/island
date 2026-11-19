use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

pub(crate) struct Resp {
    pub status: u16,
    pub final_url: String,
    pub body: Vec<u8>,
    pub(super) total: u64,
    pub(super) secs: f64,
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

/// Запрос GET (или POST с `upload` байт нулей) через `curl` на macOS.
pub(super) unsafe fn http(url: &str, upload: Option<usize>, keep: usize) -> Result<Resp, String> {
    let _ = split_url(url)?;
    let normalized = url.strip_prefix("webcal://").map(|r| format!("https://{r}")).unwrap_or_else(|| url.to_string());
    let payload = upload.map(|n| vec![0u8; n]);
    let mut r = request(
        if payload.is_some() { "POST" } else { "GET" },
        &normalized,
        &[],
        payload.as_deref(),
        keep,
    )?;
    if upload.is_some() {
        r.total = upload.unwrap_or(0) as u64;
    }
    Ok(r)
}

pub(crate) fn resolve_redirect_url(base_url: &str, location: &str) -> Option<String> {
    let loc = location.trim();
    if loc.is_empty() {
        return None;
    }
    if loc.starts_with("https://") || loc.starts_with("http://") || loc.starts_with("webcal://") {
        return Some(loc.to_string());
    }
    let (secure, host, port, base_path) = split_url(base_url).ok()?;
    let scheme = if secure { "https" } else { "http" };
    let default_port = if secure { 443 } else { 80 };
    let authority = if port == default_port {
        host
    } else {
        format!("{host}:{port}")
    };
    if loc.starts_with('/') {
        Some(format!("{scheme}://{authority}{loc}"))
    } else {
        let dir = match base_path.rfind('/') {
            Some(i) => &base_path[..=i],
            None => "/",
        };
        Some(format!("{scheme}://{authority}{dir}{loc}"))
    }
}

pub(crate) fn is_trusted_auth_redirect(url: &str) -> bool {
    let Ok((secure, host, _, _)) = split_url(url) else {
        return false;
    };
    let h = host.to_ascii_lowercase();
    #[cfg(test)]
    if h == "127.0.0.1" || h == "localhost" {
        return true;
    }
    if !secure {
        return false;
    }
    h == "icloud.com"
        || h.ends_with(".icloud.com")
        || h == "googleusercontent.com"
        || h.ends_with(".googleusercontent.com")
        || h == "google.com"
        || h.ends_with(".google.com")
        || h == "googleapis.com"
        || h.ends_with(".googleapis.com")
}

/// Произвольный HTTP-запрос (`GET`, `POST`, `PROPFIND`, `REPORT`) с заголовками и ручной обработкой редиректов.
pub(crate) unsafe fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    keep: usize,
) -> Result<Resp, String> {
    let curl = if cfg!(target_os = "macos") { "/usr/bin/curl" } else { "curl" };
    let mut cur_url = url.strip_prefix("webcal://").map(|r| format!("https://{r}")).unwrap_or_else(|| url.to_string());
    let mut allow_auth = true;
    let t0 = Instant::now();

    for _hop in 0..=5 {
        let _ = split_url(&cur_url)?;
        let mut cmd = Command::new(curl);
        cmd.arg("-sS")
            .arg("-X")
            .arg(method)
            .arg("--connect-timeout")
            .arg("8")
            .arg("--max-time")
            .arg("20")
            .arg("--dump-header")
            .arg("-")
            .arg("--user-agent")
            .arg("KiteIsland/1.9");
        for &(k, v) in headers {
            if !allow_auth && k.eq_ignore_ascii_case("Authorization") {
                continue;
            }
            cmd.arg("-H").arg(format!("{k}: {v}"));
        }
        if body.is_some() {
            cmd.arg("--data-binary").arg("@-").stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        cmd.arg(&cur_url).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("curl: {e}"))?;
        if let Some(bytes) = body {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(bytes);
            }
        }
        let out = child.wait_with_output().map_err(|e| format!("curl wait: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() { format!("http error {}", out.status) } else { err });
        }
        let raw = &out.stdout;
        let sep = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| (i, 4))
            .or_else(|| raw.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2)));
        let (hdr_bytes, body_bytes) = match sep {
            Some((i, n)) => (&raw[..i], &raw[i + n..]),
            None => (&raw[..], &[][..]),
        };
        let hdr_text = String::from_utf8_lossy(hdr_bytes);
        let status = hdr_text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(200);
        if matches!(status, 301 | 302 | 307 | 308) {
            if let Some(loc) = hdr_text
                .lines()
                .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("location")).map(|(_, v)| v.trim().to_string()))
            {
                if let Some(next_url) = resolve_redirect_url(&cur_url, &loc) {
                    allow_auth = allow_auth && is_trusted_auth_redirect(&next_url);
                    cur_url = next_url;
                    continue;
                }
            }
        }
        let total = body_bytes.len() as u64;
        let kept = body_bytes[..body_bytes.len().min(keep)].to_vec();
        return Ok(Resp {
            status,
            final_url: cur_url,
            body: kept,
            total,
            secs: t0.elapsed().as_secs_f64(),
        });
    }

    Err(format!("слишком много перенаправлений для {url}"))
}

const B64_STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub(crate) fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_STD[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_STD[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_STD[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_STD[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub(crate) fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_URL[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_URL[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_URL[((triple >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64_URL[(triple & 0x3F) as usize] as char);
        }
    }
    out
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

fn read_hex4(it: &mut std::str::Chars<'_>) -> Option<u16> {
    let mut val = 0u16;
    for _ in 0..4 {
        let d = it.next()?.to_digit(16)? as u16;
        val = (val << 4) | d;
    }
    Some(val)
}

fn json_str_inner(text: &str, key: &str, preserve_newlines: bool) -> Option<String> {
    let pat = format!("\"{key}\"");
    let mut search_from = 0;
    let rest = loop {
        let pos = text[search_from..].find(&pat)?;
        let after_key = &text[search_from + pos + pat.len()..].trim_start();
        if let Some(after_colon) = after_key.strip_prefix(':') {
            break after_colon.trim_start().strip_prefix('"')?;
        }
        search_from += pos + pat.len();
    };
    decode_json_str(rest, preserve_newlines)
}

/// Тело JSON-строки от символа после открывающей кавычки до закрывающей, с разбором экранирования.
fn decode_json_str(rest: &str, preserve_newlines: bool) -> Option<String> {
    let mut out = String::new();
    let mut it = rest.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'n' => out.push(if preserve_newlines { '\n' } else { ' ' }),
                'r' => {
                    if !preserve_newlines {
                        out.push(' ');
                    }
                }
                't' => out.push(if preserve_newlines { '\t' } else { ' ' }),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'u' => {
                    let hi = read_hex4(&mut it)?;
                    if (0xD800..=0xDBFF).contains(&hi) {
                        let mut clone = it.clone();
                        if clone.next() == Some('\\') && clone.next() == Some('u') {
                            if let Some(lo) = read_hex4(&mut clone) {
                                if (0xDC00..=0xDFFF).contains(&lo) {
                                    it = clone;
                                    let cp = 0x10000 + (((hi as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                                    if let Some(ch) = char::from_u32(cp) {
                                        out.push(ch);
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    if let Some(ch) = char::from_u32(hi as u32) {
                        out.push(ch);
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// Строка по ключу `"key":"…"` (первое вхождение, переводы строк заменяются пробелами).
pub(crate) fn json_str(text: &str, key: &str) -> Option<String> {
    json_str_inner(text, key, false)
}

/// Многострочная строка по ключу `"key":"…"` (сохраняет `\n` и декодирует `\uXXXX`).
pub(crate) fn json_str_multiline(text: &str, key: &str) -> Option<String> {
    json_str_inner(text, key, true)
}

// `json_str` ищет первое вхождение ключа во всём тексте — для массивов однотипных объектов
// (события Google Calendar с вложенными `start`/`end`) нужен разбор с учётом вложенности.

/// Границы JSON-значения, начинающегося с позиции `i` (после пробелов): строка с экранированием,
/// объект или массив целиком, число или литерал. Индексы сравниваются только по ASCII-байтам,
/// поэтому срезы по ним не режут многобайтовые символы.
fn json_value_span(s: &str, i: usize) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut i = i;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    match *b.get(i)? {
        b'"' => {
            i += 1;
            while i < b.len() {
                match b[i] {
                    b'\\' => i += 2,
                    b'"' => return Some((start, i + 1)),
                    _ => i += 1,
                }
            }
            None
        }
        b'{' | b'[' => {
            let mut depth = 0usize;
            while i < b.len() {
                match b[i] {
                    b'"' => {
                        i = json_value_span(s, i)?.1;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some((start, i + 1));
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        _ => {
            let end = s[i..].find(|c: char| matches!(c, ',' | '}' | ']') || c.is_whitespace()).map_or(b.len(), |n| i + n);
            (end > start).then_some((start, end))
        }
    }
}

/// Сырое значение поля `key` верхнего уровня объекта `obj` (`{…}`); вложенные объекты не просматриваются.
pub(crate) fn json_field<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let b = obj.as_bytes();
    let mut i = obj.find('{')? + 1;
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        if *b.get(i)? != b'"' {
            return None;
        }
        let (ks, ke) = json_value_span(obj, i)?;
        let name = &obj[ks + 1..ke - 1];
        i = ke;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if *b.get(i)? != b':' {
            return None;
        }
        let (vs, ve) = json_value_span(obj, i + 1)?;
        if name == key {
            return Some(&obj[vs..ve]);
        }
        i = ve;
    }
}

/// Вложенный объект `{…}` по ключу верхнего уровня.
pub(crate) fn json_obj<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    json_field(obj, key).filter(|v| v.starts_with('{'))
}

/// Элементы массива по ключу верхнего уровня (сырые срезы); нет поля или не массив — пусто.
pub(crate) fn json_array<'a>(obj: &'a str, key: &str) -> Vec<&'a str> {
    let Some(arr) = json_field(obj, key).filter(|v| v.starts_with('[')) else {
        return Vec::new();
    };
    let b = arr.as_bytes();
    let mut out = Vec::new();
    let mut i = 1;
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        if i >= b.len() || b[i] == b']' {
            break;
        }
        let Some((s, e)) = json_value_span(arr, i) else { break };
        out.push(&arr[s..e]);
        i = e;
    }
    out
}

/// Строковое поле верхнего уровня (переводы строк заменяются пробелами, как в `json_str`).
pub(crate) fn json_field_str(obj: &str, key: &str) -> Option<String> {
    decode_json_str(json_field(obj, key)?.strip_prefix('"')?, false)
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

    #[test]
    fn base64_and_base64url_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
        let rfc7636_bytes: [u8; 32] = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212, 37,
            77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        assert_eq!(
            base64url_encode(&rfc7636_bytes),
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        );
    }

    #[test]
    fn json_str_multiline_and_unicode() {
        let j = r###"{"body": "## [1.3.0]\r\n- Пункт 1\n- \u041f\u0443\u043d\u043a\u0442 2 \ud83d\ude80"}"###;
        assert_eq!(
            json_str_multiline(j, "body").as_deref(),
            Some("## [1.3.0]\n- Пункт 1\n- Пункт 2 🚀")
        );
    }

    #[test]
    fn json_nested_fields_and_arrays() {
        let j = r#"{"kind":"x","items":[
            {"id":"a1","status":"confirmed","creator":{"id":"nested","summary":"не то"},"summary":"Встреча, \"план\"","start":{"dateTime":"2026-11-12T10:00:00+03:00"},"end":{"dateTime":"2026-11-12T11:30:00+03:00"}},
            {"id":"b2","status":"cancelled","start":{"date":"2026-11-13"},"end":{"date":"2026-11-14"}},
            {"id":"c3","summary":"скобка } и [ в строке","start":{"date":"2026-11-15"}}
        ],"nextPageToken":null}"#;
        let items = json_array(j, "items");
        assert_eq!(items.len(), 3);
        assert_eq!(json_field_str(items[0], "id").as_deref(), Some("a1"));
        assert_eq!(json_field_str(items[0], "summary").as_deref(), Some("Встреча, \"план\""));
        let start = json_obj(items[0], "start").unwrap();
        assert_eq!(json_field_str(start, "dateTime").as_deref(), Some("2026-11-12T10:00:00+03:00"));
        assert_eq!(json_field_str(items[1], "status").as_deref(), Some("cancelled"));
        assert_eq!(json_field_str(items[2], "summary").as_deref(), Some("скобка } и [ в строке"));
        assert!(json_obj(items[2], "end").is_none());
        assert_eq!(json_field(j, "nextPageToken"), Some("null"));
        assert!(json_array(j, "kind").is_empty());
        assert!(json_array("{\"items\":[", "items").is_empty());
        assert_eq!(json_field("{\"a\":true,\"b\":false}", "b"), Some("false"));
    }

    #[test]
    fn redirect_resolution_and_trust() {
        assert_eq!(
            resolve_redirect_url("https://caldav.icloud.com/123/calendars/", "/456/calendars/").as_deref(),
            Some("https://caldav.icloud.com/456/calendars/")
        );
        assert_eq!(
            resolve_redirect_url("https://caldav.icloud.com:8443/a/b", "c").as_deref(),
            Some("https://caldav.icloud.com:8443/a/c")
        );
        assert!(is_trusted_auth_redirect("https://p42-caldav.icloud.com/123/principal/"));
        assert!(is_trusted_auth_redirect("https://apidata.googleusercontent.com/caldav/v2/"));
        assert!(is_trusted_auth_redirect("https://www.googleapis.com/calendar/v3/users/me/calendarList"));
        assert!(!is_trusted_auth_redirect("https://evilgoogleapis.com/"));
        assert!(!is_trusted_auth_redirect("http://p42-caldav.icloud.com/123/"));
        assert!(!is_trusted_auth_redirect("https://evil-icloud.com/123/"));
    }
}
