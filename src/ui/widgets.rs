//! Примитивы интерфейса: кнопки, плитки, слайдеры, переключатели, чипы, палитры.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::{card, chip_bg, dim, fg, icon, ink, mix, rgb, selection, track, txt, Canvas, Rect, Ui};

/// Плитка не шире этого рисуется без текста (только значок); к `NARROW_TILE + TILE_FADE` текст
/// проявляется целиком.
const NARROW_TILE: f32 = 84.0;
const TILE_FADE: f32 = 28.0;

/// Стрелка справа на широкой плитке.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Chevron {
    None,
    Plain,
    /// С тонкой вертикальной чертой перед стрелкой (выпадающий список).
    Divided,
}

/// Результат нажатия на широкую плитку.
#[derive(Default, Clone, Copy)]
pub struct TileHit {
    pub toggled: bool,
    pub chevron: bool,
}

impl Ui {
    // --- примитивы ---

    /// Плитка-кнопка (Power): иконка сверху, жирная подпись под ней.
    pub unsafe fn button(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, label: &str) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let rect = r.inset(1.0 * pr);
        cv.rr(rect, 10.0, mix(card(), fg(), 0.055 * hv + 0.03 * pr));
        cv.hairline(rect, 10.0);
        if label.is_empty() {
            cv.glyph(glyph, rect, false, txt());
        } else if glyph.is_empty() {
            cv.text(label, rect, &cv.g.fonts.body_c, txt());
        } else {
            cv.glyph(glyph, Rect::new(rect.x, rect.cy() - 18.0, rect.w, 18.0), false, txt());
            cv.text(label, Rect::new(rect.x, rect.cy() + 1.0, rect.w, 16.0), &cv.g.fonts.body_c, txt());
        }
        clicked
    }

    /// Широкая плитка-тоггл как в макете: круглый бейдж с иконкой слева, заголовок и подпись по
    /// центру, справа может быть стрелка «⌄» (с разделителем или без). При «вкл» плитка залита
    /// акцентом с тёмным текстом. Узкая плитка (одна колонка сетки) — только бейдж по центру и
    /// узкая стрелка; длинный заголовок уменьшается до мелкого шрифта.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn tile_wide(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        glyph: &str,
        title: &str,
        sub: &str,
        on: &mut bool,
        chevron: Chevron,
    ) -> TileHit {
        // Доля «полной» плитки: при перетекании сетки ширина меняется плавно, текст гаснет вместе с ней.
        let full = ((r.w - NARROW_TILE) / TILE_FADE).clamp(0.0, 1.0);
        let chev_w = if chevron == Chevron::None { 0.0 } else { 16.0 + 8.0 * full };
        let body = Rect::new(r.x, r.y, r.w - chev_w, r.h);
        let chev = Rect::new(r.right() - chev_w, r.y, chev_w, r.h);
        let mut hit = TileHit::default();
        if chev_w > 0.0 {
            let (_, c) = self.interact(id | 0x8000, chev);
            hit.chevron = c;
        }
        let (_, clicked) = self.interact(id, body);
        if clicked {
            *on = !*on;
            hit.toggled = true;
        }
        let chev_hv = if chev_w > 0.0 { self.widget(id | 0x8000).hover.value } else { 0.0 };
        let w = self.widget(id);
        w.on.target = if *on { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);
        let rect = r.inset(0.8 * pr);
        let fill = mix(mix(card(), fg(), 0.05 * hv.max(chev_hv)), mix(cv.accent, fg(), 0.06 * hv), ov);
        cv.rr(rect, 10.0, fill);
        cv.hairline(rect, 10.0);
        let fg = mix(txt(), ink(), ov);
        let sub_c = mix(dim(), D2D1_COLOR_F { a: 1.0, ..mix(ink(), cv.accent, 0.35) }, ov);
        // Бейдж: у узкой плитки по центру свободного места, у широкой слева.
        let centered = rect.x + (rect.w - chev_w) / 2.0;
        let (bx, by) = (centered + (rect.x + 19.0 - centered) * full, rect.cy());
        cv.circle(bx, by, 11.0, mix(chip_bg(), mix(cv.accent, ink(), 0.16), ov));
        cv.glyph(glyph, Rect::new(bx - 11.0, by - 11.0, 22.0, 22.0), false, fg);
        // Текст по центру между бейджем и стрелкой.
        if full > 0.02 {
            let tcv = Canvas { g: cv.g, alpha: cv.alpha * full, accent: cv.accent };
            let tx = rect.x + 32.0;
            let tw = (rect.w - 32.0 - chev_w - 4.0).max(10.0);
            let fonts = &cv.g.fonts;
            let tf = if cv.text_w(title, &fonts.body_c) > tw { &fonts.small_c } else { &fonts.body_c };
            tcv.text(title, Rect::new(tx, rect.cy() - 12.0, tw, 13.0), tf, fg);
            tcv.text(sub, Rect::new(tx, rect.cy() + 1.0, tw, 11.0), &fonts.small_c, sub_c);
        }
        if chevron != Chevron::None {
            if chevron == Chevron::Divided {
                let mut c = fg;
                c.a = 0.22;
                cv.rr(Rect::new(chev.x, rect.y + 8.0, 1.0, rect.h - 16.0), 0.5, c);
            }
            let mut c = fg;
            c.a = 0.75 + 0.25 * chev_hv;
            cv.glyph(icon::DOWN, Rect::new(chev.x, rect.y, chev_w - 2.0, rect.h), false, c);
        }
        hit
    }

    /// Полоса-слайдер из макета: тёмная дорожка, акцентная заливка от левого края с иконкой,
    /// процент справа.
    pub unsafe fn slider_bar(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, value: &mut f32) -> bool {
        let hit = Rect::new(r.x, r.y - 4.0, r.w, r.h + 8.0);
        let _ = self.interact(id, hit);
        let mut changed = false;
        if self.is_dragging(id) {
            let v = ((self.mouse.0 - r.x) / r.w.max(1.0)).clamp(0.0, 1.0);
            if (v - *value).abs() > f32::EPSILON {
                *value = v;
                changed = true;
            }
        }
        let w = self.widget(id);
        let hv = w.hover.value;
        let v = value.clamp(0.0, 1.0);
        cv.rr(r, r.h / 2.0, mix(card(), fg(), 0.04 * hv));
        cv.hairline(r, r.h / 2.0);
        if v > 0.0 {
            let fill = Rect::new(r.x, r.y, (r.w * v).max(r.h), r.h);
            cv.rr(fill, r.h / 2.0, cv.accent);
            if crate::theme::glass() {
                // Мягкий блик сверху заполненной части.
                let top = D2D1_COLOR_F { a: 0.10, ..crate::theme::get().rim_hi };
                cv.gradient_stops(fill, r.h / 2.0, &[(0.0, top), (0.6, D2D1_COLOR_F { a: 0.0, ..top })], false);
            }
        }
        cv.glyph(glyph, Rect::new(r.x + 6.0, r.y, 18.0, r.h), false, if v > 0.04 { ink() } else { txt() });
        let col = if v > 0.93 { ink() } else { txt() };
        cv.text(&format!("{}%", (v * 100.0).round() as i32), Rect::new(r.right() - 52.0, r.y, 42.0, r.h), &cv.g.fonts.body_r, col);
        changed
    }

    /// Прозрачная кнопка-иконка (звезда, крестик): `active` красит её акцентом.
    pub unsafe fn icon_button(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, active: bool) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let mut bg = fg();
        bg.a = 0.10 * hv;
        cv.rr(r, r.h / 2.0, bg);
        let col = if active { cv.accent } else { mix(rgb(0.45, 0.45, 0.48), txt(), hv) };
        cv.glyph(glyph, r.inset(1.0 * pr), false, col);
        clicked
    }

    /// Компактная строка списка (Apps, Clipboard, задачи): квадратик с буквой или иконкой,
    /// заголовок (по левому краю или по центру), справа короткая подпись. `right_pad` — место
    /// под кнопки справа.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn row_compact(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        lead: &str,
        lead_is_glyph: bool,
        title: &str,
        right: &str,
        right_pad: f32,
        selected: bool,
        centered: bool,
    ) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        w.on.target = if selected { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);
        let mut bg = fg();
        bg.a = 0.045 * hv + 0.03 * pr;
        cv.rr(r, 8.0, bg);
        if ov > 0.01 {
            let mut c = selection(cv.accent);
            c.a *= ov;
            cv.rr(r, 8.0, c);
        }
        let ts = (r.h - 7.0).clamp(12.0, 17.0);
        let tile = Rect::new(r.x + 6.0, r.cy() - ts / 2.0, ts, ts);
        cv.rr(tile, 4.0, mix(chip_bg(), mix(cv.accent, chip_bg(), 0.45), ov));
        if lead_is_glyph {
            cv.glyph(lead, tile, false, mix(dim(), txt(), ov));
        } else {
            cv.text(lead, tile, &cv.g.fonts.small_c, txt());
        }
        let tx = tile.right() + 8.0;
        let rw = if right.is_empty() { 0.0 } else { cv.text_w(right, &cv.g.fonts.small_r) + 6.0 };
        let tw = (r.right() - tx - right_pad - rw - 8.0).max(10.0);
        let font = if centered { &cv.g.fonts.body_c } else { &cv.g.fonts.body };
        cv.text(title, Rect::new(tx, r.y, tw, r.h), font, txt());
        cv.text(right, Rect::new(r.right() - right_pad - rw - 6.0, r.y, rw, r.h), &cv.g.fonts.small_r, dim());
        clicked
    }

    /// Сплошная кнопка с подписью (Старт/Сброс, «Замерить скорость»).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn solid_button(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        label: &str,
        bg: D2D1_COLOR_F,
        fg: D2D1_COLOR_F,
        radius: f32,
    ) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let rect = r.inset(0.8 * pr);
        cv.rr(rect, radius, mix(bg, crate::ui::fg(), 0.08 * hv));
        cv.text(label, rect, &cv.g.fonts.small_c, fg);
        clicked
    }

    /// Область для самостоятельно нарисованных элементов: (наведён, нажат) с общей логикой
    /// «клик = отпускание над тем же элементом»; пружина наведения доступна по `hover_of`.
    pub fn click_area(&mut self, id: u32, r: Rect) -> (bool, bool) {
        self.interact(id, r)
    }

    /// Курсор над прямоугольником (без захвата нажатия, в отличие от `click_area`).
    pub fn hovering(&self, r: Rect) -> bool {
        self.interactive && r.contains(self.mouse)
    }

    pub fn hover_of(&mut self, id: u32) -> f32 {
        self.widget(id).hover.value
    }

    /// Чип-фильтр. Выбранный залит акцентом. `flat` — невыбранный без фона (фильтры в строке
    /// поиска). `true` — нажат.
    pub unsafe fn chip(&mut self, cv: &Canvas, id: u32, r: Rect, label: &str, selected: bool) -> bool {
        self.chip_ex(cv, id, r, label, selected, false, None)
    }

    /// Чип с выбором стиля: `tint` — цвет выбранного вместо сплошного акцента (выбранный пресет
    /// таймера: тёмно-оранжевый фон и оранжевый текст).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn chip_ex(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        label: &str,
        selected: bool,
        flat: bool,
        tint: Option<D2D1_COLOR_F>,
    ) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        w.on.target = if selected { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);
        let rect = r.inset(0.8 * pr);
        let mut base = if flat { card() } else { chip_bg() };
        base = mix(base, fg(), 0.06 * hv);
        if flat && ov < 0.01 && hv < 0.01 {
            // Плоский невыбранный: только текст.
        } else {
            let mut bg = match tint {
                Some(t) => mix(base, D2D1_COLOR_F { a: 1.0, ..mix(card(), t, 0.28) }, ov),
                None => mix(base, cv.accent, ov),
            };
            if flat {
                bg.a = bg.a.max(0.0) * (ov + hv * 0.6).min(1.0);
            }
            cv.rr(rect, rect.h / 2.0, bg);
        }
        let fg = match tint {
            Some(t) => mix(mix(dim(), txt(), hv), t, ov),
            None => mix(mix(rgb(0.78, 0.78, 0.78), txt(), hv), ink(), ov),
        };
        cv.text(label, rect, &cv.g.fonts.small_c, fg);
        clicked
    }

    /// Переключатель-«пилюля» 30×16 в прямоугольнике `r`. `enabled == false` — серый и неактивный.
    /// Возвращает `true` в кадре, когда значение переключили.
    pub unsafe fn switch(&mut self, cv: &Canvas, id: u32, r: Rect, on: &mut bool, enabled: bool) -> bool {
        let clicked = if enabled { self.interact(id, r).1 } else { false };
        if clicked {
            *on = !*on;
        }
        let w = self.widget(id);
        w.on.target = if *on { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);
        let mut bg = mix(mix(chip_bg(), fg(), 0.05 * hv), cv.accent, ov);
        let mut knob = mix(dim(), ink(), ov);
        if !enabled {
            bg.a = 0.45;
            knob.a = 0.5;
        }
        cv.rr(r, r.h / 2.0, bg);
        let kr = r.h / 2.0 - 3.0 + 0.6 * pr;
        let kx = r.x + r.h / 2.0 + (r.w - r.h) * ov;
        cv.circle(kx, r.cy(), kr, knob);
        clicked
    }

    /// Тонкий ползунок 0..1: дорожка, акцентная заливка и круглая ручка. Возвращает `true`, пока значение меняется.
    pub unsafe fn slider_thin(&mut self, cv: &Canvas, id: u32, r: Rect, value: &mut f32) -> bool {
        let hit = Rect::new(r.x - 8.0, r.y - 8.0, r.w + 16.0, r.h + 16.0);
        let _ = self.interact(id, hit);
        let mut changed = false;
        if self.is_dragging(id) {
            let v = ((self.mouse.0 - r.x) / r.w.max(1.0)).clamp(0.0, 1.0);
            if (v - *value).abs() > f32::EPSILON {
                *value = v;
                changed = true;
            }
        }
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let v = value.clamp(0.0, 1.0);
        let cy = r.cy();
        cv.rr(Rect::new(r.x, cy - 2.0, r.w, 4.0), 2.0, track());
        if v > 0.0 {
            cv.rr(Rect::new(r.x, cy - 2.0, (r.w * v).max(4.0), 4.0), 2.0, cv.accent);
        }
        let kx = r.x + r.w * v;
        cv.circle(kx, cy, 8.0 + 2.0 * hv + 1.5 * pr, D2D1_COLOR_F { a: 0.16 * (hv + pr).min(1.0), ..cv.accent });
        cv.circle(kx, cy, 5.5 + 0.8 * hv, txt());
        cv.circle(kx, cy, 2.4, cv.accent);
        changed
    }

    /// Область, которую можно тянуть мышью (палитра цвета): пока кнопка зажата, возвращает положение
    /// курсора внутри `r` в долях 0..1 (за пределами — прижато к краю).
    pub fn drag_area(&mut self, id: u32, r: Rect) -> Option<(f32, f32)> {
        let _ = self.interact(id, r);
        if self.is_dragging(id) {
            Some((
                ((self.mouse.0 - r.x) / r.w.max(1.0)).clamp(0.0, 1.0),
                ((self.mouse.1 - r.y) / r.h.max(1.0)).clamp(0.0, 1.0),
            ))
        } else {
            None
        }
    }

    /// Круглая плашка темы: фон, внутри точка акцента; выбранная обведена акцентом. `true` — нажата.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn swatch(&mut self, cv: &Canvas, id: u32, cx: f32, cy: f32, radius: f32, bg: D2D1_COLOR_F, accent: D2D1_COLOR_F, selected: bool) -> bool {
        let hit = Rect::new(cx - radius - 2.0, cy - radius - 2.0, 2.0 * radius + 4.0, 2.0 * radius + 4.0);
        let (_, clicked) = self.interact(id, hit);
        let w = self.widget(id);
        w.on.target = if selected { 1.0 } else { 0.0 };
        let (hv, ov) = (w.hover.value, w.on.value);
        let rad = radius + 1.0 * hv;
        cv.circle(cx, cy, rad + 2.0, D2D1_COLOR_F { a: ov, ..cv.accent });
        cv.circle(cx, cy, rad, chip_bg());
        cv.circle(cx, cy, rad - 1.2, bg);
        cv.circle(cx, cy, (rad - 1.2) * 0.5, accent);
        clicked
    }
}
