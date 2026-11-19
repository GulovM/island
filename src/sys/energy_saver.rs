//! Энергосбережение macOS (`pmset -g` `lowpowermode`) + сохранение порога для тестов.

#[cfg(test)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(test)]
const ALWAYS: u32 = 100;
#[cfg(test)]
const DEFAULT_THRESHOLD: u32 = 20;
#[cfg(test)]
static SAVED: AtomicU32 = AtomicU32::new(DEFAULT_THRESHOLD);

#[cfg(test)]
fn saved() -> u32 {
    SAVED.load(Ordering::Relaxed)
}

pub fn energy_saver() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/pmset")
            .args(["-g"])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("lowpowermode") {
                if let Some(v) = rest.split_whitespace().next() {
                    return Some(v == "1");
                }
            }
        }
        Some(false)
    }
    #[cfg(not(target_os = "macos"))]
    None
}

pub fn set_energy_saver(on: bool) -> bool {
    let val = if on { "1" } else { "0" };
    #[cfg(target_os = "macos")]
    {
        if std::process::Command::new("/usr/bin/pmset")
            .args(["-a", "lowpowermode", val])
            .status()
            .is_ok_and(|s| s.success())
        {
            return true;
        }
    }
    let _ = val;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_threshold_is_below_always() {
        assert!(saved() < ALWAYS);
    }
}
