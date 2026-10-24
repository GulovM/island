//! Форматирование скоростей, объёмов данных и времени работы.

pub(super) fn fmt_rate(bps: f32) -> String {
    const K: f32 = 1024.0;
    if bps < K {
        format!("{:.0} {}", bps, crate::i18n::pick("Б/с", "B/s"))
    } else if bps < K * K {
        format!("{:.0} {}", bps / K, crate::i18n::pick("КБ/с", "KB/s"))
    } else {
        format!("{:.1} {}", bps / (K * K), crate::i18n::pick("МБ/с", "MB/s"))
    }
}

pub(super) fn fmt_bytes(b: f64) -> String {
    let mb = b / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} {}", mb / 1024.0, crate::i18n::pick("ГБ", "GB"))
    } else {
        format!("{:.0} {}", mb, crate::i18n::pick("МБ", "MB"))
    }
}

/// Время работы двумя частями: старшая единица (число и её подпись) и остаток под ней.
pub(super) fn uptime_parts(secs: u64) -> (u64, &'static str, String) {
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    let (du, hu, mu) = if crate::i18n::en() { ("d", "h", "min") } else { ("д", "ч", "мин") };
    if d > 0 {
        (d, du, format!("{h} {hu} {m} {mu}"))
    } else if h > 0 {
        (h, hu, format!("{m} {mu}"))
    } else {
        (m, mu, String::new())
    }
}

/// Когда включили компьютер: «с 19 окт, 06:52» по местному времени (`now_min` — минуты от 1970-01-01).
pub(super) fn fmt_since(now_min: i64, uptime_s: u64) -> String {
    let boot = now_min - (uptime_s / 60) as i64;
    let (_, m, d) = crate::online::civil_from_days(boot.div_euclid(1440));
    let t = boot.rem_euclid(1440);
    let month = crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1];
    let when = if crate::i18n::en() { format!("{month} {d}") } else { format!("{d} {month}") };
    format!("{} {when}, {:02}:{:02}", crate::i18n::pick("с", "since"), t / 60, t % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_rate(500.0), "500 Б/с");
        assert_eq!(fmt_rate(2048.0), "2 КБ/с");
        assert_eq!(fmt_rate(3.5 * 1024.0 * 1024.0), "3.5 МБ/с");
        assert_eq!(uptime_parts(90_000), (1, "д", "1 ч 0 мин".to_string()));
        assert_eq!(uptime_parts(3_700), (1, "ч", "1 мин".to_string()));
        assert_eq!(uptime_parts(120), (2, "мин", String::new()));
        assert_eq!(fmt_bytes(1.5 * 1024.0 * 1024.0 * 1024.0), "1.5 ГБ");
        assert_eq!(fmt_bytes(820.0 * 1024.0 * 1024.0), "820 МБ");
        // 2026-10-22 10:09 минус 3 д 4 ч 17 мин — 19 октября, 05:52.
        let now = crate::online::days_from_civil(2026, 10, 22) * 1440 + 10 * 60 + 9;
        assert_eq!(fmt_since(now, 3 * 86_400 + 4 * 3600 + 17 * 60), "с 19 окт, 05:52");
    }
}
