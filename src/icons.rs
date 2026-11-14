//! Единый кэш и извлечение иконок приложений для вкладки Apps, виджета «Топ приложений» и
//! уведомлений Windows.
//!
//! Иконки хранятся в едином формате (`SIZE_PX`×`SIZE_PX` premultiplied BGRA) в `Arc<Vec<u8>>`
//! и кэшируются на диске в `%APPDATA%\island\icons.bin`. Предзагрузка запускается сразу при
//! старте острова на коротком фоновом потоке `island-icons`; известные записи читаются из кэша
//! без повторных вызовов Shell/AppX.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY, SIZE};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapDecoder, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapInterpolationModeHighQualityCubic,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    STGM_READ,
};
use windows::Win32::System::ProcessStatus::EnumProcesses;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    ExtractIconExW, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItemImageFactory, IShellLinkW,
    SHCreateItemFromParsingName, SHGetKnownFolderItem, SHLoadIndirectString, ShellLink, BHID_EnumItems,
    KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, PrivateExtractIconsW, HICON};

use crate::{bg, log, sys};

pub const SIZE_PX: u32 = 48;
const PX_BYTES: usize = (SIZE_PX * SIZE_PX * 4) as usize;
const CACHE_FILE: &str = "icons.bin";
const CACHE_MAGIC: &[u8; 4] = b"ISLC";
const CACHE_VERSION: u32 = 1;
const MAX_DISK_ENTRIES: usize = 512;
const MAX_UNKNOWN_RAM: usize = 64;

/// `System.AppUserModel.ID` (`{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, 5`).
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

pub struct Uwp {
    pub name: String,
    /// `shell:AppsFolder\<AUMID>` — запускается через `ShellExecute`.
    pub target: String,
    pub px: Arc<Vec<u8>>,
}

