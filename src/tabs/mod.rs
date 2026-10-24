//! Вкладки острова (этап 3). Каждая вкладка — отдельный модуль со своим состоянием;
//! рисуется и опрашивает данные только пока она видна (`show`/`hide`/`poll`).
//!
//! Файлы данных лежат в `%APPDATA%\island`, по одной записи в строке:
//! - `todo.txt` — `D|текст` (D = 1, если выполнено);
//! - `clipboard.txt` — `P|unix_секунды|текст` (история буфера; `\n`, `\r`, `\t`, `\\` экранированы; P = 0,
//!   закрепление переехало в `quick.txt`, а прежние записи с P = 1 переносятся туда при чтении);
//! - `quick.txt` — `unix_секунды|текст` («Быстрый доступ», до 30 записей, тот же вид текста; картинки — в `quick\`);
//! - `apps.txt` — `запусков|unix_секунды|избранное|путь`;
//! - `wallpapers.txt` — пути недавних обоев, новые сверху.

use std::path::PathBuf;
use std::time::Instant;

use crate::config::Config;
use crate::ui::{Canvas, Ui};

pub mod apps;
pub mod clipboard;
pub mod controls;
pub mod grid;
pub mod layout;
mod legacy;
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
pub const TOP: f32 = 38.0;
pub const PAD: f32 = 14.0;

/// Просьба вкладки к главному окну.
pub enum Action {
    /// Положить текст в буфер обмена, свернуть остров и вставить в окно, где работали.
    Paste(String),
    /// Только положить текст в буфер обмена.
    Copy(String),
    /// То же для изображения из истории или быстрого доступа (путь к PNG).
    PasteImage(PathBuf),
    CopyImage(PathBuf),
    /// Открыть файл или ссылку из быстрого доступа и свернуть остров.
    Open(String),
    /// Запись быстрого доступа потащили наружу.
    DragOut(crate::dnd::Out),
    /// Новая горячая клавиша раздела «Быстрый доступ» (пустая — снять).
    SetQuickHotkey(String),
    /// Свернуть остров (после запуска приложения и т. п.).
    Collapse,
    /// Раскладка виджетов вкладки изменилась: записать строку в `config.toml`.
    Layout(usize, String),
    /// Режим правки закончился: пересчитать высоту острова.
    Resize,
}

/// Описание виджетов вкладки для сетки (у остальных вкладок сетки нет).
fn spec_of(tab: usize) -> &'static layout::TabSpec {
    match tab {
        CONTROLS => &controls::SPEC,
        TODAY => &today::SPEC,
        STATS => &stats::SPEC,
        _ => &layout::EMPTY,
    }
}

pub fn id(tab: usize, n: u32) -> u32 {
    ((tab as u32 + 1) << 16) | n
}

fn data_path(name: &str) -> PathBuf {
    crate::sys::data_dir().join(name)
}

