//! Вкладка Controls: Wi-Fi, Bluetooth, звук, ночной свет (ярлык в Параметры), тёмная тема,
//! громкость, медиаплеер. Раскладка снята с макета 1:1 (панель 400×242).
//! Медленные вызовы (WinRT, COM) идут в фоновом потоке `bg`; здесь только снимок состояния.

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::anim::Spring;
use crate::bg::{self, Cmd, DevItem, ListKind, Media, Timeline};
use crate::sys;
use crate::ios::SysColor;
use crate::ui::{hue, icon, mix, on_color, slider_fill, Canvas, Chevron, Rect, Submit, Ui, chip_bg, dim, ink, txt, WHITE};

use super::grid::{Grid, Item};
use super::layout::{sz, Size, Spec, TabSpec, CELL, GAP};
use super::{id, CONTROLS};

/// Сколько после перетаскивания слайдера не принимать громкость из снимка (иначе ползунок дёргается).
const HOLD: Duration = Duration::from_millis(700);

// Номера виджетов в `SPEC` (они же биты маски недоступных виджетов).
const WIFI: usize = 0;
const BLUETOOTH: usize = 1;
const AUDIO: usize = 2;
const NIGHT: usize = 3;
const DARK: usize = 4;
const VOLUME: usize = 5;
const BRIGHTNESS: usize = 6;
const MEDIA: usize = 7;

/// Высота выпадающего списка устройств.
const LIST_H: f32 = 116.0;

static TILE: [Size; 4] = [sz(4, 3), sz(8, 3), sz(12, 3), sz(24, 3)];
static SLIDER: [Size; 2] = [sz(12, 2), sz(24, 2)];
static PLAYER: [Size; 3] = [sz(24, 4), sz(12, 6), sz(24, 6)];

/// Сетка вкладки: 24 колонки по 16 px (шаг CELL=16, зазор GAP=8), 16 рядов. Плитки по умолчанию занимают
/// 8 ячеек (1/3 ширины) или 12 ячеек (1/2 ширины) при высоте 3 ячейки (40 px).
///
/// Статический `min_rows = 4` задаёт компактный размер при скрытых виджетах или пустой вкладке.
/// Выпадающий список устройств LIST_H = 116 px запрашивает место динамически через `overlay_rows`.
pub static SPEC: TabSpec = TabSpec {
    cols: 24,
    rows: 16,
    min_cols: 8,
    min_rows: 4,
    specs: &[
        Spec { key: "wifi", title: "Wi-Fi", sizes: &TILE, size: 1, col: 0, row: 0 },
        Spec { key: "bluetooth", title: "Bluetooth", sizes: &TILE, size: 1, col: 8, row: 0 },
        Spec { key: "audio", title: "Аудио", sizes: &TILE, size: 1, col: 16, row: 0 },
        Spec { key: "night", title: "Ночной свет", sizes: &TILE, size: 2, col: 0, row: 3 },
        Spec { key: "dark", title: "Тёмная тема", sizes: &TILE, size: 2, col: 12, row: 3 },
        Spec { key: "volume", title: "Громкость", sizes: &SLIDER, size: 1, col: 0, row: 6 },
        Spec { key: "brightness", title: "Яркость", sizes: &SLIDER, size: 1, col: 0, row: 14 },
        Spec { key: "media", title: "Плеер", sizes: &PLAYER, size: 2, col: 0, row: 8 },
    ],
};

/// Вычисляет число рядов сетки, необходимое для выпадающего списка устройств.
/// Динамический расчёт overlay_rows:
/// Список устройств LIST_H = 116 px открывается под плитками (низ на `low` px от верха окна).
/// С учётом зазоров GAP между плитками и списком и запаса GAP до нижнего края окна:
/// ceil((low + GAP + LIST_H + GAP) / CELL).
/// Для плиток по умолчанию в 2 ряда (низ row 6 -> low = 88 px):
/// (88 + 8 + 116 + 8) / 16 = 220 / 16 = 13.75 -> 14 рядов.
#[inline]
pub fn controls_overlay_rows(low: f32) -> u8 {
    let needed = low + GAP + LIST_H + GAP;
    (needed / CELL).ceil().max(0.0) as u8
}

/// Прозрачность выпадающего списка: пружина, которой у `Default` нет.
struct Fade(Spring);

impl Default for Fade {
    fn default() -> Self {
        Fade(Spring::unit(0.0, 420.0, 34.0))
    }
}

fn state(on: bool) -> &'static str {
    if on {
        "Вкл"
    } else {
        "Выкл"
    }
}

/// Форматирует время в секундах в строку «M:SS» или «H:MM:SS».
fn format_time(secs: f32) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return "0:00".into();
    }
    let s = secs as u32;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let s = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Пересчитывает координату курсора `click_x` вдоль прогресс-бара шириной `bar_w` от `bar_x`
/// в позицию трека (в секундах) с зажимом в границы `min_seek` и `max_seek`.
#[cfg(test)]
fn seek_position(click_x: f32, bar_x: f32, bar_w: f32, dur: f32, min_seek: f32, max_seek: f32) -> f32 {
    Timeline { pos: 0.0, dur, min_seek, max_seek, can_seek: true }.seek_to(click_x, bar_x, bar_w)
}

/// Смещает позицию трека на `offset` секунд (например, −10.0 или +10.0) с зажимом в допустимые границы.
#[cfg(test)]
fn seek_relative(current_pos: f32, offset: f32, dur: f32, min_seek: f32, max_seek: f32) -> f32 {
    Timeline { pos: current_pos, dur, min_seek, max_seek, can_seek: true }.relative(offset)
}

