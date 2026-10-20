//! Вкладка Apps: индекс ярлыков меню «Пуск», нечёткий поиск, секции «Избранное / Недавние /
//! Приложения», счётчик запусков, быстрые ссылки (сниппеты) из `[snippets]` конфига.
//! Иконки берутся из Shell в фоновом потоке (`icons`), UWP-приложения добавляются из
//! `shell:AppsFolder`; пока иконка не пришла, в строке — первая буква названия.

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
/// Глиф «молния» для быстрых ссылок.
const BOLT: &str = "\u{E945}";

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

    /// Список под запрос с названием секции для каждого элемента. Без запроса: избранное,
    /// недавние, остальное по алфавиту; с запросом — одним списком по убыванию совпадения.
    fn results(&self, query: &str) -> Vec<(Item, &'static str)> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            let mut out = Vec::new();
            for (i, e) in self.entries.iter().enumerate() {
                if e.fav {
                    out.push((Item::App(i), "ИЗБРАННОЕ"));
                }
            }
            let mut recent: Vec<usize> =
                (0..self.entries.len()).filter(|&i| !self.entries[i].fav && self.entries[i].launches > 0).collect();
            recent.sort_by_key(|&i| std::cmp::Reverse(self.entries[i].last));
            recent.truncate(RECENT_COUNT);
            for &i in &recent {
                out.push((Item::App(i), "НЕДАВНИЕ"));
            }
            for (i, e) in self.entries.iter().enumerate() {
                if !e.fav && !recent.contains(&i) {
                    out.push((Item::App(i), "ПРИЛОЖЕНИЯ"));
                }
            }
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
        let items = self.results(&query);

        // Клавиатура: стрелки двигают выбор, Ctrl+P — избранное, Enter запускает.
        for n in ui.take_nav() {
            match n {
                Nav::Down => self.sel = Some(self.sel.map_or(0, |s| (s + 1).min(items.len().saturating_sub(1)))),
                Nav::Up => self.sel = Some(self.sel.map_or(0, |s| s.saturating_sub(1))),
                Nav::Pin => {
                    if let Some((Item::App(i), _)) = self.sel.and_then(|s| items.get(s)).copied() {
                        self.entries[i].fav = !self.entries[i].fav;
                        self.save();
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
        let mut y_list = TOP + 33.0;
        let mut launch = None;
        if query.trim().is_empty() && !self.snippets.is_empty() {
            let mut x = PAD;
            for (i, (name, _)) in self.snippets.iter().enumerate() {
                let pw = name.chars().count() as f32 * 5.2 + 30.0;
                if x + pw > w - PAD {
                    break;
                }
                let r = Rect::new(x, TOP + 35.0, pw, 20.0);
                let pid = id(APPS, 600 + i as u32);
                let (_, clicked) = ui.click_area(pid, r);
                let hv = ui.hover_of(pid);
                cv.rr(r, 10.0, mix(card(), fg(), 0.06 * hv));
                cv.circle(r.x + 10.0, r.cy(), 6.5, mix(card(), cv.accent, 0.3));
                cv.text(BOLT, Rect::new(r.x + 3.5, r.y, 13.0, r.h), &f.icon, cv.accent);
                cv.text(name, Rect::new(r.x + 21.0, r.y, pw - 23.0, r.h), &f.small_b, txt());
                if clicked {
                    launch = Some(Item::Snippet(i));
                }
                x += pw + 5.0;
            }
            y_list = TOP + 59.0;
        }

        // Список с заголовками секций.
        const ROW: f32 = 24.0;
        const PITCH: f32 = 26.0;
        const HEAD: f32 = 18.0;
        let view = Rect::new(PAD, y_list, w - 2.0 * PAD, (h - y_list - 8.0).max(ROW));
        let mut rows: Vec<(f32, Option<&'static str>, usize)> = Vec::new();
        let (mut y, mut last) = (0.0f32, None::<&'static str>);
        for (p, &(_, sec)) in items.iter().enumerate() {
            if !sec.is_empty() && last != Some(sec) {
                rows.push((y, Some(sec), p));
                y += HEAD;
                last = Some(sec);
            }
            rows.push((y, None, p));
            y += PITCH;
        }

        let sid = id(APPS, 100);
        let off = ui.scroll_begin(cv, sid, view, y);
        let mut fav = None;
        for (ry, head, p) in rows {
            let yy = view.y + ry - off;
            let hgt = if head.is_some() { HEAD } else { ROW };
            if yy + hgt < view.y || yy > view.bottom() {
                continue;
            }
            if let Some(title) = head {
                cv.text(title, Rect::new(view.x + 6.0, yy + 4.0, view.w, 12.0), &f.caps, bare());
                continue;
            }
            let (it, _) = items[p];
            let r = Rect::new(view.x, yy, view.w - 4.0, ROW);
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
                    let right = if selected { "↵ Открыть" } else { "Приложение" };
                    // Звезда регистрирует нажатие раньше строки, иначе строка захватит мышь
                    // и клик по звезде запустит приложение.
                    let star = Rect::new(r.right() - 25.0, yy + 3.0, 18.0, 18.0);
                    let sid_star = id(APPS, 5000 + p as u32);
                    let show = e.fav || ui.hover_of(rid) > 0.3 || ui.hover_of(sid_star) > 0.3;
                    if show {
                        ui.click_area(sid_star, star);
                    }
                    if ui.row_compact(cv, rid, r, &letter, false, &e.name, right, 26.0, selected, false) {
                        launch = Some(it);
                    }
                    if let Some(px) = &e.icon {
                        // Иконка поверх плашки строки (той же геометрии, что у `row_compact`).
                        let ts = (r.h - 7.0).clamp(12.0, 17.0);
                        let tile = Rect::new(r.x + 6.0, r.cy() - ts / 2.0, ts, ts);
                        let key = {
                            use std::hash::{Hash, Hasher};
                            let mut hs = std::collections::hash_map::DefaultHasher::new();
                            e.path.hash(&mut hs);
                            (0x41u64 << 56) | (hs.finish() >> 8)
                        };
                        if let Some(bmp) = ui.cache.get(&cv.g.dc, key, icons::SIZE_PX, icons::SIZE_PX, || px.clone()) {
                            cv.bitmap(&bmp, tile, 1.0);
                        }
                    }
                    // Звезда видна у избранного и при наведении на строку.
                    if show {
                        let glyph = if e.fav { icon::STAR_FILL } else { icon::STAR };
                        if ui.icon_button(cv, sid_star, star, glyph, e.fav) {
                            fav = Some(i);
                        }
                    }
                }
                Item::Snippet(i) => {
                    let (name, _) = &self.snippets[i];
                    let right = if selected { "↵ Открыть" } else { "Ссылка" };
                    if ui.row_compact(cv, rid, r, icon::LINK, true, name, right, 26.0, selected, false) {
                        launch = Some(it);
                    }
                }
            }
        }
        ui.scroll_end(cv, sid);
        if items.is_empty() {
            let msg = if self.entries.is_empty() { "Ярлыки меню «Пуск» не найдены" } else { "Ничего не найдено" };
            cv.text(msg, Rect::new(view.x + 6.0, view.y + 6.0, view.w, 18.0), &f.small, bare());
        }
        if let Some(i) = fav {
            self.entries[i].fav = !self.entries[i].fav;
            self.save();
        } else if let Some(it) = launch {
            self.launch(it, act);
        }
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
    fn sections_without_query() {
        let mut a = Apps::default();
        a.entries = vec![entry("alpha", false, 0), entry("beta", true, 3), entry("gamma", false, 5), entry("delta", false, 0)];
        let r = a.results("");
        assert_eq!(r[0], (Item::App(1), "ИЗБРАННОЕ"));
        assert_eq!(r[1], (Item::App(2), "НЕДАВНИЕ"));
        assert_eq!(r[2], (Item::App(0), "ПРИЛОЖЕНИЯ"));
        assert_eq!(r[3], (Item::App(3), "ПРИЛОЖЕНИЯ"));
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
