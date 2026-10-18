//! Wallpaper Engine: поиск установленных обоев (подписки Steam Workshop и проекты самой программы)
//! и их включение через командную строку WE (`-control openWallpaper`). Обои рендерит сам
//! Wallpaper Engine, поэтому анимация, звук, скрипты, плагины и пользовательские настройки обоев
//! работают как при выборе в его окне; остров показывает только превью проекта.
//!
//! Где лежат обои: `<библиотека Steam>\steamapps\workshop\content\431960\<id>\project.json`,
//! а также `wallpaper_engine\projects\myprojects` и `projects\defaultprojects`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::log;

/// Steam AppID Wallpaper Engine.
const APP_ID: &str = "431960";
const EXES: [&str; 2] = ["wallpaper64.exe", "wallpaper32.exe"];

/// Установка Wallpaper Engine.
#[derive(Clone, Debug)]
pub struct Install {
    pub dir: PathBuf,
    pub exe: PathBuf,
    /// Папки с проектами (в каждой — подпапки с `project.json`); первая — подписки Workshop.
    pub roots: Vec<PathBuf>,
}

/// Обои Wallpaper Engine.
#[derive(Clone, Debug)]
pub struct Item {
    pub title: String,
    /// Путь к `project.json`: его WE принимает в `-file`.
    pub project: PathBuf,
    /// Статичное превью (`preview.jpg`/`.png`/`.gif` — у GIF берётся первый кадр).
    pub preview: PathBuf,
}

// ---------- Поиск установки ----------

/// Ищет Wallpaper Engine по библиотекам Steam, затем по записи деинсталлятора.
pub fn find() -> Option<Install> {
    let mut libs: Vec<PathBuf> = Vec::new();
    if let Some(steam) = crate::sys::reg_str(false, "Software\\Valve\\Steam", "SteamPath") {
        let steam = PathBuf::from(steam.replace('/', "\\"));
        if let Ok(b) = std::fs::read(steam.join("steamapps").join("libraryfolders.vdf")) {
            libs.extend(library_paths(&String::from_utf8_lossy(&b)).into_iter().map(PathBuf::from));
        }
        if !libs.iter().any(|l| same_path(l, &steam)) {
            libs.insert(0, steam);
        }
    }

    let mut dir = libs.iter().map(|l| l.join("steamapps").join("common").join("wallpaper_engine")).find(|d| exe_in(d).is_some());
    if dir.is_none() {
        let key = format!("Microsoft\\Windows\\CurrentVersion\\Uninstall\\Steam App {APP_ID}");
        dir = ["SOFTWARE\\WOW6432Node\\", "SOFTWARE\\"]
            .iter()
            .filter_map(|p| crate::sys::reg_str(true, &format!("{p}{key}"), "InstallLocation"))
            .map(PathBuf::from)
            .find(|d| exe_in(d).is_some());
    }
    let dir = dir?;
    let exe = exe_in(&dir)?;

    let mut roots: Vec<PathBuf> = Vec::new();
    // Workshop обычно в той же библиотеке, что и программа (`..\..\workshop`), но проверяем все.
    let mut workshop: Vec<PathBuf> = libs.iter().map(|l| l.join("steamapps").join("workshop").join("content").join(APP_ID)).collect();
    if let Some(steamapps) = dir.parent().and_then(Path::parent) {
        workshop.insert(0, steamapps.join("workshop").join("content").join(APP_ID));
    }
    for w in workshop {
        if w.is_dir() && !roots.iter().any(|r| same_path(r, &w)) {
            roots.push(w);
        }
    }
    for p in ["myprojects", "defaultprojects"] {
        let d = dir.join("projects").join(p);
        if d.is_dir() {
            roots.push(d);
        }
    }
    Some(Install { dir, exe, roots })
}

fn exe_in(dir: &Path) -> Option<PathBuf> {
    // На 64-битной Windows — 64-битная версия, иначе 32-битная.
    let order: &[&str] = if cfg!(target_pointer_width = "64") { &EXES } else { &EXES[1..] };
    order.iter().map(|e| dir.join(e)).find(|p| p.is_file())
}

fn same_path(a: &Path, b: &Path) -> bool {
    norm(&a.to_string_lossy()) == norm(&b.to_string_lossy())
}

/// Нормализованная папка проекта по пути к любому его файлу.
pub fn project_dir(file: &str) -> String {
    let n = norm(file);
    match n.rfind('/') {
        Some(i) => n[..i].to_string(),
        None => n,
    }
}

