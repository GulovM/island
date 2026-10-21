//! Обновление программы через GitHub Releases. Проверка — один короткий поток с запросом
//! `releases/latest`; автоматически — при запуске (если прошёл час с прошлой проверки, время
//! лежит в `update.txt`) и в начале каждого часа по местному времени, ещё — по кнопке в Настройках. Установщик скачивается во
//! временную папку, сверяется с `SHA256SUMS.txt` из того же релиза и запускается тихо: он сам
//! закрывает остров, заменяет файлы и запускает его снова. В простое потоков нет.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::{w, PCWSTR};
use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::{bg, log, online, sys};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const API: &str = "https://api.github.com/repos/GulovM/island/releases/latest";
/// Скачивать можно только отсюда: ссылки из ответа API проверяются по этому префиксу.
const DOWNLOADS: &str = "https://github.com/GulovM/island/releases/download/";
const STAMP_FILE: &str = "update.txt";
const HOUR_SECS: u64 = 60 * 60;
const MAX_SETUP: usize = 128 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    setup_url: String,
    sums_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading,
    Failed(&'static str),
}

static STATE: Mutex<State> = Mutex::new(State::Idle);

pub fn state() -> State {
    STATE.lock().map(|s| s.clone()).unwrap_or(State::Idle)
}

fn set(s: State) {
    if let Ok(mut g) = STATE.lock() {
        *g = s;
    }
    bg::notify();
}

/// Идёт проверка или скачивание — новый запуск не нужен.
pub fn busy() -> bool {
    matches!(state(), State::Checking | State::Downloading)
}

/// Найдено обновление, которое ещё не установлено.
pub fn available() -> bool {
    matches!(state(), State::Available(_))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn stamp_path() -> std::path::PathBuf {
    sys::data_dir().join(STAMP_FILE)
}

fn last_check() -> u64 {
    std::fs::read_to_string(stamp_path()).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

fn due(last: u64, now: u64) -> bool {
    now.saturating_sub(last) >= HOUR_SECS || last > now
}

/// При запуске: если с последней проверки прошёл час — проверяет обновления. При `auto_install`
/// найденное ставится само.
pub fn tick(auto_install: bool) {
    if !busy() && due(last_check(), now_secs()) {
        check(auto_install);
    }
}

/// По часовому таймеру (ровно в 00:00, 01:00, …): проверяет обновления без оглядки на прошлую проверку.
pub fn hourly(auto_install: bool) {
    if !busy() {
        check(auto_install);
    }
}

/// Миллисекунды до ближайшего начала часа по местному времени (с запасом в секунду,
/// чтобы таймер не сработал чуть раньше границы и не повторился).
pub fn ms_to_next_hour() -> u32 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    let past = (t.wMinute as u32 * 60 + t.wSecond as u32) * 1000 + t.wMilliseconds as u32;
    3_600_000 - past + 1000
}

/// Проверяет обновления сейчас (кнопка в Настройках или суточная проверка).
pub fn check(auto_install: bool) {
    if busy() {
        return;
    }
    set(State::Checking);
    let spawned = std::thread::Builder::new().name("island-update".into()).spawn(move || {
        let next = match fetch_latest() {
            Ok(found) => {
                let _ = std::fs::create_dir_all(sys::data_dir());
                let _ = std::fs::write(stamp_path(), now_secs().to_string());
                match found {
                    Some(r) => State::Available(r),
                    None => State::UpToDate,
                }
            }
            Err(e) => {
                log::error(&format!("проверка обновлений: {e}"));
                State::Failed("Нет связи с GitHub")
            }
        };
        let release = if let State::Available(r) = &next { Some(r.clone()) } else { None };
        set(next);
        if let (Some(r), true) = (release, auto_install) {
            run_install(&r);
        }
    });
    if spawned.is_err() {
        set(State::Failed("Нет связи с GitHub"));
    }
}

/// Скачивает найденное обновление и запускает установщик.
pub fn install() {
    let State::Available(r) = state() else { return };
    set(State::Downloading);
    let spawned = std::thread::Builder::new().name("island-install".into()).spawn(move || run_install(&r));
    if spawned.is_err() {
        set(State::Failed("Не удалось скачать"));
    }
}

fn run_install(r: &Release) {
    set(State::Downloading);
    match download_and_launch(r) {
        // Установщик закроет остров сам; состояние остаётся «скачивается» до выхода.
        Ok(()) => log::info(&format!("запущен установщик версии {}", r.version)),
        Err(msg) => set(State::Failed(msg)),
    }
}

fn fetch_latest() -> Result<Option<Release>, String> {
    let text = online::get_text(API)?;
    Ok(parse_release(&text).filter(|r| is_newer(&r.version, VERSION)))
}

/// Версия и ссылки на установщик и контрольные суммы из ответа `releases/latest`.
fn parse_release(json: &str) -> Option<Release> {
    let tag = online::json_str(json, "tag_name")?;
    let version = tag.trim_start_matches('v').to_string();
    parse_ver(&version)?;
    let key = "\"browser_download_url\":";
    let urls: Vec<String> = json
        .match_indices(key)
        .filter_map(|(i, _)| {
            let rest = json[i + key.len()..].trim_start().strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_string())
        })
        .filter(|u| u.starts_with(DOWNLOADS))
        .collect();
    let setup_url = urls.iter().find(|u| u.ends_with("-setup.exe"))?.clone();
    let sums_url = urls.iter().find(|u| u.ends_with("/SHA256SUMS.txt"))?.clone();
    Some(Release { version, setup_url, sums_url })
}

