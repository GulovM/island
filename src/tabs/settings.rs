//! Страница настроек внутри острова (шестерёнка в ряду вкладок). Слева — цвета: переключатель
//! «Цвет системы», готовые темы и полная палитра (квадрат насыщенности/яркости, полоса оттенка, hex);
//! справа — радиус углов (пока ползунок тянут, остров «засвечивается» — см. `glowing`) и список
//! вкладок: включение/выключение и порядок. Страница правит рабочую копию `Config` и отдаёт её
//! наружу через `take_change`; применяет и сохраняет изменения главное окно.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::anim::Spring;
use crate::config::{Config, DEFAULT_RADIUS, DEFAULT_SCALE, MAX_RADIUS, MAX_SCALE, MIN_SCALE};
use crate::theme::{self, Rgb};
use crate::ui::{chip_bg, dim, icon, ink, inset, txt, Canvas, Rect, Submit, Ui, WHITE};

use super::{id, PAD, SETTINGS, TOP};

// Номера элементов внутри страницы (см. `super::id`).
const N_SYSTEM: u32 = 1;
const N_CHIP_ACCENT: u32 = 2;
const N_CHIP_BG: u32 = 3;
const N_SV: u32 = 4;
const N_HUE: u32 = 5;
const N_HEX: u32 = 6;
const N_RADIUS: u32 = 7;
const N_AUTOSTART: u32 = 8;
const N_FACE: u32 = 9;
const N_RESET: u32 = 10;
const N_SCALE: u32 = 11;
const N_STRIP: u32 = 12;
/// Занимает три номера подряд (13–15): следующие свободные — 16 и выше.
const N_LANG: u32 = 13;
const N_AUTOUPDATE: u32 = 16;
const N_UPDATE: u32 = 17;
const N_GLASS: u32 = 18;
const N_OPACITY: u32 = 19;
const N_PRESET: u32 = 20;
const N_SWITCH: u32 = 40;
const N_UP: u32 = 60;
const N_DOWN: u32 = 80;

const ROW_H: f32 = 23.0;
const ROW_PITCH: f32 = 26.0;

const BLACK: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
const CLEAR_BLACK: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// Готовые темы: (акцент, фон).
const PRESETS: [(&str, &str); 6] = [
    ("#D4AF5E", "#0D0D0D"),
    ("#4DA3FF", "#0B1220"),
    ("#4ADE9B", "#0A1512"),
    ("#FF6B9A", "#160C12"),
    ("#A78BFA", "#110E1B"),
    ("#2F6FED", "#F1F3F6"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Accent,
    Bg,
}

pub struct Settings {
    /// Рабочая копия настроек; наружу уходит через `take_change`.
    cfg: Config,
    changed: bool,
    target: Target,
    /// Редактируемый цвет в HSV: оттенок не теряется, когда насыщенность или яркость доходят до нуля.
    hsv: [f32; 3],
    /// Ползунок радиуса сейчас тянут.
    glow: bool,
    /// Значение масштаба, пока ползунок тянут: применяется при отпускании мыши
    /// (иначе панель меняет размер под курсором и ползунок «убегает»).
    scale_drag: Option<f32>,
    autostart: bool,
    /// Вертикальные позиции строк вкладок по их номеру (плавная перестановка).
    rows: [Spring; 7],
    rows_ready: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cfg: Config::default(),
            changed: false,
            target: Target::Accent,
            hsv: theme::rgb_to_hsv(theme::DEFAULT_ACCENT),
            glow: false,
            scale_drag: None,
            autostart: false,
            rows: std::array::from_fn(|_| Spring::new(0.0, 520.0, 38.0)),
            rows_ready: false,
        }
    }
}

fn col(c: Rgb) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a: 1.0 }
}

fn rgb_of(c: D2D1_COLOR_F) -> Rgb {
    [c.r, c.g, c.b]
}

