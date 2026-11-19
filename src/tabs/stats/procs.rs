//! Обход процессов и рабочий поток фонового опроса на macOS.

use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::bg;
#[cfg(target_os = "macos")]
use crate::icons;

use super::gpu::{best_adapter, GpuQuery};
use super::{GEN, HEAVY};

#[derive(Default)]
struct Procs {
    top_ram: Vec<(String, f64)>,
    top_cpu: Vec<(String, f64)>,
    app_icons: HashMap<String, Arc<Vec<u8>>>,
}

#[derive(Default)]
struct ProcSampler {
    #[cfg(target_os = "macos")]
    icons: HashMap<String, Option<Arc<Vec<u8>>>>,
}

impl ProcSampler {
    unsafe fn sample(&mut self, _ncpu: f64) -> Procs {
        #[cfg(target_os = "macos")]
        {
            let Ok(out) = std::process::Command::new("/bin/ps")
                .args(["-axo", "pid=,pcpu=,rss=,comm="])
                .output()
            else {
                return Procs::default();
            };
            let s = String::from_utf8_lossy(&out.stdout);
            let own = std::process::id();
            let (mut ram, mut cpu): (HashMap<String, f64>, HashMap<String, f64>) =
                (HashMap::new(), HashMap::new());
            let mut paths: HashMap<String, String> = HashMap::new();
            for line in s.lines() {
                let mut parts = line.split_whitespace();
                let Some(pid_s) = parts.next() else { continue };
                let Some(cpu_s) = parts.next() else { continue };
                let Some(rss_s) = parts.next() else { continue };
                let Some(cmd_rest) = line.split_once(rss_s).map(|x| x.1.trim()) else { continue };
                let Ok(pid) = pid_s.parse::<u32>() else { continue };
                if pid == 0 || pid == own {
                    continue;
                }
                let pcpu = cpu_s.replace(',', ".").parse::<f64>().unwrap_or(0.0) / 100.0;
                let rss_bytes = rss_s.parse::<f64>().unwrap_or(0.0) * 1024.0;
                let name = Path::new(cmd_rest)
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .unwrap_or(cmd_rest)
                    .to_string();
                let icon_path = if let Some((prefix, _)) = cmd_rest.split_once(".app/") {
                    format!("{prefix}.app")
                } else {
                    cmd_rest.to_string()
                };
                paths.entry(name.clone()).or_insert(icon_path);
                *ram.entry(name.clone()).or_default() += rss_bytes;
                *cpu.entry(name).or_default() += pcpu;
            }
            let top = |m: HashMap<String, f64>| {
                let mut v: Vec<(String, f64)> = m.into_iter().filter(|x| x.1 > 0.0).collect();
                v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                v.truncate(5);
                v
            };
            let (top_ram, top_cpu) = (top(ram), top(cpu));
            let mut app_icons = HashMap::new();
            for (name, _) in top_ram.iter().chain(&top_cpu) {
                let Some(path) = paths.get(name) else { continue };
                let icon = self.icons.entry(path.clone()).or_insert_with(|| icons::exe_icon(path, Some(name)));
                if let Some(px) = icon {
                    app_icons.insert(name.clone(), px.clone());
                }
            }
            icons::flush_cache();
            Procs { top_ram, top_cpu, app_icons }
        }
        #[cfg(not(target_os = "macos"))]
        Procs::default()
    }
}

pub(super) unsafe fn worker(gen: u32) {
    let gpu = GpuQuery::new();
    let adapter = best_adapter();
    let mut procs = ProcSampler::default();
    let ncpu = super::hw::cpu_count() as f64;
    let mut tick = 0u32;
    while GEN.load(Ordering::SeqCst) == gen {
        let mut h = HEAVY.lock().ok().and_then(|h| h.clone()).unwrap_or_default();
        let integrated = adapter.as_ref().is_some_and(|a| a.1 < 512.0 * 1024.0 * 1024.0);
        if let Some(q) = gpu.as_ref().filter(|_| tick == 0 || tick % 2 == 1) {
            let (util, mem, shared) = q.sample();
            h.gpu = util;
            if let Some(m) = if integrated { shared } else { mem } {
                h.vram_used = m;
            }
        }
        if let Some((name, dedicated, shared)) = &adapter {
            h.gpu_name = name.clone();
            h.vram_total = if integrated { *shared } else { *dedicated };
        }
        if tick % 2 == 0 {
            let p = procs.sample(ncpu);
            h.top_ram = p.top_ram;
            h.top_cpu = p.top_cpu;
            h.app_icons = p.app_icons;
        }
        if let Ok(mut g) = HEAVY.lock() {
            *g = Some(h);
        }
        bg::notify();
        tick += 1;
        for _ in 0..10 {
            if GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
