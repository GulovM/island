//! Вкладка Apps: индекс ярлыков меню «Пуск», нечёткий поиск, секции «Избранное / Недавние /
//! Приложения», счётчик запусков, быстрые ссылки (сниппеты) из `[snippets]` конфига.
//! Иконки берутся из Shell в фоновом потоке (`icons`), UWP-приложения добавляются из
//! `shell:AppsFolder`; пока иконка не пришла, в строке — первая буква названия.
//! Вид (список / сетка) и режим «Избранные / Все приложения» запоминаются в `state.txt`. Режим
//! «Избранные» включён, только если есть избранные; без них сразу виден полный список. В полном
//! списке сверху недавние, ниже все приложения по алфавиту с буквенными заголовками и
//! алфавитной полосой справа.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::icons;
use crate::sys;
use crate::ui::{icon, mix, Canvas, Nav, Rect, Submit, Ui, bare, card, fg, txt};

use super::{id, read_lines, write_lines, Action, APPS, PAD, TOP};

const FILE: &str = "apps.txt";
/// Пути, скрытые пользователем клавишей Delete.
const HIDDEN_FILE: &str = "apps_hidden.txt";
const MAX_DEPTH: u32 = 4;
const RECENT_COUNT: usize = 5;
/// Заголовок секции недавних (для буквенной полосы не буква).
const RECENT: &str = "НЕДАВНИЕ";
/// Ширина алфавитной полосы у правого края списка.
const BAR_W: f32 = 16.0;
/// Размеры плитки в виде «сетка».
const CELL_W: f32 = 84.0;
const CELL_H: f32 = 64.0;
/// Сторона иконки в плитке сетки.
const ICON_BIG: f32 = 30.0;
struct Entry {
    name: String,
    lower: String,
    path: PathBuf,
    launches: u32,
    last: u64,
    fav: bool,
    /// Иконка `icons::SIZE_PX`² BGRA premultiplied (когда загрузится).
    icon: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct Apps {
    entries: Vec<Entry>,
    indexed: bool,
    snippets: Vec<(String, String)>,
    /// Выбранная позиция в текущем списке (для клавиатуры).
    sel: Option<usize>,
    /// Сохранённая статистика по пути: (запусков, время последнего, избранное). Хранится и для
    /// приложений, которых сейчас нет в индексе (UWP подгружаются позже).
    stats: HashMap<String, (u32, u64, bool)>,
    icons_gen: u32,
    hidden: HashSet<String>,
    /// Вид «сетка» вместо списка.
    grid: bool,
    /// Выбран полный список (иначе только избранные, если они есть).
    show_all: bool,
    /// Смещение списка в прошлом кадре: движение подсвечивает букву на алфавитной полосе.
    last_off: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    App(usize),
    Snippet(usize),
}

impl Apps {
    pub fn configure(&mut self, cfg: &Config) {
        self.snippets = cfg.snippets.clone();
    }

    /// Восстанавливает сохранённые вид, сортировку и режим.
    pub fn load(&mut self, grid: bool, show_all: bool) {
        self.grid = grid;
        self.show_all = show_all;
    }

    fn persist(&self) {
        crate::state::set_apps_view(self.grid, self.show_all);
    }

    fn has_fav(&self) -> bool {
        self.entries.iter().any(|e| e.fav)
    }

    /// Показан полный список: выбран явно либо избранных нет вовсе.
    fn all_mode(&self) -> bool {
        self.show_all || !self.has_fav()
    }

    /// Ставит или снимает звезду. Первая звезда при пустом избранном не должна прятать
    /// остальные приложения, поэтому запоминает режим «все».
    fn toggle_fav(&mut self, i: usize) {
        if !self.has_fav() && !self.show_all {
            self.show_all = true;
            self.persist();
        }
        self.entries[i].fav = !self.entries[i].fav;
        self.save();
    }

    pub fn show(&mut self) -> bool {
        if self.indexed {
            return false;
        }
        self.indexed = true;
        self.build_index();
        true
    }

