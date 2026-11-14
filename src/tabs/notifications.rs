//! Вкладка уведомлений: центр системных уведомлений Windows и напоминаний календаря.
//! Поддерживает отметку «Прочитано» (по одному и все сразу), контекстное меню по правой
//! кнопке мыши (отключить уведомления приложения, беззвучный режим без показа в чёлке и без
//! звука, переход в параметры Windows) с автоскрытием через 1,5 с после ухода курсора.
//! Оформление адаптировано под оба режима темы — Windows (Fluent) и macOS (Liquid Glass).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::bg::{NotifAccess, SysNotif, NOTIF_ICON_PX};
use crate::ios::SysColor;
use crate::sys;
use crate::ui::{
    bare, card, card_radius, chip_bg, dim, fg, hue, icon, ink, is_ios_palette, mix, on_color,
    press_scale, txt, Align, Canvas, Rect, Ui, WHITE,
};

use super::{escape, id, read_lines, unescape, write_lines, Action, NOTIFICATIONS, PAD, TOP};

const RULES_FILE: &str = "notif_rules.txt";
const READ_FILE: &str = "notif_read.txt";
const MAX_READ_HISTORY: usize = 200;
const MAX_CAL_ITEMS: usize = 30;

pub const CALENDAR_APP_ID: &str = "island.calendar";

const N_BTN_APPS: u32 = 1;
const N_BTN_READ_ALL: u32 = 2;
const N_BTN_CLEAR: u32 = 3;
const N_BTN_ALLOW: u32 = 4;
const N_BTN_WIN_PRIVACY: u32 = 5;
const N_SCROLL: u32 = 6;
const N_BTN_APPS_WIN_CFG: u32 = 7;

const BASE_CARD: u32 = 1000;
const BASE_READ: u32 = 2000;
const BASE_DISMISS: u32 = 3000;
const BASE_DISABLE: u32 = 4000;
const BASE_SILENT: u32 = 5000;
const BASE_WIN_CFG: u32 = 6000;
const BASE_RULE_NORM: u32 = 7000;
const BASE_RULE_SILENT: u32 = 8000;
const BASE_RULE_OFF: u32 = 9000;

/// Правило доставки уведомлений для конкретного приложения.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AppRule {
    /// Обычный режим: уведомление попадает во вкладку и всплывает в чёлке (со звуком).
    #[default]
    Normal,
    /// Беззвучный режим: уведомления приходят во вкладку, но не показываются в чёлке и без звука.
    Silent,
    /// Отключены: уведомления для этого приложения полностью подавляются.
    Disabled,
}

impl AppRule {
    fn code(self) -> &'static str {
        match self {
            Self::Normal => "N",
            Self::Silent => "S",
            Self::Disabled => "D",
        }
    }

    fn from_code(s: &str) -> Self {
        match s.trim() {
            "S" => Self::Silent,
            "D" => Self::Disabled,
            _ => Self::Normal,
        }
    }
}

#[derive(Clone, Debug)]
struct RuleEntry {
    app_id: String,
    app_name: String,
    rule: AppRule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifSource {
    Win(u32),
    Cal(String),
}

#[derive(Clone, Debug)]
pub struct NotifItem {
    pub key: String,
    pub source: NotifSource,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub time_secs: u64,
    pub icon: Option<Arc<Vec<u8>>>,
    pub is_calendar: bool,
}

/// Всплывающее уведомление для показа в свёрнутой чёлке.
#[derive(Clone, Debug)]
pub struct PopupNotice {
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub icon: Option<Arc<Vec<u8>>>,
    pub is_calendar: bool,
}

#[derive(Default)]
pub struct Notifications {
    items: Vec<NotifItem>,
    cal_items: Vec<NotifItem>,
    rules: Vec<RuleEntry>,
    read_keys: Vec<String>,
    read_set: HashSet<String>,
    seen_win_ids: HashSet<u32>,
    initial_sync_done: bool,
    last_notif_rev: u64,
    access: NotifAccess,
    dnd_active: bool,
    /// Ключ уведомления, у которого открыто меню правой кнопки мыши.
    context_menu: Option<String>,
    /// Взведён ли таймер автоскрытия контекстного меню (1,5 с после ухода мыши с карточки).
    leave_timer_armed: bool,
    /// Режим просмотра/восстановления правил приложений (отключённые / беззвучные).
    manage_apps: bool,
    /// Прямоугольники карточек в текущем кадре (в координатах содержимого вкладки) для ПКМ.
    card_hits: Vec<(String, Rect)>,
    loaded: bool,
}

impl Notifications {
    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        for line in read_lines(RULES_FILE) {
            let mut parts = line.splitn(3, '\t');
            let (Some(code), Some(id_esc), Some(name_esc)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            let rule = AppRule::from_code(code);
            let app_id = unescape(id_esc);
            let app_name = unescape(name_esc);
            if !app_id.is_empty() && rule != AppRule::Normal {
                self.rules.push(RuleEntry { app_id, app_name, rule });
            }
        }
        for line in read_lines(READ_FILE) {
            let k = line.trim().to_string();
            if !k.is_empty() && self.read_set.insert(k.clone()) {
                self.read_keys.push(k);
            }
        }
    }

    fn save_rules(&self) {
        let lines: Vec<String> = self
            .rules
            .iter()
            .filter(|r| r.rule != AppRule::Normal)
            .map(|r| format!("{}\t{}\t{}", r.rule.code(), escape(&r.app_id), escape(&r.app_name)))
            .collect();
        write_lines(RULES_FILE, &lines);
    }

    fn save_read(&mut self) {
        if self.read_keys.len() > MAX_READ_HISTORY {
            let drop_n = self.read_keys.len() - MAX_READ_HISTORY;
            for k in self.read_keys.drain(..drop_n) {
                self.read_set.remove(&k);
            }
        }
        write_lines(READ_FILE, &self.read_keys);
    }

