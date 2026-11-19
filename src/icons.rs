//! Единый кэш и извлечение иконок приложений для вкладки Apps, виджета «Топ приложений» и
//! уведомлений Windows.
//!
//! Иконки хранятся в едином формате (`SIZE_PX`×`SIZE_PX` premultiplied BGRA) в `Arc<Vec<u8>>`
//! и кэшируются на диске в `%APPDATA%\island\icons.bin`. Предзагрузка запускается сразу при
//! старте острова на коротком фоновом потоке `island-icons`; известные записи читаются из кэша
//! без повторных вызовов Shell/AppX.

#[cfg(target_os = "macos")]
use std::collections::HashSet;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use windows::core::Interface;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapDecoder, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapInterpolationModeHighQualityCubic,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

use crate::{bg, log, sys};

pub const SIZE_PX: u32 = 48;
const PX_BYTES: usize = (SIZE_PX * SIZE_PX * 4) as usize;
const CACHE_FILE: &str = "icons.bin";
const CACHE_MAGIC: &[u8; 4] = b"ISLC";
const CACHE_VERSION: u32 = 1;
const MAX_DISK_ENTRIES: usize = 512;
const MAX_UNKNOWN_RAM: usize = 64;

pub struct Uwp {
    pub name: String,
    /// `shell:AppsFolder\<AUMID>` — запускается через `ShellExecute`.
    pub target: String,
    pub px: Arc<Vec<u8>>,
}

pub enum Found {
    /// Иконка для записи с таким путём.
    Icon(String, Arc<Vec<u8>>),
    #[allow(dead_code)]
    Uwp(Uwp),
    /// Ярлык с таким путём ведёт в никуда (приложение удалено) — из списка его надо убрать.
    Dead(String),
}

/// Цель запуска и фокусировки приложения, собранная при индексации ярлыков и `AppsFolder`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppLaunchTarget {
    /// Путь к ярлыку `.lnk` в меню «Пуск».
    pub path: Option<String>,
    /// Целевой `.exe` ярлыка (если не является общим лаунчером вроде `Update.exe`).
    pub exe: Option<String>,
    /// Оригинальный AUMID из `shell:AppsFolder` или свойства ярлыка.
    pub aumid: Option<String>,
}

struct CachedEntry {
    stamp: u64,
    last_used: u64,
    px: Arc<Vec<u8>>,
}

#[derive(Default)]
struct IconStore {
    loaded: bool,
    dirty: bool,
    seq: u64,
    entries: HashMap<String, CachedEntry>,
    /// Временные алиасы текущей сессии (`name:...`, `stem:...`, `aumid:...` → канонический ключ в `entries`).
    aliases: HashMap<String, String>,
    /// Зарегистрированные в `shell:AppsFolder` элементы: `aumid_lower` → оригинальный parsing-name.
    apps_folder: HashMap<String, String>,
    /// Цели запуска по точному AUMID (`aumid_lower`).
    launch_by_aumid: HashMap<String, AppLaunchTarget>,
    /// Цели запуска по отображаемому имени ярлыка/приложения (`name_lower`).
    launch_by_name: HashMap<String, AppLaunchTarget>,
    /// Цели запуска по имени исполняемого файла без `.exe` (`stem_lower`).
    launch_by_stem: HashMap<String, AppLaunchTarget>,
    /// Кэш иконок уведомлений без `app_id` (`windows.unknown`), на диск не пишется.
    unknown: HashMap<String, Arc<Vec<u8>>>,
    /// Неудачные поиски иконок уведомлений: ключ → `PRELOAD_GEN`, при котором была попытка.
    neg: HashMap<String, u32>,
}

impl IconStore {
    fn next_seq(&mut self) -> u64 {
        self.seq = self.seq.saturating_add(1);
        self.seq
    }

    fn insert_entry(&mut self, key: String, stamp: u64, px: Arc<Vec<u8>>) {
        let lu = self.next_seq();
        self.entries.insert(key, CachedEntry { stamp, last_used: lu, px });
        self.dirty = true;
    }

    fn get_by_key(&mut self, key: &str) -> Option<Arc<Vec<u8>>> {
        let lu = self.next_seq();
        let e = self.entries.get_mut(key)?;
        e.last_used = lu;
        Some(e.px.clone())
    }
}

static GEN: AtomicU32 = AtomicU32::new(0);
/// Увеличивается после завершения фоновой индексации, чтобы уведомления перепроверили новые алиасы.
static PRELOAD_GEN: AtomicU32 = AtomicU32::new(0);
static READY: Mutex<Vec<(u32, Found)>> = Mutex::new(Vec::new());
static STORE: OnceLock<Mutex<IconStore>> = OnceLock::new();

fn store() -> &'static Mutex<IconStore> {
    STORE.get_or_init(|| Mutex::new(IconStore::default()))
}

/// Ключ для `BitmapCache` с префиксом области `tag` (предотвращает коллизии между вкладками и тостом).
pub fn bitmap_key(tag: u8, primary: &str, secondary: &str) -> u64 {
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    if primary.is_empty() || primary == "windows.unknown" {
        primary.hash(&mut hs);
        secondary.hash(&mut hs);
    } else {
        primary.hash(&mut hs);
    }
    ((tag as u64) << 56) | (hs.finish() >> 8)
}

fn cache_file_path() -> PathBuf {
    sys::data_dir().join(CACHE_FILE)
}

fn path_key(path: &str) -> String {
    format!("path:{}", path.trim().replace('/', "\\").to_lowercase())
}

fn aumid_key(aumid: &str) -> String {
    format!("aumid:{}", aumid.trim().to_lowercase())
}

