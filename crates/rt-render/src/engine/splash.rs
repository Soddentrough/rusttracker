pub(in crate::engine) fn gamepad_icon(g_type: crate::state::GamepadType, action: &str) -> String {
    match action {
        "A" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e997}",
            crate::state::GamepadType::Nintendo => "\u{e974}",
            _ => "\u{e994}",
        },
        "B" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e999}",
            crate::state::GamepadType::Nintendo => "\u{e994}",
            _ => "\u{e974}",
        },
        "X" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e998}",
            crate::state::GamepadType::Nintendo => "\u{e996}",
            _ => "\u{e995}",
        },
        "Y" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e99a}",
            crate::state::GamepadType::Nintendo => "\u{e995}",
            _ => "\u{e996}",
        },
        "L1" => match g_type {
            crate::state::GamepadType::Nintendo => "\u{e99c}",
            _ => "\u{e99d}",
        },
        "R1" => match g_type {
            crate::state::GamepadType::Nintendo => "\u{e9a5}",
            _ => "\u{e9a4}",
        },
        "L2" => "\u{e99e}",
        "R2" => "\u{e9a3}",
        "L2 / R2" => "\u{e99e} \u{e9a3}",
        "L1 / R1" => match g_type {
            crate::state::GamepadType::Nintendo => "\u{e99c} \u{e9a5}",
            _ => "\u{e99d} \u{e9a4}",
        },
        "Select" => "\u{e9a9}",
        "Start" => "\u{e9a8}",
        "D-Pad L/R" => "\u{e9af} \u{e9ad}",
        "D-Pad U/D" => "\u{e9ac} \u{e9ae}",
        _ => action,
    }
    .to_string()
}

#[derive(Clone, Copy)]
pub(crate) struct SplashStar {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub speed: f32,
    pub phase: f32,
}

pub(crate) struct SplashCache {
    pub cached_rect: egui::Rect,
    pub cached_ppp: f32,
    pub static_bg_mesh: std::sync::Arc<egui::Mesh>,
    pub stars: Vec<SplashStar>,
    pub horizon_y: f32,
    pub grid_color: egui::Color32,
    pub star_base: egui::Color32,
    pub cached_title_rect: egui::Rect,
    pub cached_title_ppp: f32,
    pub cached_font_tex_size: [usize; 2],
    pub title_base_mesh: egui::Mesh,
    pub title_slice_ranges: Vec<(usize, usize)>,
    pub title_glyph_bounds: egui::Rect,
    pub title_glow_range: (usize, usize),
}

impl Default for SplashCache {
    fn default() -> Self {
        Self {
            cached_rect: egui::Rect::NOTHING,
            cached_ppp: 0.0,
            static_bg_mesh: std::sync::Arc::new(egui::Mesh::default()),
            stars: Vec::new(),
            horizon_y: 0.0,
            grid_color: egui::Color32::TRANSPARENT,
            star_base: egui::Color32::WHITE,
            cached_title_rect: egui::Rect::NOTHING,
            cached_title_ppp: 0.0,
            cached_font_tex_size: [0, 0],
            title_base_mesh: egui::Mesh::default(),
            title_slice_ranges: Vec::new(),
            title_glyph_bounds: egui::Rect::NOTHING,
            title_glow_range: (0, 0),
        }
    }
}

impl SplashCache {
    #[inline]
    pub fn tint_rgb(r: u8, g: u8, b: u8) -> egui::Color32 {
        let nr = ((r as u32 * 75 + 10 * 180 + 127) / 255) as u8;
        let ng = ((g as u32 * 75 + 10 * 180 + 127) / 255) as u8;
        let nb = ((b as u32 * 75 + 15 * 180 + 127) / 255) as u8;
        egui::Color32::from_rgb(nr, ng, nb)
    }

    #[inline]
    fn pseudo_rand(seed: u32) -> f32 {
        let mut x = seed.wrapping_mul(136453);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        (x % 1000) as f32 / 1000.0
    }

