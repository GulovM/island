//! Обновление программы через GitHub Releases. Проверка — один короткий поток с запросом
//! `releases/latest`; автоматически — при запуске (если прошёл час с прошлой проверки, время
//! лежит в `update.txt`) и в начале каждого часа по местному времени, ещё — по кнопке в Настройках. Установщик скачивается во
//! временную папку, сверяется с `SHA256SUMS.txt` из того же релиза и запускается тихо: он сам
//! закрывает остров, заменяет файлы и запускает его снова. В простое потоков нет.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::{w, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::{bg, log, online, sys};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CURRENT_CHANGELOG: &str = env!("ISLAND_CURRENT_CHANGELOG");
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
    pub notes_ru: Vec<String>,
    pub notes_en: Vec<String>,
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

/// Извлекает секцию `## [X.Y.Z]` из `CHANGELOG.md` (если `version == None` — первую секцию).
pub fn extract_changelog_section(md: &str, version: Option<&str>) -> String {
    let mut in_sec = false;
    let mut out = Vec::new();
    for raw in md.lines() {
        let line = raw.trim_end();
        if let Some(rest) = line.strip_prefix("## [") {
            if rest.starts_with("Unreleased]") {
                continue;
            }
            if in_sec {
                break;
            }
            if let Some(ver) = version {
                if let Some((sec_ver, _)) = rest.split_once(']') {
                    if sec_ver.trim() == ver.trim_start_matches('v') {
                        in_sec = true;
                    }
                }
            } else {
                in_sec = true;
            }
            continue;
        }
        if in_sec && !line.trim().is_empty() {
            out.push(line);
        }
    }
    out.join("\n")
}

/// Разбирает строки списка изменений (с настоящими `\n` или экранированными `\n`),
/// убирает маркеры `- ` / `* ` и возвращает до 8 пунктов.
pub fn parse_changelog_notes(raw: &str) -> Vec<String> {
    let normalized = raw.replace(r"\n", "\n");
    normalized
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            l.strip_prefix("- ")
                .or_else(|| l.strip_prefix("* "))
                .or_else(|| l.strip_prefix("• "))
                .unwrap_or(l)
                .trim()
                .to_string()
        })
        .filter(|l| !l.is_empty())
        .take(8)
        .collect()
}

/// Пункты на одном языке из секции, разделённой на подразделы `### RU` и `### EN`.
/// Секция без таких подразделов (старые релизы) — одноязычная русская: для английского
/// интерфейса каждый пункт проходит через словарь `i18n::tr`, непереведённые остаются как есть.
pub fn parse_changelog_notes_lang(section_md: &str, en: bool) -> Vec<String> {
    let normalized = section_md.replace(r"\n", "\n");
    let lang_of = |l: &str| match l.trim().to_ascii_uppercase().as_str() {
        "### RU" => Some(false),
        "### EN" => Some(true),
        _ => None,
    };
    if !normalized.lines().any(|l| lang_of(l).is_some()) {
        let notes = parse_changelog_notes(&normalized);
        return if en { notes.iter().map(|n| crate::i18n::tr(n).into_owned()).collect() } else { notes };
    }
    let mut block = String::new();
    let mut inside = false;
    for line in normalized.lines() {
        if let Some(is_en) = lang_of(line) {
            inside = is_en == en;
        } else if line.trim_start().starts_with("## ") {
            inside = false;
        } else if inside {
            block.push_str(line);
            block.push('\n');
        }
    }
    parse_changelog_notes(&block)
}

/// Есть ли непросмотренный список «Что нового» для текущей установленной версии.
pub fn has_unseen_notes() -> bool {
    crate::state::last_seen_version() != VERSION && !parse_changelog_notes_lang(CURRENT_CHANGELOG, false).is_empty()
}

/// Отмечает список «Что нового» текущей версии как просмотренный.
pub fn mark_notes_seen() {
    if crate::state::last_seen_version() != VERSION {
        crate::state::set_last_seen_version(VERSION);
    }
}

