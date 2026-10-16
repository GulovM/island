//! Лог в файл `%APPDATA%\island\island.log` (консоли у программы нет).

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::System::SystemInformation::GetLocalTime;

/// После этого размера старый лог переезжает в `island.log.old`.
const MAX_BYTES: u64 = 256 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init() {
    let dir = crate::sys::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("island.log");
    if std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("island.log.old"));
    }
    let _ = PATH.set(path);

    // `panic = "abort"`: хук успевает записать причину до аварийного завершения.
    std::panic::set_hook(Box::new(|info| error(&format!("паника: {info}"))));
}

pub fn info(msg: &str) {
    write("INFO ", msg);
}

pub fn error(msg: &str) {
    write("ERROR", msg);
}

fn write(level: &str, msg: &str) {
    let Some(path) = PATH.get() else { return };
    let t = unsafe { GetLocalTime() };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {level} {msg}\n",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    );
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}
