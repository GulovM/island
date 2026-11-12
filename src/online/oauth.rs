//! Авторизация OAuth 2.0 с PKCE (S256) для настольных приложений (RFC 8252 / RFC 7636).
//! Используется для подключения Google Календаря через локальный loopback-сервер `127.0.0.1:0`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::http;
use crate::sys;

pub const GOOGLE_CLIENT_ID: Option<&str> = option_env!("ISLAND_GOOGLE_CLIENT_ID");
pub const GOOGLE_CLIENT_SECRET: Option<&str> = option_env!("ISLAND_GOOGLE_CLIENT_SECRET");

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// `calendar.readonly` читает список календарей и события, `calendar.events` разрешает остров
/// записывать события, созданные в нём (только по отметке в форме события).
const GOOGLE_CALENDAR_SCOPE: &str =
    "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events";

static REFRESH_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OAuthError {
    NotConfigured,
    Cancelled,
    StateMismatch,
    InvalidGrant,
    Http(String),
}

impl OAuthError {
    pub fn message(&self) -> String {
        match self {
            Self::NotConfigured => "Ключи Google OAuth не вшиты в эту сборку".into(),
            Self::Cancelled => "Авторизация отменена или истекло время ожидания".into(),
            Self::StateMismatch => "Ошибка безопасности OAuth (не совпал state)".into(),
            Self::InvalidGrant => "Доступ к Google Календарю отозван — переподключите аккаунт".into(),
            Self::Http(e) => e.clone(),
        }
    }
}

/// Доступна ли авторизация Google Календаря в этой сборке.
pub fn google_available() -> bool {
    match (GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET) {
        (Some(id), Some(sec)) => !id.trim().is_empty() && !sec.trim().is_empty(),
        _ => false,
    }
}

fn google_credentials() -> Result<(&'static str, &'static str), OAuthError> {
    match (GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET) {
        (Some(id), Some(sec)) if !id.trim().is_empty() && !sec.trim().is_empty() => Ok((id.trim(), sec.trim())),
        _ => Err(OAuthError::NotConfigured),
    }
}

fn random_bytes(buf: &mut [u8]) -> Result<(), OAuthError> {
    unsafe {
        BCryptGenRandom(None, buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG)
            .ok()
            .map_err(|e| OAuthError::Http(format!("BCryptGenRandom: {e}")))
    }
}

pub fn generate_code_verifier() -> Result<String, OAuthError> {
    let mut bytes = [0u8; 32];
    random_bytes(&mut bytes)?;
    Ok(http::base64url_encode(&bytes))
}