    pub fn ensure_background(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let ppp = ctx.pixels_per_point().max(0.5);
        let rect_same = (rect.min.x - self.cached_rect.min.x).abs() < 0.5
            && (rect.min.y - self.cached_rect.min.y).abs() < 0.5
            && (rect.max.x - self.cached_rect.max.x).abs() < 0.5
            && (rect.max.y - self.cached_rect.max.y).abs() < 0.5
            && (ppp - self.cached_ppp).abs() < 0.01
            && !self.static_bg_mesh.is_empty();
        if rect_same {
            return;
        }

        self.cached_rect = rect;
        self.cached_ppp = ppp;
        let horizon_y = rect.top() + rect.height() * 0.45;
        self.horizon_y = horizon_y;
        let center_x = rect.center().x;
        let sky_height = (horizon_y - rect.top()).max(1.0);

        self.star_base = Self::tint_rgb(255, 255, 255);
        self.grid_color = Self::tint_rgb(212, 34, 161);

        let mut static_shapes: Vec<egui::epaint::ClippedShape> = Vec::with_capacity(128);
        let mut push_shape = |shape: egui::Shape| {
            static_shapes.push(egui::epaint::ClippedShape::new(rect, shape));
        };

        // 1. Multi-stop Synthwave Sky Gradient + Horizon Bloom + Floor Base (single hardware-interpolated Mesh)
        let c_sky_top = Self::tint_rgb(34, 12, 74);
        let c_sky_upper = Self::tint_rgb(98, 16, 112);
        let c_sky_mid = Self::tint_rgb(186, 30, 128);
        let c_sky_bot = Self::tint_rgb(255, 126, 103);
        let c_floor_top = Self::tint_rgb(24, 14, 60);
        let c_floor_bot = Self::tint_rgb(40, 22, 92);

        let mut base_mesh = egui::Mesh::default();
        let white_uv = egui::pos2(0.0, 0.0);
        let sky_stops = [
            (rect.top(), c_sky_top),
            (rect.top() + 0.45 * sky_height, c_sky_upper),
            (rect.top() + 0.78 * sky_height, c_sky_mid),
            (horizon_y, c_sky_bot),
        ];
        for &(y_pos, col) in &sky_stops {
            base_mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(rect.left(), y_pos),
                uv: white_uv,
                color: col,
            });
            base_mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(rect.right(), y_pos),
                uv: white_uv,
                color: col,
            });
        }
        base_mesh.indices.extend_from_slice(&[
            0, 1, 2, 1, 3, 2, // top -> upper
            2, 3, 4, 3, 5, 4, // upper -> mid
            4, 5, 6, 5, 7, 6, // mid -> horizon
        ]);
        // Floor gradient quad (vertices 8..11)
        let f_base = base_mesh.vertices.len() as u32;
        for &(y_pos, col) in &[(horizon_y, c_floor_top), (rect.bottom(), c_floor_bot)] {
            base_mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(rect.left(), y_pos),
                uv: white_uv,
                color: col,
            });
            base_mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(rect.right(), y_pos),
                uv: white_uv,
                color: col,
            });
        }
        base_mesh.indices.extend_from_slice(&[
            f_base,
            f_base + 1,
            f_base + 2,
            f_base + 1,
            f_base + 3,
            f_base + 2,
        ]);
        push_shape(egui::Shape::mesh(base_mesh));

        // 2. Precompute Starfield coordinates & twinkle parameters (confined to upper sky above sun/mountains)
        self.stars.clear();
        self.stars.reserve(56);
        for i in 0..56 {
            let rx = Self::pseudo_rand(i * 31);
            let ry = Self::pseudo_rand(i * 31 + 1);
            let rr = Self::pseudo_rand(i * 31 + 2);
            let rt = Self::pseudo_rand(i * 31 + 3);
            self.stars.push(SplashStar {
                x: rect.left() + rx * rect.width(),
                y: rect.top() + ry * sky_height * 0.52,
                radius: rr * 1.4 + 0.6,
                speed: 1.0 + rt * 2.2,
                phase: rx * 100.0,
            });
        }

        // 3. Retro Sliced Sun
        let sun_radius = rect.width().min(rect.height()) * 0.19;
        let sun_center = egui::pos2(center_x, horizon_y - sun_radius * 0.30);
        let sun_steps = 40usize;
        let is_sun_slice_drawn = |idx: usize| -> bool {
            if idx >= sun_steps {
                return false;
            }
            let st = idx as f32 / sun_steps as f32;
            if st > 0.45 {
                let slice_t = (st - 0.45) / 0.55;
                let slice_val = (slice_t * 15.0).fract();
                let gap_threshold = 0.2 + slice_t * 0.6;
                if slice_val < gap_threshold {
                    return false;
                }
            }
            true
        };

        for i in 0..sun_steps {
            if !is_sun_slice_drawn(i) {
                continue;
            }
            let t = i as f32 / sun_steps as f32;
            let next_t = (i + 1) as f32 / sun_steps as f32;

            let y_min = sun_center.y - sun_radius + t * sun_radius * 2.0;
            let mut y_max = sun_center.y - sun_radius + next_t * sun_radius * 2.0;
            if is_sun_slice_drawn(i + 1) {
                y_max = (y_max + 0.6).min(sun_center.y + sun_radius);
            }
            if y_min >= horizon_y {
                continue;
            }
            y_max = y_max.min(horizon_y);
            if y_max <= y_min {
                continue;
            }

            let r = (225.0 * (1.0 - t) + 195.0 * t) as u8;
            let g = (175.0 * (1.0 - t) + 42.0 * t) as u8;
            let b = (32.0 * (1.0 - t) + 112.0 * t) as u8;
            let color = egui::Color32::from_rgb(r, g, b);

            let sub_steps = 2usize;
            let mut pts = Vec::with_capacity((sub_steps + 1) * 2);
            for k in 0..=sub_steps {
                let frac = k as f32 / sub_steps as f32;
                let yk = y_min + (y_max - y_min) * frac;
                let dy = (yk - sun_center.y).clamp(-sun_radius, sun_radius);
                let dx = (sun_radius * sun_radius - dy * dy).max(0.0).sqrt();
                pts.push(egui::pos2(sun_center.x + dx, yk));
            }
            for k in (0..=sub_steps).rev() {
                let frac = k as f32 / sub_steps as f32;
                let yk = y_min + (y_max - y_min) * frac;
                let dy = (yk - sun_center.y).clamp(-sun_radius, sun_radius);
                let dx = (sun_radius * sun_radius - dy * dy).max(0.0).sqrt();
                if dx > 0.5 {
                    pts.push(egui::pos2(sun_center.x - dx, yk));
                }
            }
            if pts.len() >= 3 {
                push_shape(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
            }
        }

        // 4. Layered Wireframe Mountains (sculpted lower across the central sun valley so the sun remains iconic)
        let mtn_fill = Self::tint_rgb(24, 22, 54);
        let mtn_stroke = egui::Stroke::new(1.1_f32, Self::tint_rgb(105, 58, 225));
        for i in 0..25 {
            let h1 = Self::pseudo_rand(i * 17);
            let h2 = Self::pseudo_rand(i * 17 + 1);
            let h3 = Self::pseudo_rand(i * 17 + 2);

            let cx = rect.left() + h1 * rect.width();
            let cy = horizon_y;
            let center_dist = ((cx - center_x).abs() / (rect.width() * 0.5)).clamp(0.0, 1.0);
            let valley_half_span = (sun_radius / (rect.width() * 0.5).max(1.0)).clamp(0.38, 0.72);
            let valley_scale = (center_dist / valley_half_span).clamp(0.16, 1.0);
            let width = rect.width() * (0.09 + h2 * 0.20);
            let height = sky_height * (0.11 + h3 * 0.42) * valley_scale;

            let p1 = egui::pos2(cx - width, cy);
            let p2 = egui::pos2(cx + width, cy);
            let p3 = egui::pos2(cx, cy - height);

            push_shape(egui::Shape::convex_polygon(
                vec![p1, p2, p3],
                mtn_fill,
                mtn_stroke,
            ));

            let ridge_offset = (Self::pseudo_rand(i * 17 + 3) - 0.5) * 0.4;
            push_shape(egui::Shape::line_segment(
                [p3, egui::pos2(cx + width * ridge_offset, cy)],
                mtn_stroke,
            ));
        }

        // 5. Vertical Radiating Perspective Grid Lines + Horizon Laser Line
        let num_v_lines = 40;
        for i in 0..=num_v_lines {
            let t = i as f32 / num_v_lines as f32;
            let bottom_x = rect.left() + (t - 0.5) * rect.width() * 8.0;
            let dx = bottom_x - center_x;
            let dy = rect.bottom() - horizon_y;
            let end_pt = if bottom_x < rect.left() && dx.abs() > 1e-3 {
                let clip_t = (rect.left() - center_x) / dx;
                egui::pos2(rect.left(), horizon_y + clip_t * dy)
            } else if bottom_x > rect.right() && dx.abs() > 1e-3 {
                let clip_t = (rect.right() - center_x) / dx;
                egui::pos2(rect.right(), horizon_y + clip_t * dy)
            } else {
                egui::pos2(bottom_x, rect.bottom())
            };

            push_shape(egui::Shape::line_segment(
                [egui::pos2(center_x, horizon_y), end_pt],
                egui::Stroke::new(1.0_f32, self.grid_color),
            ));
        }
        push_shape(egui::Shape::line_segment(
            [
                egui::pos2(rect.left(), horizon_y),
                egui::pos2(rect.right(), horizon_y),
            ],
            egui::Stroke::new(1.5_f32, Self::tint_rgb(255, 95, 205)),
        ));

        // Pre-tessellate all static background primitives once and merge into a single cached Mesh
        let mut tessellator = egui::epaint::Tessellator::new(
            ppp,
            egui::epaint::TessellationOptions::default(),
            [1, 1],
            Vec::new(),
        );
        let primitives = tessellator.tessellate_shapes(static_shapes);
        let mut merged = egui::Mesh::default();
        for prim in primitives {
            if let egui::epaint::Primitive::Mesh(m) = prim.primitive
                && m.texture_id == egui::TextureId::default()
            {
                merged.append(m);
            }
        }
        self.static_bg_mesh = std::sync::Arc::new(merged);
    }

    /// Appends the animated elements of the Splash background (twinkling starfield + scrolling
    /// perspective horizontal grid lines) directly into `mesh`, bypassing `epaint::Tessellator`.
    pub fn append_dynamic_bg(&self, mesh: &mut egui::Mesh, rect: egui::Rect, time: f32, ppp: f32) {
        let num_h_lines = 30usize;
        mesh.reserve_vertices(self.stars.len() * 8 + num_h_lines * 8);
        mesh.reserve_triangles(self.stars.len() * 6 + num_h_lines * 6);
        let white_uv = egui::pos2(0.0, 0.0);
        let feather = (1.0 / ppp.max(1.0)).clamp(0.4, 1.0);

        for star in &self.stars {
            let twinkle = ((time * star.speed + star.phase).sin() * 0.5 + 0.5) * 205.0 + 50.0;
            let core_col = egui::Color32::from_rgba_unmultiplied(
                self.star_base.r(),
                self.star_base.g(),
                self.star_base.b(),
                twinkle as u8,
            );
            let r = star.radius;
            let rf = r + feather;
            let base = mesh.vertices.len() as u32;
            for &(dx, dy, col) in &[
                (0.0, -r, core_col),
                (r, 0.0, core_col),
                (0.0, r, core_col),
                (-r, 0.0, core_col),
                (0.0, -rf, egui::Color32::TRANSPARENT),
                (rf, 0.0, egui::Color32::TRANSPARENT),
                (0.0, rf, egui::Color32::TRANSPARENT),
                (-rf, 0.0, egui::Color32::TRANSPARENT),
            ] {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: egui::pos2(star.x + dx, star.y + dy),
                    uv: white_uv,
                    color: col,
                });
            }
            mesh.indices.extend_from_slice(&[
                base,
                base + 1,
                base + 2,
                base,
                base + 2,
                base + 3,
                base,
                base + 4,
                base + 5,
                base,
                base + 5,
                base + 1,
                base + 1,
                base + 5,
                base + 6,
                base + 1,
                base + 6,
                base + 2,
                base + 2,
                base + 6,
                base + 7,
                base + 2,
                base + 7,
                base + 3,
                base + 3,
                base + 7,
                base + 4,
                base + 3,
                base + 4,
                base,
            ]);
        }

        let horizon_y = self.horizon_y;
        let floor_h = (rect.bottom() - horizon_y).max(1.0);
        let frac_t = (time * 1.5).fract();
        for i in 0..num_h_lines {
            let offset = (i as f32 - frac_t) / num_h_lines as f32;
            if offset <= 0.0 {
                continue;
            }
            let depth = offset.powf(3.0);
            let y = horizon_y + floor_h * depth;
            if y >= rect.bottom() {
                continue;
            }
            let thickness = 1.0 + offset * 2.0;
            let half_t = thickness * 0.5;
            let alpha = (offset * 3.2).clamp(0.15, 1.0);
            let col = egui::Color32::from_rgba_unmultiplied(
                self.grid_color.r(),
                self.grid_color.g(),
                self.grid_color.b(),
                (255.0 * alpha) as u8,
            );

            let y0_out = (y - half_t - feather).max(horizon_y);
            let y0_in = (y - half_t).max(horizon_y);
            let y1_in = (y + half_t).min(rect.bottom());
            let y1_out = (y + half_t + feather).min(rect.bottom());

            let base = mesh.vertices.len() as u32;
            for &(yv, c) in &[
                (y0_out, egui::Color32::TRANSPARENT),
                (y0_in, col),
                (y1_in, col),
                (y1_out, egui::Color32::TRANSPARENT),
            ] {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: egui::pos2(rect.left(), yv),
                    uv: white_uv,
                    color: c,
                });
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: egui::pos2(rect.right(), yv),
                    uv: white_uv,
                    color: c,
                });
            }
            mesh.indices.extend_from_slice(&[
                base,
                base + 1,
                base + 2,
                base + 1,
                base + 3,
                base + 2,
                base + 2,
                base + 3,
                base + 4,
                base + 3,
                base + 5,
                base + 4,
                base + 4,
                base + 5,
                base + 6,
                base + 5,
                base + 7,
                base + 6,
            ]);
        }
    }

    pub fn build_combined_bg_primitive(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        time: f32,
    ) -> egui::ClippedPrimitive {
        self.ensure_background(ctx, rect);
        let mut mesh = (*self.static_bg_mesh).clone();
        self.append_dynamic_bg(&mut mesh, rect, time, ctx.pixels_per_point());
        if !self.title_base_mesh.is_empty() {
            mesh.append_ref(&self.title_base_mesh);
        }
        egui::ClippedPrimitive {
            clip_rect: rect,
            primitive: egui::epaint::Primitive::Mesh(mesh),
        }
    }

    /// Evaluates the 80s synthwave airbrushed chrome reflection color at normalized title
    /// coordinates `(nx, ny)` in `[0.0, 1.0]^2`, per-glyph horizontal facet `gx` in `[0.0, 1.0]`,
    /// and animation timestamp `time` (seconds).
    ///
    /// Unlike a naive vertical wrap (`(ny + time * speed).fract()`), this anchors the reflection
    /// horizon near the optical waist of the letters while animating:
    /// 1. A multi-harmonic liquid-chrome horizon undulation across `nx` and `time` (zero wrap seam).
    /// 2. Continuous scrolling atmospheric reflection bands in the upper sky and scrolling neon
    ///    grid reflection ripples in the lower ground half.
    /// 3. A traveling diagonal anamorphic specular light-bar sweep across `(nx, ny)`.
    #[inline]
    pub fn evaluate_chrome_color(nx: f32, ny: f32, gx: f32, time: f32) -> egui::Color32 {
        let lerp = |a: f32, b: f32, t: f32| -> f32 { a + (b - a) * t };
        let smoothstep = |t: f32| -> f32 {
            let c = t.clamp(0.0, 1.0);
            c * c * (3.0 - 2.0 * c)
        };

        // Liquid-metal horizon ripple + subtle per-glyph cylindrical bevel tilt
        let horizon_wave = (nx * 5.2 - time * 1.45).sin() * 0.036
            + (nx * 11.5 + time * 0.85).cos() * 0.015
            + (time * 0.65).sin() * 0.018;
        let env_y = (ny + horizon_wave + (gx - 0.5) * 0.026).clamp(0.0, 1.0);

        let (mut r, mut g, mut b) = if env_y < 0.48 {
            // Upper Sky Reflection: Deep cobalt -> electric cerulean -> ice cyan -> blazing white horizon
            let sky_t = (env_y / 0.48).clamp(0.0, 1.0);
            let (mut sr, mut sg, mut sb) = if sky_t < 0.48 {
                let u = smoothstep(sky_t / 0.48);
                (
                    lerp(18.0, 0.0, u),
                    lerp(42.0, 162.0, u),
                    lerp(142.0, 250.0, u),
                )
            } else if sky_t < 0.84 {
                let u = smoothstep((sky_t - 0.48) / 0.36);
                (
                    lerp(0.0, 145.0, u),
                    lerp(162.0, 245.0, u),
                    lerp(250.0, 255.0, u),
                )
            } else {
                let u = smoothstep((sky_t - 0.84) / 0.16);
                (
                    lerp(145.0, 255.0, u),
                    lerp(245.0, 255.0, u),
                    lerp(255.0, 255.0, u),
                )
            };

            // Continuous upward-scrolling sky sheen bands (attenuated near the white horizon core)
            let sky_band = ((ny * 10.5 - time * 1.75 + nx * 2.4).sin() * 0.5 + 0.5).powi(2);
            let band_mask = (1.0 - sky_t).powf(0.75) * 0.28;
            sr = lerp(sr, 195.0, sky_band * band_mask);
            sg = lerp(sg, 248.0, sky_band * band_mask);
            sb = lerp(sb, 255.0, sky_band * band_mask);
            (sr, sg, sb)
        } else if env_y < 0.54 {
            // High-contrast 80s Chrome Horizon Crease: Blazing White -> Deep Obsidian-Violet
            let u = smoothstep((env_y - 0.48) / 0.06);
            (
                lerp(255.0, 22.0, u),
                lerp(255.0, 8.0, u),
                lerp(255.0, 48.0, u),
            )
        } else {
            // Lower Sunset / Neon Grid Reflection: Obsidian -> Neon Magenta -> Sunset Coral -> Solar Gold
            let gnd_t = ((env_y - 0.54) / 0.46).clamp(0.0, 1.0);
            let (mut gr, mut gg, mut gb) = if gnd_t < 0.36 {
                let u = smoothstep(gnd_t / 0.36);
                (
                    lerp(22.0, 210.0, u),
                    lerp(8.0, 28.0, u),
                    lerp(48.0, 152.0, u),
                )
            } else if gnd_t < 0.72 {
                let u = smoothstep((gnd_t - 0.36) / 0.36);
                (
                    lerp(210.0, 255.0, u),
                    lerp(28.0, 102.0, u),
                    lerp(152.0, 88.0, u),
                )
            } else {
                let u = smoothstep((gnd_t - 0.72) / 0.28);
                (
                    lerp(255.0, 255.0, u),
                    lerp(102.0, 215.0, u),
                    lerp(88.0, 92.0, u),
                )
            };

            // Continuous downward-scrolling perspective neon grid reflection ripples
            let grid_ripple =
                ((gnd_t.powf(0.8) * 12.0 - time * 2.1 - nx * 1.8).sin() * 0.5 + 0.5).powi(2);
            let ripple_mask = gnd_t * (1.0 - gnd_t * 0.45) * 0.30;
            gr = lerp(gr, 255.0, grid_ripple * ripple_mask);
            gg = lerp(gg, 185.0, grid_ripple * ripple_mask);
            gb = lerp(gb, 230.0, grid_ripple * ripple_mask);
            (gr, gg, gb)
        };

        // Traveling diagonal anamorphic specular light-bar sweep across the full title
        // Period spans [-0.45, 1.55] so diag_coord in [-0.04, 1.06] has zero intensity at wrap ends.
        let sweep_center = (time * 0.30).rem_euclid(2.0) - 0.45;
        let diag_coord = nx * 0.82 + ny * 0.22 - gx * 0.04;
        let d_main = (diag_coord - sweep_center).abs();
        let glint_main = smoothstep(1.0 - (d_main / 0.14).clamp(0.0, 1.0)).powi(2);
        let d_trail = (diag_coord - (sweep_center - 0.17)).abs();
        let glint_trail = smoothstep(1.0 - (d_trail / 0.065).clamp(0.0, 1.0)).powi(2) * 0.50;
        let glint = (glint_main * 0.68 + glint_trail).clamp(0.0, 1.0);

        r = lerp(r, 248.0, glint);
        g = lerp(g, 253.0, glint);
        b = lerp(b, 255.0, glint);

        egui::Color32::from_rgb(
            r.round().clamp(0.0, 255.0) as u8,
            g.round().clamp(0.0, 255.0) as u8,
            b.round().clamp(0.0, 255.0) as u8,
        )
    }

    /// Builds or updates the cached `RustTracker` chrome title mesh in-place.
    /// Full font layout, 3D extrusion, directional bevel, and 24-slice geometry construction
    /// happen only when `title_rect` or `pixels_per_point` changes; on all subsequent frames
    /// only the outer neon corona and interior chrome vertex colors are updated in-place on
    /// `self.title_base_mesh` and appended directly into the combined Splash primitive.
    pub fn update_title_mesh(
        &mut self,
        ui: &egui::Ui,
        title_rect: egui::Rect,
        gradient_extent: f32,
        font_size: f32,
        time: f32,
    ) {
        let ppp = ui.ctx().pixels_per_point().max(0.5);
        let current_font_tex_size = ui.ctx().fonts_mut(|f| f.font_image_size());
        let steps = 24usize;
        let rect_same = (title_rect.min.x - self.cached_title_rect.min.x).abs() < 0.5
            && (title_rect.min.y - self.cached_title_rect.min.y).abs() < 0.5
            && (title_rect.max.x - self.cached_title_rect.max.x).abs() < 0.5
            && (title_rect.max.y - self.cached_title_rect.max.y).abs() < 0.5
            && (ppp - self.cached_title_ppp).abs() < 0.01
            && self.cached_font_tex_size == current_font_tex_size
            && !self.title_base_mesh.is_empty();

        if !rect_same {
            self.cached_title_rect = title_rect;
            self.cached_title_ppp = ppp;

            let painter = ui.painter();
            let font_id = egui::FontId::new(font_size, egui::FontFamily::Name("Orbitron".into()));
            let galley =
                painter.layout_no_wrap("RustTracker".to_string(), font_id, egui::Color32::WHITE);
            let base_pos = egui::Align2::CENTER_CENTER
                .anchor_size(title_rect.center(), galley.size())
                .min;
            let snapped_pos = egui::pos2(
                (base_pos.x * ppp).round() / ppp,
                (base_pos.y * ppp).round() / ppp,
            );
            let font_tex_size = ui.ctx().fonts_mut(|f| f.font_image_size());
            self.cached_font_tex_size = font_tex_size;
            let uv_norm = egui::vec2(
                1.0 / (font_tex_size[0].max(1) as f32),
                1.0 / (font_tex_size[1].max(1) as f32),
            );

            let mut glyph_quads = Vec::with_capacity(16);
            let mut gx_min = f32::INFINITY;
            let mut gy_min = f32::INFINITY;
            let mut gx_max = f32::NEG_INFINITY;
            let mut gy_max = f32::NEG_INFINITY;

            for placed_row in &galley.rows {
                let row_pos = snapped_pos + placed_row.pos.to_vec2();
                for q in placed_row.row.visuals.mesh.vertices.as_chunks::<4>().0 {
                    let x_min =
                        q[0].pos.x.min(q[1].pos.x).min(q[2].pos.x).min(q[3].pos.x) + row_pos.x;
                    let x_max =
                        q[0].pos.x.max(q[1].pos.x).max(q[2].pos.x).max(q[3].pos.x) + row_pos.x;
                    let y_min =
                        q[0].pos.y.min(q[1].pos.y).min(q[2].pos.y).min(q[3].pos.y) + row_pos.y;
                    let y_max =
                        q[0].pos.y.max(q[1].pos.y).max(q[2].pos.y).max(q[3].pos.y) + row_pos.y;
                    let u_min = q[0].uv.x.min(q[1].uv.x).min(q[2].uv.x).min(q[3].uv.x) * uv_norm.x;
                    let u_max = q[0].uv.x.max(q[1].uv.x).max(q[2].uv.x).max(q[3].uv.x) * uv_norm.x;
                    let v_min = q[0].uv.y.min(q[1].uv.y).min(q[2].uv.y).min(q[3].uv.y) * uv_norm.y;
                    let v_max = q[0].uv.y.max(q[1].uv.y).max(q[2].uv.y).max(q[3].uv.y) * uv_norm.y;
                    gx_min = gx_min.min(x_min);
                    gy_min = gy_min.min(y_min);
                    gx_max = gx_max.max(x_max);
                    gy_max = gy_max.max(y_max);
                    glyph_quads.push((x_min, y_min, x_max, y_max, u_min, v_min, u_max, v_max));
                }
            }

            self.title_glyph_bounds = if gx_max > gx_min && gy_max > gy_min {
                egui::Rect::from_min_max(egui::pos2(gx_min, gy_min), egui::pos2(gx_max, gy_max))
            } else {
                egui::Rect::from_center_size(
                    title_rect.center(),
                    egui::vec2(
                        title_rect.width().max(1.0),
                        (gradient_extent * 2.0).max(1.0),
                    ),
                )
            };

            // Scale all bevel, extrusion, and outline offsets proportionally with font_size
            let s = (font_size / 66.0).clamp(0.42, 1.5);

            let mut title_mesh = egui::Mesh::with_texture(egui::TextureId::default());
            title_mesh.reserve_vertices(glyph_quads.len() * (29 + steps) * 4);
            title_mesh.reserve_triangles(glyph_quads.len() * (29 + steps) * 2);

            let push_offset_layer =
                |mesh: &mut egui::Mesh, dx: f32, dy: f32, color: egui::Color32| {
                    for &(x_min, y_min, x_max, y_max, u_min, v_min, u_max, v_max) in &glyph_quads {
                        mesh.add_rect_with_uv(
                            egui::Rect::from_min_max(
                                egui::pos2(x_min + dx, y_min + dy),
                                egui::pos2(x_max + dx, y_max + dy),
                            ),
                            egui::Rect::from_min_max(
                                egui::pos2(u_min, v_min),
                                egui::pos2(u_max, v_max),
                            ),
                            color,
                        );
                    }
                };

            // 1. Deep Grounding Drop Shadow (lifts logo off the upper synthwave sky)
            let drop_shadow = egui::Color32::from_rgba_unmultiplied(6, 3, 20, 185);
            for (dx, dy) in [(2.4 * s, 4.4 * s), (0.8 * s, 4.2 * s), (3.8 * s, 3.6 * s)] {
                push_offset_layer(&mut title_mesh, dx, dy, drop_shadow);
            }

            // 2. Animated Neon Corona / Outer Bloom Ring (updated in-place each frame)
            let glow_start = title_mesh.vertices.len();
            let base_glow = egui::Color32::from_rgba_unmultiplied(212, 34, 161, 48);
            for (dx, dy) in [
                (-3.8 * s, 0.0),
                (3.8 * s, 0.0),
                (0.0, -3.5 * s),
                (0.0, 3.8 * s),
                (-2.8 * s, -2.6 * s),
                (2.8 * s, -2.6 * s),
                (-2.8 * s, 2.8 * s),
                (2.8 * s, 2.8 * s),
            ] {
                push_offset_layer(&mut title_mesh, dx, dy, base_glow);
            }
            self.title_glow_range = (glow_start, title_mesh.vertices.len());

            // 3. 3D Metallic Block Extrusion (receding down-right in cosmic indigo/violet)
            for (dx, dy, col) in [
                (3.2 * s, 3.3 * s, egui::Color32::from_rgb(32, 14, 76)),
                (2.5 * s, 2.6 * s, egui::Color32::from_rgb(52, 22, 108)),
                (1.9 * s, 2.0 * s, egui::Color32::from_rgb(76, 32, 142)),
            ] {
                push_offset_layer(&mut title_mesh, dx, dy, col);
            }

            // 4. Directional Specular Bevel Rim (Icy Cyan-Silver top-left highlight vs. Neon Magenta bottom-right rim)
            let bevel_highlight = egui::Color32::from_rgb(218, 246, 255);
            for (dx, dy) in [
                (-1.8 * s, -1.8 * s),
                (0.0, -1.9 * s),
                (-1.9 * s, 0.0),
                (1.5 * s, -1.6 * s),
            ] {
                push_offset_layer(&mut title_mesh, dx, dy, bevel_highlight);
            }
            let bevel_rim = egui::Color32::from_rgb(245, 90, 200);
            for (dx, dy) in [
                (1.7 * s, 1.7 * s),
                (0.0, 1.8 * s),
                (1.8 * s, 0.0),
                (-1.5 * s, 1.6 * s),
            ] {
                push_offset_layer(&mut title_mesh, dx, dy, bevel_rim);
            }

            // 5. Crisp 8-Direction Dark Obsidian Inner Crease (eliminates corner gaps on thin strokes)
            let crease_color = egui::Color32::from_rgb(10, 6, 24);
            let c = 0.92 * s;
            for (dx, dy) in [
                (-c, -c),
                (0.0, -c),
                (c, -c),
                (-c, 0.0),
                (c, 0.0),
                (-c, c),
                (0.0, c),
                (c, c),
            ] {
                push_offset_layer(&mut title_mesh, dx, dy, crease_color);
            }

            // 6. 24-Slice Chrome Interior across exact tight glyph vertical bounds
            self.title_slice_ranges.clear();
            self.title_slice_ranges.reserve(steps);
            let top_y = self.title_glyph_bounds.min.y;
            let bottom_y = self.title_glyph_bounds.max.y;
            let height = (bottom_y - top_y).max(1.0);

            for i in 0..steps {
                let t = i as f32 / steps as f32;
                let next_t = (i + 1) as f32 / steps as f32;
                let min_y = top_y + t * height;
                let max_y = top_y + next_t * height;
                let v_start = title_mesh.vertices.len();

                for &(x_min, y_min, x_max, y_max, u_min, v_min, u_max, v_max) in &glyph_quads {
                    let sy0 = min_y.max(y_min);
                    let sy1 = max_y.min(y_max);
                    let gh = y_max - y_min;
                    if sy1 > sy0 && gh > 0.0 {
                        let f0 = (sy0 - y_min) / gh;
                        let f1 = (sy1 - y_min) / gh;
                        let sv0 = v_min + (v_max - v_min) * f0;
                        let sv1 = v_min + (v_max - v_min) * f1;
                        let idx = title_mesh.vertices.len() as u32;
                        title_mesh.vertices.extend_from_slice(&[
                            egui::epaint::Vertex {
                                pos: egui::pos2(x_min, sy0),
                                uv: egui::pos2(u_min, sv0),
                                color: egui::Color32::WHITE,
                            },
                            egui::epaint::Vertex {
                                pos: egui::pos2(x_max, sy0),
                                uv: egui::pos2(u_max, sv0),
                                color: egui::Color32::WHITE,
                            },
                            egui::epaint::Vertex {
                                pos: egui::pos2(x_max, sy1),
                                uv: egui::pos2(u_max, sv1),
                                color: egui::Color32::WHITE,
                            },
                            egui::epaint::Vertex {
                                pos: egui::pos2(x_min, sy1),
                                uv: egui::pos2(u_min, sv1),
                                color: egui::Color32::WHITE,
                            },
                        ]);
                        title_mesh.indices.extend_from_slice(&[
                            idx,
                            idx + 1,
                            idx + 2,
                            idx,
                            idx + 2,
                            idx + 3,
                        ]);
                    }
                }
                let v_end = title_mesh.vertices.len();
                self.title_slice_ranges.push((v_start, v_end));
            }

            self.title_base_mesh = title_mesh;
        }

        // Fast per-frame vertex color update (zero glyph layout or tessellation)
        let bounds = self.title_glyph_bounds;
        let inv_w = 1.0 / bounds.width().max(1.0);
        let inv_h = 1.0 / bounds.height().max(1.0);

        // Update animated outer neon corona (cyan <-> magenta wave across the logo)
        let (glow_start, glow_end) = self.title_glow_range;
        if glow_end > glow_start && glow_end <= self.title_base_mesh.vertices.len() {
            let pulse = (time * 1.8).sin() * 0.5 + 0.5;
            let alpha = (38.0 + pulse * 22.0) as u8;
            for quad in self.title_base_mesh.vertices[glow_start..glow_end]
                .as_chunks_mut::<4>()
                .0
            {
                let nx0 = ((quad[0].pos.x - bounds.min.x) * inv_w).clamp(0.0, 1.0);
                let nx1 = ((quad[1].pos.x - bounds.min.x) * inv_w).clamp(0.0, 1.0);
                let mix0 = (nx0 * std::f32::consts::PI - time * 1.1).sin() * 0.5 + 0.5;
                let mix1 = (nx1 * std::f32::consts::PI - time * 1.1).sin() * 0.5 + 0.5;
                let c0 = egui::Color32::from_rgba_unmultiplied(
                    (225.0 * (1.0 - mix0) + 0.0 * mix0) as u8,
                    (40.0 * (1.0 - mix0) + 215.0 * mix0) as u8,
                    (175.0 * (1.0 - mix0) + 255.0 * mix0) as u8,
                    alpha,
                );
                let c1 = egui::Color32::from_rgba_unmultiplied(
                    (225.0 * (1.0 - mix1) + 0.0 * mix1) as u8,
                    (40.0 * (1.0 - mix1) + 215.0 * mix1) as u8,
                    (175.0 * (1.0 - mix1) + 255.0 * mix1) as u8,
                    alpha,
                );
                quad[0].color = c0;
                quad[1].color = c1;
                quad[2].color = c1;
                quad[3].color = c0;
            }
        }

        // Update 24 interior chrome slices at exact per-vertex (nx, ny, gx) coordinates
        // so adjacent slices share identical boundary colors with zero Mach banding.
        for &(v_start, v_end) in &self.title_slice_ranges {
            for quad in self.title_base_mesh.vertices[v_start..v_end]
                .as_chunks_mut::<4>()
                .0
            {
                let nx0 = ((quad[0].pos.x - bounds.min.x) * inv_w).clamp(0.0, 1.0);
                let nx1 = ((quad[1].pos.x - bounds.min.x) * inv_w).clamp(0.0, 1.0);
                let ny0 = ((quad[0].pos.y - bounds.min.y) * inv_h).clamp(0.0, 1.0);
                let ny1 = ((quad[2].pos.y - bounds.min.y) * inv_h).clamp(0.0, 1.0);

                quad[0].color = Self::evaluate_chrome_color(nx0, ny0, 0.0, time);
                quad[1].color = Self::evaluate_chrome_color(nx1, ny0, 1.0, time);
                quad[2].color = Self::evaluate_chrome_color(nx1, ny1, 1.0, time);
                quad[3].color = Self::evaluate_chrome_color(nx0, ny1, 0.0, time);
            }
        }
    }
}