pub fn read_lines(name: &str) -> Vec<String> {
    // Байты читаем «как есть»: одна битая последовательность не должна обнулять весь файл.
    std::fs::read(data_path(name))
        .map(|b| String::from_utf8_lossy(&b).lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Пишет через временный файл и переименование: сбой посреди записи не оставит обрезанный файл.
pub fn write_lines(name: &str, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    write_bytes(name, text.as_bytes());
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
    crate::i18n::ago_long(d)
}

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
    /// Сетки виджетов вкладок Controls, Today, Stats (индекс = номер вкладки).
    grids: [grid::Grid; 3],
}

impl Default for Tabs {
    fn default() -> Self {
        let mut grids: [grid::Grid; 3] = Default::default();
        for (tab, g) in grids.iter_mut().enumerate() {
            g.configure(tab, spec_of(tab), "");
        }
        Self {
            power: Default::default(),
            stats: Default::default(),
            controls: Default::default(),
            today: Default::default(),
            clip: Default::default(),
            apps: Default::default(),
            wall: Default::default(),
            settings: Default::default(),
            actions: Default::default(),
            grids,
        }
    }
}

/// Фиксированные размеры вкладок без сетки и страницы настроек: (ширина, высота).
/// Для вкладок с сеткой (Controls, Today, Stats) размеры вычисляются из `TabSpec`.
const NON_GRID_SIZES: [(f32, f32); 5] = [
    (590.0, 328.0), // CLIPBOARD
    (480.0, 420.0), // APPS
    (560.0, 236.0), // WALLPAPER
    (420.0, 112.0), // POWER
    (600.0, 476.0), // SETTINGS
];

impl Tabs {
    /// Загружает сохранённые данные (один раз при старте).
    pub fn load(&mut self, cfg: &Config, state: &crate::state::State) {
        self.today.load();
        self.today.load_timer(state);
        self.today.configure(cfg);
        self.clip.load(state.clipboard_paused, state.clipboard_section);
        self.clip.set_hotkey(&cfg.quick_hotkey);
        self.stats.load(state.stats_top_by_cpu);
        self.wall.load(state.wallpaper_filter);
        self.apps.configure(cfg);
        self.apps.load(state.apps_grid, state.apps_all);
        self.wall.configure(cfg);
        for (tab, g) in self.grids.iter_mut().enumerate() {
            g.set_compact(!cfg.free_layout);
            g.configure(tab, spec_of(tab), &cfg.layouts[tab]);
        }
    }

    pub fn apply_config(&mut self, cfg: &Config) {
        self.clip.set_hotkey(&cfg.quick_hotkey);
        self.today.configure(cfg);
        self.apps.configure(cfg);
        self.wall.configure(cfg);
        self.settings.set_config(cfg);
        self.set_compact(!cfg.free_layout);
        for (tab, g) in self.grids.iter_mut().enumerate() {
            g.set_text(&cfg.layouts[tab]);
        }
    }

    /// Включает или выключает автоуплотнение раскладки на всех вкладках с сеткой.
    pub fn set_compact(&mut self, on: bool) {
        for g in &mut self.grids {
            g.set_compact(on);
        }
    }

    /// Можно ли настраивать виджеты вкладки (показывается карандаш).
    pub fn editable(&self, tab: usize) -> bool {
        self.grids.get(tab).is_some_and(grid::Grid::can_edit)
    }

    pub fn editing(&self, tab: usize) -> bool {
        self.grids.get(tab).is_some_and(grid::Grid::is_editing)
    }

    /// Переключает режим правки. `true` — состояние изменилось.
    pub fn toggle_edit(&mut self, tab: usize) -> bool {
        let Some(g) = self.grids.get_mut(tab) else { return false };
        if !g.can_edit() {
            return false;
        }
        if g.is_editing() {
            g.end_edit();
        } else {
            g.begin_edit();
            // Выпадающие панели поверх виджетов в правке только мешают: закрываем.
            match tab {
                CONTROLS => self.controls.hide(),
                TODAY => self.today.hide_overlays(),
                _ => {}
            }
        }
        true
    }

    /// На сколько остров выше, пока вкладку правят.
    pub fn edit_extra(&self, tab: usize) -> f32 {
        self.grids.get(tab).map_or(0.0, grid::Grid::extra)
    }

    /// Устанавливает минимальную ширину сетки (в ячейках) для всех вкладок с сеткой
    /// для согласования с длиной ряда вкладок острова.
    pub fn set_min_cols(&mut self, min_cols: u8) {
        for g in &mut self.grids {
            g.set_min_cols(min_cols);
        }
    }

    /// Размер острова для вкладки `tab`: (ширина, высота) с учётом `edit_extra` в режиме правки.
    /// Для вкладок с сеткой (Controls, Today, Stats) вычисляется из текущего окна сетки:
    /// `ширина = cols * CELL - GAP + 2 * PAD`, `высота = TOP + rows * CELL - GAP + PAD`.
    /// Для остальных вкладок берутся их фиксированные размеры.
    pub fn island_size(&self, tab: usize) -> (f32, f32) {
        let (w, mut h) = match tab {
            CONTROLS | TODAY | STATS => {
                let win = self.grids[tab].window();
                (win.island_w(), win.island_h(0.0))
            }
            CLIPBOARD..=SETTINGS => NON_GRID_SIZES[tab - CLIPBOARD],
            _ => (400.0, 200.0),
        };
        h += self.edit_extra(tab);
        (w, h)
    }

    /// Рисует содержимое вкладки `tab` в панели размером `w`×`h` (логические пиксели).
    pub unsafe fn draw(&mut self, tab: usize, ui: &mut Ui, cv: &Canvas, w: f32, h: f32) {
        let act = &mut self.actions;
        match tab {
            CONTROLS => self.controls.draw(ui, cv, &mut self.grids[CONTROLS], w, h),
            TODAY => self.today.draw(ui, cv, &mut self.grids[TODAY], w, h),
            STATS => self.stats.draw(ui, cv, &mut self.grids[STATS], w, h),
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
        // Правка сетки: новую раскладку надо записать, окончание правки — пересчитать высоту острова.
        if let Some(g) = self.grids.get_mut(tab) {
            if let Some(text) = g.take_saved() {
                self.actions.push(Action::Layout(tab, text));
            }
            if g.take_resize() {
                self.actions.push(Action::Resize);
            }
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.controls.step(dt);
        self.today.step(dt);
        self.stats.step(dt);
        self.wall.step(dt);
        self.settings.step(dt);
        for g in &mut self.grids {
            g.step(dt);
        }
    }

    pub fn animating(&self) -> bool {
        self.controls.animating()
            || self.today.animating()
            || self.stats.animating()
            || self.wall.animating()
            || self.settings.animating()
            || self.grids.iter().any(grid::Grid::animating)
    }

    /// Вкладка стала видимой. `true` — нужен кадр.
    pub fn show(&mut self, tab: usize) -> bool {
        if let Some(g) = self.grids.get_mut(tab) {
            g.show();
        }
        match tab {
            CONTROLS => self.controls.show(),
            STATS => self.stats.show(),
            APPS => self.apps.show(),
            WALLPAPER => self.wall.show(),
            TODAY => self.today.show(),
            CLIPBOARD => self.clip.show(),
            _ => false,
        }
    }

    pub fn hide(&mut self, tab: usize) {
        if let Some(g) = self.grids.get_mut(tab) {
            g.hide();
        }
        if tab == POWER {
            self.power.reset();
        }
        if tab == STATS {
            self.stats.hide();
        }
        if tab == CONTROLS {
            self.controls.hide();
        }
        if tab == TODAY {
            self.today.hide_overlays();
        }
        if tab == CLIPBOARD {
            self.clip.hide();
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

    /// Обрывает сигнал окончания таймера.
    pub fn stop_alarm(&self) {
        today::Today::stop_alarm();
    }

    /// Звучит ли сигнал окончания таймера (уведомление ждёт его конца).
    pub fn alarm_playing(&self) -> bool {
        today::Today::alarm_playing()
    }

    /// Запущенный или стоящий на паузе таймер Today (для свёрнутого острова).
    pub fn timer_view(&self) -> Option<crate::live::TimerView> {
        self.today.view()
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

    /// Открывает на вкладке «Буфер» раздел «Быстрый доступ» (горячая клавиша раздела).
    pub fn open_quick(&mut self) {
        self.clip.set_section(clipboard::Section::Quick);
    }

    /// Вкладка ждёт нажатия сочетания для горячей клавиши раздела.
    pub fn recording_hotkey(&self) -> bool {
        self.clip.recording()
    }

    /// Сочетание нажато: просьба сохранить его уходит главному окну.
    pub fn hotkey_captured(&mut self, combo: String) {
        self.clip.stop_recording();
        self.actions.push(Action::SetQuickHotkey(combo));
    }

    pub fn cancel_hotkey_recording(&mut self) {
        self.clip.stop_recording();
    }

    /// Горячая клавиша раздела сменилась (или не заняла: тогда с пояснением).
    pub fn set_quick_hotkey(&mut self, hotkey: &str, note: Option<&'static str>) {
        self.clip.set_hotkey(hotkey);
        self.clip.set_note(note);
    }

    /// Брошенное на остров попадает в «Быстрый доступ».
    pub fn add_quick(&mut self, entries: &[String]) -> clipboard::Added {
        self.clip.add_quick(entries)
    }

    /// В «Быстром доступе» нет места.
    pub fn quick_full(&self) -> bool {
        self.clip.quick_full()
    }

    /// Файл из «Быстрого доступа» не открылся: пометить недоступным.
    pub fn mark_missing(&mut self, path: &str) {
        self.clip.mark_missing(path);
    }

    /// Срок хранения истории буфера (раз в час и при показе вкладки). `true` — список изменился.
    pub fn prune_clip(&mut self) -> bool {
        self.clip.prune()
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

/// Как [`read_lines`], но файл зашифрован DPAPI. Старый открытый файл читается как есть и при
/// следующей записи становится зашифрованным. Не удалось расшифровать — пустая история.
pub fn read_lines_sealed(name: &str) -> Vec<String> {
    let Ok(bytes) = std::fs::read(data_path(name)) else { return Vec::new() };
    match crate::sys::unseal(&bytes) {
        Some(plain) => String::from_utf8_lossy(&plain).lines().map(str::to_string).collect(),
        None => {
            crate::log::error(&format!("не удалось расшифровать {name}: файл создан другой учётной записью?"));
            Vec::new()
        }
    }
}

/// Как [`write_lines`], но с шифрованием DPAPI. Если шифрование недоступно — на диск ничего не
/// пишется (открытым текстом приватные данные не сохраняем).
pub fn write_lines_sealed(name: &str, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    let Some(sealed) = crate::sys::seal(text.as_bytes()) else {
        crate::log::error(&format!("DPAPI недоступен, {name} не сохранён"));
        return;
    };
    write_bytes(name, &sealed);
}

/// Атомарная запись байтов в папку данных (временный файл + переименование).
pub fn write_bytes(name: &str, bytes: &[u8]) {
    let dir = crate::sys::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let tmp = dir.join(format!("{name}.tmp"));
    let res = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, dir.join(name)));
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        crate::log::error(&format!("не удалось записать {name}: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::layout::Layout;

    #[test]
    fn test_island_sizes() {
        let tabs = Tabs::default();

        // Вкладки с сеткой: из формулы cols*56-8+2*PAD и TOP+rows*56-8+PAD по окну сетки.
        // Окно по умолчанию — все виджеты в сетке 6×4 (Controls) и 10×4 (Today, Stats).
        assert_eq!(tabs.island_size(CONTROLS), (356.0, 268.0));
        assert_eq!(tabs.island_size(TODAY), (580.0, 268.0));
        assert_eq!(tabs.island_size(STATS), (580.0, 268.0));

        // Вкладки без сетки:
        assert_eq!(tabs.island_size(CLIPBOARD), (590.0, 328.0));
        assert_eq!(tabs.island_size(APPS), (480.0, 420.0));
        assert_eq!(tabs.island_size(WALLPAPER), (560.0, 236.0));
        assert_eq!(tabs.island_size(POWER), (420.0, 112.0));
        assert_eq!(tabs.island_size(SETTINGS), (600.0, 476.0));
    }

    #[test]
    fn test_all_widget_sizes_are_standard() {
        for tab in [CONTROLS, TODAY, STATS] {
            for s in spec_of(tab).specs {
                for z in s.sizes {
                    assert!(z.is_standard(), "{}: нестандартный размер {}×{}", s.key, z.w, z.h);
                }
                assert!(s.size < s.sizes.len(), "{}: размер по умолчанию вне списка", s.key);
            }
        }
    }

    #[test]
    fn test_all_grid_tabs_no_overlap_and_fit() {
        for tab in [CONTROLS, TODAY, STATS] {
            let spec = layout::resolve_full(spec_of(tab), 8);
            let lay = Layout::default_for(spec);
            let placed = lay.pack(spec, 0);

            // Проверяем, что все виджеты на месте
            assert_eq!(placed.len(), spec.specs.len(), "вкладка {tab}: не все виджеты упакованы");

            // Проверяем, что нет пересечений
            for (i, a) in placed.iter().enumerate() {
                for b in &placed[i + 1..] {
                    let cols = a.col < b.col + b.w && b.col < a.col + a.w;
                    let rows = a.row < b.row + b.h && b.row < a.row + a.h;
                    assert!(
                        !(cols && rows),
                        "вкладка {tab}: виджеты {} ({}@{},{}) и {} ({}@{},{}) пересекаются",
                        a.idx, spec.specs[a.idx].key, a.col, a.row,
                        b.idx, spec.specs[b.idx].key, b.col, b.row
                    );
                }
            }

            // Проверяем, что все виджеты помещаются в границы сетки
            assert!(
                lay.fits(spec, 0, 1000.0),
                "вкладка {tab}: виджеты выходят за границы сетки cols={}, rows={}",
                spec.cols,
                spec.rows
            );
        }
    }
}
