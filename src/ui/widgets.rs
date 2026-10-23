//! Примитивы интерфейса: кнопки, плитки, слайдеры, переключатели, чипы, палитры.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use super::{card, chip_bg, dim, fg, icon, ink, mix, selection, track, txt, Canvas, Rect, Ui};

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

/// Стандартный боковой отступ текста внутри пилюльки (чипа): 8 px слева и 8 px справа.
pub const CHIP_PAD: f32 = 16.0;

/// Ширина пилюльки (чипа) по измеренной ширине текста: ширина текста + постоянный отступ `CHIP_PAD`.
pub fn chip_width(text_w: f32) -> f32 {
    text_w + CHIP_PAD
}

/// Суммарная ширина ряда пилюлек с заданным шагом (`gap`) между ними.
pub fn chips_total_width(widths: &[f32], gap: f32) -> f32 {
    if widths.is_empty() {
        return 0.0;
    }
    widths.iter().sum::<f32>() + gap * (widths.len() - 1) as f32
}

impl Ui {
    // --- примитивы ---

    /// Плитка-кнопка (Power): иконка сверху, жирная подпись под ней.
    /// Плитка-кнопка, у которой значок окрашен цветом действия `tint` (в стиле iOS): блокировка,
    /// сон, перезагрузка и выключение различаются с первого взгляда. Подпись остаётся основным цветом.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn button_tinted(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, label: &str, tint: Option<D2D1_COLOR_F>) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let bg_rect = press_scale(r, pr);
        let rad = super::card_radius();
        cv.rr(bg_rect, rad, mix(card(), fg(), 0.055 * hv + 0.03 * pr));
        cv.hairline(bg_rect, rad);
        let mark = tint.filter(|_| super::color::is_ios_palette()).unwrap_or_else(txt);
        if label.is_empty() {
            cv.glyph(glyph, r, false, mark);
        } else if glyph.is_empty() {
            cv.text(label, r, &cv.g.fonts.body_c, txt());
        } else {
            cv.symbol(glyph, Rect::new(r.x, r.cy() - 22.0, r.w, 22.0), 17.0, mark);
            cv.text(label, Rect::new(r.x, r.cy() + 2.0, r.w, 16.0), &cv.g.fonts.body_c, txt());
        }
        clicked
    }

    /// Широкая плитка-тоггл как в макете: круглый бейдж с иконкой слева, заголовок и подпись по
    /// центру, справа может быть стрелка «⌄» (с разделителем или без). При «вкл» плитка залита
    /// акцентом с тёмным текстом. Узкая плитка (одна колонка сетки) — только бейдж по центру и
    /// узкая стрелка; длинный заголовок уменьшается до мелкого шрифта.
    /// `tint` — цвет функции во включённом состоянии (`hue`): в стиле iOS им заливается только
    /// бейдж, а плитка остаётся стеклом с лёгким оттенком (как модули Пункта управления iOS 26) —
    /// текст на любом цвете читается; в стиле Windows плитка заливается целиком.
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
        tint: D2D1_COLOR_F,
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
        let ios = super::color::is_ios_palette();
        let base = mix(card(), fg(), 0.05 * hv.max(chev_hv));
        let (fill, fg, sub_c, badge, mark) = if ios {
            let wash = D2D1_COLOR_F { a: 0.30, ..tint };
            let badge = mix(chip_bg(), mix(tint, fg(), 0.08 * hv), ov);
            (mix(base, wash, 0.5 * ov), txt(), dim(), badge, mix(txt(), super::color::on_color(tint), ov))
        } else {
            let label = mix(txt(), ink(), ov);
            let sub_c = mix(dim(), D2D1_COLOR_F { a: 1.0, ..mix(ink(), tint, 0.35) }, ov);
            (mix(base, mix(tint, fg(), 0.06 * hv), ov), label, sub_c, mix(chip_bg(), mix(tint, ink(), 0.16), ov), label)
        };
        cv.rr(rect, 10.0, fill);
        cv.hairline(rect, 10.0);
        // Бейдж: у узкой плитки по центру свободного места, у широкой слева.
        let centered = rect.x + (rect.w - chev_w) / 2.0;
        let (bx, by) = (centered + (rect.x + 19.0 - centered) * full, rect.cy());
        cv.circle(bx, by, 11.0, badge);
        cv.glyph(glyph, Rect::new(bx - 11.0, by - 11.0, 22.0, 22.0), false, mark);
        // Текст слева, сразу за бейджем (как плитки Пункта управления iOS): заголовок полужирный,
        // подпись обычным начертанием — иерархия держится весом, а не только цветом.
        if full > 0.02 {
            let tcv = Canvas { g: cv.g, alpha: cv.alpha * full, accent: cv.accent };
            let tx = rect.x + 38.0;
            let tw = (rect.w - 38.0 - chev_w - 4.0).max(10.0);
            let fonts = &cv.g.fonts;
            let tf = if cv.text_w(title, &fonts.body) > tw { &fonts.small_b } else { &fonts.body };
            tcv.text(title, Rect::new(tx, rect.cy() - 12.0, tw, 13.0), tf, fg);
            tcv.text(sub, Rect::new(tx, rect.cy() + 1.0, tw, 11.0), &fonts.small, sub_c);
        }
        if chevron != Chevron::None {
            // В узкой плитке черта зажата между значком и стрелкой: она проявляется вместе с текстом.
            if chevron == Chevron::Divided && full > 0.02 {
                let mut c = fg;
                c.a = 0.22 * full;
                cv.rr(Rect::new(chev.x, rect.y + 8.0, 1.0, rect.h - 16.0), 0.5, c);
            }
            let mut c = fg;
            c.a = 0.75 + 0.25 * chev_hv;
            cv.glyph(icon::DOWN, Rect::new(chev.x, rect.y, chev_w - 2.0, rect.h), false, c);
        }
        hit
    }

    /// Полоса-слайдер из макета: тёмная дорожка, заливка `fill` от левого края с иконкой, процент
    /// справа. Значок и процент поверх заливки — цветом, контрастным к ней (`on_color`).
    pub unsafe fn slider_bar(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, value: &mut f32, fill: D2D1_COLOR_F) -> bool {
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
        let on_fill = super::color::on_color(fill);
        if v > 0.0 {
            let fr = Rect::new(r.x, r.y, (r.w * v).max(r.h), r.h);
            cv.rr(fr, r.h / 2.0, fill);
            if crate::theme::glass() {
                // Мягкий блик сверху заполненной части.
                let top = D2D1_COLOR_F { a: 0.10, ..crate::theme::get().rim_hi };
                cv.gradient_stops(fr, r.h / 2.0, &[(0.0, top), (0.6, D2D1_COLOR_F { a: 0.0, ..top })], false);
            }
        }
        cv.glyph(glyph, Rect::new(r.x + 6.0, r.y, 18.0, r.h), false, if v > 0.04 { on_fill } else { txt() });
        let col = if v > 0.93 { on_fill } else { txt() };
        cv.text(&format!("{}%", (v * 100.0).round() as i32), Rect::new(r.right() - 52.0, r.y, 42.0, r.h), &cv.g.fonts.body_r, col);
        changed
    }

    /// Прозрачная кнопка-иконка (звезда, крестик): `active` красит её акцентом.
    pub unsafe fn icon_button(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, active: bool) -> bool {
        self.icon_button_tinted(cv, id, r, glyph, active, cv.accent)
    }

    /// Кнопка-иконка, которая во включённом состоянии красится цветом `tint` (жёлтая звезда избранного).
    pub unsafe fn icon_button_tinted(&mut self, cv: &Canvas, id: u32, r: Rect, glyph: &str, active: bool, tint: D2D1_COLOR_F) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let mut bg = fg();
        bg.a = 0.10 * hv;
        cv.rr(r, r.h / 2.0, bg);
        let col = if active { tint } else { mix(dim(), txt(), hv) };
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
        self.row_compact_tinted(cv, id, r, lead, lead_is_glyph, title, right, right_pad, selected, centered, None)
    }

    /// Как `row_compact`, но значок слева на квадратике цвета `tint` (белый значок на цветной
    /// плашке, как строки Настроек iOS): по цвету сразу видно, что это — сеть, ссылка, картинка.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn row_compact_tinted(
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
        tint: Option<D2D1_COLOR_F>,
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
        match tint.filter(|_| lead_is_glyph && super::color::is_ios_palette()) {
            Some(t) => {
                cv.rr(tile, 4.5, t);
                cv.symbol(lead, tile, ts * 0.78, super::color::on_color(t));
            }
            None => {
                cv.rr(tile, 4.0, mix(chip_bg(), mix(cv.accent, chip_bg(), 0.45), ov));
                if lead_is_glyph {
                    cv.symbol(lead, tile, ts * 0.78, mix(dim(), txt(), ov));
                } else {
                    cv.text(lead, tile, &cv.g.fonts.small_c, txt());
                }
            }
        }
        let tx = tile.right() + 8.0;
        let rw = if right.is_empty() { 0.0 } else { cv.text_w(right, &cv.g.fonts.small_r) + 6.0 };
        let tw = (r.right() - tx - right_pad - rw - 8.0).max(10.0);
        let font = if centered { &cv.g.fonts.body_c } else { &cv.g.fonts.body };
        cv.text(title, Rect::new(tx, r.y, tw, r.h), font, txt());
        // Подпись справа — вторичная (тип, время): приглушённая, чтобы не спорить с заголовком.
        cv.text(right, Rect::new(r.right() - right_pad - rw - 6.0, r.y, rw, r.h), &cv.g.fonts.small_r, dim());
        clicked
    }

    /// Сплошная кнопка с подписью (Старт/Сброс, «Замерить скорость»).
    /// `_radius` игнорируется: капсула iOS 26 (при h <= 40.0) или card_radius().
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn solid_button(
        &mut self,
        cv: &Canvas,
        id: u32,
        r: Rect,
        label: &str,
        bg: D2D1_COLOR_F,
        fg: D2D1_COLOR_F,
        _radius: f32,
    ) -> bool {
        let (_, clicked) = self.interact(id, r);
        let w = self.widget(id);
        let (hv, pr) = (w.hover.value, w.press.value);
        let bg_rect = press_scale(r, pr);
        let rad = if bg_rect.h <= 40.0 { bg_rect.h / 2.0 } else { super::card_radius() };
        cv.rr(bg_rect, rad, mix(bg, crate::ui::fg(), 0.08 * hv));
        cv.text(label, r, &cv.g.fonts.small_c, fg);
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
        let bg_rect = press_scale(r, pr);
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
            cv.rr(bg_rect, bg_rect.h / 2.0, bg);
            cv.hairline(bg_rect, bg_rect.h / 2.0);
        }
        let fg = match tint {
            Some(t) => mix(mix(dim(), txt(), hv), t, ov),
            None => mix(mix(dim(), txt(), hv), ink(), ov),
        };
        cv.text(label, r, &cv.g.fonts.small_c, fg);
        clicked
    }

    /// Переключатель-«пилюля» в прямоугольнике `r` (пропорции iOS 26: высота 31, диаметр ручки 27, отступ 2).
    /// `enabled == false` — серый и неактивный (opacity 0.40).
    /// Возвращает `true` в кадре, когда значение переключили.
    pub unsafe fn switch(&mut self, cv: &Canvas, id: u32, r: Rect, on: &mut bool, enabled: bool) -> bool {
        let clicked = if enabled { self.interact(id, r).1 } else { false };
        if clicked {
            *on = !*on;
        }
        let w = self.widget(id);
        w.on.target = if *on { 1.0 } else { 0.0 };
        let (hv, pr, ov) = (w.hover.value, w.press.value, w.on.value);

        // Цвет включённого состояния: системный зелёный iOS или текущий акцент Windows
        let on_col = super::color::switch_on_color(cv.accent);
        let off_col = mix(chip_bg(), fg(), 0.05 * hv);
        let mut bg = mix(off_col, on_col, ov);
        if !enabled {
            bg.a = 0.40;
        }
        cv.rr(r, r.h / 2.0, bg);

        // Ручка переключателя с отступом 2/31 и диаметром 27/31 от высоты дорожки
        let thumb_rect = switch_thumb_rect(r, ov, pr);
        let thumb_rad = thumb_rect.h / 2.0;

        // Мягкая тень ручки iOS (Figma: 0px 3px 8px rgba(0,0,0,0.15), 0px 3px 1px rgba(0,0,0,0.06))
        if enabled {
            let scale_h = r.h / 31.0;
            let s1_dy = 2.0 * scale_h;
            let s1_spread = 1.5 * scale_h;
            let s1 = Rect::new(
                thumb_rect.x - s1_spread,
                thumb_rect.y + s1_dy - s1_spread,
                thumb_rect.w + 2.0 * s1_spread,
                thumb_rect.h + 2.0 * s1_spread,
            );
            cv.rr(s1, s1.h / 2.0, D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.15 });

            let s2_dy = 1.0 * scale_h;
            let s2_spread = 0.5 * scale_h;
            let s2 = Rect::new(
                thumb_rect.x - s2_spread,
                thumb_rect.y + s2_dy - s2_spread,
                thumb_rect.w + 2.0 * s2_spread,
                thumb_rect.h + 2.0 * s2_spread,
            );
            cv.rr(s2, s2.h / 2.0, D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.08 });
        }

        // Белая ручка переключателя
        let mut thumb_col = super::color::WHITE;
        if !enabled {
            thumb_col.a = 0.40;
        }
        cv.rr(thumb_rect, thumb_rad, thumb_col);

        clicked
    }

    /// Тонкий ползунок 0..1: дорожка, акцентная заливка и белая круглая ручка с мягкой тенью.
    /// Возвращает `true`, пока значение меняется.
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
        // Ручка: белая круглая с мягкой тенью (вместо трёх кругов), под нажатием слегка растёт (подобрано).
        let kr = 5.5 + 0.4 * hv + 1.2 * pr;
        cv.circle(kx, cy + 1.2, kr + 1.0, D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.12 });
        cv.circle(kx, cy + 0.6, kr + 0.3, D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.15 });
        cv.circle(kx, cy, kr, super::color::WHITE);
        changed
    }

    /// Недоступный ползунок: серая дорожка и ручка, без взаимодействия.
    pub unsafe fn slider_thin_disabled(&self, cv: &Canvas, r: Rect, value: f32) {
        let dis = D2D1_COLOR_F { a: 0.45, ..dim() };
        let cy = r.cy();
        let v = value.clamp(0.0, 1.0);
        cv.rr(Rect::new(r.x, cy - 2.0, r.w, 4.0), 2.0, track());
        if v > 0.0 {
            cv.rr(Rect::new(r.x, cy - 2.0, (r.w * v).max(4.0), 4.0), 2.0, dis);
        }
        cv.circle(r.x + r.w * v, cy, 5.5, dis);
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

/// Диаметр ручки переключателя iOS (27/31 от высоты дорожки, docs/figma/toggles.md).
pub fn switch_thumb_diameter(track_h: f32) -> f32 {
    track_h * (27.0 / 31.0)
}

/// Отступ ручки переключателя iOS от края дорожки (2/31 от высоты дорожки, docs/figma/toggles.md).
pub fn switch_thumb_offset(track_h: f32) -> f32 {
    track_h * (2.0 / 31.0)
}

/// Растяжение ручки переключателя по ширине при нажатии (пружина press).
/// Подобрано: при press = 1.0 ручка вытягивается на ~3.5/31 высоты дорожки (~1.8 px при высоте 16 px).
pub fn switch_thumb_stretch(track_h: f32, press: f32) -> f32 {
    track_h * (3.5 / 31.0) * press.clamp(0.0, 1.0)
}

/// Прямоугольник ручки переключателя: диаметр 27/31 h, отступ 2/31 h.
/// При нажатии ручка слегка вытягивается по ширине в сторону центра дорожки,
/// оставаясь внутри дорожки с отступом `offset`.
pub fn switch_thumb_rect(track: Rect, on_val: f32, press: f32) -> Rect {
    let d = switch_thumb_diameter(track.h);
    let off = switch_thumb_offset(track.h);
    let stretch = switch_thumb_stretch(track.h, press);
    let w = d + stretch;
    let h = d;
    let ov = on_val.clamp(0.0, 1.0);
    // Номинальный центр без растяжения: от (track.x + off + d / 2) до (track.right() - off - d / 2)
    let nominal_cx = track.x + off + d / 2.0 + (track.w - 2.0 * off - d) * ov;
    // При растяжении ручка расширяется внутрь дорожки:
    // при ov = 0 левый край привязан к track.x + offset,
    // при ov = 1 правый край привязан к track.right() - offset.
    let cx = nominal_cx + (0.5 - ov) * stretch;
    let cy = track.cy();
    Rect::new(cx - w / 2.0, cy - h / 2.0, w, h)
}

/// Масштабирует прямоугольник относительно его центра при нажатии:
/// при `press == 0.0` масштаб равен 1.0, при `press == 1.0` масштаб уменьшается до ~0.97 (сжатие на 3%).
pub fn press_scale(r: Rect, press: f32) -> Rect {
    let scale = 1.0 - 0.03 * press.clamp(0.0, 1.0);
    let dw = r.w * (1.0 - scale);
    let dh = r.h * (1.0 - scale);
    Rect::new(r.x + dw / 2.0, r.y + dh / 2.0, r.w - dw, r.h - dh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_switch_proportions() {
        let h = 31.0;
        assert_eq!(switch_thumb_diameter(h), 27.0);
        assert_eq!(switch_thumb_offset(h), 2.0);
        assert!((2.0 * switch_thumb_offset(h) + switch_thumb_diameter(h) - h).abs() < 1e-5);

        // Для высоты острова 16.0:
        let h16 = 16.0;
        let d = switch_thumb_diameter(h16);
        let off = switch_thumb_offset(h16);
        assert!((2.0 * off + d - h16).abs() < 1e-5);
    }

    #[test]
    fn test_switch_thumb_bounds() {
        let track = Rect::new(10.0, 20.0, 51.0, 31.0);
        let off = switch_thumb_offset(track.h);
        assert_eq!(off, 2.0);

        // ov = 0, press = 0: левый край строго равен track.x + offset
        let r0_unpressed = switch_thumb_rect(track, 0.0, 0.0);
        assert_eq!(r0_unpressed.x, track.x + off);
        assert_eq!(r0_unpressed.w, 27.0);
        assert_eq!(r0_unpressed.h, 27.0);

        // ov = 0, press = 1: левый край по-прежнему на track.x + offset, а правый растянут вправо
        let r0_pressed = switch_thumb_rect(track, 0.0, 1.0);
        assert!((r0_pressed.x - (track.x + off)).abs() < 1e-5);
        assert!(r0_pressed.w > 27.0);
        assert!(r0_pressed.right() < track.right());

        // ov = 1, press = 0: правый край строго равен track.right() - offset
        let r1_unpressed = switch_thumb_rect(track, 1.0, 0.0);
        assert!((r1_unpressed.right() - (track.right() - off)).abs() < 1e-5);
        assert_eq!(r1_unpressed.w, 27.0);

        // ov = 1, press = 1: правый край по-прежнему на track.right() - offset, а левый растянут влево
        let r1_pressed = switch_thumb_rect(track, 1.0, 1.0);
        assert!((r1_pressed.right() - (track.right() - off)).abs() < 1e-5);
        assert!(r1_pressed.w > 27.0);
        assert!(r1_pressed.x > track.x);
    }

    #[test]
    fn test_press_scale() {
        let r = Rect::new(10.0, 20.0, 100.0, 40.0);

        // Без нажатия: исходный прямоугольник
        let p0 = press_scale(r, 0.0);
        assert_eq!(p0.x, r.x);
        assert_eq!(p0.y, r.y);
        assert_eq!(p0.w, r.w);
        assert_eq!(p0.h, r.h);

        // Полное нажатие (press = 1.0): сжатие на 3% (масштаб 0.97) относительно центра
        let p1 = press_scale(r, 1.0);
        assert!((p1.cx() - r.cx()).abs() < 1e-5);
        assert!((p1.cy() - r.cy()).abs() < 1e-5);
        assert!((p1.w - 97.0).abs() < 1e-5);
        assert!((p1.h - 38.8).abs() < 1e-5);
    }

    #[test]
    fn test_chip_width_and_total_width() {
        assert_eq!(CHIP_PAD, 16.0);
        assert_eq!(chip_width(40.0), 56.0);
        assert_eq!(chip_width(0.0), 16.0);

        assert_eq!(chips_total_width(&[], 4.0), 0.0);
        assert_eq!(chips_total_width(&[44.0], 4.0), 44.0);
        assert_eq!(chips_total_width(&[56.0, 44.0], 4.0), 104.0);
        assert_eq!(chips_total_width(&[10.0, 20.0, 30.0], 5.0), 70.0);
    }
}