fn parse_ver(s: &str) -> Option<Vec<u32>> {
    let core = s.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let parts: Option<Vec<u32>> = core.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parse_ver(candidate), parse_ver(current)) else { return false };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

fn download_and_launch(r: &Release) -> Result<(), &'static str> {
    let name = r.setup_url.rsplit('/').next().filter(|n| !n.is_empty()).ok_or("Не удалось скачать")?;
    let sums = online::get_text(&r.sums_url).map_err(|e| fail("контрольные суммы", e, "Не удалось скачать"))?;
    let want = expected_hash(&sums, name).ok_or("Файл повреждён")?;
    let bytes = online::get_bytes(&r.setup_url, MAX_SETUP).map_err(|e| fail("установщик", e, "Не удалось скачать"))?;
    if sha256_hex(&bytes).as_deref() != Some(want.as_str()) {
        log::error("контрольная сумма установщика не совпала");
        return Err("Файл повреждён");
    }
    let dir = std::env::temp_dir().join("island-update");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(name);
    std::fs::write(&path, &bytes).map_err(|e| fail("запись установщика", e, "Не удалось скачать"))?;

    // Перед запуском установщика сбрасываем всё состояние острова на диск.
    crate::state::flush();

    // ShellExecute, а не Command: при установке «для всех» установщик сам запросит права. Установку
    // «для себя» в защищённую папку (например, Program Files) он не повышает — просим права сами,
    // иначе старый island.exe не заменить («отказано в доступе»).
    let verb = if install_dir_writable() { w!("open") } else { w!("runas") };
    let file = sys::wz(&path.to_string_lossy());
    let h = unsafe {
        ShellExecuteW(None, verb, PCWSTR(file.as_ptr()), w!("/SILENT /SP- /NORESTART"), PCWSTR::null(), SW_SHOWNORMAL)
    };
    if (h.0 as isize) <= 32 {
        log::error("не удалось запустить установщик");
        return Err("Не удалось запустить установщик");
    }
    Ok(())
}

/// Можно ли без повышения прав писать в папку, где лежит island.exe (пробный файл создаётся и удаляется).
fn install_dir_writable() -> bool {
    let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) else { return true };
    let probe = dir.join(format!(".island-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(f) => {
            drop(f);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(_) => true,
    }
}

fn fail(what: &str, e: impl std::fmt::Display, msg: &'static str) -> &'static str {
    log::error(&format!("обновление, {what}: {e}"));
    msg
}

/// Хэш файла `name` из `SHA256SUMS.txt` (строки вида «хэш  имя»).
fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (hash, file) = (it.next()?, it.next()?);
        (file.trim_start_matches('*') == name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_hex(data: &[u8]) -> Option<String> {
    let mut out = [0u8; 32];
    // SAFETY: псевдодескриптор SHA-256, буферы живут на время вызова.
    let st = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    st.is_ok().then(|| out.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.3.1", "0.3.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("мусор", "0.3.0"));
        assert!(is_newer("v0.4.0-beta", "0.3.0"));
    }

    #[test]
    fn release_json_is_parsed_and_foreign_links_are_ignored() {
        let json = format!(
            r#"{{"tag_name":"v0.4.0","assets":[{{"name":"a","browser_download_url":"https://evil.example/island-0.4.0-setup.exe"}},
            {{"name":"island-0.4.0-setup.exe","browser_download_url":"{d}v0.4.0/island-0.4.0-setup.exe"}},
            {{"name":"SHA256SUMS.txt","browser_download_url":"{d}v0.4.0/SHA256SUMS.txt"}}]}}"#,
            d = DOWNLOADS
        );
        let r = parse_release(&json).unwrap();
        assert_eq!(r.version, "0.4.0");
        assert!(r.setup_url.starts_with(DOWNLOADS) && r.setup_url.ends_with("island-0.4.0-setup.exe"));
        assert!(r.sums_url.ends_with("SHA256SUMS.txt"));
        assert!(parse_release(r#"{"tag_name":"v0.4.0","assets":[]}"#).is_none());
    }

    #[test]
    fn hashes_match_by_file_name() {
        let h = "a".repeat(64);
        let sums = format!("{h}  island-0.4.0-portable.zip\n{}  island-0.4.0-setup.exe\n", "B".repeat(64));
        assert_eq!(expected_hash(&sums, "island-0.4.0-setup.exe"), Some("b".repeat(64)));
        assert_eq!(expected_hash(&sums, "nope.exe"), None);
        assert_eq!(sha256_hex(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn check_is_due_once_a_day() {
        assert!(due(0, 100_000));
        assert!(!due(100_000, 100_000 + HOUR_SECS - 1));
        assert!(due(100_000, 100_000 + HOUR_SECS));
        assert!(due(200_000, 100_000), "часы ушли назад — проверяем");
    }
}
