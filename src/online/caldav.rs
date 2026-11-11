//! Клиент протокола CalDAV (RFC 4791 / WebDAV RFC 4918) поверх WinHTTP.
//! Реализует 3-шаговое обнаружение календарей (`PROPFIND`) и выборку событий за период (`REPORT`)
//! для Google Calendar и Apple iCloud Calendar без внешних XML-зависимостей.

use super::http;

pub const GOOGLE_CALDAV_BASE: &str = "https://apidata.googleusercontent.com/caldav/v2/";
pub const APPLE_CALDAV_BASE: &str = "https://caldav.icloud.com/";

const MAX_CALDAV_BYTES: usize = 2 << 20;

/// Разрешает относительный `<D:href>` относительно `final_url` ответа (с учётом редиректов на шард iCloud).
pub fn resolve_href(final_url: &str, href: &str) -> String {
    http::resolve_redirect_url(final_url, href).unwrap_or_else(|| href.trim().to_string())
}

/// Проверяет, соответствует ли имя тега `tag_name` (например, `D:response` или `response`) локальному имени `local`.
fn matches_local_name(tag_name: &str, local: &str) -> bool {
    let clean = tag_name.rsplit(':').next().unwrap_or(tag_name);
    clean.eq_ignore_ascii_case(local)
}

/// Извлекает внутреннее содержимое всех парных тегов `<*:local ...>...</*:local>`.
pub fn extract_xml_blocks<'a>(xml: &'a str, local: &str) -> Vec<&'a str> {
    let mut results = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let Some(rel_lt) = xml[i..].find('<') else { break };
        let lt = i + rel_lt;
        if xml[lt..].starts_with("<![CDATA[") {
            if let Some(end_cdata) = xml[lt + 9..].find("]]>") {
                i = lt + 9 + end_cdata + 3;
                continue;
            }
            break;
        }
        if xml[lt..].starts_with("<!--") {
            if let Some(end_comm) = xml[lt + 4..].find("-->") {
                i = lt + 4 + end_comm + 3;
                continue;
            }
            break;
        }
        let Some(rel_gt) = xml[lt + 1..].find('>') else { break };
        let gt = lt + 1 + rel_gt;
        let inside = xml[lt + 1..gt].trim();
        if inside.starts_with('/') || inside.starts_with('?') || inside.starts_with('!') || inside.ends_with('/') {
            i = gt + 1;
            continue;
        }
        let tag_name = inside.split_whitespace().next().unwrap_or("");
        if matches_local_name(tag_name, local) {
            let content_start = gt + 1;
            let mut search = content_start;
            let mut found_end = None;
            while search < bytes.len() {
                let Some(r_lt) = xml[search..].find('<') else { break };
                let c_lt = search + r_lt;
                if xml[c_lt..].starts_with("<![CDATA[") {
                    if let Some(end_c) = xml[c_lt + 9..].find("]]>") {
                        search = c_lt + 9 + end_c + 3;
                        continue;
                    }
                    break;
                }
                let Some(r_gt) = xml[c_lt + 1..].find('>') else { break };
                let c_gt = c_lt + 1 + r_gt;
                let c_in = xml[c_lt + 1..c_gt].trim();
                if let Some(close_name) = c_in.strip_prefix('/') {
                    if matches_local_name(close_name.trim(), local) {
                        found_end = Some((c_lt, c_gt + 1));
                        break;
                    }
                }
                search = c_gt + 1;
            }
            if let Some((content_end, next_i)) = found_end {
                results.push(&xml[content_start..content_end]);
                i = next_i;
                continue;
            }
        }
        i = gt + 1;
    }

    results
}

