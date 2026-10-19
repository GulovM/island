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

pub(super) fn fmt_uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    let (du, hu, mu) = if crate::i18n::en() { ("d", "h", "min") } else { ("д", "ч", "мин") };
    if d > 0 {
        format!("{d} {du} {h} {hu}")
    } else if h > 0 {
        format!("{h} {hu} {m} {mu}")
    } else {
        format!("{m} {mu}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_rate(500.0), "500 Б/с");
        assert_eq!(fmt_rate(2048.0), "2 КБ/с");
        assert_eq!(fmt_rate(3.5 * 1024.0 * 1024.0), "3.5 МБ/с");
        assert_eq!(fmt_uptime(90_000), "1 д 1 ч");
        assert_eq!(fmt_uptime(3_700), "1 ч 1 мин");
        assert_eq!(fmt_uptime(120), "2 мин");
        assert_eq!(fmt_bytes(1.5 * 1024.0 * 1024.0 * 1024.0), "1.5 ГБ");
        assert_eq!(fmt_bytes(820.0 * 1024.0 * 1024.0), "820 МБ");
    }
}