/// Доля ползунка (0..1) → множитель к DPI. Шаг — 5 п.п. суммарного масштаба (DPI × множитель),
/// именно его пользователь видит в подписи.
fn snap_scale(frac: f32, dpi: f32) -> f32 {
    let v = MIN_SCALE + frac.clamp(0.0, 1.0) * (MAX_SCALE - MIN_SCALE);
    (((v * dpi * 20.0).round() / 20.0) / dpi).clamp(MIN_SCALE, MAX_SCALE)
}

fn close(a: Rgb, b: Rgb) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1.0 / 255.0)
}

impl Settings {
    /// Страница открылась: берём актуальные настройки.
    pub fn load(&mut self, cfg: &Config) {
        self.cfg = cfg.clone();
        self.changed = false;
        self.glow = false;
        self.autostart = crate::sys::autostart_enabled();
        self.rows_ready = false;
        self.hsv = theme::rgb_to_hsv(self.current());
    }

    /// Конфиг перечитан из файла, пока страница закрыта.
    pub fn set_config(&mut self, cfg: &Config) {
        if !self.changed {
            self.cfg = cfg.clone();
            self.rows_ready = false;
        }
    }

    /// Изменённые настройки (однократно).
    pub fn take_change(&mut self) -> Option<Config> {
        if std::mem::take(&mut self.changed) {
            Some(self.cfg.clone())
        } else {
            None
        }
    }

    /// Остров должен светиться: ползунок радиуса тянут.
    pub fn glowing(&self) -> bool {
        self.glow
    }

    pub fn step(&mut self, dt: f32) {
        for r in &mut self.rows {
            r.step(dt);
            if r.settled() {
                r.snap();
            }
        }
    }

    pub fn animating(&self) -> bool {
        self.rows.iter().any(|r| !r.settled())
    }

    /// Редактируемый цвет: при «Цвет системы» — тот, что сейчас реально на острове.
    fn current(&self) -> Rgb {
        let pal = theme::get();
        match (self.cfg.system_colors, self.target) {
            (true, Target::Accent) => rgb_of(pal.accent),
            (true, Target::Bg) => rgb_of(pal.bg),
            (false, Target::Accent) => self.cfg.accent,
            (false, Target::Bg) => self.cfg.bg,
        }
    }

    fn set_color(&mut self, c: Rgb) {
        match self.target {
            Target::Accent => self.cfg.accent = c,
            Target::Bg => self.cfg.bg = c,
        }
        self.cfg.system_colors = false;
        self.changed = true;
    }

    /// Применяет `hsv` как цвет выбранной цели.
    fn set_hsv(&mut self, hsv: [f32; 3]) {
        self.hsv = hsv;
        self.set_color(theme::hsv_to_rgb(hsv));
    }

    /// Порядок вкладок: `order[позиция] = номер вкладки`.
    fn order(&self) -> [usize; 7] {
        self.cfg.resolve_order(&crate::TABS)
    }

    fn hidden(&self, tab: usize) -> bool {
        self.cfg.hidden_tabs.iter().any(|h| h.eq_ignore_ascii_case(crate::TABS[tab]))
    }

    fn set_hidden(&mut self, tab: usize, hide: bool) {
        let name = crate::TABS[tab];
        self.cfg.hidden_tabs.retain(|h| !h.eq_ignore_ascii_case(name));
        if hide {
            self.cfg.hidden_tabs.push(name.to_string());
        }
        self.changed = true;
    }

    fn move_tab(&mut self, pos: usize, to: usize) {
        let mut order = self.order();
        order.swap(pos, to);
        self.cfg.tabs = order.iter().map(|&i| crate::TABS[i].to_string()).collect();
        self.changed = true;
    }

