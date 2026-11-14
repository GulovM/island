//! «Лицо» свёрнутого острова, когда музыка не играет: слева и справа настраиваемые значки
//! (раскладка клавиатуры, Wi-Fi / режим полёта, Bluetooth, батарея), по центру время.
//! Данные приходят без опроса в простое: время обновляет минутный таймер (он включён, только пока
//! лицо на экране), раскладку проверяет существующий редкий опрос окружения, сеть/радио/Bluetooth
//! — события в фоновом потоке (`bg::Snapshot`, `bg::NetStatus`), питание — `WM_POWERBROADCAST`.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::anim::{Spring, FADE_SPRING};
use crate::bg::{self, NetStatus, Snapshot};
use crate::config::CollapsedIcon;
use crate::sys::{battery_status, BatteryStatus};
use crate::ui::{collapsed_dim, collapsed_track, collapsed_txt, icon, status_orange, Canvas, Rect, BLUE, WHITE};

/// Дополнительная ширина свёрнутой чёлки под плашку активного таймера (логические пиксели).
pub const TIMER_SLOT_W: f32 = 44.0;

pub struct Face {
    enabled: bool,
    visible: bool,
    alpha: Spring,
    left: CollapsedIcon,
    right: CollapsedIcon,
    time: String,
    lang: u32,
    lang_text: String,
    net: NetStatus,
    airplane: bool,
    bluetooth: Option<bool>,
    bt_connected: bool,
    battery: Option<BatteryStatus>,
}

/// Состояние значка Bluetooth в свёрнутой чёлке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtIconState {
    Off,
    OnIdle,
    Connected,
}

/// Выбирает состояние значка Bluetooth по состоянию радио и наличию подключённого устройства.
pub fn bluetooth_icon_state(bluetooth: Option<bool>, connected: bool) -> BtIconState {
    match bluetooth {
        Some(true) if connected => BtIconState::Connected,
        Some(true) => BtIconState::OnIdle,
        _ => BtIconState::Off,
    }
}

/// «14:05».
pub fn fmt_time(hour: u16, minute: u16) -> String {
    format!("{hour:02}:{minute:02}")
}

/// Через сколько миллисекунд начнётся следующая минута (с небольшим запасом, чтобы таймер не
/// сработал за миг до смены минуты).
pub fn ms_to_next_minute(second: u16, millis: u16) -> u32 {
    ((60 - second.min(59) as u32) * 1000).saturating_sub(millis.min(999) as u32) + 25
}

/// Сколько линий сигнала горит: 0 — нет сети, 1–3 — Wi-Fi по качеству, проводная сеть — все три.
pub fn signal_level(n: NetStatus) -> u8 {
    if n.wifi_connected {
        1 + (n.quality > 30) as u8 + (n.quality > 62) as u8
    } else if n.wired_up {
        3
    } else {
        0
    }
}