pub enum Found {
    /// Иконка для записи с таким путём.
    Icon(String, Arc<Vec<u8>>),
    Uwp(Uwp),
    /// Ярлык с таким путём ведёт в никуда (приложение удалено) — из списка его надо убрать.
    Dead(String),
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

/// Читает целевой путь и `System.AppUserModel.ID` у ярлыка `.lnk` за одно открытие файла.
unsafe fn lnk_info(path: &str) -> (Option<String>, Option<String>) {
    let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else {
        return (None, None);
    };
    let Ok(file) = link.cast::<IPersistFile>() else {
        return (None, None);
    };
    let wide = sys::wz(path);
    if file.Load(PCWSTR(wide.as_ptr()), STGM_READ).is_err() {
        return (None, None);
    }
    let mut buf = [0u16; 520];
    let target = if link.GetPath(&mut buf, std::ptr::null_mut(), 0).is_ok() {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    } else {
        None
    };
    let aumid = link.cast::<IPropertyStore>().ok().and_then(|ps| {
        let mut pv = ps.GetValue(&PKEY_APP_USER_MODEL_ID).ok()?;
        let val = if pv.Anonymous.Anonymous.vt == VT_LPWSTR {
            pv.Anonymous.Anonymous.Anonymous.pwszVal.to_string().ok()
        } else {
            None
        };
        let _ = PropVariantClear(&mut pv);
        val.filter(|v| !v.trim().is_empty())
    });
    (target, aumid)
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

/// Иконка по пути ярлыка, exe или элемента Shell (включая `shell:AppsFolder\<AUMID>`).
pub unsafe fn path_icon(path: &str) -> Option<Vec<u8>> {
    let clean = if path.starts_with("shell:") { path.to_string() } else { path.replace('/', "\\") };
    let wide = sys::wz(&clean);
    let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
    image_px(&factory)
}

/// Иконка для `.exe` (виджет «Топ приложений»): берёт из единого кэша по `(путь, mtime, размер)`.
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

/// Разворачивает строковый ресурс вида `@C:\...\app.dll,-101`.
pub unsafe fn resolve_indirect(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('@') {
        return None;
    }
    let wide = sys::wz(trimmed);
    let mut out = [0u16; 512];
    SHLoadIndirectString(PCWSTR(wide.as_ptr()), &mut out, None).ok()?;
    let end = out.iter().position(|&c| c == 0).unwrap_or(out.len());
    let text = String::from_utf16_lossy(&out[..end]).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Имя приложения из реестра `AppUserModelId\<app_id>\DisplayName`, если WinRT его не вернул.
pub unsafe fn reg_app_name(app_id: &str) -> Option<String> {
    if app_id.is_empty() || app_id == "windows.unknown" {
        return None;
    }
    let subkey = format!(r"Software\Classes\AppUserModelId\{app_id}");
    let raw = sys::reg_str(false, &subkey, "DisplayName").or_else(|| sys::reg_str(true, &subkey, "DisplayName"))?;
    if raw.starts_with('@') {
        resolve_indirect(&raw)
    } else if raw.starts_with("ms-resource:") {
        resolve_indirect(&format!("@{raw}"))
    } else {
        Some(raw)
    }
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

unsafe fn running_process_icon(app_id: &str, app_name: &str) -> Option<Arc<Vec<u8>>> {
    let mut targets: Vec<String> = Vec::new();
    let nlow = app_name.trim().to_lowercase();
    if !nlow.is_empty() {
        targets.push(nlow.clone());
        for tok in nlow.split(|c: char| !c.is_alphanumeric()) {
            if tok.chars().count() >= 4 && !is_generic_stem(tok) {
                targets.push(tok.to_string());
            }
        }
    }
    if !app_id.is_empty() && app_id != "windows.unknown" {
        let stem = app_id
            .split('!')
            .next()
            .unwrap_or(app_id)
            .rsplit(['\\', '/', '.'])
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if stem.chars().count() >= 4 && !is_generic_stem(&stem) {
            targets.push(stem);
        }
    }
    if targets.is_empty() {
        return None;
    }

    let mut pids = vec![0u32; 2048];
    let mut needed = 0u32;
    if EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut needed).is_err() {
        return None;
    }
    let own = std::process::id();
    for &pid in &pids[..(needed as usize / 4).min(pids.len())] {
        if pid == 0 || pid == own {
            continue;
        }
        let Ok(hp) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(hp, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(hp);
        if !ok {
            continue;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let Some(stem) = Path::new(&path).file_stem().and_then(|s| s.to_str()) else { continue };
        let stem_low = stem.to_lowercase();
        if !is_generic_stem(&stem_low) && targets.contains(&stem_low) {
            if let Some(px) = exe_icon(&path, Some(app_name)) {
                return Some(px);
            }
        }
    }
    None
}

unsafe fn icon_from_uri(raw: &str) -> Option<Vec<u8>> {
    let s = raw.trim();
    if s.is_empty() || s.starts_with("ms-resource:") || s.starts_with("ms-appx:") {
        return None;
    }
    let s = s.strip_prefix("file:///").or_else(|| s.strip_prefix("file://")).unwrap_or(s);
    let unquoted = s.trim_matches('"');

    if let Some((left, right)) = unquoted.rsplit_once(',') {
        if let Ok(idx) = right.trim().parse::<i32>() {
            let file = left.trim().trim_start_matches('@').trim_matches('"').replace('/', "\\");
            if let Some(px) = extract_icon_index(&file, idx) {
                return Some(px);
            }
            if idx == 0 {
                return path_icon(&file);
            }
            return None;
        }
    }

    let path = unquoted.replace('/', "\\").replace("%20", " ");
    if Path::new(&path).exists() {
        return wic_file_to_px(&path).or_else(|| path_icon(&path));
    }
    None
}

unsafe fn extract_icon_index(path: &str, index: i32) -> Option<Vec<u8>> {
    if !Path::new(path).exists() {
        return None;
    }
    let wide = sys::wz(path);
    let mut hicon = HICON::default();
    if wide.len() <= 260 {
        let mut arr = [0u16; 260];
        arr[..wide.len()].copy_from_slice(&wide);
        let mut icons = [HICON::default()];
        let n = PrivateExtractIconsW(&arr, index, SIZE_PX as i32, SIZE_PX as i32, Some(&mut icons), None, 0);
        if n != u32::MAX && n > 0 && !icons[0].is_invalid() {
            hicon = icons[0];
        }
    }
    if hicon.is_invalid() {
        let n = ExtractIconExW(PCWSTR(wide.as_ptr()), index, Some(&mut hicon), None, 1);
        if n == u32::MAX || n == 0 || hicon.is_invalid() {
            return None;
        }
    }
    let px = hicon_to_px(hicon);
    let _ = DestroyIcon(hicon);
    px
}

unsafe fn hicon_to_px(hicon: HICON) -> Option<Vec<u8>> {
    let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let bmp = factory.CreateBitmapFromHICON(hicon).ok()?;
    let src: IWICBitmapSource = bmp.cast().ok()?;
    wic_source_to_px(&factory, &src)
}

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

unsafe fn image_px(f: &IShellItemImageFactory) -> Option<Vec<u8>> {
    let hbmp = f.GetImage(SIZE { cx: SIZE_PX as i32, cy: SIZE_PX as i32 }, SIIGBF_ICONONLY).ok()?;
    let mut bm = BITMAP::default();
    let got = GetObjectW(HGDIOBJ(hbmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void));
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        return None;
    }
    let (w, h) = (bm.bmWidth as usize, bm.bmHeight as usize);
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; w * h * 4];
    let dc = GetDC(None);
    let lines = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    let _ = DeleteObject(HGDIOBJ(hbmp.0));
    if lines == 0 {
        return None;
    }
    Some(normalize(&buf, w, h))
}

/// Приводит пиксели к `SIZE_PX`×`SIZE_PX` BGRA premultiplied. Альфа Shell приходит «прямая»;
/// если альфы нет вовсе (все нули) — картинка считается непрозрачной.
fn normalize(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let n = SIZE_PX as usize;
    let opaque = src.chunks_exact(4).all(|p| p[3] == 0);
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        let (y0, y1) = (y * h / n, ((y + 1) * h / n).max(y * h / n + 1).min(h));
        for x in 0..n {
            let (x0, x1) = (x * w / n, ((x + 1) * w / n).max(x * w / n + 1).min(w));
            // Среднее по блоку исходных пикселей: крупная иконка Shell уменьшается без «лесенки».
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

/// Перечисляет `shell:AppsFolder`: UWP-приложения (с «!») идут во вкладку Apps, а все
/// элементы (включая Win32 AUMID) запоминаются для быстрого поиска иконок уведомлений.
unsafe fn enumerate_apps_folder(gen: u32, push: &dyn Fn(Found)) {
    let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None::<HANDLE>) else {
        log::error("не удалось открыть shell:AppsFolder");
        return;
    };
    let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return };
    let mut seen_uwp = HashSet::new();
    let mut completed = true;
    loop {
        if GEN.load(Ordering::SeqCst) != gen {
            completed = false;
            break;
        }
        let mut item: [Option<IShellItem>; 1] = [None];
        let mut got = 0u32;
        let _ = en.Next(&mut item, Some(&mut got));
        let Some(item) = item[0].take() else { break };
        if got == 0 {
            break;
        }
        let take = |sigdn| -> Option<String> {
            let p = item.GetDisplayName(sigdn).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            s
        };
        let (Some(name), Some(parse)) = (take(SIGDN_NORMALDISPLAY), take(SIGDN_PARENTRELATIVEPARSING)) else { continue };
        let key = aumid_key(&parse);
        let name_low = name.trim().to_lowercase();
        if let Ok(mut s) = store().lock() {
            s.apps_folder.insert(parse.to_lowercase(), parse.clone());
        }
        if !parse.contains('!') {
            if let Ok(mut s) = store().lock() {
                if let Some(canon) = s.aliases.get(&format!("name:{name_low}")).cloned() {
                    if s.entries.contains_key(&canon) {
                        s.aliases.insert(key, canon);
                    }
                }
            }
            continue;
        }
        seen_uwp.insert(key.clone());
        let cached = store().lock().ok().and_then(|mut s| s.get_by_key(&key));
        let px = match cached {
            Some(px) => px,
            None => {
                let Ok(factory) = item.cast::<IShellItemImageFactory>() else { continue };
                let Some(px) = image_px(&factory) else { continue };
                let px = Arc::new(px);
                if let Ok(mut s) = store().lock() {
                    s.insert_entry(key.clone(), 0, px.clone());
                }
                px
            }
        };
        if let Ok(mut s) = store().lock() {
            register_aliases(&mut s, &key, Some(&name), Some(&parse), None);
        }
        push(Found::Uwp(Uwp { name, target: format!("shell:AppsFolder\\{parse}"), px }));
    }

    if completed {
        if let Ok(mut s) = store().lock() {
            let before = s.entries.len();
            s.entries.retain(|k, _| !(k.starts_with("aumid:") && k.contains('!') && !seen_uwp.contains(k)));
            if s.entries.len() != before {
                s.dirty = true;
            }
        }
    }
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
}
