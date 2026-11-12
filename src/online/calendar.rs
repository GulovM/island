use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{bg, log};

use super::caldav::{self, APPLE_CALDAV_BASE};
use super::datetime::{civil_from_days, days_from_civil, offset_at, weekday, SYSTEM_TZ};
use super::http::{self, get_text};
use super::oauth::{self, OAuthError};
use super::{spawn, stale, with, RETRY_AFTER};

const CALENDAR_TTL: Duration = Duration::from_secs(5 * 60);
const CALENDAR_CACHE_PREFIX: &str = "calendar_cache_";
const LEGACY_CALENDAR_CACHE_FILE: &str = "calendar_cache.ics";
const ACCOUNTS_FILE: &str = "cal_accounts.bin";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoogleAccount {
    pub email: String,
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppleAccount {
    pub apple_id: String,
    pub app_password: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalendarAccounts {
    pub google: Option<GoogleAccount>,
    pub apple: Option<AppleAccount>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalAccountStatus {
    pub google_email: Option<String>,
    pub google_busy: bool,
    pub apple_id: Option<String>,
    pub apple_busy: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct AuthUiState {
    google_busy: bool,
    apple_busy: bool,
    error: Option<String>,
}

static ACCOUNTS_STORE: Mutex<Option<CalendarAccounts>> = Mutex::new(None);
static AUTH_UI: Mutex<AuthUiState> = Mutex::new(AuthUiState {
    google_busy: false,
    apple_busy: false,
    error: None,
});

fn serialize_accounts(acc: &CalendarAccounts) -> String {
    let mut out = String::from("# Island CalDAV accounts\n");
    if let Some(g) = &acc.google {
        out.push_str("[google]\n");
        out.push_str(&format!("email={}\n", g.email.replace('\n', "")));
        out.push_str(&format!("refresh_token={}\n", g.refresh_token.replace('\n', "")));
        out.push_str(&format!("access_token={}\n", g.access_token.replace('\n', "")));
        out.push_str(&format!("expires_at={}\n", g.expires_at));
    }
    if let Some(a) = &acc.apple {
        out.push_str("[apple]\n");
        out.push_str(&format!("apple_id={}\n", a.apple_id.replace('\n', "")));
        out.push_str(&format!("app_password={}\n", a.app_password.replace('\n', "")));
    }
    out
}

fn parse_accounts(text: &str) -> CalendarAccounts {
    let mut acc = CalendarAccounts::default();
    let mut section = "";
    let mut g = GoogleAccount::default();
    let mut has_g = false;
    let mut a = AppleAccount::default();
    let mut has_a = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(sec) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = sec.trim();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match (section, k) {
            ("google", "email") => {
                g.email = v.to_string();
                has_g = true;
            }
            ("google", "refresh_token") => {
                g.refresh_token = v.to_string();
                has_g = true;
            }
            ("google", "access_token") => {
                g.access_token = v.to_string();
                has_g = true;
            }
            ("google", "expires_at") => {
                g.expires_at = v.parse().unwrap_or(0);
            }
            ("apple", "apple_id") => {
                a.apple_id = v.to_string();
                has_a = true;
            }
            ("apple", "app_password") => {
                a.app_password = v.to_string();
                has_a = true;
            }
            _ => {}
        }
    }
    if has_g && (!g.refresh_token.is_empty() || !g.access_token.is_empty()) {
        if g.email.is_empty() {
            g.email = "Google".to_string();
        }
        acc.google = Some(g);
    }
    if has_a && !a.apple_id.is_empty() && !a.app_password.is_empty() {
        acc.apple = Some(a);
    }
    acc
}

pub fn pack_accounts(acc: &CalendarAccounts) -> Option<Vec<u8>> {
    crate::sys::seal(serialize_accounts(acc).as_bytes())
}

pub fn unpack_accounts(bytes: &[u8]) -> Option<CalendarAccounts> {
    if bytes.is_empty() || !crate::sys::is_sealed(bytes) {
        return None;
    }
    let plain = crate::sys::unseal(bytes)?;
    let text = String::from_utf8_lossy(&plain);
    if !text.starts_with("# Island CalDAV accounts\n") {
        return None;
    }
    Some(parse_accounts(&text))
}

fn read_accounts_from_disk() -> CalendarAccounts {
    let path = crate::sys::data_dir().join(ACCOUNTS_FILE);
    let Ok(bytes) = std::fs::read(&path) else {
        return CalendarAccounts::default();
    };
    if bytes.is_empty() {
        return CalendarAccounts::default();
    }
    match unpack_accounts(&bytes) {
        Some(acc) => acc,
        None => {
            log::error("не удалось расшифровать cal_accounts.bin (DPAPI), аккаунты календаря сброшены");
            CalendarAccounts::default()
        }
    }
}

pub fn load_accounts() -> CalendarAccounts {
    let mut guard = ACCOUNTS_STORE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(acc) = guard.as_ref() {
        return acc.clone();
    }
    let acc = read_accounts_from_disk();
    *guard = Some(acc.clone());
    acc
}

fn save_accounts(acc: &CalendarAccounts) {
    if let Ok(mut guard) = ACCOUNTS_STORE.lock() {
        *guard = Some(acc.clone());
    }
    if acc.google.is_none() && acc.apple.is_none() {
        let _ = std::fs::remove_file(crate::sys::data_dir().join(ACCOUNTS_FILE));
        return;
    }
    if let Some(sealed) = pack_accounts(acc) {
        crate::tabs::write_bytes(ACCOUNTS_FILE, &sealed);
    } else {
        log::error("DPAPI недоступен, не удалось сохранить cal_accounts.bin");
    }
}

/// Возвращает список активных ключей источников календаря (ICS-ссылки + подключённые аккаунты CalDAV).
pub fn active_calendar_keys_for(ics_urls: &[String], acc: &CalendarAccounts) -> Vec<String> {
    let mut keys: Vec<String> = ics_urls
        .iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .collect();
    if let Some(g) = &acc.google {
        if !g.refresh_token.is_empty() || !g.access_token.is_empty() {
            keys.push(format!("caldav:google:{}", g.email));
        }
    }
    if let Some(a) = &acc.apple {
        if !a.apple_id.is_empty() && !a.app_password.is_empty() {
            keys.push(format!("caldav:apple:{}", a.apple_id));
        }
    }
    keys
}

/// Возвращает список активных ключей календаря с учётом сохранённых аккаунтов Google/Apple.
pub fn active_calendar_keys(ics_urls: &[String]) -> Vec<String> {
    #[cfg(test)]
    {
        active_calendar_keys_for(ics_urls, &CalendarAccounts::default())
    }
    #[cfg(not(test))]
    {
        active_calendar_keys_for(ics_urls, &load_accounts())
    }
}

pub fn cal_account_status() -> CalAccountStatus {
    let acc = load_accounts();
    let ui = AUTH_UI.lock().unwrap_or_else(|e| e.into_inner());
    CalAccountStatus {
        google_email: acc.google.map(|g| g.email),
        google_busy: ui.google_busy,
        apple_id: acc.apple.map(|a| a.apple_id),
        apple_busy: ui.apple_busy,
        error: ui.error.clone(),
    }
}

pub fn clear_cal_auth_error() {
    if let Ok(mut ui) = AUTH_UI.lock() {
        ui.error = None;
    }
}

fn set_cal_auth_error(msg: impl Into<String>) {
    if let Ok(mut ui) = AUTH_UI.lock() {
        ui.error = Some(msg.into());
    }
    bg::notify();
}

/// Запускает подключение Google Календаря в фоновом потоке.
pub fn connect_google(ics_urls: Vec<String>) {
    {
        let mut ui = AUTH_UI.lock().unwrap_or_else(|e| e.into_inner());
        if ui.google_busy {
            return;
        }
        ui.google_busy = true;
        ui.error = None;
    }
    bg::notify();
    spawn("island-google-oauth", move || {
        let res = (|| -> Result<GoogleAccount, String> {
            let tok = oauth::authorize_google().map_err(|e| e.message())?;
            // Заодно проверяет, что токен открывает Calendar API: адрес основного календаря — email.
            // Ошибки OAuth уже записаны в журнал в `oauth.rs`; здесь — только ошибки Calendar API.
            let (email_opt, _) = fetch_google_calendar_list(&tok.access_token)
                .inspect_err(|e| log::error(&format!("подключение Google Календаря: {e}")))?;
            let email = email_opt.unwrap_or_else(|| "Google".to_string());
            let expires_at = crate::sys::now_secs() + tok.expires_in.saturating_sub(60);
            Ok(GoogleAccount {
                email,
                refresh_token: tok.refresh_token.unwrap_or_default(),
                access_token: tok.access_token,
                expires_at,
            })
        })();

        match res {
            Ok(g_acc) => {
                let mut acc = load_accounts();
                acc.google = Some(g_acc);
                save_accounts(&acc);
                if let Ok(mut ui) = AUTH_UI.lock() {
                    ui.google_busy = false;
                    ui.error = None;
                }
                with(|s| s.cal_at = None);
                bg::notify();
                refresh_calendar_age(&ics_urls, Duration::ZERO);
            }
            Err(err) => {
                if let Ok(mut ui) = AUTH_UI.lock() {
                    ui.google_busy = false;
                    ui.error = Some(err);
                }
                bg::notify();
            }
        }
    });
}

/// Отключает аккаунт Google Календаря и очищает его зашифрованный кэш.
pub fn disconnect_google(ics_urls: &[String]) {
    let mut acc = load_accounts();
    acc.google = None;
    save_accounts(&acc);
    clear_cal_auth_error();
    let keys = active_calendar_keys_for(ics_urls, &acc);
    clean_orphan_caches(&keys);
    with(|s| {
        s.cal_at = None;
        s.cal_key.clear();
        if keys.is_empty() {
            s.snap.cal = Arc::default();
            s.snap.cal_ver += 1;
            s.snap.calendar_set = false;
            s.snap.calendar_failed = false;
        }
    });
    bg::notify();
    if !keys.is_empty() {
        refresh_calendar_age(ics_urls, Duration::ZERO);
    }
}

/// Проверяет и подключает аккаунт Apple iCloud Календаря по Apple ID и паролю приложения.
pub fn connect_apple(apple_id: String, app_password: String, ics_urls: Vec<String>) {
    let apple_id = apple_id.trim().to_string();
    let app_password = app_password.trim().replace('-', "");
    if apple_id.is_empty() || app_password.is_empty() {
        return;
    }
    {
        let mut ui = AUTH_UI.lock().unwrap_or_else(|e| e.into_inner());
        if ui.apple_busy {
            return;
        }
        ui.apple_busy = true;
        ui.error = None;
    }
    bg::notify();
    spawn("island-apple-caldav", move || {
        let auth = caldav::basic_auth_header(&apple_id, &app_password);
        match caldav::discover_calendars(APPLE_CALDAV_BASE, &auth) {
            Ok((_, cals)) if !cals.is_empty() => {
                let mut acc = load_accounts();
                acc.apple = Some(AppleAccount { apple_id, app_password });
                save_accounts(&acc);
                if let Ok(mut ui) = AUTH_UI.lock() {
                    ui.apple_busy = false;
                    ui.error = None;
                }
                with(|s| s.cal_at = None);
                bg::notify();
                refresh_calendar_age(&ics_urls, Duration::ZERO);
            }
            Ok(_) => {
                log::error("подключение Apple Календаря: в аккаунте iCloud нет календарей");
                if let Ok(mut ui) = AUTH_UI.lock() {
                    ui.apple_busy = false;
                    ui.error = Some("В аккаунте iCloud не найдено календарей".into());
                }
                bg::notify();
            }
            Err(e) => {
                log::error(&format!("подключение Apple Календаря: {e}"));
                let msg = if e.contains("401") || e.contains("403") {
                    "Неверный Apple ID или пароль приложения iCloud".to_string()
                } else {
                    format!("Ошибка iCloud CalDAV: {e}")
                };
                if let Ok(mut ui) = AUTH_UI.lock() {
                    ui.apple_busy = false;
                    ui.error = Some(msg);
                }
                bg::notify();
            }
        }
    });
}

/// Отключает аккаунт Apple iCloud Календаря и очищает его кэш.
pub fn disconnect_apple(ics_urls: &[String]) {
    let mut acc = load_accounts();
    acc.apple = None;
    save_accounts(&acc);
    clear_cal_auth_error();
    let keys = active_calendar_keys_for(ics_urls, &acc);
    clean_orphan_caches(&keys);
    with(|s| {
        s.cal_at = None;
        s.cal_key.clear();
        if keys.is_empty() {
            s.snap.cal = Arc::default();
            s.snap.cal_ver += 1;
            s.snap.calendar_set = false;
            s.snap.calendar_failed = false;
        }
    });
    bg::notify();
    if !keys.is_empty() {
        refresh_calendar_age(ics_urls, Duration::ZERO);
    }
}

fn caldav_time_window() -> (String, String) {
    let now_days = (crate::sys::now_secs() / 86_400) as i64;
    let (y1, m1, d1) = civil_from_days(now_days - 7);
    let (y2, m2, d2) = civil_from_days(now_days + 60);
    (
        format!("{y1:04}{m1:02}{d1:02}T000000Z"),
        format!("{y2:04}{m2:02}{d2:02}T235959Z"),
    )
}

fn fetch_all_caldav_events(cals: &[String], auth_header: &str) -> Result<String, String> {
    let (start_utc, end_utc) = caldav_time_window();
    let mut combined = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n");
    let mut any_ok = false;
    let mut last_err = String::new();
    for cal_url in cals {
        match caldav::fetch_calendar_events(cal_url, auth_header, &start_utc, &end_utc) {
            Ok(ics) => {
                any_ok = true;
                for line in ics.lines() {
                    let t = line.trim_end_matches('\r');
                    if !t.eq_ignore_ascii_case("BEGIN:VCALENDAR")
                        && !t.eq_ignore_ascii_case("END:VCALENDAR")
                        && !t.to_ascii_uppercase().starts_with("VERSION:")
                    {
                        combined.push_str(t);
                        combined.push_str("\r\n");
                    }
                }
            }
            Err(e) => last_err = e,
        }
    }
    if !any_ok && !cals.is_empty() {
        return Err(last_err);
    }
    combined.push_str("END:VCALENDAR\r\n");
    Ok(combined)
}

fn refresh_google_account_token(g: &mut GoogleAccount) -> Result<(), String> {
    if g.refresh_token.is_empty() {
        return Err("нет refresh_token для Google Календаря".into());
    }
    match oauth::refresh_google_token(&g.refresh_token) {
        Ok(tok) => {
            g.access_token = tok.access_token;
            if let Some(rt) = tok.refresh_token {
                if !rt.is_empty() {
                    g.refresh_token = rt;
                }
            }
            g.expires_at = crate::sys::now_secs() + tok.expires_in.saturating_sub(60);
            let mut acc = load_accounts();
            acc.google = Some(g.clone());
            save_accounts(&acc);
            Ok(())
        }
        Err(OAuthError::InvalidGrant) => {
            let msg = OAuthError::InvalidGrant.message();
            set_cal_auth_error(msg.clone());
            Err(msg)
        }
        Err(e) => Err(e.message()),
    }
}

// Google Calendar REST API v3. CalDAV у Google устаревший и требует отдельного включения в
// консоли проекта, а JSON-интерфейс работает напрямую с токеном области `calendar.readonly`.
// События переводятся в компактный VCALENDAR, чтобы кэш и разбор остались общими с iCal и iCloud.

const GOOGLE_API: &str = "https://www.googleapis.com/calendar/v3";
/// Предел ответа: с `fields` событие занимает ~200 байт, 2500 событий — около 0,5 МБ.
const MAX_GOOGLE_BYTES: usize = 4 << 20;

/// GET к Google Calendar API. Код ответа попадает в текст ошибки: по «401» вызывающий
/// обновляет токен и повторяет запрос.
fn google_get(url: &str, token: &str) -> Result<String, String> {
    let auth = format!("Bearer {token}");
    let resp = unsafe { http::request("GET", url, &[("Authorization", &auth)], None, MAX_GOOGLE_BYTES)? };
    let body = String::from_utf8_lossy(&resp.body).into_owned();
    if resp.status >= 400 {
        // Google объясняет отказ полем `message` (например, Calendar API не включён в проекте).
        let reason: String = http::json_str(&body, "message").unwrap_or_default().chars().take(160).collect();
        return Err(format!("Google Calendar API: HTTP {} {reason}", resp.status).trim_end().to_string());
    }
    if resp.total as usize > MAX_GOOGLE_BYTES {
        return Err(format!("ответ Google Calendar API больше {MAX_GOOGLE_BYTES} байт"));
    }
    Ok(body)
}

/// Календари аккаунта, показываемые в интерфейсе Google: `(id основного календаря, все id)`.
/// Id основного календаря совпадает с адресом почты аккаунта.
fn fetch_google_calendar_list(token: &str) -> Result<(Option<String>, Vec<String>), String> {
    let url = format!(
        "{GOOGLE_API}/users/me/calendarList?minAccessRole=reader&fields={}",
        http::urlenc("items(id,primary,selected,deleted)")
    );
    Ok(parse_google_calendar_list(&google_get(&url, token)?))
}

fn parse_google_calendar_list(json: &str) -> (Option<String>, Vec<String>) {
    let mut primary = None;
    let mut ids = Vec::new();
    for item in http::json_array(json, "items") {
        let Some(id) = http::json_field_str(item, "id") else { continue };
        let is_primary = http::json_field(item, "primary") == Some("true");
        // У Google `selected` по умолчанию false: календарь, скрытый в интерфейсе, приходит без поля.
        let shown = is_primary || http::json_field(item, "selected") == Some("true");
        if http::json_field(item, "deleted") == Some("true") || !shown {
            continue;
        }
        if is_primary {
            primary = Some(id.clone());
        }
        ids.push(id);
    }
    let email = primary.or_else(|| ids.first().cloned());
    (email, ids)
}

/// Окно событий как у CalDAV-источников (неделя назад, два месяца вперёд), в RFC 3339.
fn google_time_window() -> (String, String) {
    let now_days = (crate::sys::now_secs() / 86_400) as i64;
    let (y1, m1, d1) = civil_from_days(now_days - 7);
    let (y2, m2, d2) = civil_from_days(now_days + 60);
    (format!("{y1:04}-{m1:02}-{d1:02}T00:00:00Z"), format!("{y2:04}-{m2:02}-{d2:02}T23:59:59Z"))
}

/// `2026-11-12T10:00:00[.sss](Z|±HH:MM)` → `20261112T070000Z` (UTC, как понимает `parse_dt`).
fn rfc3339_to_ics_utc(s: &str) -> Option<String> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let off_min = match rest.as_bytes() {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let v = rest.get(1..3)?.parse::<i64>().ok()? * 60 + rest.get(4..6)?.parse::<i64>().ok()?;
            if *sign == b'-' {
                -v
            } else {
                v
            }
        }
        _ => return None,
    };
    let t = days_from_civil(y as i32, mo as u32, d as u32) * 86_400 + h * 3600 + mi * 60 + se.min(59) - off_min * 60;
    let (yy, mm, dd) = civil_from_days(t.div_euclid(86_400));
    let r = t.rem_euclid(86_400);
    Some(format!("{yy:04}{mm:02}{dd:02}T{:02}{:02}{:02}Z", r / 3600, r % 3600 / 60, r % 60))
}

/// Начало или конец события Google: `(значение для DTSTART/DTEND, весь день)`.
fn google_event_time(obj: &str) -> Option<(String, bool)> {
    if let Some(date) = http::json_field_str(obj, "date") {
        let compact: String = date.chars().filter(|c| *c != '-').collect();
        return (compact.len() == 8 && compact.bytes().all(|c| c.is_ascii_digit())).then_some((compact, true));
    }
    rfc3339_to_ics_utc(&http::json_field_str(obj, "dateTime")?).map(|t| (t, false))
}

/// Текст для свойства iCal: экранирование по RFC 5545 и без управляющих символов, чтобы строка
/// из чужого ответа не могла начать новое свойство.
fn ics_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Добавляет события из ответа `events.list` блоками `VEVENT`; отменённые и без начала пропускает.
fn google_events_to_ics(json: &str, out: &mut String) {
    for item in http::json_array(json, "items") {
        if http::json_field_str(item, "status").as_deref() == Some("cancelled") {
            continue;
        }
        let Some((start, all_day)) = http::json_obj(item, "start").and_then(google_event_time) else { continue };
        let end = http::json_obj(item, "end").and_then(google_event_time);
        let prop = |name: &str, v: &str, date: bool| format!("{name}{}:{v}\r\n", if date { ";VALUE=DATE" } else { "" });
        out.push_str("BEGIN:VEVENT\r\n");
        if let Some(id) = http::json_field_str(item, "id") {
            out.push_str(&format!("UID:{}\r\n", ics_text(&id)));
        }
        out.push_str(&format!("SUMMARY:{}\r\n", ics_text(&http::json_field_str(item, "summary").unwrap_or_default())));
        out.push_str(&prop("DTSTART", &start, all_day));
        if let Some((end, end_all_day)) = end {
            out.push_str(&prop("DTEND", &end, end_all_day));
        }
        out.push_str("END:VEVENT\r\n");
    }
}

fn sync_google_rest(mut g: GoogleAccount) -> Result<String, String> {
    let now = crate::sys::now_secs();
    if g.access_token.is_empty() || now + 60 >= g.expires_at {
        refresh_google_account_token(&mut g)?;
    }
    let cals = match fetch_google_calendar_list(&g.access_token) {
        Ok((_, cals)) => cals,
        Err(e) if e.contains("HTTP 401") && !g.refresh_token.is_empty() => {
            refresh_google_account_token(&mut g)?;
            fetch_google_calendar_list(&g.access_token)?.1
        }
        Err(e) => return Err(e),
    };
    let (time_min, time_max) = google_time_window();
    let mut combined = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n");
    let mut any_ok = false;
    let mut last_err = String::new();
    for cal in &cals {
        // `orderBy=startTime` с окном от недели назад: при пределе в 2500 событий отрезается
        // дальний конец окна, а не ближайшие дни.
        let url = format!(
            "{GOOGLE_API}/calendars/{}/events?singleEvents=true&orderBy=startTime&maxResults=2500&timeMin={}&timeMax={}&fields={}",
            http::urlenc(cal),
            http::urlenc(&time_min),
            http::urlenc(&time_max),
            http::urlenc("items(id,status,summary,start,end)"),
        );
        match google_get(&url, &g.access_token) {
            Ok(json) => {
                any_ok = true;
                google_events_to_ics(&json, &mut combined);
            }
            Err(e) => last_err = e,
        }
    }
    if !any_ok && !cals.is_empty() {
        return Err(last_err);
    }
    combined.push_str("END:VCALENDAR\r\n");
    Ok(combined)
}

fn sync_apple_caldav(a: AppleAccount) -> Result<String, String> {
    let auth = caldav::basic_auth_header(&a.apple_id, &a.app_password);
    let (_, cals) = match caldav::discover_calendars(APPLE_CALDAV_BASE, &auth) {
        Ok(res) => res,
        Err(e) => {
            if e.contains("401") || e.contains("403") {
                set_cal_auth_error("Неверный Apple ID или пароль приложения iCloud");
            }
            return Err(e);
        }
    };
    fetch_all_caldav_events(&cals, &auth)
}

fn hash_url(url: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    url.trim().hash(&mut h);
    format!("{:016x}", h.finish())
}

fn cache_filename(url: &str) -> String {
    format!("{CALENDAR_CACHE_PREFIX}{}.ics", hash_url(url))
}

/// Упаковывает кэш календаря: заголовок с хэшем ссылки и шифрование через DPAPI (`sys::seal`).
fn pack_cached_calendar(url: &str, ics: &str) -> Option<Vec<u8>> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let header = format!("URL_HASH:{}\n", hash_url(url));
    let mut payload = Vec::with_capacity(header.len() + ics.len());
    payload.extend_from_slice(header.as_bytes());
    payload.extend_from_slice(ics.as_bytes());
    crate::sys::seal(&payload)
}