pub fn generate_state() -> Result<String, OAuthError> {
    let mut bytes = [0u8; 16];
    random_bytes(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Вычисляет `code_challenge = BASE64URL-ENCODE(SHA256(ASCII(code_verifier)))` по RFC 7636.
pub fn pkce_s256_challenge(verifier: &str) -> Option<String> {
    let digest = sys::sha256(verifier.as_bytes())?;
    Some(http::base64url_encode(&digest))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = ((bytes[i + 1] as char).to_digit(16), (bytes[i + 2] as char).to_digit(16)) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, name: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == name {
            return Some(percent_decode(v));
        }
    }
    None
}

fn send_html_response(stream: &mut std::net::TcpStream, title: &str, body_text: &str) {
    let html = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
        <style>body{{font-family:system-ui,-apple-system,sans-serif;background:#111;color:#eee;\
        display:flex;align-items:center;justify-content:center;height:100vh;margin:0}}\
        .card{{background:#1c1c1e;padding:28px 36px;border-radius:14px;text-align:center;\
        box-shadow:0 8px 32px rgba(0,0,0,0.4)}}h2{{margin:0 0 8px;font-size:18px}}p{{margin:0;color:#999;font-size:14px}}</style>\
        </head><body><div class=\"card\"><h2>{title}</h2><p>{body_text}</p></div></body></html>"
    );
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

/// Ожидает OAuth-редирект на `listener` до `timeout`. Игнорирует пустые сокеты и `/favicon.ico`.
pub fn wait_for_oauth_callback(
    listener: &TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, OAuthError> {
    let _ = listener.set_nonblocking(true);
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 4096];

    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => continue,
                    Ok(n) => n,
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let Some(first_line) = req.lines().next() else { continue };
                let mut parts = first_line.split_whitespace();
                let _method = parts.next().unwrap_or("");
                let target = parts.next().unwrap_or("");

                if target.starts_with("/favicon.ico") {
                    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    let _ = stream.flush();
                    continue;
                }

                if let Some(rest) = target.strip_prefix("/callback") {
                    let query = rest.strip_prefix('?').unwrap_or("");
                    if let Some(err) = query_param(query, "error") {
                        send_html_response(&mut stream, "Авторизация отклонена", &format!("Ошибка: {err}. Окно можно закрыть."));
                        return Err(OAuthError::Cancelled);
                    }
                    if let Some(code) = query_param(query, "code") {
                        let st = query_param(query, "state").unwrap_or_default();
                        if st != expected_state {
                            send_html_response(&mut stream, "Ошибка проверки state", "Попробуйте подключить календарь ещё раз.");
                            return Err(OAuthError::StateMismatch);
                        }
                        send_html_response(&mut stream, "Авторизация успешна", "Окно браузера можно закрыть и вернуться в Kite Island.");
                        return Ok(code);
                    }
                }

                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = stream.flush();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => return Err(OAuthError::Http(format!("ошибка локального сокета: {e}"))),
        }
    }

    Err(OAuthError::Cancelled)
}

fn parse_token_response(status: u16, body: &[u8]) -> Result<TokenResponse, OAuthError> {
    let text = String::from_utf8_lossy(body);
    if status >= 400 {
        if http::json_str(&text, "error").as_deref() == Some("invalid_grant") || text.contains("invalid_grant") {
            return Err(OAuthError::InvalidGrant);
        }
        let desc = http::json_str(&text, "error_description")
            .or_else(|| http::json_str(&text, "error"))
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(OAuthError::Http(desc));
    }
    let access_token = http::json_str(&text, "access_token")
        .ok_or_else(|| OAuthError::Http("в ответе OAuth нет access_token".into()))?;
    let refresh_token = http::json_str(&text, "refresh_token");
    let expires_in = http::json_num(&text, "expires_in").map(|n| n.max(60.0) as u64).unwrap_or(3600);
    Ok(TokenResponse {
        access_token,
        refresh_token,
        expires_in,
    })
}

pub fn exchange_code_with_url(
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, OAuthError> {
    let form = format!(
        "client_id={}&client_secret={}&code={}&code_verifier={}&grant_type=authorization_code&redirect_uri={}",
        http::urlenc(client_id),
        http::urlenc(client_secret),
        http::urlenc(code),
        http::urlenc(verifier),
        http::urlenc(redirect_uri),
    );
    let headers = [("Content-Type", "application/x-www-form-urlencoded")];
    let resp = unsafe {
        http::request("POST", token_url, &headers, Some(form.as_bytes()), 64 * 1024)
            .map_err(OAuthError::Http)?
    };
    parse_token_response(resp.status, &resp.body)
}

pub fn refresh_token_with_url(
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenResponse, OAuthError> {
    let _guard = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let form = format!(
        "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
        http::urlenc(client_id),
        http::urlenc(client_secret),
        http::urlenc(refresh_token),
    );
    let headers = [("Content-Type", "application/x-www-form-urlencoded")];
    let resp = unsafe {
        http::request("POST", token_url, &headers, Some(form.as_bytes()), 64 * 1024)
            .map_err(OAuthError::Http)?
    };
    parse_token_response(resp.status, &resp.body)
}

/// Пишет причину сбоя OAuth в `island.log`: в интерфейсе видна только последняя ошибка, и
/// без журнала не понять, на каком шаге оборвался вход. Токены и секреты в сообщения не попадают.
fn log_oauth_error(stage: &str, e: &OAuthError) {
    crate::log::error(&format!("Google OAuth ({stage}): {}", e.message()));
}

/// Запускает полный цикл авторизации Google OAuth 2.0 PKCE через системный браузер.
pub fn authorize_google() -> Result<TokenResponse, OAuthError> {
    authorize_google_inner().inspect_err(|e| log_oauth_error("вход", e))
}

fn authorize_google_inner() -> Result<TokenResponse, OAuthError> {
    let (client_id, client_secret) = google_credentials()?;
    let verifier = generate_code_verifier()?;
    let challenge = pkce_s256_challenge(&verifier).ok_or_else(|| OAuthError::Http("SHA-256 недоступен".into()))?;
    let state = generate_state()?;

    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| OAuthError::Http(format!("не удалось открыть локальный порт: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| OAuthError::Http(format!("local_addr: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let auth_url = format!(
        "{GOOGLE_AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        http::urlenc(client_id),
        http::urlenc(&redirect_uri),
        http::urlenc(GOOGLE_CALENDAR_SCOPE),
        http::urlenc(&challenge),
        http::urlenc(&state),
    );

    let urlw = sys::wz(&auth_url);
    let h = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(urlw.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if (h.0 as isize) <= 32 {
        return Err(OAuthError::Http("не удалось открыть браузер".into()));
    }

    let code = wait_for_oauth_callback(&listener, &state, Duration::from_secs(120))?;
    exchange_code_with_url(GOOGLE_TOKEN_URL, client_id, client_secret, &code, &verifier, &redirect_uri)
}

/// Обновляет `access_token` Google по сохранённому `refresh_token`.
pub fn refresh_google_token(refresh_token: &str) -> Result<TokenResponse, OAuthError> {
    let (client_id, client_secret) = google_credentials().inspect_err(|e| log_oauth_error("обновление токена", e))?;
    refresh_token_with_url(GOOGLE_TOKEN_URL, client_id, client_secret, refresh_token)
        .inspect_err(|e| log_oauth_error("обновление токена", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    #[test]
    fn pkce_s256_rfc7636_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_s256_challenge(verifier).as_deref(),
            Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        );
        let gen = generate_code_verifier().unwrap();
        assert_eq!(gen.len(), 43);
        let st = generate_state().unwrap();
        assert_eq!(st.len(), 32);
    }

    #[test]
    fn loopback_listener_handles_favicon_and_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            // 1. Пустое preflight-соединение браузера
            let _ = TcpStream::connect(addr).unwrap();
            // 2. Запрос /favicon.ico
            if let Ok(mut s) = TcpStream::connect(addr) {
                let _ = s.write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
                let mut r = [0u8; 128];
                let _ = s.read(&mut r);
            }
            // 3. Основной /callback с code и state
            if let Ok(mut s) = TcpStream::connect(addr) {
                let _ = s.write_all(b"GET /callback?code=auth_code_42&state=sec_state_1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
                let mut r = Vec::new();
                let _ = s.read_to_end(&mut r);
                assert!(String::from_utf8_lossy(&r).contains("200 OK"));
            }
        });

        let code = wait_for_oauth_callback(&listener, "sec_state_1", Duration::from_secs(5)).unwrap();
        assert_eq!(code, "auth_code_42");
        handle.join().unwrap();
    }

    fn read_full_http_request(s: &mut TcpStream) -> String {
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let mut data = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match s.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    data.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&data);
                    if let Some(hdr_end) = text.find("\r\n\r\n") {
                        let body_start = hdr_end + 4;
                        let clen = text[..hdr_end]
                            .lines()
                            .find_map(|l| {
                                let (k, v) = l.split_once(':')?;
                                k.trim().eq_ignore_ascii_case("Content-Length").then(|| v.trim().parse::<usize>().ok())?
                            })
                            .unwrap_or(0);
                        if data.len() >= body_start + clen {
                            break;
                        }
                    }
                }
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    #[test]
    fn token_exchange_and_invalid_grant_on_mock_server() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}/token", addr.port());

        let handle = std::thread::spawn(move || {
            // Первый запрос: успешный обмен кода на токены
            if let Ok((mut s, _)) = server.accept() {
                let req = read_full_http_request(&mut s);
                assert!(req.contains("grant_type=authorization_code"));
                assert!(req.contains("code_verifier=ver123"));
                let body = r#"{"access_token":"ya29.test_tok","refresh_token":"1//ref_tok","expires_in":3599}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.flush();
            }
            // Второй запрос: invalid_grant при обновлении токена
            if let Ok((mut s, _)) = server.accept() {
                let _ = read_full_http_request(&mut s);
                let body = r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#;
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.flush();
            }
        });

        let tok = exchange_code_with_url(&url, "cid", "csec", "code1", "ver123", "http://127.0.0.1:9999/callback").unwrap();
        assert_eq!(tok.access_token, "ya29.test_tok");
        assert_eq!(tok.refresh_token.as_deref(), Some("1//ref_tok"));
        assert_eq!(tok.expires_in, 3599);

        let err = refresh_token_with_url(&url, "cid", "csec", "bad_ref").unwrap_err();
        assert_eq!(err, OAuthError::InvalidGrant);

        handle.join().unwrap();
    }
}
