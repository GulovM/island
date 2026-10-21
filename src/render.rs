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

        // Прогресс раскрытия острова: согласует пружину open и фактические размеры w/h.
        let base_h = collapsed_h * s;
        // Эталон раскрытия — самая маленькая вкладка: размер текущей менялся бы при смене вкладки,
        // и содержимое с размытием мигали бы на каждом переключении.
        let content_h = (0..=tabs::SETTINGS).map(|t| tabs.island_size(t).1).fold(f32::MAX, f32::min) * s;
        let span_h = (content_h - base_h).max(1.0);
        let h_prog = ((ah - base_h) / span_h).clamp(0.0, 1.0);

        let base_w = self.collapsed().0 * s;
        let content_w = (0..=tabs::SETTINGS).map(|t| tabs.island_size(t).0).fold(f32::MAX, f32::min).max(self.tab_row_width() + TAB_SIDE_LEFT + TAB_SIDE_RIGHT) * s;
        let span_w = (content_w - base_w).max(1.0);
        let w_prog = ((aw - base_w) / span_w).clamp(0.0, 1.0);

        let raw_prog = open.min(h_prog).min(w_prog);
        const EXPAND_THRESHOLD: f32 = 0.08;
        let p = if raw_prog > EXPAND_THRESHOLD {
            ((raw_prog - EXPAND_THRESHOLD) / (1.0 - EXPAND_THRESHOLD)).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Вычисление всех слоёв тела острова: тинт фона, осветление сверху, кромка и тень.
        let real = self.real_glass();
        let bg_rgb = [pal.bg.r, pal.bg.g, pal.bg.b];
        let layers = crate::glass::body_layers(
            self.cfg.material,
            bg_rgb,
            self.cfg.glass,
            real,
            self.cfg.glass_opacity,
            p,
            k,
        );

        self.apply_blur(x0, aw, ah, ear, r, real && p > 0.001 && k > 0.01);

        // Тело острова рисуется по BodyLayers (тинт материала или фона)
        let body = self.body_path(g, x0, aw, ah, ear, r, false)?;

        // Мягкая тень стекла: ios::GLASS_SHADOW (0 8 40 rgba(0,0,0,0.12)) вниз и по бокам.
        // Верх острова упирается в край экрана (y = 0), поэтому тень не распространяется вверх.
        // Запас окна WIN_LOGICAL (800×530) при максимальном острове (700×476) оставляет снизу 54 px и по бокам 42 px,
        // поэтому полная тень (40 px по бокам и 48 px снизу) помещается целиком.
        // Адаптивное ограничение max_spread оставлено как страховка на случай нестандартных геометрий.
        // Тень вычитает геометрию тела (CombineWithGeometry с D2D1_COMBINE_MODE_EXCLUDE) через PushLayer,
        // чтобы не рисоваться под полупрозрачным телом.
        if layers.shadow.alpha > 0.005 {
            let avail_bottom = (self.win_h as f32 - ah).max(0.0);
            let avail_side = (x0 - ear).max(0.0);
            let max_spread = (avail_bottom - 2.0 * s)
                .min(avail_side - 2.0 * s)
                .clamp(2.0 * s, layers.shadow.blur_radius * s);
            let eff_dy = (layers.shadow.offset_y * s) * (max_spread / (layers.shadow.blur_radius * s));

            let max_x0 = x0 - max_spread;
            let max_aw = aw + 2.0 * max_spread;
            let max_ah = ah + eff_dy + max_spread;
            let max_ear = (ear - max_spread).max(0.0);
            let max_r = r + max_spread;

            if let Ok(outer_geo) = self.body_path(g, max_x0, max_aw, max_ah, max_ear, max_r, false) {
                let mask_geo = g.d2d.CreatePathGeometry()?;
                let sink = mask_geo.Open()?;
                outer_geo.CombineWithGeometry(&body, D2D1_COMBINE_MODE_EXCLUDE, None, D2D1_DEFAULT_FLATTENING_TOLERANCE, &sink)?;
                sink.Close()?;

                let params = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: D2D_RECT_F { left: f32::MIN, top: f32::MIN, right: f32::MAX, bottom: f32::MAX },
                    geometricMask: std::mem::ManuallyDrop::new(Some(mask_geo.cast::<ID2D1Geometry>()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: ui::matrix(1.0, 0.0, 0.0),
                    opacity: 1.0,
                    opacityBrush: std::mem::ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                };
                g.dc.PushLayer(&params, None);

                const SHADOW_RINGS: usize = 24;
                let rings = crate::glass::shadow_rings(max_spread, layers.shadow.alpha, SHADOW_RINGS);
                for &(grow, ring_alpha) in &rings {
                    let dy = eff_dy * (grow / max_spread);
                    let s_x0 = x0 - grow;
                    let s_aw = aw + 2.0 * grow;
                    let s_ah = ah + dy + grow;
                    let s_ear = (ear - grow).max(0.0);
                    let s_r = r + grow;
                    if let Ok(shadow_geo) = self.body_path(g, s_x0, s_aw, s_ah, s_ear, s_r, false) {
                        g.brush.SetColor(&D2D1_COLOR_F {
                            r: layers.shadow.color[0],
                            g: layers.shadow.color[1],
                            b: layers.shadow.color[2],
                            a: ring_alpha,
                        });
                        g.dc.FillGeometry(&shadow_geo, &g.brush, None);
                    }
                }

                g.dc.PopLayer();
                drop(std::mem::ManuallyDrop::into_inner(params.geometricMask));
            }
        }

        g.brush.SetColor(&D2D1_COLOR_F {
            r: layers.tint_color[0],
            g: layers.tint_color[1],
            b: layers.tint_color[2],
            a: layers.tint_alpha,
        });
        g.dc.FillGeometry(&body, &g.brush, None);

        // Осветление сверху (sheen)
        if layers.sheen.alpha > 0.005 {
            let top_color = D2D1_COLOR_F {
                r: layers.sheen.color[0],
                g: layers.sheen.color[1],
                b: layers.sheen.color[2],
                a: layers.sheen.alpha,
            };
            let bot_color = D2D1_COLOR_F { a: 0.0, ..top_color };
            let sheen_h = ah * layers.sheen.height_fraction;
            if let Some(br) = gradient_brush(g, (0.0, 0.0), (0.0, sheen_h), &[(0.0, top_color), (1.0, bot_color)]) {
                g.dc.FillGeometry(&body, &br, None);
            }
        }

        // Зеркальная кромка (rim): блик слева-сверху и мягкая дуга справа-снизу
        if layers.rim.hi_alpha > 0.005 || layers.rim.lo_alpha > 0.005 {
            let half = (layers.rim.width * 0.5) * s;
            let rim = self.body_path(g, x0 + half, aw - 2.0 * half, ah - half, ear, crate::ui::inner_radius(r, half), true)?;
            let stops = [
                (0.00, D2D1_COLOR_F { r: layers.rim.hi_color[0], g: layers.rim.hi_color[1], b: layers.rim.hi_color[2], a: layers.rim.hi_alpha }),
                (0.35, D2D1_COLOR_F { r: layers.rim.mid_color[0], g: layers.rim.mid_color[1], b: layers.rim.mid_color[2], a: layers.rim.mid_alpha }),
                (0.65, D2D1_COLOR_F { r: layers.rim.mid_color[0], g: layers.rim.mid_color[1], b: layers.rim.mid_color[2], a: layers.rim.mid_alpha }),
                (1.00, D2D1_COLOR_F { r: layers.rim.lo_color[0], g: layers.rim.lo_color[1], b: layers.rim.lo_color[2], a: layers.rim.lo_alpha }),
            ];
            if let Some(br) = gradient_brush(g, (x0, 0.0), (x0 + aw, ah), &stops) {
                g.dc.DrawGeometry(&rim, &br, 2.0 * half, None);
            }

            // Тонкая внутренняя подсветка по верхней половине (подобрано).
            if layers.rim.inner_highlight_alpha > 0.005 {
                let inner_inset = half + 0.8 * s; // подобрано
                if let Ok(inner_rim) = self.body_path(g, x0 + inner_inset, aw - 2.0 * inner_inset, ah - inner_inset, ear, crate::ui::inner_radius(r, inner_inset), true) {
                    let inner_stops = [
                        (0.00, D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: layers.rim.inner_highlight_alpha }),
                        (0.50, D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0 }),
                        (1.00, D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.0 }),
                    ];
                    if let Some(ibr) = gradient_brush(g, (x0, 0.0), (x0, ah * 0.6), &inner_stops) {
                        g.dc.DrawGeometry(&inner_rim, &ibr, 1.0 * s, None);
                    }
                }
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

        if p > 0.001 {
            // Точка слева в ряду вкладок: режим полоски включён.
            if self.cfg.strip_over_maximized {
                g.dc.SetTransform(&ui::matrix(1.0, 0.0, 0.0));
                let mut c = accent;
                c.a = p;
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
            let pill_a = self.pill_a.value.clamp(0.0, 1.0) * p;
            if self.cfg.glass && pill_a > 0.01 {
                let (px, pw) = (self.pill_x.value * s, self.pill_w.value.max(1.0) * s);
                let (py, ph) = (TAB_TOP * s, TAB_H * s);
                let rr = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: px, top: py, right: px + pw, bottom: py + ph }, radiusX: ph / 2.0, radiusY: ph / 2.0 };

                let is_ios = pal.style == theme::PaletteStyle::Ios;

                // Мягкая компактная тень Liquid Glass Small:
                // В стиле iOS: 0 1 4 rgba(0,0,0,0.10) (подобрано).
                // В стиле Windows: 0 2 8 rgba(0,0,0,0.18) (подобрано).
                // Рисуется ТОЛЬКО снаружи капли (вычитание геометрии через PushLayer), чтобы тень не темнила полупрозрачную каплю.
                let (shadow_dy, shadow_blur, shadow_base_alpha) = if is_ios {
                    (1.0 * s, 4.0 * s, 0.10)
                } else {
                    (2.0 * s, 8.0 * s, 0.18)
                };
                let outer_rr = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: px - shadow_blur,
                        top: py - shadow_blur,
                        right: px + pw + shadow_blur,
                        bottom: py + ph + shadow_dy + shadow_blur,
                    },
                    radiusX: ph / 2.0 + shadow_blur,
                    radiusY: ph / 2.0 + shadow_blur,
                };
                if let (Ok(pill_geo), Ok(outer_geo)) = (
                    g.d2d.CreateRoundedRectangleGeometry(&rr),
                    g.d2d.CreateRoundedRectangleGeometry(&outer_rr),
                ) {
                    if let Ok(mask_geo) = g.d2d.CreatePathGeometry() {
                        if let Ok(sink) = mask_geo.Open() {
                            let combine_res = outer_geo.CombineWithGeometry(
                                &pill_geo,
                                D2D1_COMBINE_MODE_EXCLUDE,
                                None,
                                D2D1_DEFAULT_FLATTENING_TOLERANCE,
                                &sink,
                            );
                            if combine_res.is_ok() && sink.Close().is_ok() {
                                if let Ok(geom_mask) = mask_geo.cast::<ID2D1Geometry>() {
                                    let params = D2D1_LAYER_PARAMETERS1 {
                                        contentBounds: D2D_RECT_F { left: f32::MIN, top: f32::MIN, right: f32::MAX, bottom: f32::MAX },
                                        geometricMask: std::mem::ManuallyDrop::new(Some(geom_mask)),
                                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                                        maskTransform: ui::matrix(1.0, 0.0, 0.0),
                                        opacity: 1.0,
                                        opacityBrush: std::mem::ManuallyDrop::new(None),
                                        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                                    };
                                    g.dc.PushLayer(&params, None);

                                    const PILL_SHADOW_STEPS: usize = 4;
                                    for step in 1..=PILL_SHADOW_STEPS {
                                        let t = step as f32 / PILL_SHADOW_STEPS as f32;
                                        let weight = (1.0 - t * 0.6) / PILL_SHADOW_STEPS as f32;
                                        let step_alpha = shadow_base_alpha * pill_a * weight * 1.5;
                                        let grow = shadow_blur * t;
                                        let dy = shadow_dy * t;
                                        let step_rr = D2D1_ROUNDED_RECT {
                                            rect: D2D_RECT_F {
                                                left: px - grow,
                                                top: py - grow + dy,
                                                right: px + pw + grow,
                                                bottom: py + ph + grow + dy,
                                            },
                                            radiusX: ph / 2.0 + grow,
                                            radiusY: ph / 2.0 + grow,
                                        };
                                        g.brush.SetColor(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: step_alpha });
                                        g.dc.FillRoundedRectangle(&step_rr, &g.brush);
                                    }

                                    g.dc.PopLayer();
                                    drop(std::mem::ManuallyDrop::into_inner(params.geometricMask));
                                }
                            }
                        }
                    }
                }

                let mut c = if self.cfg.accent_pill {
                    D2D1_COLOR_F { a: 0.26, ..accent }
                } else {
                    pal.pill
                };
                c.a *= pill_a;
                g.brush.SetColor(&c);
                g.dc.FillRoundedRectangle(&rr, &g.brush);

                let h = 0.5 * s;
                let line = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F { left: px + h, top: py + h, right: px + pw - h, bottom: py + ph - h },
                    radiusX: ph / 2.0 - h,
                    radiusY: ph / 2.0 - h,
                };
                if is_ios {
                    // В стиле iOS: симметричная тонкая обводка 1 px (0.5·scale·2) без диагонального градиента.
                    // Вертикальный градиент: верх alpha ≈ 0.28 белого / 0.18 чёрного для светлой темы, низ ≈ 0.10.
                    let light = theme::is_light([pal.bg.r, pal.bg.g, pal.bg.b]);
                    let (top_a, bot_a) = if light {
                        (0.18, 0.10)
                    } else {
                        (0.28, 0.10)
                    };
                    let top_col = D2D1_COLOR_F { a: top_a * pill_a, ..pal.fg };
                    let bot_col = D2D1_COLOR_F { a: bot_a * pill_a, ..pal.fg };
                    let stops = [(0.0, top_col), (1.0, bot_col)];
                    if let Some(br) = gradient_brush(g, (px, py), (px, py + ph), &stops) {
                        g.dc.DrawRoundedRectangle(&line, &br, 2.0 * h, None);
                    }
                } else {
                    let fade = |c: D2D1_COLOR_F, factor: f32| D2D1_COLOR_F { a: c.a * pill_a * factor, ..c };
                    // Усилен блик слева-сверху (0.85 вместо 0.60)
                    let stops = [(0.0, fade(pal.rim_hi, 0.85)), (0.5, fade(pal.rim_mid, 0.60)), (1.0, fade(pal.rim_lo, 0.60))];
                    if let Some(br) = gradient_brush(g, (px, py), (px + pw, py + ph), &stops) {
                        g.dc.DrawRoundedRectangle(&line, &br, 2.0 * h, None);
                    }
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
                    c.a = 0.08 * hv * p;
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
                    c.a = 0.26 * p;
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
                // Подписи вкладок лежат прямо на теле острова: невыбранные белые (`bare`), а не серые.
                let mut c = if active { accent } else { ui::bare() };
                c.a *= p;
                g.draw_text(crate::i18n::tab_name(self.order[i]), px, py, pw, ph, c);
            }

            // Шестерёнка настроек: при наведении поворачивается, при открытых настройках залита акцентом.
            let (gx, gy) = self.gear_center();
            let gear_hot = self.expanded && (cursor.0 - gx).hypot(cursor.1 - gy) <= GEAR_R + 3.0;
            let gv = ui.hover_amount(0xF100, gear_hot);
            let gcv = Canvas { g, alpha: p, accent };
            if self.settings_open {
                g.dc.SetTransform(&ui::matrix(s, 0.0, 0.0));
                gcv.circle(gx, gy, GEAR_R, D2D1_COLOR_F { a: 0.26, ..accent });
            }
            g.dc.SetTransform(&ui::matrix_rot(s, gx, gy, 60.0 * gv));
            let gcol = if self.settings_open { accent } else { ui::mix(ui::dim(), ui::txt(), gv) };
            gcv.glyph(ui::icon::SETTINGS, ui::Rect::new(gx - GEAR_R, gy - GEAR_R, 2.0 * GEAR_R, 2.0 * GEAR_R), false, gcol);

            // Карандаш справа (левее шестерёнки): правка виджетов. Появляется только на вкладках с сеткой, в режиме правки залит акцентом.
            let cur = self.normal_id();
            let pv = ui.presence(0xF102, self.expanded && !self.settings_open && tabs.editable(cur));
            if pv > 0.01 {
                let (ex, ey) = self.pencil_center();
                let ev = ui.presence(0xF103, tabs.editing(cur));
                let hot = self.expanded && (cursor.0 - ex).hypot(cursor.1 - ey) <= GEAR_R + 3.0;
                let hv = ui.hover_amount(0xF104, hot);
                let pcv = Canvas { g, alpha: p * pv, accent };
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
            // Со стеклом маска тела нужна всегда (если r > 0), чтобы содержимое не вылезало за
            // скруглённые углы полупрозрачного тела; без стекла — только при большом радиусе (> 16 px).
            let mask = if (self.cfg.glass && r > 0.0) || r > 16.0 * s {
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
                let (w, h) = (self.tab_w(id), self.tab_h(id));
                let ox = centre - w / 2.0 + dx;
                g.dc.SetTransform(&ui::matrix(s, ox * s, 0.0));
                ui.begin_content(ox, interactive);
                let cv = Canvas { g, alpha, accent };
                tabs.draw(id, ui, &cv, w, h);
            };
            if let Some(prev) = self.tab_prev {
                layer(prev, p * (1.0 - fade), -self.tab_dir * SLIDE * fade, false, ui, tabs);
            }
            layer(self.cur_id(), p * fade, self.tab_dir * SLIDE * (1.0 - fade), true, ui, tabs);
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

    /// Подложка стекла: окно с размытием фона под телом острова.
    /// В Windows 11 с поддержкой DWM-скругления (`self.blur_round`) окно подложки позиционируется
    /// как скруглённый DWM-прямоугольник (`glass::blur_plate`), вписанный в тело острова.
    /// Прямоугольный регион `SetWindowRgn` отключает системную тень DWM, не влияя на скругление размытия.
    /// На Windows 10 форма окна по-прежнему вырезается регионом `SetWindowRgn` по `glass::body_outline`.
    unsafe fn apply_blur(&self, x0: f32, aw: f32, ah: f32, ear: f32, r: f32, on: bool) {
        if self.blur_round {
            let dpi = GetDpiForWindow(self.hwnd);
            let dpi = if dpi == 0 { 96 } else { dpi };
            let r_dwm = 8.0 * (dpi as f32 / 96.0);
            let (left_off, width, height, _inset) = crate::glass::blur_plate(x0, aw, ah, r, r_dwm);
            let (gx, gy) = (self.win_x + left_off, self.win_y);
            let key = if on { (gx, gy, width, height, 0) } else { (0, 0, 0, 0, -1) };
            if self.blur_key.get() == key {
                return;
            }
            self.blur_key.set(key);
            if !on || width <= 0 || height <= 0 {
                let _ = ShowWindow(self.glass_wnd, SW_HIDE);
                return;
            }
            let _ = SetWindowPos(
                self.glass_wnd,
                Some(self.hwnd),
                gx,
                gy,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let rgn = CreateRectRgn(0, 0, width, height);
            SetWindowRgn(self.glass_wnd, Some(rgn), true);
        } else {
            let shape = crate::glass::BlurShape::from_outline(&crate::glass::body_outline(x0, aw, ah, ear, r, 1.0));
            let (gx, gy) = (self.win_x + shape.left, self.win_y + shape.top);
            let key = if on { (gx, gy, shape.width, shape.height, shape.signature()) } else { (0, 0, 0, 0, -1) };
            if self.blur_key.get() == key {
                return;
            }
            self.blur_key.set(key);
            if !on || shape.points.len() < 3 {
                let _ = ShowWindow(self.glass_wnd, SW_HIDE);
                return;
            }

            // Позиционируем окно подложки СТРОГО ДО SetWindowRgn, чтобы SetWindowPos не сбрасывал форму окна в DWM.
            let _ = SetWindowPos(self.glass_wnd, Some(self.hwnd), gx, gy, shape.width, shape.height, SWP_NOACTIVATE | SWP_SHOWWINDOW);

            let pts: Vec<POINT> = shape.points.iter().map(|&(x, y)| POINT { x, y }).collect();
            let rgn = windows::Win32::Graphics::Gdi::CreatePolygonRgn(&pts, windows::Win32::Graphics::Gdi::ALTERNATE);
            // bRedraw = true сообщает DWM о перерасчёте формы окна; система забирает владение регионом себе.
            SetWindowRgn(self.glass_wnd, Some(rgn), true);
        }
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
