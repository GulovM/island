//! Области прокрутки, инерция и визуализация полосы прокрутки.

use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED;

use crate::anim::Spring;
use super::canvas::Canvas;
use super::color::dim;
use super::geom::Rect;
use super::Ui;

pub(super) struct Scroll {
    pub(super) off: f32,
    pub(super) vel: f32,
    pub(super) max: f32,
    pub(super) content: f32,
    pub(super) view: Rect,
    pub(super) vis: Spring,
    /// Курсор над полосой или она перетаскивается: полоса остаётся видимой.
    pub(super) hot: bool,
    /// Расстояние от верха бегунка до курсора в момент захвата.
    pub(super) grab: f32,
}

/// Ширина зоны захвата полосы прокрутки у правого края области.
const SCROLLBAR_GRAB_W: f32 = 12.0;

/// Импульс прокрутки на одну «ступеньку» колеса (пикселей/с) и затухание инерции (1/с).
const WHEEL_IMPULSE: f32 = 620.0;
pub(super) const SCROLL_FRICTION: f32 = 7.0;

impl Ui {
    /// Начинает область прокрутки; возвращает смещение содержимого (пиксели вниз).
    /// Элементы рисуются с `y - offset`; `scroll_end` закрывает область.
    pub unsafe fn scroll_begin(&mut self, cv: &Canvas, id: u32, view: Rect, content_h: f32) -> f32 {
        let hov = self.interactive && view.contains(self.mouse);
        let wheel = self.wheel;
        let s = self.scrolls.entry(id).or_insert_with(|| Scroll {
            off: 0.0,
            vel: 0.0,
            max: 0.0,
            content: 0.0,
            view,
            vis: Spring::unit(0.0, 300.0, 30.0),
            hot: false,
            grab: 0.0,
        });
        s.max = (content_h - view.h).max(0.0);
        s.content = content_h;
        s.view = view;
        s.off = s.off.clamp(0.0, s.max);

        // Перетаскивание полосы: нажатие в зоне у правого края захватывает мышь раньше
        // содержимого (область рисуется до строк), дальше бегунок следует за курсором.
        let (interactive, mouse, pressed, down) = (self.interactive, self.mouse, self.pressed, self.down);
        let mut dragging = self.drag == Some(id) && down;
        s.hot = dragging;
        if s.max > 0.0 {
            let th = (view.h * view.h / content_h).max(24.0).min(view.h);
            let track = (view.h - th).max(1.0);
            let zone = Rect::new(view.right() - SCROLLBAR_GRAB_W, view.y, SCROLLBAR_GRAB_W, view.h);
            let over = interactive && zone.contains(mouse);
            if over && self.drag.is_none() {
                s.hot = true;
                if pressed {
                    let ty = view.y + track * (s.off / s.max);
                    s.grab = if mouse.1 >= ty && mouse.1 <= ty + th { mouse.1 - ty } else { th / 2.0 };
                    self.drag = Some(id);
                    dragging = true;
                }
            }
            if dragging {
                s.off = ((mouse.1 - s.grab - view.y) / track).clamp(0.0, 1.0) * s.max;
                s.vel = 0.0;
                s.vis.target = 1.0;
            }
        }
        let hov = hov && !dragging;
        if hov && wheel != 0.0 {
            s.vel -= wheel * WHEEL_IMPULSE;
            self.wheel = 0.0;
        }
        let off = s.off;
        cv.g.dc.PushAxisAlignedClip(&view.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
        self.clip = Some(view);
        off
    }

    pub unsafe fn scroll_end(&mut self, cv: &Canvas, id: u32) {
        cv.g.dc.PopAxisAlignedClip();
        self.clip = None;
        let Some(s) = self.scrolls.get(&id) else { return };
        if s.max <= 0.0 || s.vis.value < 0.01 {
            return;
        }
        let th = (s.view.h * s.view.h / s.content).max(24.0).min(s.view.h);
        let ty = s.view.y + (s.view.h - th) * (s.off / s.max);
        let mut c = dim();
        c.a = 0.7 * s.vis.value;
        cv.rr(Rect::new(s.view.right() - 4.0, ty, 3.0, th), 1.5, c);
    }

    /// Колесо мыши над прямоугольником `r` (в координатах слоя): забирает накопленное значение.
    pub fn wheel_in(&mut self, r: Rect) -> f32 {
        if self.interactive && r.contains(self.mouse) && self.wheel != 0.0 {
            std::mem::take(&mut self.wheel)
        } else {
            0.0
        }
    }
}
