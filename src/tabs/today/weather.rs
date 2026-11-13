//! Карточка погоды: три вида по размеру и ввод города.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::{Today, SUN_COLOR};
use crate::online;
use crate::tabs::{id, TODAY};
use crate::ui::{
    dim, fg, icon, mix, readable, status_blue, txt, Align, Canvas, Corner, Rect, Submit, Ui,
};

// ID погоды на вкладке: карточка и поле города.
const ID_CARD: u32 = 60;
const ID_CITY: u32 = 61;

/// Занятые погодой ID вкладки для теста непересечения в `mod.rs`.
#[cfg(test)]
pub(super) const ID_RANGES: &[(u32, u32)] = &[(ID_CARD, ID_CITY + 1)];

/// Форматирует температуру: без буквы C, с настоящим математическим минусом U+2212 для отрицательных.
fn format_temp(temp: f32) -> String {
    let t = temp.round() as i32;
    if t < 0 {
        format!("\u{2212}{}°", t.abs())
    } else {
        format!("{t}°")
    }
}

impl Today {
    /// Погода. `size`: 0 — 2×1 (104×48), 1 — 2×2 (104×104), 2 — 4×1 (216×48).
    /// Клик по карточке открывает поле для ввода города (пусто — авто по IP).
    pub(super) unsafe fn weather_card(&mut self, ui: &mut Ui, cv: &Canvas, wc: Rect, size: usize) {
        let f = &cv.g.fonts;
        cv.card(wc);
        // Голубоватый ореол в правом верхнем углу по макету тёмного стекла.
        cv.glow(wc, 14.0, status_blue(), Corner::TopRight);

        let field = id(TODAY, ID_CITY);
        if self.editing {
            if let Some(city) = ui.take_submitted(field) {
                self.city = city.trim().to_string();
                crate::config::save_str("weather_city", &self.city);
                online::refresh_weather(&self.city);
                self.online = online::snapshot();
                self.editing = false;
                ui.blur();
            } else if !ui.has_focus(field) {
                // Esc или клик мимо — закрываем без сохранения изменений.
                self.editing = false;
            }
        }
        if self.editing {
            if size == 1 {
                cv.line(crate::i18n::pick("ГОРОД", "CITY"), &f.caps, wc.x + 10.0, wc.w - 20.0, wc.y + 16.0, Align::Leading, dim());
                ui.text_field_ex(cv, field, Rect::new(wc.x + 8.0, wc.y + 28.0, wc.w - 16.0, 24.0), crate::i18n::pick("по IP", "by IP"), icon::SEARCH, Submit::Keep);
                cv.line(crate::i18n::pick("Enter — сохранить", "Enter to save"), &f.small_c, wc.x + 8.0, wc.w - 16.0, wc.y + 64.0, Align::Leading, dim());
                cv.line(crate::i18n::pick("пусто — по IP", "empty for IP"), &f.small_c, wc.x + 8.0, wc.w - 16.0, wc.y + 78.0, Align::Leading, mix(dim(), crate::ui::card(), 0.35));
            } else {
                // В компактных карточках 2×1 и 4×1 подсказки не помещаются по высоте.
                ui.text_field_ex(cv, field, Rect::new(wc.x + 8.0, wc.cy() - 12.0, wc.w - 16.0, 24.0), crate::i18n::pick("по IP", "by IP"), icon::SEARCH, Submit::Keep);
            }
            return;
        }

        let (hover, clicked) = ui.click_area(id(TODAY, ID_CARD), wc);
        if clicked {
            self.editing = true;
            ui.set_field_text(field, &self.city);
            ui.focus = Some(field);
            ui.caret_on = true;
        }
        let hv = ui.hover_of(id(TODAY, ID_CARD));
        if hover || hv > 0.01 {
            // Мягкая подсветка фона карточки при наведении.
            cv.rr(wc, crate::ui::card_radius(), D2D1_COLOR_F { a: 0.04 * hv, ..fg() });
        }

        // Данные погоды: значок, температура без "C", описание и город.
        let (glyph, col, temp, tcol, desc, city) = match &self.online.weather {
            Some(wt) => {
                let (g, col) = if wt.code <= 1 {
                    (icon::SUN, crate::ui::hue(crate::ios::SysColor::Yellow, SUN_COLOR))
                } else {
                    (icon::CLOUD, mix(dim(), txt(), 0.5))
                };
                (g, col, format_temp(wt.temp), txt(), crate::i18n::tr(online::weather_text(wt.code)).to_string(), wt.city.clone())
            }
            None => {
                let note = self.online.weather_note.as_deref().unwrap_or("Загрузка…");
                (icon::CLOUD, dim(), "—".to_string(), dim(), crate::i18n::tr(note).to_string(), String::new())
            }
        };
        let desc_col = if self.online.weather.is_some() { txt() } else { dim() };

        match size {
            // 2×1 (104×48): значок слева ~26px (отступ 10), температура ~20px bold, под ней город.
            0 => {
                let icon_sz = 26.0;
                let icon_r = Rect::new(wc.x + 10.0, wc.cy() - icon_sz / 2.0, icon_sz, icon_sz);
                cv.symbol(glyph, icon_r, icon_sz, col);

                let tx = wc.x + 10.0 + icon_sz + 6.0;
                let tw = (wc.right() - 8.0 - 12.0 * hv - tx).max(0.0);

                cv.digits(&temp, tx, tw, wc.y + 17.0, 20.0, 600, Align::Leading, tcol);

                // Если погода загружена — город, иначе статус загрузки/ошибки.
                let city_line = if !city.is_empty() {
                    &city
                } else if !self.city.is_empty() {
                    &self.city
                } else if self.online.weather.is_none() {
                    &desc
                } else {
                    "—"
                };
                cv.line(city_line, &f.small, tx, tw, wc.y + 35.0, Align::Leading, dim());
            }

            // 2×2 (104×104): сверху PIN (~10px) и город (f.small_b, txt); справа значок ~30px;
            // внизу слева температура ~38px weight 600, под ней описание (f.small_b, txt).
            1 => {
                let icon_sz = 30.0;
                let icon_r = Rect::new(wc.right() - 10.0 - icon_sz, wc.y + 10.0, icon_sz, icon_sz);
                // При наведении значок мягко уступает место иконке карандаша.
                let icon_col = D2D1_COLOR_F { a: col.a * (1.0 - 0.7 * hv), ..col };
                cv.symbol(glyph, icon_r, icon_sz, icon_col);

                let top_cy = wc.y + 19.0;
                let pin_sz = 10.0;
                cv.symbol(icon::PIN, Rect::new(wc.x + 10.0, top_cy - pin_sz / 2.0, pin_sz, pin_sz), pin_sz, readable(status_blue(), 45.0));

                let city_name = if !city.is_empty() {
                    &city
                } else if !self.city.is_empty() {
                    &self.city
                } else {
                    "—"
                };
                let city_col = if self.online.weather.is_some() { txt() } else { dim() };
                let city_x = wc.x + 10.0 + pin_sz + 3.0;
                let city_w = (icon_r.x - 4.0 - city_x).max(10.0);
                cv.line(city_name, &f.small_b, city_x, city_w, top_cy, Align::Leading, city_col);

                let temp_cy = wc.y + 57.0;
                let temp_w = (wc.w - 20.0).max(10.0);
                cv.digits(&temp, wc.x + 10.0, temp_w, temp_cy, 38.0, 600, Align::Leading, tcol);

                let desc_cy = wc.bottom() - 15.0;
                cv.line(&desc, &f.small_b, wc.x + 10.0, wc.w - 20.0, desc_cy, Align::Leading, desc_col);
            }

            // 4×1 (216×48): слева значок ~28px, температура ~24px bold, описание (f.body, txt),
            // справа город с PIN (f.small, dim), всё по центру cy; город обрезается первым.
            _ => {
                let cy = wc.cy();
                let icon_sz = 28.0;
                cv.symbol(glyph, Rect::new(wc.x + 10.0, cy - icon_sz / 2.0, icon_sz, icon_sz), icon_sz, col);

                let temp_x = wc.x + 10.0 + icon_sz + 6.0;
                let tw = cv.digits_w(&temp, 24.0, 600);
                cv.digits(&temp, temp_x, tw + 2.0, cy, 24.0, 600, Align::Leading, tcol);

                let desc_x = temp_x + tw + 8.0;
                let right_x = wc.right() - 10.0 - 12.0 * hv;
                let total_space = (right_x - desc_x).max(0.0);

                let desc_needed = cv.text_w(&desc, &f.body);
                let desc_w = desc_needed.min(total_space);
                cv.line(&desc, &f.body, desc_x, desc_w, cy, Align::Leading, desc_col);

                let city_name = if !city.is_empty() {
                    &city
                } else if !self.city.is_empty() {
                    &self.city
                } else {
                    "—"
                };
                let pin_sz = 10.0;
                let pin_gap = 3.0;
                let city_text_w = cv.text_w(city_name, &f.small);
                let full_city_w = pin_sz + pin_gap + city_text_w;

                let mid_gap = 8.0;
                let space_for_city = (right_x - (desc_x + desc_w + mid_gap)).max(0.0);
                if space_for_city >= pin_sz + pin_gap + 4.0 {
                    if space_for_city >= full_city_w {
                        let city_block_x = right_x - full_city_w;
                        cv.symbol(icon::PIN, Rect::new(city_block_x, cy - pin_sz / 2.0, pin_sz, pin_sz), pin_sz, dim());
                        cv.line(city_name, &f.small, city_block_x + pin_sz + pin_gap, city_text_w + 2.0, cy, Align::Leading, dim());
                    } else {
                        let city_block_x = desc_x + desc_w + mid_gap;
                        cv.symbol(icon::PIN, Rect::new(city_block_x, cy - pin_sz / 2.0, pin_sz, pin_sz), pin_sz, dim());
                        let rem_text_w = space_for_city - (pin_sz + pin_gap);
                        cv.line(city_name, &f.small, city_block_x + pin_sz + pin_gap, rem_text_w, cy, Align::Leading, dim());
                    }
                }
            }
        }

        // Карандаш редактирования поверх содержимого в правом верхнем углу.
        if hover || hv > 0.01 {
            cv.glyph(icon::EDIT, Rect::new(wc.right() - 18.0, wc.y + 3.0, 14.0, 14.0), false, D2D1_COLOR_F { a: hv, ..dim() });
        }
    }
}