    /// Правило доставки для приложения (`Normal`, `Silent`, `Disabled`).
    pub fn rule_of(&mut self, app_id: &str) -> AppRule {
        self.ensure_loaded();
        self.rules
            .iter()
            .find(|r| r.app_id.eq_ignore_ascii_case(app_id))
            .map_or(AppRule::Normal, |r| r.rule)
    }

    pub fn set_rule(&mut self, app_id: &str, app_name: &str, rule: AppRule) {
        self.ensure_loaded();
        if let Some(pos) = self.rules.iter().position(|r| r.app_id.eq_ignore_ascii_case(app_id)) {
            if rule == AppRule::Normal {
                self.rules.remove(pos);
            } else {
                self.rules[pos].rule = rule;
                if !app_name.is_empty() {
                    self.rules[pos].app_name = app_name.to_string();
                }
            }
        } else if rule != AppRule::Normal {
            self.rules.push(RuleEntry {
                app_id: app_id.to_string(),
                app_name: app_name.to_string(),
                rule,
            });
        }
        if rule == AppRule::Disabled {
            self.items.retain(|it| !it.app_id.eq_ignore_ascii_case(app_id));
            self.cal_items.retain(|it| !it.app_id.eq_ignore_ascii_case(app_id));
        }
        self.save_rules();
    }

    fn mark_key_read_mem(&mut self, key: &str) {
        if self.read_set.insert(key.to_string()) {
            self.read_keys.push(key.to_string());
        }
        self.items.retain(|it| it.key != key);
        self.cal_items.retain(|it| it.key != key);
        if self.context_menu.as_deref() == Some(key) {
            self.context_menu = None;
            self.leave_timer_armed = false;
        }
    }

    fn mark_key_read(&mut self, key: &str) {
        self.mark_key_read_mem(key);
        self.save_read();
    }

    fn mark_all_read_mem(&mut self) {
        for it in self.items.iter().chain(self.cal_items.iter()) {
            if self.read_set.insert(it.key.clone()) {
                self.read_keys.push(it.key.clone());
            }
        }
        self.items.clear();
        self.cal_items.clear();
        self.context_menu = None;
        self.leave_timer_armed = false;
    }

    fn mark_all_read(&mut self) {
        self.mark_all_read_mem();
        self.save_read();
    }

    /// Число непрочитанных уведомлений для значка на иконке колокольчика.
    pub fn unread_count(&self) -> usize {
        self.items.len()
    }

    /// Переключает состояние уведомлений по снимку из фонового потока.
    /// Во вкладке хранятся только непрочитанные уведомления.
    /// Возвращает всплывающие уведомления для чёлки (только новые и только с `AppRule::Normal`).
    pub fn sync_snapshot(
        &mut self,
        win_list: &[SysNotif],
        notif_rev: u64,
        access: NotifAccess,
        dnd_active: bool,
    ) -> Vec<PopupNotice> {
        self.ensure_loaded();
        self.access = access;
        self.dnd_active = dnd_active;

        let mut popups = Vec::new();
        if self.initial_sync_done && notif_rev == self.last_notif_rev {
            return popups;
        }

        let rules_copy: HashMap<String, AppRule> = self
            .rules
            .iter()
            .map(|r| (r.app_id.to_ascii_lowercase(), r.rule))
            .collect();

        let mut rebuilt = Vec::with_capacity(win_list.len() + self.cal_items.len());
        for n in win_list {
            let rule = rules_copy
                .get(&n.app_id.to_ascii_lowercase())
                .copied()
                .unwrap_or(AppRule::Normal);
            if rule == AppRule::Disabled {
                self.seen_win_ids.insert(n.id);
                continue;
            }
            let key = format!("win:{}:{}", n.id, n.time_secs);
            let is_new = self.seen_win_ids.insert(n.id);
            if self.read_set.contains(&key) {
                continue;
            }
            if self.initial_sync_done && is_new && rule == AppRule::Normal {
                popups.push(PopupNotice {
                    app_name: n.app_name.clone(),
                    title: n.title.clone(),
                    body: n.body.clone(),
                    icon: n.icon.clone(),
                    is_calendar: false,
                });
            }
            rebuilt.push(NotifItem {
                key,
                source: NotifSource::Win(n.id),
                app_id: n.app_id.clone(),
                app_name: n.app_name.clone(),
                title: n.title.clone(),
                body: n.body.clone(),
                time_secs: n.time_secs,
                icon: n.icon.clone(),
                is_calendar: false,
            });
        }

        for c in &self.cal_items {
            if self.read_set.contains(&c.key) {
                continue;
            }
            if rules_copy
                .get(&c.app_id.to_ascii_lowercase())
                .copied()
                .unwrap_or(AppRule::Normal)
                != AppRule::Disabled
            {
                rebuilt.push(c.clone());
            }
        }
        rebuilt.sort_by_key(|a| std::cmp::Reverse(a.time_secs));
        self.items = rebuilt;
        self.last_notif_rev = notif_rev;
        self.initial_sync_done = true;
        popups
    }

    /// Добавляет событие календаря во вкладку уведомлений (если приложение не отключено).
    pub fn push_calendar(&mut self, cal_id: &str, title: &str, tag: &str) -> AppRule {
        self.ensure_loaded();
        let app_name = crate::i18n::pick("Календарь", "Calendar").to_string();
        let rule = self.rule_of(CALENDAR_APP_ID);
        if rule == AppRule::Disabled {
            return AppRule::Disabled;
        }
        let key = format!("cal:{cal_id}");
        if self.read_set.contains(&key) || self.cal_items.iter().any(|it| it.key == key) {
            return rule;
        }
        let now = sys::now_secs();
        let item = NotifItem {
            key,
            source: NotifSource::Cal(cal_id.to_string()),
            app_id: CALENDAR_APP_ID.to_string(),
            app_name,
            title: title.to_string(),
            body: tag.to_string(),
            time_secs: now,
            icon: None,
            is_calendar: true,
        };
        self.cal_items.insert(0, item);
        if self.cal_items.len() > MAX_CAL_ITEMS {
            self.cal_items.truncate(MAX_CAL_ITEMS);
        }
        self.rebuild_with_cal();
        rule
    }

