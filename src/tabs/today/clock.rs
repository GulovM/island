//! Циферблаты часов. Шесть стилей, у каждого своя композиция под каждый размер карточки
//! (`CLOCK_SIZES`: 0 — 2×2 = 104×104, 1 — высокая 2×4 = 104×216, 2 — широкая 4×2 = 216×104):
//!
//! 0. «Часы» — цифры и полоса суток;
//! 1. «Классика» — стрелочный циферблат с цифрами 1–12;
//! 2. «Точки» — цифры из светящихся точек (матрицы 5×7, как табло);
//! 3. «Шкала» — бегущая линейка времени с неподвижной отметкой «сейчас»;
//! 4. «Минимал» — стрелки без цифр;
//! 5. «Небо» — цвет неба по времени суток и путь солнца от восхода до заката.

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::sun::sun_times;
use super::Today;
#[cfg(test)]
use crate::online::civil_from_days;
use crate::online::days_from_civil;
use crate::ui::{card_radius, dim, mix, on_color, readable, track, txt, Align, Canvas, Corner, Rect};

/// Внутренний отступ карточки часов.
const P: f32 = 11.0;

/// Восход и закат (минуты от полуночи), пока у погоды нет координат или в точке полярный день/ночь.
const DEFAULT_SUN: (u16, u16) = (7 * 60, 19 * 60);

/// Вид карточки по индексу размера в `CLOCK_SIZES`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Small,
    Tall,
    Wide,
}

impl Shape {
    fn of(size: usize) -> Self {
        match size {
            0 => Shape::Small,
            1 => Shape::Tall,
            _ => Shape::Wide,
        }
    }
}

/// Номер недели по ISO 8601 (неделя с понедельника, первая — та, где первый четверг года).
#[cfg(test)]
pub(super) fn iso_week(day: i64) -> u32 {
    // 1970-01-01 — четверг: после сдвига 0 — понедельник.
    let dow = (day + 3).rem_euclid(7);
    let thursday = day - dow + 3;
    let (y, _, _) = civil_from_days(thursday);
    ((thursday - days_from_civil(y, 1, 1)) / 7 + 1) as u32
}

/// Углы стрелок в градусах от «12» по часовой: часовая, минутная, секундная.
fn hand_angles(h: u16, m: u16, s: u16) -> (f32, f32, f32) {
    let (h, m, s) = (f32::from(h % 12), f32::from(m), f32::from(s.min(59)));
    ((h + m / 60.0) * 30.0, (m + s / 60.0) * 6.0, s * 6.0)
}

/// Подложка циферблата: белая на светлом острове, затемнение на тёмном.
fn face() -> D2D1_COLOR_F {
    let bg = crate::theme::get().bg;
    if crate::theme::is_light([bg.r, bg.g, bg.b]) {
        D2D1_COLOR_F { a: 0.92, ..crate::ui::WHITE }
    } else {
        D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.42 }
    }
}

/// Тот же цвет, но прозрачнее в `a` раз.
fn fade(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: c.a * a, ..c }
}

const fn rgb(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}

/// Часть суток по часу: 0 — утро, 1 — день, 2 — вечер, 3 — ночь.
fn day_part(h: u16) -> usize {
    match h {
        5..=11 => 0,
        12..=16 => 1,
        17..=21 => 2,
        _ => 3,
    }
}

/// «утра / дня / вечера / ночи» — после времени («9:41 утра»).
const PART_GEN: [(&str, &str); 4] = [("утра", "morning"), ("дня", "afternoon"), ("вечера", "evening"), ("ночи", "night")];
/// «утро / день / вечер / ночь» — сама по себе.
const PART_NOM: [(&str, &str); 4] = [("утро", "morning"), ("день", "afternoon"), ("вечер", "evening"), ("ночь", "night")];

/// Часы «00»…«23» для подписей линейки: без форматирования на кадр.
const HOURS: [&str; 24] = [
    "00", "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23",
];
const NUMS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"];

