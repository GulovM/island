//! Восход и закат по солнечным формулам NOAA (склонение и уравнение времени по дню года).
//! Чистая арифметика без аллокаций: для циферблата «Небо» кэш не нужен.

use std::f64::consts::{PI, TAU};

use crate::online::{civil_from_days, days_from_civil};

/// Зенитное расстояние центра Солнца на восходе и закате: 90° + рефракция + радиус диска.
const ZENITH_DEG: f64 = 90.833;

/// Минуты от местной полуночи (в часовом поясе компьютера, `utc_off_min`) восхода и заката
/// в день `day` (дни от 1970-01-01) в точке (`lat`, `lon`). `None` — полярный день или ночь.
pub(super) fn sun_times(day: i64, lat: f64, lon: f64, utc_off_min: i64) -> Option<(u16, u16)> {
    let (year, _, _) = civil_from_days(day);
    // Дробный год в радианах на полдень (номер дня с нуля).
    let g = TAU / 365.0 * (day - days_from_civil(year, 1, 1)) as f64;
    let eq_time = 229.18
        * (0.000075 + 0.001868 * g.cos() - 0.032077 * g.sin() - 0.014615 * (2.0 * g).cos() - 0.040849 * (2.0 * g).sin());
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos() + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    let lat_r = lat.to_radians();
    let cos_ha = ZENITH_DEG.to_radians().cos() / (lat_r.cos() * decl.cos()) - lat_r.tan() * decl.tan();
    if !(-1.0..=1.0).contains(&cos_ha) {
        return None;
    }
    let ha = cos_ha.acos() * 180.0 / PI;
    // Минуты UTC → местные; 4 минуты на градус долготы.
    let local = |utc: f64| (utc + utc_off_min as f64).rem_euclid(1440.0).round() as u16 % 1440;
    Some((local(720.0 - 4.0 * (lon + ha) - eq_time), local(720.0 - 4.0 * (lon - ha) - eq_time)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(got: u16, h: u16, m: u16) -> bool {
        (i32::from(got) - i32::from(h * 60 + m)).abs() <= 3
    }

    #[test]
    fn moscow_in_november() {
        let day = days_from_civil(2026, 11, 13);
        let (rise, set) = sun_times(day, 55.75, 37.62, 180).unwrap();
        // Восход по формуле NOAA 7:59 (день 8 ч 30 м, как у реальной Москвы в середине ноября);
        // «7:50» из задания — приблизительное значение макета.
        assert!(near(rise, 7, 59), "{rise}");
        assert!(near(set, 16, 31), "{set}");
    }

    #[test]
    fn dushanbe_in_november() {
        let day = days_from_civil(2026, 11, 13);
        let (rise, set) = sun_times(day, 38.56, 68.79, 300).unwrap();
        // День ~10 ч 15 м, полдень около 12:09.
        assert!(rise > 6 * 60 + 45 && rise < 7 * 60 + 20, "{rise}");
        assert!(set > 17 * 60 && set < 17 * 60 + 35, "{set}");
        assert!(rise < set);
    }

    #[test]
    fn polar_day_and_night_give_none() {
        assert!(sun_times(days_from_civil(2026, 6, 21), 80.0, 20.0, 60).is_none());
        assert!(sun_times(days_from_civil(2026, 12, 21), 80.0, 20.0, 60).is_none());
    }
}