    fn rebuild_with_cal(&mut self) {
        self.items.retain(|it| !it.is_calendar);
        self.items.extend(self.cal_items.iter().cloned());
        self.items.sort_by_key(|a| std::cmp::Reverse(a.time_secs));
    }

    /// Обработка правого клика внутри панели вкладки. `true` — клик попал по уведомлению.
    pub fn on_right_click(&mut self, mx: f32, my: f32) -> (bool, bool) {
        if self.manage_apps {
            return (false, false);
        }
        let hit = self
            .card_hits
            .iter()
            .find(|(_, r)| r.contains((mx, my)))
            .map(|(k, _)| k.clone());
        let Some(key) = hit else {
            return (false, false);
        };
        let cancel_timer = self.leave_timer_armed;
        self.leave_timer_armed = false;
        if self.context_menu.as_deref() == Some(key.as_str()) {
            // Повторный клик правой кнопкой мыши скрывает кнопки.
            self.context_menu = None;
        } else {
            self.context_menu = Some(key);
        }
        (true, cancel_timer)
    }

    /// Таймер 1,5 с после ухода курсора с карточки: скрывает кнопки контекстного меню.
    pub fn on_menu_leave_timeout(&mut self) -> bool {
        self.leave_timer_armed = false;
        self.context_menu.take().is_some()
    }

    pub fn show(&mut self) {
        self.ensure_loaded();
        self.leave_timer_armed = false;
    }

    pub fn hide(&mut self) {
        self.context_menu = None;
        self.leave_timer_armed = false;
        self.manage_apps = false;
    }

