//! Миграция раскладок со старой мелкой сетки (шаг 16 px) на крупную (шаг 56 px, `layout::CELL`).
//!
//! Раньше у каждого виджета был свой набор размеров из мелких ячеек, а позиции считались в тех же
//! мелких ячейках. Здесь хранятся старые размеры по ключам виджетов: сохранённый размер приводится
//! к ближайшему стандартному из нового набора `Spec::sizes`, а координата делится на 4.

use super::layout::{sz, Size};

/// Сколько мелких ячеек старой сетки в одной крупной.
const FINE_PER_CELL: u16 = 4;

/// Старый шаг и зазор сетки, логических пикселей.
const FINE_CELL: f32 = 16.0;
const FINE_GAP: f32 = 8.0;

static TILE: [Size; 4] = [sz(4, 3), sz(8, 3), sz(12, 3), sz(24, 3)];
static SLIDER: [Size; 2] = [sz(12, 2), sz(24, 2)];
static PLAYER: [Size; 3] = [sz(24, 4), sz(12, 6), sz(24, 6)];
static CLOCK: [Size; 3] = [sz(8, 6), sz(8, 10), sz(16, 6)];
static WEATHER: [Size; 3] = [sz(8, 4), sz(8, 6), sz(16, 4)];
static WEEK: [Size; 4] = [sz(16, 6), sz(16, 8), sz(24, 8), sz(16, 16)];
static TIMER: [Size; 3] = [sz(16, 4), sz(16, 8), sz(24, 8)];
static TODO: [Size; 3] = [sz(16, 8), sz(16, 16), sz(24, 16)];
static RING: [Size; 3] = [sz(8, 4), sz(8, 10), sz(16, 6)];
static INFO: [Size; 2] = [sz(8, 5), sz(16, 5)];
static APPS: [Size; 3] = [sz(16, 6), sz(16, 10), sz(24, 10)];
static SPEED: [Size; 3] = [sz(8, 5), sz(16, 5), sz(24, 5)];

/// Старые размеры виджета по ключу и индекс размера, который в старой версии был по умолчанию.
fn old_sizes(key: &str) -> Option<(&'static [Size], usize)> {
    Some(match key {
        "wifi" | "bluetooth" | "audio" => (&TILE, 1),
        "night" | "dark" => (&TILE, 2),
        "volume" | "brightness" => (&SLIDER, 1),
        "media" => (&PLAYER, 2),
        "clock" => (&CLOCK, 1),
        "weather" => (&WEATHER, 1),
        "week" => (&WEEK, 1),
        "timer" => (&TIMER, 1),
        "todo" => (&TODO, 1),
        "cpu" | "gpu" | "ram" => (&RING, 1),
        "disk" | "net" | "uptime" => (&INFO, 0),
        "apps" => (&APPS, 1),
        "speed" => (&SPEED, 1),
        _ => return None,
    })
}

/// Координата старой сетки → крупная ячейка (с округлением к ближайшей).
pub fn migrate_cell(fine: u8) -> u8 {
    ((u16::from(fine) + FINE_PER_CELL / 2) / FINE_PER_CELL) as u8
}

/// Индекс в `new` (новые стандартные размеры) для старого размера виджета `key`: выбирается
/// размер, ближайший по пикселям. `old` — сохранённый индекс старого размера, `None` — размер
/// по умолчанию той версии. `None` в ответе — виджет незнаком.
pub fn migrate_size(key: &str, new: &[Size], old: Option<usize>) -> Option<usize> {
    let (olds, def) = old_sizes(key)?;
    let from = olds[old.filter(|&n| n < olds.len()).unwrap_or(def)];
    let (want_w, want_h) = (f32::from(from.w) * FINE_CELL - FINE_GAP, f32::from(from.h) * FINE_CELL - FINE_GAP);
    new.iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let cost = |s: &Size| (s.pixel_w() - want_w).abs() + (s.pixel_h() - want_h).abs();
            cost(a).total_cmp(&cost(b))
        })
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_round_to_nearest() {
        assert_eq!(migrate_cell(0), 0);
        assert_eq!(migrate_cell(1), 0);
        assert_eq!(migrate_cell(2), 1);
        assert_eq!(migrate_cell(8), 2);
        assert_eq!(migrate_cell(10), 3);
        assert_eq!(migrate_cell(24), 6);
    }

    #[test]
    fn unknown_key_is_not_migrated() {
        assert_eq!(migrate_size("nope", &[sz(1, 1)], Some(0)), None);
    }

    #[test]
    fn every_old_size_maps_to_a_new_one() {
        use crate::tabs::{controls, stats, today};
        for spec in [&controls::SPEC, &today::SPEC, &stats::SPEC] {
            for s in spec.specs {
                let (olds, def) = old_sizes(s.key).unwrap_or_else(|| panic!("нет старых размеров для {}", s.key));
                assert!(def < olds.len());
                for old in 0..olds.len() {
                    let n = migrate_size(s.key, s.sizes, Some(old)).unwrap();
                    assert!(n < s.sizes.len(), "{}: {old} → {n}", s.key);
                }
                // Размер по умолчанию старой версии (индекс не записывался) тоже переносится.
                assert_eq!(migrate_size(s.key, s.sizes, None), migrate_size(s.key, s.sizes, Some(def)));
            }
        }
    }

    #[test]
    fn exact_sizes_keep_their_pixels() {
        // Недельная карточка 248×120 в новой версии — 4×2, как была 16×8 в мелких ячейках.
        let new = [sz(4, 2), sz(6, 2), sz(4, 4)];
        assert_eq!(migrate_size("week", &new, Some(1)), Some(0));
        assert_eq!(migrate_size("week", &new, Some(2)), Some(1));
        assert_eq!(migrate_size("week", &new, Some(3)), Some(2));
    }
}
