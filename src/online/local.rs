//! События, созданные в самом острове: запись в файле, повторы и напоминания.
//! В календаре они выглядят как обычные `RawEvent`, поэтому неделя, месяц и «ближайшее событие»
//! работают с ними без отдельной логики.

use super::calendar::{merge_raw_events, occurrence_from, RawEvent};

/// Как часто событие повторяется.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    None,
    Daily,
    /// По дням недели из `LocalEvent::days`; если дни не выбраны — по дню недели начала.
    Weekly,
    Monthly,
    Yearly,
}

impl Repeat {
    const fn code(self) -> &'static str {
        match self {
            Self::None => "-",
            Self::Daily => "d",
            Self::Weekly => "w",
            Self::Monthly => "m",
            Self::Yearly => "y",
        }
    }

    fn from_code(s: &str) -> Self {
        match s {
            "d" => Self::Daily,
            "w" => Self::Weekly,
            "m" => Self::Monthly,
            "y" => Self::Yearly,
            _ => Self::None,
        }
    }

    pub const ALL: [Repeat; 5] = [Self::None, Self::Daily, Self::Weekly, Self::Monthly, Self::Yearly];

    /// Подпись по-русски и по-английски.
    pub const fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::None => ("Не повторять", "No repeat"),
            Self::Daily => ("Каждый день", "Daily"),
            Self::Weekly => ("Каждую неделю", "Weekly"),
            Self::Monthly => ("Каждый месяц", "Monthly"),
            Self::Yearly => ("Каждый год", "Yearly"),
        }
    }
}

/// Варианты «напомнить за», минут; 0 — без напоминания.
pub const REMIND_CHOICES: [u32; 5] = [0, 5, 10, 30, 60];

/// Сколько событий можно создать в острове.
pub const MAX_LOCAL: usize = 200;

/// На сколько назад от «сейчас» ещё напоминаем о пропущенном событии (остров был выключен,
/// экран погашен или компьютер заблокирован): одни сутки.
pub const MISSED_WINDOW: i64 = 1440;

const ENG_DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalEvent {
    pub id: u64,
    pub title: String,
    /// Начало в местном времени: минуты от 1970-01-01 (у события на весь день — полночь).
    pub start: i64,
    /// Конец; у события на весь день — начало следующего дня. 0 — не задан.
    pub end: i64,
    pub all_day: bool,
    pub repeat: Repeat,
    /// Маска дней недели для недельного повтора: бит 0 — понедельник, бит 6 — воскресенье.
    pub days: u8,
    /// Напомнить за столько минут до начала; 0 — не напоминать.
    pub remind: u32,
}

impl LocalEvent {
    /// Запись одной строкой: заголовок идёт последним, чтобы в нём мог быть любой символ.
    pub fn to_line(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.id,
            self.start,
            self.end,
            u8::from(self.all_day),
            self.repeat.code(),
            self.days,
            self.remind,
            crate::tabs::escape(&self.title)
        )
    }

    pub fn from_line(line: &str) -> Option<Self> {
        let mut p = line.splitn(8, '|');
        let id = p.next()?.parse().ok()?;
        let start = p.next()?.parse().ok()?;
        let end = p.next()?.parse().ok()?;
        let all_day = p.next()? == "1";
        let repeat = Repeat::from_code(p.next()?);
        let days = p.next()?.parse::<u8>().ok()? & 0x7f;
        let remind = p.next()?.parse().ok()?;
        let title = crate::tabs::unescape(p.next()?);
        Some(Self { id, title, start, end, all_day, repeat, days, remind })
    }

    fn rrule(&self) -> Option<String> {
        Some(match self.repeat {
            Repeat::None => return None,
            Repeat::Daily => "FREQ=DAILY".to_string(),
            Repeat::Weekly => {
                let days: Vec<&str> = (0..7).filter(|i| self.days >> i & 1 == 1).map(|i| ENG_DAYS[i]).collect();
                if days.is_empty() {
                    "FREQ=WEEKLY".to_string()
                } else {
                    format!("FREQ=WEEKLY;BYDAY={}", days.join(","))
                }
            }
            Repeat::Monthly => "FREQ=MONTHLY".to_string(),
            Repeat::Yearly => "FREQ=YEARLY".to_string(),
        })
    }

    pub fn to_raw(&self) -> RawEvent {
        RawEvent {
            start: self.start,
            end: self.end,
            all_day: self.all_day,
            title: self.title.clone(),
            rrule: self.rrule(),
            exdates: Vec::new(),
            uid: format!("island-local-{}", self.id),
        }
    }
}

/// События из календарей по ссылкам и созданные в острове — одним списком.
pub fn merge_local(remote: &[RawEvent], local: &[LocalEvent]) -> Vec<RawEvent> {
    merge_raw_events([remote.to_vec(), local.iter().map(LocalEvent::to_raw).collect()])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DueKind {
    /// Скоро начало: за столько минут.
    Soon(u32),
    /// Событие началось.
    Started,
}

/// Напоминание, время которого наступило.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    pub id: u64,
    pub title: String,
    /// Начало вхождения (местные минуты).
    pub start: i64,
    pub all_day: bool,
    pub kind: DueKind,
    /// Когда напоминание должно было прозвучать.
    pub at: i64,
}