/// Путь для сравнения: `/` вместо `\`, нижний регистр, без завершающей косой.
pub fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Пути библиотек из `libraryfolders.vdf` (строки `"path"  "D:\\SteamLibrary"`).
fn library_paths(vdf: &str) -> Vec<String> {
    vdf.lines()
        .map(vdf_strings)
        .filter(|p| p.len() == 2 && p[0].eq_ignore_ascii_case("path"))
        .map(|mut p| p.remove(1))
        .collect()
}

/// Строки в кавычках из строки VDF (с экранированием `\\` и `\"`).
fn vdf_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = line.chars();
    while let Some(c) = it.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = it.next() {
            match c {
                '"' => break,
                '\\' => {
                    if let Some(n) = it.next() {
                        s.push(n);
                    }
                }
                c => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

// ---------- Список обоев ----------

/// Обои из всех папок установки, новые (по времени изменения папки проекта) первыми, не больше `max`.
pub fn scan(inst: &Install, max: usize) -> Vec<Item> {
    let mut found: Vec<(std::time::SystemTime, Item)> = Vec::new();
    for root in &inst.roots {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        for dir in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            if let Some(item) = read_project(&dir) {
                let t = std::fs::metadata(&dir).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                found.push((t, item));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.to_lowercase().cmp(&b.1.title.to_lowercase())));
    found.into_iter().take(max).map(|(_, i)| i).collect()
}

/// Читает `project.json` папки проекта. `None` — это не проект WE или у него нет превью.
fn read_project(dir: &Path) -> Option<Item> {
    let project = dir.join("project.json");
    let bytes = std::fs::read(&project).ok()?;
    let json = Json::parse(&String::from_utf8_lossy(&bytes))?;
    let preview = json
        .get("preview")
        .and_then(Json::as_str)
        .map(|p| dir.join(p))
        .filter(|p| p.is_file())
        .or_else(|| ["preview.jpg", "preview.png", "preview.gif"].iter().map(|n| dir.join(n)).find(|p| p.is_file()))?;
    let title = json
        .get("title")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Some(Item { title, project, preview })
}

/// Папки проектов, выбранных сейчас в Wallpaper Engine (нормализованные пути, по одному на
/// монитор), из `config.json`: `<пользователь>.general.wallpaperconfig.selectedwallpapers.<монитор>.file`.
/// Там лежит путь к файлу содержимого (`scene.pkg`, `.mp4`, `project.json`), поэтому берём его папку.
pub fn current(inst: &Install) -> Vec<String> {
    let Ok(b) = std::fs::read(inst.dir.join("config.json")) else { return Vec::new() };
    let Some(json) = Json::parse(&String::from_utf8_lossy(&b)) else { return Vec::new() };
    let mut out = Vec::new();
    collect_selected(&json, &mut out);
    out
}

fn collect_selected(j: &Json, out: &mut Vec<String>) {
    let Json::Obj(fields) = j else { return };
    for (k, v) in fields {
        if k.eq_ignore_ascii_case("selectedwallpapers") {
            if let Json::Obj(monitors) = v {
                for (_, m) in monitors {
                    if let Some(f) = m.get("file").and_then(Json::as_str) {
                        out.push(project_dir(f));
                    }
                }
            }
        } else {
            collect_selected(v, out);
        }
    }
}

// ---------- Управление ----------

pub fn running() -> bool {
    crate::sys::process_running(&EXES)
}

/// Включает обои на всех мониторах. Если Wallpaper Engine не запущен, сначала запускает его
/// (команды `-control` принимает только работающий WE). Блокирует поток — вызывать в фоне.
pub fn apply(inst: &Install, project: &Path) -> bool {
    if !running() {
        if let Err(e) = Command::new(&inst.exe).current_dir(&inst.dir).spawn() {
            log::error(&format!("не удалось запустить Wallpaper Engine: {e}"));
            return false;
        }
        let start = Instant::now();
        while !running() && start.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(300));
        }
        // Процесс появляется раньше, чем начинает принимать команды.
        std::thread::sleep(Duration::from_secs(3));
    }
    let file = project.display().to_string();
    let mut ok = true;
    for m in 0..crate::sys::monitor_count() {
        let m = m.to_string();
        ok &= control(inst, &["openWallpaper", "-file", &file, "-monitor", &m]);
    }
    ok
}

/// Убирает обои Wallpaper Engine со всех мониторов, чтобы стали видны обычные обои Windows.
pub fn close_all(inst: &Install) {
    if running() {
        control(inst, &["closeWallpaper"]);
    }
}

/// Отправляет команду работающему WE. `false` — процесс не запустился.
fn control(inst: &Install, args: &[&str]) -> bool {
    match Command::new(&inst.exe).current_dir(&inst.dir).arg("-control").args(args).status() {
        Ok(_) => true,
        Err(e) => {
            log::error(&format!("Wallpaper Engine -control {}: {e}", args.first().unwrap_or(&"")));
            false
        }
    }
}

