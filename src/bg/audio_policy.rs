//! Переключение устройства вывода звука по умолчанию на macOS.

pub(super) unsafe fn audio_set_default(id: &str) {
    #[cfg(target_os = "macos")]
    {
        for bin in ["/opt/homebrew/bin/SwitchAudioSource", "/usr/local/bin/SwitchAudioSource"] {
            if std::path::Path::new(bin).exists() {
                let _ = std::process::Command::new(bin).args(["-s", id]).status();
                return;
            }
        }
    }
    let _ = id;
}