fn file_stamp(path: &str) -> u64 {
    let Ok(meta) = std::fs::metadata(path) else { return 0 };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    (mtime << 20) ^ meta.len()
}

fn is_generic_stem(stem: &str) -> bool {
    matches!(
        stem,
        "update"
            | "updater"
            | "launcher"
            | "cmd"
            | "powershell"
            | "pwsh"
            | "conhost"
            | "wscript"
            | "cscript"
            | "rundll32"
            | "msiexec"
            | "javaw"
            | "java"
            | "python"
            | "pythonw"
            | "setup"
            | "install"
            | "uninstall"
            | "helper"
            | "service"
            | "host"
            | "svchost"
            | "explorer"
    )
}

#[cfg_attr(not(test), allow(dead_code))]
fn is_generic_app_token(tok: &str) -> bool {
    is_generic_stem(tok)
        || matches!(
            tok,
            "desktop" | "windows" | "microsoft" | "client" | "portable" | "beta" | "canary" | "edition" | "app" | "browser"
        )
}

fn merge_launch_target(dst: &mut AppLaunchTarget, src: &AppLaunchTarget) {
    if dst.path.is_none() {
        dst.path = src.path.clone();
    }
    if dst.exe.is_none() {
        dst.exe = src.exe.clone();
    }
    if dst.aumid.is_none() {
        dst.aumid = src.aumid.clone();
    }
}

fn register_launch_target(
    s: &mut IconStore,
    name: Option<&str>,
    aumid: Option<&str>,
    stem: Option<&str>,
    target: AppLaunchTarget,
) {
    if let Some(id) = aumid.map(str::trim).filter(|id| !id.is_empty()) {
        let entry = s.launch_by_aumid.entry(id.to_lowercase()).or_default();
        merge_launch_target(entry, &target);
    }
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
        let entry = s.launch_by_name.entry(n.to_lowercase()).or_default();
        merge_launch_target(entry, &target);
    }
    if let Some(st) = stem.map(str::trim).filter(|st| !st.is_empty()) {
        let low = st.to_lowercase();
        if !is_generic_stem(&low) {
            let entry = s.launch_by_stem.entry(low).or_default();
            merge_launch_target(entry, &target);
        }
    }
}

/// Извлекает вероятные имена процесса/приложения (без `.exe`) из `app_id` и `app_name`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn app_candidate_stems(app_id: &str, app_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |raw: &str| {
        let s = raw.trim().to_lowercase();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };

    let nlow = app_name.trim().to_lowercase();
    if !nlow.is_empty() {
        if !is_generic_app_token(&nlow) {
            push(&nlow);
        }
        for tok in nlow.split(|c: char| !c.is_alphanumeric()) {
            let min_ok = tok.chars().count() >= 4 || tok == "vk";
            if min_ok && !is_generic_app_token(tok) {
                push(tok);
            }
        }
    }

    if !app_id.is_empty() && app_id != "windows.unknown" {
        let before_bang = app_id.split('!').next().unwrap_or(app_id);
        let base = before_bang.rsplit(['\\', '/']).next().unwrap_or(before_bang);
        let base_no_ext = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
        let prefix = base_no_ext.split("._").next().unwrap_or(base_no_ext);
        for tok in prefix.split(['.', '-', '_']) {
            let low = tok.trim().to_lowercase();
            let trimmed = low.strip_suffix("toast").unwrap_or(&low);
            let is_hex_hash = trimmed.len() >= 16 && trimmed.chars().all(|c| c.is_ascii_hexdigit());
            let min_ok = trimmed.chars().count() >= 3 || trimmed == "vk";
            if min_ok && !is_hex_hash && !is_generic_app_token(trimmed) {
                push(trimmed);
            }
        }
        let low_id = app_id.to_lowercase();
        if low_id.contains("yandex") || nlow.contains("яндекс") || nlow.contains("yandex") {
            push("browser");
        }
        if low_id.contains("vk") || nlow.contains("вк") {
            push("vk");
            push("vkmessenger");
        }
        if low_id.contains("teams") || nlow.contains("teams") {
            push("ms-teams");
            push("teams");
        }
    }

    out
}

/// Подбирает сохранённые пути запуска (`.lnk`, `.exe`, `shell:AppsFolder\<AUMID>`) для приложения уведомления.
#[allow(dead_code)]
pub fn launch_target(app_id: &str, app_name: &str) -> AppLaunchTarget {
    let Ok(s) = store().lock() else { return AppLaunchTarget::default() };
    launch_target_in(&s, app_id, app_name)
}

#[cfg_attr(not(test), allow(dead_code))]
fn launch_target_in(s: &IconStore, app_id: &str, app_name: &str) -> AppLaunchTarget {
    let mut out = AppLaunchTarget::default();
    if !app_id.is_empty() && app_id != "windows.unknown" {
        let id_low = app_id.trim().to_lowercase();
        if let Some(t) = s.launch_by_aumid.get(&id_low) {
            merge_launch_target(&mut out, t);
        }
        if out.aumid.is_none() {
            if let Some(af) = s.apps_folder.get(&id_low) {
                out.aumid = Some(af.clone());
            }
        }
        // Для PWA из Chrome/Edge (`Chrome._crx_...`) не подставляем общий ярлык браузера,
        // если у приложения есть собственный ярлык/AUMID (или их ещё нет в индексе — пусть
        // сначала сработает поиск окна/процесса).
        let is_crx_pwa = id_low.starts_with("chrome._crx_") || id_low.starts_with("msedge._crx_");
        if is_crx_pwa && (out.path.is_some() || out.aumid.is_some()) {
            return out;
        }
    }

    let name_low = app_name.trim().to_lowercase();
    if !name_low.is_empty() {
        if let Some(t) = s.launch_by_name.get(&name_low) {
            merge_launch_target(&mut out, t);
        }
        if let Some(t) = s.launch_by_stem.get(&name_low) {
            merge_launch_target(&mut out, t);
        }
    }

    for cand in app_candidate_stems(app_id, app_name) {
        if let Some(t) = s.launch_by_stem.get(&cand) {
            merge_launch_target(&mut out, t);
        }
        if let Some(t) = s.launch_by_name.get(&cand) {
            merge_launch_target(&mut out, t);
        }
    }
    out
}

