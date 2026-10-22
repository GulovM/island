//! Карточка погоды: три вида по размеру и ввод города.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::{Today, SUN_COLOR};
use crate::online;
use crate::tabs::{id, TODAY};
use crate::ui::{dim, fg, icon, mix, txt, Align, Canvas, Rect, Submit, Ui};

impl Today {
    /// Погода. `size`: 0 — компактная (значок, градусы, описание), 1 — обычная, 2 — широкая в одну
    /// строку. Клик по карточке — поле для города (пусто — определить по IP).
    pub(super) unsafe fn weather_card(&mut self, ui: &mut Ui, cv: &Canvas, wc: Rect, size: usize) {
        let f = &cv.g.fonts;
        cv.card(wc);
        let field = id(TODAY, 61);
        if self.editing {
            if let Some(city) = ui.take_submitted(field) {
                self.city = city.trim().to_string();
                crate::config::save_str("weather_city", &self.city);
                online::refresh_weather(&self.city);
                self.online = online::snapshot();
                self.editing = false;
                ui.blur();
            } else if !ui.has_focus(field) {
                // Esc или клик мимо — без изменений.
                self.editing = false;
            }
        }
        if self.editing {
            if size == 1 {
                cv.text("Город", Rect::new(wc.x + 9.0, wc.y + 8.0, wc.w - 18.0, 12.0), &f.small_b, dim());
                ui.text_field_ex(cv, field, Rect::new(wc.x + 5.0, wc.y + 24.0, wc.w - 10.0, 24.0), "по IP", icon::SEARCH, Submit::Keep);
                cv.text("Enter — сохранить", Rect::new(wc.x + 4.0, wc.y + 53.0, wc.w - 8.0, 12.0), &f.small_c, dim());
                cv.text("пусто — по IP", Rect::new(wc.x + 4.0, wc.y + 66.0, wc.w - 8.0, 12.0), &f.small_c, mix(dim(), crate::ui::card(), 0.35));
            } else {
                // Низкая карточка: только поле, подсказки не помещаются.
                ui.text_field_ex(cv, field, Rect::new(wc.x + 5.0, wc.cy() - 12.0, wc.w - 10.0, 24.0), "по IP", icon::SEARCH, Submit::Keep);
            }
            return;
        }
        let (hover, clicked) = ui.click_area(id(TODAY, 60), wc);
        if clicked {
            self.editing = true;
            ui.set_field_text(field, &self.city);
            ui.focus = Some(field);
            ui.caret_on = true;
        }
        let hv = ui.hover_of(id(TODAY, 60));
        if hover || hv > 0.01 {
            cv.rr(wc, crate::ui::card_radius(), D2D1_COLOR_F { a: 0.04 * hv, ..fg() });
            cv.glyph(icon::EDIT, Rect::new(wc.right() - 18.0, wc.y + 3.0, 14.0, 14.0), false, D2D1_COLOR_F { a: hv, ..dim() });
        }

        // Значок, температура, описание и город (его нет, пока погода не загружена).
        let (glyph, col, temp, tcol, desc, city) = match &self.online.weather {
            Some(wt) => {
                let (g, col) = if wt.code <= 1 { (icon::SUN, crate::ui::hue(crate::ios::SysColor::Yellow, SUN_COLOR)) } else { (icon::CLOUD, mix(dim(), txt(), 0.5)) };
                (g, col, format!("{}°C", wt.temp.round() as i32), txt(), online::weather_text(wt.code).to_string(), wt.city.clone())
            }
            None => {
                let note = self.online.weather_note.as_deref().unwrap_or("Загрузка…").to_string();
                (icon::CLOUD, dim(), "—".to_string(), dim(), note, String::new())
            }
        };
        let faint = mix(dim(), crate::ui::card(), 0.35);
        const P: f32 = 12.0;
        let (iw, cy) = (wc.w - 2.0 * P, wc.cy());
        let city_or = |fallback: &str| if city.is_empty() { fallback.to_string() } else { city.clone() };
        match size {
            // Карточка 120×56: температура крупно, состояние под ней, значок справа.
            0 => {
                let col = D2D1_COLOR_F { a: col.a * (1.0 - hv), ..col };
                cv.glyph(glyph, Rect::new(wc.right() - P - 20.0, wc.y + P - 2.0, 20.0, 20.0), true, col);
                cv.digits(&temp, wc.x + P - 1.0, iw - 20.0, cy - 6.0, 22.0, 600, Align::Leading, tcol);
                cv.line(&desc, &f.small, wc.x + P, iw, cy + 14.0, Align::Leading, dim());
            }
            // Карточка 120×88: город, крупная температура, внизу значок и состояние (как виджет Погоды iOS).
            1 => {
                cv.line(&city_or(crate::i18n::pick("Погода", "Weather")), &f.body, wc.x + P, iw - 14.0, wc.y + P + 5.0, Align::Leading, txt());
                cv.digits(&temp, wc.x + P - 1.0, iw, wc.y + 44.0, 30.0, 500, Align::Leading, tcol);
                cv.glyph(glyph, Rect::new(wc.x + P - 1.0, wc.bottom() - P - 13.0, 14.0, 14.0), false, col);
                cv.line(&desc, &f.small_b, wc.x + P + 16.0, iw - 16.0, wc.bottom() - P - 6.0, Align::Leading, txt());
            }
            // Широкая 248×56: значок и температура слева, состояние и город справа.
            _ => {
                cv.glyph(glyph, Rect::new(wc.x + P, cy - 13.0, 26.0, 26.0), true, col);
                let tw = cv.digits_w(&temp, 28.0, 500);
                cv.digits(&temp, wc.x + P + 34.0, tw + 4.0, cy, 28.0, 500, Align::Leading, tcol);
                let tx = wc.x + P + 34.0 + tw + 12.0;
                let rw = wc.right() - P - tx;
                cv.line(&desc, &f.body, tx, rw, cy - 7.0, Align::Trailing, txt());
                cv.line(&city_or("—"), &f.small, tx, rw, cy + 8.0, Align::Trailing, faint);
            }
        }
    }
}