    fn build_index(&mut self) {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for (var, tail) in [("ProgramData", "Programs"), ("APPDATA", "Programs")] {
            if let Some(base) = std::env::var_os(var) {
                dirs.push(PathBuf::from(base).join("Microsoft").join("Windows").join("Start Menu").join(tail));
            }
        }
        let mut files = Vec::new();
        for d in &dirs {
            scan(d, 0, &mut files);
        }
        self.hidden = read_lines(HIDDEN_FILE).into_iter().filter(|l| !l.is_empty()).collect();
        let mut entries: Vec<Entry> = Vec::new();
        for p in files {
            if p.to_str().is_some_and(|s| self.hidden.contains(s)) {
                continue;
            }
            let Some(name) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
            let lower = name.to_lowercase();
            if lower.contains("uninstall") || lower.contains("удалить") || lower.contains("удаление") {
                continue;
            }
            if entries.iter().any(|e| e.lower == lower) {
                continue;
            }
            entries.push(Entry { name, lower, path: p, launches: 0, last: 0, fav: false, icon: None });
        }
        for line in read_lines(FILE) {
            let mut parts = line.splitn(4, '|');
            let (Some(l), Some(t), Some(f), Some(path)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            self.stats.insert(path.to_string(), (l.parse().unwrap_or(0), t.parse().unwrap_or(0), f == "1"));
        }
        for e in &mut entries {
            if let Some(&(l, t, f)) = e.path.to_str().and_then(|p| self.stats.get(p)) {
                e.launches = l;
                e.last = t;
                e.fav = f;
            }
        }
        entries.sort_by(|a, b| a.lower.cmp(&b.lower));
        self.entries = entries;
        let paths = self.entries.iter().map(|e| e.path.display().to_string()).collect();
        self.icons_gen = icons::request(paths);
    }

    /// Забирает иконки и UWP-приложения из фонового потока. `true` — список изменился.
    pub fn sync(&mut self) -> bool {
        if self.icons_gen == 0 {
            return false;
        }
        let found = icons::drain(self.icons_gen);
        if found.is_empty() {
            return false;
        }
        let (mut resort, mut dead) = (false, false);
        for f in found {
            match f {
                icons::Found::Icon(path, px) => {
                    if let Some(e) = self.entries.iter_mut().find(|e| e.path.to_str() == Some(path.as_str())) {
                        e.icon = Some(px);
                    }
                }
                icons::Found::Dead(path) => {
                    self.entries.retain(|e| e.path.to_str() != Some(path.as_str()));
                    self.stats.remove(&path);
                    dead = true;
                }
                icons::Found::Uwp(u) => {
                    if self.hidden.contains(&u.target) {
                        continue;
                    }
                    let lower = u.name.to_lowercase();
                    if self.entries.iter().any(|e| e.lower == lower) {
                        continue;
                    }
                    let (launches, last, fav) = self.stats.get(&u.target).copied().unwrap_or((0, 0, false));
                    self.entries.push(Entry {
                        name: u.name,
                        lower,
                        path: PathBuf::from(u.target),
                        launches,
                        last,
                        fav,
                        icon: Some(u.px),
                    });
                    resort = true;
                }
            }
        }
        if resort {
            self.entries.sort_by(|a, b| a.lower.cmp(&b.lower));
        }
        if dead {
            // Статистика мёртвых ярлыков больше не нужна.
            self.save();
        }
        true
    }

    fn save(&mut self) {
        for e in &self.entries {
            let Some(p) = e.path.to_str() else { continue };
            if e.launches > 0 || e.fav {
                self.stats.insert(p.to_string(), (e.launches, e.last, e.fav));
            } else {
                self.stats.remove(p);
            }
        }
        let lines: Vec<String> = self
            .stats
            .iter()
            .map(|(path, (l, t, f))| format!("{}|{}|{}|{}", l, t, if *f { 1 } else { 0 }, path))
            .collect();
        write_lines(FILE, &lines);
    }

    /// Список под запрос с названием секции для каждого элемента. Без запроса: только избранные
    /// по алфавиту (если они есть и выбран этот режим) либо недавние и все приложения по
    /// алфавиту с буквенными заголовками; с запросом — всё найденное по убыванию совпадения.
    fn results(&self, query: &str) -> Vec<(Item, &'static str)> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            // `entries` всегда отсортирован по имени.
            if !self.all_mode() {
                return (0..self.entries.len()).filter(|&i| self.entries[i].fav).map(|i| (Item::App(i), "")).collect();
            }
            let mut out = Vec::new();
            let mut recent: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].launches > 0).collect();
            recent.sort_by_key(|&i| std::cmp::Reverse(self.entries[i].last));
            recent.truncate(RECENT_COUNT);
            out.extend(recent.into_iter().map(|i| (Item::App(i), RECENT)));
            out.extend(self.entries.iter().enumerate().map(|(i, e)| (Item::App(i), letter_head(&e.lower))));
            return out;
        }
        let mut scored: Vec<(i32, Item)> = Vec::new();
        for (i, (name, _)) in self.snippets.iter().enumerate() {
            if let Some(s) = fuzzy(&q, &name.to_lowercase()) {
                // Быстрые ссылки выше приложений.
                scored.push((s + 10_000, Item::Snippet(i)));
            }
        }
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(s) = fuzzy(&q, &e.lower) {
                let bonus = (e.launches.min(50) as i32) * 2 + if e.fav { 40 } else { 0 };
                scored.push((s + bonus, Item::App(i)));
            }
        }
        scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        scored.into_iter().map(|(_, it)| (it, "")).collect()
    }

    /// Убирает запись из списка насовсем (для ярлыков, которые не определить как мёртвые).
    fn hide(&mut self, i: usize) {
        let e = self.entries.remove(i);
        if let Some(p) = e.path.to_str() {
            self.stats.remove(p);
            self.hidden.insert(p.to_string());
        }
        let mut lines: Vec<String> = self.hidden.iter().cloned().collect();
        lines.sort();
        write_lines(HIDDEN_FILE, &lines);
        self.save();
    }

    fn launch(&mut self, it: Item, act: &mut Vec<Action>) {
        match it {
            Item::App(i) => {
                let e = &mut self.entries[i];
                // Отказ Shell (цель удалена) не считается запуском и не закрывает остров.
                if !sys::open(&e.path.display().to_string()) {
                    return;
                }
                e.launches += 1;
                e.last = sys::now_secs();
                self.save();
            }
            Item::Snippet(i) => {
                if !sys::open(&self.snippets[i].1) {
                    return;
                }
            }
        }
        act.push(Action::Collapse);
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32, act: &mut Vec<Action>) {
        let f = &cv.g.fonts;
        let fid = id(APPS, 0);
        ui.text_field_ex(
            cv,
            fid,
            Rect::new(PAD, TOP, w - 2.0 * PAD, 28.0),
            "Поиск приложений, файлов и ссылок…",
            icon::SEARCH,
            Submit::Keep,
        );
        let query = ui.field_text(fid);
        let searching = !query.trim().is_empty();

        // Ряд управления: слева «‹ Избранные» (в полном списке), справа «Все приложения ›»
        // (в избранном) и кнопка вида: её значок показывает текущий вид. Во время поиска ряд
        // скрыт — поиск идёт по всем приложениям.
        let mut y_list = TOP + 33.0;
        if !searching {
            let y = TOP + 34.0;
            let view_btn = Rect::new(w - PAD - 22.0, y - 1.0, 22.0, 19.0);
            let glyph = if self.grid { icon::APPS } else { icon::LIST };
            let tip = if self.grid { crate::i18n::pick("Список", "List") } else { crate::i18n::pick("Сетка", "Grid") };
            if ui.icon_button(cv, id(APPS, 13), view_btn, glyph, false, tip) {
                self.grid = !self.grid;
                self.sel = None;
                self.persist();
            }
            if self.has_fav() {
                if self.all_mode() {
                    if nav_label(ui, cv, id(APPS, 11), PAD, y, "Избранные", true) {
                        self.show_all = false;
                        self.sel = None;
                        self.persist();
                    }
                } else {
                    let x = view_btn.x - 8.0 - nav_label_w(cv, "Все приложения");
                    if nav_label(ui, cv, id(APPS, 10), x, y, "Все приложения", false) {
                        self.show_all = true;
                        self.sel = None;
                        self.persist();
                    }
                }
            }
            y_list = TOP + 55.0;
        }

        let items = self.results(&query);
        let bar_w = if !searching && self.all_mode() && !items.is_empty() { BAR_W } else { 0.0 };
        let cols = if self.grid { (((w - 2.0 * PAD - bar_w - 4.0) / CELL_W) as usize).max(1) } else { 1 };

        // Клавиатура: стрелки двигают выбор, Ctrl+P — избранное, Enter запускает.
        for n in ui.take_nav() {
            match n {
                Nav::Down => self.sel = Some(self.sel.map_or(0, |s| (s + cols).min(items.len().saturating_sub(1)))),
                Nav::Up => self.sel = Some(self.sel.map_or(0, |s| s.saturating_sub(cols))),
                Nav::Pin => {
                    if let Some((Item::App(i), _)) = self.sel.and_then(|s| items.get(s)).copied() {
                        self.toggle_fav(i);
                    }
                }
                Nav::Delete => {
                    if let Some((Item::App(i), _)) = self.sel.and_then(|s| items.get(s)).copied() {
                        self.hide(i);
                        return;
                    }
                }
                Nav::Copy | Nav::Tab => {}
            }
        }
        if ui.take_submitted(fid).is_some() {
            if let Some(&(it, _)) = self.sel.and_then(|s| items.get(s)).or_else(|| items.first()) {
                self.launch(it, act);
                return;
            }
        }

        // Быстрые ссылки плашками под поиском (пока запрос пуст).
        let mut launch = None;
        if !searching && !self.snippets.is_empty() {
            let mut x = PAD;
            for (i, (name, _)) in self.snippets.iter().enumerate() {
                let pw = name.chars().count() as f32 * 5.2 + 30.0;
                if x + pw > w - PAD {
                    break;
                }
                let r = Rect::new(x, y_list + 2.0, pw, 20.0);
                let pid = id(APPS, 600 + i as u32);
                let (_, clicked) = ui.click_area(pid, r);
                let hv = ui.hover_of(pid);
                cv.rr(r, 10.0, mix(card(), fg(), 0.06 * hv));
                cv.circle(r.x + 10.0, r.cy(), 6.5, mix(card(), cv.accent, 0.3));
                cv.symbol(icon::BOLT, Rect::new(r.x + 3.5, r.y, 13.0, r.h), 9.0, cv.accent);
                cv.text(name, Rect::new(r.x + 21.0, r.y, pw - 23.0, r.h), &f.small_b, txt());
                if clicked {
                    launch = Some(Item::Snippet(i));
                }
                x += pw + 5.0;
            }
            y_list += 26.0;
        }

        // Список (или сетка) с заголовками секций.
        const ROW: f32 = 24.0;
        const PITCH: f32 = 26.0;
        const HEAD: f32 = 18.0;
        let view = Rect::new(PAD, y_list, w - 2.0 * PAD - bar_w, (h - y_list - 8.0).max(ROW));
        let pitch = if self.grid { CELL_H } else { PITCH };
        // (y, заголовок секции, позиция в `items`, колонка)
        let mut rows: Vec<(f32, Option<&'static str>, usize, usize)> = Vec::new();
        // Буквенные заголовки с их положением в содержимом: якоря алфавитной полосы.
        let mut anchors: Vec<(&'static str, f32)> = Vec::new();
        let (mut y, mut last, mut col) = (0.0f32, None::<&'static str>, 0usize);
        for (p, &(_, sec)) in items.iter().enumerate() {
            if !sec.is_empty() && last != Some(sec) {
                if col != 0 {
                    y += pitch;
                    col = 0;
                }
                rows.push((y, Some(sec), p, 0));
                if sec != RECENT {
                    anchors.push((sec, y));
                }
                y += HEAD;
                last = Some(sec);
            }
            rows.push((y, None, p, col));
            col += 1;
            if col == cols {
                col = 0;
                y += pitch;
            }
        }
        if col != 0 {
            y += pitch;
        }
        let cell_w = (view.w - 4.0) / cols as f32;

        // Своя прокрутка на каждую комбинацию режима и вида: переключение «Избранные» ↔
        // «Все приложения» возвращает на прежнее место списка.
        let sid = id(APPS, 100 + self.all_mode() as u32 * 2 + self.grid as u32);

        // Алфавитная полоса: нажатие и протяжка по ней прокручивают к букве. Разбор нажатия идёт
        // до `scroll_begin`, чтобы список уже в этом кадре встал на нужное место.
        let bar = (bar_w > 0.0 && !anchors.is_empty()).then(|| {
            let area = Rect::new(view.right() + 2.0, y_list, bar_w - 2.0, view.h);
            let n = anchors.len();
            // Букв больше, чем влезает: рисуем каждую `stride`-ю, а нажатие всё равно точное.
            let natural = area.h / n as f32;
            let stride = if natural >= 9.0 { 1 } else { (9.0 / natural).ceil() as usize };
            let shown = n.div_ceil(stride);
            let slot = (area.h / shown as f32).min(15.0);
            let top = area.y + (area.h - slot * shown as f32) / 2.0;
            (area, stride, shown, slot, top)
        });
        let mut touched = None;
        if let Some((area, stride, shown, slot, top)) = bar {
            let hit = Rect::new(area.x - 6.0, area.y, area.w + 6.0, area.h);
            let bid = id(APPS, 14);
            let (_, clicked) = ui.click_area(bid, hit);
            if ui.is_dragging(bid) || clicked {
                let d = (((ui.pointer().1 - top) / slot).floor().max(0.0) as usize).min(shown - 1);
                let a = (d * stride).min(anchors.len() - 1);
                ui.scroll_jump(sid, anchors[a].1);
                touched = Some(a);
            }
        }

        let off = ui.scroll_begin(cv, sid, view, y);
        let mut fav = None;
        for (ry, head, p, col) in rows {
            let yy = view.y + ry - off;
            let hgt = if head.is_some() { HEAD } else if self.grid { CELL_H } else { ROW };
            if yy + hgt < view.y || yy > view.bottom() {
                continue;
            }
            if let Some(title) = head {
                cv.text(title, Rect::new(view.x + 6.0, yy + 4.0, view.w, 12.0), &f.caps, crate::ui::dim());
                continue;
            }
            let (it, _) = items[p];
            let r = if self.grid {
                Rect::new(view.x + col as f32 * cell_w, yy, cell_w - 2.0, CELL_H - 4.0)
            } else {
                Rect::new(view.x, yy, view.w - 4.0, ROW)
            };
            let selected = self.sel == Some(p);
            let rid = id(APPS, 1000 + p as u32);
            match it {
                Item::App(i) => {
                    let e = &self.entries[i];
                    let letter: String = if e.icon.is_some() {
                        String::new()
                    } else {
                        e.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()
                    };
                    // Звезда регистрирует нажатие раньше строки, иначе строка захватит мышь
                    // и клик по звезде запустит приложение.
                    let star = if self.grid {
                        Rect::new(r.right() - 20.0, r.y + 2.0, 18.0, 18.0)
                    } else {
                        Rect::new(r.right() - 25.0, yy + 3.0, 18.0, 18.0)
                    };
                    let sid_star = id(APPS, 5000 + p as u32);
                    let show = e.fav || ui.hover_of(rid) > 0.3 || ui.hover_of(sid_star) > 0.3;
                    if show {
                        ui.click_area(sid_star, star);
                    }
                    if self.grid {
                        let (_, clicked) = ui.click_area(rid, r);
                        let hv = ui.hover_of(rid);
                        cv.rr(r, 10.0, mix(card(), fg(), 0.06 * hv + if selected { 0.08 } else { 0.0 }));
                        let tile = Rect::new(r.cx() - ICON_BIG / 2.0, r.y + 7.0, ICON_BIG, ICON_BIG);
                        if !paint_icon(ui, cv, e, tile) {
                            cv.circle(tile.cx(), tile.cy(), ICON_BIG / 2.0, mix(card(), cv.accent, 0.25));
                            cv.text(&letter, tile, &f.body_c, txt());
                        }
                        cv.text(&e.name, Rect::new(r.x + 3.0, r.y + 7.0 + ICON_BIG + 4.0, r.w - 6.0, 14.0), &f.small_c, txt());
                        if clicked {
                            launch = Some(it);
                        }
                    } else {
                        let right = if selected { "↵ Открыть" } else { "Приложение" };
                        if ui.row_compact(cv, rid, r, &letter, false, &e.name, right, 26.0, selected, false) {
                            launch = Some(it);
                        }
                        // Иконка поверх плашки строки (той же геометрии, что у `row_compact`).
                        let ts = (r.h - 7.0).clamp(12.0, 17.0);
                        paint_icon(ui, cv, e, Rect::new(r.x + 6.0, r.cy() - ts / 2.0, ts, ts));
                    }
                    // Звезда видна у избранного и при наведении на строку.
                    if show {
                        let glyph = if e.fav { icon::STAR_FILL } else { icon::STAR };
                        let tip = if e.fav {
                            crate::i18n::pick("Убрать из избранного", "Remove from favorites")
                        } else {
                            crate::i18n::pick("В избранное", "Add to favorites")
                        };
                        // Звезда избранного — жёлтая, как в iOS.
                        if ui.icon_button_tinted(cv, sid_star, star, glyph, e.fav, crate::ui::hue(crate::ios::SysColor::Yellow, cv.accent), tip) {
                            fav = Some(i);
                        }
                    }
                }
                Item::Snippet(i) => {
                    let (name, _) = &self.snippets[i];
                    if self.grid {
                        let (_, clicked) = ui.click_area(rid, r);
                        let hv = ui.hover_of(rid);
                        cv.rr(r, 10.0, mix(card(), fg(), 0.06 * hv + if selected { 0.08 } else { 0.0 }));
                        let tile = Rect::new(r.cx() - ICON_BIG / 2.0, r.y + 7.0, ICON_BIG, ICON_BIG);
                        cv.circle(tile.cx(), tile.cy(), ICON_BIG / 2.0, crate::ui::hue(crate::ios::SysColor::Blue, cv.accent));
                        cv.symbol(icon::LINK, tile, 14.0, crate::ui::WHITE);
                        cv.text(name, Rect::new(r.x + 3.0, r.y + 7.0 + ICON_BIG + 4.0, r.w - 6.0, 14.0), &f.small_c, txt());
                        if clicked {
                            launch = Some(it);
                        }
                    } else {
                        let right = if selected { "↵ Открыть" } else { "Ссылка" };
                        if ui.row_compact_tinted(cv, rid, r, icon::LINK, true, name, right, 26.0, selected, false, Some(crate::ui::hue(crate::ios::SysColor::Blue, cv.accent))) {
                            launch = Some(it);
                        }
                    }
                }
            }
        }
        ui.scroll_end(cv, sid);

        // Буквы полосы: текущая — акцентом; пока полосу трогают, наводят на неё курсор или
        // список едет, буквы рядом с текущей вырастают и сдвигаются влево-вверх.
        let now = ui.scroll_offset(sid);
        if let Some((area, stride, shown, slot, top)) = bar {
            let active = touched
                .unwrap_or_else(|| anchors.iter().rposition(|&(_, ay)| ay <= now + 1.0).unwrap_or(0));
            let hot = touched.is_some()
                || ui.hovering(Rect::new(area.x - 6.0, area.y, area.w + 6.0, area.h))
                || (now - self.last_off).abs() > 0.5;
            let act_d = active as f32 / stride as f32;
            for d in 0..shown {
                let a = (d * stride).min(anchors.len() - 1);
                let near = if hot { (1.0 - (d as f32 - act_d).abs() / 3.0).clamp(0.0, 1.0) } else { 0.0 };
                let near = near * near * (3.0 - 2.0 * near);
                let k = ui.anim_to(id(APPS, 3000 + d as u32), near);
                let cy = top + slot * (d as f32 + 0.5);
                let col = if d == active / stride { cv.accent } else { mix(crate::ui::dim(), txt(), k) };
                let r = Rect::new(area.x - 11.0 * k, cy - 8.0 - 3.5 * k, area.w, 16.0);
                cv.text_scaled(anchors[a].0, r, &f.small_c, col, 1.0 + 0.75 * k);
            }
        }
        self.last_off = now;

        if items.is_empty() {
            let msg = if self.entries.is_empty() { "Ярлыки меню «Пуск» не найдены" } else { "Ничего не найдено" };
            cv.text(msg, Rect::new(view.x + 6.0, view.y + 6.0, view.w - 12.0, 18.0), &f.small, bare());
        }
        if let Some(i) = fav {
            self.toggle_fav(i);
        } else if let Some(it) = launch {
            self.launch(it, act);
        }
        ui.paint_tip(cv, Rect::new(0.0, 0.0, w, h));
    }
}

/// Ширина текстовой кнопки со стрелкой.
unsafe fn nav_label_w(cv: &Canvas, text: &str) -> f32 {
    cv.chip_w(text) - crate::ui::CHIP_PAD + 17.0
}

/// Кнопка-ярлык со стрелкой: «‹ Избранные» (`back`) или «Все приложения ›». При наведении
/// текст светлеет, а стрелка чуть сдвигается в сторону перехода. Возвращает `true` по клику.
unsafe fn nav_label(ui: &mut Ui, cv: &Canvas, bid: u32, x: f32, y: f32, text: &str, back: bool) -> bool {
    let total = nav_label_w(cv, text);
    let tw = total - 17.0;
    let r = Rect::new(x, y, total, 18.0);
    let (_, clicked) = ui.click_area(bid, r);
    let hv = ui.hover_of(bid);
    let col = mix(crate::ui::dim(), txt(), 0.35 + 0.65 * hv);
    let f = &cv.g.fonts;
    if back {
        let chev = Rect::new(x - 1.5 * hv, y, 12.0, 18.0);
        cv.symbol(icon::LEFT, chev, 10.0, cv.accent);
        cv.text(text, Rect::new(x + 14.0, y, tw + 6.0, 18.0), &f.small_b, col);
    } else {
        cv.text(text, Rect::new(x, y, tw + 6.0, 18.0), &f.small_b, col);
        let chev = Rect::new(x + tw + 6.0 + 1.5 * hv, y, 12.0, 18.0);
        cv.symbol(icon::RIGHT, chev, 10.0, cv.accent);
    }
    clicked
}

/// Заголовок секции по первой букве имени (в нижнем регистре): латиница и кириллица (Ё как Е),
/// всё остальное — «#».
fn letter_head(lower: &str) -> &'static str {
    const LATIN: [&str; 26] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X",
        "Y", "Z",
    ];
    const CYR: [&str; 32] = [
        "А", "Б", "В", "Г", "Д", "Е", "Ж", "З", "И", "Й", "К", "Л", "М", "Н", "О", "П", "Р", "С", "Т", "У", "Ф", "Х", "Ц", "Ч",
        "Ш", "Щ", "Ъ", "Ы", "Ь", "Э", "Ю", "Я",
    ];
    let c = match lower.chars().next() {
        Some('ё') => 'е',
        Some(c) => c,
        None => return "#",
    };
    if c.is_ascii_lowercase() {
        LATIN[(c as u8 - b'a') as usize]
    } else if ('а'..='я').contains(&c) {
        CYR[(c as u32 - 'а' as u32) as usize]
    } else {
        "#"
    }
}

