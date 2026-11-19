//! Стандартные пользовательские каталоги (System/Library, Pictures).

use std::path::PathBuf;

pub fn windows_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/System/Library")
    }
    #[cfg(not(target_os = "macos"))]
    PathBuf::from(r"C:\Windows")
}

pub fn pictures_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join("Pictures"))
}