fn register_aliases(
    s: &mut IconStore,
    canonical: &str,
    name: Option<&str>,
    aumid: Option<&str>,
    stem: Option<&str>,
) {
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
        s.aliases.insert(format!("name:{}", n.to_lowercase()), canonical.to_string());
    }
    if let Some(id) = aumid.map(str::trim).filter(|id| !id.is_empty()) {
        s.aliases.insert(aumid_key(id), canonical.to_string());
    }
    if let Some(st) = stem.map(str::trim).filter(|st| !st.is_empty()) {
        let low = st.to_lowercase();
        if !is_generic_stem(&low) {
            s.aliases.entry(format!("stem:{low}")).or_insert_with(|| canonical.to_string());
        }
    }
}

fn encode_cache(entries: &HashMap<String, CachedEntry>) -> Vec<u8> {
    let mut ordered: Vec<(&str, &CachedEntry)> = entries
        .iter()
        .filter(|(_, e)| e.px.len() == PX_BYTES)
        .map(|(k, e)| (k.as_str(), e))
        .collect();
    if ordered.len() > MAX_DISK_ENTRIES {
        ordered.sort_by_key(|(_, e)| std::cmp::Reverse(e.last_used));
        ordered.truncate(MAX_DISK_ENTRIES);
    }

    let total: usize = 16 + ordered.iter().map(|(k, _)| 4 + k.len() + 16 + PX_BYTES).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(CACHE_MAGIC);
    out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    out.extend_from_slice(&SIZE_PX.to_le_bytes());
    out.extend_from_slice(&(ordered.len() as u32).to_le_bytes());
    for (k, e) in ordered {
        out.extend_from_slice(&(k.len() as u32).to_le_bytes());
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&e.stamp.to_le_bytes());
        out.extend_from_slice(&e.last_used.to_le_bytes());
        out.extend_from_slice(&e.px);
    }
    out
}

fn decode_cache(bytes: &[u8]) -> Vec<(String, u64, u64, Arc<Vec<u8>>)> {
    if bytes.len() < 16 || &bytes[..4] != CACHE_MAGIC {
        return Vec::new();
    }
    let ver = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4]));
    let px = u32::from_le_bytes(bytes[8..12].try_into().unwrap_or([0; 4]));
    let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap_or([0; 4])) as usize;
    if ver != CACHE_VERSION || px != SIZE_PX {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(count.min(MAX_DISK_ENTRIES));
    let mut pos = 16usize;
    for _ in 0..count.min(4096) {
        let Some(klen_b) = bytes.get(pos..pos + 4) else { break };
        let klen = u32::from_le_bytes(klen_b.try_into().unwrap_or([0; 4])) as usize;
        pos += 4;
        if klen == 0 || klen > 2048 {
            break;
        }
        let Some(k_b) = bytes.get(pos..pos + klen) else { break };
        pos += klen;
        let Ok(key) = std::str::from_utf8(k_b) else { break };
        let Some(meta_b) = bytes.get(pos..pos + 16) else { break };
        pos += 16;
        let stamp = u64::from_le_bytes(meta_b[..8].try_into().unwrap_or([0; 8]));
        let last_used = u64::from_le_bytes(meta_b[8..16].try_into().unwrap_or([0; 8]));
        let Some(px_b) = bytes.get(pos..pos + PX_BYTES) else { break };
        pos += PX_BYTES;
        out.push((key.to_string(), stamp, last_used, Arc::new(px_b.to_vec())));
    }
    out
}

fn load_cache_from(s: &mut IconStore, path: &Path) {
    s.loaded = true;
    let Ok(bytes) = std::fs::read(path) else { return };
    let mut max_seq = 0u64;
    for (k, stamp, last_used, px) in decode_cache(&bytes) {
        max_seq = max_seq.max(last_used);
        s.entries.insert(k, CachedEntry { stamp, last_used, px });
    }
    s.seq = s.seq.max(max_seq);
}

fn save_cache_to(path: &Path, data: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("bin.tmp");
    let res = std::fs::write(&tmp, data).and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        log::error(&format!("не удалось сохранить кэш иконок: {e}"));
    }
}

fn ensure_loaded() {
    let Ok(mut s) = store().lock() else { return };
    if !s.loaded {
        load_cache_from(&mut s, &cache_file_path());
    }
}

/// Записывает накопленные изменения кэша иконок на диск (если кэш менялся).
pub fn flush_cache() {
    let payload = {
        let Ok(mut s) = store().lock() else { return };
        if !s.dirty {
            return;
        }
        s.dirty = false;
        encode_cache(&s.entries)
    };
    save_cache_to(&cache_file_path(), &payload);
}

