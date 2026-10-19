//! Обход процессов и рабочий поток фонового опроса.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::ProcessStatus::{EnumProcesses, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::{bg, icons, sys};

use super::gpu::{best_adapter, GpuQuery};
use super::hw::cpu_count;
use super::{ft, GEN, HEAVY};

#[derive(Default)]
struct Procs {
    top_ram: Vec<(String, f64)>,
    top_cpu: Vec<(String, f64)>,
    app_icons: HashMap<String, Arc<Vec<u8>>>,
}

/// Обход процессов: память и процессорное время, сгруппированные по имени программы.
#[derive(Default)]
struct ProcSampler {
    prev: HashMap<u32, u64>,
    at: Option<Instant>,
    /// Название программы по пути к exe (описание читается из ресурсов файла — дорого).
    names: HashMap<String, String>,
    /// Процесс → (время создания, название): путь запрашивается один раз, а не на каждом проходе.
    /// Время создания отличает новый процесс, занявший тот же pid.
    known: HashMap<u32, (u64, String, String)>,
    /// Иконка по пути к exe (None — не удалось получить, повторно не просим).
    icons: HashMap<String, Option<Arc<Vec<u8>>>>,
}

impl ProcSampler {
    unsafe fn sample(&mut self, ncpu: f64) -> Procs {
        let mut pids = vec![0u32; 4096];
        let mut needed = 0u32;
        if EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut needed).is_err() {
            return Procs::default();
        }
        let now = Instant::now();
        let dt = self.at.map(|t| (now - t).as_secs_f64()).unwrap_or(0.0);
        let (mut ram, mut cpu): (HashMap<String, f64>, HashMap<String, f64>) = (HashMap::new(), HashMap::new());
        let mut next = HashMap::new();
        let mut paths: HashMap<String, String> = HashMap::new();
        let mut known = HashMap::with_capacity(self.known.len());
        let own = std::process::id();
        for &pid in &pids[..(needed / 4) as usize] {
            if pid == 0 || pid == own {
                continue;
            }
            let Ok(hp) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
            let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            let (created, t) = if GetProcessTimes(hp, &mut c, &mut e, &mut k, &mut u).is_ok() { (ft(c), ft(k) + ft(u)) } else { (0, 0) };
            let (name, path) = match self.known.remove(&pid) {
                Some((cr, name, path)) if cr == created => (name, path),
                _ => {
                    let mut buf = [0u16; 520];
                    let mut len = buf.len() as u32;
                    if QueryFullProcessImageNameW(hp, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_err() {
                        let _ = CloseHandle(hp);
                        continue;
                    }
                    let path = String::from_utf16_lossy(&buf[..len as usize]);
                    let name = self.names.entry(path.clone()).or_insert_with(|| describe(&path)).clone();
                    (name, path)
                }
            };
            let mut pmc = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
            let ws = if GetProcessMemoryInfo(hp, &mut pmc, pmc.cb).is_ok() { pmc.WorkingSetSize as f64 } else { 0.0 };
            let _ = CloseHandle(hp);
            paths.entry(name.clone()).or_insert_with(|| path.clone());
            *ram.entry(name.clone()).or_default() += ws;
            if let (Some(&p), true) = (self.prev.get(&pid), dt > 0.0) {
                *cpu.entry(name.clone()).or_default() += t.saturating_sub(p) as f64 / (dt * 1e7 * ncpu);
            }
            next.insert(pid, t);
            known.insert(pid, (created, name, path));
        }
        self.prev = next;
        self.known = known;
        self.at = Some(now);
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
            let icon = self.icons.entry(path.clone()).or_insert_with(|| icons::path_icon(path).map(Arc::new));
            if let Some(px) = icon {
                app_icons.insert(name.clone(), px.clone());
            }
        }
        Procs { top_ram, top_cpu, app_icons }
    }
}

/// Название программы из ресурса версии (FileDescription), иначе имя файла.
unsafe fn describe(path: &str) -> String {
    let stem = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_string();
    let wp = sys::wz(path);
    let size = GetFileVersionInfoSizeW(PCWSTR(wp.as_ptr()), None);
    if size == 0 {
        return stem;
    }
    let mut data = vec![0u8; size as usize];
    if GetFileVersionInfoW(PCWSTR(wp.as_ptr()), None, size, data.as_mut_ptr() as *mut c_void).is_err() {
        return stem;
    }
    let (mut p, mut len) = (std::ptr::null_mut::<c_void>(), 0u32);
    if !VerQueryValueW(data.as_ptr() as *const c_void, w!("\\VarFileInfo\\Translation"), &mut p, &mut len).as_bool() || len < 4 {
        return stem;
    }
    let lang = *(p as *const u16);
    let cp = *(p as *const u16).add(1);
    let key = sys::wz(&format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\FileDescription"));
    if VerQueryValueW(data.as_ptr() as *const c_void, PCWSTR(key.as_ptr()), &mut p, &mut len).as_bool() && len > 1 {
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize - 1));
        let s = s.trim().to_string();
        if !s.is_empty() && s.chars().count() <= 40 {
            return s;
        }
    }
    stem
}

pub(super) unsafe fn worker(gen: u32) {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let gpu = GpuQuery::new();
    let adapter = best_adapter();
    let mut procs = ProcSampler::default();
    let ncpu = cpu_count() as f64;
    let mut tick = 0u32;
    while GEN.load(Ordering::SeqCst) == gen {
        let mut h = HEAVY.lock().ok().and_then(|h| h.clone()).unwrap_or_default();
        // У встроенной графики своей памяти почти нет (128 МБ) — показываем общую, как диспетчер задач.
        let integrated = adapter.as_ref().is_some_and(|a| a.1 < 512.0 * 1024.0 * 1024.0);
        // Счётчик «GPU Engine(*)» перебирает сотни экземпляров — раз в 2 секунды, не в ту же секунду,
        // что и обход процессов.
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
        // Процессы обходим раз в 2 секунды; первая выборка только запоминает времена.
        if tick % 2 == 0 {
            let p = procs.sample(ncpu);
            h.top_ram = p.top_ram;
            if tick > 0 {
                h.top_cpu = p.top_cpu;
            }
            h.app_icons = p.app_icons;
        }
        if let Ok(mut g) = HEAVY.lock() {
            *g = Some(h);
        }
        bg::notify();
        tick += 1;
        // PDH и обход процессов при первых опросах подтягивают десятки МБ страниц, которые
        // дальше не нужны: отдаём их системе один раз, когда оба опроса уже сделаны.
        if tick == 2 {
            sys::trim_working_set();
        }
        for _ in 0..10 {
            if GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