    /// Рисует содержимое вкладки уведомлений в панели `w × h`.
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32, actions: &mut Vec<Action>) {
        self.ensure_loaded();
        self.card_hits.clear();

        let f = &cv.g.fonts;
        let ios = is_ios_palette();
        let x = PAD;
        let iw = w - 2.0 * PAD;

        // ---------- Шапка ----------
        let title = crate::i18n::pick("Уведомления", "Notifications");
        let title_w = cv.text_w(title, &f.title) + 2.0;
        cv.text(title, Rect::new(x, TOP, title_w, 22.0), &f.title, txt());

        let mut lx = x + title_w + 8.0;
        let unread = self.unread_count();
        if unread > 0 {
            let badge = unread.to_string();
            let bw = (cv.text_w(&badge, &f.small_c) + 12.0).max(20.0);
            let br = Rect::new(lx, TOP + 2.0, bw, 18.0);
            cv.rr(br, if ios { 9.0 } else { 5.0 }, cv.accent);
            cv.text(&badge, br, &f.small_c, crate::ui::readable(cv.accent, 45.0));
            lx += bw + 6.0;
        }

        if self.dnd_active {
            let dnd_lbl = crate::i18n::pick("Не беспокоить", "Do Not Disturb");
            let dw = cv.text_w(dnd_lbl, &f.small_c) + 22.0;
            if lx + dw < w - PAD - 120.0 {
                let dr = Rect::new(lx, TOP + 2.0, dw, 18.0);
                let pill_col = if ios {
                    D2D1_COLOR_F { a: 0.18, ..hue(SysColor::Indigo, cv.accent) }
                } else {
                    chip_bg()
                };
                cv.rr(dr, if ios { 9.0 } else { 5.0 }, pill_col);
                cv.symbol(icon::MOON, Rect::new(dr.x + 5.0, dr.y + 3.0, 12.0, 12.0), 10.0, txt());
                cv.text(dnd_lbl, Rect::new(dr.x + 17.0, dr.y, dw - 21.0, 18.0), &f.small_c, txt());
            }
        }

        // Правые чипы шапки: Приложения (если есть правила), Прочитать все, Очистить.
        let mut rx = w - PAD;
        if self.manage_apps {
            let back_lbl = crate::i18n::pick("Назад", "Back");
            let bw = cv.chip_w(back_lbl);
            rx -= bw;
            if ui.chip(cv, id(NOTIFICATIONS, N_BTN_APPS), Rect::new(rx, TOP + 1.0, bw, 20.0), back_lbl, true) {
                self.manage_apps = false;
            }
        } else {
            if !self.items.is_empty() {
                let clr_lbl = crate::i18n::pick("Очистить", "Clear");
                let cw = cv.chip_w(clr_lbl);
                rx -= cw;
                if ui.chip(cv, id(NOTIFICATIONS, N_BTN_CLEAR), Rect::new(rx, TOP + 1.0, cw, 20.0), clr_lbl, false) {
                    self.items.clear();
                    self.cal_items.clear();
                    self.context_menu = None;
                    crate::bg::send(crate::bg::Cmd::NotifClearAll);
                }
                rx -= 6.0;
            }
            if unread > 0 {
                let ra_lbl = crate::i18n::pick("Прочитать все", "Mark all read");
                let rw = cv.chip_w(ra_lbl);
                rx -= rw;
                if ui.chip(cv, id(NOTIFICATIONS, N_BTN_READ_ALL), Rect::new(rx, TOP + 1.0, rw, 20.0), ra_lbl, false) {
                    self.mark_all_read();
                }
                rx -= 6.0;
            }
            if !self.rules.is_empty() {
                let apps_lbl = format!("{} ({})", crate::i18n::pick("Приложения", "Apps"), self.rules.len());
                let aw = cv.chip_w(&apps_lbl);
                rx -= aw;
                if ui.chip(cv, id(NOTIFICATIONS, N_BTN_APPS), Rect::new(rx, TOP + 1.0, aw, 20.0), &apps_lbl, false) {
                    self.manage_apps = true;
                    self.context_menu = None;
                }
            }
        }

        let content_top = TOP + 28.0;
        let view = Rect::new(x, content_top, iw, (h - content_top - PAD).max(40.0));

        if self.manage_apps {
            self.draw_manage_apps(ui, cv, view);
            ui.paint_tip(cv, Rect::new(0.0, 0.0, w, h));
            return;
        }

        if self.access != NotifAccess::Allowed && self.items.is_empty() {
            self.draw_permission(ui, cv, view);
            ui.paint_tip(cv, Rect::new(0.0, 0.0, w, h));
            return;
        }

        if self.items.is_empty() {
            self.draw_empty(cv, view);
            if self.leave_timer_armed {
                self.leave_timer_armed = false;
                actions.push(Action::NotifMenuLeave(false));
            }
            self.context_menu = None;
            ui.paint_tip(cv, Rect::new(0.0, 0.0, w, h));
            return;
        }

        // ---------- Лента карточек уведомлений ----------
        let text_col_w = (view.w - 64.0).max(160.0);
        let mut layouts: Vec<CardLayout> = Vec::with_capacity(self.items.len());
        let mut total_h = 0.0f32;
        for (i, item) in self.items.iter().enumerate() {
            let menu_open = self.context_menu.as_deref() == Some(item.key.as_str());
            let show_read_btn = !menu_open;
            let title_line = if item.title.is_empty() {
                item.app_name.as_str()
            } else {
                item.title.as_str()
            };
            let title_clip = cv.wrap_clip(title_line, &f.body_b, text_col_w, 1).into_owned();
            let body_clip = if item.body.is_empty() {
                String::new()
            } else {
                cv.wrap_clip(&item.body, &f.small, text_col_w, 2).into_owned()
            };
            let body_lines = if body_clip.is_empty() {
                0
            } else if body_clip.contains('\n') || cv.text_w(&body_clip, &f.small) > text_col_w {
                2
            } else {
                1
            };
            let mut ch = 10.0 + 15.0 + 18.0 + body_lines as f32 * 14.0 + 8.0;
            if show_read_btn {
                ch += 28.0;
            } else if menu_open {
                ch += 36.0;
            }
            total_h += ch + if i + 1 < self.items.len() { 8.0 } else { 0.0 };
            layouts.push(CardLayout {
                title_clip,
                body_clip,
                body_lines,
                h: ch,
                menu_open,
                show_read_btn,
            });
        }

        let sid = id(NOTIFICATIONS, N_SCROLL);
        let off = ui.scroll_begin(cv, sid, view, total_h);
        let list_w = if total_h > view.h { view.w - 10.0 } else { view.w };

        let mut cy = view.y - off;
        let mut mark_read_key: Option<String> = None;
        let mut dismiss_item: Option<(String, NotifSource)> = None;
        let mut disable_app: Option<(String, String)> = None;
        let mut toggle_silent_app: Option<(String, String)> = None;
        let mut open_win_cfg = false;
        let mut hovering_menu_card = false;

        for (i, item) in self.items.iter().enumerate() {
            let lay = &layouts[i];
            let card_r = Rect::new(view.x, cy, list_w, lay.h);
            cy += lay.h + 8.0;
            if card_r.bottom() < view.y - 20.0 || card_r.y > view.bottom() + 20.0 {
                continue;
            }

            let visible_hit = Rect::new(
                card_r.x,
                card_r.y.max(view.y),
                card_r.w,
                (card_r.bottom().min(view.bottom()) - card_r.y.max(view.y)).max(0.0),
            );
            if visible_hit.h > 1.0 {
                self.card_hits.push((item.key.clone(), visible_hit));
            }
            let card_hot = ui.hovering(visible_hit);
            if lay.menu_open && card_hot {
                hovering_menu_card = true;
            }
            let cid = id(NOTIFICATIONS, BASE_CARD + i as u32);
            let hv = ui.hover_amount(cid, card_hot);

            let rad = if ios { card_radius().max(12.0) } else { card_radius().min(8.0) };
            let card_bg = mix(card(), fg(), 0.045 * hv + if lay.menu_open { 0.04 } else { 0.0 });
            cv.rr(card_r, rad, card_bg);

            if ios {
                cv.rr_line(
                    card_r,
                    rad,
                    D2D1_COLOR_F { a: if lay.menu_open { 0.35 } else { 0.14 }, ..fg() },
                    1.0,
                );
            } else {
                cv.rr_line(
                    card_r,
                    rad,
                    if lay.menu_open { cv.accent } else { crate::ui::hairline() },
                    1.0,
                );
                // В стиле Windows: тонкая вертикальная акцентная полоска слева у непрочитанного.
                let bar = Rect::new(card_r.x + 2.5, card_r.y + 8.0, 2.5, card_r.h - 16.0);
                cv.rr(bar, 1.25, cv.accent);
            }

            // Иконка приложения слева.
            let ix = card_r.x + if ios { 11.0 } else { 12.0 };
            let iy = card_r.y + 10.0;
            let icon_r = Rect::new(ix, iy, 30.0, 30.0);
            paint_app_icon(ui, cv, item, icon_r, ios);

            // Текстовая колонка.
            let tx = icon_r.right() + 10.0;
            let tw = (card_r.right() - tx - 12.0).max(80.0);
            let rule = self
                .rules
                .iter()
                .find(|r| r.app_id.eq_ignore_ascii_case(&item.app_id))
                .map_or(AppRule::Normal, |r| r.rule);

            // Верхняя строка: приложение · время · индикаторы справа.
            let when = if item.time_secs > 0 {
                super::ago(sys::now_secs(), item.time_secs)
            } else {
                String::new()
            };
            let meta = if when.is_empty() {
                item.app_name.clone()
            } else {
                format!("{} · {}", item.app_name, when)
            };
            let meta_w = (tw - 44.0).max(40.0);
            cv.text(&meta, Rect::new(tx, iy, meta_w, 14.0), &f.small_b, bare());

            let mut rx_ind = card_r.right() - 10.0;
            if card_hot || lay.menu_open {
                let dis_r = Rect::new(rx_ind - 16.0, iy - 1.0, 16.0, 16.0);
                if ui.icon_button(
                    cv,
                    id(NOTIFICATIONS, BASE_DISMISS + i as u32),
                    dis_r,
                    icon::CLOSE,
                    false,
                    crate::i18n::pick("Убрать уведомление", "Dismiss"),
                ) {
                    dismiss_item = Some((item.key.clone(), item.source.clone()));
                }
                rx_ind -= 20.0;
            }
            cv.circle(rx_ind - 4.0, iy + 7.0, 3.5, cv.accent);
            rx_ind -= 14.0;
            if rule == AppRule::Silent {
                cv.symbol(icon::MUTE, Rect::new(rx_ind - 14.0, iy, 13.0, 13.0), 10.5, dim());
            }

            // Заголовок и текст сообщения.
            cv.text(&lay.title_clip, Rect::new(tx, iy + 16.0, tw, 17.0), &f.body_b, txt());
            if lay.body_lines > 0 {
                let bh = lay.body_lines as f32 * 14.0 + 2.0;
                cv.text(&lay.body_clip, Rect::new(tx, iy + 34.0, tw, bh), &f.small, dim());
            }

            // Подвал карточки: кнопка «Прочитано» ИЛИ 3 кнопки по правой кнопке мыши.
            let footer_y = card_r.bottom() - if lay.menu_open { 31.0 } else { 28.0 };
            if lay.show_read_btn {
                let r_lbl = crate::i18n::pick("Прочитано", "Read");
                let rw = cv.text_w(r_lbl, &f.small_c) + 30.0;
                let btn_r = Rect::new(tx, footer_y, rw, 22.0);
                if notif_action_btn(
                    ui,
                    cv,
                    id(NOTIFICATIONS, BASE_READ + i as u32),
                    btn_r,
                    Some(icon::CHECK),
                    r_lbl,
                    BtnTone::Accent,
                ) {
                    mark_read_key = Some(item.key.clone());
                }
            } else if lay.menu_open {
                let div_y = footer_y - 5.0;
                cv.rr(
                    Rect::new(tx, div_y, (card_r.right() - 10.0 - tx).max(1.0), 1.0),
                    0.5,
                    D2D1_COLOR_F { a: 0.14, ..fg() },
                );
                let l_disable = crate::i18n::pick("Отключить уведомления", "Turn off notifications");
                let l_silent = if rule == AppRule::Silent {
                    crate::i18n::pick("Выкл. беззвучный режим", "Disable silent mode")
                } else {
                    crate::i18n::pick("Установить беззвучный режим", "Set silent mode")
                };
                let l_cfg = crate::i18n::pick("Доп. настройки", "More settings");

                let w1 = cv.text_w(l_disable, &f.small_c) + 16.0;
                let w2 = cv.text_w(l_silent, &f.small_c) + 16.0;
                let w3 = cv.text_w(l_cfg, &f.small_c) + 16.0;
                let gap = 6.0;
                let avail = card_r.right() - 10.0 - tx;
                let scale_k = ((avail - gap * 2.0) / (w1 + w2 + w3)).min(1.0);
                let bw1 = (w1 * scale_k).max(60.0);
                let bw2 = (w2 * scale_k).max(60.0);
                let bw3 = (w3 * scale_k).max(60.0);

                let r1 = Rect::new(tx, footer_y, bw1, 24.0);
                let r2 = Rect::new(r1.right() + gap, footer_y, bw2, 24.0);
                let r3 = Rect::new(r2.right() + gap, footer_y, bw3, 24.0);

                if notif_action_btn(ui, cv, id(NOTIFICATIONS, BASE_DISABLE + i as u32), r1, None, l_disable, BtnTone::Danger) {
                    disable_app = Some((item.app_id.clone(), item.app_name.clone()));
                }
                if notif_action_btn(ui, cv, id(NOTIFICATIONS, BASE_SILENT + i as u32), r2, None, l_silent, BtnTone::Silent) {
                    toggle_silent_app = Some((item.app_id.clone(), item.app_name.clone()));
                }
                if notif_action_btn(ui, cv, id(NOTIFICATIONS, BASE_WIN_CFG + i as u32), r3, None, l_cfg, BtnTone::Neutral) {
                    open_win_cfg = true;
                }
            }
        }
        ui.scroll_end(cv, sid);

        // Следим за уходом курсора с карточки с открытым контекстным меню: через 1,5 с скроем.
        if self.context_menu.is_some() {
            if !hovering_menu_card && !self.leave_timer_armed {
                self.leave_timer_armed = true;
                actions.push(Action::NotifMenuLeave(true));
            } else if hovering_menu_card && self.leave_timer_armed {
                self.leave_timer_armed = false;
                actions.push(Action::NotifMenuLeave(false));
            }
        } else if self.leave_timer_armed {
            self.leave_timer_armed = false;
            actions.push(Action::NotifMenuLeave(false));
        }

        if let Some(k) = mark_read_key {
            self.mark_key_read(&k);
        }
        if let Some((k, src)) = dismiss_item {
            self.items.retain(|it| it.key != k);
            self.cal_items.retain(|it| it.key != k);
            if self.context_menu.as_deref() == Some(k.as_str()) {
                self.context_menu = None;
            }
            if let NotifSource::Win(win_id) = src {
                crate::bg::send(crate::bg::Cmd::NotifRemove(win_id));
            }
        }
        if let Some((app_id, app_name)) = disable_app {
            self.set_rule(&app_id, &app_name, AppRule::Disabled);
            self.context_menu = None;
        }
        if let Some((app_id, app_name)) = toggle_silent_app {
            let next = if self.rule_of(&app_id) == AppRule::Silent {
                AppRule::Normal
            } else {
                AppRule::Silent
            };
            self.set_rule(&app_id, &app_name, next);
            self.context_menu = None;
        }
        if open_win_cfg {
            let _ = sys::open("ms-settings:notifications");
            self.context_menu = None;
        }

        ui.paint_tip(cv, Rect::new(0.0, 0.0, w, h));
    }