/// Запускает загрузку иконок для `paths` (ярлыки) и перечисление UWP-приложений.
/// Сначала берёт готовые иконки из дискового кэша, из Shell извлекает только новые/изменённые.
pub fn request(paths: Vec<String>) -> u32 {
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new().name("island-icons".into()).spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        ensure_loaded();
        let push = |f: Found| {
            if let Ok(mut r) = READY.lock() {
                r.push((gen, f));
            }
        };
        for p in paths {
            if GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let lower = p.to_lowercase();
            let (dead, target, lnk_aumid) = if lower.ends_with(".url") {
                (std::fs::read(&p).is_ok_and(|b| url_dead(&String::from_utf8_lossy(&b))), None, None)
            } else if lower.ends_with(".lnk") {
                let (tgt, aumid) = lnk_info(&p);
                let dead = tgt
                    .as_deref()
                    .is_some_and(|t| !t.is_empty() && !t.starts_with("\\\\") && !Path::new(t).exists());
                (dead, tgt, aumid)
            } else {
                (false, None, None)
            };

            let key = path_key(&p);
            if dead {
                if let Ok(mut s) = store().lock() {
                    if s.entries.remove(&key).is_some() {
                        s.dirty = true;
                    }
                }
                push(Found::Dead(p));
                continue;
            }

            let stamp = file_stamp(&p);
            let cached = store().lock().ok().and_then(|mut s| {
                if stamp != 0 && s.entries.get(&key).is_some_and(|e| e.stamp == stamp) {
                    s.get_by_key(&key)
                } else {
                    None
                }
            });

            let px = match cached {
                Some(px) => Some(px),
                None => path_icon(&p).map(Arc::new).inspect(|px| {
                    if let Ok(mut s) = store().lock() {
                        s.insert_entry(key.clone(), stamp, px.clone());
                    }
                }),
            };

            if !dead && lower.ends_with(".lnk") {
                let name = Path::new(&p).file_stem().and_then(|s| s.to_str());
                let exe_stem = target
                    .as_deref()
                    .and_then(|t| Path::new(t).file_stem().and_then(|s| s.to_str()));
                let exe = target.as_deref().filter(|t| {
                    let is_exe = t.to_ascii_lowercase().ends_with(".exe");
                    let stem_ok = Path::new(t)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|st| !is_generic_stem(&st.to_lowercase()));
                    is_exe && stem_ok
                });
                if let Ok(mut s) = store().lock() {
                    register_launch_target(
                        &mut s,
                        name,
                        lnk_aumid.as_deref(),
                        exe_stem,
                        AppLaunchTarget {
                            path: Some(p.clone()),
                            exe: exe.map(str::to_string),
                            aumid: lnk_aumid.clone(),
                        },
                    );
                }
            }

            if let Some(px) = px {
                let name = Path::new(&p).file_stem().and_then(|s| s.to_str());
                let exe_stem = target
                    .as_deref()
                    .and_then(|t| Path::new(t).file_stem().and_then(|s| s.to_str()));
                if let Ok(mut s) = store().lock() {
                    register_aliases(&mut s, &key, name, lnk_aumid.as_deref(), exe_stem);
                }
                push(Found::Icon(p, px));
            }
        }
        bg::notify();
        enumerate_apps_folder(gen, &push);
        flush_cache();
        PRELOAD_GEN.fetch_add(1, Ordering::SeqCst);
        bg::notify();
        bg::send(bg::Cmd::NotifRefresh);
        // Shell и AppX подтянули ~55 МБ библиотек и куч, которые потоку больше не нужны.
        sys::trim_working_set();
    });
    if let Err(e) = spawned {
        log::error(&format!("не удалось запустить поток иконок: {e}"));
    }
    gen
}

pub fn drain(gen: u32) -> Vec<Found> {
    let Ok(mut r) = READY.lock() else { return Vec::new() };
    std::mem::take(&mut *r).into_iter().filter(|(g, _)| *g == gen).map(|(_, f)| f).collect()
}

#[allow(unused_variables)]
unsafe fn lnk_info(_path: &str) -> (Option<String>, Option<String>) {
    (None, None)
}

/// `.url` мёртв, если ведёт на `file:` без файла либо на нестандартный протокол (`steam://`,
/// `goggalaxy://`…), а его иконка лежит по абсолютному пути, которого уже нет: так выглядят
/// удалённые игры, оставившие ярлык в меню «Пуск».
fn url_dead(text: &str) -> bool {
    let (mut url, mut icon) = ("", "");
    for l in text.lines() {
        let l = l.trim();
        if let Some(v) = l.strip_prefix("URL=") {
            url = v.trim();
        } else if let Some(v) = l.strip_prefix("IconFile=") {
            icon = v.trim().trim_matches('"');
        }
    }
    let low = url.to_ascii_lowercase();
    let absolute = |p: &str| p.len() > 2 && p.as_bytes()[1] == b':';
    if let Some(rest) = low.strip_prefix("file:") {
        let rest = rest.trim_start_matches('/');
        return !rest.contains('%') && absolute(rest) && !Path::new(rest).exists();
    }
    let web = ["http:", "https:", "ftp:"].iter().any(|s| low.starts_with(s));
    !url.is_empty() && !web && absolute(icon) && !Path::new(icon).exists()
}