/// Шрифт цифр из точек: 10 цифр × 7 рядов, в ряду 5 бит (старший — левая колонка).
/// Ноль перечёркнут диагональю, чтобы не путался с буквой О.
const DOT_FONT: [[u8; 7]; 10] = [
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
    [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
];

// Небо: ночь, день и тёплая полоса рассвета/заката (верх и низ градиента).
const NIGHT: (D2D1_COLOR_F, D2D1_COLOR_F) = (rgb(0.04, 0.06, 0.18), rgb(0.10, 0.12, 0.30));
const DAY: (D2D1_COLOR_F, D2D1_COLOR_F) = (rgb(0.184, 0.427, 0.710), rgb(0.553, 0.714, 0.910));
const DAWN: (D2D1_COLOR_F, D2D1_COLOR_F) = (rgb(0.27, 0.30, 0.62), rgb(0.98, 0.58, 0.40));
const SUN_COLOR: D2D1_COLOR_F = rgb(1.0, 0.82, 0.25);

/// Подписи даты для карточки.
struct Dates {
    /// «ЧЕТВЕРГ».
    weekday: String,
    /// «ЧЕТВЕРГ,».
    weekday_comma: String,
    /// «ЧЕТВЕРГ, 13 НОЯБРЯ».
    full: String,
    /// «ЧТ, 13 НОЯБРЯ».
    short_full: String,
    /// «ЧТ, 13 НОЯ».
    short: String,
    /// «Чт, 13 ноя».
    brief: String,
    /// «Чт 13».
    tiny: String,
    /// «13 ноября».
    long: String,
    /// «13 НОЯБРЯ».
    long_up: String,
}

impl Dates {
    fn new(t: &SYSTEMTIME, dow: usize) -> Self {
        let month = crate::i18n::months_short()[(t.wMonth as usize).clamp(1, 12) - 1];
        let wd = crate::i18n::weekdays()[dow];
        let weekday = crate::i18n::weekdays_full()[dow].to_uppercase();
        let long = crate::i18n::day_month(t.wDay as u32, t.wMonth as u32);
        let long_up = long.to_uppercase();
        Dates {
            weekday_comma: format!("{weekday},"),
            full: format!("{weekday}, {long_up}"),
            short_full: format!("{wd}, {long_up}").to_uppercase(),
            short: format!("{wd}, {} {month}", t.wDay).to_uppercase(),
            brief: format!("{wd}, {} {month}", t.wDay),
            tiny: format!("{wd} {}", t.wDay),
            weekday,
            long,
            long_up,
        }
    }
}

/// Подпись капсом 9.5 px semibold с трекингом (как в макете).
unsafe fn cap(cv: &Canvas, s: &str, x: f32, w: f32, cy: f32, align: Align, c: D2D1_COLOR_F) -> f32 {
    cv.line(s, &cv.g.fonts.caps_at(9.5, 620, 0.07), x, w, cy, align, c)
}

/// Мелкая подпись любого кегля и веса без трекинга.
#[allow(clippy::too_many_arguments)]
unsafe fn small(cv: &Canvas, s: &str, size: f32, weight: u32, x: f32, w: f32, cy: f32, align: Align, c: D2D1_COLOR_F) -> f32 {
    cv.line(s, &cv.g.fonts.caps_at(size, weight, 0.0), x, w, cy, align, c)
}

/// Город справа сверху (10 px, dim) шириной не больше трети полосы; возвращает занятую ширину с зазором.
unsafe fn city_label(cv: &Canvas, c: Rect, city: &str, cy: f32) -> f32 {
    if city.is_empty() {
        return 0.0;
    }
    let f = cv.g.fonts.caps_at(10.0, 500, 0.0);
    let w = (cv.text_w(city, &f) + 2.0).min((c.w - 2.0 * P) / 3.0);
    cv.line(city, &f, c.right() - P - w, w, cy, Align::Trailing, dim());
    w + 6.0
}

/// «09:41» тремя кусками: цифры цветом `ink`, двоеточие — `colon`. `x` — левый край либо
/// середина (`center`). `cy` — середина цифр (см. `Canvas::digits`).
#[allow(clippy::too_many_arguments)]
unsafe fn time_tone(cv: &Canvas, hh: &str, mm: &str, x: f32, cy: f32, size: f32, weight: u32, center: bool, ink: D2D1_COLOR_F, colon: D2D1_COLOR_F) {
    let (wh, wc, wm) = (cv.digits_w(hh, size, weight), cv.digits_w(":", size, weight), cv.digits_w(mm, size, weight));
    let x0 = if center { x - (wh + wc + wm) / 2.0 } else { x };
    cv.digits(hh, x0, wh + 2.0, cy, size, weight, Align::Leading, ink);
    cv.digits(":", x0 + wh, wc + 2.0, cy - 0.04 * size, size, weight, Align::Leading, colon);
    cv.digits(mm, x0 + wh + wc, wm + 2.0, cy, size, weight, Align::Leading, ink);
}

/// Аналоговый циферблат радиуса `r` с центром (`cx`, `cy`): «Классика» (`classic`: подложка, цифры,
/// акцентные метки) или «Минимал» (только метки и стрелки).
unsafe fn analog(cv: &Canvas, cx: f32, cy: f32, r: f32, t: &SYSTEMTIME, classic: bool) {
    let acc = cv.accent;
    let at = |deg: f32, k: f32| {
        let a = deg.to_radians();
        (cx + r * k * a.sin(), cy - r * k * a.cos())
    };
    if classic {
        cv.circle(cx, cy, r, face());
        if crate::theme::glass() {
            cv.rr_line(Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r, crate::ui::hairline(), 1.0);
        }
        cv.rr_line(Rect::new(cx - r + 1.0, cy - r + 1.0, 2.0 * r - 2.0, 2.0 * r - 2.0), r - 1.0, fade(acc, 0.7), 1.0);
        for (i, n) in NUMS.iter().enumerate() {
            let (x, y) = at((i + 1) as f32 * 30.0, 0.70);
            cv.digits(n, x - 8.0, 16.0, y, 8.0, 500, Align::Center, fade(txt(), 0.9));
        }
        for i in 0..60 {
            let deg = i as f32 * 6.0;
            if i % 5 == 0 {
                let long = i % 15 == 0;
                cv.stroke_line(at(deg, if long { 0.82 } else { 0.86 }), at(deg, 0.94), if long { 1.6 } else { 1.2 }, acc);
            } else {
                cv.stroke_line(at(deg, 0.91), at(deg, 0.94), 0.6, fade(dim(), 0.7));
            }
        }
    } else {
        for h in 0..12 {
            let deg = h as f32 * 30.0;
            if h % 3 == 0 {
                cv.stroke_line(at(deg, 0.78), at(deg, 0.98), 1.6, txt());
            } else {
                let (x, y) = at(deg, 0.90);
                cv.circle(x, y, 1.0, dim());
            }
        }
    }
    let (ha, ma, sa) = hand_angles(t.wHour, t.wMinute, t.wSecond);
    cv.stroke_line(at(ha + 180.0, 0.08), at(ha, 0.50), 2.4, txt());
    cv.stroke_line(at(ma + 180.0, 0.08), at(ma, 0.78), 1.8, txt());
    cv.stroke_line(at(sa + 180.0, 0.18), at(sa, 0.86), 1.0, acc);
    if classic {
        cv.circle(cx, cy, 2.5, acc);
    } else {
        let (x, y) = at(sa, 0.86);
        cv.circle(x, y, 2.2, acc);
        cv.circle(cx, cy, 2.0, acc);
    }
}

/// Цифра `d` (0–9) из точек: левый верх матрицы — (`x`, `y`), шаг `pitch`, радиус точки `rad`.
/// Незажжённые точки рисуются цветом `track()`: видна вся сетка.
unsafe fn dot_digit(cv: &Canvas, d: u16, x: f32, y: f32, pitch: f32, rad: f32, on: D2D1_COLOR_F) {
    let off = track();
    for (row, bits) in DOT_FONT[(d % 10) as usize].iter().enumerate() {
        for col in 0..5 {
            let lit = bits >> (4 - col) & 1 == 1;
            cv.circle(x + (col as f32 + 0.5) * pitch, y + (row as f32 + 0.5) * pitch, rad, if lit { on } else { off });
        }
    }
}

/// Две цифры числа `v` из точек (5 + 1 + 5 колонок).
unsafe fn dot_pair(cv: &Canvas, v: u16, x: f32, y: f32, pitch: f32, rad: f32, on: D2D1_COLOR_F) {
    dot_digit(cv, v / 10, x, y, pitch, rad, on);
    dot_digit(cv, v % 10, x + 6.0 * pitch, y, pitch, rad, on);
}

/// Бегущая линейка: деления каждые 5 минут, `now` (минуты суток) стоит неподвижно в середине
/// `area` (по горизонтали или вертикали), всё остальное сдвигается. Деления растут от `base`
/// (y для горизонтальной, x для вертикальной) в сторону уменьшения координаты; к краям
/// линейки штрихи плавно гаснут. 60 px на час.
unsafe fn ruler(cv: &Canvas, area: Rect, vertical: bool, now: f32, base: f32) {
    const PX_PER_MIN: f32 = 1.0;
    let (mid, half) = if vertical { (area.cy(), area.h / 2.0) } else { (area.cx(), area.w / 2.0) };
    let span = half / PX_PER_MIN;
    let font = cv.g.fonts.caps_at(9.0, 500, 0.0);
    let (first, last) = (((now - span) / 5.0).floor() as i32, ((now + span) / 5.0).ceil() as i32);
    for i in first..=last {
        let minute = i as f32 * 5.0;
        let pos = mid + (minute - now) * PX_PER_MIN;
        let a = ((half - (pos - mid).abs()) / 14.0).clamp(0.0, 1.0);
        if a <= 0.02 {
            continue;
        }
        let m5 = (i * 5).rem_euclid(60);
        let (len, w, c) = match m5 {
            0 => (14.0, 1.4, dim()),
            30 => (10.0, 1.0, dim()),
            15 | 45 => (7.0, 1.0, fade(dim(), 0.75)),
            _ => (5.0, 1.0, track()),
        };
        let c = fade(c, a);
        if vertical {
            cv.stroke_line((base, pos), (base - len, pos), w, c);
        } else {
            cv.stroke_line((pos, base), (pos, base - len), w, c);
        }
        // Подпись часа не лезет под отметку «сейчас».
        if m5 == 0 && (pos - mid).abs() > 9.0 {
            let label = HOURS[(i * 5 / 60).rem_euclid(24) as usize];
            if vertical {
                cv.line(label, &font, base - 14.0 - 4.0 - 16.0, 16.0, pos, Align::Trailing, fade(dim(), a));
            } else {
                cv.line(label, &font, pos - 8.0, 16.0, base + 8.0, Align::Center, fade(dim(), a));
            }
        }
    }
}

/// Тёмная плашка с данными у нижнего края карточки высотой `h`: верх прямой (там горизонт),
/// низ скруглён по карточке.
unsafe fn plate(cv: &Canvas, c: Rect, h: f32) {
    use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE;
    cv.g.dc.PushAxisAlignedClip(&Rect::new(c.x, c.bottom() - h, c.w, h).d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    cv.rr(c, card_radius(), D2D1_COLOR_F { r: 0.03, g: 0.06, b: 0.13, a: 0.80 });
    cv.g.dc.PopAxisAlignedClip();
}

/// Положение Солнца относительно суток: идёт ли день, доля пути (по дню либо по ночи) и
/// сколько минут до ближайшего восхода/заката. `now`, `sr`, `ss` — минуты от полуночи.
fn sun_state(now: i32, sr: i32, mut ss: i32) -> (bool, f32, i32) {
    if ss <= sr {
        // Закат после полуночи (далёкий часовой пояс): считаем его «завтрашним».
        ss += 1440;
    }
    let mut n = now;
    if n < sr && n + 1440 < ss {
        n += 1440;
    }
    if n >= sr && n < ss {
        (true, (n - sr) as f32 / (ss - sr) as f32, ss - n)
    } else if n >= ss {
        (false, (n - ss) as f32 / (sr + 1440 - ss) as f32, sr + 1440 - n)
    } else {
        let prev = ss - 1440;
        (false, (n - prev) as f32 / (sr - prev) as f32, sr - n)
    }
}

/// Цвета неба (верх, низ) и «дневность» 0..1: ночь → час до восхода … час после → день → то же у заката.
fn sky(now: i32, sr: i32, ss: i32) -> (D2D1_COLOR_F, D2D1_COLOR_F) {
    let ss = if ss <= sr { ss + 1440 } else { ss };
    let n = if now < sr && now + 1440 < ss { now + 1440 } else { now };
    let rise = (n - (sr - 60)) as f32 / 120.0;
    let set = ((ss + 60) - n) as f32 / 120.0;
    let u = rise.min(set).clamp(0.0, 1.0);
    // Тёплая полоса — пик ровно на восходе/закате.
    let warm = (u * std::f32::consts::PI).sin();
    let k = u * u * (3.0 - 2.0 * u);
    let top = mix(mix(NIGHT.0, DAY.0, k), DAWN.0, warm * 0.35);
    let bottom = mix(mix(NIGHT.1, DAY.1, k), DAWN.1, warm * 0.9);
    (top, bottom)
}

/// «7:50» из минут от полуночи.
fn hm(m: i32) -> String {
    format!("{}:{:02}", m.div_euclid(60), m.rem_euclid(60))
}

/// Всё, что нужно стилям для одной отрисовки.
struct Face<'a> {
    c: Rect,
    shape: Shape,
    t: &'a SYSTEMTIME,
    d: Dates,
    hh: String,
    mm: String,
    city: &'a str,
    /// Восход и закат, минуты от местной полуночи.
    sun: (u16, u16),
    /// Восход и закат взяты по умолчанию (нет координат города): время на экране — со знаком «~».
    sun_est: bool,
}

impl Face<'_> {
    /// Минуты от полуночи.
    fn minutes(&self) -> u32 {
        u32::from(self.t.wHour) * 60 + u32::from(self.t.wMinute)
    }

    /// Доля суток, 0..1.
    fn day_frac(&self) -> f32 {
        self.minutes() as f32 / 1440.0
    }

    /// Цветной акцент для мелкого текста (на светлой теме подтянут к контрасту).
    fn acc_text(cv: &Canvas) -> D2D1_COLOR_F {
        readable(cv.accent, 60.0)
    }

    /// 0 «Часы»: цифры и полоса суток.
    unsafe fn clock_digits(&self, cv: &Canvas) {
        let (c, d) = (self.c, &self.d);
        let (acc, acc_t) = (cv.accent, Self::acc_text(cv));
        let (x, iw) = (c.x + P, c.w - 2.0 * P);
        cv.glow(c, card_radius(), acc, Corner::TopRight);
        match self.shape {
            Shape::Small => {
                cap(cv, &d.weekday, x, iw, c.y + 14.0, Align::Leading, acc_t);
                time_tone(cv, &self.hh, &self.mm, c.cx(), c.y + 44.0, 30.0, 650, true, txt(), acc);
                small(cv, &d.long, 11.0, 500, x, iw, c.y + 67.0, Align::Center, dim());
                cv.progress(Rect::new(x, c.bottom() - 15.0, iw, 4.0), self.day_frac(), acc);
            }
            Shape::Tall => {
                cap(cv, &d.weekday, x, iw, c.y + 13.0, Align::Leading, acc_t);
                small(cv, &d.long, 11.0, 500, x, iw, c.y + 29.0, Align::Leading, dim());
                cv.digits(&self.hh, x - 2.0, iw + 4.0, c.y + 84.0, 56.0, 600, Align::Leading, txt());
                cv.digits(&self.mm, x - 2.0, iw + 4.0, c.y + 138.0, 56.0, 600, Align::Leading, acc);
                // Два ряда по 12 точек: часы суток, текущий — частично.
                let (pitch, hours) = (iw / 12.0, self.minutes() as f32 / 60.0);
                for row in 0..2 {
                    for i in 0..12 {
                        let f = (hours - (row * 12 + i) as f32).clamp(0.0, 1.0);
                        cv.circle(x + (i as f32 + 0.5) * pitch, c.y + 174.0 + row as f32 * 9.0, 2.4, mix(track(), acc, f));
                    }
                }
                let pct = (self.day_frac() * 100.0) as u32;
                let (ru, en) = (format!("прошло {pct}% дня"), format!("{pct}% of the day done"));
                small(cv, crate::i18n::pick(&ru, &en), 8.5, 500, x, iw, c.y + 203.0, Align::Leading, dim());
            }
            Shape::Wide => {
                let cw = city_label(cv, c, self.city, c.y + 14.0);
                cap(cv, &d.full, x, iw - cw, c.y + 14.0, Align::Leading, dim());
                time_tone(cv, &self.hh, &self.mm, x, c.y + 47.0, 44.0, 650, false, txt(), acc);
                cv.segments(Rect::new(x, c.y + 70.0, iw, 11.0), 24, (self.minutes() as f32 / 60.0).ceil() as usize, acc);
                for (k, s) in ["0", "6", "12", "18", "24"].iter().enumerate() {
                    let px = x + iw * (k as f32 * 6.0 / 24.0);
                    let (lx, align) = match k {
                        0 => (px, Align::Leading),
                        4 => (px - 20.0, Align::Trailing),
                        _ => (px - 10.0, Align::Center),
                    };
                    small(cv, s, 8.0, 500, lx, 20.0, c.y + 92.0, align, dim());
                }
            }
        }
    }

    /// Центр и радиус круглого циферблата 88 px для стилей 1 и 4; `tall_y` — верх по высоте на 2×4.
    fn dial_at(&self, tall_y: f32) -> (f32, f32, f32) {
        let c = self.c;
        match self.shape {
            Shape::Small => (c.cx(), c.cy(), 44.0),
            Shape::Tall => (c.cx(), c.y + tall_y + 44.0, 44.0),
            Shape::Wide => (c.x + 7.0 + 44.0, c.cy(), 44.0),
        }
    }

    /// 1 «Классика»: стрелки с цифрами 1–12.
    unsafe fn clock_classic(&self, cv: &Canvas) {
        let (c, d) = (self.c, &self.d);
        cv.glow(c, card_radius(), cv.accent, Corner::BottomRight);
        let (cx, cy, r) = self.dial_at(67.0);
        analog(cv, cx, cy, r, self.t, true);
        let part = PART_GEN[day_part(self.t.wHour)];
        let part = crate::i18n::pick(part.0, part.1);
        match self.shape {
            Shape::Small => {}
            Shape::Tall => {
                cap(cv, &d.weekday, c.x + P, c.w - 2.0 * P, c.y + 13.0, Align::Leading, Self::acc_text(cv));
                small(cv, &d.long, 11.0, 500, c.x + P, c.w - 2.0 * P, c.y + 27.0 + 1.0, Align::Leading, fade(txt(), 0.5));
                small(cv, part, 11.0, 500, c.x + P, c.w - 2.0 * P, c.y + 185.0, Align::Leading, fade(txt(), 0.4));
            }
            Shape::Wide => {
                let (x, w) = (c.x + 108.0, c.right() - P - c.x - 108.0);
                cap(cv, &d.weekday_comma, x, w, c.cy() - 17.0, Align::Leading, fade(txt(), 0.85));
                cap(cv, &d.long_up, x, w, c.cy() - 4.0, Align::Leading, fade(txt(), 0.85));
                small(cv, part, 11.0, 500, x, w, c.cy() + 15.0, Align::Leading, dim());
            }
        }
    }

    /// 2 «Точки»: цифры из светящихся точек.
    unsafe fn clock_dots(&self, cv: &Canvas) {
        let (c, d) = (self.c, &self.d);
        let (acc, acc_t) = (cv.accent, Self::acc_text(cv));
        let (hc, mc) = (txt(), readable(acc, 60.0));
        let (x, iw) = (c.x + P, c.w - 2.0 * P);
        cv.glow(c, card_radius(), acc, Corner::TopRight);
        let (h, m) = (self.t.wHour, self.t.wMinute);
        match self.shape {
            Shape::Small => {
                // Две пары в столбик: шаг ограничен и шириной, и высотой (7 + 1 + 7 рядов).
                let pitch = ((c.w - 24.0) / 11.0).min((c.h - 14.0) / 15.0);
                let (x0, y0) = (c.cx() - 11.0 * pitch / 2.0, c.y + (c.h - 15.0 * pitch) / 2.0);
                dot_pair(cv, h, x0, y0, pitch, 0.38 * pitch, hc);
                dot_pair(cv, m, x0, y0 + 8.0 * pitch, pitch, 0.38 * pitch, mc);
            }
            Shape::Tall => {
                cap(cv, &d.short, x, iw, c.y + 13.0, Align::Leading, acc_t);
                let pitch = (c.w - 24.0) / 11.0;
                let x0 = c.cx() - 11.0 * pitch / 2.0;
                dot_pair(cv, h, x0, c.y + 40.0, pitch, 0.38 * pitch, hc);
                dot_pair(cv, m, x0, c.y + 40.0 + 7.0 * pitch + 12.0, pitch, 0.38 * pitch, mc);
                // Ряд из 12 точек: каждая — 5 секунд.
                let lit = usize::from(self.t.wSecond.min(59) / 5);
                let sp = iw / 12.0;
                for i in 0..12 {
                    cv.circle(x + (i as f32 + 0.5) * sp, c.y + 174.0, 2.4, if i < lit { acc } else { track() });
                }
                small(cv, crate::i18n::pick("секунды", "seconds"), 8.0, 500, x, iw, c.y + 193.0, Align::Leading, dim());
            }
            Shape::Wide => {
                let cw = city_label(cv, c, self.city, c.y + 14.0);
                cap(cv, &d.full, x, iw - cw, c.y + 14.0, Align::Leading, dim());
                // 5+1+5 колонок часов, колонка-зазор с двоеточием, 5+1+5 минут: 25 колонок на ширину.
                let pitch = (c.w - 24.0) / 25.0;
                let (x0, y0, rad) = (c.x + 12.0, c.y + 32.0, 0.38 * pitch);
                dot_pair(cv, h, x0, y0, pitch, rad, hc);
                for row in [2.0, 4.0] {
                    cv.circle(x0 + 12.5 * pitch, y0 + (row + 0.5) * pitch, rad, mc);
                }
                dot_pair(cv, m, x0 + 14.0 * pitch, y0, pitch, rad, mc);
            }
        }
    }

    /// 3 «Шкала»: бегущая линейка времени.
    unsafe fn clock_ruler(&self, cv: &Canvas) {
        let (c, d) = (self.c, &self.d);
        let (acc, acc_t) = (cv.accent, Self::acc_text(cv));
        let (x, iw) = (c.x + P, c.w - 2.0 * P);
        let now = self.minutes() as f32 + f32::from(self.t.wSecond.min(59)) / 60.0;
        cv.glow(c, card_radius(), acc, Corner::TopRight);
        match self.shape {
            Shape::Small => {
                time_tone(cv, &self.hh, &self.mm, x, c.y + 22.0, 26.0, 650, false, txt(), txt());
                small(cv, &d.brief, 8.5, 500, x, iw, c.y + 43.0, Align::Leading, dim());
                let base = c.y + 82.0;
                ruler(cv, Rect::new(c.x + 6.0, c.y, c.w - 12.0, c.h), false, now, base);
                self.now_mark(cv, (c.cx(), c.y + 57.0), (c.cx(), base + 1.0));
            }
            Shape::Tall => {
                cap(cv, &d.short, x, iw, c.y + 13.0, Align::Leading, acc_t);
                cv.digits(&self.hh, x - 1.0, 56.0, c.cy() - 20.0, 36.0, 650, Align::Leading, txt());
                cv.digits(&self.mm, x - 1.0, 56.0, c.cy() + 20.0, 36.0, 650, Align::Leading, acc);
                let base = c.right() - 8.0;
                ruler(cv, Rect::new(c.x, c.y + 26.0, c.w, c.h - 52.0), true, now, base);
                self.now_mark(cv, (c.right() - 50.0, c.cy()), (base + 1.0, c.cy()));
            }
            Shape::Wide => {
                let part = PART_NOM[day_part(self.t.wHour)];
                small(cv, crate::i18n::pick(part.0, part.1), 10.0, 500, x, iw, c.y + 14.0, Align::Trailing, dim());
                cap(cv, &d.full, x, iw - 64.0, c.y + 14.0, Align::Leading, dim());
                time_tone(cv, &self.hh, &self.mm, x, c.y + 40.0, 36.0, 650, false, txt(), txt());
                let base = c.y + 80.0;
                ruler(cv, Rect::new(x, c.y, iw, c.h), false, now, base);
                self.now_mark(cv, (c.cx(), c.y + 58.0), (c.cx(), base + 1.0));
            }
        }
    }

    /// Отметка «сейчас» на линейке: линия акцента от `from` (с кружком) до `to`.
    unsafe fn now_mark(&self, cv: &Canvas, from: (f32, f32), to: (f32, f32)) {
        cv.stroke_line(from, to, 1.5, cv.accent);
        cv.circle(from.0, from.1, 2.6, cv.accent);
    }

    /// 4 «Минимал»: метки и стрелки без цифр.
    unsafe fn clock_minimal(&self, cv: &Canvas) {
        let (c, d) = (self.c, &self.d);
        let (cx, cy, r) = self.dial_at(55.0);
        analog(cv, cx, cy, r, self.t, false);
        match self.shape {
            Shape::Small => {}
            Shape::Tall => {
                small(cv, &d.tiny, 12.0, 600, c.x + P, c.w - 2.0 * P, c.y + 172.0, Align::Leading, Self::acc_text(cv));
            }
            Shape::Wide => {
                let x = c.x + 108.0;
                cap(cv, &d.short_full, x, c.right() - P - x, c.cy(), Align::Leading, dim());
            }
        }
    }

    /// 5 «Небо»: градиент по времени суток и путь солнца между восходом и закатом.
    unsafe fn clock_sky(&self, cv: &Canvas) {
        let c = self.c;
        let (sr, ss) = (i32::from(self.sun.0), i32::from(self.sun.1));
        let now = self.minutes() as i32;
        let (top, bottom) = sky(now, sr, ss);
        // Текст на небе — по контрасту с его верхом, а не по теме острова.
        let ink = on_color(top);
        let plate_h = match self.shape {
            Shape::Small => 26.0,
            Shape::Tall => 64.0,
            Shape::Wide => 28.0,
        };
        let horizon = c.bottom() - plate_h;
        let hf = ((horizon - c.y) / c.h).clamp(0.1, 0.95);
        cv.gradient_stops(c, card_radius(), &[(0.0, top), (hf, bottom), (1.0, bottom)], false);

        let (up, f, left) = sun_state(now, sr, ss);
        let (x, iw) = (c.x + P, c.w - 2.0 * P);
        let (hh, mm) = (&self.hh, &self.mm);
        // Дуга и подписи сверху по размерам.
        let (ax, r) = match self.shape {
            Shape::Small => (c.cx(), 32.0),
            Shape::Tall => (c.cx(), 38.0),
            Shape::Wide => (c.right() - P - 46.0, 44.0),
        };
        match self.shape {
            Shape::Small => time_tone(cv, hh, mm, x, c.y + 20.0, 26.0, 650, false, ink, ink),
            Shape::Tall => {
                cap(cv, &self.d.weekday, x, iw, c.y + 13.0, Align::Leading, fade(ink, 0.8));
                time_tone(cv, hh, mm, x, c.y + 44.0, 34.0, 650, false, ink, ink);
                small(cv, &self.d.long, 11.0, 500, x, iw, c.y + 72.0, Align::Leading, fade(ink, 0.75));
            }
            Shape::Wide => {
                cap(cv, &self.d.full, x, iw, c.y + 13.0, Align::Leading, fade(ink, 0.8));
                time_tone(cv, hh, mm, x, c.y + 46.0, 38.0, 650, false, ink, ink);
            }
        }
        self.sky_arc(cv, (ax, horizon), r, up, f, (top, bottom, hf));
        plate(cv, c, plate_h);

        // Плашка: восход/закат и сколько осталось.
        let white = fade(crate::ui::WHITE, 0.92);
        let tilde = if self.sun_est { "~" } else { "" };
        let (rise_s, set_s) = (format!("{tilde}{}", hm(sr)), format!("{tilde}{}", hm(ss)));
        let in_day = crate::i18n::pick("светло ещё", "daylight left");
        let until = if up { crate::i18n::pick("до заката", "sunset in") } else { crate::i18n::pick("до рассвета", "sunrise in") };
        let py = horizon + plate_h / 2.0;
        match self.shape {
            Shape::Small => self.rise_set(cv, &rise_s, &set_s, py, 8.5),
            Shape::Tall => {
                let col2 = c.cx() + 4.0;
                let w = c.right() - P - col2;
                small(cv, crate::i18n::pick("Восход", "Sunrise"), 8.0, 500, x, w, horizon + 14.0, Align::Leading, fade(white, 0.65));
                small(cv, crate::i18n::pick("Закат", "Sunset"), 8.0, 500, col2, w, horizon + 14.0, Align::Leading, fade(white, 0.65));
                small(cv, &rise_s, 15.0, 600, x, w, horizon + 30.0, Align::Leading, white);
                small(cv, &set_s, 15.0, 600, col2, w, horizon + 30.0, Align::Leading, white);
                let word = if up { in_day } else { until };
                let h_m = (crate::i18n::pick("ч", "h"), crate::i18n::pick("м", "m"));
                let s = format!("{word} {} {} {} {}", left / 60, h_m.0, left % 60, h_m.1);
                small(cv, &s, 8.0, 500, x, iw, horizon + 53.0, Align::Leading, fade(white, 0.75));
            }
            Shape::Wide => {
                let rw = 88.0;
                let s = format!("{until} {}", hm(left));
                small(cv, &s, 9.0, 500, x, iw - rw - 6.0, py, Align::Leading, fade(white, 0.85));
                self.rise_set(cv, &rise_s, &set_s, py, 9.0);
            }
        }
    }

    /// «↑ 7:50  16:31 ↓» у краёв плашки на высоте `py`.
    unsafe fn rise_set(&self, cv: &Canvas, rise: &str, set: &str, py: f32, size: f32) {
        let c = self.c;
        let white = fade(crate::ui::WHITE, 0.92);
        let w = 40.0;
        // Стрелки рисуем векторно: в шрифте нужных символов может не быть.
        let ax = c.right() - P - 3.0;
        if self.shape == Shape::Wide {
            let rx = c.right() - P - 3.0 - 6.0 - 38.0 - 40.0 - 4.0;
            cv.arrow(rx + 3.0, py, 6.0, true, white);
            small(cv, rise, size, 600, rx + 9.0, 38.0, py, Align::Leading, white);
            small(cv, set, size, 600, c.right() - P - 9.0 - 38.0, 38.0, py, Align::Trailing, white);
        } else {
            cv.arrow(c.x + P + 3.0, py, 6.0, true, white);
            small(cv, rise, size, 600, c.x + P + 9.0, w, py, Align::Leading, white);
            small(cv, set, size, 600, c.right() - P - 9.0 - w, w, py, Align::Trailing, white);
        }
        cv.arrow(ax, py, 6.0, false, white);
    }

    /// Пунктирная полуокружность над горизонтом и Солнце (днём) либо луна (ночью) на ней.
    /// `center` — середина дуги на горизонте, `f` — доля пути светила, `sky` — (верх, низ, доля высоты, где небо
    /// становится «низом») для выреза серпа цветом неба в точке луны.
    unsafe fn sky_arc(&self, cv: &Canvas, center: (f32, f32), r: f32, up: bool, f: f32, sky: (D2D1_COLOR_F, D2D1_COLOR_F, f32)) {
        let (ax, ay) = center;
        // Штрих ~2 px (1 px + круглые концы), шаг ~5 px по длине дуги.
        let n = ((std::f32::consts::PI * r / 5.0) as usize).max(2);
        let (step, dash) = (180.0 / n as f32, (1.0 / r).to_degrees());
        let col = fade(crate::ui::WHITE, if up { 0.5 } else { 0.2 });
        for i in 0..n {
            let a = -90.0 + i as f32 * step;
            cv.stroke_arc(ax, ay, r, a, a + dash, 1.0, col);
        }
        // Солнце идёт по всей дуге от горизонта до горизонта; луна — чуть выше горизонта, чтобы не прятаться в плашке.
        let f = if up { f.clamp(0.0, 1.0) } else { f.clamp(0.08, 0.92) };
        let a = (-90.0 + 180.0 * f).to_radians();
        let (x, y) = (ax + r * a.sin(), ay - r * a.cos());
        if up {
            for (rad, alpha) in [(16.0, 0.06), (12.0, 0.10), (9.5, 0.18)] {
                cv.circle(x, y, rad, fade(SUN_COLOR, alpha));
            }
            cv.circle(x, y, 7.0, SUN_COLOR);
        } else {
            // Луна — серп: светлый круг и вырез цветом неба в этой точке; вокруг мягкое свечение.
            let moon = rgb(0.93, 0.95, 1.0);
            let (top, bottom, hf) = sky;
            let t = ((y - self.c.y) / self.c.h / hf.max(0.01)).clamp(0.0, 1.0);
            let bg = mix(top, bottom, t);
            for (rad, alpha) in [(13.0, 0.05), (10.0, 0.09)] {
                cv.circle(x, y, rad, fade(moon, alpha));
            }
            cv.circle(x, y, 6.0, moon);
            cv.circle(x + 2.8, y - 1.6, 5.2, bg);
        }
    }
}