    unsafe fn draw_manage_apps(&mut self, ui: &mut Ui, cv: &Canvas, view: Rect) {
        let f = &cv.g.fonts;
        let ios = is_ios_palette();
        let rad = if ios { card_radius().max(11.0) } else { card_radius().min(8.0) };

        let cap = crate::i18n::pick("ПРАВИЛА ПРИЛОЖЕНИЙ", "APP RULES");
        cv.text(cap, Rect::new(view.x, view.y, view.w - 170.0, 14.0), &f.caps, dim());
        let cfg_lbl = crate::i18n::pick("Доп. настройки Windows", "Windows notification settings");
        let cw = cv.text_w(cfg_lbl, &f.small_c) + 16.0;
        if notif_action_btn(
            ui,
            cv,
            id(NOTIFICATIONS, N_BTN_APPS_WIN_CFG),
            Rect::new(view.right() - cw, view.y - 2.0, cw, 20.0),
            None,
            cfg_lbl,
            BtnTone::Neutral,
        ) {
            let _ = sys::open("ms-settings:notifications");
        }

        let mut y = view.y + 24.0;
        let mut change: Option<(usize, AppRule)> = None;
        for (i, r) in self.rules.iter().enumerate() {
            if y + 40.0 > view.bottom() {
                break;
            }
            let row = Rect::new(view.x, y, view.w, 36.0);
            cv.rr(row, rad, card());
            if !ios {
                cv.rr_line(row, rad, crate::ui::hairline(), 1.0);
            }
            let name = if r.app_name.is_empty() { &r.app_id } else { &r.app_name };
            cv.text(name, Rect::new(row.x + 12.0, row.y + 8.0, row.w - 260.0, 20.0), &f.body, txt());

            let l_on = crate::i18n::pick("Вкл", "On");
            let l_sil = crate::i18n::pick("Без звука", "Silent");
            let l_off = crate::i18n::pick("Выкл", "Off");
            let w_off = cv.chip_w(l_off);
            let w_sil = cv.chip_w(l_sil);
            let w_on = cv.chip_w(l_on);
            let mut bx = row.right() - 10.0;
            bx -= w_off;
            if ui.chip(cv, id(NOTIFICATIONS, BASE_RULE_OFF + i as u32), Rect::new(bx, row.y + 8.0, w_off, 20.0), l_off, r.rule == AppRule::Disabled) {
                change = Some((i, AppRule::Disabled));
            }
            bx -= w_sil + 4.0;
            if ui.chip(cv, id(NOTIFICATIONS, BASE_RULE_SILENT + i as u32), Rect::new(bx, row.y + 8.0, w_sil, 20.0), l_sil, r.rule == AppRule::Silent) {
                change = Some((i, AppRule::Silent));
            }
            bx -= w_on + 4.0;
            if ui.chip(cv, id(NOTIFICATIONS, BASE_RULE_NORM + i as u32), Rect::new(bx, row.y + 8.0, w_on, 20.0), l_on, r.rule == AppRule::Normal) {
                change = Some((i, AppRule::Normal));
            }
            y += 42.0;
        }
        if let Some((idx, next_rule)) = change {
            let (id_s, name_s) = (self.rules[idx].app_id.clone(), self.rules[idx].app_name.clone());
            self.set_rule(&id_s, &name_s, next_rule);
            if self.rules.is_empty() {
                self.manage_apps = false;
            }
        }
    }