/// Напоминания, время которых попало в `(last, now]`; самые ранние первыми. Если вхождение уже
/// началось, «скоро начнётся» не показывается: достаточно «началось».
pub fn due_reminders(events: &[LocalEvent], offset: i64, last: i64, now: i64) -> Vec<Due> {
    let mut out = Vec::new();
    if now <= last {
        return out;
    }
    for ev in events {
        let raw = ev.to_raw();
        let remind = ev.remind as i64;
        let mut from = last + 1;
        for _ in 0..32 {
            let Some(occ) = occurrence_from(&raw, from, offset) else { break };
            if occ > now + remind {
                break;
            }
            let fire = if occ > last && occ <= now {
                Some((DueKind::Started, occ))
            } else if remind > 0 && occ - remind > last && occ - remind <= now {
                Some((DueKind::Soon(ev.remind), occ - remind))
            } else {
                None
            };
            if let Some((kind, at)) = fire {
                out.push(Due { id: ev.id, title: ev.title.clone(), start: occ, all_day: ev.all_day, kind, at });
            }
            from = if ev.all_day { (occ.div_euclid(1440) + 1) * 1440 } else { occ + 1 };
        }
    }
    out.sort_by_key(|d| (d.at, d.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online::{days_from_civil, utc_offset_minutes};

    fn ev(id: u64, start: i64) -> LocalEvent {
        LocalEvent { id, title: format!("e{id}"), start, end: 0, all_day: false, repeat: Repeat::None, days: 0, remind: 0 }
    }

    fn at(y: i32, m: u32, d: u32, h: i64, min: i64) -> i64 {
        days_from_civil(y, m, d) * 1440 + h * 60 + min
    }

    #[test]
    fn line_roundtrip_keeps_separators_in_title() {
        let mut e = ev(7, at(2026, 10, 25, 9, 30));
        e.title = "a|b\nc\\d".into();
        e.repeat = Repeat::Weekly;
        e.days = 0b0000101;
        e.remind = 10;
        e.end = e.start + 60;
        assert_eq!(LocalEvent::from_line(&e.to_line()), Some(e));
    }

    #[test]
    fn bad_line_is_rejected() {
        assert_eq!(LocalEvent::from_line("x|1|2"), None);
        assert_eq!(LocalEvent::from_line(""), None);
    }

    #[test]
    fn rrule_for_weekly_days() {
        let mut e = ev(1, 0);
        e.repeat = Repeat::Weekly;
        e.days = 0b0010001;
        assert_eq!(e.rrule().as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,FR"));
        e.days = 0;
        assert_eq!(e.rrule().as_deref(), Some("FREQ=WEEKLY"));
    }

    #[test]
    fn reminder_and_start_fire_once_each() {
        let off = utc_offset_minutes();
        let start = at(2026, 10, 25, 10, 0);
        let mut e = ev(1, start);
        e.remind = 10;
        let evs = [e];
        assert!(due_reminders(&evs, off, start - 30, start - 11).is_empty());
        let d = due_reminders(&evs, off, start - 11, start - 9);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DueKind::Soon(10));
        assert!(due_reminders(&evs, off, start - 9, start - 1).is_empty());
        let d = due_reminders(&evs, off, start - 1, start);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DueKind::Started);
    }

    #[test]
    fn missed_event_shows_only_started() {
        let off = utc_offset_minutes();
        let start = at(2026, 10, 25, 10, 0);
        let mut e = ev(1, start);
        e.remind = 10;
        let d = due_reminders(&[e], off, start - 60, start + 30);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DueKind::Started);
    }

    #[test]
    fn daily_event_fires_every_day() {
        let off = utc_offset_minutes();
        let mut e = ev(1, at(2026, 10, 1, 8, 0));
        e.repeat = Repeat::Daily;
        let d = due_reminders(&[e], off, at(2026, 10, 10, 0, 0), at(2026, 10, 12, 23, 59));
        let days: Vec<i64> = d.iter().map(|x| x.start.div_euclid(1440)).collect();
        assert_eq!(days, vec![days_from_civil(2026, 10, 10), days_from_civil(2026, 10, 11), days_from_civil(2026, 10, 12)]);
    }

    #[test]
    fn all_day_event_fires_once() {
        let off = utc_offset_minutes();
        let day = days_from_civil(2026, 10, 25) * 1440;
        let mut e = ev(1, day);
        e.all_day = true;
        e.end = day + 1440;
        let d = due_reminders(&[e], off, day - 5, day + 600);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn local_events_merge_into_calendar() {
        let e = ev(3, at(2026, 10, 25, 9, 0));
        let merged = merge_local(&[], &[e]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].uid, "island-local-3");
    }
}