/// Иконка по пути (`.app` бандл на macOS или файл изображения).
pub unsafe fn path_icon(path: &str) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        if path.ends_with(".app") || path.contains(".app/") {
            let app_root = path
                .split(".app")
                .next()
                .map(|p| PathBuf::from(format!("{p}.app")))?;
            let res_dir = app_root.join("Contents").join("Resources");
            if let Ok(rd) = std::fs::read_dir(&res_dir) {
                let mut icns: Option<PathBuf> = None;
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("icns")) {
                        if p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("AppIcon")) {
                            icns = Some(p);
                            break;
                        }
                        if icns.is_none() {
                            icns = Some(p);
                        }
                    }
                }
                if let Some(icns_path) = icns {
                    let tmp = std::env::temp_dir().join(format!("island-icns-{}.png", std::process::id()));
                    if std::process::Command::new("/usr/bin/sips")
                        .args(["-s", "format", "png", "-z", "48", "48"])
                        .arg(&icns_path)
                        .arg("--out")
                        .arg(&tmp)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .is_ok_and(|s| s.success())
                    {
                        let px = std::fs::read(&tmp).ok().and_then(|b| wic_bytes_to_px(&b));
                        let _ = std::fs::remove_file(&tmp);
                        if px.is_some() {
                            return px;
                        }
                    }
                }
            }
        }
        wic_file_to_px(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

#[cfg(target_os = "macos")]
pub unsafe fn exe_icon(path: &str, display_name: Option<&str>) -> Option<Arc<Vec<u8>>> {
    ensure_loaded();
    let key = path_key(path);
    let stamp = file_stamp(path);
    let stem = Path::new(path).file_stem().and_then(|s| s.to_str());
    if let Ok(mut s) = store().lock() {
        if s.entries.get(&key).is_some_and(|e| stamp == 0 || e.stamp == stamp) {
            if let Some(px) = s.get_by_key(&key) {
                register_aliases(&mut s, &key, display_name, None, stem);
                return Some(px);
            }
        }
    }
    let px = Arc::new(path_icon(path)?);
    if let Ok(mut s) = store().lock() {
        s.insert_entry(key.clone(), stamp, px.clone());
        register_aliases(&mut s, &key, display_name, None, stem);
    }
    Some(px)
}

#[allow(dead_code)]
pub unsafe fn resolve_indirect(_raw: &str) -> Option<String> {
    None
}

pub unsafe fn reg_app_name(_app_id: &str) -> Option<String> {
    None
}

/// Подбирает и кэширует иконку приложения для системного уведомления:
/// 1) общий кэш / алиасы; 2) `shell:AppsFolder\<app_id>` (UWP и Win32 с AUMID);
/// 3) реестр `AppUserModelId\IconUri`; 4) `AppInfo::DisplayInfo().GetLogo` (`winrt_bytes`);
/// 5) путь, имя приложения в индексе ярлыков/UWP/процессов, либо запущенный `.exe`.
pub unsafe fn resolve_notif_icon(
    app_id: &str,
    app_name: &str,
    winrt_bytes: impl FnOnce() -> Option<Vec<u8>>,
) -> Option<Arc<Vec<u8>>> {
    ensure_loaded();
    let pgen = PRELOAD_GEN.load(Ordering::SeqCst);
    let unknown_id = app_id.trim().is_empty() || app_id == "windows.unknown";
    let neg_key = if unknown_id {
        format!("unk:{}", app_name.trim().to_lowercase())
    } else {
        aumid_key(app_id)
    };

    let exact_af = {
        let Ok(mut s) = store().lock() else { return None };
        if !unknown_id {
            if let Some(px) = s.get_by_key(&neg_key) {
                return Some(px);
            }
            if let Some(canon) = s.aliases.get(&neg_key).cloned() {
                if let Some(px) = s.get_by_key(&canon) {
                    return Some(px);
                }
            }
        } else if let Some(px) = s.unknown.get(&app_name.trim().to_lowercase()) {
            return Some(px.clone());
        }
        if s.neg.get(&neg_key).copied() == Some(pgen) {
            return None;
        }
        s.apps_folder.get(&app_id.to_lowercase()).cloned()
    };

    let mut extracted: Option<Arc<Vec<u8>>> = None;

    if !unknown_id {
        let af_id = exact_af.as_deref().unwrap_or(app_id);
        extracted = path_icon(&format!("shell:AppsFolder\\{af_id}")).map(Arc::new);
    }

    if extracted.is_none() && !unknown_id {
        let subkey = format!(r"Software\Classes\AppUserModelId\{app_id}");
        if let Some(uri) = sys::reg_str(false, &subkey, "IconUri").or_else(|| sys::reg_str(true, &subkey, "IconUri")) {
            extracted = icon_from_uri(&uri).map(Arc::new);
        }
    }

    if extracted.is_none() {
        if let Some(bytes) = winrt_bytes() {
            extracted = wic_bytes_to_px(&bytes).map(Arc::new);
        }
    }

    if extracted.is_none() && !unknown_id && (app_id.contains('\\') || app_id.contains('/') || app_id.contains(':')) {
        extracted = path_icon(app_id).map(Arc::new);
    }

    if extracted.is_none() {
        if let Ok(mut s) = store().lock() {
            extracted = match_store_alias(&mut s, app_id, app_name);
        }
    }

    if extracted.is_none() {
        extracted = running_process_icon(app_id, app_name);
    }

    let mut need_flush = false;
    {
        let Ok(mut s) = store().lock() else { return extracted };
        match &extracted {
            Some(px) => {
                s.neg.remove(&neg_key);
                if !unknown_id {
                    s.insert_entry(neg_key, 0, px.clone());
                    need_flush = true;
                } else {
                    if s.unknown.len() >= MAX_UNKNOWN_RAM {
                        s.unknown.clear();
                    }
                    s.unknown.insert(app_name.trim().to_lowercase(), px.clone());
                }
            }
            None => {
                s.neg.insert(neg_key, pgen);
            }
        }
    }
    if need_flush {
        flush_cache();
    }
    extracted
}

fn match_store_alias(s: &mut IconStore, app_id: &str, app_name: &str) -> Option<Arc<Vec<u8>>> {
    let mut candidates: Vec<String> = Vec::new();
    let name_low = app_name.trim().to_lowercase();
    if !name_low.is_empty() {
        candidates.push(format!("name:{name_low}"));
        candidates.push(format!("stem:{name_low}"));
        for tok in name_low.split(|c: char| !c.is_alphanumeric()) {
            if tok.chars().count() >= 4 && !is_generic_stem(tok) {
                candidates.push(format!("name:{tok}"));
                candidates.push(format!("stem:{tok}"));
            }
        }
    }
    if !app_id.is_empty() && app_id != "windows.unknown" {
        let base = app_id.split(['!', '\\', '/']).next().unwrap_or(app_id);
        let prefix = base.split("._").next().unwrap_or(base).split('-').next().unwrap_or(base).trim().to_lowercase();
        if prefix.chars().count() >= 3 && !is_generic_stem(&prefix) {
            candidates.push(format!("name:{prefix}"));
            candidates.push(format!("stem:{prefix}"));
        }
    }
    for alias in candidates {
        if let Some(canon) = s.aliases.get(&alias).cloned() {
            if let Some(px) = s.get_by_key(&canon) {
                return Some(px);
            }
        }
    }
    None
}

unsafe fn running_process_icon(_app_id: &str, _app_name: &str) -> Option<Arc<Vec<u8>>> {
    None
}

unsafe fn icon_from_uri(raw: &str) -> Option<Vec<u8>> {
    let s = raw.trim();
    let p = s.strip_prefix("file://").unwrap_or(s);
    if Path::new(p).exists() {
        return wic_file_to_px(p).or_else(|| path_icon(p));
    }
    None
}

#[allow(dead_code)]
unsafe fn wic_file_to_px(path: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    wic_bytes_to_px(&bytes)
}

/// Декодирует изображение (PNG/ICO/BMP/JPEG) из памяти в `SIZE_PX`×`SIZE_PX` BGRA premultiplied
/// с сохранением пропорций и выбором лучшего кадра для многокадровых `.ico`.
pub unsafe fn wic_bytes_to_px(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.is_empty() {
        return None;
    }
    let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let ws = factory.CreateStream().ok()?;
    ws.InitializeFromMemory(bytes).ok()?;
    let dec: IWICBitmapDecoder = factory
        .CreateDecoderFromStream(&ws, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
        .ok()?;
    let count = dec.GetFrameCount().ok().unwrap_or(1).max(1);
    let mut best_idx = 0u32;
    let mut best_score = (true, u32::MAX);
    if count > 1 {
        for i in 0..count {
            let Ok(fr) = dec.GetFrame(i) else { continue };
            let (mut w, mut h) = (0u32, 0u32);
            if fr.GetSize(&mut w, &mut h).is_err() || w == 0 || h == 0 {
                continue;
            }
            let side = w.min(h);
            let score = if side >= SIZE_PX { (false, side) } else { (true, u32::MAX - side) };
            if score < best_score {
                best_score = score;
                best_idx = i;
            }
        }
    }
    let frame = dec.GetFrame(best_idx).ok()?;
    let src: IWICBitmapSource = frame.cast().ok()?;
    wic_source_to_px(&factory, &src)
}

unsafe fn wic_source_to_px(factory: &IWICImagingFactory, src: &IWICBitmapSource) -> Option<Vec<u8>> {
    let (mut w, mut h) = (0u32, 0u32);
    src.GetSize(&mut w, &mut h).ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    let k = (SIZE_PX as f32 / w as f32).min(SIZE_PX as f32 / h as f32);
    let nw = ((w as f32 * k).round() as u32).clamp(1, SIZE_PX);
    let nh = ((h as f32 * k).round() as u32).clamp(1, SIZE_PX);
    let mode = if k < 1.0 {
        WICBitmapInterpolationModeFant
    } else {
        WICBitmapInterpolationModeHighQualityCubic
    };
    let scaler = factory.CreateBitmapScaler().ok()?;
    scaler.Initialize(src, nw, nh, mode).ok()?;
    let conv = factory.CreateFormatConverter().ok()?;
    conv.Initialize(
        &scaler,
        &GUID_WICPixelFormat32bppPBGRA,
        WICBitmapDitherTypeNone,
        None,
        0.0,
        WICBitmapPaletteTypeCustom,
    )
    .ok()?;

    if nw == SIZE_PX && nh == SIZE_PX {
        let mut buf = vec![0u8; PX_BYTES];
        conv.CopyPixels(std::ptr::null(), SIZE_PX * 4, &mut buf).ok()?;
        return Some(buf);
    }

    let row_bytes = (nw * 4) as usize;
    let mut small = vec![0u8; row_bytes * nh as usize];
    conv.CopyPixels(std::ptr::null(), nw * 4, &mut small).ok()?;
    let mut out = vec![0u8; PX_BYTES];
    let ox = ((SIZE_PX - nw) / 2) as usize;
    let oy = ((SIZE_PX - nh) / 2) as usize;
    for y in 0..nh as usize {
        let src_off = y * row_bytes;
        let dst_off = ((oy + y) * SIZE_PX as usize + ox) * 4;
        out[dst_off..dst_off + row_bytes].copy_from_slice(&small[src_off..src_off + row_bytes]);
    }
    Some(out)
}

#[allow(dead_code)]
fn normalize(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let n = SIZE_PX as usize;
    let opaque = src.chunks_exact(4).all(|p| p[3] == 0);
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        let (y0, y1) = (y * h / n, ((y + 1) * h / n).max(y * h / n + 1).min(h));
        for x in 0..n {
            let (x0, x1) = (x * w / n, ((x + 1) * w / n).max(x * w / n + 1).min(w));
            let (mut acc, mut cnt) = ([0u32; 4], 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let s = (sy * w + sx) * 4;
                    let a = if opaque { 255 } else { src[s + 3] as u32 };
                    for c in 0..3 {
                        acc[c] += if opaque { src[s + c] as u32 } else { src[s + c] as u32 * a / 255 };
                    }
                    acc[3] += a;
                    cnt += 1;
                }
            }
            let o = (y * n + x) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / cnt.max(1)) as u8;
            }
        }
    }
    out
}