    unsafe fn draw_permission(&self, ui: &mut Ui, cv: &Canvas, view: Rect) {
        let f = &cv.g.fonts;
        let ios = is_ios_palette();
        let rad = if ios { card_radius().max(12.0) } else { card_radius().min(8.0) };
        cv.rr(view, rad, card());
        if !ios {
            cv.rr_line(view, rad, crate::ui::hairline(), 1.0);
        }
        let cy = view.cy() - 28.0;
        cv.symbol(icon::RINGER, Rect::new(view.cx() - 14.0, cy - 34.0, 28.0, 28.0), 22.0, cv.accent);
        let msg = crate::i18n::pick(
            "Разрешите доступ к уведомлениям Windows, чтобы видеть их здесь",
            "Allow notification access in Windows to view them here",
        );
        cv.text(msg, Rect::new(view.x + 24.0, cy, view.w - 48.0, 24.0), &f.body_c, txt());

        let l_allow = crate::i18n::pick("Разрешить", "Allow");
        let l_win = crate::i18n::pick("Параметры Windows", "Windows settings");
        let bw1 = cv.text_w(l_allow, &f.small_c) + 26.0;
        let bw2 = cv.text_w(l_win, &f.small_c) + 22.0;
        let sx = view.cx() - (bw1 + bw2 + 8.0) / 2.0;
        if ui.solid_button(
            cv,
            id(NOTIFICATIONS, N_BTN_ALLOW),
            Rect::new(sx, cy + 32.0, bw1, 24.0),
            l_allow,
            cv.accent,
            crate::ui::readable(cv.accent, 45.0),
            6.0,
        ) {
            crate::bg::send(crate::bg::Cmd::NotifRequestAccess);
        }
        if ui.solid_button(
            cv,
            id(NOTIFICATIONS, N_BTN_WIN_PRIVACY),
            Rect::new(sx + bw1 + 8.0, cy + 32.0, bw2, 24.0),
            l_win,
            chip_bg(),
            txt(),
            6.0,
        ) {
            let _ = sys::open("ms-settings:privacy-notifications");
        }
    }