#[derive(Default)]
pub struct Controls {
    wifi: bool,
    bluetooth: bool,
    dark: bool,
    /// Ночной свет; `None` — запись в реестре незнакомая, переключать нельзя (плитка открывает Параметры).
    night: Option<bool>,
    muted: bool,
    volume: f32,
    hold_until: Option<Instant>,
    brightness: f32,
    bright_hold: Option<Instant>,
    rev: u64,
    media: Option<Media>,
    /// Защита от возврата ползунка трека назад сразу после перемотки.
    seek_hold: Option<Instant>,
    /// Время последней отправки Cmd::Seek во время перетаскивания (для троттлинга).
    last_seek_send: Option<Instant>,
    ssid: Option<String>,
    audio: Option<String>,
    /// Раскрытый выпадающий список (занимает место громкости и плеера).
    open: Option<ListKind>,
    list: Vec<DevItem>,
    /// Секунд с открытия списка Bluetooth (его обновляем реже: перечисление дорогое).
    open_ticks: u32,
    /// Сеть Wi-Fi, для которой вводится пароль (вместо списка показывается поле).
    join: Option<DevItem>,
    /// Последний введённый пароль короче 8 символов (подсказка под полем).
    short_key: bool,
    /// Какой список рисуется (и пока гаснет после закрытия) и насколько он проявлен.
    shown: Option<ListKind>,
    list_t: Fade,
}

impl Controls {
    /// Расстояние от верха содержимого до низа видимых плиток (в логических пикселях).
    /// Считается строго по дискретным координатам раскладки сетки во избежание дрожания.
    pub fn tiles_low(grid: &Grid) -> f32 {
        let mut max_r = 0u8;
        for &idx in &[WIFI, BLUETOOTH, AUDIO, NIGHT, DARK] {
            if let Some(e) = grid.layout().entry(idx) {
                if !e.hidden {
                    let sz = SPEC.specs[idx].sizes[e.size];
                    max_r = max_r.max(e.row + sz.h);
                }
            }
        }
        let r_rel = max_r.saturating_sub(grid.window().r0);
        if r_rel > 0 {
            r_rel as f32 * super::layout::CELL - super::layout::GAP
        } else {
            0.0
        }
    }

    pub fn show(&mut self) -> bool {
        self.dark = sys::dark_mode();
        self.night = sys::night_light();
        self.seek_hold = None;
        self.last_seek_send = None;
        self.list_t.0.target = 0.0;
        self.list_t.0.snap();
        bg::send(Cmd::Refresh);
        self.sync();
        true
    }

    /// Вкладка скрыта или её начали править: список закрывается.
    pub fn hide(&mut self) {
        self.open = None;
        self.shown = None;
        self.list_t.0.target = 0.0;
        self.list_t.0.snap();
        self.join = None;
        self.seek_hold = None;
        self.last_seek_send = None;
    }

    pub fn step(&mut self, dt: f32) {
        self.list_t.0.step(dt);
    }

    pub fn animating(&self) -> bool {
        !self.list_t.0.settled()
    }

    /// Открывает/закрывает список устройств по стрелке плитки.
    fn toggle(&mut self, kind: ListKind) {
        self.join = None;
        if self.open == Some(kind) {
            self.open = None;
        } else {
            self.open = Some(kind);
            self.list = bg::list(kind);
            self.open_ticks = 0;
            bg::send(Cmd::List(kind));
        }
    }

    pub fn poll(&mut self) -> bool {
        bg::send(Cmd::Tick);
        // Ночной свет могут переключить и из Windows (центр уведомлений, расписание): чтение реестра дешёвое.
        let night = sys::night_light();
        let night_changed = night != self.night;
        self.night = night;
        if let Some(kind) = self.open {
            self.open_ticks += 1;
            if kind != ListKind::Bluetooth || self.open_ticks % 6 == 0 {
                bg::send(Cmd::List(kind));
            }
        }
        // Сообщение о подключении гаснет по времени — перерисовываем, пока оно есть.
        let status = self.open.is_some_and(|k| bg::status(k).is_some());
        // Прогресс трека двигается и между обновлениями снимка.
        night_changed || status || self.media.as_ref().is_some_and(|m| m.playing)
    }

    /// Забирает свежий снимок фонового потока. `true` — что-то изменилось.
    pub fn sync(&mut self) -> bool {
        let s = bg::snapshot();
        if s.rev == self.rev {
            return false;
        }
        self.rev = s.rev;
        self.wifi = s.wifi.unwrap_or(false);
        self.bluetooth = s.bluetooth.unwrap_or(false);
        self.muted = s.muted;
        let holding = self.hold_until.is_some_and(|t| Instant::now() < t);
        if let (Some(v), false) = (s.volume, holding) {
            self.volume = v;
        }
        let holding = self.bright_hold.is_some_and(|t| Instant::now() < t);
        if let (Some(v), false) = (s.brightness, holding) {
            self.brightness = v;
        }
        // Позиция трека у приложений приходит неточной: пока трек тот же и расхождение мало,
        // оставляем свой отсчёт, иначе время дёргается назад-вперёд.
        // Если только что была перемотка — удерживаем установленную позицию, пока плеер применяет её.
        let seeking = self.seek_hold.is_some_and(|t| Instant::now() < t);
        self.media = match (s.media, self.media.take()) {
            (Some(mut new), Some(old)) if new.key == old.key => {
                if seeking || (new.playing && old.playing && new.rate == old.rate && (new.position_now() - old.position_now()).abs() < 2.0) {
                    new.pos = old.pos;
                    new.at = old.at;
                }
                Some(new)
            }
            (new, _) => new,
        };
        self.ssid = s.ssid;
        self.audio = s.audio;
        if let Some(kind) = self.open {
            self.list = bg::list(kind);
        }
        true
    }