/// Расшифровывает кэш календаря и проверяет соответствие хэша ссылки.
/// Возвращает `None` при битом кэше, несовпадении ссылки или пустом URL (без паник).
fn unpack_cached_calendar(url: &str, bytes: &[u8]) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || bytes.is_empty() {
        return None;
    }
    let unsealed = crate::sys::unseal(bytes)?;
    let text = String::from_utf8_lossy(&unsealed);
    let (header, ics) = text.split_once('\n')?;
    let hash = header.trim().strip_prefix("URL_HASH:")?;
    if hash != hash_url(url) {
        return None;
    }
    Some(ics.to_string())
}

fn save_cached_calendar(url: &str, ics: &str) {
    let Some(sealed) = pack_cached_calendar(url, ics) else {
        log::error("DPAPI недоступен или пустая ссылка, кэш календаря не сохранён");
        return;
    };
    let file = cache_filename(url);
    crate::tabs::write_bytes(&file, &sealed);
}

fn load_cached_calendar(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let data_dir = crate::sys::data_dir();
    let path = data_dir.join(cache_filename(url));
    if let Ok(bytes) = std::fs::read(&path) {
        if let Some(ics) = unpack_cached_calendar(url, &bytes) {
            return Some(ics);
        }
    }
    // Обратная совместимость с единым файлом calendar_cache.ics из T8
    let legacy_path = data_dir.join(LEGACY_CALENDAR_CACHE_FILE);
    if let Ok(bytes) = std::fs::read(&legacy_path) {
        if let Some(ics) = unpack_cached_calendar(url, &bytes) {
            save_cached_calendar(url, &ics);
            return Some(ics);
        }
    }
    None
}