/// Проверяет, встречается ли внутри `xml` открывающий или самозакрывающийся тег `<*:local ...>`.
pub fn has_xml_tag(xml: &str, local: &str) -> bool {
    let mut i = 0;
    while let Some(rel_lt) = xml[i..].find('<') {
        let lt = i + rel_lt;
        let Some(rel_gt) = xml[lt + 1..].find('>') else { break };
        let gt = lt + 1 + rel_gt;
        let inside = xml[lt + 1..gt].trim().trim_end_matches('/').trim();
        if !inside.starts_with('/') && !inside.starts_with('?') && !inside.starts_with('!') {
            let tag_name = inside.split_whitespace().next().unwrap_or("");
            if matches_local_name(tag_name, local) {
                return true;
            }
        }
        i = gt + 1;
    }
    false
}

/// Декодирует `CDATA` и стандартные XML-сущности (`&lt;`, `&gt;`, `&amp;`, `&quot;`, `&apos;`, `&#13;`, `&#10;`).
pub fn decode_xml_text(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(inner) = s.strip_prefix("<![CDATA[") {
        if let Some(clean) = inner.strip_suffix("]]>") {
            return clean.to_string();
        }
    }
    let mut out = String::with_capacity(s.len());
    while !s.is_empty() {
        if let Some( rest) = s.strip_prefix("<![CDATA[") {
            if let Some(end) = rest.find("]]>") {
                out.push_str(&rest[..end]);
                s = &rest[end + 3..];
                continue;
            }
        }
        if let Some(amp) = s.find('&') {
            out.push_str(&s[..amp]);
            let after = &s[amp + 1..];
            if let Some(semi) = after.find(';') {
                let ent = &after[..semi];
                match ent {
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "amp" => out.push('&'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    "#13" | "#xD" | "#xd" => out.push('\r'),
                    "#10" | "#xA" | "#xa" => out.push('\n'),
                    _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                        if let Some(cp) = u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32) {
                            out.push(cp);
                        }
                    }
                    _ if ent.starts_with('#') => {
                        if let Some(cp) = ent[1..].parse::<u32>().ok().and_then(char::from_u32) {
                            out.push(cp);
                        }
                    }
                    _ => {
                        out.push('&');
                        out.push_str(ent);
                        out.push(';');
                    }
                }
                s = &after[semi + 1..];
            } else {
                out.push('&');
                s = after;
            }
        } else {
            out.push_str(s);
            break;
        }
    }
    out
}

