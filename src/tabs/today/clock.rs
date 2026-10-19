//! Циферблаты часов на карточке слева.

use windows::Win32::Foundation::SYSTEMTIME;

use super::Today;
use crate::ui::{track, txt, Canvas, Rect};

/// Доля круга секунд 0..1: на :59 круг замкнут, дальше счёт идёт заново (+1 минута).
fn second_frac(sec: u16) -> f32 {
    (f32::from(sec.min(59)) + 1.0) / 60.0
}

impl Today {
    /// Циферблат: 0 — столбик ЧЧ/ММ; 1 — столбик с секундами; 2 — «ЧЧ:ММ»; 3 — «ЧЧ:ММ» с полоской секунд;
    /// 4 — круг секунд (полный оборот — +1 минута) вокруг «ЧЧ:ММ».
    pub(super) unsafe fn clock_face(&mut self, cv: &Canvas, c: Rect, t: &SYSTEMTIME) {
        let f = &cv.g.fonts;
        let hm = format!("{:02}:{:02}", t.wHour, t.wMinute);
        let (hh, mm) = (format!("{:02}", t.wHour), format!("{:02}", t.wMinute));
        let ss = format!(":{:02}", t.wSecond);
        let frac = second_frac(t.wSecond);
        if self.clock_style >= 3 {
            // Новая минута: круг начинается заново, а не откатывается назад.
            if frac < self.sec_ring.target - 0.5 {
                self.sec_ring.value = 0.0;
            }
            self.sec_ring.target = frac;
        }
        match self.clock_style {
            0 => {
                cv.text(&hh, Rect::new(c.x, c.y + 24.0, c.w, 36.0), &f.big_c, txt());
                cv.text(&mm, Rect::new(c.x, c.y + 66.0, c.w, 36.0), &f.big_c, txt());
            }
            1 => {
                cv.text(&hh, Rect::new(c.x, c.y + 18.0, c.w, 36.0), &f.big_c, txt());
                cv.text(&mm, Rect::new(c.x, c.y + 52.0, c.w, 36.0), &f.big_c, txt());
                cv.text(&ss, Rect::new(c.x, c.y + 94.0, c.w, 18.0), &f.num_c, cv.accent);
            }
            2 => {
                cv.text(&hm, Rect::new(c.x, c.y + 44.0, c.w, 36.0), &f.big_c, txt());
            }
            3 => {
                cv.text(&hm, Rect::new(c.x, c.y + 32.0, c.w, 36.0), &f.big_c, txt());
                let bar = Rect::new(c.x + 18.0, c.y + 78.0, c.w - 36.0, 4.0);
                cv.rr(bar, 2.0, track());
                cv.rr(Rect::new(bar.x, bar.y, (bar.w * self.sec_ring.value.clamp(0.0, 1.0)).max(4.0), bar.h), 2.0, cv.accent);
                cv.text(&ss, Rect::new(c.x, c.y + 90.0, c.w, 18.0), &f.num_c, cv.accent);
            }
            _ => {
                let cy = c.y + 68.0;
                cv.ring(c.cx(), cy, 42.0, 4.0, self.sec_ring.value, cv.accent);
                cv.text(&hm, Rect::new(c.x, cy - 18.0, c.w, 36.0), &f.num_c, txt());
            }
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
}
