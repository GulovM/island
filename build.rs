//! Вшивает в exe иконку, манифест и сведения о версии (island.rc) через `rc.exe` из Windows SDK.
//! Локально без `rc.exe` сборка продолжается (exe будет без иконки); в CI и при `ISLAND_REQUIRE_RC=1` — падает,
//! чтобы релиз не вышел без манифеста.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    for f in ["build.rs", "island.rc", "island.manifest", "assets/island.ico"] {
        println!("cargo:rerun-if-changed={f}");
    }
    println!("cargo:rerun-if-env-changed=PATH");
    println!("cargo:rerun-if-env-changed=ISLAND_REQUIRE_RC");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let strict = std::env::var_os("CI").is_some() || std::env::var_os("ISLAND_REQUIRE_RC").is_some();
    let fail = |msg: &str| {
        if strict {
            panic!("{msg}");
        }
        println!("cargo:warning={msg}");
    };
    let Some(rc) = find_rc() else {
        fail("rc.exe не найден: exe будет без иконки, манифеста и сведений о версии");
        return;
    };
    let ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut nums: Vec<String> = ver.split(['.', '-', '+']).take(3).map(|p| p.chars().filter(char::is_ascii_digit).collect::<String>()).collect();
    nums.resize(3, "0".into());
    let comma = format!("{},{},{},0", nums[0], nums[1], nums[2]);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("island.res");
    let ok = Command::new(&rc)
        .args(["/nologo", "/fo"])
        .arg(&out)
        .arg(format!("/dISLAND_VER_COMMA={comma}"))
        .arg(format!("/dISLAND_VER=\"{ver}\""))
        .arg("island.rc")
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        println!("cargo:rustc-link-arg-bins={}", out.display());
    } else {
        fail("rc.exe завершился с ошибкой: exe будет без иконки, манифеста и сведений о версии");
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
    let mut versions: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(&kits)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .map(|p| {
            let key = p.file_name().and_then(|n| n.to_str()).unwrap_or("").split('.').map(|s| s.parse().unwrap_or(0)).collect();
            (key, p)
        })
        .collect();
    // Каталоги вида 10.0.26100.0: сравниваем числами, не строками.
    versions.sort();
    versions.into_iter().rev().map(|(_, v)| v.join("x64").join("rc.exe")).find(|p| p.is_file())
}