/// Сканирует `/Applications`, `/System/Applications` и `~/Applications` на macOS для вкладки Apps.
unsafe fn enumerate_apps_folder(gen: u32, push: &dyn Fn(Found)) {
    #[cfg(target_os = "macos")]
    {
        let mut dirs = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
            PathBuf::from("/System/Applications/Utilities"),
        ];
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join("Applications"));
        }
        let mut seen = HashSet::new();
        for dir in dirs {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for entry in rd.flatten() {
                if GEN.load(Ordering::SeqCst) != gen {
                    return;
                }
                let p = entry.path();
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) {
                    let Some(name) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
                    let target = p.to_string_lossy().into_owned();
                    let key = path_key(&target);
                    if !seen.insert(key.clone()) {
                        continue;
                    }
                    let px = exe_icon(&target, Some(&name)).unwrap_or_else(|| Arc::new(vec![0u8; PX_BYTES]));
                    push(Found::Uwp(Uwp { name, target, px }));
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (gen, push);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Живая проверка (нужен рабочий стол): иконки ярлыков и UWP в сетку PNG.
    #[test]
    #[ignore]
    fn live_dump() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let dir = PathBuf::from(std::env::var("ISLAND_DUMP").unwrap());
            let mut lnks = Vec::new();
            for base in [std::env::var("ProgramData").unwrap(), std::env::var("APPDATA").unwrap()] {
                let d = PathBuf::from(base).join("Microsoft/Windows/Start Menu/Programs");
                if let Ok(rd) = std::fs::read_dir(d) {
                    for e in rd.flatten() {
                        if e.path().extension().is_some_and(|x| x == "lnk") {
                            lnks.push(e.path().display().to_string());
                        }
                    }
                }
            }
            let mut icons: Vec<Vec<u8>> = lnks.iter().take(24).filter_map(|p| path_icon(p)).collect();
            let n_lnk = icons.len();
            let found = Mutex::new(Vec::new());
            enumerate_apps_folder(0, &|f| {
                if let Found::Uwp(u) = f {
                    found.lock().unwrap().push((u.name, u.px));
                }
            });
            let uwp = found.into_inner().unwrap();
            println!("lnk icons: {n_lnk} of {}, uwp: {}", lnks.len(), uwp.len());
            for (n, _) in uwp.iter().take(10) {
                println!("  uwp: {n}");
            }
            icons.extend(uwp.into_iter().take(16).map(|(_, px)| px.as_ref().clone()));
            let cols = 8usize;
            let n = SIZE_PX as usize;
            let rows = icons.len().div_ceil(cols);
            let (w, h) = (cols * n, rows * n);
            // Фон шахматкой, поверх — иконка с учётом premultiplied-альфы.
            let mut out = vec![0u8; w * h * 4];
            for (k, px) in icons.iter().enumerate() {
                let (ox, oy) = ((k % cols) * n, (k / cols) * n);
                for y in 0..n {
                    for x in 0..n {
                        let bg = if (x / 4 + y / 4) % 2 == 0 { 90u32 } else { 40u32 };
                        let s = (y * n + x) * 4;
                        let o = ((oy + y) * w + ox + x) * 4;
                        let a = px[s + 3] as u32;
                        for c in 0..3 {
                            out[o + c] = (px[s + c] as u32 + bg * (255 - a) / 255).min(255) as u8;
                        }
                        out[o + 3] = 255;
                    }
                }
            }
            crate::img::save_png(&dir.join("icons.png"), w as u32, h as u32, &out).expect("png");
        }
    }

    #[test]
    fn url_shortcuts_to_removed_games_are_dead() {
        // Удалённая игра Steam: протокол нестандартный, иконки на диске нет.
        assert!(url_dead("[InternetShortcut]\r\nURL=steam://rungameid/1\r\nIconFile=C:\\no\\such\\dir\\game.ico\r\n"));
        // Иконка на месте (берём существующий файл) — игра считается установленной.
        let exe = std::env::current_exe().unwrap();
        assert!(!url_dead(&format!("URL=steam://rungameid/1\nIconFile={}\n", exe.display())));
        // Обычные ссылки и ссылки без иконки не трогаем.
        assert!(!url_dead("URL=https://example.com\nIconFile=C:\\no\\such.ico\n"));
        assert!(!url_dead("URL=steam://rungameid/1\n"));
        assert!(!url_dead(""));
        // file: на несуществующий файл.
        assert!(url_dead("URL=file:///C:/no/such/file.exe\n"));
        assert!(!url_dead(&format!("URL=file:///{}\n", exe.display().to_string().replace('\\', "/"))));
    }

    #[test]
    fn normalize_premultiplies_and_resizes() {
        // 1×1 полупрозрачный → SIZE_PX² с цветом, умноженным на альфу.
        let px = normalize(&[200, 100, 50, 128], 1, 1);
        assert_eq!(px.len(), PX_BYTES);
        assert_eq!(&px[..4], &[100, 50, 25, 128]);
    }

    #[test]
    fn normalize_without_alpha_is_opaque() {
        let px = normalize(&[10, 20, 30, 0], 1, 1);
        assert_eq!(&px[..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn disk_cache_roundtrip_and_corruption_guard() {
        let mut entries = HashMap::new();
        let px1 = Arc::new(vec![42u8; PX_BYTES]);
        let px2 = Arc::new(vec![7u8; PX_BYTES]);
        entries.insert("path:c:\\a.lnk".to_string(), CachedEntry { stamp: 123, last_used: 10, px: px1.clone() });
        entries.insert("aumid:app!id".to_string(), CachedEntry { stamp: 0, last_used: 20, px: px2.clone() });

        let raw = encode_cache(&entries);
        let decoded = decode_cache(&raw);
        assert_eq!(decoded.len(), 2);
        let map: HashMap<String, (u64, u64, Arc<Vec<u8>>)> =
            decoded.into_iter().map(|(k, s, lu, p)| (k, (s, lu, p))).collect();
        assert_eq!(map["path:c:\\a.lnk"].0, 123);
        assert_eq!(map["path:c:\\a.lnk"].2.as_slice(), px1.as_slice());
        assert_eq!(map["aumid:app!id"].1, 20);
        assert_eq!(map["aumid:app!id"].2.as_slice(), px2.as_slice());

        // Битый заголовок и обрезанный файл не паникуют.
        assert!(decode_cache(b"BAD!").is_empty());
        assert!(decode_cache(&raw[..raw.len() / 2]).len() < 2);
    }

    #[test]
    fn store_alias_matches_name_and_prefix() {
        let mut s = IconStore::default();
        let px = Arc::new(vec![99u8; PX_BYTES]);
        s.insert_entry("path:c:\\firefox.lnk".into(), 1, px.clone());
        register_aliases(&mut s, "path:c:\\firefox.lnk", Some("Firefox"), None, Some("firefox"));

        assert_eq!(match_store_alias(&mut s, "FirefoxToast-308046", "Mozilla Firefox"), Some(px.clone()));
        assert_eq!(match_store_alias(&mut s, "Chrome._crx_abc", "Google Chrome"), None);

        let chrome_px = Arc::new(vec![55u8; PX_BYTES]);
        s.insert_entry("path:c:\\chrome.lnk".into(), 1, chrome_px.clone());
        register_aliases(&mut s, "path:c:\\chrome.lnk", Some("Google Chrome"), None, Some("chrome"));
        assert_eq!(match_store_alias(&mut s, "Chrome._crx_abc", "Site"), Some(chrome_px));
    }

    #[test]
    fn bitmap_key_distinguishes_unknown_apps_and_unifies_known() {
        let k1 = bitmap_key(b'N', "electron.app.Antigravity", "Antigravity");
        let k2 = bitmap_key(b'N', "electron.app.Antigravity", "Other");
        assert_eq!(k1, k2);

        let u1 = bitmap_key(b'N', "windows.unknown", "AppA");
        let u2 = bitmap_key(b'N', "windows.unknown", "AppB");
        assert_ne!(u1, u2);
    }

    #[test]
    fn launch_target_resolves_messengers_and_browsers() {
        let mut s = IconStore::default();
        register_launch_target(
            &mut s,
            Some("Telegram"),
            Some("Telegram.TelegramDesktop.84a2f0c1"),
            Some("Telegram"),
            AppLaunchTarget {
                path: Some(r"C:\Start\Telegram.lnk".into()),
                exe: Some(r"C:\Apps\Telegram\Telegram.exe".into()),
                aumid: Some("Telegram.TelegramDesktop.84a2f0c1".into()),
            },
        );
        register_launch_target(
            &mut s,
            Some("Discord"),
            Some("com.squirrel.Discord.Discord"),
            Some("Update"),
            AppLaunchTarget {
                path: Some(r"C:\Start\Discord.lnk".into()),
                exe: None,
                aumid: Some("com.squirrel.Discord.Discord".into()),
            },
        );
        s.apps_folder.insert(
            "5319275a.whatsappdesktop_cv1g1gvanyjgm!app".into(),
            "5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App".into(),
        );

        let tg = launch_target_in(&s, "Telegram.TelegramDesktop.84a2f0c1", "Telegram Desktop");
        assert_eq!(tg.path.as_deref(), Some(r"C:\Start\Telegram.lnk"));
        assert_eq!(tg.exe.as_deref(), Some(r"C:\Apps\Telegram\Telegram.exe"));

        // Даже если хэш AUMID отличается (переносная установка / обновление), находим по имени/токену.
        let tg_alt = launch_target_in(&s, "Telegram.TelegramDesktop.deadbeefdeadbeef", "Telegram Desktop");
        assert_eq!(tg_alt.path.as_deref(), Some(r"C:\Start\Telegram.lnk"));

        let dc = launch_target_in(&s, "com.squirrel.Discord.Discord", "Discord");
        assert_eq!(dc.path.as_deref(), Some(r"C:\Start\Discord.lnk"));
        assert_eq!(dc.exe, None, "Update.exe не должен запоминаться как exe приложения");

        let wa = launch_target_in(&s, "5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App", "WhatsApp");
        assert_eq!(wa.aumid.as_deref(), Some("5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App"));

        let cands = app_candidate_stems("FirefoxToast-308046B0AF4A39CB", "Mozilla Firefox");
        assert!(cands.contains(&"firefox".to_string()));
        let vk_cands = app_candidate_stems("com.vk.messenger", "VK Messenger");
        assert!(vk_cands.contains(&"vk".to_string()));
    }
}