/// Throttles formatted telemetry strings in the Stats panel to ~10 Hz (every 100 ms)
/// so `egui`'s `GalleyCache` achieves a >90% cache hit rate while making numbers
/// steady and readable for human eyes.
#[derive(Default)]
pub(crate) struct StatsDisplayCache {
    pub last_update_time: f64,
    pub initialized: bool,
    pub last_file_loaded: bool,
    pub fps_line: String,
    pub cpu_active_line: String,
    pub cpu_fft_line: String,
    pub gpu_compute_line: Option<String>,
    pub cpu_decode_line: String,
    pub video_decode_line: Option<String>,
    pub vis_shader_line: Option<String>,
    pub audio_buffer_line: String,
    pub audio_buffer_color: egui::Color32,
    pub video_buffer_line: Option<(String, egui::Color32)>,
    pub phase_lines: [(String, egui::Color32); 7],
    pub total_phases_line: String,
    pub backend_label: String,
    pub timeline_key: Option<(u64, u64, bool, bool)>,
    pub timeline_text: String,
    pub max_freq_key: u32,
    pub max_freq_label: String,
}

impl StatsDisplayCache {
    pub fn update_if_needed(
        &mut self,
        state: &crate::state::AppState,
        has_video_state: bool,
        now_time: f64,
    ) {
        if self.initialized
            && self.last_file_loaded == state.file_loaded
            && (now_time - self.last_update_time).abs() < 0.10
        {
            return;
        }
        self.initialized = true;
        self.last_file_loaded = state.file_loaded;
        self.last_update_time = now_time;

        self.fps_line = format!(
            "FPS: {:.1} ({:.2} ms)",
            state.current_fps,
            1000.0 / state.current_fps.max(1.0)
        );
        let cpu_active_us = state.stats.phase_lock_update_us
            + state.stats.phase_snapshot_us
            + state.stats.phase_egui_layout_us
            + state.stats.phase_encode_us
            + state.stats.phase_post_us;
        self.cpu_active_line = format!("CPU Active Total: {:.2} ms", cpu_active_us / 1000.0);
        self.cpu_fft_line = if state.video_mode == 3
            && (state.has_video_stream || state.video_frame_rx.is_some() || has_video_state)
        {
            "CPU FFT: Off (Fullscreen Video)".to_string()
        } else {
            format!("CPU FFT: {:.2} ms", state.stats.fft_us / 1000.0)
        };
        self.gpu_compute_line = if state.file_loaded && state.stats.gpu_compute_us > 0.0 {
            Some(format!(
                "GPU Compute: {:.2} ms",
                state.stats.gpu_compute_us / 1000.0
            ))
        } else {
            None
        };
        self.cpu_decode_line = format!("CPU Decode: {:.2} ms", state.stats.decode_us / 1000.0);
        self.video_decode_line = if state.file_loaded && state.stats.video_decode_us > 0.0 {
            Some(format!(
                "Video Decode: {:.2} ms",
                state.stats.video_decode_us / 1000.0
            ))
        } else {
            None
        };

        let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
        let mut total_vis_us = state.stats.shader_us;
        let has_gpu_sim_pass = (vis_def.requires_fire && vis_def.id != 5)
            || vis_def.id == 20
            || vis_def.requires_flip_water
            || vis_def.requires_acoustic_waves;
        if has_gpu_sim_pass {
            total_vis_us += state.stats.fire_us;
        }
        self.vis_shader_line = if state.file_loaded && total_vis_us > 0.0 {
            Some(format!(
                "Visualization Shader (GPU): {:.2} ms",
                total_vis_us / 1000.0
            ))
        } else {
            None
        };

        if !state.file_loaded {
            self.audio_buffer_line = "Audio Buffer: Idle".to_string();
            self.audio_buffer_color = egui::Color32::GRAY;
        } else {
            let buffer_label = if state.duration_seconds <= 0.0 {
                "Network Buffer"
            } else {
                "Audio Buffer"
            };
            self.audio_buffer_line = format!(
                "{}: {:.1}%",
                buffer_label, state.stats.audio_buffer_fill_pct
            );
            self.audio_buffer_color = if state.stats.audio_buffer_fill_pct < 5.0 {
                egui::Color32::RED
            } else if state.stats.audio_buffer_fill_pct > 95.0 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::GREEN
            };
        }