pub fn extract_first_tag_text(xml: &str, local: &str) -> Option<String> {
    extract_xml_blocks(xml, local)
        .into_iter()
        .next()
        .map(|b| decode_xml_text(b).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = ((bytes[i + 1] as char).to_digit(16), (bytes[i + 2] as char).to_digit(16)) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Извлекает email или идентификатор пользователя из Google CalDAV principal href вида `/caldav/v2/<email>/user`.
pub fn extract_principal_email(href: &str) -> Option<String> {
    let marker = "/caldav/v2/";
    let idx = href.find(marker)? + marker.len();
    let rest = &href[idx..];
    let raw_id = rest.split('/').next()?.trim();
    if raw_id.is_empty() {
        return None;
    }
    let decoded = percent_decode(raw_id);
    Some(decoded)
}

fn caldav_request(method: &str, url: &str, auth_header: &str, depth: &str, xml_body: &str) -> Result<(String, String), String> {
    let headers = [
        ("Authorization", auth_header),
        ("Depth", depth),
        ("Content-Type", "application/xml; charset=utf-8"),
    ];
    let resp = unsafe { http::request(method, url, &headers, Some(xml_body.as_bytes()), MAX_CALDAV_BYTES)? };
    if resp.status == 401 || resp.status == 403 {
        return Err(format!("HTTP {}", resp.status));
    }
    if resp.status >= 400 {
        return Err(format!("HTTP {} для {url}", resp.status));
    }
    Ok((resp.final_url, String::from_utf8_lossy(&resp.body).into_owned()))
}

/// Выполняет 3-шаговое CalDAV-обнаружение (`current-user-principal` → `calendar-home-set` → список коллекций `VEVENT`).
/// Возвращает `(discovered_email_if_any, vec_of_calendar_urls)`.
pub fn discover_calendars(base_url: &str, auth_header: &str) -> Result<(Option<String>, Vec<String>), String> {
    // Шаг 1: current-user-principal
    let body1 = r#"<D:propfind xmlns:D="DAV:"><D:prop><D:current-user-principal/></D:prop></D:propfind>"#;
    let (final1, xml1) = caldav_request("PROPFIND", base_url, auth_header, "0", body1)?;
    let principal_block = extract_xml_blocks(&xml1, "current-user-principal")
        .into_iter()
        .next()
        .ok_or_else(|| "CalDAV: не найден current-user-principal".to_string())?;
    let principal_href = extract_first_tag_text(principal_block, "href")
        .ok_or_else(|| "CalDAV: пустой href у current-user-principal".to_string())?;
    let principal_url = resolve_href(&final1, &principal_href);
    let discovered_email = extract_principal_email(&principal_href).or_else(|| extract_principal_email(&principal_url));

    // Шаг 2: calendar-home-set
    let body2 = r#"<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><C:calendar-home-set/></D:prop></D:propfind>"#;
    let (final2, xml2) = caldav_request("PROPFIND", &principal_url, auth_header, "0", body2)?;
    let home_block = extract_xml_blocks(&xml2, "calendar-home-set")
        .into_iter()
        .next()
        .ok_or_else(|| "CalDAV: не найден calendar-home-set".to_string())?;
    let home_href = extract_first_tag_text(home_block, "href")
        .ok_or_else(|| "CalDAV: пустой href у calendar-home-set".to_string())?;
    let home_url = resolve_href(&final2, &home_href);

    // Шаг 3: перечисление календарей (Depth: 1)
    let body3 = r#"<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><D:resourcetype/><D:displayname/><C:supported-calendar-component-set/></D:prop></D:propfind>"#;
    let (final3, xml3) = caldav_request("PROPFIND", &home_url, auth_header, "1", body3)?;

    let mut calendars = Vec::new();
    for resp_block in extract_xml_blocks(&xml3, "response") {
        let Some(href) = extract_first_tag_text(resp_block, "href") else { continue };
        let Some(rt_block) = extract_xml_blocks(resp_block, "resourcetype").into_iter().next() else { continue };
        if !has_xml_tag(rt_block, "calendar") {
            continue;
        }
        // Пропускаем служебные очереди iCloud (schedule-inbox, schedule-outbox, notification, dropbox)
        if has_xml_tag(rt_block, "schedule-inbox")
            || has_xml_tag(rt_block, "schedule-outbox")
            || has_xml_tag(rt_block, "notification")
            || has_xml_tag(rt_block, "dropbox-home")
        {
            continue;
        }
        let supports_vevent = match extract_xml_blocks(resp_block, "supported-calendar-component-set").into_iter().next() {
            Some(comp_set) if !comp_set.trim().is_empty() => comp_set.to_ascii_uppercase().contains("VEVENT"),
            _ => true,
        };
        if !supports_vevent {
            continue;
        }
        let cal_url = resolve_href(&final3, &href);
        if !calendars.contains(&cal_url) {
            calendars.push(cal_url);
        }
    }

    Ok((discovered_email, calendars))
}

/// Запрашивает события календаря `cal_url` за интервал `[start_utc, end_utc]` (в формате `YYYYMMDDTHHMMSSZ`)
/// с раскрытием повторений на стороне сервера (`<C:expand>`) и возвращает объединённый текст `VCALENDAR`.
pub fn fetch_calendar_events(
    cal_url: &str,
    auth_header: &str,
    start_utc: &str,
    end_utc: &str,
) -> Result<String, String> {
    let body = format!(
        r#"<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:prop><D:getetag/><C:calendar-data><C:expand start="{start_utc}" end="{end_utc}"/></C:calendar-data></D:prop><C:filter><C:comp-filter name="VCALENDAR"><C:comp-filter name="VEVENT"><C:time-range start="{start_utc}" end="{end_utc}"/></C:comp-filter></C:comp-filter></C:filter></C:calendar-query>"#
    );
    let (_, xml) = caldav_request("REPORT", cal_url, auth_header, "1", &body)?;
    let mut combined = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n");
    for data_block in extract_xml_blocks(&xml, "calendar-data") {
        let ics = decode_xml_text(data_block);
        for line in ics.lines() {
            let trimmed = line.trim_end_matches('\r');
            if trimmed.eq_ignore_ascii_case("BEGIN:VCALENDAR")
                || trimmed.eq_ignore_ascii_case("END:VCALENDAR")
                || trimmed.to_ascii_uppercase().starts_with("VERSION:")
                || trimmed.to_ascii_uppercase().starts_with("PRODID:")
                || trimmed.to_ascii_uppercase().starts_with("CALSCALE:")
            {
                continue;
            }
            if !trimmed.is_empty() {
                combined.push_str(trimmed);
                combined.push_str("\r\n");
            }
        }
    }
    combined.push_str("END:VCALENDAR\r\n");
    Ok(combined)
}

/// Формирует заголовок `Authorization: Basic ...` для Apple iCloud CalDAV.
pub fn basic_auth_header(user: &str, pass: &str) -> String {
    let credentials = format!("{user}:{pass}");
    format!("Basic {}", http::base64_encode(credentials.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn xml_parser_handles_namespaces_cdata_and_entities() {
        let xml = r#"
        <d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav">
          <d:response>
            <d:href>/caldav/v2/user%40gmail.com/events/</d:href>
            <d:propstat>
              <d:prop>
                <cal:calendar-data><![CDATA[BEGIN:VCALENDAR
BEGIN:VEVENT
SUMMARY:Встреча & Обсуждение
END:VEVENT
END:VCALENDAR]]></cal:calendar-data>
              </d:prop>
            </d:propstat>
          </d:response>
          <D:response>
            <D:propstat>
              <D:prop>
                <C:calendar-data xmlns:C="urn:ietf:params:xml:ns:caldav">BEGIN:VCALENDAR&#13;&#10;BEGIN:VEVENT&#13;&#10;SUMMARY:Тест &amp; &lt;Код&gt;&#13;&#10;END:VEVENT&#13;&#10;END:VCALENDAR</C:calendar-data>
              </D:prop>
            </D:propstat>
          </D:response>
        </d:multistatus>
        "#;

        let blocks = extract_xml_blocks(xml, "calendar-data");
        assert_eq!(blocks.len(), 2);
        let d0 = decode_xml_text(blocks[0]);
        assert!(d0.contains("SUMMARY:Встреча & Обсуждение"));
        let d1 = decode_xml_text(blocks[1]);
        assert!(d1.contains("SUMMARY:Тест & <Код>"));
        assert_eq!(
            extract_principal_email("/caldav/v2/user%40gmail.com/user").as_deref(),
            Some("user@gmail.com")
        );
    }

    #[test]
    fn propfind_and_report_with_307_redirect_on_mock_server() {
        let shard = TcpListener::bind("127.0.0.1:0").unwrap();
        let shard_port = shard.local_addr().unwrap().port();

        let entry = TcpListener::bind("127.0.0.1:0").unwrap();
        let entry_port = entry.local_addr().unwrap().port();

        let entry_thread = std::thread::spawn(move || {
            if let Ok((mut s, _)) = entry.accept() {
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.starts_with("PROPFIND"));
                assert!(req.contains("Authorization: Bearer tok_test"));
                let loc = format!("http://127.0.0.1:{shard_port}/caldav/v2/");
                let resp = format!(
                    "HTTP/1.1 307 Temporary Redirect\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });

        let shard_thread = std::thread::spawn(move || {
            // 1. Шаг 1 после редиректа: current-user-principal
            if let Ok((mut s, _)) = shard.accept() {
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.starts_with("PROPFIND /caldav/v2/"));
                assert!(req.contains("Authorization: Bearer tok_test"));
                let xml = r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:"><D:response><D:propstat><D:prop><D:current-user-principal><D:href>/caldav/v2/alice%40example.com/user</D:href></D:current-user-principal></D:prop></D:propstat></D:response></D:multistatus>"#;
                let resp = format!("HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", xml.len(), xml);
                let _ = s.write_all(resp.as_bytes());
            }
            // 2. Шаг 2: calendar-home-set
            if let Ok((mut s, _)) = shard.accept() {
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.starts_with("PROPFIND /caldav/v2/alice%40example.com/user"));
                let xml = r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:propstat><D:prop><C:calendar-home-set><D:href>/calendars/alice/</D:href></C:calendar-home-set></D:prop></D:propstat></D:response></D:multistatus>"#;
                let resp = format!("HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", xml.len(), xml);
                let _ = s.write_all(resp.as_bytes());
            }
            // 3. Шаг 3: список календарей (один VEVENT, один VTODO, один schedule-inbox)
            if let Ok((mut s, _)) = shard.accept() {
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.starts_with("PROPFIND /calendars/alice/"));
                let xml = r#"<?xml version="1.0" encoding="utf-8"?>
                <D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
                  <D:response>
                    <D:href>/calendars/alice/work/</D:href>
                    <D:propstat><D:prop>
                      <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
                      <C:supported-calendar-component-set><C:comp name="VEVENT"/></C:supported-calendar-component-set>
                    </D:prop></D:propstat>
                  </D:response>
                  <D:response>
                    <D:href>/calendars/alice/tasks/</D:href>
                    <D:propstat><D:prop>
                      <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
                      <C:supported-calendar-component-set><C:comp name="VTODO"/></C:supported-calendar-component-set>
                    </D:prop></D:propstat>
                  </D:response>
                  <D:response>
                    <D:href>/calendars/alice/inbox/</D:href>
                    <D:propstat><D:prop>
                      <D:resourcetype><D:collection/><C:calendar/><C:schedule-inbox/></D:resourcetype>
                    </D:prop></D:propstat>
                  </D:response>
                </D:multistatus>"#;
                let resp = format!("HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", xml.len(), xml);
                let _ = s.write_all(resp.as_bytes());
            }
            // 4. REPORT: запрос событий
            if let Ok((mut s, _)) = shard.accept() {
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.starts_with("REPORT /calendars/alice/work/"));
                let xml = r#"<?xml version="1.0" encoding="utf-8"?>
                <D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
                  <D:response>
                    <D:href>/calendars/alice/work/ev1.ics</D:href>
                    <D:propstat><D:prop>
                      <C:calendar-data>BEGIN:VCALENDAR&#13;&#10;VERSION:2.0&#13;&#10;BEGIN:VEVENT&#13;&#10;DTSTART:20261111T090000Z&#13;&#10;DTEND:20261111T100000Z&#13;&#10;SUMMARY:Планерка&#13;&#10;END:VEVENT&#13;&#10;END:VCALENDAR</C:calendar-data>
                    </D:prop></D:propstat>
                  </D:response>
                </D:multistatus>"#;
                let resp = format!("HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", xml.len(), xml);
                let _ = s.write_all(resp.as_bytes());
            }
        });

        let entry_url = format!("http://127.0.0.1:{entry_port}/caldav/v2/");
        let (email, cals) = discover_calendars(&entry_url, "Bearer tok_test").unwrap();
        assert_eq!(email.as_deref(), Some("alice@example.com"));
        assert_eq!(cals, vec![format!("http://127.0.0.1:{shard_port}/calendars/alice/work/")]);

        let ics = fetch_calendar_events(&cals[0], "Bearer tok_test", "20261101T000000Z", "20261201T000000Z").unwrap();
        assert!(ics.contains("SUMMARY:Планерка"));

        entry_thread.join().unwrap();
        shard_thread.join().unwrap();
    }
}
