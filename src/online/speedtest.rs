use crate::{bg, log, sys};

use super::http::{get_text, http};
use super::{spawn, with};

const DOWN_URL: &str = "https://speed.cloudflare.com/__down?bytes=25000000";
const UP_URL: &str = "https://speed.cloudflare.com/__up";
const UP_BYTES: usize = 8_000_000;

#[derive(Clone, Debug, Default)]
pub struct Speed {
    /// Мбит/с.
    pub down: f32,
    pub up: f32,
    /// Код узла Cloudflare (город), через который шёл замер.
    pub place: String,
    /// Когда замерили (unix-секунды); 0 — ещё не замеряли.
    pub at: u64,
    pub running: bool,
    pub failed: bool,
}

/// Последний замер из `state.txt`. Без срока давности: значения меняет только новый замер.
pub fn init_cached_speed(down: f32, up: f32, place: String, at: u64) {
    if at == 0 || (down <= 0.0 && up <= 0.0) {
        return;
    }
    with(|s| {
        if s.snap.speed.at == 0 && !s.snap.speed.running {
            s.snap.speed = Speed { down, up, place, at, running: false, failed: false };
        }
    });
}

pub fn run_speedtest() {
    let go = with(|s| {
        if s.snap.speed.running {
            return false;
        }
        // Виджет сразу забывает старые цифры, чтобы не выдавать их за текущие; в `state.txt` они
        // остаются до успешного замера — сбой без сети не стирает последний хороший результат.
        s.snap.speed = Speed { running: true, ..Speed::default() };
        true
    });
    if !go {
        return;
    }
    bg::notify();
    spawn("island-speed", || {
        let place = get_text("https://speed.cloudflare.com/cdn-cgi/trace")
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("colo=").map(str::to_string)))
            .unwrap_or_default();
        let down = measure(None);
        let up = measure(Some(UP_BYTES));
        let saved = with(|s| {
            let sp = &mut s.snap.speed;
            sp.running = false;
            match (down, up) {
                (Ok(d), Ok(u)) => {
                    sp.down = d;
                    sp.up = u;
                    sp.place = place;
                    sp.at = sys::now_secs();
                    Some(sp.clone())
                }
                (d, u) => {
                    log::error(&format!("замер скорости: {:?} / {:?}", d.err(), u.err()));
                    sp.failed = true;
                    None
                }
            }
        });
        // Запись на диск — вне блокировки снимка, чтобы не держать её на время файлового ввода-вывода.
        if let Some(sp) = saved {
            crate::state::save_speed(sp.down, sp.up, &sp.place, sp.at);
        }
        bg::notify();
    });
}

fn measure(upload: Option<usize>) -> Result<f32, String> {
    let url = if upload.is_some() { UP_URL } else { DOWN_URL };
    let r = unsafe { http(url, upload, 0)? };
    if r.status >= 400 {
        return Err(format!("HTTP {}", r.status));
    }
    let bytes = upload.unwrap_or(r.total as usize) as f64;
    Ok((bytes * 8.0 / r.secs.max(0.001) / 1e6) as f32)
}
