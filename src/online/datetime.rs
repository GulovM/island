use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;

/// Вместо фиксированного смещения: переводить время из UTC по правилам часового пояса системы на
/// дату события (учёт летнего времени).
pub const SYSTEM_TZ: i64 = i64::MIN;

/// Число дней от 1970-01-01 до гражданской даты (алгоритм Говарда Хиннанта).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y } as i32, m, d)
}

/// День недели, 0 — понедельник.
pub fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

pub fn now_local_minutes() -> i64 {
    let t = unsafe { GetLocalTime() };
    days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32) * 1440 + t.wHour as i64 * 60 + t.wMinute as i64
}

pub fn utc_offset_minutes() -> i64 {
    let u = unsafe { GetSystemTime() };
    let utc = days_from_civil(u.wYear as i32, u.wMonth as u32, u.wDay as u32) * 1440 + u.wHour as i64 * 60 + u.wMinute as i64;
    now_local_minutes() - utc
}

/// Смещение местного времени от UTC на момент `utc` (минуты с эпохи). `offset == SYSTEM_TZ` —
/// правила системы, иначе заданное смещение.
pub(super) fn offset_at(utc: i64, offset: i64) -> i64 {
    if offset != SYSTEM_TZ {
        return offset;
    }
    let (y, mo, d) = civil_from_days(utc.div_euclid(1440));
    let m = utc.rem_euclid(1440);
    let st = SYSTEMTIME { wYear: y as u16, wMonth: mo as u16, wDay: d as u16, wHour: (m / 60) as u16, wMinute: (m % 60) as u16, ..Default::default() };
    let mut local = SYSTEMTIME::default();
    if unsafe { SystemTimeToTzSpecificLocalTime(None, &st, &mut local) }.is_err() {
        return utc_offset_minutes();
    }
    let l = days_from_civil(local.wYear as i32, local.wMonth as u32, local.wDay as u32) * 1440 + local.wHour as i64 * 60 + local.wMinute as i64;
    l - utc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 1440 + h * 60 + mi
    }

    #[test]
    fn civil_roundtrip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        let z = days_from_civil(2026, 10, 15);
        assert_eq!(civil_from_days(z), (2026, 10, 15));
        assert_eq!(weekday(z), 3); // четверг
    }

    #[test]
    fn tz_offset_per_date() {
        assert_eq!(offset_at(at(2026, 7, 1, 0, 0), 180), 180);
        let o = offset_at(at(2026, 7, 1, 0, 0), SYSTEM_TZ);
        assert!((-14 * 60..=14 * 60).contains(&o));
    }
}
