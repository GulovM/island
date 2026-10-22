//! Циферблаты часов. Пять стилей, у каждого своя композиция под каждый размер карточки
//! (`CLOCK_SIZES`: 0 — 120×88, 1 — высокая 120×152, 2 — широкая 248×88):
//!
//! 0. «Классика» — день недели и дата сверху, крупное время снизу (как экран блокировки iOS);
//! 1. «Стрелки» — аналоговый циферблат с делениями и секундной стрелкой;
//! 2. «Кольцо» — кольцо секунд вокруг времени (полный круг — минута);
//! 3. «StandBy» — крупные жирные цифры в два цвета: часы основным, минуты акцентом;
//! 4. «Перекидные» — цифры на плитках с линией сгиба.

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::Today;
use crate::anim::Spring;
use crate::ios::SysColor;
use crate::online::{civil_from_days, days_from_civil};
use crate::ui::{dim, fg, hue, txt, Align, Canvas, Rect, WHITE};

/// Внутренний отступ карточки часов.
const P: f32 = 12.0;

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

/// Доля круга секунд 0..1: на :59 круг замкнут, дальше счёт идёт заново (+1 минута).
fn second_frac(sec: u16) -> f32 {
    (f32::from(sec.min(59)) + 1.0) / 60.0
}

/// Цель кольца секунд: новая минута начинает круг заново, а не откатывает его назад.
pub(super) fn aim_seconds(ring: &mut Spring, sec: u16) {
    let frac = second_frac(sec);
    if frac < ring.target - 0.5 {
        ring.value = 0.0;
    }
    ring.target = frac;
}

/// Номер недели по ISO 8601 (неделя с понедельника, первая — та, где первый четверг года).
fn iso_week(day: i64) -> u32 {
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

/// Подложка циферблата и плиток: белая на светлом острове, затемнение на тёмном.
fn face() -> D2D1_COLOR_F {
    let bg = crate::theme::get().bg;
    if crate::theme::is_light([bg.r, bg.g, bg.b]) {
        D2D1_COLOR_F { a: 0.92, ..WHITE }
    } else {
        D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.42 }
    }
}

/// Подписи даты для карточки.
struct Dates {
    /// «ЧЕТВЕРГ».
    weekday: String,
    /// «ЧТ, 22 ОКТ».
    short: String,
    /// «22 октября».
    long: String,
    /// «Неделя 43».
    week: String,
}

impl Dates {
    fn new(t: &SYSTEMTIME, dow: usize) -> Self {
        let day = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        let month = crate::i18n::months_short()[(t.wMonth as usize).clamp(1, 12) - 1];
        Dates {
            weekday: crate::i18n::weekdays_full()[dow].to_uppercase(),
            short: format!("{}, {} {}", crate::i18n::weekdays()[dow], t.wDay, month).to_uppercase(),
            long: crate::i18n::day_month(t.wDay as u32, t.wMonth as u32),
            week: format!("{} {}", crate::i18n::pick("Неделя", "Week"), iso_week(day)),
        }
    }
}

/// Крупные цифры с серединой на `cy` (см. `Canvas::digits`).
#[allow(clippy::too_many_arguments)]
unsafe fn digits(cv: &Canvas, s: &str, x: f32, w: f32, cy: f32, size: f32, weight: u32, align: Align, c: D2D1_COLOR_F) {
    cv.digits(s, x, w, cy, size, weight, align, c);
}

/// «10:09» в два цвета (часы, двоеточие, минуты) по центру `cx`.
#[allow(clippy::too_many_arguments)]
unsafe fn two_tone(cv: &Canvas, hh: &str, mm: &str, cx: f32, cy: f32, size: f32, weight: u32, hc: D2D1_COLOR_F, mc: D2D1_COLOR_F) {
    let (wh, wc, wm) = (cv.digits_w(hh, size, weight), cv.digits_w(":", size, weight), cv.digits_w(mm, size, weight));
    let x = cx - (wh + wc + wm) / 2.0;
    digits(cv, hh, x, wh + 2.0, cy, size, weight, Align::Leading, hc);
    digits(cv, ":", x + wh, wc + 2.0, cy - 0.04 * size, size, weight, Align::Leading, D2D1_COLOR_F { a: 0.28, ..dim() });
    digits(cv, mm, x + wh + wc, wm + 2.0, cy, size, weight, Align::Leading, mc);
}

