//! Системные настройки (заглушка Windows Registry на macOS).

pub fn reg_str(_machine: bool, _key: &str, _name: &str) -> Option<String> {
    None
}

pub fn windows_build_number() -> u32 {
    0
}
