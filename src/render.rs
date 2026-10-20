use super::*;

impl App {
    /// Тулкит и состояние вкладок на время кадра выносятся из `self`, чтобы рисовать с `&self`.
    pub(crate) unsafe fn render(&mut self) -> Result<()> {
        // Окно спрятано на время полноэкранной игры: кадры никто не увидит, GPU игре нужнее.
        if self.hidden {
            return Ok(());
        }
        let mut ui = std::mem::take(&mut self.ui);
        let mut tabs = std::mem::take(&mut self.tabs);
        let r = self.render_frame(&mut ui, &mut tabs);
        self.ui = ui;
        self.tabs = tabs;
        r
    }

    pub(crate) unsafe fn render_frame(&self, ui: &mut Ui, tabs: &mut tabs::Tabs) -> Result<()> {
        let Some(g) = &self.gfx else { return Ok(()) };
        let t0 = Instant::now();
        let s = self.scale;
        let aw = self.w.value.max(1.0);
        let ah = self.h.value.max(1.0);
        let x0 = (self.win_w as f32 - aw) / 2.0;
        let open = self.open.value.clamp(0.0, 1.0);
        let accent = self.accent();
        let collapsed_h = self.collapsed().1;

        let img: Option<ID2D1Image> = g.target.as_ref().and_then(|b| b.cast().ok());
        g.dc.SetTarget(img.as_ref());
        g.dc.BeginDraw();
        g.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

        // «Ушки» уменьшаются вместе с высотой, чтобы в полоске их не было.
        let ear = (EAR * s * (ah / (collapsed_h * s)).min(1.0)).min(ah);
        let ear = if ear < 0.5 { 0.0 } else { ear };
        // Скругление только снизу: из настроек, в свёрнутом виде чуть меньше (0,72 → 1 при раскрытии).
        let r = (self.cfg.corner_radius * (0.72 + 0.28 * open) * s).min(aw / 2.0).min(ah - ear).max(0.0);
        // По мере сжатия в полоску цвет уходит от фона к чистому чёрному.
        let thin = ((collapsed_h * s - ah) / ((collapsed_h - STRIP_H) * s)).clamp(0.0, 1.0);
        let k = 1.0 - thin;
        let pal = theme::get();
        // Прозрачность стекла: 0 — почти прозрачное (виден размытый фон), 100 — сплошное тело. Без размытия
        // (выключены «Эффекты прозрачности», нет blur-behind), в свёрнутом виде и в полоске тело сплошное.
        let real = self.cfg.glass && self.transparency && self.blur_ok;
        let body_a = if real { 1.0 - (1.0 - (0.20 + 0.80 * self.cfg.glass_opacity / 100.0)) * open * k } else { 1.0 };
        g.brush.SetColor(&D2D1_COLOR_F { r: pal.bg.r * k, g: pal.bg.g * k, b: pal.bg.b * k, a: body_a });
        self.apply_blur(x0, aw, ah, r, real && open > 0.01 && k > 0.01);
        let body = self.body_path(g, x0, aw, ah, ear, r, false)?;
        g.dc.FillGeometry(&body, &g.brush, None);

        // Стекло раскрытого острова: осветление сверху и блик на кромке (слева-сверху). Свёрнутая «чёлка»
        // и полоска остаются плоскими: множитель `open · k` гасит эффект вместе с раскрытием.
        let glass = if self.cfg.glass { open * k } else { 0.0 };
        if glass > 0.01 {
            let fade = |c: D2D1_COLOR_F, a: f32| D2D1_COLOR_F { a: c.a * a, ..c };
            if let Some(br) = gradient_brush(g, (0.0, 0.0), (0.0, ah * 0.45), &[(0.0, fade(pal.sheen, glass)), (1.0, fade(pal.sheen, 0.0))]) {
                g.dc.FillGeometry(&body, &br, None);
            }
            // Линия идёт по центру контура, поэтому контур сдвинут внутрь на полтолщины.
            let half = 0.6 * s;
            let rim = self.body_path(g, x0 + half, aw - 2.0 * half, ah - half, ear, crate::ui::inner_radius(r, half), true)?;
            let stops = [
                (0.00, fade(pal.rim_hi, glass)),
                (0.35, fade(pal.rim_mid, glass)),
                (0.65, fade(pal.rim_mid, glass)),
                (1.00, fade(pal.rim_lo, glass)),
            ];
            if let Some(br) = gradient_brush(g, (x0, 0.0), (x0 + aw, ah), &stops) {
                g.dc.DrawGeometry(&rim, &br, 2.0 * half, None);
            }
        }

        // Пока тянут ползунок радиуса, остров «засвечивается»: заливка подкрашивается акцентом, а по бокам
        // и снизу идёт светящийся контур — по нему хорошо видно, как меняется скругление.
        let glow = self.glow.value.clamp(0.0, 1.0);
        if glow > 0.01 {
            let mut c = accent;
            c.a = 0.12 * glow;
            g.brush.SetColor(&c);
            g.dc.FillGeometry(&body, &g.brush, None);
            let outline = self.body_path(g, x0, aw, ah, ear, r, true)?;
            for (width, alpha) in [(9.0, 0.07), (6.0, 0.12), (3.5, 0.24), (1.6, 0.95)] {
                c.a = alpha * glow;
                g.brush.SetColor(&c);
                g.dc.DrawGeometry(&outline, &g.brush, width * s, None);
            }
        }

        let mut cp = POINT::default();
        let _ = GetCursorPos(&mut cp);
        let cursor = ((cp.x - self.win_x) as f32 / s, (cp.y - self.win_y) as f32 / s);
        ui.begin_frame(cursor, self.expanded && self.cursor_inside());

        // Индикатор свёрнутой формы: мини-плеер и уведомление проявляются и гаснут пружинами,
        // раскрытие панели их гасит (множитель 1 − open). Раскладка — по целевому размеру формы,
        // чтобы содержимое не «сплющивалось» вместе с высотой острова.
        let live_open = 1.0 - open;
        let centre = self.win_w as f32 / s / 2.0;
        // Что рисуем в свёрнутом виде: мини-плеер, уведомление или «лицо» (раскладка, время, Wi-Fi).
        #[derive(PartialEq)]
        enum Slot {
            Mini,
            Toast,
            Face,
        }
        for (alpha, slot) in [
            (self.live.mini_alpha(), Slot::Mini),
            (self.live.toast_alpha(), Slot::Toast),
            (self.face.alpha(), Slot::Face),
        ] {
            let alpha = alpha * live_open;
            if alpha < 0.01 {
                continue;
            }
            let (fw, fh) = match slot {
                Slot::Mini => live::MINI,
                Slot::Toast => live::TOAST,
                Slot::Face => self.collapsed(),
            };
            let fw = fw.max(self.collapsed().0);
            g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
            g.dc.PushAxisAlignedClip(
                &D2D_RECT_F { left: x0, top: 0.0, right: x0 + aw, bottom: ah },
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            g.dc.SetTransform(&ui::matrix(s, 0.0, 0.0));
            let cv = Canvas { g, alpha, accent };
            let r = ui::Rect::new(centre - fw / 2.0, 0.0, fw, fh);
            match slot {
                Slot::Mini => self.live.draw_mini(ui, &cv, r),
                Slot::Toast => self.live.draw_toast(&cv, r),
                Slot::Face => self.face.draw(&cv, r),
            }
            g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
            g.dc.PopAxisAlignedClip();
        }

        if open > 0.02 {
            // Точка слева в ряду вкладок: режим полоски включён.
            if self.cfg.strip_over_maximized {
                g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
                let mut c = accent;
                c.a = open;
                g.brush.SetColor(&c);
                g.dc.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: Vector2 { X: x0 + 7.0 * s, Y: (TAB_TOP + TAB_H / 2.0) * s },
                        radiusX: 2.2 * s,
                        radiusY: 2.2 * s,
                    },
                    &g.brush,
                );
            }
            let rects = self.tab_rects();
            let pill_a = self.pill_a.value.clamp(0.0, 1.0) * open;
            if self.cfg.glass && pill_a > 0.01 {
                let (px, pw) = (self.pill_x.value * s, self.pill_w.value.max(1.0) * s);
                let (py, ph) = (TAB_TOP * s, TAB_H * s);
                let rr = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: px, top: py, right: px + pw, bottom: py + ph }, radiusX: ph / 2.0, radiusY: ph / 2.0 };
                let mut c = pal.pill;
                c.a *= pill_a;
                g.brush.SetColor(&c);
                g.dc.FillRoundedRectangle(&rr, &g.brush);
                let fade = |c: D2D1_COLOR_F| D2D1_COLOR_F { a: c.a * pill_a * 0.6, ..c };
                let stops = [(0.0, fade(pal.rim_hi)), (0.5, fade(pal.rim_mid)), (1.0, fade(pal.rim_lo))];
                if let Some(br) = gradient_brush(g, (px, py), (px + pw, py + ph), &stops) {
                    let h = 0.5 * s;
                    let line = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F { left: px + h, top: py + h, right: px + pw - h, bottom: py + ph - h },
                        radiusX: ph / 2.0 - h,
                        radiusY: ph / 2.0 - h,
                    };
                    g.dc.DrawRoundedRectangle(&line, &br, 2.0 * h, None);
                }
            }
            for (i, (tx, tw)) in rects.iter().enumerate() {
                let (px, pw) = (tx * s, tw * s);
                let (py, ph) = (TAB_TOP * s, TAB_H * s);
                let active = !self.settings_open && i == self.tab;
                let hot = cursor.0 >= *tx && cursor.0 <= tx + tw && cursor.1 >= TAB_TOP && cursor.1 <= TAB_TOP + TAB_H;
                let hv = ui.hover_amount(0xF000 + i as u32, hot && self.expanded);
                if !active && hv > 0.01 {
                    let mut c = ui::fg();
                    c.a = 0.08 * hv * open;
                    g.brush.SetColor(&c);
                    g.dc.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F { left: px, top: py, right: px + pw, bottom: py + ph },
                            radiusX: ph / 2.0,
                            radiusY: ph / 2.0,
                        },
                        &g.brush,
                    );
                }
                // Со стеклом выбранную вкладку подсвечивает одна подвижная капля (ниже), без — плашка на вкладке.
                if active && !self.cfg.glass {
                    let mut c = accent;
                    c.a = 0.26 * open;
                    g.brush.SetColor(&c);
                    g.dc.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F { left: px, top: py, right: px + pw, bottom: py + ph },
                            radiusX: ph / 2.0,
                            radiusY: ph / 2.0,
                        },
                        &g.brush,
                    );
                }
                let mut c = if active { accent } else { ui::dim() };
                c.a *= open;
                g.draw_text(crate::i18n::tab_name(self.order[i]), px, py, pw, ph, c);
            }

            // Шестерёнка настроек: при наведении поворачивается, при открытых настройках залита акцентом.
            let (gx, gy) = self.gear_center();
            let gear_hot = self.expanded && (cursor.0 - gx).hypot(cursor.1 - gy) <= GEAR_R + 3.0;
            let gv = ui.hover_amount(0xF100, gear_hot);
            let gcv = Canvas { g, alpha: open, accent };
            if self.settings_open {
                g.dc.SetTransform(&ui::matrix(s, 0.0, 0.0));
                gcv.circle(gx, gy, GEAR_R, D2D1_COLOR_F { a: 0.26, ..accent });
            }
            g.dc.SetTransform(&ui::matrix_rot(s, gx, gy, 60.0 * gv));
            let gcol = if self.settings_open { accent } else { ui::mix(ui::dim(), ui::txt(), gv) };
            gcv.glyph(ui::icon::SETTINGS, ui::Rect::new(gx - GEAR_R, gy - GEAR_R, 2.0 * GEAR_R, 2.0 * GEAR_R), false, gcol);

            // Карандаш слева: правка виджетов. Появляется только на вкладках с сеткой, в режиме правки залит акцентом.
            let cur = self.normal_id();
            let pv = ui.presence(0xF102, self.expanded && !self.settings_open && tabs.editable(cur));
            if pv > 0.01 {
                let (ex, ey) = self.pencil_center();
                let ev = ui.presence(0xF103, tabs.editing(cur));
                let hot = self.expanded && (cursor.0 - ex).hypot(cursor.1 - ey) <= GEAR_R + 3.0;
                let hv = ui.hover_amount(0xF104, hot);
                let pcv = Canvas { g, alpha: open * pv, accent };
                g.dc.SetTransform(&ui::matrix(s, 0.0, 0.0));
                if ev > 0.01 {
                    pcv.circle(ex, ey, GEAR_R, D2D1_COLOR_F { a: 0.26 * ev, ..accent });
                }
                let col = ui::mix(ui::mix(ui::dim(), ui::txt(), hv), accent, ev);
                pcv.glyph(ui::icon::EDIT, ui::Rect::new(ex - 7.0, ey - 7.0, 14.0, 14.0), false, col);
            }

            // Точка на шестерёнке: найдено обновление, его можно установить в Настройках.
            if crate::update::available() && !self.settings_open {
                g.dc.SetTransform(&ui::matrix(s, 0.0, 0.0));
                gcv.circle(gx + GEAR_R * 0.75, gy - GEAR_R * 0.75, 2.8, accent);
            }

            // Содержимое вкладок: клип по текущему телу острова, кроссфейд старой и новой.
            g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
            // При большом радиусе нижние углы карточек вылезли бы за скругление острова: обрезаем по его форме.
            let mask = if r > 16.0 * s {
                let params = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: D2D_RECT_F { left: f32::MIN, top: f32::MIN, right: f32::MAX, bottom: f32::MAX },
                    geometricMask: std::mem::ManuallyDrop::new(Some(body.cast::<ID2D1Geometry>()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: ui::matrix(1.0, 0.0, 0.0),
                    opacity: 1.0,
                    opacityBrush: std::mem::ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                };
                g.dc.PushLayer(&params, None);
                Some(params)
            } else {
                None
            };
            g.dc.PushAxisAlignedClip(
                &D2D_RECT_F {
                    left: x0 + 6.0 * s,
                    top: 31.0 * s,
                    right: x0 + aw - 6.0 * s,
                    bottom: (ah - 4.0 * s).max(31.0 * s),
                },
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            let fade = self.tab_t.value.clamp(0.0, 1.0);
            let centre = self.win_w as f32 / s / 2.0;
            let layer = |id: usize, alpha: f32, dx: f32, interactive: bool, ui: &mut Ui, tabs: &mut tabs::Tabs| {
                if alpha < 0.01 {
                    return;
                }
                let (w, h) = (self.tab_w(id), App::tab_h(id));
                let ox = centre - w / 2.0 + dx;
                g.dc.SetTransform(&ui::matrix(s, ox * s, 0.0));
                ui.begin_content(ox, interactive);
                let cv = Canvas { g, alpha, accent };
                tabs.draw(id, ui, &cv, w, h);
            };
            if let Some(prev) = self.tab_prev {
                layer(prev, open * (1.0 - fade), -self.tab_dir * SLIDE * fade, false, ui, tabs);
            }
            layer(self.cur_id(), open * fade, self.tab_dir * SLIDE * (1.0 - fade), true, ui, tabs);
            g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
            g.dc.PopAxisAlignedClip();
            if let Some(params) = mask {
                g.dc.PopLayer();
                drop(std::mem::ManuallyDrop::into_inner(params.geometricMask));
            }
        }

        g.dc.EndDraw(None, None)?;
        perf::draw_done(t0);
        g.swapchain.Present(1, DXGI_PRESENT(0)).ok()?;
        ui.end_frame();
        Ok(())
    }

    /// Подкладка стекла: окно с размытием фона точно под телом острова (по форме скруглённых нижних углов).
    unsafe fn apply_blur(&self, x0: f32, aw: f32, ah: f32, r: f32, on: bool) {
        let key = if on {
            (self.win_x + x0.floor() as i32, self.win_y, (aw.ceil() as i32) << 16 | ah.ceil() as i32, r.round() as i32, 0)
        } else {
            (0, 0, 0, 0, -1)
        };
        if self.blur_key.get() == key {
            return;
        }
        self.blur_key.set(key);
        if !on {
            let _ = ShowWindow(self.glass_wnd, SW_HIDE);
            return;
        }
        let (w, h) = (aw.ceil() as i32, ah.ceil() as i32);
        // Верхние углы уезжают выше окна, остаются скруглены только нижние.
        let d = (2 * key.3).max(1);
        let rgn = CreateRoundRectRgn(0, -d, w + 1, h + 1, d, d);
        // Система забирает регион себе, удалять его не нужно.
        SetWindowRgn(self.glass_wnd, Some(rgn), false);
        let _ = SetWindowPos(self.glass_wnd, Some(self.hwnd), key.0, key.1, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }

    /// Прячет подложку стекла (остров скрыт); следующий кадр покажет её снова.
    pub(crate) unsafe fn hide_glass(&self) {
        self.blur_key.set((i32::MIN, 0, 0, 0, 0));
        let _ = ShowWindow(self.glass_wnd, SW_HIDE);
    }

    /// Контур чёлки: плоский верх на уровне края экрана, вогнутые «ушки» по бокам,
    /// скруглённые нижние углы.
    /// `outline` — открытый контур без верхней грани (для свечения): сверху остров упирается в край экрана.
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn body_path(&self, g: &Gfx, x0: f32, aw: f32, ah: f32, ear: f32, r: f32, outline: bool) -> Result<ID2D1PathGeometry1> {
        let pt = |x: f32, y: f32| Vector2 { X: x, Y: y };
        let arc = |x: f32, y: f32, radius: f32, dir: D2D1_SWEEP_DIRECTION| D2D1_ARC_SEGMENT {
            point: pt(x, y),
            size: D2D_SIZE_F { width: radius, height: radius },
            rotationAngle: 0.0,
            sweepDirection: dir,
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        let geo = g.d2d.CreatePathGeometry()?;
        let sink = geo.Open()?;
        sink.BeginFigure(pt(x0 - ear, 0.0), if outline { D2D1_FIGURE_BEGIN_HOLLOW } else { D2D1_FIGURE_BEGIN_FILLED });
        if ear > 0.0 {
            sink.AddArc(&arc(x0, ear, ear, D2D1_SWEEP_DIRECTION_CLOCKWISE));
        }
        sink.AddLine(pt(x0, ah - r));
        if r > 0.0 {
            sink.AddArc(&arc(x0 + r, ah, r, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
        }
        sink.AddLine(pt(x0 + aw - r, ah));
        if r > 0.0 {
            sink.AddArc(&arc(x0 + aw, ah - r, r, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
        }
        sink.AddLine(pt(x0 + aw, ear));
        if ear > 0.0 {
            sink.AddArc(&arc(x0 + aw + ear, 0.0, ear, D2D1_SWEEP_DIRECTION_CLOCKWISE));
        }
        sink.EndFigure(if outline { D2D1_FIGURE_END_OPEN } else { D2D1_FIGURE_END_CLOSED });
        sink.Close()?;
        Ok(geo)
    }
}

/// Линейный градиент между двумя точками (физические пиксели окна); цвета уже с нужной прозрачностью.
unsafe fn gradient_brush(g: &Gfx, a: (f32, f32), b: (f32, f32), stops: &[(f32, D2D1_COLOR_F)]) -> Option<ID2D1LinearGradientBrush> {
    let list: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|&(position, color)| D2D1_GRADIENT_STOP { position, color }).collect();
    let coll = ID2D1RenderTarget::CreateGradientStopCollection(&g.dc, &list, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok()?;
    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2 { X: a.0, Y: a.1 }, endPoint: Vector2 { X: b.0, Y: b.1 } };
    g.dc.CreateLinearGradientBrush(&props, None, &coll).ok()
}