/// Мелкая подпись прописными с выравниванием в полосе `x..x + w`.
unsafe fn caps(cv: &Canvas, s: &str, x: f32, w: f32, cy: f32, align: Align, c: D2D1_COLOR_F) {
    cv.line(s, &cv.g.fonts.caps, x, w, cy, align, c);
}

/// Строка подписи: `strong` — основная (Body), иначе вторичная (Subheadline).
#[allow(clippy::too_many_arguments)]
unsafe fn label(cv: &Canvas, s: &str, x: f32, w: f32, cy: f32, align: Align, strong: bool, c: D2D1_COLOR_F) {
    let f = if strong { &cv.g.fonts.body } else { &cv.g.fonts.small };
    cv.line(s, f, x, w, cy, align, c);
}

/// Аналоговый циферблат радиуса `r` с центром (`cx`, `cy`).
unsafe fn dial(cv: &Canvas, cx: f32, cy: f32, r: f32, t: &SYSTEMTIME) {
    let face_c = face();
    cv.circle(cx, cy, r, face_c);
    if crate::theme::glass() {
        cv.rr_line(Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r, crate::ui::hairline(), 1.0);
    }
    let at = |deg: f32, k: f32| {
        let a = deg.to_radians();
        (cx + r * k * a.sin(), cy - r * k * a.cos())
    };
    // Деления: часовые — штрихи, минутные — тонкие (только на крупном циферблате).
    for i in 0..60 {
        let deg = i as f32 * 6.0;
        if i % 5 == 0 {
            let w = if i % 15 == 0 { (r * 0.045).max(1.3) } else { (r * 0.032).max(1.0) };
            cv.stroke_line(at(deg, 0.80), at(deg, 0.90), w, D2D1_COLOR_F { a: 0.85, ..txt() });
        } else if r >= 40.0 {
            cv.stroke_line(at(deg, 0.86), at(deg, 0.90), 0.6, D2D1_COLOR_F { a: 0.35, ..dim() });
        }
    }
    // Цифры 12, 3, 6, 9.
    let fs = (r * 0.2).max(6.5);
    let nf = cv.g.fonts.display(fs, 600, Align::Center);
    for (k, n) in ["12", "3", "6", "9"].iter().enumerate() {
        let (x, y) = at(k as f32 * 90.0, 0.62);
        cv.text(n, Rect::new(x - fs, y - 0.73 * fs, 2.0 * fs, 1.33 * fs), &nf, txt());
    }
    let (ha, ma, sa) = hand_angles(t.wHour, t.wMinute, t.wSecond);
    let hand = txt();
    cv.stroke_line(at(ha + 180.0, 0.08), at(ha, 0.50), (r * 0.075).max(2.2), hand);
    cv.stroke_line(at(ma + 180.0, 0.08), at(ma, 0.80), (r * 0.05).max(1.6), hand);
    cv.circle(cx, cy, (r * 0.075).max(2.2), hand);
    let sec = hue(SysColor::Orange, cv.accent);
    cv.stroke_line(at(sa + 180.0, 0.20), at(sa, 0.88), (r * 0.022).max(0.8), sec);
    cv.circle(cx, cy, (r * 0.05).max(1.5), sec);
    cv.circle(cx, cy, (r * 0.022).max(0.7), face_c);
}

/// Перекидная плитка с цифрами и линией сгиба посередине.
unsafe fn flip(cv: &Canvas, r: Rect, s: &str, size: f32, c: D2D1_COLOR_F) {
    let rad = (r.h * 0.16).clamp(5.0, 9.0);
    cv.rr(r, rad, face());
    cv.hairline(r, rad);
    // Верхняя половина чуть светлее — как у механических табло.
    cv.gradient_stops(r, rad, &[(0.0, D2D1_COLOR_F { a: 0.05, ..fg() }), (0.5, D2D1_COLOR_F { a: 0.0, ..fg() })], false);
    digits(cv, s, r.x, r.w, r.cy(), size, 650, Align::Center, c);
    cv.rr(Rect::new(r.x + 1.0, r.cy() - 0.5, r.w - 2.0, 1.0), 0.0, D2D1_COLOR_F { a: 0.14, ..fg() });
}

