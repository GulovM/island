//! Холст Direct2D: примитивы рисования, текст, градиенты, изображения.

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows_numerics::Matrix3x2;

use crate::gfx::Gfx;
use super::color::{bare, card, chip_bg, hairline, track, txt};
use super::fonts::{cover_transform, ellipsize_units, Align};
use super::geom::{pt, Rect};

/// Рисование одного слоя содержимого: общая прозрачность и акцентный цвет.
pub struct Canvas<'a> {
    pub g: &'a Gfx,
    pub alpha: f32,
    pub accent: D2D1_COLOR_F,
}

impl Canvas<'_> {
    fn col(&self, mut c: D2D1_COLOR_F) -> D2D1_COLOR_F {
        c.a *= self.alpha;
        c
    }

    pub unsafe fn rr(&self, r: Rect, radius: f32, c: D2D1_COLOR_F) {
        self.g.brush.SetColor(&self.col(c));
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        self.g.dc.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad },
            &self.g.brush,
        );
    }

    pub unsafe fn rr_line(&self, r: Rect, radius: f32, c: D2D1_COLOR_F, width: f32) {
        self.g.brush.SetColor(&self.col(c));
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        self.g.dc.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT { rect: r.inset(width / 2.0).d2d(), radiusX: rad, radiusY: rad },
            &self.g.brush,
            width,
            None,
        );
    }

    pub unsafe fn circle(&self, cx: f32, cy: f32, radius: f32, c: D2D1_COLOR_F) {
        self.g.brush.SetColor(&self.col(c));
        self.g.dc.FillEllipse(
            &D2D1_ELLIPSE { point: pt(cx, cy), radiusX: radius, radiusY: radius },
            &self.g.brush,
        );
    }

    /// Текст, увеличенный в `k` раз вокруг центра рамки `r` (алфавитная полоса Apps).
    pub unsafe fn text_scaled(&self, s: &str, r: Rect, f: &IDWriteTextFormat, c: D2D1_COLOR_F, k: f32) {
        if (k - 1.0).abs() < 0.01 {
            return self.text(s, r, f, c);
        }
        let mut base = Matrix3x2::default();
        self.g.dc.GetTransform(&mut base);
        let (tx, ty) = (r.cx() * (1.0 - k), r.cy() * (1.0 - k));
        self.g.dc.SetTransform(&Matrix3x2 {
            M11: k * base.M11,
            M12: k * base.M12,
            M21: k * base.M21,
            M22: k * base.M22,
            M31: tx * base.M11 + ty * base.M21 + base.M31,
            M32: tx * base.M12 + ty * base.M22 + base.M32,
        });
        self.text(s, r, f, c);
        self.g.dc.SetTransform(&base);
    }

    pub unsafe fn text(&self, s: &str, r: Rect, f: &IDWriteTextFormat, c: D2D1_COLOR_F) {
        if s.is_empty() {
            return;
        }
        self.g.brush.SetColor(&self.col(c));
        if let Some(layout) = self.g.fonts.layout(s, f, r.w, r.h) {
            self.g.dc.DrawTextLayout(pt(r.x, r.y), &layout, &self.g.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            return;
        }
        crate::sys::with_utf16(s, |wide| {
            self.g.dc.DrawText(
                wide,
                f,
                &r.d2d(),
                &self.g.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        });
    }

    /// Жирный абзац по центру с переносом (лишнее обрезается).
    pub unsafe fn text_wrap_c(&self, s: &str, r: Rect, c: D2D1_COLOR_F) {
        self.text(s, r, &self.g.fonts.wrap_b, c);
    }

    /// Скруглённый прямоугольник с вертикальным градиентом (обложка трека в плеере).
    pub unsafe fn gradient_rr(&self, r: Rect, radius: f32, top: D2D1_COLOR_F, bottom: D2D1_COLOR_F) {
        let stops = [(0.0, self.col(top)), (1.0, self.col(bottom))];
        let Some(br) = self.g.linear_gradient(pt(r.x, r.y), pt(r.right(), r.bottom()), &stops) else { return };
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        self.g.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad }, &br);
    }

    /// Скруглённый прямоугольник с линейным градиентом из нескольких остановок (палитра цвета):
    /// `horizontal` — слева направо, иначе сверху вниз. Цвета могут быть полупрозрачными.
    pub unsafe fn gradient_stops(&self, r: Rect, radius: f32, stops: &[(f32, D2D1_COLOR_F)], horizontal: bool) {
        let list: Vec<(f32, D2D1_COLOR_F)> = stops.iter().map(|&(p, c)| (p, self.col(c))).collect();
        let (a, b) = if horizontal { (pt(r.x, r.y), pt(r.right(), r.y)) } else { (pt(r.x, r.y), pt(r.x, r.bottom())) };
        let Some(br) = self.g.linear_gradient(a, b, &list) else { return };
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        self.g.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad }, &br);
    }

    /// Ширина строки в логических пикселях по формату `f` (раскладка из кэша); при отказе DirectWrite —
    /// грубая оценка по числу символов.
    /// Сколько строк займёт `s` при переносе по словам в ширину `w` и какова высота строки.
    pub unsafe fn wrapped(&self, s: &str, f: &IDWriteTextFormat, w: f32) -> (usize, f32) {
        if let Some(l) = self.g.fonts.layout(s, f, w, 10_000.0) {
            let mut m = DWRITE_TEXT_METRICS::default();
            if l.GetMetrics(&mut m).is_ok() && m.lineCount > 0 {
                return (m.lineCount as usize, m.height / m.lineCount as f32);
            }
        }
        (1, 11.0)
    }

    /// Текст, не длиннее `max_lines` строк при переносе в ширину `w`: лишнее заменяется «…».
    pub unsafe fn wrap_clip<'s>(&self, s: &'s str, f: &IDWriteTextFormat, w: f32, max_lines: usize) -> std::borrow::Cow<'s, str> {
        let Some(l) = self.g.fonts.layout(s, f, w, 10_000.0) else { return s.into() };
        let mut m = DWRITE_TEXT_METRICS::default();
        if l.GetMetrics(&mut m).is_err() || (m.lineCount as usize) <= max_lines {
            return s.into();
        }
        let mut lines = vec![DWRITE_LINE_METRICS::default(); m.lineCount as usize];
        let mut n = 0u32;
        if l.GetLineMetrics(Some(&mut lines), &mut n).is_err() {
            return s.into();
        }
        let units: u32 = lines.iter().take(max_lines).map(|x| x.length).sum();
        ellipsize_units(s, units as usize).into()
    }

    /// Как `text`, но с переключаемым зачёркиванием (для выполненных задач).
    pub unsafe fn text_strike(&self, s: &str, r: Rect, f: &IDWriteTextFormat, c: D2D1_COLOR_F, strike: bool) {
        if s.is_empty() {
            return;
        }
        self.g.brush.SetColor(&self.col(c));
        if let Some(layout) = self.g.fonts.layout(s, f, r.w, r.h) {
            // Раскладка общая из кэша: признак ставится явно при каждой отрисовке.
            let range = DWRITE_TEXT_RANGE { startPosition: 0, length: s.encode_utf16().count() as u32 };
            let _ = layout.SetStrikethrough(strike, range);
            self.g.dc.DrawTextLayout(pt(r.x, r.y), &layout, &self.g.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
    }

    /// Крупные цифры формата `Fonts::display`: `cy` — середина самих цифр (а не строки), поэтому
    /// числа разных кеглей ровно встают по одной оси. `x`/`w` — полоса для выравнивания.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn digits(&self, s: &str, x: f32, w: f32, cy: f32, size: f32, weight: u32, align: Align, c: D2D1_COLOR_F) {
        let f = self.g.fonts.display(size, weight, align);
        // Строка Segoe UI Variable ≈ 1,33 кегля, середина цифр — на 0,73 кегля от её верха.
        self.text(s, Rect::new(x, cy - 0.73 * size, w, 1.33 * size), &f, c);
    }

    /// Ширина строки цифр формата `Fonts::display(size, weight)`.
    pub unsafe fn digits_w(&self, s: &str, size: f32, weight: u32) -> f32 {
        self.text_w(s, &self.g.fonts.display(size, weight, Align::Leading))
    }

    /// Строка формата `f` (выровненного по левому краю) с серединой на `cy`, выровненная в полосе
    /// `x..x + w`; не влезающее обрезается многоточием. Возвращает ширину текста.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn line(&self, s: &str, f: &IDWriteTextFormat, x: f32, w: f32, cy: f32, align: Align, c: D2D1_COLOR_F) -> f32 {
        let tw = (self.text_w(s, f) + 2.0).min(w.max(0.0));
        let left = match align {
            Align::Leading => x,
            Align::Center => x + (w - tw) / 2.0,
            Align::Trailing => x + w - tw,
        };
        let h = f.GetFontSize() * 1.6;
        self.text(s, Rect::new(left, cy - h / 2.0, tw, h), f, c);
        tw
    }

    pub unsafe fn text_w(&self, s: &str, f: &IDWriteTextFormat) -> f32 {
        if let Some(l) = self.g.fonts.layout(s, f, 10_000.0, 100.0) {
            let mut m = DWRITE_TEXT_METRICS::default();
            if l.GetMetrics(&mut m).is_ok() {
                return m.widthIncludingTrailingWhitespace.ceil();
            }
        }
        let tr_s = crate::i18n::tr(s);
        tr_s.chars().count() as f32 * 4.6
    }

    /// Ширина пилюльки (чипа) по тексту: измеренная ширина шрифтом `small_c` + постоянный отступ `CHIP_PAD`.
    pub unsafe fn chip_w(&self, label: &str) -> f32 {
        super::chip_width(self.text_w(label, &self.g.fonts.small_c))
    }

    /// Плашка клавиши («Ctrl P») и подпись после неё. Возвращает общую ширину.
    pub unsafe fn keycap(&self, x: f32, cy: f32, key: &str, label: &str) -> f32 {
        let kw = self.text_w(key, &self.g.fonts.small_c) + 9.0;
        let k = Rect::new(x, cy - 7.0, kw, 14.0);
        self.rr(k, 3.5, chip_bg());
        self.text(key, k, &self.g.fonts.small_c, txt());
        let lw = self.text_w(label, &self.g.fonts.small) + 4.0;
        // Подпись без плашки, прямо на теле острова.
        self.text(label, Rect::new(k.right() + 4.0, cy - 7.0, lw + 10.0, 14.0), &self.g.fonts.small, bare());
        kw + 4.0 + lw
    }

    /// Картинка без обрезки в прямоугольнике `r` (иконки приложений).
    pub unsafe fn bitmap(&self, bmp: &ID2D1Bitmap1, r: Rect, opacity: f32) {
        self.g.dc.DrawBitmap(
            bmp,
            Some(&r.d2d()),
            opacity * self.alpha,
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            None,
            None,
        );
    }

    /// Картинка в параллелограмме со скруглёнными углами: верхняя грань сдвинута вправо на
    /// `slant` (карусель обоев).
    pub unsafe fn bitmap_quad(&self, bmp: Option<&ID2D1Bitmap1>, r: Rect, slant: f32, opacity: f32) {
        let Ok(geo) = self.g.d2d.CreatePathGeometry() else { return };
        let Ok(sink) = geo.Open() else { return };
        let corners = [
            (r.x + slant, r.y),
            (r.right() + slant, r.y),
            (r.right(), r.bottom()),
            (r.x, r.bottom()),
        ];
        let rad = 8.0f32.min(r.w / 3.0).min(r.h / 3.0);
        // Точка на отрезке a→b на расстоянии `d` от a.
        let toward = |a: (f32, f32), b: (f32, f32), d: f32| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            pt(a.0 + dx / len * d, a.1 + dy / len * d)
        };
        for k in 0..4 {
            let c = corners[k];
            let prev = corners[(k + 3) % 4];
            let next = corners[(k + 1) % 4];
            let a = toward(c, prev, rad);
            let b = toward(c, next, rad);
            if k == 0 {
                sink.BeginFigure(a, D2D1_FIGURE_BEGIN_FILLED);
            } else {
                sink.AddLine(a);
            }
            sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: pt(c.0, c.1), point2: b });
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        if sink.Close().is_err() {
            return;
        }
        let size = bmp.map(|b| b.GetPixelSize());
        match (bmp, size) {
            (Some(b), Some(sz)) if sz.width > 0 && sz.height > 0 => {
                let Ok(br) = self.g.dc.CreateBitmapBrush(b, None, None) else { return };
                // Кубическая фильтрация: линейная при сильном уменьшении (узкие карточки карусели) рябит.
                br.SetInterpolationMode1(D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC);
                br.SetOpacity(opacity * self.alpha);
                // Картинка не сжимается по одной оси: масштаб «cover», лишнее по краям обрезается.
                let (s, dx, dy) = cover_transform(r.w + slant, r.h, sz.width as f32, sz.height as f32);
                br.SetTransform(&Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: r.x + dx, M32: r.y + dy });
                self.g.dc.FillGeometry(&geo, &br, None);
            }
            _ => {
                let mut c = card();
                c.a *= opacity * self.alpha;
                self.g.brush.SetColor(&c);
                self.g.dc.FillGeometry(&geo, &self.g.brush, None);
            }
        }
    }

    /// Значок по центру прямоугольника: векторный (`symbols`), для неизвестного кода — глиф шрифта.
    pub unsafe fn glyph(&self, g: &str, r: Rect, big: bool, c: D2D1_COLOR_F) {
        let size = if big { super::ICON_BIG } else { super::ICON };
        self.symbol(g, r, size, c);
    }

    /// Значок кеглем `size` (не больше прямоугольника `r`).
    pub unsafe fn symbol(&self, g: &str, r: Rect, size: f32, c: D2D1_COLOR_F) {
        self.g.brush.SetColor(&self.col(c));
        if super::symbols::draw(self.g, g, r, size.min(r.w).min(r.h)) {
            return;
        }
        let f = if size > super::ICON { &self.g.fonts.icon_big } else { &self.g.fonts.icon };
        self.text(g, r, f, c);
    }

    /// Дуга-кольцо: `frac` 0..1 по часовой стрелке от верхней точки, поверх серой дорожки.
    pub unsafe fn ring(&self, cx: f32, cy: f32, radius: f32, thick: f32, frac: f32, c: D2D1_COLOR_F) {
        let frac = frac.clamp(0.0, 1.0);
        let ell = D2D1_ELLIPSE { point: pt(cx, cy), radiusX: radius, radiusY: radius };
        self.g.brush.SetColor(&self.col(track()));
        self.g.dc.DrawEllipse(&ell, &self.g.brush, thick, None);
        if frac < 0.004 {
            return;
        }
        self.g.brush.SetColor(&self.col(c));
        if frac > 0.996 {
            self.g.dc.DrawEllipse(&ell, &self.g.brush, thick, None);
            return;
        }
        let a = frac * std::f32::consts::TAU;
        let end = pt(cx + radius * a.sin(), cy - radius * a.cos());
        let Ok(geo) = self.g.d2d.CreatePathGeometry() else { return };
        let Ok(sink) = geo.Open() else { return };
        sink.BeginFigure(pt(cx, cy - radius), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: end,
            size: D2D_SIZE_F { width: radius, height: radius },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: if frac > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
        });
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        if sink.Close().is_ok() {
            self.g.dc.DrawGeometry(&geo, &self.g.brush, thick, &self.g.round);
        }
    }

    pub unsafe fn card(&self, r: Rect) {
        let rad = card_radius();
        self.rr(r, rad, card());
        self.hairline(r, rad);
    }

    /// Волосяная граница 1 px: рисуется только при включённом стекле.
    pub unsafe fn hairline(&self, r: Rect, radius: f32) {
        if crate::theme::glass() {
            self.rr_line(r, radius, hairline(), 1.0);
        }
    }

    /// Обводка дуги окружности: углы в градусах от верхней точки по часовой стрелке
    /// (как у `ring`), концы круглые. Дуга короче 360°.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn stroke_arc(&self, cx: f32, cy: f32, radius: f32, from_deg: f32, to_deg: f32, thick: f32, c: D2D1_COLOR_F) {
        let at = |deg: f32| {
            let a = deg.to_radians();
            pt(cx + radius * a.sin(), cy - radius * a.cos())
        };
        let Ok(geo) = self.g.d2d.CreatePathGeometry() else { return };
        let Ok(sink) = geo.Open() else { return };
        sink.BeginFigure(at(from_deg), D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: at(to_deg),
            size: D2D_SIZE_F { width: radius, height: radius },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: if to_deg - from_deg > 180.0 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
        });
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        if sink.Close().is_ok() {
            self.g.brush.SetColor(&self.col(c));
            self.g.dc.DrawGeometry(&geo, &self.g.brush, thick, &self.g.round);
        }
    }

    /// Линия с круглыми концами.
    pub unsafe fn stroke_line(&self, a: (f32, f32), b: (f32, f32), thick: f32, c: D2D1_COLOR_F) {
        self.g.brush.SetColor(&self.col(c));
        self.g.dc.DrawLine(pt(a.0, a.1), pt(b.0, b.1), &self.g.brush, thick, &self.g.round);
    }

    pub unsafe fn progress(&self, r: Rect, frac: f32, c: D2D1_COLOR_F) {
        self.rr(r, r.h / 2.0, track());
        let fw = (r.w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { r.h } else { 0.0 });
        if fw > 0.0 {
            self.rr(Rect::new(r.x, r.y, fw, r.h), r.h / 2.0, c);
        }
    }
}

/// Радиус скругления стандартных карточек по спецификации iOS 26 в масштабе острова:
/// `(ios::R_MEDIUM * ios::island_scale()).round() = 14.0`.
pub fn card_radius() -> f32 {
    (crate::ios::R_MEDIUM * crate::ios::island_scale()).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_card_radius() {
        assert_eq!(card_radius(), 14.0);
        let expected = (crate::ios::R_MEDIUM * crate::ios::island_scale()).round();
        assert_eq!(card_radius(), expected);
    }
}
