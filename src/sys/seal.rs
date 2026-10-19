//! Защита данных пользователя через DPAPI (CryptProtectData/CryptUnprotectData).

use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

/// Признак зашифрованного DPAPI файла: `ISL1` + blob `CryptProtectData`.
pub const SEALED_MAGIC: &[u8; 4] = b"ISL1";

/// Шифрует байты ключом текущего пользователя Windows (DPAPI): файл не читается с другой учётной
/// записи или после копирования на другой компьютер. `None` — DPAPI отказал.
pub fn seal(data: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: u32::try_from(data.len()).ok()?, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let mut v = Vec::with_capacity(SEALED_MAGIC.len() + out.cbData as usize);
        v.extend_from_slice(SEALED_MAGIC);
        v.extend_from_slice(std::slice::from_raw_parts(out.pbData, out.cbData as usize));
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        Some(v)
    }
}

/// Обратное к [`seal`]. Данные без признака `ISL1` (файлы старых версий) возвращаются как есть;
/// `None` — файл зашифрован, но расшифровать не удалось (чужая учётная запись, повреждение).
pub fn unseal(data: &[u8]) -> Option<Vec<u8>> {
    let Some(blob) = data.strip_prefix(SEALED_MAGIC) else { return Some(data.to_vec()) };
    let input = CRYPT_INTEGER_BLOB { cbData: u32::try_from(blob.len()).ok()?, pbData: blob.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        Some(v)
    }
}

/// Файл (или его начало) записан через [`seal`].
pub fn is_sealed(data: &[u8]) -> bool {
    data.starts_with(SEALED_MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_roundtrip() {
        let data = "секрет: пароль 123".as_bytes();
        let sealed = seal(data).expect("DPAPI");
        assert!(is_sealed(&sealed));
        assert!(!sealed.windows(data.len()).any(|w| w == data));
        assert_eq!(unseal(&sealed).unwrap(), data);
        // Файл старой версии (без признака) читается как есть.
        assert_eq!(unseal(b"plain text").unwrap(), b"plain text");
        // Повреждённый blob не расшифровывается.
        assert!(unseal(b"ISL1garbage").is_none());
    }
}