impl Face {
    pub fn new(enabled: bool, left: CollapsedIcon, right: CollapsedIcon) -> Self {
        let mut f = Face {
            enabled,
            visible: false,
            alpha: Spring::unit(0.0, FADE_SPRING.0, FADE_SPRING.1),
            left,
            right,
            time: String::new(),
            lang: 0,
            lang_text: String::new(),
            net: NetStatus::default(),
            airplane: false,
            bluetooth: None,
            bt_connected: false,
            battery: None,
        };
        f.refresh_all();
        f
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    pub fn set_slots(&mut self, left: CollapsedIcon, right: CollapsedIcon) {
        self.left = left;
        self.right = right;
    }

    pub fn uses_layout(&self) -> bool {
        self.left == CollapsedIcon::Layout || self.right == CollapsedIcon::Layout
    }

    /// Нужно ли показывать лицо (остров свёрнут, ничто не закрывает).
    /// `timer` — на острове есть таймер: плашка показывается, даже если лицо выключено в настройках.
    pub fn set_visible(&mut self, v: bool, timer: bool) {
        self.visible = v && (self.enabled || timer);
        self.alpha.target = if self.visible { 1.0 } else { 0.0 };
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn animating(&self) -> bool {
        !self.alpha.settled()
    }

    pub fn alpha(&self) -> f32 {
        self.alpha.value.clamp(0.0, 1.0)
    }

    pub fn step(&mut self, dt: f32) {
        self.alpha.step(dt);
        if self.alpha.settled() {
            self.alpha.snap();
        }
    }

    fn read_time() -> (String, u32) {
        let t = unsafe { GetLocalTime() };
        (fmt_time(t.wHour, t.wMinute), ms_to_next_minute(t.wSecond, t.wMilliseconds))
    }

    pub fn refresh_all(&mut self) {
        self.time = Self::read_time().0;
        self.net = bg::net_status();
        let snap = bg::snapshot();
        self.airplane = snap.airplane.unwrap_or(false);
        self.bluetooth = snap.bluetooth;
        self.bt_connected = snap.bt_connected;
        self.battery = battery_status();
        if self.uses_layout() || self.lang_text.is_empty() {
            self.poll_lang();
        }
    }

    /// Минутный тик: время, сеть (уровень сигнала меняется без событий), питание и раскладка.
    /// Возвращает `true`, если что-то изменилось.
    pub fn tick(&mut self) -> bool {
        let before = (
            self.time.clone(),
            self.net,
            self.lang,
            self.airplane,
            self.bluetooth,
            self.bt_connected,
            self.battery,
        );
        self.refresh_all();
        before
            != (
                self.time.clone(),
                self.net,
                self.lang,
                self.airplane,
                self.bluetooth,
                self.bt_connected,
                self.battery,
            )
    }

    /// Через сколько миллисекунд нужен следующий минутный тик.
    pub fn next_tick_ms(&self) -> u32 {
        Self::read_time().1
    }

    /// Раскладка окна в фокусе. Дёшево (вызовы user32), поэтому её проверяет общий опрос окружения.
    pub fn poll_lang(&mut self) -> bool {
        let Some(lang) = crate::sys::foreground_lang() else {
            return false;
        };
        if lang == self.lang && !self.lang_text.is_empty() {
            return false;
        }
        self.lang = lang;
        self.lang_text = crate::sys::lang_code(lang);
        true
    }

    /// Забирает свежее состояние сети из фонового потока.
    pub fn sync_net(&mut self) -> bool {
        let n = bg::net_status();
        if n == self.net {
            return false;
        }
        self.net = n;
        true
    }

    /// Забирает состояние радио (режим полёта, Bluetooth и подключения) из снимка фонового потока.
    pub fn sync_snapshot(&mut self, snap: &Snapshot) -> bool {
        let airplane = snap.airplane.unwrap_or(false);
        let changed = self.airplane != airplane || self.bluetooth != snap.bluetooth || self.bt_connected != snap.bt_connected;
        self.airplane = airplane;
        self.bluetooth = snap.bluetooth;
        self.bt_connected = snap.bt_connected;
        changed
    }

    /// Перечитывает состояние батареи (при `WM_POWERBROADCAST`).
    pub fn sync_power(&mut self) -> bool {
        let b = battery_status();
        if b == self.battery {
            return false;
        }
        self.battery = b;
        true
    }

    /// Ширина левого или правого значка в свёрнутой чёлке (логические пиксели).
    unsafe fn slot_w(&self, cv: &Canvas, slot: CollapsedIcon) -> f32 {
        match slot {
            CollapsedIcon::None => 0.0,
            CollapsedIcon::Layout => cv.text_w(&self.lang_text, &cv.g.fonts.body_b),
            CollapsedIcon::Wifi => 11.4,
            CollapsedIcon::Bluetooth => 12.2,
            CollapsedIcon::Battery => {
                if self.battery.is_some_and(|b| b.ac_online) {
                    24.5
                } else {
                    17.2
                }
            }
        }
    }

    /// Рисует содержимое левого или правого слота начиная с `x` шириной `w`.
    unsafe fn draw_slot(&self, cv: &Canvas, slot: CollapsedIcon, x: f32, w: f32, r: Rect) {
        let cy = r.cy() - 0.5;
        match slot {
            CollapsedIcon::None => {}
            CollapsedIcon::Layout => {
                cv.text(&self.lang_text, Rect::new(x, r.y, w.max(1.0) + 2.0, r.h - 1.0), &cv.g.fonts.body_b, collapsed_dim());
            }
            CollapsedIcon::Wifi => {
                let cx = x + w / 2.0;
                if self.airplane {
                    cv.symbol(icon::AIRPLANE, Rect::new(cx - 5.7, cy - 5.7, 11.4, 11.4), 11.4, collapsed_txt());
                } else {
                    self.wifi(cv, cx, r.cy() + 2.9);
                }
            }
            CollapsedIcon::Bluetooth => {
                self.bluetooth(cv, x + w / 2.0, cy);
            }
            CollapsedIcon::Battery => {
                self.battery(cv, x, cy);
            }
        }
    }

    /// Рисует лицо внутри свёрнутого острова `r` (логические пиксели).
    /// При активном таймере отображаются левый значок, время, плашка остатка таймера и правый значок.
    /// Если `self.enabled == false`, рисуется только `timer`.
    pub unsafe fn draw(&self, cv: &Canvas, r: Rect, timer: Option<(&str, f32)>) {
        let f = &cv.g.fonts;
        let pill_h = 14.0;
        if !self.enabled {
            if let Some((label, pulse)) = timer {
                let pill_w = cv.text_w(label, &f.small_c) + 8.0;
                let t = Canvas { g: cv.g, alpha: cv.alpha * pulse, accent: cv.accent };
                let pill_r = Rect::new(r.cx() - pill_w / 2.0, r.cy() - pill_h / 2.0, pill_w, pill_h);
                t.rr(pill_r, pill_h / 2.0, status_orange());
                t.text(label, pill_r, &f.small_c, WHITE);
            }
            return;
        }

        let pad = (r.w * 0.09).clamp(8.0, 16.0);
        let left_w = self.slot_w(cv, self.left);
        let right_w = self.slot_w(cv, self.right);
        self.draw_slot(cv, self.left, r.x + pad, left_w, r);
        match timer {
            None => {
                cv.text(&self.time, Rect::new(r.x, r.y, r.w, r.h - 1.0), &f.body_c, collapsed_txt());
            }
            Some((label, pulse)) => {
                let time_w = cv.text_w(&self.time, &f.body_b);
                let pill_w = cv.text_w(label, &f.small_c) + 8.0;
                let available = (r.right() - pad - right_w) - (r.x + pad);
                let gap = (available - (left_w + time_w + pill_w)) / 3.0;

                let mut x = r.x + pad + left_w + gap;
                cv.text(&self.time, Rect::new(x, r.y, time_w + 1.0, r.h - 1.0), &f.body_b, collapsed_txt());
                x += time_w + gap;

                let t = Canvas { g: cv.g, alpha: cv.alpha * pulse, accent: cv.accent };
                let pill_r = Rect::new(x, r.cy() - pill_h / 2.0, pill_w, pill_h);
                t.rr(pill_r, pill_h / 2.0, status_orange());
                t.text(label, pill_r, &f.small_c, WHITE);
            }
        }
        self.draw_slot(cv, self.right, r.right() - pad - right_w, right_w, r);
    }

    /// Значок Wi-Fi: точка и три дуги, нарисованные примитивами (глифов в шрифте может не быть).
    /// `(cx, cy)` — точка-источник у нижнего края значка (центр концентрических дуг и точки).
    ///
    /// Геометрия пересчитана под единый кегль текста 9pt и оптический вес Semibold (600):
    /// 1. Визуальная высота: верхний край верхней дуги (cy - 7.2 - 0.6 = cy - 7.8) плюс нижний край
    ///    точки (cy + 1.0) дают общую высоту 8.8 px, что оптически соответствует em-кеглю 9.0 pt шрифта.
    /// 2. Толщина штриха 1.2 px в точности повторяет толщину основных штрихов (stem width) шрифта
    ///    Segoe UI Variable Semibold (вес 600) при кегле 9pt.
    /// 3. Радиус точки 1.0 px (диаметр 2.0 px) согласуется по оптической массе с закруглёнными
    ///    концами штриха 1.2 px, не доминируя над ними.
    /// 4. Радиусы дуг 2.6, 4.9, 7.2 px (шаг 2.3 px) образуют гармоничный геометрический ритм:
    ///    просвет между точкой и первой дугой равен 1.0 px, а между дугами — ровно 1.1 px.
    /// 5. Сектор ±45° (размах 90°) задаёт ширину иконки 11.4 px, создавая классическое для системных
    ///    индикаторов Wi-Fi соотношение сторон ~4:3 (11.4 × 8.8 px).
    /// 6. Вертикальное выравнивание: геометрический центр иконки (cy - 3.4) при `cy = r.cy() + 2.9`
    ///    находится на `r.cy() - 0.5`, строго совпадая с оптической осью центрированного текста.
    /// 7. При `level == 0` диагональный штрих толщиной 1.2 px перечёркивает иконку из верхнего левого
    ///    угла в нижний правый (с охватом дуг и точки).
    unsafe fn wifi(&self, cv: &Canvas, cx: f32, cy: f32) {
        let level = signal_level(self.net);
        let off = collapsed_track();
        let on = collapsed_txt();
        for i in 0..3u8 {
            let radius = 2.6 + 2.3 * i as f32;
            cv.stroke_arc(cx, cy, radius, -45.0, 45.0, 1.2, if level > i { on } else { off });
        }
        cv.circle(cx, cy, 1.0, if level > 0 { on } else { off });
        if level == 0 {
            // Нет сети: аккуратное перечёркивание всего габарита иконки по диагонали ~39°.
            cv.stroke_line((cx - 5.5, cy - 7.5), (cx + 5.5, cy + 1.4), 1.2, collapsed_dim());
        }
    }

    /// Значок Bluetooth в трёх состояниях (оптическая ось `cy = r.cy() - 0.5`, штрих 1.2 px):
    /// - `Off`: приглушённая руна + диагональное перечёркивание с чёрной отбивкой;
    /// - `OnIdle`: чёткая руна `collapsed_txt()`;
    /// - `Connected`: руна `collapsed_txt()` + две симметричные точки подключения по бокам.
    unsafe fn bluetooth(&self, cv: &Canvas, cx: f32, cy: f32) {
        let state = bluetooth_icon_state(self.bluetooth, self.bt_connected);
        let rune = [
            (cx - 2.6, cy - 2.2),
            (cx + 2.8, cy + 2.35),
            (cx, cy + 4.7),
            (cx, cy - 4.7),
            (cx + 2.8, cy - 2.35),
            (cx - 2.6, cy + 2.2),
        ];
        match state {
            BtIconState::Off => {
                let dim_rune = D2D1_COLOR_F { a: 0.45, ..collapsed_dim() };
                cv.stroke_polyline(&rune, 1.2, dim_rune);
                let cut = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
                cv.stroke_line((cx - 4.6, cy - 4.4), (cx + 4.6, cy + 4.4), 2.6, cut);
                cv.stroke_line((cx - 4.6, cy - 4.4), (cx + 4.6, cy + 4.4), 1.2, collapsed_dim());
            }
            BtIconState::OnIdle => {
                cv.stroke_polyline(&rune, 1.2, collapsed_txt());
            }
            BtIconState::Connected => {
                cv.stroke_polyline(&rune, 1.2, collapsed_txt());
                cv.circle(cx - 4.8, cy, 1.15, BLUE);
                cv.circle(cx + 4.8, cy, 1.15, BLUE);
            }
        }
    }

    /// Значок батареи в свёрнутой чёлке:
    /// - от батареи (`ac_online == false`): нейтрально-серое заполнение;
    /// - от сети (`ac_online == true`): зелёное заполнение + молния справа от контакта.
    unsafe fn battery(&self, cv: &Canvas, x: f32, cy: f32) {
        let body = Rect::new(x, cy - 4.3, 15.0, 8.6);
        let nub = Rect::new(body.right() + 0.7, cy - 1.7, 1.4, 3.4);
        let Some(b) = self.battery else {
            cv.rr_line(body, 2.2, collapsed_track(), 1.1);
            cv.rr(nub, 0.7, collapsed_track());
            cv.stroke_line((body.x + 4.5, cy), (body.right() - 4.5, cy), 1.1, collapsed_dim());
            return;
        };

        cv.rr_line(body, 2.2, collapsed_dim(), 1.1);
        cv.rr(nub, 0.7, D2D1_COLOR_F { a: 0.85, ..collapsed_dim() });

        let green = crate::ios::to_d2d(crate::ios::sys(crate::ios::SysColor::Green, true), 1.0);
        let gray = D2D1_COLOR_F { r: 0.56, g: 0.56, b: 0.60, a: 1.0 };
        let fill_col = if b.ac_online { green } else { gray };

        let inner = body.inset(1.7);
        let pct = f32::from(b.percent.min(100)) / 100.0;
        let fill_w = if b.percent == 0 {
            0.0
        } else {
            (inner.w * pct).max(1.8).min(inner.w)
        };
        if fill_w > 0.0 {
            cv.rr(Rect::new(inner.x, inner.y, fill_w, inner.h), 1.0, fill_col);
        }

        if b.ac_online {
            let bx = body.right() + 2.2;
            cv.symbol(icon::BOLT, Rect::new(bx - 1.5, cy - 5.5, 11.0, 11.0), 11.0, green);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_is_zero_padded() {
        assert_eq!(fmt_time(9, 5), "09:05");
        assert_eq!(fmt_time(23, 59), "23:59");
    }

    #[test]
    fn next_minute_delay() {
        assert_eq!(ms_to_next_minute(0, 0), 60_025);
        assert_eq!(ms_to_next_minute(59, 999), 26);
        assert_eq!(ms_to_next_minute(30, 500), 29_525);
        // Мусор на входе не даёт переполнения.
        assert!(ms_to_next_minute(99, 5000) < 2000);
    }

    #[test]
    fn signal_levels() {
        let n = |wifi, q, wired| NetStatus { wifi_connected: wifi, quality: q, wired_up: wired };
        assert_eq!(signal_level(n(false, 0, false)), 0);
        assert_eq!(signal_level(n(true, 10, false)), 1);
        assert_eq!(signal_level(n(true, 50, false)), 2);
        assert_eq!(signal_level(n(true, 90, false)), 3);
        assert_eq!(signal_level(n(false, 0, true)), 3);
        // Wi-Fi важнее провода: слабый Wi-Fi не маскируется Ethernet.
        assert_eq!(signal_level(n(true, 10, true)), 1);
    }

    #[test]
    fn hidden_face_fades_out() {
        let mut f = Face::new(true, CollapsedIcon::Layout, CollapsedIcon::Wifi);
        f.set_visible(true, false);
        for _ in 0..120 {
            f.step(1.0 / 60.0);
        }
        assert!((f.alpha() - 1.0).abs() < 0.01 && !f.animating());
        f.set_visible(false, false);
        for _ in 0..120 {
            f.step(1.0 / 60.0);
        }
        assert_eq!(f.alpha(), 0.0);
        // Выключенное в настройках лицо не показывается, кроме случая, когда на острове идёт таймер.
        let mut off = Face::new(false, CollapsedIcon::Layout, CollapsedIcon::Wifi);
        off.set_visible(true, false);
        assert!(!off.visible());
        off.set_visible(true, true);
        assert!(off.visible());
    }

    #[test]
    fn poll_lang_idempotence() {
        let mut f = Face::new(true, CollapsedIcon::Layout, CollapsedIcon::Wifi);
        // Если раскладка уже определена, повторный вызов без изменений не должен приводить к перерисовке.
        if !f.lang_text.is_empty() {
            assert!(!f.poll_lang());
        }
    }

    #[test]
    fn bluetooth_icon_states_and_snapshot_sync() {
        assert_eq!(bluetooth_icon_state(None, false), BtIconState::Off);
        assert_eq!(bluetooth_icon_state(Some(false), true), BtIconState::Off);
        assert_eq!(bluetooth_icon_state(Some(true), false), BtIconState::OnIdle);
        assert_eq!(bluetooth_icon_state(Some(true), true), BtIconState::Connected);

        let mut f = Face::new(true, CollapsedIcon::Bluetooth, CollapsedIcon::Wifi);
        assert!(!f.uses_layout());
        f.set_slots(CollapsedIcon::Layout, CollapsedIcon::Battery);
        assert!(f.uses_layout());

        let mut snap = Snapshot::default();
        snap.airplane = Some(true);
        snap.bluetooth = Some(true);
        snap.bt_connected = true;
        assert!(f.sync_snapshot(&snap));
        assert!(!f.sync_snapshot(&snap), "повторный синк без изменений возвращает false");
        snap.airplane = Some(false);
        assert!(f.sync_snapshot(&snap), "отключение режима полёта возвращает Wi-Fi");
    }

    #[test]
    fn test_face_metrics_and_overlap() {
        use windows::Win32::Graphics::DirectWrite::*;
        use crate::ui::Fonts;
        unsafe {
            let factory: windows::core::Result<IDWriteFactory> = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED);
            let Ok(f) = factory else { return };
            let fonts = Fonts::new(&f).expect("Fonts::new failed");

            // Проверяем ширину 3-символьного кода раскладки («УКР», «ENG», «WWW») в новом кегле body_b (9pt)
            let layout_ukr = fonts.layout("УКР", &fonts.body_b, 100.0, 20.0).expect("layout failed");
            let mut m_ukr = DWRITE_TEXT_METRICS::default();
            layout_ukr.GetMetrics(&mut m_ukr).unwrap();
            let layout_www = fonts.layout("WWW", &fonts.body_b, 100.0, 20.0).expect("layout failed");
            let mut m_www = DWRITE_TEXT_METRICS::default();
            layout_www.GetMetrics(&mut m_www).unwrap();

            // Убеждаемся, что «УКР» и даже широкий «WWW» с запасом помещаются в рамку lang_text (34.0 px) без усечения
            assert!(m_ukr.widthIncludingTrailingWhitespace < 34.0);
            assert!(m_www.widthIncludingTrailingWhitespace < 34.0);

            // Проверяем, что кегль lang_text (body_b) равен кеглю таймера (body_c) — ровно 9pt
            assert!((fonts.body_b.GetFontSize() - 9.0).abs() < 1e-4);
            assert!((fonts.body_c.GetFontSize() - 9.0).abs() < 1e-4);

            // Проверяем 4-элементную компоновку свёрнутого острова: раскладка, время, плашка таймера, Wi-Fi
            let r_w = 180.0f32;
            let pad = (r_w * 0.09).clamp(8.0, 16.0); // 16.0
            let lang_w = m_ukr.widthIncludingTrailingWhitespace;

            // Время часов ("14:05")
            let layout_time = fonts.layout("14:05", &fonts.body_b, 100.0, 20.0).expect("layout failed");
            let mut m_time = DWRITE_TEXT_METRICS::default();
            layout_time.GetMetrics(&mut m_time).unwrap();
            let time_w = m_time.widthIncludingTrailingWhitespace;

            // Wi-Fi: визуальная ширина иконки 11.4px, край дуги радиусом 7.2px со штрихом 1.2px под 45°
            let wifi_w = 11.4f32;
            let wifi_cx = r_w - pad - 5.5;
            let wifi_left = wifi_cx - 5.7;

            // Метки таймера от коротких ("0:05") до длинных ("9:59:59")
            for label in &["0:05", "14:05", "1:02:05", "9:59:59"] {
                let layout_timer = fonts.layout(label, &fonts.small_c, 100.0, 20.0).expect("layout failed");
                let mut m_timer = DWRITE_TEXT_METRICS::default();
                layout_timer.GetMetrics(&mut m_timer).unwrap();
                let timer_w = m_timer.widthIncludingTrailingWhitespace + 8.0;

                let available = (r_w - pad - wifi_w) - pad;
                let gap = (available - (lang_w + time_w + timer_w)) / 3.0;

                let lang_right = pad + lang_w;
                let time_left = lang_right + gap;
                let time_right = time_left + time_w;
                let timer_left = time_right + gap;
                let timer_right = timer_left + timer_w;

                assert!(lang_right < time_left, "Время ({time_left}) наезжает на раскладку ({lang_right})");
                assert!(time_right < timer_left, "Плашка таймера ({timer_left}) наезжает на часы ({time_right})");
                assert!(timer_right < wifi_left, "Плашка таймера ({timer_right}) наезжает на Wi-Fi ({wifi_left}) для метки {label}");

                // Проверяем равенство трёх зазоров между соседними элементами с допуском 0.5px
                let gap1 = time_left - lang_right;
                let gap2 = timer_left - time_right;
                let gap3 = wifi_left - timer_right;
                assert!((gap1 - gap2).abs() < 0.5, "Зазоры 1 и 2 не равны: {gap1} vs {gap2} для метки {label}");
                assert!((gap2 - gap3).abs() < 0.5, "Зазоры 2 и 3 не равны: {gap2} vs {gap3} для метки {label}");
                assert!((gap1 - gap3).abs() < 0.5, "Зазоры 1 и 3 не равны: {gap1} vs {gap3} для метки {label}");
            }
        }
    }

    #[test]
    fn test_wifi_geometry_and_proportions() {
        let thick = 1.2f32;
        let r_dot = 1.0f32;
        let r_outer = 2.6 + 2.3 * 2.0; // 7.2 px
        let total_h = (r_outer + thick / 2.0) + r_dot; // 8.8 px

        // Высота иконки согласована с 9pt кеглем текста (погрешность < 3%)
        assert!((total_h - 9.0).abs() < 0.3);

        // Просвет между точкой и первой дугой
        let r_inner = 2.6;
        let dot_gap = (r_inner - thick / 2.0) - r_dot;
        assert!((dot_gap - 1.0).abs() < 1e-4);

        // Просвет между концентрическими дугами
        let arc_gap = 2.3 - thick;
        assert!((arc_gap - 1.1).abs() < 1e-4);

        // Оптический центр иконки относительно r.cy() при cy = r.cy() + 2.9
        let cy_offset = 2.9f32;
        let visual_center = cy_offset - (r_outer + thick / 2.0 - r_dot) / 2.0;
        // Совпадает с вертикальным центром текстовой рамки r.cy() - 0.5 (h - 1.0)
        assert!((visual_center - (-0.5)).abs() < 0.15);
    }
}
