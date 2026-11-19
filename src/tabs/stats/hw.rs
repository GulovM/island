//! Определение оборудования: число ядер, название CPU и GPU.

pub(super) fn cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 256))
        .unwrap_or(8)
}

/// Модель процессора (на macOS — `sysctl -n machdep.cpu.brand_string`).
pub(super) fn cpu_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            if out.status.success() {
                let s = clean_cpu(&String::from_utf8_lossy(&out.stdout));
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    String::new()
}

#[cfg(any(target_os = "macos", test))]
fn clean_cpu(raw: &str) -> String {
    let mut s = raw.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    for cut in [" with ", " w/ ", " @ ", " CPU"] {
        if let Some(i) = s.find(cut) {
            s.truncate(i);
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn short_gpu(raw: &str) -> String {
    raw.replace("(R)", "").replace("(TM)", "").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_names() {
        assert_eq!(clean_cpu("AMD Ryzen 7 7840HS w/ Radeon 780M Graphics     "), "AMD Ryzen 7 7840HS");
        assert_eq!(clean_cpu("13th Gen Intel(R) Core(TM) i7-13700H"), "13th Gen Intel Core i7-13700H");
        assert_eq!(clean_cpu("Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz"), "Intel Core i5-8250U");
    }
}
