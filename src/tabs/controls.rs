//! Вкладка Controls: Wi-Fi, Bluetooth, звук, ночной свет (ярлык в Параметры), тёмная тема,
//! громкость, медиаплеер. Раскладка снята с макета 1:1 (панель 400×242).
//! Медленные вызовы (WinRT, COM) идут в фоновом потоке `bg`; здесь только снимок состояния.

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::anim::Spring;
use crate::bg::{self, Cmd, DevItem, ListKind, Media};
use crate::sys;
use crate::ui::{icon, mix, Canvas, Chevron, Rect, Submit, Ui, chip_bg, dim, ink, txt, WHITE};

use super::grid::{Grid, Item};
use super::layout::{sz, Size, Spec, TabSpec, GAP};
use super::{id, CONTROLS, PAD, TOP};

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

static TILE: [Size; 4] = [sz(1, 40.0), sz(2, 40.0), sz(3, 40.0), sz(6, 40.0)];
static SLIDER: [Size; 2] = [sz(3, 24.0), sz(6, 24.0)];
static PLAYER: [Size; 3] = [sz(6, 56.0), sz(3, 90.0), sz(6, 90.0)];

/// Сетка вкладки: 6 колонок по ~59 px (402 px внутри полей); плитки по умолчанию занимают 2 или 3
/// колонки, как в исходной раскладке. Колонок шесть, а не четыре, потому что 402 px не делятся на
/// четыре «красивых» ширины плиток 127 и 196 px.
pub static SPEC: TabSpec = TabSpec {
    cols: 6,
    specs: &[
        Spec { key: "wifi", title: "Wi-Fi", sizes: &TILE, size: 1, col: 0 },
        Spec { key: "bluetooth", title: "Bluetooth", sizes: &TILE, size: 1, col: 2 },
        Spec { key: "audio", title: "Аудио", sizes: &TILE, size: 1, col: 4 },
        Spec { key: "night", title: "Ночной свет", sizes: &TILE, size: 2, col: 0 },
        Spec { key: "dark", title: "Тёмная тема", sizes: &TILE, size: 2, col: 3 },
        Spec { key: "volume", title: "Громкость", sizes: &SLIDER, size: 1, col: 0 },
        Spec { key: "brightness", title: "Яркость", sizes: &SLIDER, size: 1, col: 0 },
        Spec { key: "media", title: "Плеер", sizes: &PLAYER, size: 2, col: 0 },
    ],
};

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

fn mmss(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
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
    /// Низ самой нижней плитки в этом и прошлом кадре: под ней открывается список.
    low: f32,
    low_prev: f32,
}

impl Controls {
    pub fn show(&mut self) -> bool {
        self.dark = sys::dark_mode();
        self.night = sys::night_light();
        self.list_t.0.target = 0.0;
        self.list_t.0.snap();
        bg::send(Cmd::Refresh);
        self.sync();
        true
    }