    /// Рисует вкладку: виджеты по сетке, а поверх них — выпадающий список устройств. `h` — высота
    /// панели без полосы правки.
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, grid: &mut Grid, w: f32, _h: f32) {
        if let Some(kind) = self.open {
            self.shown = Some(kind);
        }
        self.list_t.0.target = if self.open.is_some() { 1.0 } else { 0.0 };
        if self.open.is_none() && self.list_t.0.settled() {
            self.shown = None;
        }

        let low = Self::tiles_low(grid);
        let list_active = self.open.is_some() || (self.shown.is_some() && self.list_t.0.value > 0.0);
        let overlay_rows = if list_active {
            controls_overlay_rows(low)
        } else {
            0
        };
        grid.set_overlay_rows(overlay_rows);

        let area = grid.area(w);
        let skip = if crate::bright::available() { 0 } else { 1 << BRIGHTNESS };
        // Виджеты под списком гаснут и не принимают нажатий (иначе они перехватят их у строк списка).
        let cover = (self.list_t.0.value > 0.004).then(|| self.list_rect(area, low));
        grid.draw(ui, cv, area, skip, |ui, cv, it| self.widget(ui, cv, it, cover));
        self.overlay(ui, cv, area, low);
    }

    /// Где лежит список устройств: сразу под самой нижней плиткой (а если места не хватает —
    /// прижат к низу области).
    fn list_rect(&self, area: Rect, low: f32) -> Rect {
        let y = (area.y + low + GAP).min(area.bottom() - LIST_H).max(area.y);
        Rect::new(area.x, y, area.w, LIST_H)
    }

    unsafe fn overlay(&mut self, ui: &mut Ui, cv: &Canvas, area: Rect, low: f32) {
        let t = self.list_t.0.value.clamp(0.0, 1.0);
        let Some(kind) = self.shown.filter(|_| t > 0.01) else { return };
        let mut r = self.list_rect(area, low);
        r.y += (1.0 - t) * 6.0;
        let ocv = Canvas { g: cv.g, alpha: cv.alpha * t, accent: cv.accent };
        // Закрывающийся список уже не принимает нажатий.
        let prev = if self.open.is_none() { Some(ui.mute()) } else { None };
        self.device_list(ui, &ocv, r, kind);
        if let Some(p) = prev {
            ui.unmute(p);
        }
    }

    /// Один виджет сетки; если его закрывает открытый список, он гаснет и становится немым.
    unsafe fn widget(&mut self, ui: &mut Ui, cv: &Canvas, it: &Item, cover: Option<Rect>) {
        let covered = cover.is_some_and(|c| c.intersects(&it.rect));
        let k = if covered { 1.0 - self.list_t.0.value.clamp(0.0, 1.0) } else { 1.0 };
        if k < 0.004 {
            return;
        }
        let sub = Canvas { g: cv.g, alpha: cv.alpha * k, accent: cv.accent };
        let prev = if covered { Some(ui.mute()) } else { None };
        self.paint(ui, &sub, it);
        if let Some(p) = prev {
            ui.unmute(p);
        }
    }

    unsafe fn paint(&mut self, ui: &mut Ui, cv: &Canvas, it: &Item) {
        let r = it.rect;
        match it.idx {
            WIFI => {
                let sub = if self.wifi { self.ssid.clone().unwrap_or_else(|| "Вкл".into()) } else { "Выкл".into() };
                let hit = ui.tile_wide(cv, id(CONTROLS, 0), r, icon::WIFI, "Wi-Fi", &sub, &mut self.wifi, Chevron::Plain, hue(SysColor::Blue, cv.accent));
                if hit.toggled {
                    bg::send(Cmd::Wifi(self.wifi));
                }
                if hit.chevron {
                    self.toggle(ListKind::Wifi);
                }
            }
            BLUETOOTH => {
                let hit = ui.tile_wide(cv, id(CONTROLS, 1), r, icon::BLUETOOTH, "Bluetooth", state(self.bluetooth), &mut self.bluetooth, Chevron::Divided, hue(SysColor::Blue, cv.accent));
                if hit.toggled {
                    bg::send(Cmd::Bluetooth(self.bluetooth));
                }
                if hit.chevron {
                    self.toggle(ListKind::Bluetooth);
                }
            }
            AUDIO => {
                let mut sound = !self.muted;
                let sub = if sound { self.device() } else { "Без звука".into() };
                let hit = ui.tile_wide(cv, id(CONTROLS, 3), r, icon::VOLUME, "Аудио", &sub, &mut sound, Chevron::Divided, hue(SysColor::Pink, cv.accent));
                if hit.toggled {
                    self.muted = !sound;
                    bg::send(Cmd::Mute(self.muted));
                }
                if hit.chevron {
                    self.toggle(ListKind::Audio);
                }
            }
            NIGHT => {
                // Ночной свет переключается через реестр, стрелка открывает Параметры.
                let mut night = self.night.unwrap_or(false);
                let sub = match self.night {
                    Some(on) => state(on),
                    None => "Параметры",
                };
                let hit = ui.tile_wide(cv, id(CONTROLS, 4), r, icon::MOON, "Ночной свет", sub, &mut night, Chevron::Plain, hue(SysColor::Orange, cv.accent));
                if hit.chevron || (hit.toggled && self.night.is_none()) {
                    sys::open("ms-settings:nightlight");
                } else if hit.toggled {
                    if !sys::set_night_light(night) {
                        crate::log::error("не удалось переключить ночной свет");
                    }
                    self.night = sys::night_light();
                }
            }
            DARK => {
                let hit = ui.tile_wide(cv, id(CONTROLS, 2), r, icon::CONTRAST, "Тёмная тема", state(self.dark), &mut self.dark, Chevron::None, hue(SysColor::Indigo, cv.accent));
                if hit.toggled {
                    let dark = self.dark;
                    // Рассылка WM_SETTINGCHANGE может занять время — не в потоке интерфейса.
                    std::thread::spawn(move || sys::set_dark_mode(dark));
                }
            }
            VOLUME => {
                let g = match self.volume {
                    _ if self.muted => icon::MUTE,
                    v if v < 0.01 => icon::MUTE,
                    v if v < 0.34 => icon::VOLUME1,
                    v if v < 0.67 => icon::VOLUME2,
                    _ => icon::VOLUME3,
                };
                if ui.slider_bar(cv, id(CONTROLS, 10), r, g, &mut self.volume, slider_fill(cv.accent)) {
                    self.hold_until = Some(Instant::now() + HOLD);
                    // Как в Windows: громкость выше нуля снимает «без звука».
                    if self.muted && self.volume > 0.0 {
                        self.muted = false;
                        bg::send(Cmd::Mute(false));
                    }
                    bg::send(Cmd::Volume(self.volume));
                }
            }
            BRIGHTNESS => {
                if ui.slider_bar(cv, id(CONTROLS, 11), r, icon::BRIGHTNESS, &mut self.brightness, slider_fill(cv.accent)) {
                    self.bright_hold = Some(Instant::now() + HOLD);
                    bg::send(Cmd::Brightness(self.brightness));
                }
            }
            MEDIA => {
                let device = self.device();
                self.media_card(ui, cv, r, it.size, &device);
            }
            _ => {}
        }
    }

    /// Устройство вывода звука для подписей.
    fn device(&self) -> String {
        self.audio.clone().unwrap_or_else(|| "Динамики".into())
    }

    /// Выпадающий список устройств вместо громкости и плеера.
    unsafe fn device_list(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, kind: ListKind) {
        const ROW: f32 = 22.0;
        cv.card(r);
        let f = &cv.g.fonts;
        let (title, glyph, settings, empty) = match kind {
            ListKind::Wifi => ("СЕТИ WI-FI", icon::WIFI, "ms-settings:network-wifi", "Сети не найдены (Wi-Fi выключен?)"),
            ListKind::Bluetooth => ("BLUETOOTH", icon::BLUETOOTH, "ms-settings:bluetooth", "Нет сопряжённых устройств"),
            ListKind::Audio => ("УСТРОЙСТВА ВЫВОДА", icon::VOLUME, "ms-settings:sound", "Устройства не найдены"),
        };
        match bg::status(kind) {
            Some(st) => cv.text(&st, Rect::new(r.x + 11.0, r.y + 3.0, r.w - 100.0, 16.0), &f.small_b, cv.accent),
            None => cv.text(title, Rect::new(r.x + 11.0, r.y + 3.0, 180.0, 16.0), &f.caps, dim()),
        }
        let link = Rect::new(r.right() - 82.0, r.y + 3.0, 74.0, 16.0);
        let (_, open_settings) = ui.click_area(id(CONTROLS, 40), link);
        let hv = ui.hover_of(id(CONTROLS, 40));
        cv.text("Параметры", link, &f.small_r, mix(dim(), txt(), hv));
        if open_settings {
            sys::open(settings);
        }

        if kind == ListKind::Wifi && self.join.is_some() {
            self.join_form(ui, cv, r);
            return;
        }

        let view = Rect::new(r.x + 5.0, r.y + 21.0, r.w - 10.0, r.h - 25.0);
        let items = self.list.clone();
        let sid = id(CONTROLS, 41);
        let off = ui.scroll_begin(cv, sid, view, items.len() as f32 * ROW);
        let mut chosen = None;
        for (i, it) in items.iter().enumerate() {
            let y = view.y + i as f32 * ROW - off;
            if y + ROW < view.y || y > view.bottom() {
                continue;
            }
            let row = Rect::new(view.x, y, view.w - 4.0, ROW - 2.0);
            let lead = match kind {
                _ if it.active => icon::CHECK,
                // Сеть, для которой нужен пароль.
                ListKind::Wifi if it.id.is_empty() && it.extra.starts_with("WPA") => icon::LOCK,
                _ => glyph,
            };
            // Плашка значка — цветом функции, подключённое устройство — зелёной с галочкой.
            let tint = match kind {
                _ if it.active => hue(SysColor::Green, cv.accent),
                ListKind::Audio => hue(SysColor::Pink, cv.accent),
                ListKind::Wifi | ListKind::Bluetooth => hue(SysColor::Blue, cv.accent),
            };
            if ui.row_compact_tinted(cv, id(CONTROLS, 100 + i as u32), row, lead, true, &it.name, &it.sub, 4.0, it.active, false, Some(tint)) {
                chosen = Some(i);
            }
        }
        ui.scroll_end(cv, sid);
        if items.is_empty() {
            cv.text(empty, Rect::new(r.x + 11.0, r.y + 30.0, r.w - 22.0, 18.0), &f.small, dim());
        }
        if let Some(i) = chosen {
            let it = &items[i];
            let busy = bg::status(kind).is_some_and(|s| s.ends_with('…'));
            match kind {
                _ if busy => {}
                ListKind::Wifi => {
                    if it.active {
                        bg::send(Cmd::WifiDisconnect);
                    } else if !it.id.is_empty() {
                        bg::send(Cmd::WifiConnect(it.name.clone(), it.id.clone()));
                    } else if it.extra == "open|none" {
                        bg::send(Cmd::WifiJoin(it.name.clone(), it.extra.clone(), String::new()));
                    } else if !it.extra.is_empty() {
                        // Новая защищённая сеть: пароль вводится прямо в острове.
                        let field = id(CONTROLS, 50);
                        ui.set_field_text(field, "");
                        ui.set_secret(field, true);
                        ui.focus = Some(field);
                        ui.caret_on = true;
                        self.join = Some(it.clone());
                        self.short_key = false;
                    } else {
                        // Корпоративная сеть (логин и сертификат) — только через Параметры.
                        sys::open(settings);
                    }
                }
                ListKind::Audio => {
                    if !it.extra.is_empty() {
                        // Отключённые Bluetooth-наушники: подключить и сделать основными.
                        bg::send(Cmd::BtAudio(it.extra.clone(), true));
                    } else if !it.active {
                        bg::send(Cmd::AudioDefault(it.id.clone()));
                    }
                }
                ListKind::Bluetooth => {
                    if it.extra.is_empty() {
                        // Не аудио (мышь, клавиатура…): публичного API подключения нет.
                        sys::open(settings);
                    } else {
                        bg::send(Cmd::BtAudio(it.extra.clone(), !it.active));
                    }
                }
            }
        }
    }

    /// Ввод пароля новой сети Wi-Fi на месте списка. Enter или «Подключить» — подключение.
    unsafe fn join_form(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect) {
        let Some(net) = self.join.clone() else { return };
        let f = &cv.g.fonts;
        let field = id(CONTROLS, 50);
        cv.glyph(icon::WIFI, Rect::new(r.x + 11.0, r.y + 22.0, 14.0, 16.0), false, dim());
        cv.text(&net.name, Rect::new(r.x + 30.0, r.y + 22.0, r.w - 40.0, 16.0), &f.body, txt());
        ui.text_field_ex(cv, field, Rect::new(r.x + 8.0, r.y + 43.0, r.w - 16.0, 26.0), "Пароль сети", icon::LOCK, Submit::Keep);
        let mut go = ui.take_submitted(field);
        let bw = 92.0;
        let by = r.bottom() - 32.0;
        if ui.solid_button(cv, id(CONTROLS, 51), Rect::new(r.right() - 8.0 - bw, by, bw, 24.0), "Подключить", cv.accent, ink(), 8.0) {
            go = Some(ui.field_text(field));
        }
        if ui.solid_button(cv, id(CONTROLS, 52), Rect::new(r.right() - 14.0 - 2.0 * bw, by, bw, 24.0), "Отмена", chip_bg(), txt(), 8.0) {
            self.join = None;
            ui.blur();
            return;
        }
        let hint = Rect::new(r.x + 11.0, by + 4.0, 160.0, 16.0);
        match go {
            // У WPA пароль 8–63 символа: короче система всё равно не примет.
            Some(key) if key.chars().count() < 8 => {
                self.short_key = true;
                ui.focus = Some(field);
            }
            Some(key) => {
                bg::send(Cmd::WifiJoin(net.name.clone(), net.extra.clone(), key));
                ui.set_field_text(field, "");
                ui.blur();
                self.join = None;
                self.short_key = false;
                return;
            }
            None => {}
        }
        let (msg, color) = if self.short_key { ("Нужно не меньше 8 символов", cv.accent) } else { ("Enter — подключить", dim()) };
        cv.text(msg, hint, &f.small, color);
    }

    /// Круглая кнопка плеера. `true` — нажата.
    #[allow(clippy::too_many_arguments)]
    unsafe fn round_button(ui: &mut Ui, cv: &Canvas, id: u32, cx: f32, cy: f32, rad: f32, glyph: &str, fill: D2D1_COLOR_F, fg: D2D1_COLOR_F) -> bool {
        let r = Rect::new(cx - rad, cy - rad, 2.0 * rad, 2.0 * rad);
        let (_, clicked) = ui.click_area(id, r);
        let hv = ui.hover_of(id);
        cv.circle(cx, cy, rad + 0.8 * hv, mix(fill, crate::ui::fg(), 0.1 * hv));
        cv.glyph(glyph, r, false, fg);
        clicked
    }

    /// Плеер в трёх вариантах (`size` — индекс в `PLAYER`): 0 — низкая полоса без времени и имени
    /// устройства, 1 — узкая карточка с кнопками во втором ряду, 2 — полная.
    unsafe fn media_card(&mut self, ui: &mut Ui, cv: &Canvas, r: Rect, size: usize, device: &str) {
        let (compact, narrow) = (size == 0, size == 1);
        cv.card(r);
        let f = &cv.g.fonts;
        if !compact && !narrow {
            cv.glyph(icon::VOLUME, Rect::new(r.x + 8.0, r.y + 4.0, 12.0, 14.0), false, dim());
            cv.text(device, Rect::new(r.x + 21.0, r.y + 4.0, 150.0, 14.0), &f.small, dim());
        }

        let art = if compact {
            Rect::new(r.x + 10.0, r.cy() - 18.0, 36.0, 36.0)
        } else if narrow {
            Rect::new(r.x + 10.0, r.y + 9.0, 36.0, 36.0)
        } else {
            Rect::new(r.x + 11.0, r.y + 24.0, 36.0, 36.0)
        };
        let cover = self.media.as_ref().and_then(|m| m.art.clone().map(|px| (m.key, px)));
        let mut has_cover = false;
        if let Some((key, px)) = cover {
            let key = (0x4Du64 << 56) | (key >> 8);
            if let Some(bmp) = ui.cache.get(&cv.g.dc, key, bg::ART, bg::ART, || px.to_vec()) {
                cv.bitmap_quad(Some(&bmp), art, 0.0, 1.0);
                has_cover = true;
            }
        }
        if !has_cover {
            let (top, bottom) = (D2D1_COLOR_F { r: 0.35, g: 0.22, b: 0.47, a: 1.0 }, D2D1_COLOR_F { r: 0.76, g: 0.42, b: 0.60, a: 1.0 });
            cv.gradient_rr(art, 6.0, top, bottom);
        }

        let (title, artist, playing) = match &self.media {
            Some(m) => (m.title.as_str(), m.artist.as_str(), m.playing),
            None => ("Ничего не играет", "", false),
        };
        let mut tl = self.media.as_ref().map_or(Timeline::empty(), |m| m.timeline());
        if self.seek_hold.is_some_and(|t| Instant::now() < t) {
            if let Some(m) = &self.media {
                tl.pos = m.pos;
            }
        }
        if !has_cover {
            cv.glyph(icon::MUSIC, art, false, D2D1_COLOR_F { a: 0.7, ..WHITE });
        }
        let (tx, ty, tw) = if compact {
            let right_limit = r.right() - 95.0;
            (r.x + 56.0, r.cy() - 17.0, (right_limit - (r.x + 56.0)).max(40.0))
        } else if narrow {
            (r.x + 56.0, r.y + 10.0, r.w - 66.0)
        } else {
            let right_limit = r.right() - 95.0;
            (r.x + 56.0, r.y + 25.0, (right_limit - (r.x + 56.0)).max(40.0))
        };
        cv.text(title, Rect::new(tx, ty, tw, 20.0), &f.title, txt());
        cv.text(artist, Rect::new(tx, ty + 18.0, tw, 14.0), &f.small, dim());

        // Кнопки плеера: prev, play/pause, next; перемотка только по полосе прогресса и колесу мыши.
        let (cy, xs) = if narrow {
            (r.y + 57.0, [r.cx() - 30.0, r.cx(), r.cx() + 30.0])
        } else {
            (if compact { r.cy() } else { r.y + 40.0 }, [r.right() - 75.0, r.right() - 45.0, r.right() - 16.0])
        };

        let btn_rad = if narrow { 9.0 } else { 9.5 };
        let play_rad = if narrow { 13.0 } else { 14.0 };

        if Self::round_button(ui, cv, id(CONTROLS, 20), xs[0], cy, btn_rad, icon::PREV, chip_bg(), txt()) {
            bg::send(Cmd::Prev);
        }
        let toggle = if playing { icon::PAUSE } else { icon::PLAY_SOLID };
        if Self::round_button(ui, cv, id(CONTROLS, 21), xs[1], cy, play_rad, toggle, slider_fill(cv.accent), on_color(slider_fill(cv.accent))) {
            bg::send(Cmd::PlayPause);
        }
        if Self::round_button(ui, cv, id(CONTROLS, 22), xs[2], cy, btn_rad, icon::NEXT, chip_bg(), txt()) {
            bg::send(Cmd::Next);
        }

        // Прогресс и время.
        if compact {
            let bar_r = Rect::new(r.x + 10.0, r.bottom() - 7.0, r.w - 20.0, 2.0);
            self.draw_seek_bar(ui, cv, id(CONTROLS, 23), bar_r, tl);
            return;
        }

        let py = r.y + 76.0;
        let time_w = if tl.dur >= 3600.0 { 44.0 } else { 32.0 };
        let left_rect = Rect::new(r.x + 9.0, py - 6.0, time_w, 12.0);
        let right_rect = Rect::new(r.right() - 9.0 - time_w, py - 6.0, time_w, 12.0);
        let bar_x = left_rect.right() + 5.0;
        let bar_w = (right_rect.x - 5.0 - bar_x).max(10.0);
        let bar_r = Rect::new(bar_x, py - 1.0, bar_w, 2.0);

        cv.text(&format_time(tl.pos), left_rect, &f.small, dim());
        cv.text(&format_time(tl.dur), right_rect, &f.small_r, dim());

        self.draw_seek_bar(ui, cv, id(CONTROLS, 23), bar_r, tl);
    }

    fn apply_seek(&mut self, target_pos: f32, force: bool) {
        if let Some(m) = &mut self.media {
            m.pos = target_pos;
            m.at = Instant::now();
        }
        self.seek_hold = Some(Instant::now() + HOLD);
        let now = Instant::now();
        let should_send = force
            || self
                .last_seek_send
                .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(100));
        if should_send {
            self.last_seek_send = Some(now);
            bg::send(Cmd::Seek(target_pos));
        }
    }

    /// Интерактивный прогресс-бар: клик, перетаскивание ползунка, колесо мыши над дорожкой.
    unsafe fn draw_seek_bar(
        &mut self,
        ui: &mut Ui,
        cv: &Canvas,
        id: u32,
        bar: Rect,
        tl: Timeline,
    ) {
        if !tl.can_seek || tl.dur <= 0.0 {
            // Сессия не поддерживает перемотку — безопасная деградация в статический прогресс-бар.
            cv.progress(bar, tl.frac(), if tl.dur > 0.0 { slider_fill(cv.accent) } else { dim() });
            return;
        }

        let hit = Rect::new(bar.x, bar.cy() - 7.0, bar.w, 14.0);
        let (hover, clicked) = ui.click_area(id, hit);
        let dragging = ui.is_dragging(id);
        let hv = ui.hover_of(id);

        let mut frac = tl.frac();
        if dragging || clicked {
            let target = tl.seek_to(ui.pointer().0, bar.x, bar.w);
            let final_drop = dragging && ui.released;
            let force = clicked || final_drop;
            self.apply_seek(target, force);
            if tl.dur > 0.0 {
                frac = (target / tl.dur).clamp(0.0, 1.0);
            }
        } else if hover && ui.wheel.abs() > 0.01 {
            let delta = ui.wheel.signum() * 10.0;
            let target = tl.relative(delta);
            self.apply_seek(target, true);
            if tl.dur > 0.0 {
                frac = (target / tl.dur).clamp(0.0, 1.0);
            }
        }

        let h = 2.0 + 1.0 * hv;
        let track_rect = Rect::new(bar.x, bar.cy() - h / 2.0, bar.w, h);
        cv.rr(track_rect, h / 2.0, mix(crate::ui::track(), crate::ui::fg(), 0.04 * hv));

        let fw = (bar.w * frac).max(h);
        let fill_rect = Rect::new(bar.x, bar.cy() - h / 2.0, fw, h);
        cv.rr(fill_rect, h / 2.0, slider_fill(cv.accent));

        let active = hv.max(if dragging { 1.0 } else { 0.0 });
        if active > 0.01 {
            let thumb_r = 3.2 + 1.6 * active;
            let thumb_x = (bar.x + bar.w * frac).clamp(bar.x, bar.right());
            cv.circle(thumb_x, bar.cy(), thumb_r, slider_fill(cv.accent));
            if crate::theme::glass() {
                cv.circle(thumb_x, bar.cy(), thumb_r * 0.4, WHITE);
            }
        }
    }
}