/// Рисует иконку приложения в `tile`; `false` — иконка ещё не загружена.
unsafe fn paint_icon(ui: &mut Ui, cv: &Canvas, e: &Entry, tile: Rect) -> bool {
    let Some(px) = &e.icon else { return false };
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        e.path.hash(&mut hs);
        (0x41u64 << 56) | (hs.finish() >> 8)
    };
    match ui.cache.get(&cv.g.dc, key, icons::SIZE_PX, icons::SIZE_PX, || px.clone()) {
        Some(bmp) => {
            cv.bitmap(&bmp, tile, 1.0);
            true
        }
        None => false,
    }
}

fn scan(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < MAX_DEPTH {
                scan(&p, depth + 1, out);
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("lnk") || x.eq_ignore_ascii_case("url"))
        {
            out.push(p);
        }
    }
}

/// Оценка совпадения запроса с именем (оба в нижнем регистре): подстрока выше, чем
/// последовательность символов с пропусками; `None` — не подходит. Пустой запрос подходит всему.
pub fn fuzzy(q: &str, name: &str) -> Option<i32> {
    if q.is_empty() {
        return Some(0);
    }
    if let Some(byte_pos) = name.find(q) {
        let pos = name[..byte_pos].chars().count() as i32;
        let start_bonus = if pos == 0 { 200 } else { 0 };
        return Some(1000 + start_bonus - pos.min(100));
    }
    let mut it = name.chars().enumerate();
    let (mut last, mut gaps) = (None::<usize>, 0i32);
    for c in q.chars() {
        let (i, _) = it.find(|&(_, n)| n == c)?;
        if let Some(l) = last {
            gaps += (i - l - 1) as i32;
        }
        last = Some(i);
    }
    Some((500 - gaps * 5).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: &str, fav: bool, l: u32) -> Entry {
        Entry { name: n.into(), lower: n.into(), path: PathBuf::from(n), launches: l, last: l as u64, fav, icon: None }
    }

    #[test]
    fn fuzzy_ranks_prefix_over_substring_over_subsequence() {
        let prefix = fuzzy("note", "notepad").unwrap();
        let inner = fuzzy("pad", "notepad").unwrap();
        let sub = fuzzy("ntp", "notepad").unwrap();
        assert!(prefix > inner && inner > sub);
        assert_eq!(fuzzy("xyz", "notepad"), None);
        assert_eq!(fuzzy("", "anything"), Some(0));
    }

    #[test]
    fn fuzzy_handles_cyrillic() {
        assert!(fuzzy("блок", "блокнот").is_some());
        assert!(fuzzy("бнт", "блокнот").is_some());
    }

    #[test]
    fn without_favorites_everything_is_shown_recent_first_then_alphabet() {
        let mut a = Apps::default();
        // `entries` отсортирован по имени, как после `build_index`.
        a.entries = vec![entry("alpha", false, 0), entry("beta", false, 3), entry("delta", false, 0), entry("gamma", false, 5)];
        assert!(a.all_mode());
        let r = a.results("");
        assert_eq!(
            r,
            vec![
                (Item::App(3), RECENT),
                (Item::App(1), RECENT),
                (Item::App(0), "A"),
                (Item::App(1), "B"),
                (Item::App(2), "D"),
                (Item::App(3), "G"),
            ]
        );
    }

    #[test]
    fn favorites_mode_lists_only_favorites_alphabetically() {
        let mut a = Apps::default();
        a.entries = vec![entry("alpha", true, 1), entry("beta", false, 9), entry("delta", true, 4)];
        assert!(!a.all_mode());
        assert_eq!(a.results(""), vec![(Item::App(0), ""), (Item::App(2), "")]);
        a.show_all = true;
        assert!(a.all_mode());
        assert_eq!(a.results("").len(), 3 + 3);
    }

    #[test]
    fn first_star_keeps_the_full_list() {
        let mut a = Apps::default();
        a.entries = vec![entry("alpha", false, 0), entry("beta", false, 0)];
        assert!(a.all_mode());
        a.toggle_fav(0);
        assert!(a.entries[0].fav);
        assert!(a.show_all && a.all_mode());
        // Снятие последней звезды снова ведёт к полному списку.
        a.show_all = false;
        a.toggle_fav(0);
        assert!(a.all_mode());
    }

    #[test]
    fn search_ignores_favorites_only_mode() {
        let mut a = Apps::default();
        a.entries = vec![entry("alpha", false, 0), entry("gamma", true, 5)];
        assert_eq!(a.results("alp")[0].0, Item::App(0));
    }

    #[test]
    fn letter_heads() {
        assert_eq!(letter_head("notepad"), "N");
        assert_eq!(letter_head("блокнот"), "Б");
        assert_eq!(letter_head("ёлка"), "Е");
        assert_eq!(letter_head("7-zip"), "#");
        assert_eq!(letter_head("über"), "#");
        assert_eq!(letter_head(""), "#");
    }

    #[test]
    fn search_puts_snippets_first() {
        let mut a = Apps::default();
        a.entries = vec![entry("alpha", false, 0), entry("gamma", false, 5)];
        a.snippets.push(("gmail".into(), "https://mail.google.com".into()));
        let r = a.results("gma");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].0, Item::Snippet(0));
        assert_eq!(r[1].0, Item::App(1));
        assert_eq!(r[0].1, "");
    }
}