/// Удаляет файлы кэша, хэши которых не соответствуют ни одной из настроенных ссылок и аккаунтов CalDAV.
pub fn clean_orphan_caches(urls: &[String]) {
    let active = active_calendar_keys(urls);
    let valid_hashes: std::collections::HashSet<String> = active.iter().map(|u| hash_url(u)).collect();
    let data_dir = crate::sys::data_dir();
    let Ok(entries) = std::fs::read_dir(&data_dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(hash) = name_str.strip_prefix(CALENDAR_CACHE_PREFIX).and_then(|s| s.strip_suffix(".ics")) {
            if !valid_hashes.contains(hash) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    if active.is_empty() {
        let _ = std::fs::remove_file(data_dir.join(LEGACY_CALENDAR_CACHE_FILE));
    }
}

/// Инициализирует снимок календаря сохранённым зашифрованным кэшем с диска (до первого сетевого ответа).
/// Загружает кэш только для тех ссылок и аккаунтов CalDAV, которые сейчас настроены; кэши удалённых удаляются.
pub fn init_cached_calendar(urls: &[String]) {
    let active_keys = active_calendar_keys(urls);
    clean_orphan_caches(urls);
    if active_keys.is_empty() {
        return;
    }
    let mut cached_lists = Vec::new();
    for key in &active_keys {
        if let Some(ics) = load_cached_calendar(key) {
            let raws = parse_ics(&ics, SYSTEM_TZ);
            if !raws.is_empty() {
                cached_lists.push(raws);
            }
        }
    }
    let merged = merge_raw_events(cached_lists);
    if !merged.is_empty() {
        with(|s| {
            if s.snap.cal.is_empty() {
                s.snap.cal = Arc::new(merged);
                s.snap.cal_ver = 1;
                s.snap.calendar_set = true;
                s.cal_key = active_keys.join("\n");
            }
        });
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Начало в местном времени: минуты от 1970-01-01.
    pub start: i64,
    /// Конец того же вхождения; у событий на весь день — начало дня после последнего. 0 — не задан.
    pub end: i64,
    pub all_day: bool,
    pub title: String,
    /// Номер события, созданного в самом острове (его можно править).
    pub local_id: Option<u64>,
}

/// Событие календаря как в файле: одно вхождение или правило повтора.
#[derive(Clone, Debug, PartialEq)]
pub struct RawEvent {
    /// Начало в местном времени: минуты от 1970-01-01.
    pub start: i64,
    /// Конец; у событий на весь день — начало следующего дня. 0 — не задан.
    pub end: i64,
    pub all_day: bool,
    pub title: String,
    pub rrule: Option<String>,
    /// Исключённые повторы (минуты; для событий на весь день важен только день).
    pub exdates: Vec<i64>,
    /// Идентификатор события (UID) для дедупликации между календарями.
    pub uid: String,
}

/// Объединяет списки событий из нескольких календарей с дедупликацией:
/// события с одинаковым непустым UID и одинаковым временем начала считаются дубликатами.
/// Если UID пустой, дедупликация производится по названию и времени начала.
pub fn merge_raw_events(lists: impl IntoIterator<Item = Vec<RawEvent>>) -> Vec<RawEvent> {
    let mut out = Vec::new();
    let mut seen_uid = std::collections::HashSet::new();
    let mut seen_title = std::collections::HashSet::new();
    for list in lists {
        for ev in list {
            if !ev.uid.is_empty() {
                if !seen_uid.insert((ev.uid.clone(), ev.start)) {
                    continue;
                }
            } else if !seen_title.insert((ev.title.clone(), ev.start)) {
                continue;
            }
            out.push(ev);
        }
    }
    out
}

/// Событие в списке выбранного дня.
#[derive(Clone, Debug, PartialEq)]
pub struct DayEvent {
    /// Минуты от полуночи; `None` — весь день (или продолжение многодневного события).
    pub minute: Option<i64>,
    /// Минуты окончания от полуночи; `None` — весь день или не задано.
    pub end_minute: Option<i64>,
    pub title: String,
    /// Номер события, созданного в самом острове (его можно править и удалять).
    pub local_id: Option<u64>,
}

/// Ответ сервера — календарь iCal. Неверная ссылка часто отдаёт обычную страницу (вход,
/// «не найдено») с кодом 200: без этой проверки она считалась пустым календарём, а не ошибкой.
fn check_ics(body: String) -> Result<String, String> {
    let head: String = body.trim_start_matches('\u{feff}').trim_start().chars().take(15).collect();
    if head.eq_ignore_ascii_case("BEGIN:VCALENDAR") {
        Ok(body)
    } else {
        Err("ответ не похож на календарь iCal".into())
    }
}

/// Обновляет события из настроенных iCal-ссылок (раз в 5 минут).
pub fn refresh_calendar(urls: &[String]) {
    refresh_calendar_age(urls, CALENDAR_TTL);
}

/// То же, но обновляет, если данным больше `max_age` (при открытии вкладки — чаще обычного).
pub fn refresh_calendar_age(urls: &[String], max_age: Duration) {
    let clean_urls: Vec<String> = urls
        .iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .collect();
    #[cfg(test)]
    let acc = CalendarAccounts::default();
    #[cfg(not(test))]
    let acc = load_accounts();
    let active_keys = active_calendar_keys_for(&clean_urls, &acc);
    let key = active_keys.join("\n");
    let go = with(|s| {
        s.snap.calendar_set = !active_keys.is_empty();
        if s.cal_busy || (s.cal_key == key && !stale(s.cal_at, max_age)) {
            return false;
        }
        if s.cal_key != key && !s.snap.cal.is_empty() {
            // Список источников сменился: сбрасываем старые события
            s.snap.cal = Arc::default();
            s.snap.cal_ver += 1;
        }
        s.snap.calendar_failed = false;
        if active_keys.is_empty() {
            s.cal_key.clear();
            s.cal_at = Some(Instant::now());
            clean_orphan_caches(&[]);
            return false;
        }
        s.cal_busy = true;
        true
    });
    if !go {
        return;
    }
    spawn("island-calendar", move || {
        let mut fetched: Vec<(String, Result<String, String>)> = Vec::with_capacity(active_keys.len());
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(active_keys.len());
            for url in &clean_urls {
                handles.push(scope.spawn(move || (url.clone(), get_text(url).and_then(check_ics))));
            }
            if let Some(g) = acc.google.clone() {
                let g_key = format!("caldav:google:{}", g.email);
                handles.push(scope.spawn(move || (g_key, sync_google_rest(g))));
            }
            if let Some(a) = acc.apple.clone() {
                let a_key = format!("caldav:apple:{}", a.apple_id);
                handles.push(scope.spawn(move || (a_key, sync_apple_caldav(a))));
            }
            for h in handles {
                if let Ok(res) = h.join() {
                    fetched.push(res);
                }
            }
        });

        let mut all_events = Vec::new();
        let mut any_success = false;
        let mut any_failed = false;

        for (src_key, res) in fetched {
            match res {
                Ok(ics) => {
                    save_cached_calendar(&src_key, &ics);
                    let raws = parse_ics(&ics, SYSTEM_TZ);
                    all_events.push(raws);
                    any_success = true;
                }
                Err(e) => {
                    log::error(&format!("календарь {src_key}: {e}"));
                    any_failed = true;
                    if let Some(cached_ics) = load_cached_calendar(&src_key) {
                        let raws = parse_ics(&cached_ics, SYSTEM_TZ);
                        if !raws.is_empty() {
                            all_events.push(raws);
                        }
                    }
                }
            }
        }

        let merged = merge_raw_events(all_events);
        with(|s| {
            s.cal_busy = false;
            s.cal_key = key.clone();
            if any_success || !merged.is_empty() {
                s.snap.cal = Arc::new(merged);
                s.snap.cal_ver += 1;
            }
            s.snap.calendar_failed = any_failed;
            if any_failed {
                s.cal_at = Instant::now().checked_sub(CALENDAR_TTL - RETRY_AFTER).or_else(|| Some(Instant::now()));
            } else {
                s.cal_at = Some(Instant::now());
            }
        });
        bg::notify();
    });
}

/// Разбирает `DTSTART`: (местные минуты, весь день). Время с `Z` переводится из UTC в местное,
/// время с `TZID` считается местным.
fn parse_dt(value: &str, offset: i64) -> Option<(i64, bool)> {
    let v = value.trim();
    let num = |a: usize, b: usize| v.get(a..b)?.parse::<u32>().ok();
    let day = days_from_civil(num(0, 4)? as i32, num(4, 6)?, num(6, 8)?);
    if v.len() < 13 || v.as_bytes().get(8) != Some(&b'T') {
        return Some((day * 1440, true));
    }
    let mut t = day * 1440 + num(9, 11)? as i64 * 60 + num(11, 13)? as i64;
    if v.ends_with('Z') {
        t += offset_at(t, offset);
    }
    Some((t, false))
}

fn rule_part<'a>(rule: &'a str, key: &str) -> Option<&'a str> {
    rule.split(';').find_map(|p| p.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

/// Ближайшее наступление события, не раньше `now` (для событий на весь день — не раньше
/// сегодняшнего дня). Повторы: DAILY, WEEKLY (с BYDAY), MONTHLY, YEARLY, INTERVAL, UNTIL, COUNT;
/// исключения EXDATE обрабатывает `occurrence_from`.
fn next_occurrence(start: i64, all_day: bool, rrule: Option<&str>, now: i64, offset: i64) -> Option<i64> {
    let threshold = if all_day { now - now.rem_euclid(1440) } else { now };
    if start >= threshold {
        return Some(start);
    }
    let rule = rrule?;
    let interval = rule_part(rule, "INTERVAL").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let until = rule_part(rule, "UNTIL").and_then(|v| parse_dt(v, offset)).map(|(t, day)| if day { t + 1439 } else { t });
    let count = rule_part(rule, "COUNT").and_then(|v| v.parse::<i64>().ok());
    let tod = start.rem_euclid(1440);
    let sday = start.div_euclid(1440);
    let tday = threshold.div_euclid(1440);
    let ok = |t: i64| until.is_none_or(|u| t <= u);
    let found = match rule_part(rule, "FREQ")? {
        "DAILY" => {
            let mut n = (tday - sday).max(0).div_euclid(interval);
            let mut t = (sday + n * interval) * 1440 + tod;
            if t < threshold {
                n += 1;
                t += interval * 1440;
            }
            if count.is_some_and(|c| n >= c) {
                return None;
            }
            Some(t)
        }
        "WEEKLY" => {
            let days: Vec<i64> = rule_part(rule, "BYDAY")
                .map(|v| {
                    v.split(',')
                        .filter_map(|d| {
                            let d = d.trim_start_matches(|c: char| c == '-' || c == '+' || c.is_ascii_digit());
                            ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|x| *x == d).map(|p| p as i64)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let days = if days.is_empty() { vec![weekday(sday)] } else { days };
            let start_monday = sday - weekday(sday);
            let hit = |d: i64| ((d - weekday(d) - start_monday) / 7) % interval == 0 && days.contains(&weekday(d));
            let first = (tday.max(sday)..tday.max(sday) + 7 * interval + 7).find(|&d| hit(d) && d * 1440 + tod >= threshold);
            // COUNT: номер найденного наступления среди всех с начала серии.
            first.filter(|&d| count.is_none_or(|c| (sday..=d).filter(|&x| hit(x)).count() as i64 <= c)).map(|d| d * 1440 + tod)
        }
        freq @ ("MONTHLY" | "YEARLY") => {
            let (y, m, d) = civil_from_days(sday);
            let step = if freq == "MONTHLY" { interval } else { 12 * interval };
            let mut seen = 0i64;
            let mut res = None;
            for k in 0..1200 {
                let months = (m as i64 - 1) + k * step;
                let (yy, mm) = (y as i64 + months.div_euclid(12), months.rem_euclid(12) as u32 + 1);
                let day = days_from_civil(yy as i32, mm, d);
                // Нет такого числа в месяце (31-е) — пропускаем.
                if civil_from_days(day).2 != d {
                    continue;
                }
                seen += 1;
                let t = day * 1440 + tod;
                if t >= threshold {
                    res = count.is_none_or(|c| seen <= c).then_some(t);
                    break;
                }
            }
            res
        }
        _ => None,
    };
    found.filter(|&t| ok(t))
}

fn unescape_ics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push(' '),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Default)]
struct Cur {
    start: Option<(i64, bool)>,
    end: Option<i64>,
    /// `DURATION` в минутах: заменяет `DTEND`, если конца нет.
    duration: Option<i64>,
    title: String,
    rrule: Option<String>,
    exdates: Vec<i64>,
    uid: String,
    /// `RECURRENCE-ID`: это изменённое вхождение повтора, а не отдельное событие.
    rid: Option<i64>,
    cancelled: bool,
}

/// Разбирает `DURATION` (`PT1H30M`, `P1D`, `P1W`, `P1DT2H`) в минуты; секунды отбрасываются.
fn parse_duration(v: &str) -> Option<i64> {
    let v = v.trim().strip_prefix('P')?;
    let (date, time) = v.split_once('T').unwrap_or((v, ""));
    let mut total = 0i64;
    for (part, units) in [(date, [('W', 10080), ('D', 1440)].as_slice()), (time, [('H', 60), ('M', 1)].as_slice())] {
        let mut num = String::new();
        for ch in part.chars() {
            if ch.is_ascii_digit() {
                num.push(ch);
            } else if let Some(&(_, mul)) = units.iter().find(|(u, _)| *u == ch) {
                total += num.parse::<i64>().ok()? * mul;
                num.clear();
            } else if ch != 'S' {
                return None;
            } else {
                num.clear();
            }
        }
    }
    Some(total)
}

/// Все события календаря в формате iCalendar.
pub fn parse_ics(ics: &str, offset: i64) -> Vec<RawEvent> {
    // Длинные строки iCal переносятся: продолжение начинается с пробела или табуляции.
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.split('\n') {
        let l = raw.trim_end_matches('\r');
        match lines.last_mut() {
            Some(last) if l.starts_with(' ') || l.starts_with('\t') => last.push_str(&l[1..]),
            _ => lines.push(l.to_string()),
        }
    }
    let mut found: Vec<(String, Option<i64>, RawEvent)> = Vec::new();
    let mut cur: Option<Cur> = None;
    for l in &lines {
        if l == "BEGIN:VEVENT" {
            cur = Some(Cur::default());
            continue;
        }
        if l == "END:VEVENT" {
            if let Some(c) = cur.take() {
                if let (Some((start, all_day)), false) = (c.start, c.cancelled) {
                    // Конец: у даты без времени `DTEND` — первый день после события.
                    let end = c
                        .end
                        .filter(|&e| e > start)
                        .or_else(|| c.duration.filter(|&d| d > 0).map(|d| start + if all_day { d.max(1440) } else { d }))
                        .unwrap_or(0);
                    let ev = RawEvent {
                        start,
                        end,
                        all_day,
                        title: unescape_ics(&c.title),
                        rrule: c.rrule,
                        exdates: c.exdates,
                        uid: c.uid.clone(),
                    };
                    found.push((c.uid, c.rid, ev));
                }
            }
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        let Some((name, value)) = l.split_once(':') else { continue };
        match name.split(';').next().unwrap_or("") {
            "DTSTART" => c.start = parse_dt(value, offset),
            "DTEND" => c.end = parse_dt(value, offset).map(|(t, _)| t),
            "DURATION" => c.duration = parse_duration(value),
            "SUMMARY" => c.title = value.to_string(),
            "RRULE" => c.rrule = Some(value.to_string()),
            "EXDATE" => c.exdates.extend(value.split(',').filter_map(|v| parse_dt(v, offset)).map(|(t, _)| t)),
            "UID" => c.uid = value.trim().to_string(),
            "RECURRENCE-ID" => c.rid = parse_dt(value, offset).map(|(t, _)| t),
            "STATUS" => c.cancelled = value.trim().eq_ignore_ascii_case("CANCELLED"),
            _ => {}
        }
    }
    // Изменённое вхождение заменяет исходное: исходное в этот день пропускаем.
    let moved: Vec<(String, i64)> = found.iter().filter_map(|(u, r, _)| r.map(|r| (u.clone(), r))).collect();
    for (uid, rid) in moved {
        if let Some((_, _, master)) = found.iter_mut().find(|(u, r, e)| *u == uid && r.is_none() && e.rrule.is_some()) {
            master.exdates.push(rid);
        }
    }
    found.into_iter().map(|(_, _, e)| e).collect()
}

/// Начало следующего вхождения `raw` не раньше `from` с учётом исключений.
pub(super) fn occurrence_from(raw: &RawEvent, from: i64, offset: i64) -> Option<i64> {
    let mut from = from;
    for _ in 0..64 {
        let t = next_occurrence(raw.start, raw.all_day, raw.rrule.as_deref(), from, offset)?;
        let skipped = raw.exdates.iter().any(|&x| if raw.all_day { x.div_euclid(1440) == t.div_euclid(1440) } else { x == t });
        if !skipped {
            return Some(t);
        }
        from = if raw.all_day { (t.div_euclid(1440) + 1) * 1440 } else { t + 1 };
    }
    None
}

/// Как далеко вперёд ищется ближайшее событие, дней: более далёкие не показываются.
pub const NEXT_EVENT_DAYS: i64 = 30;

/// Ближайшее событие не раньше `now` и не позже чем через `NEXT_EVENT_DAYS` дней.
pub fn next_of(raws: &[RawEvent], now: i64, offset: i64) -> Option<Event> {
    let horizon = now + NEXT_EVENT_DAYS * 1440;
    raws.iter()
        .filter_map(|r| occurrence_from(r, now, offset).map(|t| (t, r)))
        .filter(|(t, _)| *t <= horizon)
        .min_by_key(|(t, _)| *t)
        .map(|(t, r)| {
            // Длительность та же у каждого повтора: конец вхождения = его начало + длительность.
            let end = if r.end > r.start { t + (r.end - r.start) } else { 0 };
            let local_id = r.uid.strip_prefix("island-local-").and_then(|n| n.parse().ok());
            Event { start: t, end, all_day: r.all_day, title: r.title.clone(), local_id }
        })
}

/// Ближайшее событие календаря в формате iCalendar.
#[cfg(test)]
pub fn next_event(ics: &str, now: i64, offset: i64) -> Option<Event> {
    next_of(&parse_ics(ics, offset), now, offset)
}

/// События по дням: `days` дней подряд, начиная с `first_day` (дни от 1970-01-01).
/// В каждом дне сначала события на весь день, потом по времени.
pub fn events_by_day(raws: &[RawEvent], first_day: i64, days: usize, offset: i64) -> Vec<Vec<DayEvent>> {
    let (lo, hi) = (first_day, first_day + days as i64);
    let mut out: Vec<Vec<DayEvent>> = vec![Vec::new(); days];
    for raw in raws {
        let dur = if raw.end > raw.start { raw.end - raw.start } else { 0 };
        let local_id = raw.uid.strip_prefix("island-local-").and_then(|n| n.parse().ok());
        let mut add = |t: i64| {
            let first = t.div_euclid(1440);
            let last = if dur > 0 { (t + dur - 1).div_euclid(1440).max(first) } else { first };
            for d in first.max(lo)..=last.min(hi - 1) {
                let minute = (!raw.all_day && d == first).then_some(t.rem_euclid(1440));
                let end_minute = if raw.all_day {
                    None
                } else if dur > 0 {
                    let end_t = t + dur;
                    let end_d = end_t.div_euclid(1440);
                    if end_d == d {
                        Some(end_t.rem_euclid(1440))
                    } else if end_d > d {
                        Some(1440)
                    } else {
                        minute
                    }
                } else {
                    minute
                };
                out[(d - lo) as usize].push(DayEvent { minute, end_minute, title: raw.title.clone(), local_id });
            }
        };
        if raw.rrule.is_none() {
            add(raw.start);
            continue;
        }
        let mut from = lo * 1440;
        for _ in 0..400 {
            let Some(t) = occurrence_from(raw, from, offset) else { break };
            if t >= hi * 1440 {
                break;
            }
            add(t);
            from = if raw.all_day { (t.div_euclid(1440) + 1) * 1440 } else { t + 1 };
        }
    }
    for day in &mut out {
        day.sort_by(|a, b| a.minute.cmp(&b.minute).then_with(|| a.title.cmp(&b.title)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 1440 + h * 60 + mi
    }

    #[test]
    fn google_rfc3339_to_compact_utc() {
        assert_eq!(rfc3339_to_ics_utc("2026-11-12T10:00:00+03:00").as_deref(), Some("20261112T070000Z"));
        assert_eq!(rfc3339_to_ics_utc("2026-11-12T10:00:00Z").as_deref(), Some("20261112T100000Z"));
        assert_eq!(rfc3339_to_ics_utc("2026-11-12T10:00:00.250-05:30").as_deref(), Some("20261112T153000Z"));
        // Переход через полночь и границу года.
        assert_eq!(rfc3339_to_ics_utc("2027-01-01T01:30:00+03:00").as_deref(), Some("20261231T223000Z"));
        assert_eq!(rfc3339_to_ics_utc("2026-11-12T10:00:00"), None, "без зоны время неоднозначно");
        assert_eq!(rfc3339_to_ics_utc("2026-13-12T10:00:00Z"), None);
        assert_eq!(rfc3339_to_ics_utc("мусор"), None);
    }

    #[test]
    fn google_calendar_list_picks_shown_and_primary() {
        let j = r#"{"items":[
            {"id":"team@group.calendar.google.com","selected":true},
            {"id":"user@gmail.com","primary":true,"selected":true},
            {"id":"hidden@group.calendar.google.com"},
            {"id":"gone@group.calendar.google.com","selected":true,"deleted":true}
        ]}"#;
        let (email, ids) = parse_google_calendar_list(j);
        assert_eq!(email.as_deref(), Some("user@gmail.com"));
        assert_eq!(ids, ["team@group.calendar.google.com", "user@gmail.com"]);
        assert_eq!(parse_google_calendar_list(r#"{"items":[]}"#), (None, Vec::new()));
    }

    #[test]
    fn google_events_become_parsable_ics() {
        let j = r#"{"items":[
            {"id":"ev1","status":"confirmed","summary":"План; бюджет, Q4\nитоги","start":{"dateTime":"2026-11-12T10:00:00+03:00"},"end":{"dateTime":"2026-11-12T11:00:00+03:00"}},
            {"id":"ev2","status":"cancelled","summary":"Отменено","start":{"date":"2026-11-13"},"end":{"date":"2026-11-14"}},
            {"id":"ev3","summary":"Отпуск","start":{"date":"2026-11-14"},"end":{"date":"2026-11-16"}},
            {"id":"ev4","summary":"Без начала"}
        ]}"#;
        let mut ics = String::from("BEGIN:VCALENDAR\r\n");
        google_events_to_ics(j, &mut ics);
        ics.push_str("END:VCALENDAR\r\n");
        assert!(ics.contains("UID:ev1\r\n") && ics.contains("DTSTART:20261112T070000Z\r\n"));
        assert!(ics.contains("DTSTART;VALUE=DATE:20261114\r\n"));
        let raws = parse_ics(&ics, 180);
        assert_eq!(raws.len(), 2, "отменённое и событие без начала пропускаются");
        assert_eq!(raws[0].title, "План; бюджет, Q4 итоги");
        assert_eq!((raws[0].start, raws[0].end), (at(2026, 11, 12, 10, 0), at(2026, 11, 12, 11, 0)));
        assert_eq!(raws[0].uid, "ev1");
        assert!(raws[1].all_day);
        assert_eq!((raws[1].start, raws[1].end), (at(2026, 11, 14, 0, 0), at(2026, 11, 16, 0, 0)));
    }

    #[test]
    fn ics_next_single_and_utc() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20261015T080000Z\r\nSUMMARY:Встреча \r\n с командой\r\nEND:VEVENT\r\n\
                   BEGIN:VEVENT\r\nDTSTART;TZID=Europe/Moscow:20261016T090000\r\nSUMMARY:Позже\r\nEND:VEVENT\r\n\
                   BEGIN:VEVENT\r\nDTSTART:20261001T080000Z\r\nSUMMARY:Прошло\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let now = at(2026, 10, 15, 9, 0);
        let ev = next_event(ics, now, 180).unwrap();
        assert_eq!(ev.title, "Встреча с командой");
        assert_eq!(ev.start, at(2026, 10, 15, 11, 0)); // 08:00 UTC + 3 ч
    }

    #[test]
    fn events_are_laid_out_by_day() {
        let ics = "BEGIN:VEVENT\nDTSTART:20261016T090000\nSUMMARY:A\nEND:VEVENT\n\
                   BEGIN:VEVENT\nDTSTART;VALUE=DATE:20261016\nDTEND;VALUE=DATE:20261019\nSUMMARY:Trip\nEND:VEVENT\n\
                   BEGIN:VEVENT\nDTSTART:20261001T100000\nRRULE:FREQ=WEEKLY;BYDAY=FR\nEXDATE:20261023T100000\nSUMMARY:Fri\nEND:VEVENT\n\
                   BEGIN:VEVENT\nDTSTART:20261001T080000\nSUMMARY:Off\nSTATUS:CANCELLED\nEND:VEVENT\n";
        let raws = parse_ics(ics, 0);
        assert_eq!(raws.len(), 3, "отменённое событие пропускается");
        let first = days_from_civil(2026, 10, 12);
        let days = events_by_day(&raws, first, 28, 0);
        let on = |d: u32| &days[(days_from_civil(2026, 10, d) - first) as usize];
        // 16-го: поездка на весь день, затем 09:00 и пятничный повтор в 10:00.
        let t16: Vec<_> = on(16).iter().map(|e| (e.minute, e.title.as_str())).collect();
        assert_eq!(t16, vec![(None, "Trip"), (Some(540), "A"), (Some(600), "Fri")]);
        // Поездка на весь день занимает 16–18-е, 19-е свободно.
        assert!(on(17).iter().any(|e| e.title == "Trip") && on(18).iter().any(|e| e.title == "Trip"));
        assert!(on(19).is_empty());
        // 23-е исключено EXDATE, 30-е повторяется.
        assert!(on(23).is_empty());
        assert_eq!(on(30).len(), 1);
    }

    #[test]
    fn next_event_only_within_30_days() {
        let now = at(2026, 10, 15, 9, 0);
        let one = |start: &str| format!("BEGIN:VEVENT\nDTSTART:{start}\nSUMMARY:X\nEND:VEVENT\n");
        // Ровно через 30 дней — ещё показывается, через 30 дней и минуту — уже нет.
        assert!(next_event(&one("20261114T090000"), now, 0).is_some());
        assert!(next_event(&one("20261114T090100"), now, 0).is_none());
        // Далёкое событие не мешает ближнему, а повтор подхватывается в окне.
        let two = format!("{}{}", one("20270101T100000"), one("20261020T100000"));
        assert_eq!(next_event(&two, now, 0).unwrap().start, at(2026, 10, 20, 10, 0));
        let yearly = "BEGIN:VEVENT\nDTSTART:20250101T100000\nRRULE:FREQ=YEARLY\nSUMMARY:Y\nEND:VEVENT\n";
        assert!(next_event(yearly, now, 0).is_none());
    }

    #[test]
    fn duration_values() {
        assert_eq!(parse_duration("PT1H30M"), Some(90));
        assert_eq!(parse_duration("P1D"), Some(1440));
        assert_eq!(parse_duration("P1W"), Some(10080));
        assert_eq!(parse_duration("P1DT2H"), Some(1560));
        assert_eq!(parse_duration("PT45M"), Some(45));
        assert_eq!(parse_duration("PT30S"), Some(0));
        assert_eq!(parse_duration("мусор"), None);
        assert_eq!(parse_duration("P1X"), None);
    }

    #[test]
    fn next_event_carries_end_from_dtend_or_duration() {
        let now = at(2026, 10, 15, 9, 0);
        let dtend = "BEGIN:VEVENT
DTSTART:20261015T100000
DTEND:20261015T113000
SUMMARY:A
END:VEVENT
";
        let ev = next_event(dtend, now, 0).unwrap();
        assert_eq!((ev.start, ev.end), (at(2026, 10, 15, 10, 0), at(2026, 10, 15, 11, 30)));
        let dur = "BEGIN:VEVENT
DTSTART:20261015T100000
DURATION:PT1H
SUMMARY:B
END:VEVENT
";
        assert_eq!(next_event(dur, now, 0).unwrap().end, at(2026, 10, 15, 11, 0));
        // Повтор: конец сдвигается вместе с началом.
        let daily = "BEGIN:VEVENT
DTSTART:20261001T100000
DTEND:20261001T104500
RRULE:FREQ=DAILY
SUMMARY:C
END:VEVENT
";
        let ev = next_event(daily, now, 0).unwrap();
        assert_eq!((ev.start, ev.end), (at(2026, 10, 15, 10, 0), at(2026, 10, 15, 10, 45)));
        // Без конца — end = 0, показывается только начало.
        let none = "BEGIN:VEVENT
DTSTART:20261015T100000
SUMMARY:D
END:VEVENT
";
        assert_eq!(next_event(none, now, 0).unwrap().end, 0);
    }

    #[test]
    fn events_by_day_end_minute() {
        let ics = "BEGIN:VEVENT\nDTSTART:20261016T090000\nDTEND:20261016T103000\nSUMMARY:Meeting\nEND:VEVENT\n";
        let raws = parse_ics(ics, 0);
        let first = days_from_civil(2026, 10, 16);
        let days = events_by_day(&raws, first, 1, 0);
        assert_eq!(days[0].len(), 1);
        assert_eq!(days[0][0].minute, Some(540));
        assert_eq!(days[0][0].end_minute, Some(630));
    }

    #[test]
    fn moved_instance_replaces_the_original() {
        let ics = "BEGIN:VEVENT\nUID:x\nDTSTART:20261001T100000\nRRULE:FREQ=DAILY\nSUMMARY:Daily\nEND:VEVENT\n\
                   BEGIN:VEVENT\nUID:x\nRECURRENCE-ID:20261015T100000\nDTSTART:20261015T150000\nSUMMARY:Daily moved\nEND:VEVENT\n";
        let raws = parse_ics(ics, 0);
        let first = days_from_civil(2026, 10, 14);
        let days = events_by_day(&raws, first, 3, 0);
        assert_eq!(days[0].len(), 1);
        let mid: Vec<_> = days[1].iter().map(|e| (e.minute, e.title.as_str())).collect();
        assert_eq!(mid, vec![(Some(900), "Daily moved")]);
    }

    #[test]
    fn ics_recurring() {
        let daily = "BEGIN:VEVENT\nDTSTART;TZID=X:20260901T110000\nRRULE:FREQ=DAILY\nSUMMARY:Daily\nEND:VEVENT\n";
        let now = at(2026, 10, 15, 12, 0);
        assert_eq!(next_event(daily, now, 0).unwrap().start, at(2026, 10, 16, 11, 0));

        let weekly = "BEGIN:VEVENT\nDTSTART:20260907T100000\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\nSUMMARY:W\nEND:VEVENT\n";
        // 15.10.2026 — четверг: следующая — понедельник 19.10.
        assert_eq!(next_event(weekly, now, 0).unwrap().start, at(2026, 10, 19, 10, 0));

        let ended = "BEGIN:VEVENT\nDTSTART:20260901T100000\nRRULE:FREQ=DAILY;UNTIL=20261001T000000Z\nSUMMARY:E\nEND:VEVENT\n";
        assert!(next_event(ended, now, 0).is_none());

        let monthly = "BEGIN:VEVENT\nDTSTART;VALUE=DATE:20260131\nRRULE:FREQ=MONTHLY\nSUMMARY:M\nEND:VEVENT\n";
        let ev = next_event(monthly, now, 0).unwrap();
        assert!(ev.all_day);
        assert_eq!(ev.start, at(2026, 10, 31, 0, 0));
    }

    #[test]
    fn ics_count_non_daily() {
        let now = at(2026, 10, 15, 12, 0);
        let weekly = |c: u32| format!("BEGIN:VEVENT
DTSTART:20260907T100000
RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT={c}
SUMMARY:W
END:VEVENT
");
        assert!(next_event(&weekly(2), now, 0).is_none());
        assert_eq!(next_event(&weekly(8), now, 0).unwrap().start, at(2026, 10, 19, 10, 0));
        let monthly = |c: u32| format!("BEGIN:VEVENT
DTSTART:20260901T100000
RRULE:FREQ=MONTHLY;COUNT={c}
SUMMARY:M
END:VEVENT
");
        assert!(next_event(&monthly(2), now, 0).is_none());
        assert_eq!(next_event(&monthly(3), now, 0).unwrap().start, at(2026, 11, 1, 10, 0));
    }

    #[test]
    fn calendar_cache_sealing_and_url_binding() {
        let url = "https://calendar.google.com/calendar/ical/test%40gmail.com/private-1234/basic.ics";
        let other_url = "https://calendar.google.com/calendar/ical/other%40gmail.com/private-5678/basic.ics";
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Приватная встреча\nEND:VEVENT\nEND:VCALENDAR";

        let sealed = pack_cached_calendar(url, ics).expect("seal");
        assert!(crate::sys::is_sealed(&sealed));

        // При совпадении ссылки кэш успешно расшифровывается и отдаёт оригинальный ICS.
        let unpacked = unpack_cached_calendar(url, &sealed);
        assert_eq!(unpacked.as_deref(), Some(ics));

        // При смене ссылки кэш игнорируется (защита от показа чужого кэша).
        assert_eq!(unpack_cached_calendar(other_url, &sealed), None);

        // При пустой ссылке (календарь отключён) кэш не используется.
        assert_eq!(unpack_cached_calendar("", &sealed), None);
        assert_eq!(unpack_cached_calendar("   ", &sealed), None);

        // Повреждённые байты и мусор не приводят к панике и игнорируются.
        assert_eq!(unpack_cached_calendar(url, b"ISL1corruptedgarbage"), None);
        assert_eq!(unpack_cached_calendar(url, b"random bytes"), None);
        assert_eq!(unpack_cached_calendar(url, b""), None);

        // Старый незашифрованный ICS без заголовка с хэшем ссылки игнорируется.
        assert_eq!(unpack_cached_calendar(url, ics.as_bytes()), None);
    }

    #[test]
    fn merge_raw_events_deduplication() {
        let ev = |uid: &str, title: &str, start: i64| RawEvent {
            start,
            end: start + 60,
            all_day: false,
            title: title.to_string(),
            rrule: None,
            exdates: Vec::new(),
            uid: uid.to_string(),
        };

        // Календарь 1: события A, B, C
        let cal1 = vec![
            ev("uid-1", "Meeting 1", 1000),
            ev("uid-2", "Recurring Master", 2000),
            ev("", "No UID Event", 3000),
        ];

        // Календарь 2:
        // - дубликат uid-1 в то же время (должен отсечься)
        // - то же uid-2, но другое время (изменённое вхождение — должно остаться)
        // - другое событие в то же время 1000 (должно остаться)
        // - дубликат события без UID (должен отсечься)
        // - другое событие без UID в то же время (должно остаться)
        let cal2 = vec![
            ev("uid-1", "Meeting 1 (copy)", 1000),
            ev("uid-2", "Recurring Moved", 2500),
            ev("uid-3", "Other Meeting", 1000),
            ev("", "No UID Event", 3000),
            ev("", "Another No UID", 3000),
        ];

        let merged = merge_raw_events(vec![cal1, cal2]);
        assert_eq!(merged.len(), 6);

        assert_eq!(merged[0].uid, "uid-1");
        assert_eq!(merged[0].title, "Meeting 1");

        assert_eq!(merged[1].uid, "uid-2");
        assert_eq!(merged[1].start, 2000);

        assert_eq!(merged[2].title, "No UID Event");

        assert_eq!(merged[3].uid, "uid-2");
        assert_eq!(merged[3].start, 2500);

        assert_eq!(merged[4].uid, "uid-3");
        assert_eq!(merged[5].title, "Another No UID");
    }

    #[test]
    fn parse_ics_preserves_uid() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:unique-event-id-999\nDTSTART:20261015T100000Z\nSUMMARY:Test UID\nEND:VEVENT\nEND:VCALENDAR\n";
        let raws = parse_ics(ics, 0);
        assert_eq!(raws.len(), 1);
        assert_eq!(raws[0].uid, "unique-event-id-999");
        assert_eq!(raws[0].title, "Test UID");
    }

    #[test]
    fn only_icalendar_body_counts_as_calendar() {
        assert!(check_ics("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".into()).is_ok());
        assert!(check_ics("\u{feff}\r\nbegin:vcalendar\nEND:VCALENDAR".into()).is_ok());
        // Неверная ссылка часто отдаёт обычную страницу с кодом 200: это ошибка, а не пустой календарь.
        assert!(check_ics("<!doctype html><html><body>Sign in</body></html>".into()).is_err());
        assert!(check_ics(String::new()).is_err());
    }

    #[test]
    fn calendar_cache_filename_is_deterministic() {
        let u1 = "https://a.com/basic.ics";
        let u2 = "https://b.com/basic.ics";
        assert_eq!(cache_filename(u1), cache_filename(u1));
        assert_ne!(cache_filename(u1), cache_filename(u2));
        assert!(cache_filename(u1).starts_with(CALENDAR_CACHE_PREFIX));
        assert!(cache_filename(u1).ends_with(".ics"));
    }

    #[test]
    fn cal_accounts_dpapi_roundtrip_and_active_keys() {
        let acc = CalendarAccounts {
            google: Some(GoogleAccount {
                email: "user@gmail.com".into(),
                refresh_token: "1//refresh".into(),
                access_token: "ya29.access".into(),
                expires_at: 1800000000,
            }),
            apple: Some(AppleAccount {
                apple_id: "user@icloud.com".into(),
                app_password: "abcd-efgh-ijkl-mnop".into(),
            }),
        };
        let sealed = pack_accounts(&acc).expect("DPAPI seal");
        assert!(crate::sys::is_sealed(&sealed));
        let unpacked = unpack_accounts(&sealed).expect("DPAPI unseal");
        assert_eq!(unpacked, acc);
        assert_eq!(unpack_accounts(b"corrupted"), None);
        assert_eq!(unpack_accounts(b""), None);

        let keys = active_calendar_keys_for(&["https://example.com/a.ics".to_string()], &acc);
        assert_eq!(
            keys,
            vec![
                "https://example.com/a.ics".to_string(),
                "caldav:google:user@gmail.com".to_string(),
                "caldav:apple:user@icloud.com".to_string(),
            ]
        );
    }
}
