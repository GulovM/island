//! Ночной свет (Blue Light Reduction).
//!
//! Публичного API нет. Состояние лежит в реестре в записи CloudStore — её же пишет плитка
//! «Ночной свет» в центре уведомлений, и служба Windows применяет изменение сразу:
//!   43 42 01 00 | 0A 02 01 00 2A 06 <время, LEB128> | 2A 2B 0E <длина> | 43 42 01 00 [10 00] D0 0A 02 C6 14 … | хвост
//! Поле `10 00` сразу после внутреннего заголовка присутствует, только когда ночной свет включён.
//! Формат проверяется строго: незнакомую запись не трогаем (ничего не пишем, открываем Параметры).

use std::ffi::c_void;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_BINARY};

use super::{data_dir, now_secs, read_binary};

const NIGHT_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\CloudStore\\Store\\DefaultAccount\\Current\\default$windows.data.bluelightreduction.bluelightreductionstate\\windows.data.bluelightreduction.bluelightreductionstate");
const CB: [u8; 4] = [0x43, 0x42, 0x01, 0x00];
const NIGHT_HEAD: [u8; 6] = [0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06];
const NIGHT_BODY: [u8; 3] = [0x2A, 0x2B, 0x0E];
const NIGHT_ON: [u8; 2] = [0x10, 0x00];
const NIGHT_NEXT: [u8; 2] = [0xD0, 0x0A];

/// Разобранная запись: (включён, время изменения, начало полезной части, длина полезной части).
fn night_parse(d: &[u8]) -> Option<(bool, u64, usize, usize)> {
    if d.get(0..4)? != CB || d.get(4..10)? != NIGHT_HEAD {
        return None;
    }
    // Время — LEB128 (не длиннее 9 байт).
    let (mut ts, mut i, mut shift) = (0u64, 10usize, 0u32);
    loop {
        let b = *d.get(i)?;
        ts |= ((b & 0x7F) as u64) << shift;
        i += 1;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 56 {
            return None;
        }
    }
    if d.get(i..i + 3)? != NIGHT_BODY {
        return None;
    }
    let len = *d.get(i + 3)? as usize;
    let p = i + 4;
    if len >= 0x80 || d.len() < p + len || d.get(p..p + 4)? != CB {
        return None;
    }
    let on = match d.get(p + 4..p + 6)? {
        x if x == NIGHT_ON && d.get(p + 6..p + 8)? == NIGHT_NEXT => true,
        x if x == NIGHT_NEXT => false,
        _ => return None,
    };
    if len < if on { 8 } else { 6 } {
        return None;
    }
    Some((on, ts, p, len))
}

fn leb128(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

/// Новая запись с состоянием `on` и временем изменения `now` (не раньше прежнего — иначе служба
/// может посчитать изменение устаревшим). `None` — формат незнакомый.
fn night_build(d: &[u8], on: bool, now: u64) -> Option<Vec<u8>> {
    let (cur, ts, p, len) = night_parse(d)?;
    let mut payload = CB.to_vec();
    if on {
        payload.extend_from_slice(&NIGHT_ON);
    }
    let skip = if cur { 6 } else { 4 };
    payload.extend_from_slice(&d[p + skip..p + len]);
    let mut out = CB.to_vec();
    out.extend_from_slice(&NIGHT_HEAD);
    out.extend(leb128(now.max(ts.saturating_add(1))));
    out.extend_from_slice(&NIGHT_BODY);
    // Длина — один байт без старшего бита (иначе это уже LEB128 из двух байт).
    if payload.len() >= 0x80 {
        return None;
    }
    out.push(payload.len() as u8);
    out.extend(payload);
    out.extend_from_slice(&d[p + len..]);
    Some(out)
}

/// Включён ли ночной свет; `None` — записи нет или формат незнакомый (переключать нельзя).
pub fn night_light() -> Option<bool> {
    read_binary(NIGHT_KEY, w!("Data")).and_then(|d| night_parse(&d)).map(|p| p.0)
}

/// Включает/выключает ночной свет. Перед первой записью сохраняет исходное значение в
/// `%APPDATA%\island\nightlight.bak` (вернуть можно через reg import / вручную). `true` — получилось.
pub fn set_night_light(on: bool) -> bool {
    let Some(old) = read_binary(NIGHT_KEY, w!("Data")) else { return false };
    let Some(new) = night_build(&old, on, now_secs()) else {
        crate::log::error("ночной свет: незнакомый формат записи в реестре, не трогаю");
        return false;
    };
    let backup = data_dir().join("nightlight.bak");
    if !backup.exists() {
        let _ = std::fs::create_dir_all(data_dir());
        let _ = std::fs::write(&backup, &old);
    }
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            NIGHT_KEY,
            w!("Data"),
            REG_BINARY.0,
            Some(new.as_ptr() as *const c_void),
            new.len() as u32,
        )
    };
    if r != ERROR_SUCCESS {
        crate::log::error(&format!("ночной свет: запись в реестр не удалась ({})", r.0));
        return false;
    }
    night_light() == Some(on)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Запись с этого ПК (Windows 11 26300), ночной свет выключен.
    const OFF: [u8; 41] = [
        0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0xDE, 0x93, 0x97, 0xCF, 0x06, 0x2A, 0x2B, 0x0E, 0x13, 0x43, 0x42,
        0x01, 0x00, 0xD0, 0x0A, 0x02, 0xC6, 0x14, 0xD7, 0xAA, 0xAA, 0x92, 0xED, 0xDE, 0x8C, 0xEE, 0x01, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn parses_real_record() {
        let (on, ts, p, len) = night_parse(&OFF).unwrap();
        assert!(!on);
        assert_eq!(ts, 1_776_667_102);
        assert_eq!((p, len), (19, 0x13));
    }

    #[test]
    fn enable_then_disable_roundtrips() {
        let on = night_build(&OFF, true, 1_800_000_000).unwrap();
        assert_eq!(on.len(), 43);
        assert_eq!(on[18], 0x15);
        assert_eq!(&on[23..27], &[0x10, 0x00, 0xD0, 0x0A]);
        assert_eq!(night_parse(&on).map(|p| p.0), Some(true));
        let off = night_build(&on, false, 1_800_000_001).unwrap();
        // Всё, кроме времени, как в исходной записи.
        assert_eq!(off.len(), OFF.len());
        assert_eq!(&off[15..], &OFF[15..]);
        assert_eq!(night_parse(&off).map(|p| (p.0, p.1)), Some((false, 1_800_000_001)));
    }

    #[test]
    fn time_never_goes_back() {
        let on = night_build(&OFF, true, 5).unwrap();
        assert_eq!(night_parse(&on).unwrap().1, 1_776_667_103);
    }

    #[test]
    fn unknown_format_is_refused() {
        let mut bad = OFF;
        bad[23] = 0x77;
        assert!(night_parse(&bad).is_none() && night_build(&bad, true, 1).is_none());
        assert!(night_parse(&OFF[..20]).is_none());
        assert!(night_parse(&[]).is_none());
    }
}
