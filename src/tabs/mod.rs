//! Вкладки острова (этап 3). Каждая вкладка — отдельный модуль со своим состоянием;
//! рисуется и опрашивает данные только пока она видна (`show`/`hide`/`poll`).
//!
//! Файлы данных лежат в `%APPDATA%\island`, по одной записи в строке:
//! - `todo.txt` — `D|текст` (D = 1, если выполнено);
//! - `clipboard.txt` — `P|unix_секунды|текст` (P = 1, если закреплено; `\n`, `\r`, `\t`, `\\` экранированы);
//! - `apps.txt` — `запусков|unix_секунды|избранное|путь`;
//! - `wallpapers.txt` — пути недавних обоев, новые сверху.

use std::path::PathBuf;
use std::time::Instant;

use crate::config::Config;
use crate::ui::{Canvas, Ui};

pub mod apps;
pub mod clipboard;
pub mod controls;
pub mod power;
pub mod settings;
pub mod stats;
pub mod today;
pub mod wallpaper;

/// Индексы вкладок в `TABS` (main.rs).
pub const CONTROLS: usize = 0;
pub const TODAY: usize = 1;
pub const STATS: usize = 2;
pub const CLIPBOARD: usize = 3;
pub const APPS: usize = 4;
pub const WALLPAPER: usize = 5;
pub const POWER: usize = 6;
/// Страница настроек: не вкладка ряда, а отдельный вид острова (шестерёнка); номер идёт после вкладок.
pub const SETTINGS: usize = 7;

/// Верх области содержимого под рядом вкладок и боковое поле.
pub const TOP: f32 = 35.0;
pub const PAD: f32 = 11.5;

/// Просьба вкладки к главному окну.
pub enum Action {
    /// Положить текст в буфер обмена, свернуть остров и вставить в окно, где работали.
    Paste(String),
    /// Только положить текст в буфер обмена.
    Copy(String),
    /// То же для изображения из истории (имя PNG-файла в `clip`).
    PasteImage(String),
    CopyImage(String),
    /// Свернуть остров (после запуска приложения и т. п.).
    Collapse,
}

pub fn id(tab: usize, n: u32) -> u32 {
    ((tab as u32 + 1) << 16) | n
}

fn data_path(name: &str) -> PathBuf {
    crate::sys::data_dir().join(name)
}