impl Today {
    /// Циферблат стиля `self.clock_style` в карточке `c` размера `size` (индекс в `CLOCK_SIZES`).
    pub(super) unsafe fn clock_face(&self, cv: &Canvas, c: Rect, t: &SYSTEMTIME, size: usize, dow: usize) {
        let weather = self.online.weather.as_ref();
        let day = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        // Восход и закат нужны только «Небу»: остальным стилям лишняя арифметика ни к чему.
        let (sun, sun_est) = if self.clock_style == 5 {
            match weather.and_then(|w| w.coords).and_then(|(lat, lon)| sun_times(day, lat, lon, crate::online::utc_offset_minutes())) {
                Some(s) => (s, false),
                // Нет координат (погода ещё не загрузилась) или полярный день/ночь: значения по умолчанию, с «~».
                None => (DEFAULT_SUN, true),
            }
        } else {
            (DEFAULT_SUN, false)
        };
        let face = Face {
            c,
            shape: Shape::of(size),
            t,
            d: Dates::new(t, dow),
            hh: format!("{:02}", t.wHour),
            mm: format!("{:02}", t.wMinute),
            city: weather.map_or("", |w| w.city.as_str()),
            sun,
            sun_est,
        };
        match self.clock_style {
            0 => face.clock_digits(cv),
            1 => face.clock_classic(cv),
            2 => face.clock_dots(cv),
            3 => face.clock_ruler(cv),
            4 => face.clock_minimal(cv),
            _ => face.clock_sky(cv),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_point_where_expected() {
        assert_eq!(hand_angles(3, 0, 0), (90.0, 0.0, 0.0));
        let (h, m, s) = hand_angles(22, 30, 15);
        assert!((h - 315.0).abs() < 1e-4 && (m - 181.5).abs() < 1e-4 && (s - 90.0).abs() < 1e-4);
        // Макетное время 09:41:30 (минутная уже сдвинута секундами: 41,5 мин = 249°).
        let (h, m, s) = hand_angles(9, 41, 30);
        assert!((h - 290.5).abs() < 1e-3 && (m - 249.0).abs() < 1e-3 && (s - 180.0).abs() < 1e-3);
    }

    #[test]
    fn iso_weeks() {
        // 2026-01-01 — четверг (неделя 1); 2024-12-30 — понедельник первой недели 2025 года.
        assert_eq!(iso_week(days_from_civil(2026, 1, 1)), 1);
        assert_eq!(iso_week(days_from_civil(2024, 12, 30)), 1);
        assert_eq!(iso_week(days_from_civil(2026, 10, 22)), 43);
        assert_eq!(iso_week(days_from_civil(2021, 1, 3)), 53);
    }

    #[test]
    fn day_parts_by_hour() {
        assert_eq!([4, 5, 11, 12, 16, 17, 21, 22, 0].map(day_part), [3, 0, 0, 1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn dot_font_zero_is_slashed_and_fits_five_columns() {
        assert!(DOT_FONT.iter().flatten().all(|&b| b < 32));
        assert_ne!(DOT_FONT[0], DOT_FONT[8]);
    }

    #[test]
    fn sun_state_covers_the_whole_day() {
        let (sr, ss) = (7 * 60 + 50, 16 * 60 + 31);
        let (up, f, left) = sun_state(9 * 60 + 41, sr, ss);
        assert!(up && (f - 0.2).abs() < 0.05 && left == ss - (9 * 60 + 41));
        let (up, f, left) = sun_state(20 * 60, sr, ss);
        assert!(!up && f > 0.0 && f < 1.0 && left == sr + 1440 - 20 * 60);
        let (up, f, left) = sun_state(3 * 60, sr, ss);
        assert!(!up && f > 0.0 && f < 1.0 && left == sr - 3 * 60);
    }

    #[test]
    fn minutes_format() {
        assert_eq!(hm(470), "7:50");
        assert_eq!(hm(991), "16:31");
    }
}