    unsafe fn draw_empty(&self, cv: &Canvas, view: Rect) {
        let f = &cv.g.fonts;
        cv.symbol(icon::RINGER, Rect::new(view.cx() - 14.0, view.cy() - 28.0, 28.0, 28.0), 22.0, dim());
        let msg = crate::i18n::pick("Новых уведомлений нет", "No new notifications");
        cv.text(msg, Rect::new(view.x + 20.0, view.cy() + 6.0, view.w - 40.0, 20.0), &f.body_c, dim());
        let hint = crate::i18n::pick(
            "Правый клик по уведомлению — беззвучный режим или отключение",
            "Right-click a notification for silent mode or to disable",
        );
        cv.text(hint, Rect::new(view.x + 20.0, view.cy() + 26.0, view.w - 40.0, 16.0), &f.small_c, dim());
    }
}

struct CardLayout {
    title_clip: String,
    body_clip: String,
    body_lines: usize,
    h: f32,
    menu_open: bool,
    show_read_btn: bool,
}

#[derive(Clone, Copy)]
enum BtnTone {
    Accent,
    Danger,
    Silent,
    Neutral,
}

unsafe fn notif_action_btn(
    ui: &mut Ui,
    cv: &Canvas,
    bid: u32,
    r: Rect,
    glyph: Option<&str>,
    label: &str,
    tone: BtnTone,
) -> bool {
    let ios = is_ios_palette();
    let (_, clicked) = ui.click_area(bid, r);
    let hv = ui.hover_of(bid);
    let pr = ui.press_of(bid);
    let br = press_scale(r, pr);
    let rad = if ios { br.h / 2.0 } else { 5.0 };
    let rim_hi = crate::theme::get().rim_hi;
    let red = hue(SysColor::Red, D2D1_COLOR_F { r: 0.92, g: 0.30, b: 0.28, a: 1.0 });
    let ind = hue(SysColor::Indigo, cv.accent);

    // Заливка, верхний/нижний цвета градиентного бордюра и высококонтрастный цвет текста.
    let (bg, col, border_top, border_bot, sheen_a) = if ios {
        match tone {
            BtnTone::Accent => {
                let fill = mix(cv.accent, fg(), 0.06 * hv);
                (
                    fill,
                    on_color(fill),
                    D2D1_COLOR_F { a: 0.36 + 0.14 * hv, ..rim_hi },
                    D2D1_COLOR_F { a: 0.10, ..rim_hi },
                    0.20 + 0.08 * hv,
                )
            }
            BtnTone::Danger => (
                mix(chip_bg(), red, 0.20 + 0.16 * hv),
                txt(),
                D2D1_COLOR_F { a: 0.30 + 0.14 * hv, ..rim_hi },
                D2D1_COLOR_F { a: 0.32 + 0.14 * hv, ..red },
                0.15,
            ),
            BtnTone::Silent => (
                mix(chip_bg(), ind, 0.18 + 0.16 * hv),
                txt(),
                D2D1_COLOR_F { a: 0.30 + 0.14 * hv, ..rim_hi },
                D2D1_COLOR_F { a: 0.30 + 0.14 * hv, ..ind },
                0.15,
            ),
            BtnTone::Neutral => (
                mix(chip_bg(), fg(), 0.10 * hv),
                txt(),
                D2D1_COLOR_F { a: 0.28 + 0.12 * hv, ..rim_hi },
                D2D1_COLOR_F { a: 0.12, ..fg() },
                0.14,
            ),
        }
    } else {
        // Windows Fluent 2: акцентная кнопка с AccentControlElevation и приподнятые Default/Semantic контролы.
        let base = mix(card(), fg(), 0.085 + 0.065 * hv);
        match tone {
            BtnTone::Accent => {
                let fill = mix(cv.accent, fg(), 0.08 * hv);
                (
                    fill,
                    on_color(fill),
                    D2D1_COLOR_F { a: 0.45, ..mix(fill, rim_hi, 0.32) },
                    D2D1_COLOR_F { a: 0.28, ..ink() },
                    0.15 + 0.05 * hv,
                )
            }
            BtnTone::Danger => (
                mix(base, red, 0.14 + 0.14 * hv),
                txt(),
                D2D1_COLOR_F { a: 0.36 + 0.18 * hv, ..red },
                D2D1_COLOR_F { a: 0.22 + 0.10 * hv, ..red },
                0.08,
            ),
            BtnTone::Silent => (
                mix(base, ind, 0.13 + 0.14 * hv),
                txt(),
                D2D1_COLOR_F { a: 0.34 + 0.16 * hv, ..ind },
                D2D1_COLOR_F { a: 0.20 + 0.10 * hv, ..ind },
                0.08,
            ),
            BtnTone::Neutral => (
                mix(base, fg(), 0.02 + 0.04 * hv),
                txt(),
                D2D1_COLOR_F { a: 0.18 + 0.08 * hv, ..rim_hi },
                D2D1_COLOR_F { a: 0.10, ..fg() },
                0.07,
            ),
        }
    };

    cv.rr(br, rad, bg);
    cv.gradient_stops(
        br,
        rad,
        &[
            (0.0, D2D1_COLOR_F { a: sheen_a, ..rim_hi }),
            (0.55, D2D1_COLOR_F { a: 0.0, ..rim_hi }),
        ],
        false,
    );
    cv.rr_line_grad(br, rad, border_top, border_bot, 1.0);

    if let Some(g) = glyph {
        cv.symbol(g, Rect::new(br.x + 8.0, br.y + (br.h - 11.0) / 2.0, 11.0, 11.0), 10.5, col);
        cv.line(label, &cv.g.fonts.small_c, br.x + 21.0, (br.w - 27.0).max(10.0), br.cy(), Align::Center, col);
    } else {
        cv.line(label, &cv.g.fonts.small_c, br.x + 6.0, (br.w - 12.0).max(10.0), br.cy(), Align::Center, col);
    }
    clicked
}