pub fn read_lines(name: &str) -> Vec<String> {
    std::fs::read_to_string(data_path(name))
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn write_lines(name: &str, lines: &[String]) {
    let dir = crate::sys::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut text = lines.join("\n");
    text.push('\n');
    if let Err(e) = std::fs::write(dir.join(name), text) {
        crate::log::error(&format!("не удалось записать {name}: {e}"));
    }
}

/// Экранирует перевод строки и обратную косую для хранения в одной строке файла.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// «5 мин назад», «3 ч назад» и т. п. по разнице в секундах.
pub fn ago(now: u64, then: u64) -> String {
    let d = now.saturating_sub(then);
    match d {
        0..=9 => "только что".into(),
        10..=59 => format!("{d} с назад"),
        60..=3599 => format!("{} мин назад", d / 60),
        3600..=86_399 => format!("{} ч назад", d / 3600),
        _ => format!("{} дн назад", d / 86_400),
    }
}

#[derive(Default)]
pub struct Tabs {
    power: power::Power,
    stats: stats::Stats,
    controls: controls::Controls,
    today: today::Today,
    clip: clipboard::Clip,
    apps: apps::Apps,
    wall: wallpaper::Wall,
    pub settings: settings::Settings,
    pub actions: Vec<Action>,
}

impl Tabs {
    /// Загружает сохранённые данные (один раз при старте).
    pub fn load(&mut self, cfg: &Config) {
        self.today.load();
        self.today.configure(cfg);
        self.clip.load();
        self.apps.configure(cfg);
        self.wall.configure(cfg);
    }

    pub fn apply_config(&mut self, cfg: &Config) {
        self.today.configure(cfg);
        self.apps.configure(cfg);
        self.wall.configure(cfg);
        self.settings.set_config(cfg);
    }

    /// Рисует содержимое вкладки `tab` в панели размером `w`×`h` (логические пиксели).
    pub unsafe fn draw(&mut self, tab: usize, ui: &mut Ui, cv: &Canvas, w: f32, h: f32) {
        let act = &mut self.actions;
        match tab {
            CONTROLS => self.controls.draw(ui, cv, w),
            TODAY => self.today.draw(ui, cv, w, h),
            STATS => self.stats.draw(ui, cv, w),
            CLIPBOARD => self.clip.draw(ui, cv, w, h, act),
            APPS => self.apps.draw(ui, cv, w, h, act),
            WALLPAPER => self.wall.draw(ui, cv, w, h),
            POWER => self.power.draw(ui, cv, w),
            SETTINGS => self.settings.draw(ui, cv, w, h),
            _ => {}
        }
        // Клавиши навигации, которых не ждала ни одна вкладка, не копятся.
        if tab != CLIPBOARD && tab != APPS {
            let _ = ui.take_nav();
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.today.step(dt);
        self.stats.step(dt);
        self.wall.step(dt);
        self.settings.step(dt);
    }

    pub fn animating(&self) -> bool {
        self.today.animating() || self.stats.animating() || self.wall.animating() || self.settings.animating()
    }

    /// Вкладка стала видимой. `true` — нужен кадр.
    pub fn show(&mut self, tab: usize) -> bool {
        match tab {
            CONTROLS => self.controls.show(),
            STATS => self.stats.show(),
            APPS => self.apps.show(),
            WALLPAPER => self.wall.show(),
            TODAY => self.today.show(),
            _ => false,
        }
    }

    pub fn hide(&mut self, tab: usize) {
        if tab == POWER {
            self.power.reset();
        }
        if tab == STATS {
            self.stats.hide();
        }
        if tab == CONTROLS {
            self.controls.hide();
        }
    }

    /// Нужен ли секундный опрос, пока вкладка на экране.
    pub fn wants_poll(&self, tab: usize) -> bool {
        match tab {
            CONTROLS | STATS | TODAY => true,
            POWER => self.power.armed(),
            _ => false,
        }
    }

    /// Секундный тик видимой вкладки. `true` — нужен кадр.
    pub fn poll(&mut self, tab: usize) -> bool {
        match tab {
            CONTROLS => self.controls.poll(),
            STATS => self.stats.poll(),
            TODAY => self.today.poll(),
            POWER => self.power.poll(),
            _ => false,
        }
    }

    /// Когда сработает таймер на вкладке Today (нужен и при свёрнутом острове).
    pub fn alarm_deadline(&self) -> Option<Instant> {
        self.today.deadline()
    }

    /// Таймер дошёл до нуля (для уведомления в свёрнутой форме).
    pub fn take_timer_fired(&mut self) -> bool {
        self.today.take_fired()
    }

    pub fn alarm(&mut self) -> bool {
        self.today.poll()
    }

    /// Фоновый поток или поток миниатюр сообщил о новых данных.
    pub fn on_bg(&mut self) -> bool {
        let a = self.controls.sync();
        let b = self.wall.sync();
        let c = self.stats.sync();
        let d = self.today.sync();
        let e = self.clip.sync();
        let f = self.apps.sync();
        a | b | c | d | e | f
    }

    /// Новый текст в буфере обмена (уже без «исключённых» и наших собственных записей).
    pub fn on_clipboard(&mut self, text: String) -> bool {
        self.clip.record(text)
    }

    /// В буфере обмена появилось изображение (PNG сохраняется в фоне, запись добавится в `on_bg`).
    pub fn on_clipboard_image(&mut self, w: u32, h: u32, px: Vec<u8>) {
        self.clip.capture_image(w, h, px);
    }
}