impl Today {
    /// Циферблат стиля `self.clock_style` в карточке `c` размера `size` (индекс в `CLOCK_SIZES`).
    pub(super) unsafe fn clock_face(&mut self, cv: &Canvas, c: Rect, t: &SYSTEMTIME, size: usize, dow: usize) {
        let shape = Shape::of(size);
        let hh = format!("{:02}", t.wHour);
        let mm = format!("{:02}", t.wMinute);
        let hm = format!("{hh}:{mm}");
        let ss = format!("{:02}", t.wSecond.min(59));
        let d = Dates::new(t, dow);
        // День недели — красным, как в Календаре iOS (в стиле Windows — акцентом).
        let day_c = hue(SysColor::Red, cv.accent);
        let (cx, cy) = (c.cx(), c.cy());
        let iw = c.w - 2.0 * P;
        match self.clock_style {
            // Классика.
            0 => match shape {
                Shape::Small => {
                    caps(cv, &d.weekday, c.x + P, iw, c.y + P + 4.0, Align::Leading, day_c);
                    label(cv, &d.long, c.x + P, iw, c.y + P + 18.0, Align::Leading, true, txt());
                    digits(cv, &hm, c.x + P - 1.0, iw + 2.0, c.bottom() - P - 12.0, 34.0, 600, Align::Leading, txt());
                }
                Shape::Tall => {
                    caps(cv, &d.weekday, c.x + P, iw, c.y + P + 4.0, Align::Leading, day_c);
                    label(cv, &d.long, c.x + P, iw, c.y + P + 18.0, Align::Leading, true, txt());
                    let bottom = c.bottom() - P - 18.0;
                    digits(cv, &hh, c.x + P - 2.0, iw, bottom - 46.0, 52.0, 600, Align::Leading, txt());
                    digits(cv, &mm, c.x + P - 2.0, iw, bottom, 52.0, 600, Align::Leading, txt());
                }
                Shape::Wide => {
                    digits(cv, &hm, c.x + P - 1.0, 140.0, cy, 46.0, 600, Align::Leading, txt());
                    let (x, w) = (c.right() - P - 100.0, 100.0);
                    caps(cv, &d.weekday, x, w, cy - 16.0, Align::Trailing, day_c);
                    label(cv, &d.long, x, w, cy, Align::Trailing, true, txt());
                    label(cv, &d.week, x, w, cy + 15.0, Align::Trailing, false, dim());
                }
            },
            // Стрелки.
            1 => match shape {
                Shape::Small => dial(cv, cx, cy, c.h / 2.0 - 8.0, t),
                Shape::Tall => {
                    let r = c.w / 2.0 - P - 4.0;
                    dial(cv, cx, c.y + P + r + 2.0, r, t);
                    caps(cv, &d.weekday, c.x, c.w, c.bottom() - P - 22.0, Align::Center, day_c);
                    label(cv, &d.long, c.x, c.w, c.bottom() - P - 7.0, Align::Center, true, txt());
                }
                Shape::Wide => {
                    let r = c.h / 2.0 - 9.0;
                    dial(cv, c.x + P + r, cy, r, t);
                    let x = c.x + P + 2.0 * r + 18.0;
                    let w = c.right() - P - x;
                    digits(cv, &hm, x - 1.0, w, cy - 9.0, 34.0, 600, Align::Leading, txt());
                    caps(cv, &d.short, x, w, cy + 20.0, Align::Leading, day_c);
                }
            },
            // Кольцо секунд.
            2 => {
                let frac = self.sec_ring.value.clamp(0.0, 1.0);
                let ring_c = cv.accent;
                match shape {
                    Shape::Small => {
                        let r = c.h / 2.0 - 10.0;
                        cv.ring(cx, cy, r, 4.0, frac, ring_c);
                        digits(cv, &hm, cx - r, 2.0 * r, cy - 3.0, 17.0, 650, Align::Center, txt());
                        caps(cv, &crate::i18n::weekdays()[dow].to_uppercase(), cx - r, 2.0 * r, cy + 12.0, Align::Center, day_c);
                    }
                    Shape::Tall => {
                        let r = c.w / 2.0 - P - 4.0;
                        let rc = c.y + P + r + 2.0;
                        cv.ring(cx, rc, r, 5.0, frac, ring_c);
                        digits(cv, &hm, cx - r, 2.0 * r, rc - 4.0, 21.0, 650, Align::Center, txt());
                        digits(cv, &format!(":{ss}"), cx - r, 2.0 * r, rc + 14.0, 10.0, 600, Align::Center, dim());
                        caps(cv, &d.weekday, c.x, c.w, c.bottom() - P - 22.0, Align::Center, day_c);
                        label(cv, &d.long, c.x, c.w, c.bottom() - P - 7.0, Align::Center, true, txt());
                    }
                    Shape::Wide => {
                        let r = c.h / 2.0 - 10.0;
                        let rx = c.x + P + r;
                        cv.ring(rx, cy, r, 4.0, frac, ring_c);
                        digits(cv, &ss, rx - r, 2.0 * r, cy - 3.0, 20.0, 650, Align::Center, txt());
                        caps(cv, crate::i18n::pick("СЕК", "SEC"), rx - r, 2.0 * r, cy + 12.0, Align::Center, dim());
                        let x = rx + r + 18.0;
                        let w = c.right() - P - x;
                        digits(cv, &hm, x - 1.0, w, cy - 9.0, 34.0, 600, Align::Leading, txt());
                        caps(cv, &d.short, x, w, cy + 20.0, Align::Leading, day_c);
                    }
                }
            }
            // StandBy.
            3 => {
                let mc = cv.accent;
                match shape {
                    Shape::Small => two_tone(cv, &hh, &mm, cx, cy - 7.0, 36.0, 700, txt(), mc),
                    Shape::Tall => {
                        digits(cv, &hh, c.x, c.w, c.y + P + 30.0, 64.0, 700, Align::Center, txt());
                        digits(cv, &mm, c.x, c.w, c.y + P + 84.0, 64.0, 700, Align::Center, mc);
                    }
                    Shape::Wide => two_tone(cv, &hh, &mm, cx, cy - 7.0, 60.0, 700, txt(), mc),
                }
                caps(cv, &d.short, c.x, c.w, c.bottom() - P - 4.0, Align::Center, dim());
            }
            // Перекидные.
            _ => match shape {
                Shape::Small => {
                    let (tw, th, gap) = ((c.w - 2.0 * 10.0 - 6.0) / 2.0, 50.0, 6.0);
                    let y = c.y + 10.0;
                    flip(cv, Rect::new(c.x + 10.0, y, tw, th), &hh, 32.0, txt());
                    flip(cv, Rect::new(c.x + 10.0 + tw + gap, y, tw, th), &mm, 32.0, txt());
                    caps(cv, &d.short, c.x, c.w, c.bottom() - P - 2.0, Align::Center, dim());
                }
                Shape::Tall => {
                    let (tw, th) = (c.w - 2.0 * P, 54.0);
                    flip(cv, Rect::new(c.x + P, c.y + P, tw, th), &hh, 42.0, txt());
                    flip(cv, Rect::new(c.x + P, c.y + P + th + 6.0, tw, th), &mm, 42.0, txt());
                    caps(cv, &d.short, c.x, c.w, c.bottom() - P - 2.0, Align::Center, dim());
                }
                Shape::Wide => {
                    let (tw, th, sw, sh, gap) = (70.0, 62.0, 44.0, 40.0, 8.0);
                    let x = cx - (2.0 * tw + sw + 2.0 * gap) / 2.0;
                    let y = cy - th / 2.0;
                    flip(cv, Rect::new(x, y, tw, th), &hh, 44.0, txt());
                    flip(cv, Rect::new(x + tw + gap, y, tw, th), &mm, 44.0, txt());
                    let sx = x + 2.0 * (tw + gap);
                    flip(cv, Rect::new(sx, y + th - sh, sw, sh), &ss, 26.0, cv.accent);
                    caps(cv, &crate::i18n::weekdays()[dow].to_uppercase(), sx, sw, y + 4.0, Align::Center, day_c);
                    label(cv, &t.wDay.to_string(), sx, sw, y + 15.0, Align::Center, true, txt());
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_circle() {
        assert!((second_frac(0) - 1.0 / 60.0).abs() < 1e-6);
        assert!((second_frac(59) - 1.0).abs() < 1e-6);
        assert!((second_frac(200) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hands_point_where_expected() {
        assert_eq!(hand_angles(3, 0, 0), (90.0, 0.0, 0.0));
        let (h, m, s) = hand_angles(22, 30, 15);
        assert!((h - 315.0).abs() < 1e-4 && (m - 181.5).abs() < 1e-4 && (s - 90.0).abs() < 1e-4);
    }

    #[test]
    fn iso_weeks() {
        // 2026-01-01 — четверг (неделя 1); 2024-12-30 — понедельник первой недели 2025 года.
        assert_eq!(iso_week(days_from_civil(2026, 1, 1)), 1);
        assert_eq!(iso_week(days_from_civil(2024, 12, 30)), 1);
        assert_eq!(iso_week(days_from_civil(2026, 10, 22)), 43);
        assert_eq!(iso_week(days_from_civil(2021, 1, 3)), 53);
    }
}