unsafe fn paint_app_icon(ui: &mut Ui, cv: &Canvas, item: &NotifItem, tile: Rect, ios: bool) {
    let rad = if ios { 8.0 } else { 6.0 };
    if item.is_calendar {
        let cal_col = hue(SysColor::Red, cv.accent);
        cv.rr(tile, rad, D2D1_COLOR_F { a: 0.20, ..cal_col });
        cv.symbol(icon::CALENDAR, tile, 15.0, cal_col);
        return;
    }
    if let Some(px) = &item.icon {
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        item.app_id.hash(&mut hs);
        let key = (0x4Eu64 << 56) | (hs.finish() >> 8);
        if let Some(bmp) = ui.cache.get(&cv.g.dc, key, NOTIF_ICON_PX, NOTIF_ICON_PX, || px.as_ref().clone()) {
            let bg_tile = Rect::new(tile.x, tile.y, tile.w, tile.h);
            cv.rr(bg_tile, rad, mix(card(), fg(), 0.05));
            let inner = tile.inset(3.0);
            cv.bitmap(&bmp, inner, 1.0);
            return;
        }
    }
    let ch = item
        .app_name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "W".to_string());
    let bg = D2D1_COLOR_F { a: 0.24, ..cv.accent };
    cv.rr(tile, rad, bg);
    cv.text(&ch, tile, &cv.g.fonts.body_b, WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_rules_suppress_or_silence() {
        let mut n = Notifications::default();
        n.loaded = true;
        n.initial_sync_done = true;

        n.set_rule("app.disabled", "DisabledApp", AppRule::Disabled);
        n.set_rule("app.silent", "SilentApp", AppRule::Silent);

        let batch = vec![
            SysNotif {
                id: 1,
                app_id: "app.normal".into(),
                app_name: "Normal".into(),
                title: "Hi".into(),
                body: "Body".into(),
                time_secs: 100,
                icon: None,
            },
            SysNotif {
                id: 2,
                app_id: "app.silent".into(),
                app_name: "Silent".into(),
                title: "Shh".into(),
                body: "Inbox only".into(),
                time_secs: 101,
                icon: None,
            },
            SysNotif {
                id: 3,
                app_id: "app.disabled".into(),
                app_name: "Disabled".into(),
                title: "No".into(),
                body: "Ignored".into(),
                time_secs: 102,
                icon: None,
            },
        ];

        let popups = n.sync_snapshot(&batch, 1, NotifAccess::Allowed, false);
        assert_eq!(popups.len(), 1, "всплывать должно только обычное приложение");
        assert_eq!(popups[0].app_name, "Normal");
        assert_eq!(n.items.len(), 2, "во вкладке — обычное и беззвучное; отключённое отброшено");
    }

    #[test]
    fn right_click_toggles_context_menu() {
        let mut n = Notifications::default();
        n.loaded = true;
        n.card_hits.push(("win:1:10".into(), Rect::new(10.0, 50.0, 200.0, 60.0)));

        let (hit1, _) = n.on_right_click(20.0, 70.0);
        assert!(hit1);
        assert_eq!(n.context_menu.as_deref(), Some("win:1:10"));

        let (hit2, _) = n.on_right_click(20.0, 70.0);
        assert!(hit2);
        assert_eq!(n.context_menu, None, "повторный ПКМ скрывает меню");
    }

    #[test]
    fn unread_only_filtering_and_mark_read() {
        let mut n = Notifications::default();
        n.loaded = true;
        n.initial_sync_done = true;

        let batch = vec![
            SysNotif {
                id: 10,
                app_id: "app.one".into(),
                app_name: "One".into(),
                title: "First".into(),
                body: "Body 1".into(),
                time_secs: 200,
                icon: None,
            },
            SysNotif {
                id: 11,
                app_id: "app.two".into(),
                app_name: "Two".into(),
                title: "Second".into(),
                body: "Body 2".into(),
                time_secs: 201,
                icon: None,
            },
        ];

        let _ = n.sync_snapshot(&batch, 1, NotifAccess::Allowed, false);
        assert_ne!(n.push_calendar("ev1", "Meeting", "Soon"), AppRule::Disabled);
        assert_eq!(n.unread_count(), 3);

        // Отмечаем одно уведомление прочитанным — оно сразу исчезает из списка.
        n.context_menu = Some("win:10:200".into());
        n.mark_key_read_mem("win:10:200");
        assert_eq!(n.unread_count(), 2);
        assert!(n.items.iter().all(|it| it.key != "win:10:200"));
        assert_eq!(n.context_menu, None);

        // Повторный снимок с тем же прочитанным уведомлением не возвращает его во вкладку.
        let popups = n.sync_snapshot(&batch, 2, NotifAccess::Allowed, false);
        assert!(popups.is_empty());
        assert_eq!(n.unread_count(), 2);
        assert!(n.items.iter().all(|it| it.key != "win:10:200"));

        // «Прочитать все» полностью очищает список непрочитанных.
        n.mark_all_read_mem();
        assert_eq!(n.unread_count(), 0);
        assert!(n.items.is_empty());
        assert!(n.cal_items.is_empty());

        // Прочитанные события больше не появляются при повторном снимке или push_calendar.
        let _ = n.sync_snapshot(&batch, 3, NotifAccess::Allowed, false);
        let _ = n.push_calendar("ev1", "Meeting", "Soon");
        assert_eq!(n.unread_count(), 0);
    }
}