/// Возвращает `(версия, пункты)` для блока «Что нового в версии X.Y.Z»:
/// при наличии доступного обновления с непустыми заметками — заметки новой версии,
/// иначе — вшитые заметки текущей установленной версии.
pub fn notes_to_show() -> Option<(String, Vec<String>)> {
    let en = crate::i18n::en();
    if let State::Available(r) = state() {
        let notes = if en { r.notes_en } else { r.notes_ru };
        if !notes.is_empty() {
            return Some((r.version, notes));
        }
    }
    let current = parse_changelog_notes_lang(CURRENT_CHANGELOG, en);
    if !current.is_empty() {
        Some((VERSION.to_string(), current))
    } else {
        None
    }
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
    // Тело релиза — секция версии из CHANGELOG.md (CI копирует её без заголовка `## [x.y.z]`).
    let section = online::json_str_multiline(json, "body")
        .map(|b| {
            let sec = extract_changelog_section(&b, Some(&version));
            if sec.is_empty() {
                b
            } else {
                sec
            }
        })
        .unwrap_or_default();
    let notes_ru = parse_changelog_notes_lang(&section, false);
    let notes_en = parse_changelog_notes_lang(&section, true);
    Some(Release { version, setup_url, sums_url, notes_ru, notes_en })
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
    sys::sha256(data).map(|out| out.iter().map(|b| format!("{b:02x}")).collect())
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
            r###"{{"tag_name":"v0.4.0","body":"## [0.4.0]\r\n- Первый пункт\r\n- Второй пункт","assets":[{{"name":"a","browser_download_url":"https://evil.example/island-0.4.0-setup.exe"}},
            {{"name":"island-0.4.0-setup.exe","browser_download_url":"{d}v0.4.0/island-0.4.0-setup.exe"}},
            {{"name":"SHA256SUMS.txt","browser_download_url":"{d}v0.4.0/SHA256SUMS.txt"}}]}}"###,
            d = DOWNLOADS
        );
        let r = parse_release(&json).unwrap();
        assert_eq!(r.version, "0.4.0");
        assert!(r.setup_url.starts_with(DOWNLOADS) && r.setup_url.ends_with("island-0.4.0-setup.exe"));
        assert!(r.sums_url.ends_with("SHA256SUMS.txt"));
        assert_eq!(r.notes_ru, vec!["Первый пункт".to_string(), "Второй пункт".to_string()]);
        assert!(parse_release(r#"{"tag_name":"v0.4.0","assets":[]}"#).is_none());
    }

    #[test]
    fn changelog_extraction_and_notes_parsing() {
        let md = "# Changelog\n\n## [1.3.0] - 2026-11-11\n\n- Пункт А\n- Пункт Б\n\n## [1.2.0] - 2026-10-01\n\n- Старый пункт\n";
        assert_eq!(extract_changelog_section(md, None), "- Пункт А\n- Пункт Б");
        assert_eq!(extract_changelog_section(md, Some("1.2.0")), "- Старый пункт");
        let parsed = parse_changelog_notes(r"- Пункт 1\n* Пункт 2\n## [1.3.0]");
        assert_eq!(parsed, vec!["Пункт 1".to_string(), "Пункт 2".to_string()]);
        assert!(!parse_changelog_notes_lang(CURRENT_CHANGELOG, false).is_empty());
        assert!(!parse_changelog_notes_lang(CURRENT_CHANGELOG, true).is_empty());
    }

    #[test]
    fn bilingual_notes_pick_language_block() {
        // Так тело релиза выглядит после CI: секция без заголовка версии, строки через `\r\n`.
        let body = "### RU\r\n\r\n- Пункт один\r\n- Пункт два\r\n\r\n### EN\r\n\r\n- Item one\r\n- Item two\r\n";
        assert_eq!(parse_changelog_notes_lang(body, false), ["Пункт один", "Пункт два"]);
        assert_eq!(parse_changelog_notes_lang(body, true), ["Item one", "Item two"]);
        // Вшитый в exe вид: экранированные переводы строк из build.rs.
        let embedded = r"### EN\n- Only English\n### RU\n- Только русский";
        assert_eq!(parse_changelog_notes_lang(embedded, true), ["Only English"]);
        assert_eq!(parse_changelog_notes_lang(embedded, false), ["Только русский"]);
        // Старый одноязычный релиз: русский текст как есть, подзаголовки разделов не попадают в пункты.
        let legacy = "### Добавлено\n- Пункт\n### Изменено\n- Другой";
        assert_eq!(parse_changelog_notes_lang(legacy, false), ["Пункт", "Другой"]);
        assert_eq!(parse_changelog_notes_lang(legacy, true).len(), 2);
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