    fn reset(&mut self) {
        let d = Config::default();
        self.cfg.accent = d.accent;
        self.cfg.bg = d.bg;
        self.cfg.system_colors = false;
        self.cfg.corner_radius = DEFAULT_RADIUS;
        self.cfg.glass = d.glass;
        self.cfg.glass_opacity = d.glass_opacity;
        self.cfg.ui_scale = DEFAULT_SCALE;
        self.scale_drag = None;
        self.cfg.hidden_tabs.clear();
        self.cfg.tabs = crate::TABS.iter().map(|t| t.to_string()).collect();
        self.cfg.idle_face = true;
        self.cfg.strip_over_maximized = d.strip_over_maximized;
        self.hsv = theme::rgb_to_hsv(self.current());
        self.changed = true;
    }

    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, w: f32, h: f32) {
        // Если цвет поменяли снаружи (тема, hex, сброс), HSV пересчитывается; оттенок у серых сохраняем.
        let cur = self.current();
        if !close(theme::hsv_to_rgb(self.hsv), cur) {
            let n = theme::rgb_to_hsv(cur);
            self.hsv = if n[1] < 0.01 || n[2] < 0.01 { [self.hsv[0], n[1], n[2]] } else { n };
        }

        let lc = Rect::new(PAD, TOP, 290.0, h - TOP - PAD);
        let rc = Rect::new(lc.right() + 10.0, TOP, w - PAD - (lc.right() + 10.0), lc.h);
        cv.card(lc);
        cv.card(rc);
        self.colors(ui, cv, lc);
        self.right(ui, cv, rc);
    }

    // ---------- Левая карточка: цвета ----------

    unsafe fn colors(&mut self, ui: &mut Ui, cv: &Canvas, lc: Rect) {
        let f = &cv.g.fonts;
        let x = lc.x + 10.0;
        let iw = lc.w - 20.0;
        let mut y = lc.y + 9.0;

        // «Цвет системы».
        cv.text("Цвет системы", Rect::new(x, y, iw - 40.0, 13.0), &f.body, txt());
        cv.text("Как в Windows и Wallpaper Engine", Rect::new(x, y + 13.0, iw - 40.0, 11.0), &f.small, dim());
        let mut sys = self.cfg.system_colors;
        if ui.switch(cv, id(SETTINGS, N_SYSTEM), Rect::new(lc.right() - 40.0, y + 4.0, 30.0, 16.0), &mut sys, true) {
            self.cfg.system_colors = sys;
            self.changed = true;
        }
        y += 34.0;
        let sys = self.cfg.system_colors;

        // Что красим и готовые темы.
        if ui.chip(cv, id(SETTINGS, N_CHIP_ACCENT), Rect::new(x, y, 56.0, 18.0), "Акцент", self.target == Target::Accent) {
            self.target = Target::Accent;
        }
        if ui.chip(cv, id(SETTINGS, N_CHIP_BG), Rect::new(x + 60.0, y, 44.0, 18.0), "Фон", self.target == Target::Bg) {
            self.target = Target::Bg;
        }
        for (i, (a, b)) in PRESETS.iter().enumerate() {
            let a = theme::parse_hex(a).unwrap_or(theme::DEFAULT_ACCENT);
            let b = theme::parse_hex(b).unwrap_or(theme::DEFAULT_BG);
            let selected = !self.cfg.system_colors && close(self.cfg.accent, a) && close(self.cfg.bg, b);
            let cx = x + 124.0 + i as f32 * 21.0 + 8.0;
            if ui.swatch(cv, id(SETTINGS, N_PRESET + i as u32), cx, y + 9.0, 7.5, col(b), col(a), selected) {
                self.cfg.accent = a;
                self.cfg.bg = b;
                self.cfg.system_colors = false;
                self.changed = true;
            }
        }
        y += 26.0;

        // Полная палитра. При «Цвет системы» она только показывает текущие цвета.
        let dimmed = Canvas { g: cv.g, alpha: cv.alpha * if sys { 0.4 } else { 1.0 }, accent: cv.accent };
        let sv = Rect::new(x, y, iw, 92.0);
        let hue_c = theme::hsv_to_rgb([self.hsv[0], 1.0, 1.0]);
        dimmed.gradient_stops(sv, 6.0, &[(0.0, WHITE), (1.0, col(hue_c))], true);
        dimmed.gradient_stops(sv, 6.0, &[(0.0, CLEAR_BLACK), (1.0, BLACK)], false);
        if !sys {
            if let Some((px, py)) = ui.drag_area(id(SETTINGS, N_SV), sv) {
                self.set_hsv([self.hsv[0], px, 1.0 - py]);
            }
        }
        let (mx, my) = (sv.x + sv.w * self.hsv[1], sv.y + sv.h * (1.0 - self.hsv[2]));
        dimmed.circle(mx, my, 6.2, WHITE);
        dimmed.circle(mx, my, 4.8, col(self.current()));
        y += 100.0;

        let hue = Rect::new(x, y, iw, 12.0);
        let stops: Vec<(f32, D2D1_COLOR_F)> =
            (0..=6).map(|i| (i as f32 / 6.0, col(theme::hsv_to_rgb([i as f32 * 60.0, 1.0, 1.0])))).collect();
        dimmed.gradient_stops(hue, 6.0, &stops, true);
        if !sys {
            let hit = Rect::new(hue.x, hue.y - 4.0, hue.w, hue.h + 8.0);
            if let Some((px, _)) = ui.drag_area(id(SETTINGS, N_HUE), hit) {
                self.set_hsv([(px * 360.0).min(359.99), self.hsv[1], self.hsv[2]]);
            }
        }
        let hx = hue.x + hue.w * (self.hsv[0] / 360.0);
        dimmed.circle(hx, hue.cy(), 7.0, WHITE);
        dimmed.circle(hx, hue.cy(), 5.6, col(hue_c));
        y += 20.0;

        // Hex: поле показывает цвет и принимает ввод #RRGGBB.
        let hex_id = id(SETTINGS, N_HEX);
        let hex = theme::to_hex(self.current());
        if !ui.has_focus(hex_id) {
            if ui.field_text(hex_id) != hex {
                ui.set_field_text(hex_id, &hex);
            }
        } else if !sys {
            if let Some(c) = theme::parse_hex(&ui.field_text(hex_id)) {
                if !close(c, self.current()) {
                    self.set_color(c);
                }
            }
        }
        ui.text_field_ex(cv, hex_id, Rect::new(x, y, iw - 30.0, 24.0), "#RRGGBB", icon::CONTRAST, Submit::None);
        cv.circle(lc.right() - 10.0 - 11.0, y + 12.0, 10.0, chip_bg());
        cv.circle(lc.right() - 10.0 - 11.0, y + 12.0, 8.6, col(self.current()));
        y += 32.0;

        // Системное поведение.
        cv.text("Автозапуск", Rect::new(x, y, iw - 40.0, 20.0), &f.body, txt());
        let mut auto = self.autostart;
        if ui.switch(cv, id(SETTINGS, N_AUTOSTART), Rect::new(lc.right() - 40.0, y + 2.0, 30.0, 16.0), &mut auto, true) {
            if !crate::sys::set_autostart(auto) {
                crate::log::error("не удалось изменить автозапуск");
            }
            self.autostart = crate::sys::autostart_enabled();
        }
        y += 24.0;
        cv.text("Время и Wi-Fi на свёрнутом острове", Rect::new(x, y, iw - 40.0, 20.0), &f.body, txt());
        let mut face = self.cfg.idle_face;
        if ui.switch(cv, id(SETTINGS, N_FACE), Rect::new(lc.right() - 40.0, y + 2.0, 30.0, 16.0), &mut face, true) {
            self.cfg.idle_face = face;
            self.changed = true;
        }
        y += 24.0;
        cv.text("Над окнами на весь экран — полоска", Rect::new(x, y, iw - 40.0, 20.0), &f.body, txt());
        let mut strip = self.cfg.strip_over_maximized;
        if ui.switch(cv, id(SETTINGS, N_STRIP), Rect::new(lc.right() - 40.0, y + 2.0, 30.0, 16.0), &mut strip, true) {
            self.cfg.strip_over_maximized = strip;
            self.changed = true;
        }
        y += 18.0;
        cv.text("Наведите на верхний край, чтобы раскрыть", Rect::new(x, y, iw, 13.0), &f.small, dim());
        y += 24.0;

        // Язык интерфейса: применяется сразу.
        cv.text("Язык", Rect::new(x, y, 80.0, 18.0), &f.body, txt());
        let langs = [("auto", "Авто", 46.0), ("ru", "RU", 34.0), ("en", "EN", 34.0)];
        let mut cx = lc.right() - 10.0 - (46.0 + 34.0 + 34.0 + 8.0);
        for (k, (code, label, w)) in langs.iter().enumerate() {
            if ui.chip(cv, id(SETTINGS, N_LANG + k as u32), Rect::new(cx, y, *w, 18.0), label, self.cfg.language == *code) {
                self.cfg.language = code.to_string();
                self.changed = true;
            }
            cx += w + 4.0;
        }
        y += 28.0;

        // Автообновление: раз в сутки остров проверяет сам и ставит найденное без вопросов.
        cv.text("Автообновление", Rect::new(x, y, iw - 40.0, 20.0), &f.body, txt());
        let mut au = self.cfg.auto_update;
        if ui.switch(cv, id(SETTINGS, N_AUTOUPDATE), Rect::new(lc.right() - 40.0, y + 2.0, 30.0, 16.0), &mut au, true) {
            self.cfg.auto_update = au;
            self.changed = true;
        }
        y += 28.0;

        // Радиус углов.
        cv.text("Радиус углов", Rect::new(x, y, 140.0, 13.0), &f.body, txt());
        cv.text(&format!("{} px", self.cfg.corner_radius.round() as i32), Rect::new(lc.right() - 70.0, y, 60.0, 13.0), &f.body_r, dim());
        y += 21.0;
        let mut frac = self.cfg.corner_radius / MAX_RADIUS;
        let rid = id(SETTINGS, N_RADIUS);
        if ui.slider_thin(cv, rid, Rect::new(x + 6.0, y, iw - 12.0, 14.0), &mut frac) {
            self.cfg.corner_radius = (frac * MAX_RADIUS).round();
            self.changed = true;
        }
        self.glow = ui.is_dragging(rid);
    }

    /// Строка «Версия … / Проверить»: ручная проверка обновлений и предложение установить найденное.
    unsafe fn update_row(&mut self, ui: &mut Ui, cv: &Canvas, x: f32, y: f32, iw: f32) {
        use crate::i18n::pick;
        use crate::update::{self, State};
        let f = &cv.g.fonts;
        let ver = update::VERSION;
        let (text, label, accent) = match update::state() {
            State::Idle => (format!("{} {ver}", pick("Версия", "Version")), "Проверить", false),
            State::Checking => (pick("Проверяем…", "Checking…").to_string(), "Проверка…", false),
            State::UpToDate => (format!("{} {ver} — {}", pick("Версия", "Version"), pick("актуальная", "up to date")), "Проверить", false),
            State::Available(r) => (format!("{} {}", pick("Доступна версия", "Version available:"), r.version), "Установить", true),
            State::Downloading => (pick("Скачиваем и устанавливаем…", "Downloading and installing…").to_string(), "Установка…", false),
            State::Failed(msg) => (crate::i18n::tr(msg).into_owned(), "Повторить", false),
        };
        let bw = 104.0;
        let col = if accent { cv.accent } else { dim() };
        cv.text(&text, Rect::new(x, y, iw - bw - 8.0, 22.0), &f.small, col);
        let (bg, fg) = if accent { (cv.accent, ink()) } else { (chip_bg(), txt()) };
        let clicked = ui.solid_button(cv, id(SETTINGS, N_UPDATE), Rect::new(x + iw - bw, y, bw, 22.0), label, bg, fg, 7.0);
        if clicked && !update::busy() {
            if update::available() {
                update::install();
            } else {
                update::check(false);
            }
        }
    }

    // ---------- Правая карточка: радиус и вкладки ----------

    unsafe fn right(&mut self, ui: &mut Ui, cv: &Canvas, rc: Rect) {
        let f = &cv.g.fonts;
        let x = rc.x + 10.0;
        let iw = rc.w - 20.0;
        let mut y = rc.y + 9.0;

        // Стекло: выключатель и прозрачность (0 — почти прозрачное, 100 — сплошное).
        cv.text("Стекло", Rect::new(x, y, iw - 40.0, 20.0), &f.body, txt());
        let mut glass = self.cfg.glass;
        if ui.switch(cv, id(SETTINGS, N_GLASS), Rect::new(rc.right() - 40.0, y + 2.0, 30.0, 16.0), &mut glass, true) {
            self.cfg.glass = glass;
            self.changed = true;
        }
        y += 24.0;
        cv.text("Прозрачность", Rect::new(x, y, 170.0, 13.0), &f.body, if self.cfg.glass { txt() } else { dim() });
        cv.text(&format!("{}%", self.cfg.glass_opacity.round() as i32), Rect::new(rc.right() - 70.0, y, 60.0, 13.0), &f.body_r, dim());
        y += 21.0;
        let mut frac = self.cfg.glass_opacity / 100.0;
        if ui.slider_thin(cv, id(SETTINGS, N_OPACITY), Rect::new(x + 6.0, y, iw - 12.0, 14.0), &mut frac) && self.cfg.glass {
            self.cfg.glass_opacity = (frac * 100.0).round();
            self.changed = true;
        }
        y += 28.0;

        // Масштаб интерфейса (шаг 0.05): применяется, когда мышь отпущена.
        let shown = self.scale_drag.unwrap_or(self.cfg.ui_scale);
        let dpi = crate::wnd::dpi_factor();
        cv.text("Масштаб интерфейса", Rect::new(x, y, 170.0, 13.0), &f.body, txt());
        cv.text(&format!("{}%", (shown * dpi * 100.0).round() as i32), Rect::new(rc.right() - 70.0, y, 60.0, 13.0), &f.body_r, dim());
        y += 21.0;
        let mut frac = (shown - MIN_SCALE) / (MAX_SCALE - MIN_SCALE);
        let sid = id(SETTINGS, N_SCALE);
        if ui.slider_thin(cv, sid, Rect::new(x + 6.0, y, iw - 12.0, 14.0), &mut frac) {
            self.scale_drag = Some(snap_scale(frac, dpi));
        }
        if !ui.down {
            if let Some(v) = self.scale_drag.take() {
                if (v - self.cfg.ui_scale).abs() > 0.001 {
                    self.cfg.ui_scale = v;
                    self.changed = true;
                }
            }
        }
        y += 28.0;

        // Вкладки.
        cv.text("ВКЛАДКИ — ВКЛЮЧЕНИЕ И ПОРЯДОК", Rect::new(x, y, iw, 11.0), &f.caps, dim());
        y += 15.0;
        let order = self.order();
        let visible = self.cfg.resolve_visible(&crate::TABS).len();
        if !self.rows_ready {
            for (pos, &tab) in order.iter().enumerate() {
                self.rows[tab].target = pos as f32 * ROW_PITCH;
                self.rows[tab].snap();
            }
            self.rows_ready = true;
        }
        let mut action: Option<(usize, i32)> = None;
        for (pos, &tab) in order.iter().enumerate() {
            self.rows[tab].target = pos as f32 * ROW_PITCH;
            let r = Rect::new(x, y + self.rows[tab].value, iw, ROW_H);
            cv.rr(r, 6.0, inset());
            let hidden = self.hidden(tab);
            let mut on = !hidden;
            // Последняя видимая вкладка не выключается.
            let can_toggle = hidden || visible > 1;
            if ui.switch(cv, id(SETTINGS, N_SWITCH + tab as u32), Rect::new(r.x + 6.0, r.y + 2.5, 30.0, 16.0), &mut on, can_toggle) {
                self.set_hidden(tab, !on);
            }
            cv.text(crate::i18n::tab_name(tab), Rect::new(r.x + 44.0, r.y, r.w - 92.0, r.h), &f.body, if hidden { dim() } else { txt() });
            let up = Rect::new(r.right() - 44.0, r.y + 1.5, 18.0, 18.0);
            let down = Rect::new(r.right() - 23.0, r.y + 1.5, 18.0, 18.0);
            if pos > 0 {
                if ui.icon_button(cv, id(SETTINGS, N_UP + tab as u32), up, icon::UP, false) {
                    action = Some((pos, -1));
                }
            } else {
                cv.glyph(icon::UP, up, false, D2D1_COLOR_F { a: 0.25, ..dim() });
            }
            if pos + 1 < order.len() {
                if ui.icon_button(cv, id(SETTINGS, N_DOWN + tab as u32), down, icon::DOWN, false) {
                    action = Some((pos, 1));
                }
            } else {
                cv.glyph(icon::DOWN, down, false, D2D1_COLOR_F { a: 0.25, ..dim() });
            }
        }
        if let Some((pos, d)) = action {
            self.move_tab(pos, (pos as i32 + d) as usize);
        }

        let reset = Rect::new(x, rc.bottom() - 10.0 - 22.0, iw, 22.0);
        self.update_row(ui, cv, x, reset.y - 30.0, iw);
        if ui.solid_button(cv, id(SETTINGS, N_RESET), reset, "Сбросить внешний вид", chip_bg(), txt(), 7.0) {
            self.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let mut s = Settings::default();
        s.cfg = crate::config::parse(crate::config::DEFAULT_TEMPLATE);
        s
    }

    #[test]
    fn moving_a_tab_rewrites_the_full_order() {
        let mut s = settings();
        s.move_tab(0, 1);
        assert_eq!(s.cfg.tabs[0], "Today");
        assert_eq!(s.cfg.tabs[1], "Controls");
        assert_eq!(s.cfg.tabs.len(), 7);
        assert!(s.changed);
        let mut c = s.take_change().unwrap();
        assert!(s.take_change().is_none(), "изменение отдаётся один раз");
        c.hidden_tabs.clear();
        assert_eq!(c.resolve_order(&crate::TABS)[0], crate::tabs::TODAY);
    }

    #[test]
    fn hiding_is_case_insensitive_and_reversible() {
        let mut s = settings();
        s.cfg.hidden_tabs = vec!["power".into()];
        assert!(s.hidden(crate::tabs::POWER));
        s.set_hidden(crate::tabs::POWER, false);
        assert!(!s.hidden(crate::tabs::POWER) && s.cfg.hidden_tabs.is_empty());
        s.set_hidden(crate::tabs::STATS, true);
        s.set_hidden(crate::tabs::STATS, true);
        assert_eq!(s.cfg.hidden_tabs, vec!["Stats".to_string()], "повторное скрытие не плодит записи");
    }

    #[test]
    fn picking_a_color_turns_system_colors_off() {
        let mut s = settings();
        s.cfg.system_colors = true;
        s.set_hsv([200.0, 0.5, 0.8]);
        assert!(!s.cfg.system_colors && s.changed);
        assert!(close(s.cfg.accent, theme::hsv_to_rgb([200.0, 0.5, 0.8])));
        s.target = Target::Bg;
        s.set_hsv([10.0, 0.2, 0.1]);
        assert!(close(s.cfg.bg, theme::hsv_to_rgb([10.0, 0.2, 0.1])));
    }

    #[test]
    fn scale_snaps_to_five_percent() {
        assert_eq!(snap_scale(0.0, 1.0), MIN_SCALE);
        assert_eq!(snap_scale(1.0, 1.0), MAX_SCALE);
        assert!((snap_scale(0.27, 1.0) - 1.2).abs() < 1e-4);
        assert!((snap_scale(0.5, 1.0) - 1.45).abs() < 1e-4);
        // При 125% суммарный масштаб кратен 5%: 1.2 × 125% = 150%.
        let total = snap_scale(0.27, 1.25) * 1.25 * 100.0;
        assert!((total / 5.0 - (total / 5.0).round()).abs() < 1e-2, "{total}");
    }

    #[test]
    fn reset_restores_defaults() {
        let mut s = settings();
        s.cfg.ui_scale = 1.8;
        s.cfg.corner_radius = 3.0;
        s.cfg.hidden_tabs = vec!["Apps".into()];
        s.cfg.idle_face = false;
        s.reset();
        assert_eq!(s.cfg.corner_radius, DEFAULT_RADIUS);
        assert_eq!(s.cfg.ui_scale, DEFAULT_SCALE);
        assert!(s.cfg.hidden_tabs.is_empty() && s.cfg.idle_face);
        assert_eq!(s.cfg.resolve_visible(&crate::TABS).len(), 7);
    }
}