// ---------- Минимальный JSON (project.json, config.json) ----------

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn parse(s: &str) -> Option<Json> {
        let mut p = Parser { s: s.trim_start_matches('\u{feff}').as_bytes(), i: 0 };
        let v = p.value(0)?;
        p.ws();
        (p.i == p.s.len()).then_some(v)
    }

    /// Поле объекта (ключ без учёта регистра: встречаются и `title`, и `Title`).
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(f) => f.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        let ok = self.s[self.i..].starts_with(lit.as_bytes());
        if ok {
            self.i += lit.len();
        }
        ok
    }

    fn value(&mut self, depth: u32) -> Option<Json> {
        if depth > 64 {
            return None;
        }
        self.ws();
        match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.eat("}") {
                    return Some(Json::Obj(fields));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return None;
                    }
                    fields.push((k, self.value(depth + 1)?));
                    self.ws();
                    if !self.eat(",") {
                        return self.eat("}").then_some(Json::Obj(fields));
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if !self.eat(",") {
                        return self.eat("]").then_some(Json::Arr(items));
                    }
                }
            }
            b'"' => self.string().map(Json::Str),
            b't' => self.eat("true").then_some(Json::Bool(true)),
            b'f' => self.eat("false").then_some(Json::Bool(false)),
            b'n' => self.eat("null").then_some(Json::Null),
            _ => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok().map(Json::Num)
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat("\"") {
            return None;
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hi = self.hex4()?;
                            if (0xD800..0xDC00).contains(&hi) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)).unwrap_or('\u{fffd}')
                            } else {
                                char::from_u32(hi).unwrap_or('\u{fffd}')
                            }
                        }
                        o => o as char,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_parses_nested_and_escapes() {
        let j = Json::parse("\u{feff}{\"Title\":\"Ночь \\u2014 \\\"город\\\"\",\"n\":-1.5e2,\"a\":[true,null,{}],\"general\":{\"title\":\"x\"}}")
            .unwrap();
        assert_eq!(j.get("title").and_then(Json::as_str), Some("Ночь — \"город\""));
        assert_eq!(j.get("n"), Some(&Json::Num(-150.0)));
        assert!(Json::parse("{\"a\":1,}").is_none());
        assert!(Json::parse("{\"a\":1} x").is_none());
    }

    #[test]
    fn json_surrogate_pairs() {
        let j = Json::parse("\"\\ud83d\\ude00\"").unwrap();
        assert_eq!(j.as_str(), Some("😀"));
    }

    #[test]
    fn vdf_library_paths() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(library_paths(vdf), ["C:\\Program Files (x86)\\Steam", "D:\\SteamLibrary"]);
    }

    #[test]
    fn selected_wallpapers_from_config() {
        let cfg = Json::parse(
            r#"{"user":{"general":{"wallpaperconfig":{"selectedwallpapers":{"Monitor0":{"file":"D:\\SteamLibrary\\a\\scene.pkg"},"Monitor1":{"file":"D:/b/project.json"}}}}}}"#,
        )
        .unwrap();
        let mut out = Vec::new();
        collect_selected(&cfg, &mut out);
        assert_eq!(out, ["d:/steamlibrary/a", "d:/b"]);
    }

    /// Только на машине с установленным Wallpaper Engine: `cargo test real_install -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_install() {
        let inst = find().expect("Wallpaper Engine не найден");
        let items = scan(&inst, 100);
        println!("{inst:#?}\n{} обоев, первые: {:#?}\nтекущие: {:?}", items.len(), &items[..items.len().min(3)], current(&inst));
        assert!(!items.is_empty());
    }

    #[test]
    fn scan_reads_projects_and_skips_junk() {
        let root = std::env::temp_dir().join(format!("island-we-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let put = |rel: &str, data: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        };
        put("111/project.json", r#"{"title":"Scene","file":"scene.json","preview":"preview.gif","type":"scene"}"#);
        put("111/preview.gif", "x");
        put("222/project.json", r#"{"file":"v.mp4","type":"video"}"#);
        put("222/preview.jpg", "x");
        put("333/project.json", r#"{"title":"No preview"}"#);
        put("444/readme.txt", "x");
        put("555/project.json", "not json");
        put("555/preview.jpg", "x");

        let inst = Install { dir: root.clone(), exe: root.join("wallpaper64.exe"), roots: vec![root.clone()] };
        let mut titles: Vec<String> = scan(&inst, 10).into_iter().map(|i| i.title).collect();
        titles.sort();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(titles, ["222", "Scene"]);
    }
}
