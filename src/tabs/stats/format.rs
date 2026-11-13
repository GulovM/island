//! Форматирование скоростей, объёмов данных и времени работы.

pub(super) fn fmt_rate_parts(bps: f32) -> (String, &'static str) {
    const K: f32 = 1024.0;
    if bps < K {
        (format!("{:.0}", bps), crate::i18n::pick("Б/с", "B/s"))
    } else if bps < K * K {
        (format!("{:.0}", bps / K), crate::i18n::pick("КБ/с", "KB/s"))
    } else {
        (format!("{:.1}", bps / (K * K)), crate::i18n::pick("МБ/с", "MB/s"))
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

/// Когда включили компьютер: «с пн, 09:12» (если менее 7 дней назад) или «с 19 окт, 06:52»
/// по местному времени (`now_min` — минуты от 1970-01-01).
pub(super) fn fmt_since(now_min: i64, uptime_s: u64) -> String {
    let boot = now_min - (uptime_s / 60) as i64;
    let boot_day = boot.div_euclid(1440);
    let t = boot.rem_euclid(1440);
    let wd = (crate::online::weekday(boot_day) as usize).min(6);
    let today_wd = (crate::online::weekday(now_min.div_euclid(1440)) as usize).min(6);
    // День недели однозначен, если загрузка была не раньше недели назад и не в тот же день недели,
    // что сегодня (иначе «с пн» на следующий понедельник читалось бы как «сегодня»).
    let when = if uptime_s < 7 * 86_400 && (wd != today_wd || uptime_s < 86_400) {
        let name = crate::i18n::weekdays()[wd];
        if crate::i18n::en() { name.to_string() } else { name.to_lowercase() }
    } else {
        let (_, m, d) = crate::online::civil_from_days(boot_day);
        let month = crate::i18n::months_short()[(m as usize).clamp(1, 12) - 1];
        if crate::i18n::en() { format!("{month} {d}") } else { format!("{d} {month}") }
    };
    format!("{} {when}, {:02}:{:02}", crate::i18n::pick("с", "since"), t / 60, t % 60)
}

/// Распределение аптайма по дням недели (Пн..Вс, индекс 0..6):
/// возвращает долю каждого дня (0.0..=1.0), когда ПК был включён, и индекс текущего дня `today_index`.
pub(super) fn uptime_week(now_local_minutes: i64, uptime_s: u64) -> ([f32; 7], usize) {
    let today_day = now_local_minutes.div_euclid(1440);
    let today_idx = (crate::online::weekday(today_day) as usize).min(6);
    let boot_minutes = now_local_minutes - (uptime_s / 60) as i64;
    let boot_day = boot_minutes.div_euclid(1440);
    let monday_day = today_day - today_idx as i64;

    let mut week = [0.0f32; 7];
    for (i, item) in week.iter_mut().enumerate() {
        if i > today_idx {
            *item = 0.0;
            continue;
        }
        let day_i = monday_day + i as i64;
        if day_i < boot_day {
            *item = 0.0;
        } else if day_i == boot_day && day_i == today_day {
            let mins = (now_local_minutes - boot_minutes).max(0);
            *item = (mins as f32 / 1440.0).clamp(0.0, 1.0);
        } else if day_i == boot_day {
            let boot_m_in_day = boot_minutes.rem_euclid(1440);
            let mins = 1440 - boot_m_in_day;
            *item = (mins as f32 / 1440.0).clamp(0.0, 1.0);
        } else if day_i == today_day {
            let mins_today = now_local_minutes.rem_euclid(1440);
            *item = (mins_today as f32 / 1440.0).clamp(0.0, 1.0);
        } else {
            *item = 1.0;
        }
    }
    (week, today_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_rate_parts(500.0), ("500".to_string(), "Б/с"));
        assert_eq!(fmt_rate_parts(2048.0), ("2".to_string(), "КБ/с"));
        assert_eq!(fmt_rate_parts(3.5 * 1024.0 * 1024.0), ("3.5".to_string(), "МБ/с"));

        assert_eq!(fmt_bytes(1.5 * 1024.0 * 1024.0 * 1024.0), "1.5 ГБ");
        assert_eq!(fmt_bytes(820.0 * 1024.0 * 1024.0), "820 МБ");

        // 2026-10-22 — четверг. 10:09 минус 3 д 4 ч 17 мин — 19 октября (понедельник), 05:52.
        // Uptime < 7 дней -> день недели «пн».
        let now = crate::online::days_from_civil(2026, 10, 22) * 1440 + 10 * 60 + 9;
        assert_eq!(fmt_since(now, 3 * 86_400 + 4 * 3600 + 17 * 60), "с пн, 05:52");

        // Uptime >= 7 дней -> дата с месяцем («12 окт, 05:52»)
        assert_eq!(fmt_since(now, 10 * 86_400 + 4 * 3600 + 17 * 60), "с 12 окт, 05:52");

        // Загрузка в тот же день недели неделей раньше: «с чт» читалось бы как «сегодня», поэтому дата.
        assert_eq!(fmt_since(now, 6 * 86_400 + 23 * 3600), "с 15 окт, 11:09");
    }

    #[test]
    fn test_uptime_week() {
        // 2026-10-22 — четверг (индекс 3). 14:00 (14 часов прошло = 14/24 ~ 0.5833).
        // Uptime 3 дня 14 часов = 86 часов (boot в понедельник в 00:00).
        let now = crate::online::days_from_civil(2026, 10, 22) * 1440 + 14 * 60;
        let uptime = 3 * 86_400 + 14 * 3600;
        let (week, today_idx) = uptime_week(now, uptime);
        assert_eq!(today_idx, 3);
        assert!((week[0] - 1.0).abs() < 1e-4); // Пн
        assert!((week[1] - 1.0).abs() < 1e-4); // Вт
        assert!((week[2] - 1.0).abs() < 1e-4); // Ср
        assert!((week[3] - 14.0 / 24.0).abs() < 1e-3); // Чт
        assert_eq!(week[4], 0.0); // Пт (будущее)
        assert_eq!(week[5], 0.0); // Сб (будущее)
        assert_eq!(week[6], 0.0); // Вс (будущее)
    }
}