    /// Вкладка скрыта или её начали править: список закрывается.
    pub fn hide(&mut self) {
        self.open = None;
        self.join = None;
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
        self.media = match (s.media, self.media.take()) {
            (Some(mut new), Some(old)) if new.key == old.key && new.playing && old.playing => {
                if (new.position_now() - old.position_now()).abs() < 2.0 {
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
    pub unsafe fn draw(&mut self, ui: &mut Ui, cv: &Canvas, grid: &mut Grid, w: f32, h: f32) {
        let area = Rect::new(PAD, TOP, w - 2.0 * PAD, h - TOP - PAD);
        let skip = if crate::bright::available() { 0 } else { 1 << BRIGHTNESS };
        if let Some(kind) = self.open {
            self.shown = Some(kind);
        }
        self.list_t.0.target = if self.open.is_some() { 1.0 } else { 0.0 };
        self.low_prev = std::mem::take(&mut self.low);
        // Виджеты под списком гаснут и не принимают нажатий (иначе они перехватят их у строк списка).
        let cover = (self.list_t.0.value > 0.004).then(|| self.list_rect(area));
        grid.draw(ui, cv, area, skip, |ui, cv, it| self.widget(ui, cv, it, cover));
        self.overlay(ui, cv, area);
    }

    /// Где лежит список устройств: сразу под самой нижней плиткой (а если места не хватает —
    /// прижат к низу области).
    fn list_rect(&self, area: Rect) -> Rect {
        let y = (self.low_prev + GAP).min(area.bottom() - LIST_H).max(area.y);
        Rect::new(area.x, y, area.w, LIST_H)
    }

    unsafe fn overlay(&mut self, ui: &mut Ui, cv: &Canvas, area: Rect) {
        let t = self.list_t.0.value.clamp(0.0, 1.0);
        let Some(kind) = self.shown.filter(|_| t > 0.01) else { return };
        let mut r = self.list_rect(area);
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
        if matches!(it.idx, WIFI | BLUETOOTH | AUDIO | NIGHT | DARK) {
            self.low = self.low.max(it.rect.bottom());
        }
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
                let hit = ui.tile_wide(cv, id(CONTROLS, 0), r, icon::WIFI, "Wi-Fi", &sub, &mut self.wifi, Chevron::Plain);
                if hit.toggled {
                    bg::send(Cmd::Wifi(self.wifi));
                }
                if hit.chevron {
                    self.toggle(ListKind::Wifi);
                }
            }
            BLUETOOTH => {
                let hit = ui.tile_wide(cv, id(CONTROLS, 1), r, icon::BLUETOOTH, "Bluetooth", state(self.bluetooth), &mut self.bluetooth, Chevron::Divided);
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
                let hit = ui.tile_wide(cv, id(CONTROLS, 3), r, icon::VOLUME, "Аудио", &sub, &mut sound, Chevron::Divided);
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
                let hit = ui.tile_wide(cv, id(CONTROLS, 4), r, icon::MOON, "Ночной свет", sub, &mut night, Chevron::Plain);
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
                let hit = ui.tile_wide(cv, id(CONTROLS, 2), r, icon::CONTRAST, "Тёмная тема", state(self.dark), &mut self.dark, Chevron::None);
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
                if ui.slider_bar(cv, id(CONTROLS, 10), r, g, &mut self.volume) {
                    self.hold_until = Some(Instant::now() + HOLD);
                    bg::send(Cmd::Volume(self.volume));
                }
            }
            BRIGHTNESS => {
                if ui.slider_bar(cv, id(CONTROLS, 11), r, icon::BRIGHTNESS, &mut self.brightness) {
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
            if ui.row_compact(cv, id(CONTROLS, 100 + i as u32), row, lead, true, &it.name, &it.sub, 4.0, it.active, false) {
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

        let (title, artist, pos, dur, playing) = match &self.media {
            Some(m) => (m.title.as_str(), m.artist.as_str(), m.position_now(), m.dur, m.playing),
            None => ("Ничего не играет", "", 0.0, 0.0, false),
        };
        if !has_cover {
            cv.glyph(icon::MUSIC, art, false, D2D1_COLOR_F { a: 0.7, ..WHITE });
        }
        let (tx, ty, tw) = if compact {
            (r.x + 56.0, r.cy() - 17.0, r.w - 56.0 - 100.0)
        } else if narrow {
            (r.x + 56.0, r.y + 10.0, r.w - 66.0)
        } else {
            (r.x + 56.0, r.y + 25.0, r.w - 150.0)
        };
        cv.text(title, Rect::new(tx, ty, tw, 20.0), &f.title, txt());
        cv.text(artist, Rect::new(tx, ty + 18.0, tw, 14.0), &f.small, dim());

        // Кнопки: назад, играть/пауза (светлая), вперёд. У узкой карточки — отдельным рядом.
        let (cy, xs) = if narrow {
            (r.y + 58.0, [r.cx() - 30.0, r.cx(), r.cx() + 30.0])
        } else {
            (if compact { r.cy() } else { r.y + 40.0 }, [r.right() - 75.0, r.right() - 45.0, r.right() - 16.0])
        };
        if Self::round_button(ui, cv, id(CONTROLS, 20), xs[0], cy, 9.5, icon::PREV, chip_bg(), txt()) {
            bg::send(Cmd::Prev);
        }
        let toggle = if playing { icon::PAUSE } else { icon::PLAY_SOLID };
        if Self::round_button(ui, cv, id(CONTROLS, 21), xs[1], cy, 14.0, toggle, cv.accent, ink()) {
            bg::send(Cmd::PlayPause);
        }
        if Self::round_button(ui, cv, id(CONTROLS, 22), xs[2], cy, 9.5, icon::NEXT, chip_bg(), txt()) {
            bg::send(Cmd::Next);
        }

        // Прогресс и время.
        let frac = if dur > 0.0 { pos / dur } else { 0.0 };
        if compact {
            cv.progress(Rect::new(r.x + 10.0, r.bottom() - 7.0, r.w - 20.0, 2.0), frac, cv.accent);
            return;
        }
        let py = r.y + 76.0;
        cv.text(&mmss(pos), Rect::new(r.x + 9.0, py - 6.0, 30.0, 12.0), &f.small, dim());
        cv.text(&mmss(dur), Rect::new(r.right() - 46.0, py - 6.0, 36.0, 12.0), &f.small_r, dim());
        cv.progress(Rect::new(r.x + 37.0, py - 1.0, r.w - 37.0 - 36.0, 2.0), frac, cv.accent);
    }
}
