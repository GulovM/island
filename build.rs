//! Вшивает в exe иконку и сведения о версии (island.rc) через `rc.exe` из Windows SDK.
//! Если `rc.exe` не нашёлся, сборка продолжается без ресурсов — программа от этого не страдает.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=island.rc");
    println!("cargo:rerun-if-changed=assets/island.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe не найден: exe будет без иконки и сведений о версии");
        return;
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("island.res");
    let ok = Command::new(&rc).args(["/nologo", "/fo"]).arg(&out).arg("island.rc").status().is_ok_and(|s| s.success());
    if ok {
        println!("cargo:rustc-link-arg-bins={}", out.display());
    } else {
        println!("cargo:warning=rc.exe завершился с ошибкой: exe будет без иконки и сведений о версии");
    }
}

/// `rc.exe` из PATH или из самого нового набора Windows SDK.
fn find_rc() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join("rc.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let kits = PathBuf::from(std::env::var_os("ProgramFiles(x86)")?).join("Windows Kits").join("10").join("bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&kits).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    // Каталоги вида 10.0.26100.0: сравниваем как строки — для одной разрядности версий этого хватает.
    versions.sort();
    versions.into_iter().rev().map(|v| v.join("x64").join("rc.exe")).find(|p| p.is_file())
}