        self.video_buffer_line = if state.file_loaded && state.video_frame_rx.is_some() {
            let col = if state.stats.video_buffer_fill_pct < 1.0 {
                egui::Color32::RED
            } else {
                egui::Color32::YELLOW
            };
            Some((
                format!("Video Buffer: {:.1}%", state.stats.video_buffer_fill_pct),
                col,
            ))
        } else {
            None
        };

        let phases = [
            ("  Lock+Update", state.stats.phase_lock_update_us),
            ("  Snapshot", state.stats.phase_snapshot_us),
            ("  Surface Acq (VSync Wait)", state.stats.phase_surface_us),
            ("  UI Layout + Tess", state.stats.phase_egui_layout_us),
            ("  Cmd Record (CPU)", state.stats.phase_encode_us),
            ("  Submit + Present", state.stats.phase_submit_us),
            ("  Post Write", state.stats.phase_post_us),
        ];
        let total_phases: f32 = phases.iter().map(|(_, v)| v).sum();
        for (idx, (name, val)) in phases.iter().enumerate() {
            let color = if *name == "  Surface Acq (VSync Wait)" {
                egui::Color32::from_rgb(160, 160, 160)
            } else if *name == "  Submit + Present" {
                if *val > 4000.0 {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::from_rgb(160, 160, 160)
                }
            } else if *val > 2500.0 {
                egui::Color32::RED
            } else if *val > 1500.0 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::from_rgb(160, 160, 160)
            };
            self.phase_lines[idx] = (format!("{}: {:.2} ms", name, val / 1000.0), color);
        }
        self.total_phases_line = format!("  Total Frame Loop: {:.2} ms", total_phases / 1000.0);
    }
}
