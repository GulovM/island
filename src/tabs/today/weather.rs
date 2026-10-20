//! Карточка погоды: три вида по размеру и ввод города.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::{Today, SUN_COLOR};
use crate::online;
use crate::tabs::{id, TODAY};
use crate::ui::{dim, fg, icon, mix, txt, Canvas, Rect, Submit, Ui};

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
            cv.rr(wc, 11.0, D2D1_COLOR_F { a: 0.04 * hv, ..fg() });
            cv.glyph(icon::EDIT, Rect::new(wc.right() - 22.0, wc.y + 5.0, 16.0, 16.0), false, D2D1_COLOR_F { a: hv, ..dim() });
        }

        // Значок, температура, описание и город (его нет, пока погода не загружена).
        let (glyph, col, temp, tcol, desc, city) = match &self.online.weather {
            Some(wt) => {
                let (g, col) = if wt.code <= 1 { (icon::SUN, SUN_COLOR) } else { (icon::CLOUD, mix(dim(), txt(), 0.5)) };
                (g, col, format!("{}°C", wt.temp.round() as i32), txt(), online::weather_text(wt.code).to_string(), wt.city.clone())
            }
            None => {
                let note = self.online.weather_note.as_deref().unwrap_or("Загрузка…").to_string();
                (icon::CLOUD, dim(), "—".to_string(), dim(), note, String::new())
            }
        };
        let faint = mix(dim(), crate::ui::card(), 0.35);
        match size {
            0 => {
                cv.glyph(glyph, Rect::new(wc.x + 10.0, wc.cy() - 11.0, 22.0, 22.0), true, col);
                cv.text(&temp, Rect::new(wc.x + 38.0, wc.y + 14.0, wc.w - 44.0, 22.0), &f.num, tcol);
                cv.text(&desc, Rect::new(wc.x + 38.0, wc.y + 37.0, wc.w - 44.0, 12.0), &f.small, dim());
            }
            1 => {
                cv.glyph(glyph, Rect::new(wc.x, wc.y + 11.0, wc.w, 20.0), true, col);
                cv.text(&temp, Rect::new(wc.x, wc.y + 33.0, wc.w, 22.0), &f.num_c, tcol);
                cv.text(&desc, Rect::new(wc.x + 4.0, wc.y + 56.0, wc.w - 8.0, 12.0), &f.small_c, dim());
                if !city.is_empty() {
                    cv.text(&city, Rect::new(wc.x + 4.0, wc.y + 70.0, wc.w - 8.0, 12.0), &f.small_c, faint);
                }
            }
            _ => {
                cv.glyph(glyph, Rect::new(wc.x + 14.0, wc.cy() - 13.0, 26.0, 26.0), true, col);
                cv.text(&temp, Rect::new(wc.x + 48.0, wc.cy() - 11.0, 74.0, 22.0), &f.num, tcol);
                let tx = wc.x + 124.0;
                let tw = (wc.right() - tx - 22.0).max(0.0);
                cv.text(&desc, Rect::new(tx, wc.cy() - 15.0, tw, 13.0), &f.small_b, txt());
                cv.text(&city, Rect::new(tx, wc.cy() + 3.0, tw, 13.0), &f.small, faint);
            }
        }
    }
}
