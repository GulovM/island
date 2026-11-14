//! События, созданные в острове: загрузка и сохранение, слияние с календарём по ссылкам,
//! поиск напоминаний, время которых наступило.

use std::sync::Arc;

use super::alarm::play_ding;
use super::Today;
use crate::online::{self, Due, DueKind, LocalEvent};
use crate::tabs::{read_lines, write_lines};

const EVENTS_FILE: &str = "events.txt";
/// Минута, до которой напоминания уже проверены: после перезапуска не повторяем показанные.
const SEEN_FILE: &str = "events_seen.txt";

/// Что показать в уведомлении о событии.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub cal_id: String,
    pub title: String,
    /// Короткая подпись справа: «Сейчас», «10 мин».
    pub tag: String,
}

fn notice_of(due: &[Due]) -> Option<Notice> {
    let last = due.last()?;
    let cal_id = format!("{}:{}:{:?}", last.id, last.at, last.kind);
    let title = if due.len() > 1 { format!("{} (+{})", last.title, due.len() - 1) } else { last.title.clone() };
    let tag = match last.kind {
        DueKind::Started => crate::i18n::pick("Сейчас", "Now").to_string(),
        DueKind::Soon(m) if m >= 60 && m % 60 == 0 => format!("{} {}", m / 60, crate::i18n::pick("ч", "h")),
        DueKind::Soon(m) => format!("{m} {}", crate::i18n::pick("мин", "min")),
    };
    Some(Notice { cal_id, title, tag })
}

impl Today {
    pub fn load_events(&mut self) {
        self.local = read_lines(EVENTS_FILE).iter().filter_map(|l| LocalEvent::from_line(l)).collect();
        online::set_local_snapshot(&self.local);
        self.local_ver += 1;
        let now = online::now_local_minutes();
        self.seen_min = read_lines(SEEN_FILE).first().and_then(|l| l.trim().parse().ok()).unwrap_or(now);
    }

    fn save_events(&mut self) {
        let lines: Vec<String> = self.local.iter().map(LocalEvent::to_line).collect();
        write_lines(EVENTS_FILE, &lines);
        online::set_local_snapshot(&self.local);
        self.local_ver += 1;
        self.sync();
    }

    /// Свободный номер для нового события.
    fn next_event_id(&self) -> u64 {
        self.local.iter().map(|e| e.id).max().map_or(1, |m| m + 1)
    }

    /// Добавляет событие или заменяет существующее с тем же `id` (0 — новое). `false` — достигнут предел.
    /// `sync` — записать событие в Google Календарь (снятая отметка удалит его оттуда).
    pub(super) fn save_event(&mut self, mut ev: LocalEvent, sync: bool) -> bool {
        let id = match self.local.iter().position(|e| e.id == ev.id) {
            Some(i) if ev.id != 0 => {
                let id = ev.id;
                self.local[i] = ev;
                id
            }
            _ => {
                if self.local.len() >= online::MAX_LOCAL {
                    return false;
                }
                ev.id = self.next_event_id();
                let id = ev.id;
                self.local.push(ev);
                id
            }
        };
        // Снимок событий обновляется в `save_events`, поэтому отправка идёт уже с новыми данными.
        self.save_events();
        online::set_synced(id, sync);
        online::flush_google_push();
        true
    }

    pub(super) fn remove_event(&mut self, id: u64) {
        self.local.retain(|e| e.id != id);
        self.save_events();
        online::set_synced(id, false);
        online::flush_google_push();
    }

    /// Календарь по ссылкам и события острова одним списком; пересобирается только при смене любого из них.
    pub(super) fn merge_events(&mut self) {
        let key = (self.online.cal_ver, self.local_ver);
        let list = match &self.merged {
            Some((k, list)) if *k == key => list.clone(),
            _ => {
                let list = Arc::new(online::merge_local_synced(&self.online.cal, &self.local, &online::synced_gids()));
                self.merged = Some((key, list.clone()));
                list
            }
        };
        self.online.cal = list;
        // Ключ кэша месячного вида меняется и от смены календаря, и от смены своих событий.
        self.online.cal_ver += self.local_ver;
    }

    /// Напоминание, время которого наступило. `ready` — экран включён и компьютер разблокирован:
    /// иначе ничего не показываем и не отмечаем проверенным. Пропущенное за последние сутки
    /// показывается при следующей проверке.
    pub fn take_due(&mut self, ready: bool, with_sound: bool) -> Option<Notice> {
        if !ready {
            return None;
        }
        let now = online::now_local_minutes();
        let from = self.seen_min.max(now - online::MISSED_WINDOW);
        let offset = online::utc_offset_minutes();
        let mut due = online::due_reminders(&self.local, offset, from, now);
        // Приближающиеся (за 10 мин) и начавшиеся события из подключённых календарей.
        let today = now.div_euclid(1440);
        if let Some(day_list) = online::events_by_day(&self.online.cal, today, 1, offset).into_iter().next() {
            for (idx, ev) in day_list.into_iter().enumerate() {
                if ev.local_id.is_some() {
                    continue;
                }
                let Some(m) = ev.minute else { continue };
                let ev_min = today * 1440 + m;
                let id = 1_000_000 + idx as u64;
                if ev_min > from && ev_min <= now {
                    due.push(Due { id, title: ev.title.clone(), start: ev_min, all_day: false, kind: DueKind::Started, at: ev_min });
                } else if ev_min - 10 > from && ev_min - 10 <= now {
                    due.push(Due { id, title: ev.title, start: ev_min, all_day: false, kind: DueKind::Soon(10), at: ev_min - 10 });
                }
            }
            due.sort_by_key(|d| (d.at, d.start));
        }
        if now > self.seen_min {
            self.seen_min = now;
            write_lines(SEEN_FILE, &[now.to_string()]);
        }
        let notice = notice_of(&due)?;
        if with_sound {
            play_ding();
        }
        Some(notice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn due(title: &str, kind: DueKind) -> Due {
        Due { id: 1, title: title.into(), start: 0, all_day: false, kind, at: 0 }
    }

    #[test]
    fn notice_counts_extra_events() {
        let n = notice_of(&[due("a", DueKind::Started), due("b", DueKind::Started)]).unwrap();
        assert_eq!(n.title, "b (+1)");
    }

    #[test]
    fn notice_tag_for_soon() {
        crate::i18n::set(crate::i18n::Lang::Ru);
        assert_eq!(notice_of(&[due("a", DueKind::Soon(10))]).unwrap().tag, "10 мин");
        assert_eq!(notice_of(&[due("a", DueKind::Soon(60))]).unwrap().tag, "1 ч");
        assert!(notice_of(&[]).is_none());
    }
}
