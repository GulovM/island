//! Системные пути: диск, папка Windows, папка изображений.

use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

use super::wz;

/// Системный диск вида `C:\` (для статистики диска): `%SystemDrive%`, иначе `C:`.
pub fn system_drive_root() -> Vec<u16> {
    let drive = std::env::var("SystemDrive").ok().filter(|d| d.len() == 2 && d.ends_with(':')).unwrap_or_else(|| "C:".into());
    wz(&format!("{drive}\\"))
}

/// Папка Windows (`%SystemRoot%`), с запасным вариантом.
pub fn windows_dir() -> PathBuf {
    std::env::var_os("SystemRoot").map(Into::into).unwrap_or_else(|| "C:\\Windows".into())
}

/// Папка «Изображения» пользователя (учитывает перенаправление в OneDrive и т. п.).
pub fn pictures_dir() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(Into::into)
    }
}