#[cfg(feature = "gallery")]
impl Controls {
    /// Демо-данные галереи: Wi-Fi и ночной свет включены, Bluetooth выключен, играет трек.
    pub fn demo(&mut self) {
        self.wifi = true;
        self.ssid = Some("Home-5G".into());
        self.bluetooth = false;
        self.dark = false;
        self.night = Some(true);
        self.volume = 0.62;
        self.brightness = 0.8;
        self.audio = Some("Наушники AirPods".into());
        self.media = Some(Media {
            title: "Blinding Lights".into(),
            artist: "The Weeknd".into(),
            playing: true,
            pos: 74.0,
            dur: 200.0,
            at: Instant::now(),
            rate: 0.0,
            art: None,
            key: 1,
            can_seek: true,
            min_seek: 0.0,
            max_seek: 200.0,
        });
    }

    /// Один виджет вне сетки: номер в `SPEC`, индекс размера и прямоугольник.
    pub unsafe fn gallery(&mut self, ui: &mut Ui, cv: &Canvas, idx: usize, size: usize, rect: Rect) {
        self.paint(ui, cv, &Item { idx, size, rect });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_time_zero_and_negative() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(-15.0), "0:00");
        assert_eq!(format_time(f32::NAN), "0:00");
        assert_eq!(format_time(f32::INFINITY), "0:00");
    }

    #[test]
    fn format_time_seconds_and_minutes() {
        assert_eq!(format_time(5.0), "0:05");
        assert_eq!(format_time(59.0), "0:59");
        assert_eq!(format_time(60.0), "1:00");
        assert_eq!(format_time(75.0), "1:15");
        assert_eq!(format_time(599.0), "9:59");
        assert_eq!(format_time(600.0), "10:00");
        assert_eq!(format_time(3599.0), "59:59");
    }

    #[test]
    fn format_time_hours() {
        assert_eq!(format_time(3600.0), "1:00:00");
        assert_eq!(format_time(3665.0), "1:01:05");
        assert_eq!(format_time(7325.0), "2:02:05");
    }

    #[test]
    fn seek_position_within_bounds() {
        assert_eq!(seek_position(100.0, 100.0, 200.0, 100.0, 0.0, 100.0), 0.0);
        assert_eq!(seek_position(200.0, 100.0, 200.0, 100.0, 0.0, 100.0), 50.0);
        assert_eq!(seek_position(300.0, 100.0, 200.0, 100.0, 0.0, 100.0), 100.0);
        assert_eq!(seek_position(150.0, 100.0, 200.0, 100.0, 0.0, 100.0), 25.0);
    }

    #[test]
    fn seek_position_clamps_outside_range() {
        assert_eq!(seek_position(50.0, 100.0, 200.0, 100.0, 0.0, 100.0), 0.0);
        assert_eq!(seek_position(350.0, 100.0, 200.0, 100.0, 0.0, 100.0), 100.0);
    }

    #[test]
    fn seek_position_respects_min_max_seek() {
        assert_eq!(seek_position(100.0, 100.0, 200.0, 100.0, 20.0, 80.0), 20.0);
        assert_eq!(seek_position(120.0, 100.0, 200.0, 100.0, 20.0, 80.0), 20.0);
        assert_eq!(seek_position(200.0, 100.0, 200.0, 100.0, 20.0, 80.0), 50.0);
        assert_eq!(seek_position(280.0, 100.0, 200.0, 100.0, 20.0, 80.0), 80.0);
        assert_eq!(seek_position(300.0, 100.0, 200.0, 100.0, 20.0, 80.0), 80.0);
    }

    #[test]
    fn seek_position_invalid_inputs() {
        assert_eq!(seek_position(150.0, 100.0, 0.0, 100.0, 0.0, 100.0), 0.0);
        assert_eq!(seek_position(150.0, 100.0, 200.0, 0.0, 0.0, 0.0), 0.0);
        assert_eq!(seek_position(150.0, 100.0, 200.0, -10.0, 0.0, 0.0), 0.0);
        assert_eq!(seek_position(f32::NAN, 100.0, 200.0, 100.0, 0.0, 100.0), 0.0);
    }

    #[test]
    fn seek_relative_forward_backward() {
        assert_eq!(seek_relative(50.0, 10.0, 100.0, 0.0, 100.0), 60.0);
        assert_eq!(seek_relative(50.0, -10.0, 100.0, 0.0, 100.0), 40.0);
    }

    #[test]
    fn seek_relative_clamps_to_boundaries() {
        assert_eq!(seek_relative(5.0, -10.0, 100.0, 0.0, 100.0), 0.0);
        assert_eq!(seek_relative(95.0, 10.0, 100.0, 0.0, 100.0), 100.0);
        assert_eq!(seek_relative(25.0, -10.0, 100.0, 20.0, 80.0), 20.0);
        assert_eq!(seek_relative(75.0, 10.0, 100.0, 20.0, 80.0), 80.0);
    }

    #[test]
    fn seek_relative_invalid_inputs() {
        assert_eq!(seek_relative(10.0, 10.0, 0.0, 0.0, 0.0), 0.0);
        assert_eq!(seek_relative(10.0, 10.0, -50.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn controls_default_layout_and_device_list_fit() {
        use crate::tabs::layout::{CELL, GAP, Layout};
        let lay = Layout::default_for(&SPEC);
        let p = lay.pack(&SPEC, 0);
        assert_eq!(p.len(), 8);

        // Проверяем отсутствие пересечений
        for (i, a) in p.iter().enumerate() {
            for b in &p[i + 1..] {
                let cols = a.col < b.col + b.w && b.col < a.col + a.w;
                let rows = a.row < b.row + b.h && b.row < a.row + a.h;
                assert!(!(cols && rows), "виджеты {} и {} пересекаются", a.idx, b.idx);
            }
        }

        // Проверяем размеры плитки: 8 ячеек = 120 px ширина, 3 ячейки = 40 px высота
        let wifi = lay.entry(WIFI).unwrap();
        let wifi_sz = SPEC.specs[WIFI].sizes[wifi.size];
        assert_eq!(wifi_sz.pixel_w(), 120.0);
        assert_eq!(wifi_sz.pixel_h(), 40.0);

        // Проверяем выпадающий список устройств: LIST_H=116 px
        // Низ самой нижней плитки (Night/Dark): row 3 + h 3 = row 6 -> y = 6 * 16 - 8 = 88 px.
        // y + GAP = 88.0 + 8.0 = 96.0 px (от верха содержимого).
        // Низ списка: 96 + 116 = 212 px.
        // При overlay_rows = 14 высота содержимого: 14 * 16 - 8 = 216 px (212 <= 216).
        let low_tiles = 6.0 * CELL - GAP;
        let list_top = low_tiles + GAP;
        let list_bottom = list_top + LIST_H;
        let area_h = 14.0 * CELL - GAP;
        assert!(
            list_bottom <= area_h,
            "список устройств не помещается: bottom {} > area_h {}",
            list_bottom,
            area_h
        );
        assert_eq!(controls_overlay_rows(low_tiles), 14);
    }

    #[test]
    fn test_controls_overlay_rows_and_tiles_low() {
        // По умолчанию: Night/Dark заканчиваются на row 6 -> low = 6*16 - 8 = 88 px
        assert_eq!(controls_overlay_rows(88.0), 14);

        // Только 1 ряд плиток: row 3 -> low = 3*16 - 8 = 40 px
        assert_eq!(controls_overlay_rows(40.0), 11);

        // Без плиток (все скрыты): low = 0 px
        assert_eq!(controls_overlay_rows(0.0), 9);

        // Плитки смещены в edit mode: low = 120 px -> 16 рядов
        assert_eq!(controls_overlay_rows(120.0), 16);

        // Проверяем tiles_low на сетке по умолчанию
        let mut grid = Grid::default();
        grid.configure(CONTROLS, &SPEC, "");
        let low = Controls::tiles_low(&grid);
        assert_eq!(low, 88.0);
        assert_eq!(controls_overlay_rows(low), 14);
    }
}
