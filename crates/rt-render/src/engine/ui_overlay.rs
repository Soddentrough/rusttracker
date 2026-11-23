use super::splash::gamepad_icon;
use super::{
    EngineAction, UiOverlayContext, VulkanEngine, format_vsync_status, is_vsync_present_mode,
};
use crate::state::AppState;

const TRAY_STROKE_W: f32 = 1.0;

pub(crate) const OPEN_BTN_DULL_ORANGE_TOP: egui::Color32 = egui::Color32::from_rgb(166, 104, 56);
pub(crate) const OPEN_BTN_DULL_ORANGE_MID: egui::Color32 = egui::Color32::from_rgb(146, 88, 44);
pub(crate) const OPEN_BTN_DULL_ORANGE_BOT: egui::Color32 = egui::Color32::from_rgb(115, 66, 30);

const OPEN_BTN_DULL_ORANGE_HOVER_TOP: egui::Color32 = egui::Color32::from_rgb(180, 116, 66);
const OPEN_BTN_DULL_ORANGE_HOVER_MID: egui::Color32 = egui::Color32::from_rgb(160, 100, 53);
const OPEN_BTN_DULL_ORANGE_HOVER_BOT: egui::Color32 = egui::Color32::from_rgb(128, 76, 36);

const OPEN_BTN_DULL_ORANGE_PRESSED_TOP: egui::Color32 = egui::Color32::from_rgb(110, 64, 28);
const OPEN_BTN_DULL_ORANGE_PRESSED_MID: egui::Color32 = egui::Color32::from_rgb(130, 78, 38);
const OPEN_BTN_DULL_ORANGE_PRESSED_BOT: egui::Color32 = egui::Color32::from_rgb(146, 90, 46);

pub(crate) const TOGGLE_ACTIVE_LIGHT_GREY_TOP: egui::Color32 =
    egui::Color32::from_rgb(196, 198, 202);
pub(crate) const TOGGLE_ACTIVE_LIGHT_GREY_MID: egui::Color32 =
    egui::Color32::from_rgb(176, 178, 183);
pub(crate) const TOGGLE_ACTIVE_LIGHT_GREY_BOT: egui::Color32 =
    egui::Color32::from_rgb(146, 148, 154);

pub(crate) const TOGGLE_INACTIVE_DARK_GREY_TOP: egui::Color32 = egui::Color32::from_rgb(82, 84, 89);
pub(crate) const TOGGLE_INACTIVE_DARK_GREY_MID: egui::Color32 = egui::Color32::from_rgb(69, 71, 76);
pub(crate) const TOGGLE_INACTIVE_DARK_GREY_BOT: egui::Color32 = egui::Color32::from_rgb(54, 56, 61);

struct Rounded3dButtonColors {
    top: egui::Color32,
    mid: egui::Color32,
    bot: egui::Color32,
    top_bevel: [u8; 4],
    bot_bevel: [u8; 4],
    border: [u8; 4],
    shadow: [u8; 4],
    shadow_offset_y: f32,
    text: egui::Color32,
    text_shadow: [u8; 4],
}

struct Rounded3dButtonPalette {
    idle: Rounded3dButtonColors,
    hover: Rounded3dButtonColors,
    pressed: Rounded3dButtonColors,
}

const OPEN_BTN_DULL_ORANGE_PALETTE: Rounded3dButtonPalette = Rounded3dButtonPalette {
    idle: Rounded3dButtonColors {
        top: OPEN_BTN_DULL_ORANGE_TOP,
        mid: OPEN_BTN_DULL_ORANGE_MID,
        bot: OPEN_BTN_DULL_ORANGE_BOT,
        top_bevel: [255, 224, 190, 68],
        bot_bevel: [34, 16, 4, 85],
        border: [48, 24, 10, 195],
        shadow: [0, 0, 0, 82],
        shadow_offset_y: 1.25,
        text: egui::Color32::from_rgb(246, 241, 234),
        text_shadow: [0, 0, 0, 110],
    },
    hover: Rounded3dButtonColors {
        top: OPEN_BTN_DULL_ORANGE_HOVER_TOP,
        mid: OPEN_BTN_DULL_ORANGE_HOVER_MID,
        bot: OPEN_BTN_DULL_ORANGE_HOVER_BOT,
        top_bevel: [255, 232, 206, 92],
        bot_bevel: [40, 20, 6, 78],
        border: [68, 36, 14, 205],
        shadow: [0, 0, 0, 95],
        shadow_offset_y: 1.25,
        text: egui::Color32::from_rgb(255, 251, 246),
        text_shadow: [0, 0, 0, 110],
    },
    pressed: Rounded3dButtonColors {
        top: OPEN_BTN_DULL_ORANGE_PRESSED_TOP,
        mid: OPEN_BTN_DULL_ORANGE_PRESSED_MID,
        bot: OPEN_BTN_DULL_ORANGE_PRESSED_BOT,
        top_bevel: [28, 12, 4, 90],
        bot_bevel: [255, 215, 175, 40],
        border: [42, 20, 8, 215],
        shadow: [0, 0, 0, 35],
        shadow_offset_y: 0.5,
        text: egui::Color32::from_rgb(236, 230, 222),
        text_shadow: [0, 0, 0, 110],
    },
};

const TOGGLE_ACTIVE_LIGHT_GREY_PALETTE: Rounded3dButtonPalette = Rounded3dButtonPalette {
    idle: Rounded3dButtonColors {
        top: TOGGLE_ACTIVE_LIGHT_GREY_TOP,
        mid: TOGGLE_ACTIVE_LIGHT_GREY_MID,
        bot: TOGGLE_ACTIVE_LIGHT_GREY_BOT,
        top_bevel: [255, 255, 255, 130],
        bot_bevel: [38, 40, 44, 85],
        border: [36, 38, 42, 200],
        shadow: [0, 0, 0, 88],
        shadow_offset_y: 1.25,
        text: egui::Color32::from_rgb(24, 25, 28),
        text_shadow: [255, 255, 255, 85],
    },
    hover: Rounded3dButtonColors {
        top: egui::Color32::from_rgb(204, 206, 210),
        mid: egui::Color32::from_rgb(184, 186, 191),
        bot: egui::Color32::from_rgb(154, 156, 162),
        top_bevel: [255, 255, 255, 150],
        bot_bevel: [38, 40, 44, 80],
        border: [42, 44, 48, 210],
        shadow: [0, 0, 0, 96],
        shadow_offset_y: 1.25,
        text: egui::Color32::from_rgb(18, 19, 22),
        text_shadow: [255, 255, 255, 95],
    },
    pressed: Rounded3dButtonColors {
        top: egui::Color32::from_rgb(148, 150, 156),
        mid: egui::Color32::from_rgb(164, 166, 171),
        bot: egui::Color32::from_rgb(176, 178, 183),
        top_bevel: [34, 36, 40, 82],
        bot_bevel: [255, 255, 255, 75],
        border: [32, 34, 38, 215],
        shadow: [0, 0, 0, 35],
        shadow_offset_y: 0.5,
        text: egui::Color32::from_rgb(28, 29, 33),
        text_shadow: [255, 255, 255, 65],
    },
};

const TOGGLE_INACTIVE_DARK_GREY_PALETTE: Rounded3dButtonPalette = Rounded3dButtonPalette {
    idle: Rounded3dButtonColors {
        top: TOGGLE_INACTIVE_DARK_GREY_TOP,
        mid: TOGGLE_INACTIVE_DARK_GREY_MID,
        bot: TOGGLE_INACTIVE_DARK_GREY_BOT,
        top_bevel: [255, 255, 255, 34],
        bot_bevel: [0, 0, 0, 78],
        border: [22, 23, 26, 200],
        shadow: [0, 0, 0, 52],
        shadow_offset_y: 0.65,
        text: egui::Color32::from_rgb(186, 188, 194),
        text_shadow: [0, 0, 0, 105],
    },
    hover: Rounded3dButtonColors {
        top: egui::Color32::from_rgb(98, 100, 106),
        mid: egui::Color32::from_rgb(84, 86, 92),
        bot: egui::Color32::from_rgb(66, 68, 74),
        top_bevel: [255, 255, 255, 58],
        bot_bevel: [0, 0, 0, 82],
        border: [30, 32, 36, 210],
        shadow: [0, 0, 0, 72],
        shadow_offset_y: 1.0,
        text: egui::Color32::from_rgb(228, 230, 236),
        text_shadow: [0, 0, 0, 115],
    },
    pressed: Rounded3dButtonColors {
        top: egui::Color32::from_rgb(52, 54, 58),
        mid: egui::Color32::from_rgb(64, 66, 70),
        bot: egui::Color32::from_rgb(74, 76, 82),
        top_bevel: [0, 0, 0, 88],
        bot_bevel: [255, 255, 255, 34],
        border: [18, 19, 22, 215],
        shadow: [0, 0, 0, 30],
        shadow_offset_y: 0.35,
        text: egui::Color32::from_rgb(212, 214, 220),
        text_shadow: [0, 0, 0, 95],
    },
};

fn open_style_tray_frame(corner_radius: f32, pad_l: i8, pad_r: i8, pad_v: i8) -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgba_unmultiplied(22, 22, 26, 130))
        .stroke(egui::Stroke::new(
            TRAY_STROKE_W,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 28),
        ))
        .corner_radius(corner_radius)
        .inner_margin(egui::Margin {
            left: pad_l,
            right: pad_r,
            top: pad_v,
            bottom: pad_v,
        })
}

#[inline]
fn lerp_color32(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    egui::Color32::from_rgba_unmultiplied(
        (a.r() as f32 * inv + b.r() as f32 * t).round() as u8,
        (a.g() as f32 * inv + b.g() as f32 * t).round() as u8,
        (a.b() as f32 * inv + b.b() as f32 * t).round() as u8,
        (a.a() as f32 * inv + b.a() as f32 * t).round() as u8,
    )
}

fn build_rounded_vertical_gradient_mesh(
    rect: egui::Rect,
    radius: f32,
    top: egui::Color32,
    mid: egui::Color32,
    bot: egui::Color32,
) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    let w = rect.width();
    let h = rect.height();
    if w <= 0.0 || h <= 0.0 {
        return mesh;
    }

    let r = radius.clamp(0.0, (w * 0.5).min(h * 0.5));
    let mut slices: Vec<(f32, f32, f32)> = Vec::with_capacity(15);
    let mut push_slice = |y: f32, inset: f32| {
        let xl = rect.left() + inset;
        let xr = (rect.right() - inset).max(xl);
        if slices
            .last()
            .is_none_or(|&(prev_y, _, _)| (y - prev_y).abs() > 1e-4)
        {
            slices.push((y, xl, xr));
        }
    };

    const CAP_SEGMENTS: usize = 6;
    for i in 0..=CAP_SEGMENTS {
        let angle = std::f32::consts::FRAC_PI_2 * (1.0 - i as f32 / CAP_SEGMENTS as f32);
        let y = rect.top() + r * (1.0 - angle.sin());
        let inset = r * (1.0 - angle.cos());
        push_slice(y, inset);
    }

    let y_mid = rect.top() + h * 0.45;
    if y_mid > rect.top() + r + 1e-4 && y_mid < rect.bottom() - r - 1e-4 {
        push_slice(y_mid, 0.0);
    }

    for i in 0..=CAP_SEGMENTS {
        let angle = std::f32::consts::FRAC_PI_2 * (i as f32 / CAP_SEGMENTS as f32);
        let y = rect.bottom() - r + r * angle.sin();
        let inset = r * (1.0 - angle.cos());
        push_slice(y, inset);
    }

    if slices.len() < 2 {
        return mesh;
    }

    let white_uv = egui::pos2(0.0, 0.0);
    mesh.vertices.reserve(slices.len() * 2);
    mesh.indices.reserve((slices.len() - 1) * 6);

    for &(y, xl, xr) in &slices {
        let t = ((y - rect.top()) / h).clamp(0.0, 1.0);
        let col = if t <= 0.45 {
            lerp_color32(top, mid, t / 0.45)
        } else {
            lerp_color32(mid, bot, (t - 0.45) / 0.55)
        };
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(xl, y),
            uv: white_uv,
            color: col,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(xr, y),
            uv: white_uv,
            color: col,
        });
    }

    for row in 0..(slices.len() - 1) {
        let i0 = (row * 2) as u32;
        let i1 = i0 + 1;
        let i2 = i0 + 2;
        let i3 = i0 + 3;
        mesh.indices.extend_from_slice(&[i0, i1, i3, i0, i3, i2]);
    }

    mesh
}

fn build_rounded_cap_arc(rect: egui::Rect, radius: f32, is_top: bool) -> Vec<egui::Pos2> {
    let inner = rect.shrink(1.25);
    if inner.width() <= 2.0 || inner.height() <= 2.0 {
        return Vec::new();
    }

    let r = (radius - 1.25).clamp(0.0, (inner.width() * 0.5).min(inner.height() * 0.5));
    if r < 0.25 {
        let y = if is_top { inner.top() } else { inner.bottom() };
        return vec![
            egui::pos2(inner.left() + 0.5, y),
            egui::pos2(inner.right() - 0.5, y),
        ];
    }

    let cx_left = inner.left() + r;
    let cx_right = inner.right() - r;
    let cy = if is_top {
        inner.top() + r
    } else {
        inner.bottom() - r
    };
    let y_sign = if is_top { -1.0 } else { 1.0 };

    const ARC_START: f32 = 0.60;
    const ARC_END: f32 = std::f32::consts::FRAC_PI_2;
    const ARC_SEGS: usize = 4;

    let mut pts = Vec::with_capacity((ARC_SEGS + 1) * 2);
    let mut push_pt = |pt: egui::Pos2| {
        if pts
            .last()
            .is_none_or(|p: &egui::Pos2| p.distance(pt) >= 0.05)
        {
            pts.push(pt);
        }
    };

    for k in 0..=ARC_SEGS {
        let t = k as f32 / ARC_SEGS as f32;
        let angle = ARC_START + (ARC_END - ARC_START) * t;
        push_pt(egui::pos2(
            cx_left - r * angle.cos(),
            cy + y_sign * r * angle.sin(),
        ));
    }
    for k in 0..=ARC_SEGS {
        let t = k as f32 / ARC_SEGS as f32;
        let angle = ARC_END - (ARC_END - ARC_START) * t;
        push_pt(egui::pos2(
            cx_right + r * angle.cos(),
            cy + y_sign * r * angle.sin(),
        ));
    }

    pts
}

fn render_3d_rounded_button(
    ui: &mut egui::Ui,
    label: &str,
    size: egui::Vec2,
    corner_radius: f32,
    font_size: f32,
    palette: &Rounded3dButtonPalette,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let is_pressed = response.is_pointer_button_down_on();
    let is_hovered = response.hovered() || response.has_focus();
    let colors = if is_pressed {
        &palette.pressed
    } else if is_hovered {
        &palette.hover
    } else {
        &palette.idle
    };

    let btn_rect = if is_pressed {
        rect.translate(egui::vec2(0.0, 0.75))
    } else {
        rect
    };

    #[inline]
    fn rgba(c: [u8; 4]) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
    }

    let painter = ui.painter();
    painter.rect_filled(
        btn_rect.translate(egui::vec2(0.0, colors.shadow_offset_y)),
        corner_radius,
        rgba(colors.shadow),
    );
    painter.rect_filled(btn_rect, corner_radius, colors.mid);

    let grad_mesh = build_rounded_vertical_gradient_mesh(
        btn_rect.shrink(0.5),
        (corner_radius - 0.5).max(0.0),
        colors.top,
        colors.mid,
        colors.bot,
    );
    if !grad_mesh.is_empty() {
        painter.add(egui::Shape::mesh(grad_mesh));
    }

    let top_arc = build_rounded_cap_arc(btn_rect, corner_radius, true);
    if top_arc.len() >= 2 {
        painter.add(egui::Shape::line(
            top_arc,
            egui::Stroke::new(1.0_f32, rgba(colors.top_bevel)),
        ));
    }
    let bot_arc = build_rounded_cap_arc(btn_rect, corner_radius, false);
    if bot_arc.len() >= 2 {
        painter.add(egui::Shape::line(
            bot_arc,
            egui::Stroke::new(1.0_f32, rgba(colors.bot_bevel)),
        ));
    }

    painter.rect(
        btn_rect,
        corner_radius,
        egui::Color32::TRANSPARENT,
        egui::Stroke::new(1.0_f32, rgba(colors.border)),
        egui::StrokeKind::Inside,
    );

    let font_id = egui::FontId::proportional(font_size);
    painter.text(
        btn_rect.center() + egui::vec2(0.0, 1.0),
        egui::Align2::CENTER_CENTER,
        label,
        font_id.clone(),
        rgba(colors.text_shadow),
    );
    painter.text(
        btn_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font_id,
        colors.text,
    );

    response
}

pub(crate) fn render_track_info_open_area(
    ui: &mut egui::Ui,
    is_portrait: bool,
    engine_action: &mut Vec<EngineAction>,
) -> (egui::Rect, Option<egui::Rect>, egui::Rect, bool) {
    struct OpenAreaLayout {
        show_open_label: bool,
        show_stream_btn: bool,
        btn_font: f32,
        btn_w: f32,
        pad_l: i8,
        pad_r: i8,
        div_gap: f32,
        btn_gap: f32,
        total_w: f32,
        open_galley: Option<std::sync::Arc<egui::Galley>>,
    }

    let btn_h = if is_portrait { 28.0 } else { 20.0 };
    let pad_v: i8 = if is_portrait { 3 } else { 2 };
    let btn_radius = if is_portrait { 6.5 } else { 5.5 };
    let area_radius = if is_portrait { 8.0 } else { 7.0 };
    let open_text_col = egui::Color32::from_rgb(208, 198, 186);

    #[derive(Clone, Copy)]
    struct OpenAreaTierSpec {
        show_open_label: bool,
        show_stream_btn: bool,
        btn_font: f32,
        open_font: f32,
        h_pad: f32,
        pad_l: i8,
        pad_r: i8,
        div_gap: f32,
        btn_gap: f32,
    }

    let candidates: [OpenAreaTierSpec; 4] = [
        // Standard tier
        OpenAreaTierSpec {
            show_open_label: true,
            show_stream_btn: true,
            btn_font: if is_portrait { 11.5 } else { 11.0 },
            open_font: if is_portrait { 10.5 } else { 10.0 },
            h_pad: if is_portrait { 9.0 } else { 8.0 },
            pad_l: 7,
            pad_r: 4,
            div_gap: 5.0,
            btn_gap: 5.0,
        },
        // Compact tier
        OpenAreaTierSpec {
            show_open_label: true,
            show_stream_btn: true,
            btn_font: 10.0,
            open_font: 9.5,
            h_pad: 5.5,
            pad_l: 5,
            pad_r: 3,
            div_gap: 3.5,
            btn_gap: 4.0,
        },
        // Tight tier
        OpenAreaTierSpec {
            show_open_label: false,
            show_stream_btn: true,
            btn_font: 9.5,
            open_font: 9.5,
            h_pad: 4.5,
            pad_l: 3,
            pad_r: 3,
            div_gap: 0.0,
            btn_gap: 3.0,
        },
        // Fallback tier
        OpenAreaTierSpec {
            show_open_label: false,
            show_stream_btn: false,
            btn_font: 9.5,
            open_font: 9.5,
            h_pad: 4.5,
            pad_l: 3,
            pad_r: 3,
            div_gap: 0.0,
            btn_gap: 0.0,
        },
    ];

    let measure_tier = |tier: OpenAreaTierSpec| -> OpenAreaLayout {
        let OpenAreaTierSpec {
            show_open_label,
            show_stream_btn,
            btn_font,
            open_font,
            h_pad,
            pad_l,
            pad_r,
            div_gap,
            btn_gap,
        } = tier;
        let stream_w = ui
            .painter()
            .layout_no_wrap(
                "STREAM/DEV".to_string(),
                egui::FontId::proportional(btn_font),
                egui::Color32::WHITE,
            )
            .size()
            .x;
        let file_w = ui
            .painter()
            .layout_no_wrap(
                "FILE/BLOCK".to_string(),
                egui::FontId::proportional(btn_font),
                egui::Color32::WHITE,
            )
            .size()
            .x;
        let btn_w = (stream_w.max(file_w) + h_pad * 2.0).ceil();

        let open_galley = if show_open_label {
            Some(ui.painter().layout_no_wrap(
                "OPEN".to_string(),
                egui::FontId::proportional(open_font),
                open_text_col,
            ))
        } else {
            None
        };

        let label_div_w = if let Some(ref g) = open_galley {
            g.size().x + div_gap * 2.0 + 1.0
        } else {
            0.0
        };
        let buttons_w = if show_stream_btn {
            btn_w * 2.0 + btn_gap
        } else {
            btn_w
        };
        let total_w = pad_l as f32 + label_div_w + buttons_w + pad_r as f32 + TRAY_STROKE_W * 2.0;

        OpenAreaLayout {
            show_open_label,
            show_stream_btn,
            btn_font,
            btn_w,
            pad_l,
            pad_r,
            div_gap,
            btn_gap,
            total_w,
            open_galley,
        }
    };

    let avail = ui.available_width();
    let mut chosen: Option<OpenAreaLayout> = None;
    for (idx, &tier) in candidates.iter().enumerate() {
        let measured = measure_tier(tier);
        if measured.total_w <= avail || idx + 1 == candidates.len() {
            chosen = Some(measured);
            break;
        }
    }
    let chosen = chosen.unwrap();

    let spacer = (avail - chosen.total_w).max(0.0);
    if spacer > 0.0 {
        ui.add_space(spacer);
    }

    let mut stream_rect: Option<egui::Rect> = None;
    let mut file_rect = egui::Rect::NOTHING;

    let frame_resp =
        open_style_tray_frame(area_radius, chosen.pad_l, chosen.pad_r, pad_v).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;

                if let Some(open_galley) = chosen.open_galley {
                    let open_size = open_galley.size();
                    let (open_rect, _) = ui
                        .allocate_exact_size(egui::vec2(open_size.x, btn_h), egui::Sense::hover());
                    ui.painter().galley(
                        egui::pos2(open_rect.left(), open_rect.center().y - open_size.y * 0.5),
                        open_galley,
                        open_text_col,
                    );

                    ui.add_space(chosen.div_gap);
                    let (div_rect, _) =
                        ui.allocate_exact_size(egui::vec2(1.0, btn_h), egui::Sense::hover());
                    let div_h = (btn_h - 6.0).max(10.0);
                    let div_x = div_rect.center().x;
                    ui.painter().line_segment(
                        [
                            egui::pos2(div_x, div_rect.center().y - div_h * 0.5),
                            egui::pos2(div_x, div_rect.center().y + div_h * 0.5),
                        ],
                        egui::Stroke::new(
                            1.0_f32,
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 34),
                        ),
                    );
                    ui.add_space(chosen.div_gap);
                }

                if chosen.show_stream_btn {
                    let stream_resp = render_3d_rounded_button(
                        ui,
                        "STREAM/DEV",
                        egui::vec2(chosen.btn_w, btn_h),
                        btn_radius,
                        chosen.btn_font,
                        &OPEN_BTN_DULL_ORANGE_PALETTE,
                    )
                    .on_hover_text(
                        "Open network stream URL or local audio input device (microphone)",
                    );
                    stream_rect = Some(stream_resp.rect);
                    if stream_resp.clicked() {
                        engine_action.push(EngineAction::OpenUrlDialog);
                    }
                    ui.add_space(chosen.btn_gap);
                }

                let file_resp = render_3d_rounded_button(
                    ui,
                    "FILE/BLOCK",
                    egui::vec2(chosen.btn_w, btn_h),
                    btn_radius,
                    chosen.btn_font,
                    &OPEN_BTN_DULL_ORANGE_PALETTE,
                )
                .on_hover_text(
                    "Open audio/video file, playlist, or disc/ISO block image (Shortcut: 'o')",
                );
                file_rect = file_resp.rect;
                if file_resp.clicked() {
                    engine_action.push(EngineAction::OpenFile);
                }
            });
        });

    (
        frame_resp.response.rect,
        stream_rect,
        file_rect,
        chosen.show_open_label,
    )
}

pub(crate) fn render_heatmap_eq_header(
    ui: &mut egui::Ui,
    show_surround_eq: bool,
    has_lyrics: bool,
    has_tracker_patterns: bool,
    hardware_channels: i32,
    engine_action: &mut Vec<EngineAction>,
) -> (egui::Rect, egui::Rect, egui::Rect, Option<egui::Rect>, bool) {
    struct ToggleLayout<'a> {
        left_label: &'a str,
        right_label: &'a str,
        btn_font: f32,
        btn_w: f32,
        pad_h: i8,
        btn_gap: f32,
    }

    const BTN_H: f32 = 20.0;
    const PAD_V: i8 = 2;
    const BTN_RADIUS: f32 = 5.5;
    const AREA_RADIUS: f32 = 7.0;
    const RESET_FONT: f32 = 10.5;
    let tray_h = BTN_H + (PAD_V as f32) * 2.0 + TRAY_STROKE_W * 2.0;

    ui.spacing_mut().item_spacing.x = 0.0;

    let left_full = if has_lyrics {
        "LYRICS"
    } else if has_tracker_patterns {
        "TRACKER PATTERN"
    } else {
        "PATTERN HEATMAP"
    };
    let left_short = if has_lyrics {
        "LYRICS"
    } else if has_tracker_patterns {
        "PATTERN"
    } else {
        "HEATMAP"
    };
    let right_full = if hardware_channels > 2 {
        format!(
            "SURROUND EQ ({})",
            crate::state::SurroundUpmixConfig::short_tag(hardware_channels)
        )
    } else {
        "SURROUND EQ".to_string()
    };
    let right_mid = "SURROUND EQ";
    let right_short = "EQ";

    let reset_text_w = ui
        .painter()
        .layout_no_wrap(
            "RESET".to_string(),
            egui::FontId::proportional(RESET_FONT),
            egui::Color32::WHITE,
        )
        .size()
        .x;
    let reset_w = (reset_text_w + 14.0).ceil();
    let reset_reserve = if show_surround_eq { reset_w + 6.0 } else { 0.0 };

    let avail = ui.available_width();
    let tray_budget = (avail - reset_reserve).max(110.0);

    let candidates: [(&str, &str, f32, f32, i8, f32); 4] = [
        (left_full, right_full.as_str(), 11.0, 8.0, 3, 3.0),
        (left_full, right_mid, 11.0, 7.0, 3, 3.0),
        (left_short, right_mid, 10.0, 6.0, 2, 2.0),
        (left_short, right_short, 9.5, 5.0, 2, 2.0),
    ];

    let mut chosen: Option<ToggleLayout<'_>> = None;
    for (idx, &(left_label, right_label, btn_font, h_pad, pad_h, btn_gap)) in
        candidates.iter().enumerate()
    {
        let left_w = ui
            .painter()
            .layout_no_wrap(
                left_label.to_string(),
                egui::FontId::proportional(btn_font),
                egui::Color32::WHITE,
            )
            .size()
            .x;
        let right_w = ui
            .painter()
            .layout_no_wrap(
                right_label.to_string(),
                egui::FontId::proportional(btn_font),
                egui::Color32::WHITE,
            )
            .size()
            .x;
        let btn_w = (left_w.max(right_w) + h_pad * 2.0).ceil();
        let tray_w = pad_h as f32 * 2.0 + btn_w * 2.0 + btn_gap + TRAY_STROKE_W * 2.0;
        if tray_w <= tray_budget || idx + 1 == candidates.len() {
            chosen = Some(ToggleLayout {
                left_label,
                right_label,
                btn_font,
                btn_w,
                pad_h,
                btn_gap,
            });
            break;
        }
    }
    let chosen = chosen.unwrap();

    let mut pattern_btn_rect = egui::Rect::NOTHING;
    let mut eq_btn_rect = egui::Rect::NOTHING;

    let tray_resp =
        open_style_tray_frame(AREA_RADIUS, chosen.pad_h, chosen.pad_h, PAD_V).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;

                let pattern_palette = if show_surround_eq {
                    &TOGGLE_INACTIVE_DARK_GREY_PALETTE
                } else {
                    &TOGGLE_ACTIVE_LIGHT_GREY_PALETTE
                };
                let pattern_resp = render_3d_rounded_button(
                    ui,
                    chosen.left_label,
                    egui::vec2(chosen.btn_w, BTN_H),
                    BTN_RADIUS,
                    chosen.btn_font,
                    pattern_palette,
                )
                .on_hover_text("Show Tracker Pattern / Pattern Heatmap / Lyrics viewer");
                pattern_btn_rect = pattern_resp.rect;
                if pattern_resp.clicked() && show_surround_eq {
                    engine_action.push(EngineAction::SetShowSurroundEq(false));
                }

                ui.add_space(chosen.btn_gap);

                let eq_palette = if show_surround_eq {
                    &TOGGLE_ACTIVE_LIGHT_GREY_PALETTE
                } else {
                    &TOGGLE_INACTIVE_DARK_GREY_PALETTE
                };
                let eq_resp = render_3d_rounded_button(
                    ui,
                    chosen.right_label,
                    egui::vec2(chosen.btn_w, BTN_H),
                    BTN_RADIUS,
                    chosen.btn_font,
                    eq_palette,
                )
                .on_hover_text("Show Surround Upmix Crossover & 5-band Parametric EQ controls");
                eq_btn_rect = eq_resp.rect;
                if eq_resp.clicked() && !show_surround_eq {
                    engine_action.push(EngineAction::SetShowSurroundEq(true));
                }
            });
        });

    let mut reset_btn_rect: Option<egui::Rect> = None;
    let mut reset_clicked = false;

    if show_surround_eq {
        let rem = ui.available_width();
        let has_height = crate::state::SurroundUpmixConfig::has_height_channels(hardware_channels);
        let mut legend_entries: Vec<(std::sync::Arc<egui::Galley>, egui::Color32)> =
            Vec::with_capacity(3);
        let mut push_legend = |txt: &str, col: egui::Color32| {
            let g =
                ui.painter()
                    .layout_no_wrap(txt.to_string(), egui::FontId::proportional(10.0), col);
            legend_entries.push((g, col));
        };
        push_legend("• LFE", egui::Color32::from_rgb(220, 68, 32));
        push_legend("• Bed", egui::Color32::from_rgb(255, 165, 35));
        if has_height {
            push_legend("• Height ↑", egui::Color32::from_rgb(255, 225, 130));
        }
        let legend_gap = 5.0;
        let legend_w: f32 = legend_entries.iter().map(|(g, _)| g.size().x).sum::<f32>()
            + legend_gap * legend_entries.len().saturating_sub(1) as f32;
        let show_legend = legend_w + 8.0 + reset_w <= rem;
        let right_w = if show_legend {
            legend_w + 8.0 + reset_w
        } else {
            reset_w
        };
        let spacer = (rem - right_w).max(0.0);
        if spacer > 0.0 {
            ui.add_space(spacer);
        }

        if show_legend {
            let count = legend_entries.len();
            for (idx, (g, col)) in legend_entries.into_iter().enumerate() {
                let g_size = g.size();
                let (l_rect, _) =
                    ui.allocate_exact_size(egui::vec2(g_size.x, tray_h), egui::Sense::hover());
                ui.painter().galley(
                    egui::pos2(l_rect.left(), l_rect.center().y - g_size.y * 0.5),
                    g,
                    col,
                );
                if idx + 1 < count {
                    ui.add_space(legend_gap);
                }
            }
            ui.add_space(8.0);
        }

        let reset_resp = render_3d_rounded_button(
            ui,
            "RESET",
            egui::vec2(reset_w, BTN_H),
            BTN_RADIUS,
            RESET_FONT,
            &TOGGLE_INACTIVE_DARK_GREY_PALETTE,
        )
        .on_hover_text("Reset surround routing bands, crossovers, and speaker levels to defaults");
        reset_btn_rect = Some(reset_resp.rect);
        reset_clicked = reset_resp.clicked();
    }

    (
        tray_resp.response.rect,
        pattern_btn_rect,
        eq_btn_rect,
        reset_btn_rect,
        reset_clicked,
    )
}

impl VulkanEngine {
    pub(in crate::engine) fn build_ui_overlay(
        &mut self,
        ctx: &mut egui::Ui,
        state: &AppState,
        file_dialog: &mut egui_file_dialog::FileDialog,
        overlay: UiOverlayContext<'_>,
    ) {
        let UiOverlayContext {
            central_rect,
            engine_actions,
            out_meters_rect,
            out_fire_rect,
            out_heatmap_rect,
            out_track_info_rect,
            out_top_panel_rect,
            out_video_rect,
            vis_name,
            video_info_str,
        } = overlay;
        if state.is_url_dialog_open {
            let url_modal_w = (ctx.viewport_rect().width() - 24.0).clamp(260.0, 600.0);
            let url_modal_max_h = (ctx.viewport_rect().height() - 32.0).clamp(240.0, 500.0);
            egui::Window::new("Open Stream / Device")
                .collapsible(false)
                .resizable(false)
                .default_width(url_modal_w)
                .min_width(url_modal_w)
                .max_width(url_modal_w)
                .max_height(url_modal_max_h)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("Network Stream / URL:").strong());
                    ui.label("Enter Stream URL (e.g. .pls, .m3u, or direct stream link):");
                    ui.add_space(4.0);
                    let mut url_text = state.url_input_text.clone();
                    let text_resp = ui.add(
                        egui::TextEdit::singleline(&mut url_text)
                            .desired_width(f32::INFINITY)
                            .min_size(egui::vec2(0.0, 30.0)),
                    );
                    if state.focus_url_input {
                        text_resp.request_focus();
                        engine_actions.push(EngineAction::ClearFocusUrlInput);
                    }
                    if text_resp.changed() {
                        engine_actions.push(EngineAction::SetUrlInput(url_text.clone()));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if (ui
                                .add_sized([80.0, 30.0], egui::Button::new("Open"))
                                .clicked()
                                || (text_resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                                && !url_text.is_empty()
                            {
                                engine_actions.push(EngineAction::LoadUrl(url_text));
                            }
                            if ui
                                .add_sized([80.0, 30.0], egui::Button::new("Cancel"))
                                .clicked()
                            {
                                engine_actions.push(EngineAction::CloseUrlDialog);
                            }
                        });
                    });

                    ui.separator();
                    ui.label(
                        egui::RichText::new("Audio Input Devices (Microphone / Line-In):").strong(),
                    );
                    ui.add_space(4.0);
                    let fallback_devs = [crate::audio::SYSTEM_DEFAULT_AUDIO_DEVICE.to_string()];
                    let input_devs: &[String] = if !state.available_input_devices.is_empty() {
                        &state.available_input_devices
                    } else {
                        &fallback_devs
                    };
                    egui::ScrollArea::vertical()
                        .id_salt("url_dialog_input_devices_scroll")
                        .max_height(130.0)
                        .show(ui, |ui| {
                            for dev in input_devs {
                                let is_active_mic = state.is_mic_input
                                    && state.selected_audio_device.as_deref() == Some(dev.as_str());
                                let btn_label = if is_active_mic {
                                    format!("● {} (Active)", dev)
                                } else {
                                    format!("• {}", dev)
                                };
                                let width = ui.available_width();
                                let mut btn = egui::Button::new(
                                    egui::RichText::new(btn_label).color(if is_active_mic {
                                        egui::Color32::from_rgb(220, 255, 230)
                                    } else {
                                        egui::Color32::from_rgb(230, 220, 250)
                                    }),
                                )
                                .truncate();
                                if is_active_mic {
                                    btn = btn.fill(egui::Color32::from_rgb(30, 95, 60)).stroke(
                                        egui::Stroke::new(
                                            1.0_f32,
                                            egui::Color32::from_rgb(80, 220, 130),
                                        ),
                                    );
                                }
                                if ui.add_sized([width.max(0.0), 30.0], btn).clicked() {
                                    engine_actions.push(EngineAction::LoadInputDevice(dev.clone()));
                                }
                            }
                        });

                    if !state.url_history.is_empty() {
                        ui.separator();
                        ui.label(egui::RichText::new("Recent Streams:").strong());
                        egui::ScrollArea::vertical()
                            .id_salt("url_dialog_recent_streams_scroll")
                            .max_height(120.0)
                            .show(ui, |ui| {
                                for (url, title) in state.url_history.iter().rev() {
                                    ui.horizontal(|ui| {
                                        let width = ui.available_width() - 38.0;
                                        let btn = egui::Button::new(format!("{} ({})", title, url))
                                            .truncate();
                                        if ui.add_sized([width.max(0.0), 30.0], btn).clicked() {
                                            engine_actions.push(EngineAction::LoadUrl(url.clone()));
                                        }
                                        if ui
                                            .add_sized([30.0, 30.0], egui::Button::new("📝"))
                                            .clicked()
                                        {
                                            engine_actions.push(EngineAction::EditUrl(url.clone()));
                                        }
                                    });
                                }
                            });
                    }
                });
        }

        if state.show_stats {
            self.stats_cache
                .update_if_needed(state, self.video_state.is_some(), self.smooth_time);
            let viewport_rect = ctx.viewport_rect();
            let stats_top_y =
                if viewport_rect.width() < viewport_rect.height() || cfg!(target_os = "android") {
                    48.0
                } else if state.file_loaded && state.show_hud && state.video_mode != 3 {
                    let total_h = viewport_rect.height();
                    let min_h = 120.0f32;
                    let top_h = (total_h * state.panel_split_ratio)
                        .clamp(min_h, (total_h - min_h).max(min_h));
                    if total_h - top_h >= 240.0 {
                        top_h + 10.0
                    } else {
                        10.0
                    }
                } else {
                    10.0
                };
            let max_stats_h = (viewport_rect.height() - stats_top_y - 16.0).max(160.0);
            let max_stats_w = (viewport_rect.width() - 20.0).clamp(240.0, 440.0);
            egui::Window::new("Stats")
                .anchor(egui::Align2::RIGHT_TOP, [-10.0, stats_top_y])
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .max_width(max_stats_w)
                .max_height(max_stats_h)
                .frame(
                    egui::Frame::window(&ctx.global_style())
                        .fill(egui::Color32::from_black_alpha(210))
                        .inner_margin(egui::Margin::symmetric(10, 8)),
                )
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.5;
                    egui::ScrollArea::vertical()
                        .max_height(max_stats_h - 16.0)
                        .show(ui, |ui| {
                            let vsync_on = is_vsync_present_mode(self.config.present_mode);
                            let vsync_str = format_vsync_status(self.config.present_mode);
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "RustTracker v{}",
                                        env!("CARGO_PKG_VERSION")
                                    ))
                                    .color(egui::Color32::WHITE)
                                    .strong(),
                                );
                                ui.label(
                                    egui::RichText::new("•").color(egui::Color32::from_gray(100)),
                                );
                                ui.label(
                                    egui::RichText::new(vsync_str)
                                        .color(if vsync_on {
                                            egui::Color32::GREEN
                                        } else {
                                            egui::Color32::YELLOW
                                        })
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new("•").color(egui::Color32::from_gray(100)),
                                );
                                if self.stats_cache.backend_label.is_empty() {
                                    self.stats_cache.backend_label =
                                        format!("{:?}", self.adapter_info.backend);
                                }
                                ui.label(
                                    egui::RichText::new(&self.stats_cache.backend_label)
                                        .color(egui::Color32::from_rgb(130, 210, 255))
                                        .strong(),
                                );
                            });
                            if state.file_loaded {
                                ui.label(
                                    egui::RichText::new(format!("Visualizer: {}", vis_name))
                                        .color(egui::Color32::YELLOW),
                                );
                            }
                            ui.separator();

                            if state.stats.mpegh_bitstream_active {
                                ui.label(
                                    egui::RichText::new("MPEG-H 3D Audio 7.1.4 (11.1): ACTIVE")
                                        .color(egui::Color32::from_rgb(0, 235, 255))
                                        .strong(),
                                );
                                if !state.stats.mpegh_carrier_info.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&state.stats.mpegh_carrier_info)
                                            .color(egui::Color32::from_rgb(165, 235, 255))
                                            .size(11.5),
                                    );
                                }
                                ui.separator();
                            } else if state.stats.bitstream_active {
                                ui.label(
                                    egui::RichText::new("Bitstream Passthrough: ACTIVE")
                                        .color(egui::Color32::from_rgb(0, 255, 128))
                                        .strong(),
                                );
                                if let Some(info) = &state.video_info {
                                    ui.label(
                                        egui::RichText::new(format!("Format: {}", info))
                                            .color(egui::Color32::LIGHT_GREEN),
                                    );
                                }
                                ui.separator();
                            } else if state.passthrough_enabled
                                && cfg!(any(target_os = "windows", target_os = "linux"))
                            {
                                ui.label(
                                    egui::RichText::new("Bitstream Passthrough: Inactive")
                                        .color(egui::Color32::GRAY),
                                );
                                ui.separator();
                            }

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&self.stats_cache.fps_line)
                                        .color(egui::Color32::GREEN)
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new("•").color(egui::Color32::from_gray(100)),
                                );
                                ui.label(
                                    egui::RichText::new(&self.stats_cache.cpu_active_line)
                                        .color(egui::Color32::WHITE),
                                );
                            });

                            if state.file_loaded {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        egui::RichText::new(&self.stats_cache.cpu_fft_line)
                                            .color(egui::Color32::WHITE),
                                    );
                                    ui.label(
                                        egui::RichText::new("•")
                                            .color(egui::Color32::from_gray(100)),
                                    );
                                    ui.label(
                                        egui::RichText::new(&self.stats_cache.cpu_decode_line)
                                            .color(egui::Color32::WHITE),
                                    );
                                    if let Some(ref vid_dec) = self.stats_cache.video_decode_line {
                                        ui.label(
                                            egui::RichText::new("•")
                                                .color(egui::Color32::from_gray(100)),
                                        );
                                        ui.label(
                                            egui::RichText::new(vid_dec)
                                                .color(egui::Color32::WHITE),
                                        );
                                    }
                                });
                                if self.stats_cache.gpu_compute_line.is_some()
                                    || self.stats_cache.vis_shader_line.is_some()
                                {
                                    ui.horizontal_wrapped(|ui| {
                                        if let Some(ref gpu_comp) =
                                            self.stats_cache.gpu_compute_line
                                        {
                                            ui.label(
                                                egui::RichText::new(gpu_comp)
                                                    .color(egui::Color32::LIGHT_BLUE),
                                            );
                                        }
                                        if let Some(ref vis_line) = self.stats_cache.vis_shader_line
                                        {
                                            ui.label(
                                                egui::RichText::new(vis_line)
                                                    .color(egui::Color32::LIGHT_BLUE),
                                            );
                                        }
                                    });
                                }
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&self.stats_cache.audio_buffer_line)
                                            .color(self.stats_cache.audio_buffer_color),
                                    );
                                    if let Some((ref vbuf_line, vbuf_col)) =
                                        self.stats_cache.video_buffer_line
                                    {
                                        ui.label(
                                            egui::RichText::new("•")
                                                .color(egui::Color32::from_gray(100)),
                                        );
                                        ui.label(egui::RichText::new(vbuf_line).color(vbuf_col));
                                    }
                                });
                            }
                            ui.separator();
                            ui.label(
                                egui::RichText::new("⏱ Frame Phase Breakdown:")
                                    .color(egui::Color32::from_rgb(180, 180, 255))
                                    .strong()
                                    .size(12.0),
                            );
                            egui::Grid::new("stats_phases_grid")
                                .num_columns(2)
                                .spacing([12.0, 2.0])
                                .show(ui, |ui| {
                                    for (idx, (phase_text, color)) in
                                        self.stats_cache.phase_lines.iter().enumerate()
                                    {
                                        ui.label(
                                            egui::RichText::new(phase_text.trim_start())
                                                .color(*color)
                                                .size(11.5),
                                        );
                                        if idx % 2 == 1 {
                                            ui.end_row();
                                        }
                                    }
                                    if self.stats_cache.phase_lines.len() % 2 == 1 {
                                        ui.end_row();
                                    }
                                });
                            ui.label(
                                egui::RichText::new(&self.stats_cache.total_phases_line)
                                    .color(egui::Color32::from_rgb(180, 180, 255))
                                    .size(12.0),
                            );
                            if state.file_loaded {
                                ui.horizontal(|ui| {
                                    let src_ch = state.source_channels.max(1);
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Channels: {} hw / {} src",
                                            state.hardware_channels, src_ch
                                        ))
                                        .color(if state.hardware_channels != src_ch {
                                            egui::Color32::YELLOW
                                        } else {
                                            egui::Color32::GRAY
                                        })
                                        .size(12.0),
                                    );
                                    ui.label(
                                        egui::RichText::new("•")
                                            .color(egui::Color32::from_gray(100)),
                                    );
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Clips: {}",
                                            state.stats.clipping_events
                                        ))
                                        .color(if state.stats.clipping_events > 0 {
                                            egui::Color32::RED
                                        } else {
                                            egui::Color32::GRAY
                                        })
                                        .size(12.0),
                                    );
                                });
                                if video_info_str.is_none()
                                    && let Some(abr) =
                                        state.stats.audio_bitrate_kbps.or(state.bitrate)
                                {
                                    let abr_str = if abr >= 1000 {
                                        format!(
                                            "Audio Bitrate: {} kbps ({:.2} Mbps)",
                                            abr,
                                            abr as f32 / 1000.0
                                        )
                                    } else {
                                        format!("Audio Bitrate: {} kbps", abr)
                                    };
                                    ui.label(
                                        egui::RichText::new(abr_str)
                                            .color(egui::Color32::LIGHT_GREEN)
                                            .size(12.0),
                                    );
                                }
                            }
                            if let Some(vi) = &video_info_str {
                                ui.separator();
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        egui::RichText::new("Video:")
                                            .color(egui::Color32::GRAY)
                                            .size(12.0),
                                    );
                                    ui.label(
                                        egui::RichText::new(vi)
                                            .color(egui::Color32::YELLOW)
                                            .size(12.0),
                                    );
                                });
                                ui.horizontal_wrapped(|ui| {
                                    if state.stats.video_fps > 0.0 {
                                        let fps_str = if (state.stats.video_fps
                                            - state.stats.video_fps.round())
                                        .abs()
                                            < 0.05
                                        {
                                            format!("FPS: {:.0}", state.stats.video_fps.round())
                                        } else {
                                            format!("FPS: {:.2}", state.stats.video_fps)
                                        };
                                        ui.label(
                                            egui::RichText::new(fps_str)
                                                .color(egui::Color32::LIGHT_GREEN)
                                                .size(12.0),
                                        );
                                    }
                                    if let Some(vbr) = state.stats.video_bitrate_kbps {
                                        let vbr_str = if vbr >= 1000 {
                                            format!("• Video: {:.2} Mbps", vbr as f32 / 1000.0)
                                        } else {
                                            format!("• Video: {} kbps", vbr)
                                        };
                                        ui.label(
                                            egui::RichText::new(vbr_str)
                                                .color(egui::Color32::LIGHT_GREEN)
                                                .size(12.0),
                                        );
                                    }
                                    if let Some(abr) =
                                        state.stats.audio_bitrate_kbps.or(state.bitrate)
                                    {
                                        let abr_str = if abr >= 1000 {
                                            format!("• Audio: {:.2} Mbps", abr as f32 / 1000.0)
                                        } else {
                                            format!("• Audio: {} kbps", abr)
                                        };
                                        ui.label(
                                            egui::RichText::new(abr_str)
                                                .color(egui::Color32::LIGHT_GREEN)
                                                .size(12.0),
                                        );
                                    }
                                });
                            }
                        });
                });
        }

        if state.show_help {
            egui::Window::new("Help")
                .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .frame(
                    egui::Frame::window(&ctx.global_style())
                        .fill(egui::Color32::from_black_alpha(200)),
                )
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new("Shortcuts")
                            .color(egui::Color32::WHITE)
                            .strong()
                            .size(16.0),
                    );
                    ui.separator();
                    egui::Grid::new("help_shortcuts_grid")
                        .num_columns(2)
                        .spacing([20.0, 6.0])
                        .show(ui, |ui| {
                            let ctrl_mod = if std::env::consts::OS == "macos" {
                                "⌘"
                            } else {
                                "ctrl+"
                            };
                            let shortcut =
                                |ui: &mut egui::Ui, key: &str, gp: Option<&str>, desc: &str| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        if !key.is_empty() {
                                            ui.label(
                                                egui::RichText::new(key)
                                                    .color(egui::Color32::WHITE)
                                                    .strong(),
                                            );
                                        }
                                        if let Some(gp_act) = gp {
                                            if !key.is_empty() {
                                                ui.label(
                                                    egui::RichText::new(" / ")
                                                        .color(egui::Color32::DARK_GRAY),
                                                );
                                            }
                                            ui.label(
                                                egui::RichText::new(gamepad_icon(
                                                    state.gamepad_type,
                                                    gp_act,
                                                ))
                                                .color(egui::Color32::LIGHT_BLUE)
                                                .size(16.0),
                                            );
                                        }
                                    });
                                    ui.label(egui::RichText::new(desc).color(egui::Color32::GRAY));
                                    ui.end_row();
                                };
                            shortcut(ui, "o", Some("Y"), "Open File");
                            shortcut(ui, "space", Some("A"), "Play / Pause");
                            shortcut(ui, "v", Some("X"), "Toggle Video / View");
                            shortcut(ui, "m", None, "Visualizer Modules");
                            shortcut(ui, "left/right", Some("D-Pad L/R"), "Seek Timeline");
                            shortcut(ui, "", Some("L2 / R2"), "Analog Scrub");
                            shortcut(ui, ", / .", None, "Step Frame");
                            shortcut(ui, "tab", Some("L3"), "Toggle HUD");
                            shortcut(ui, "up/down", Some("D-Pad U/D"), "Cycle Visualizer");
                            shortcut(ui, "s", Some("B"), "Toggle Stats");
                            shortcut(ui, "h", None, "Toggle Help");
                            shortcut(ui, &format!("{}q / esc", ctrl_mod), Some("Select"), "Quit");
                            shortcut(ui, "f", Some("Start"), "Toggle Fullscreen");
                            shortcut(ui, "u / e", Some("R3"), "Surround Upmix");
                            shortcut(ui, "p / n", Some("L1 / R1"), "Prev / Next Track");
                            shortcut(ui, "1-9", None, "Select Audio Track");
                            shortcut(ui, "[ / ]", Some("L/R Stick"), "Scale Panels");
                            if state.visualizer_mode == 20 {
                                shortcut(ui, "c", None, "Toggle Camera Mode");
                            } else if state.visualizer_mode == 27 {
                                shortcut(ui, "c", None, "Toggle Ferrofluid View");
                            } else if state.visualizer_mode == 31 {
                                shortcut(ui, "c", None, "Toggle Smoke Volume");
                            }
                        });
                });
        }

        if state.show_vis_picker {
            let screen_rect = ctx.viewport_rect();
            let modal_w = (screen_rect.width() - 24.0).clamp(260.0, 540.0);
            let modal_h = (screen_rect.height() - 64.0).clamp(280.0, 500.0);
            let is_narrow_modal = modal_w < 440.0;
            let margin_px = if is_narrow_modal { 12.0 } else { 20.0 };
            let scroll_max_h = (modal_h - 86.0).max(160.0);

            egui::Window::new("Visualizer Modules")
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .fixed_size([modal_w, modal_h])
                .frame(
                    egui::Frame::window(&ctx.global_style())
                        .fill(egui::Color32::from_black_alpha(230))
                        .corner_radius(12.0)
                        .inner_margin(margin_px),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("🎨 Visualization Modules")
                                .color(egui::Color32::WHITE)
                                .strong()
                                .size(if is_narrow_modal { 16.5 } else { 20.0 }),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("✕")
                                            .size(14.0)
                                            .color(egui::Color32::LIGHT_GRAY),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                engine_actions.push(EngineAction::ToggleVisPicker);
                            }
                            ui.add_space(4.0);
                            if ui.link("None").clicked() {
                                engine_actions.push(EngineAction::VisPickerEnableNone);
                            }
                            ui.label(egui::RichText::new("•").color(egui::Color32::from_gray(100)));
                            if ui.link("All").clicked() {
                                engine_actions.push(EngineAction::VisPickerEnableAll);
                            }
                            if !is_narrow_modal {
                                ui.label(
                                    egui::RichText::new("Rotation:")
                                        .color(egui::Color32::from_gray(150))
                                        .size(13.0),
                                );
                            }
                        });
                    });
                    ui.add_space(2.0);
                    let hint_str = if cfg!(target_os = "android") || is_narrow_modal {
                        "Tap row to select  •  Tap ✅ to toggle rotation  •  ✕ to close"
                    } else if state.has_gamepad {
                        "A / Enter to select  •  X / Space to toggle rotation  •  B / Esc to close"
                    } else {
                        "Enter to select  •  Space to toggle rotation  •  Esc to close"
                    };
                    ui.label(
                        egui::RichText::new(hint_str)
                            .color(egui::Color32::from_gray(130))
                            .size(11.0),
                    );
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(4.0);

                    egui::ScrollArea::vertical()
                        .max_height(scroll_max_h)
                        .show(ui, |ui| {
                            for (i, vis) in crate::state::VISUALIZERS.iter().enumerate() {
                                let is_cursor = i == state.vis_picker_cursor;
                                let is_active = i == state.current_visualizer_idx;
                                let is_enabled = state.vis_enabled.get(i).copied().unwrap_or(true);

                                let bg = if is_cursor {
                                    egui::Color32::from_rgba_unmultiplied(50, 110, 220, 100)
                                } else if is_active {
                                    egui::Color32::from_rgba_unmultiplied(30, 90, 30, 80)
                                } else {
                                    egui::Color32::TRANSPARENT
                                };

                                let row_frame = egui::Frame::NONE
                                    .fill(bg)
                                    .corner_radius(6.0)
                                    .inner_margin(egui::Margin::symmetric(10, 6));

                                let row_resp = row_frame.show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.set_min_width(ui.available_width());
                                        // Enable/disable toggle indicator
                                        let toggle_text = if is_enabled { "✅" } else { "⬜" };
                                        let toggle_color = if i == 0 {
                                            egui::Color32::from_gray(80) // Locked on
                                        } else if is_enabled {
                                            egui::Color32::WHITE
                                        } else {
                                            egui::Color32::from_gray(60)
                                        };
                                        let toggle_resp = ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(toggle_text)
                                                    .size(14.0)
                                                    .color(toggle_color),
                                            )
                                            .sense(egui::Sense::click()),
                                        );
                                        if toggle_resp.clicked() && i != 0 {
                                            engine_actions
                                                .push(EngineAction::VisPickerToggleEnabled(i));
                                        }

                                        ui.add_space(6.0);

                                        // Name
                                        let name_color = if !is_enabled {
                                            egui::Color32::from_gray(90)
                                        } else if is_active {
                                            egui::Color32::from_rgb(80, 255, 80)
                                        } else {
                                            egui::Color32::WHITE
                                        };
                                        ui.label(
                                            egui::RichText::new(vis.name)
                                                .color(name_color)
                                                .strong()
                                                .size(if is_narrow_modal { 14.0 } else { 15.0 }),
                                        );

                                        // Active indicator
                                        if is_active {
                                            ui.label(
                                                egui::RichText::new("▶")
                                                    .color(egui::Color32::from_rgb(80, 255, 80))
                                                    .size(13.0),
                                            );
                                        }

                                        // Description (right-aligned)
                                        if !is_narrow_modal {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let desc_color = if !is_enabled {
                                                        egui::Color32::from_gray(60)
                                                    } else {
                                                        egui::Color32::from_gray(120)
                                                    };
                                                    ui.label(
                                                        egui::RichText::new(vis.description)
                                                            .color(desc_color)
                                                            .size(11.5),
                                                    );
                                                },
                                            );
                                        }
                                    });
                                });

                                let rect = row_resp.response.rect;
                                let id = ui.scope_id().with(i);
                                let response = ui.interact(rect, id, egui::Sense::click());

                                // Hover moves cursor to this row
                                if response.hovered() && !is_cursor {
                                    engine_actions.push(EngineAction::VisPickerSetCursor(i));
                                }

                                // Click row to select visualizer
                                if response.clicked() {
                                    engine_actions.push(EngineAction::VisPickerSelect(i));
                                }

                                // Ensure cursor row is scrolled into view (for keyboard/gamepad navigation)
                                if is_cursor && state.vis_picker_scroll_to_cursor {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                            }
                        });
                });
        }

        let mut append = state.append_to_playlist;
        if matches!(file_dialog.state(), egui_file_dialog::DialogState::Open) {
            file_dialog.update_with_right_panel_ui(ctx, &mut |ui, _fd| {
                ui.add_space(10.0);
                ui.heading("Options");
                ui.separator();
                ui.checkbox(&mut append, "Add to Playlist instead of replacing");
            });

            if append != state.append_to_playlist {
                engine_actions.push(EngineAction::SetAppendToPlaylist(append));
            }
        }

        if let Some(paths) = file_dialog.take_picked_multiple() {
            let strings = paths.into_iter().map(|p| p.display().to_string()).collect();
            engine_actions.push(EngineAction::LoadFiles(strings, append));
        } else if let Some(path) = file_dialog.take_picked() {
            engine_actions.push(EngineAction::LoadFiles(
                vec![path.display().to_string()],
                append,
            ));
        }

        if state.file_transfer.is_some() {
            ctx.request_repaint();
        }

        let render_file_transfer_panel = |ui: &mut egui::Ui,
                                          transfer: &crate::state::FileTransferState,
                                          anim_time: f32,
                                          max_width: f32| {
            let panel_w = (max_width - 8.0).clamp(220.0, 560.0);
            egui::Frame::NONE
                .fill(egui::Color32::from_rgba_unmultiplied(12, 22, 42, 240))
                .stroke(egui::Stroke::new(
                    1.5_f32,
                    egui::Color32::from_rgb(0, 215, 255),
                ))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(panel_w);
                    ui.vertical_centered(|ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Spinner::new()
                                    .size(16.0)
                                    .color(egui::Color32::from_rgb(0, 225, 255)),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(&transfer.status_text)
                                    .size(14.5)
                                    .color(egui::Color32::from_rgb(0, 225, 255))
                                    .strong(),
                            );
                        });

                        ui.add_space(4.0);
                        let short_name = if transfer.file_name.chars().count() > 44 {
                            let prefix: String = transfer.file_name.chars().take(41).collect();
                            format!("{}…", prefix)
                        } else {
                            transfer.file_name.clone()
                        };
                        ui.label(
                            egui::RichText::new(short_name)
                                .size(13.5)
                                .color(egui::Color32::WHITE)
                                .strong(),
                        );

                        ui.add_space(8.0);
                        let bar_w = (panel_w - 16.0).max(160.0);
                        let bar_h = 16.0;
                        let (bar_rect, _) =
                            ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
                        let painter = ui.painter();
                        painter.rect(
                            bar_rect,
                            8.0,
                            egui::Color32::from_rgba_unmultiplied(6, 10, 22, 235),
                            egui::Stroke::new(
                                1.0_f32,
                                egui::Color32::from_rgba_unmultiplied(0, 185, 255, 110),
                            ),
                            egui::StrokeKind::Inside,
                        );

                        if let Some(pct) = transfer.progress_fraction() {
                            let fill_w = (bar_rect.width() * pct.clamp(0.03, 1.0))
                                .clamp(8.0, bar_rect.width());
                            let fill_rect = egui::Rect::from_min_size(
                                bar_rect.min,
                                egui::vec2(fill_w, bar_rect.height()),
                            );
                            painter.rect_filled(
                                fill_rect,
                                8.0,
                                egui::Color32::from_rgb(0, 195, 255),
                            );
                            // Moving specular shimmer across determinate fill bar
                            let shimmer_phase = (anim_time * 1.4).rem_euclid(1.0);
                            let shimmer_w = (fill_w * 0.28).clamp(12.0, 64.0);
                            let shimmer_x =
                                fill_rect.left() + (fill_w - shimmer_w).max(0.0) * shimmer_phase;
                            let shimmer_rect = egui::Rect::from_min_size(
                                egui::pos2(shimmer_x, fill_rect.top()),
                                egui::vec2(
                                    shimmer_w.min(fill_rect.right() - shimmer_x),
                                    fill_rect.height(),
                                ),
                            );
                            painter.rect_filled(
                                shimmer_rect,
                                8.0,
                                egui::Color32::from_rgba_unmultiplied(180, 245, 255, 110),
                            );
                            painter.text(
                                bar_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("{:.0}%", pct * 100.0),
                                egui::FontId::proportional(11.5),
                                egui::Color32::WHITE,
                            );
                        } else {
                            // Smooth bidirectional scanning neon bar during cloud download / openInputStream
                            let pulse_w = (bar_rect.width() * 0.34).clamp(40.0, 160.0);
                            let travel = (bar_rect.width() - pulse_w).max(0.0);
                            let t_osc = 0.5 - 0.5 * (anim_time * 2.8).cos();
                            let x0 = bar_rect.left() + travel * t_osc;
                            let scan_rect = egui::Rect::from_min_size(
                                egui::pos2(x0, bar_rect.top() + 1.0),
                                egui::vec2(pulse_w, bar_rect.height() - 2.0),
                            );
                            painter.rect_filled(
                                scan_rect,
                                7.0,
                                egui::Color32::from_rgb(0, 205, 255),
                            );
                            let core_rect = scan_rect.shrink2(egui::vec2(pulse_w * 0.22, 1.5));
                            painter.rect_filled(
                                core_rect,
                                6.0,
                                egui::Color32::from_rgb(165, 245, 255),
                            );
                        }

                        ui.add_space(6.0);
                        let detail_line = if transfer.bytes_copied > 0 {
                            let copied_mb = transfer.bytes_copied as f64 / (1024.0 * 1024.0);
                            let rate_str = if transfer.elapsed_secs > 0.15 {
                                let mb_per_s = copied_mb / (transfer.elapsed_secs as f64);
                                format!("  •  {:.1} MB/s", mb_per_s)
                            } else {
                                String::new()
                            };
                            if let Some(tot) = transfer.total_bytes
                                && tot > 0
                            {
                                let total_mb = tot as f64 / (1024.0 * 1024.0);
                                format!(
                                    "{:.1} / {:.1} MB{}  •  {:.1}s",
                                    copied_mb, total_mb, rate_str, transfer.elapsed_secs
                                )
                            } else {
                                format!(
                                    "{:.1} MB cached{}  •  {:.1}s",
                                    copied_mb, rate_str, transfer.elapsed_secs
                                )
                            }
                        } else if let Some(tot) = transfer.total_bytes
                            && tot > 0
                        {
                            let total_mb = tot as f64 / (1024.0 * 1024.0);
                            format!(
                                "Size: {:.1} MB  •  Elapsed: {:.1}s",
                                total_mb, transfer.elapsed_secs
                            )
                        } else if transfer.elapsed_secs >= 0.1 {
                            format!(
                                "Elapsed: {:.1}s  •  Waiting for cloud stream…",
                                transfer.elapsed_secs
                            )
                        } else {
                            "Requesting file stream…".to_string()
                        };
                        ui.label(
                            egui::RichText::new(detail_line)
                                .size(12.0)
                                .color(egui::Color32::from_rgb(195, 215, 235)),
                        );
                    });
                });
        };

        if !state.file_loaded {
            *central_rect = ctx.content_rect();
            let time = self.smooth_time as f32;
            let rect = ctx.content_rect();

            // Ensure cached synthwave background geometry & horizon_y are up to date;
            // the combined static+dynamic background primitive is prepended directly
            // to `clipped_primitives` after `egui_ctx.tessellate` to avoid redundant
            // per-frame `calc_bounds()` and `append_ref()` copies inside `Tessellator`.
            self.splash_cache.ensure_background(ctx, rect);

            let is_mobile = cfg!(target_os = "android");
            let is_game_mode = self.is_game_mode;

            let show_kb = !is_game_mode && !is_mobile;
            let show_gp = state.has_gamepad && !is_mobile;
            let show_touch = is_mobile;
            let is_wide_viewport = rect.width() >= 680.0;

            let bottom_reserved_h = if show_touch {
                if is_wide_viewport { 96.0 } else { 132.0 }
            } else if show_kb && show_gp {
                168.0
            } else if show_kb || show_gp {
                144.0
            } else {
                0.0
            };

            if show_kb || show_gp || show_touch {
                egui::Area::new(egui::Id::unique("splash_shortcuts_area"))
                        .order(egui::Order::Foreground)
                        .anchor(
                            egui::Align2::CENTER_BOTTOM,
                            egui::vec2(0.0, if is_mobile { -10.0 } else { -24.0 }),
                        )
                        .show(ctx, |ui| {
                            egui::Frame::NONE
                                .fill(egui::Color32::from_rgba_unmultiplied(10, 10, 20, 220))
                                .stroke(egui::Stroke::new(
                                    1.0_f32,
                                    egui::Color32::from_rgba_unmultiplied(91, 50, 212, 140),
                                ))
                                .corner_radius(12.0)
                                .inner_margin(egui::Margin::symmetric(
                                    if is_mobile { 16 } else { 22 },
                                    if is_mobile { 10 } else { 14 },
                                ))
                                .show(ui, |ui| {
                                    if show_touch {
                                        let gestures: [(&str, &str); 6] = [
                                            ("Swipe L / R", "Switch Visualizers"),
                                            ("Swipe U / D", "Tabs / Fullscreen Video"),
                                            ("Single Tap", "Play / Pause"),
                                            ("Double Tap", "Toggle HUD Visibility"),
                                            ("2-Finger Tap", "Cycle Audio Track"),
                                            ("3-Finger Tap", "Toggle Engine Stats"),
                                        ];
                                        let pairs_per_row = if is_wide_viewport { 3 } else { 2 };
                                        ui.vertical_centered(|ui| {
                                            ui.label(
                                                egui::RichText::new("Touch Gestures Guide")
                                                    .color(egui::Color32::from_rgb(210, 220, 240))
                                                    .strong()
                                                    .size(13.0),
                                            );
                                            ui.add_space(6.0);
                                            egui::Grid::new("touch_shortcuts")
                                                .num_columns(pairs_per_row * 2)
                                                .spacing([if is_wide_viewport { 16.0 } else { 12.0 }, 5.0])
                                                .show(ui, |ui| {
                                                    for (idx, (gesture, desc)) in
                                                        gestures.iter().enumerate()
                                                    {
                                                        ui.label(
                                                            egui::RichText::new(*gesture)
                                                                .color(egui::Color32::from_rgb(
                                                                    0, 220, 255,
                                                                ))
                                                                .strong()
                                                                .size(12.0),
                                                        );
                                                        ui.label(
                                                            egui::RichText::new(*desc)
                                                                .color(egui::Color32::LIGHT_GRAY)
                                                                .size(12.0),
                                                        );
                                                        if (idx + 1) % pairs_per_row == 0 {
                                                            ui.end_row();
                                                        }
                                                    }
                                                });
                                        });
                                    } else {
                                        ui.horizontal_centered(|ui| {
                                            let pairs_per_row = if show_kb && show_gp {
                                                if rect.width() >= 1100.0 { 3 } else { 2 }
                                            } else {
                                                4
                                            };

                                            if show_kb {
                                                ui.vertical(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(
                                                            "🖮 Keyboard Shortcuts",
                                                        )
                                                        .color(egui::Color32::from_rgb(
                                                            210, 220, 240,
                                                        ))
                                                        .strong()
                                                        .size(15.0),
                                                    );
                                                    ui.add_space(6.0);

                                                    egui::Grid::new("kb_shortcuts")
                                                        .num_columns(pairs_per_row)
                                                        .spacing([28.0, 6.0])
                                                        .show(ui, |ui| {
                                                            let mut col = 0;
                                                            let mut kb_shortcut =
                                                                |key: &str, desc: &str| {
                                                                    ui.horizontal(|ui| {
                                                                        ui.spacing_mut().item_spacing.x = 6.0;
                                                                        ui.label(
                                                                            egui::RichText::new(key)
                                                                                .color(egui::Color32::from_rgb(0, 220, 255))
                                                                                .strong()
                                                                                .size(12.5),
                                                                        );
                                                                        ui.label(
                                                                            egui::RichText::new(desc)
                                                                                .color(egui::Color32::LIGHT_GRAY)
                                                                                .size(12.5),
                                                                        );
                                                                    });
                                                                    col += 1;
                                                                    if col == pairs_per_row {
                                                                        ui.end_row();
                                                                        col = 0;
                                                                    }
                                                                };
                                                            let ctrl_mod =
                                                                if std::env::consts::OS == "macos" {
                                                                    "⌘"
                                                                } else {
                                                                    "ctrl+"
                                                                };
                                                            kb_shortcut("o", "Open File");
                                                            kb_shortcut("space", "Play / Pause");
                                                            kb_shortcut("v", "Toggle Video");
                                                            kb_shortcut("m", "Visualizer Modules");
                                                            kb_shortcut(
                                                                "left/right",
                                                                "Seek Timeline",
                                                            );
                                                            kb_shortcut("tab", "Toggle HUD");
                                                            kb_shortcut("up/down", "Cycle Vis");
                                                            kb_shortcut("s", "Toggle Stats");
                                                            kb_shortcut(
                                                                &format!("{}q / esc", ctrl_mod),
                                                                "Quit",
                                                            );
                                                            kb_shortcut("f", "Fullscreen");
                                                            kb_shortcut("u / e", "Surround Upmix");
                                                            kb_shortcut("1-9", "Audio Track");
                                                            kb_shortcut("p / n", "Prev/Next");
                                                            kb_shortcut(
                                                                &format!("{}L/R", ctrl_mod),
                                                                "Seek ±60s",
                                                            );
                                                            kb_shortcut("[ / ]", "Scale Panels");
                                                            kb_shortcut("h", "Toggle Help");
                                                        });
                                                });
                                            }

                                            if show_kb && show_gp {
                                                ui.add_space(16.0);
                                                let (sep_rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(1.0, 130.0),
                                                    egui::Sense::hover(),
                                                );
                                                ui.painter().line_segment(
                                                    [
                                                        sep_rect.center_top(),
                                                        sep_rect.center_bottom(),
                                                    ],
                                                    (1.0, egui::Color32::from_gray(65)),
                                                );
                                                ui.add_space(16.0);
                                            }

                                            if show_gp {
                                                ui.vertical(|ui| {
                                                    ui.label(
                                                        egui::RichText::new("🎮 Gamepad Controls")
                                                            .color(egui::Color32::from_rgb(
                                                                210, 220, 240,
                                                            ))
                                                            .strong()
                                                            .size(15.0),
                                                    );
                                                    ui.add_space(6.0);
                                                    egui::Grid::new("gp_shortcuts_wide")
                                                        .num_columns(pairs_per_row)
                                                        .spacing([28.0, 6.0])
                                                        .show(ui, |ui| {
                                                            let mut col = 0;
                                                            let mut gp_shortcut =
                                                                |gp: &str, desc: &str| {
                                                                    ui.horizontal(|ui| {
                                                                        ui.spacing_mut().item_spacing.x = 6.0;
                                                                        ui.label(
                                                                            egui::RichText::new(
                                                                                gamepad_icon(
                                                                                    state.gamepad_type,
                                                                                    gp,
                                                                                ),
                                                                            )
                                                                            .color(
                                                                                egui::Color32::LIGHT_BLUE,
                                                                            )
                                                                            .strong()
                                                                            .size(13.0),
                                                                        );
                                                                        ui.label(
                                                                            egui::RichText::new(desc)
                                                                                .color(
                                                                                    egui::Color32::LIGHT_GRAY,
                                                                                )
                                                                                .size(12.5),
                                                                        );
                                                                    });
                                                                    col += 1;
                                                                    if col == pairs_per_row {
                                                                        ui.end_row();
                                                                        col = 0;
                                                                    }
                                                                };
                                                            gp_shortcut("Y", "Open File");
                                                            gp_shortcut("A", "Play / Pause");
                                                            gp_shortcut("X", "Toggle Video");
                                                            gp_shortcut(
                                                                "D-Pad L/R",
                                                                "Seek Timeline",
                                                            );
                                                            gp_shortcut("L2 / R2", "Analog Scrub");
                                                            gp_shortcut("L3", "Toggle HUD");
                                                            gp_shortcut("D-Pad U/D", "Cycle Vis");
                                                            gp_shortcut("B", "Toggle Stats");
                                                            gp_shortcut("Select", "Quit");
                                                            gp_shortcut("Start", "Fullscreen");
                                                            gp_shortcut("R3", "Surround Upmix");
                                                            gp_shortcut("L1 / R1", "Prev/Next");
                                                            gp_shortcut("L/R Stick", "Scale Panels");
                                                        });
                                                });
                                            }
                                        });
                                    }
                                });
                        });
            }

            let frame = egui::Frame::NONE
                .fill(egui::Color32::TRANSPARENT)
                .inner_margin(if is_mobile { 12.0 } else { 28.0 });

            egui::CentralPanel::default()
                    .frame(frame)
                    .show(ctx, |ui| {
                        let real_avail_height = ui.available_height();
                        let real_avail_width = ui.available_width();

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                let is_portrait_splash = rect.width() < rect.height();
                                let top_space = if is_portrait_splash {
                                    40.0
                                } else if is_mobile {
                                    30.0
                                } else {
                                    (real_avail_height * 0.03).clamp(8.0, 28.0)
                                };
                                ui.add_space(top_space);

                                // Scale title dynamically so it sits cleanly in the upper sky without colliding with the top-right Stats panel
                                let target_w = (real_avail_width - 24.0).max(120.0);
                                let width_scale = (target_w / 950.0).clamp(0.25, 0.58);
                                let height_scale = (real_avail_height / 500.0).clamp(0.25, 0.58);
                                let scale_factor = width_scale.min(height_scale);
                                let title_width = 950.0 * scale_factor;
                                let title_height = 122.0 * scale_factor;
                                let font_size = 114.0 * scale_factor;
                                let gradient_extent = 50.0 * scale_factor;

                                // --- Glowing Animated Chrome Title (Cached glyph quads, merged into background primitive) ---
                                let (title_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(title_width, title_height),
                                    egui::Sense::hover(),
                                );
                                self.splash_cache.update_title_mesh(
                                    ui,
                                    title_rect,
                                    gradient_extent,
                                    font_size,
                                    time,
                                );

                                // Format & Engine Subtitle Pill
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new(
                                        "MOD • XM • S3M • IT • FLAC • WAV • MP3 • OPUS • MKV/MP4 • LIVE STREAM",
                                    )
                                    .size((12.0 * scale_factor.max(0.75)).clamp(9.5, 12.5))
                                    .color(egui::Color32::from_rgb(235, 195, 250))
                                    .strong(),
                                );

                                // Position the Hero Action Card over the perspective neon grid just below the sunset horizon,
                                // leaving the sliced synthwave sun and mountain peaks unobstructed in the sky above.
                                let is_narrow = real_avail_width < 600.0;
                                let after_subtitle_y = ui.cursor().min.y;
                                let transfer_extra_h =
                                    if state.file_transfer.is_some() { 112.0 } else { 0.0 };
                                let est_card_h = (if is_narrow { 255.0 } else { 222.0 }) + transfer_extra_h;
                                let max_card_top_y = (rect.bottom() - bottom_reserved_h - est_card_h - 18.0)
                                    .max(after_subtitle_y + 10.0);
                                let target_card_top_y =
                                    (self.splash_cache.horizon_y + 8.0).min(max_card_top_y);
                                let scenic_gap =
                                    (target_card_top_y - after_subtitle_y).max(10.0);
                                ui.add_space(scenic_gap);

                                // --- Unified Translucent Hero Action Card ---
                                let card_max_w = if is_narrow {
                                    (real_avail_width - 16.0).clamp(260.0, 420.0)
                                } else {
                                    620.0
                                };

                                egui::Frame::NONE
                                    .fill(egui::Color32::from_rgba_unmultiplied(14, 12, 28, 215))
                                    .stroke(egui::Stroke::new(
                                        1.0_f32,
                                        egui::Color32::from_rgba_unmultiplied(212, 34, 161, 135),
                                    ))
                                    .corner_radius(14.0)
                                    .inner_margin(egui::Margin::symmetric(
                                        if is_narrow { 16 } else { 24 },
                                        if is_narrow { 14 } else { 18 },
                                    ))
                                    .show(ui, |ui| {
                                        ui.set_max_width(card_max_w);
                                        ui.vertical_centered(|ui| {
                                            if let Some(ref transfer) = state.file_transfer {
                                                render_file_transfer_panel(
                                                    ui,
                                                    transfer,
                                                    time,
                                                    card_max_w - 24.0,
                                                );
                                                ui.add_space(12.0);
                                            }
                                            let is_file_hovered =
                                                !ui.input(|i| i.raw.hovered_files.is_empty())
                                                    || state.hovered_file.is_some();
                                            if is_file_hovered {
                                                let badge_text =
                                                    if let Some(hf) = &state.hovered_file {
                                                        let fn_only = std::path::Path::new(hf)
                                                            .file_name()
                                                            .unwrap_or_default()
                                                            .to_string_lossy();
                                                        format!(
                                                            "📥 Drop '{}' to start playing",
                                                            fn_only
                                                        )
                                                    } else {
                                                        "📥 Drop media file here to start playing"
                                                            .to_string()
                                                    };
                                                egui::Frame::NONE
                                                    .fill(egui::Color32::from_rgba_unmultiplied(
                                                        215, 105, 0, 230,
                                                    ))
                                                    .stroke(egui::Stroke::new(
                                                        1.5_f32,
                                                        egui::Color32::from_rgb(255, 175, 40),
                                                    ))
                                                    .corner_radius(8.0)
                                                    .inner_margin(egui::Margin::symmetric(14, 7))
                                                    .show(ui, |ui| {
                                                        ui.label(
                                                            egui::RichText::new(badge_text)
                                                                .size(15.0)
                                                                .color(egui::Color32::WHITE)
                                                                .strong(),
                                                        );
                                                    });
                                                ui.add_space(10.0);
                                            }

                                            let btn_text = if is_file_hovered {
                                                "📥 DROP TO PLAY"
                                            } else {
                                                "OPEN FILE"
                                            };
                                            let btn_fill = if is_file_hovered {
                                                egui::Color32::from_rgb(240, 125, 10)
                                            } else {
                                                egui::Color32::from_rgb(205, 95, 0)
                                            };
                                            let btn_stroke = if is_file_hovered {
                                                egui::Stroke::new(
                                                    2.0_f32,
                                                    egui::Color32::from_rgb(255, 195, 70),
                                                )
                                            } else {
                                                egui::Stroke::new(
                                                    1.0_f32,
                                                    egui::Color32::from_rgb(255, 145, 30),
                                                )
                                            };

                                            let btn_font_size = if is_narrow { 17.0 } else { 20.0 };
                                            let btn_h = if is_narrow { 48.0 } else { 54.0 };
                                            let btn_w = if is_narrow {
                                                (card_max_w - 32.0).clamp(200.0, 340.0)
                                            } else {
                                                265.0
                                            };

                                            let btn = egui::Button::new(
                                                egui::RichText::new(btn_text)
                                                    .size(btn_font_size)
                                                    .color(egui::Color32::WHITE)
                                                    .strong(),
                                            )
                                            .fill(btn_fill)
                                            .stroke(btn_stroke)
                                            .corner_radius(8.0);

                                            let url_btn = egui::Button::new(
                                                egui::RichText::new("OPEN STREAM/DEVICE")
                                                    .size(btn_font_size)
                                                    .color(egui::Color32::WHITE)
                                                    .strong(),
                                            )
                                            .fill(egui::Color32::from_rgb(100, 50, 150))
                                            .stroke(egui::Stroke::new(
                                                1.0_f32,
                                                egui::Color32::from_rgb(180, 80, 255),
                                            ))
                                            .corner_radius(8.0);

                                            let no_modal_open = !state.is_file_picker_open
                                                && !state.is_url_dialog_open
                                                && !state.show_vis_picker;

                                            if is_narrow {
                                                let resp1 = ui.add_sized([btn_w, btn_h], btn);
                                                if state.has_gamepad
                                                    && no_modal_open
                                                    && ui.ctx().memory(|m| m.focused().is_none())
                                                {
                                                    resp1.request_focus();
                                                }
                                                if resp1.clicked() {
                                                    engine_actions.push(EngineAction::OpenFile);
                                                }
                                                ui.add_space(10.0);
                                                let resp2 = ui.add_sized([btn_w, btn_h], url_btn);
                                                if resp2.clicked() {
                                                    engine_actions.push(EngineAction::OpenUrlDialog);
                                                }
                                            } else {
                                                ui.horizontal(|ui| {
                                                    let total_width = btn_w * 2.0 + 16.0;
                                                    let pad = ((ui.available_width() - total_width)
                                                        * 0.5)
                                                        .max(0.0);
                                                    ui.add_space(pad);
                                                    let resp1 = ui.add_sized([btn_w, btn_h], btn);
                                                    if state.has_gamepad
                                                        && no_modal_open
                                                        && ui.ctx().memory(|m| m.focused().is_none())
                                                    {
                                                        resp1.request_focus();
                                                    }
                                                    if resp1.clicked() {
                                                        engine_actions.push(EngineAction::OpenFile);
                                                    }
                                                    ui.add_space(16.0);
                                                    let resp2 =
                                                        ui.add_sized([btn_w, btn_h], url_btn);
                                                    if resp2.clicked() {
                                                        engine_actions.push(
                                                            EngineAction::OpenUrlDialog,
                                                        );
                                                    }
                                                });
                                            }

                                            ui.add_space(10.0);

                                            // Quick-Access Utility Row: Visualizer Module Picker & Recent Stream
                                            let vis_btn_label =
                                                format!("🎨 Visualizer: {}", vis_name);
                                            let vis_btn = egui::Button::new(
                                                egui::RichText::new(vis_btn_label)
                                                    .size(12.5)
                                                    .color(egui::Color32::from_rgb(190, 225, 255)),
                                            )
                                            .fill(egui::Color32::from_rgba_unmultiplied(
                                                30, 35, 65, 210,
                                            ))
                                            .stroke(egui::Stroke::new(
                                                1.0_f32,
                                                egui::Color32::from_rgba_unmultiplied(
                                                    0, 190, 255, 130,
                                                ),
                                            ))
                                            .corner_radius(6.0);

                                            if let Some((recent_url, recent_title)) =
                                                state.url_history.last()
                                            {
                                                let display_ref = if !recent_title.is_empty() {
                                                    recent_title.as_str()
                                                } else {
                                                    recent_url.as_str()
                                                };
                                                let short_url = if display_ref.chars().count() > 28
                                                {
                                                    let truncated: String =
                                                        display_ref.chars().take(26).collect();
                                                    format!("📡 Recent: {}…", truncated)
                                                } else {
                                                    format!("📡 Recent: {}", display_ref)
                                                };
                                                let recent_btn = egui::Button::new(
                                                    egui::RichText::new(short_url)
                                                        .size(12.5)
                                                        .color(egui::Color32::from_rgb(
                                                            225, 195, 255,
                                                        )),
                                                )
                                                .fill(egui::Color32::from_rgba_unmultiplied(
                                                    50, 28, 78, 210,
                                                ))
                                                .stroke(egui::Stroke::new(
                                                    1.0_f32,
                                                    egui::Color32::from_rgba_unmultiplied(
                                                        180, 90, 255, 130,
                                                    ),
                                                ))
                                                .corner_radius(6.0);

                                                if is_narrow {
                                                    if ui
                                                        .add_sized([btn_w, 30.0], vis_btn)
                                                        .clicked()
                                                    {
                                                        engine_actions.push(
                                                            EngineAction::ToggleVisPicker,
                                                        );
                                                    }
                                                    ui.add_space(6.0);
                                                    if ui
                                                        .add_sized([btn_w, 30.0], recent_btn)
                                                        .on_hover_text(recent_url.as_str())
                                                        .clicked()
                                                    {
                                                        engine_actions.push(EngineAction::LoadUrl(
                                                            recent_url.clone(),
                                                        ));
                                                    }
                                                } else {
                                                    ui.horizontal(|ui| {
                                                        let sub_w =
                                                            (btn_w * 2.0 + 16.0 - 12.0) * 0.5;
                                                        let total_w = sub_w * 2.0 + 12.0;
                                                        let pad = ((ui.available_width() - total_w)
                                                            * 0.5)
                                                            .max(0.0);
                                                        ui.add_space(pad);
                                                        if ui
                                                            .add_sized([sub_w, 30.0], vis_btn)
                                                            .clicked()
                                                        {
                                                            engine_actions.push(
                                                                EngineAction::ToggleVisPicker,
                                                            );
                                                        }
                                                        ui.add_space(12.0);
                                                        if ui
                                                            .add_sized([sub_w, 30.0], recent_btn)
                                                            .on_hover_text(recent_url.as_str())
                                                            .clicked()
                                                        {
                                                            engine_actions.push(EngineAction::LoadUrl(
                                                                recent_url.clone(),
                                                            ));
                                                        }
                                                    });
                                                }
                                            } else {
                                                let pill_w = if is_narrow {
                                                    btn_w
                                                } else {
                                                    btn_w * 2.0 + 16.0
                                                };
                                                if ui.add_sized([pill_w, 30.0], vis_btn).clicked()
                                                {
                                                    engine_actions.push(EngineAction::ToggleVisPicker);
                                                }
                                            }

                                            ui.add_space(8.0);
                                            ui.separator();
                                            ui.add_space(4.0);

                                            // Audio output toggles inside the hero card for high legibility
                                            let mut force_stereo = state.force_stereo_downmix;
                                            if ui
                                                .checkbox(
                                                    &mut force_stereo,
                                                    egui::RichText::new(
                                                        "Force Stereo Downmix (Fixes crackling on some devices)",
                                                    )
                                                    .size(12.5)
                                                    .color(egui::Color32::from_rgb(205, 210, 225)),
                                                )
                                                .changed()
                                            {
                                                engine_actions.push(
                                                    EngineAction::SetForceStereo(force_stereo),
                                                );
                                            }

                                            let mut surround_upmix = state.surround_upmix.enabled;
                                            if ui
                                                .checkbox(
                                                    &mut surround_upmix,
                                                    egui::RichText::new(
                                                        "Auto Surround Upmix (5.1/7.1 Bed or 7.1.4 Bed + Height)",
                                                    )
                                                    .size(12.5)
                                                    .color(egui::Color32::from_rgb(255, 180, 85)),
                                                )
                                                .on_hover_text("When connected to a multi-channel surround output (>2ch), upmixes stereo sources across the surround bed (5.1/7.1) or bed + overhead height channels (7.1.4). Stereo/mono outputs (<=2ch) remain completely unchanged.")
                                                .changed()
                                            {
                                                engine_actions.push(
                                                    EngineAction::ToggleSurroundUpmix(surround_upmix),
                                                );
                                            }

                                            #[cfg(any(
                                                target_os = "windows",
                                                target_os = "linux"
                                            ))]
                                            {
                                                let mut passthrough = state.passthrough_enabled;

                                                #[cfg(target_os = "windows")]
                                                let label = "Enable Bitstream Passthrough (WASAPI Exclusive)";
                                                #[cfg(target_os = "linux")]
                                                let label =
                                                    "Enable Bitstream Passthrough (PipeWire)";

                                                if ui
                                                    .checkbox(
                                                        &mut passthrough,
                                                        egui::RichText::new(label)
                                                            .size(12.5)
                                                            .color(egui::Color32::from_rgb(
                                                                205, 210, 225,
                                                            )),
                                                    )
                                                    .changed()
                                                {
                                                    engine_actions.push(
                                                        EngineAction::SetPassthrough(passthrough),
                                                    );
                                                }
                                            }
                                        });
                                    });
                            });
                        });
                    });

            // Render OSD notifications on the splash screen when not superseded by the transfer panel
            if state.file_transfer.is_none()
                && let Some(osd) = &state.osd_text
                && state.osd_timer > 0.0
            {
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::unique("splash_osd_notification"),
                ));
                let alpha = (state.osd_timer.min(0.5) * 2.0 * 255.0) as u8;
                let screen = ctx.viewport_rect();
                let is_portrait = screen.width() < screen.height();
                let max_width = if is_portrait {
                    (screen.width() - 40.0).max(100.0)
                } else {
                    (screen.width() * 0.85).max(200.0)
                };
                let font_size = if is_portrait { 18.0 } else { 22.0 };
                let text_color = egui::Color32::from_rgba_premultiplied(255, 255, 255, alpha);
                let galley = painter.layout(
                    osd.clone(),
                    egui::FontId::proportional(font_size),
                    text_color,
                    max_width,
                );
                let top_y = if is_portrait {
                    screen.top() + 52.0
                } else {
                    screen.top() + 32.0
                };
                let galley_pos = egui::pos2(screen.center().x - galley.rect.width() * 0.5, top_y);
                let bg_rect = galley
                    .rect
                    .translate(galley_pos.to_vec2())
                    .expand2(egui::vec2(14.0, 8.0));
                painter.rect_filled(
                    bg_rect,
                    8.0,
                    egui::Color32::from_rgba_unmultiplied(10, 10, 20, (alpha as f32 * 0.8) as u8),
                );
                painter.galley(galley_pos, galley, text_color);
            }

            return;
        }

        if let Some(ref transfer) = state.file_transfer {
            let anim_time = self.smooth_time as f32;
            let viewport_w = ctx.viewport_rect().width();
            egui::Area::new(egui::Id::unique("active_file_transfer_overlay"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    render_file_transfer_panel(
                        ui,
                        transfer,
                        anim_time,
                        (viewport_w - 32.0).clamp(240.0, 480.0),
                    );
                });
        }

        let is_portrait = ctx.viewport_rect().width() < ctx.viewport_rect().height();
        if state.show_hud && state.video_mode != 3 {
            let total_h = ctx.viewport_rect().height();
            let min_h = 120.0f32;
            let top_h =
                (total_h * state.panel_split_ratio).clamp(min_h, (total_h - min_h).max(min_h));
            let top_margin = if is_portrait {
                54
            } else if cfg!(target_os = "android") {
                42
            } else {
                8
            };
            let bottom_margin = if is_portrait { 14 } else { 8 };
            let display_secs = state.scrub_target_seconds.unwrap_or(state.current_seconds);
            let force_hours = state.duration_seconds >= 3600.0 || display_secs >= 3600.0;
            let is_live = state.duration_seconds <= 0.0;
            let tl_key = (
                display_secs.max(0.0) as u64,
                state.duration_seconds.max(0.0) as u64,
                is_live,
                force_hours,
            );
            if self.stats_cache.timeline_key != Some(tl_key) {
                self.stats_cache.timeline_text.clear();
                self.stats_cache
                    .timeline_text
                    .push_str(&crate::state::format_timeline_time(
                        display_secs,
                        force_hours,
                    ));
                if is_live {
                    self.stats_cache.timeline_text.push_str(" / LIVE");
                } else {
                    self.stats_cache.timeline_text.push_str(" / ");
                    self.stats_cache
                        .timeline_text
                        .push_str(&crate::state::format_timeline_time(
                            state.duration_seconds,
                            force_hours,
                        ));
                }
                self.stats_cache.timeline_key = Some(tl_key);
            }
            let cached_timeline_text = self.stats_cache.timeline_text.as_str();
            let panel_resp = egui::Panel::top("top_panel")
                    .resizable(false)
                    .frame(
                        egui::Frame::NONE
                            .fill(egui::Color32::TRANSPARENT)
                            .inner_margin(egui::Margin {
                                left: 16,
                                right: 16,
                                top: top_margin,
                                bottom: bottom_margin,
                            })
                    )
                    .exact_size(top_h)
                    .show(ctx, |ui| {
                        let render_progress_bar = |col: &mut egui::Ui, out_fire_rect: &mut Option<egui::Rect>, engine_action: &mut Vec<EngineAction>| {
                            // Custom Fire/Charred Progress Bar
                            let (rect, response) = col.allocate_exact_size(
                                egui::vec2(col.available_width(), 16.0),
                                egui::Sense::click_and_drag(),
                            );
                            *out_fire_rect = Some(rect);

                            if response.hovered() || response.dragged() {
                                col.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }

                            let pointer_pos = response
                                .interact_pointer_pos()
                                .or_else(|| col.ctx().pointer_latest_pos());
                            let pct_from_pos = pointer_pos.map(|mouse_pos| {
                                let width = rect.width().max(1.0);
                                let rel_x = (mouse_pos.x - rect.left()).clamp(0.0, width);
                                (rel_x / width).clamp(0.0, 1.0)
                            });

                            if response.drag_stopped() || response.clicked() {
                                if let Some(pct) = pct_from_pos {
                                    engine_action.push(EngineAction::Seek(pct));
                                } else if let Some(target_secs) = state.scrub_target_seconds
                                    && state.duration_seconds > 0.0
                                {
                                    let pct = (target_secs / state.duration_seconds).clamp(0.0, 1.0) as f32;
                                    engine_action.push(EngineAction::Seek(pct));
                                }
                            } else if response.dragged() || response.is_pointer_button_down_on() {
                                if let Some(pct) = pct_from_pos {
                                    engine_action.push(EngineAction::ScrubPreview(pct, 0.0));
                                }
                            } else if state.scrub_target_seconds.is_some() && !state.is_touch_scrubbing {
                                engine_action.push(EngineAction::ScrubEnd);
                            }

                            let painter = col.painter();
                            let time_text = cached_timeline_text;

                            let font_id = egui::FontId::proportional(11.0);
                            let shadow_color = egui::Color32::from_black_alpha(210);
                            for offset in [
                                egui::vec2(-1.0, 0.0),
                                egui::vec2(1.0, 0.0),
                                egui::vec2(0.0, -1.0),
                                egui::vec2(0.0, 1.0),
                            ] {
                                painter.text(
                                    rect.center() + offset,
                                    egui::Align2::CENTER_CENTER,
                                    time_text,
                                    font_id.clone(),
                                    shadow_color,
                                );
                            }
                            painter.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                time_text,
                                font_id,
                                egui::Color32::WHITE,
                            );
                        };

                        if state.video_mode == 2 && (self.video_state.is_some() || state.has_video_stream) {
                            // Video occupies the top panel with a full-width timeline bar at the bottom
                            let video_h = (ui.available_height() - 22.0).max(40.0);
                            let (v_rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), video_h),
                                egui::Sense::hover(),
                            );
                            *out_video_rect = Some(v_rect);
                            ui.add_space(4.0);
                            render_progress_bar(ui, out_fire_rect, engine_actions);
                        } else {
                            let render_col_channels = |col: &mut egui::Ui, out_meters_rect: &mut Option<egui::Rect>, out_fire_rect: &mut Option<egui::Rect>, engine_action: &mut Vec<EngineAction>| {
                                if !is_portrait {
                                    col.heading("Channels");
                                    col.separator();
                                }
                                let meters_height = if is_portrait {
                                    col.available_height()
                                } else {
                                    (col.available_height() - 30.0).max(20.0)
                                };
                                let (channel_rect, _) = col.allocate_exact_size(
                                    egui::vec2(col.available_width(), meters_height),
                                    egui::Sense::hover()
                                );
                                *out_meters_rect = Some(channel_rect);

                                let painter = col.painter();
                                let num_channels = state.channel_vus.len();
                                if num_channels > 0 {
                                    let w = channel_rect.width() / num_channels as f32;
                                    for i in 0..num_channels {
                                        let x = channel_rect.left() + i as f32 * w + w * 0.2;
                                        let bw = w * 0.6;
                                        let y_bottom = channel_rect.bottom() - 15.0;

                                        // Label
                                        if num_channels <= 16 {
                                            let label = if state.tracker_channels.is_some()
                                                && !state
                                                    .surround_upmix
                                                    .is_active_for(state.source_channels, state.hardware_channels)
                                            {
                                                if i == 0 {
                                                    "L".to_string()
                                                } else if i == num_channels - 1 {
                                                    "R".to_string()
                                                } else {
                                                    format!("{}", i)
                                                }
                                            } else {
                                                match num_channels {
                                                    2 => ["L", "R"].get(i).unwrap_or(&"?").to_string(),
                                                    3 => ["L", "C", "R"].get(i).unwrap_or(&"?").to_string(),
                                                    4 => ["Ls", "L", "R", "Rs"].get(i).unwrap_or(&"?").to_string(),
                                                    6 => ["Ls", "L", "C", "LFE", "R", "Rs"].get(i).unwrap_or(&"?").to_string(),
                                                    8 => ["Lrs", "Ls", "L", "C", "LFE", "R", "Rs", "Rrs"].get(i).unwrap_or(&"?").to_string(),
                                                    12 => ["Ltr", "Ltf", "Ls", "L", "C", "LFE", "R", "Rs", "Rtf", "Rtr", "Lrs", "Rrs"].get(i).unwrap_or(&"?").to_string(),
                                                    _ => format!("{}", i + 1),
                                                }
                                            };
                                            painter.text(
                                                egui::pos2(x + bw * 0.5, y_bottom + 2.0),
                                                egui::Align2::CENTER_TOP,
                                                label,
                                                egui::FontId::proportional(12.0),
                                                egui::Color32::GRAY,
                                            );
                                        }
                                    }
                                }

                                if !is_portrait {
                                    col.add_space(5.0);
                                    render_progress_bar(col, out_fire_rect, engine_action);
                                }
                            };

                            let render_col_heatmap = |col: &mut egui::Ui, out_heatmap_rect: &mut Option<egui::Rect>, engine_action: &mut Vec<EngineAction>| {
                                let mut eq_reset_clicked = false;
                                if !is_portrait {
                                    col.horizontal(|ui| {
                                        let (_, _, _, _, reset_clicked) = render_heatmap_eq_header(
                                            ui,
                                            state.show_surround_eq,
                                            state.lyrics.is_some(),
                                            !state.tracker_patterns_by_order.is_empty(),
                                            state.hardware_channels,
                                            engine_action,
                                        );
                                        eq_reset_clicked = reset_clicked;
                                    });
                                    col.separator();
                                }
                                if state.show_surround_eq {
                                    // Swap out the Tracker Pattern / Heatmap viewer with the Surround Routing & Crossover Curve
                                    // when the user flips the middle panel to the EQ view.
                                    *out_heatmap_rect = None;
                                    let mut cfg = state.surround_upmix;
                                    let mut cfg_changed = false;
                                    if eq_reset_clicked {
                                        cfg.reset_tuning();
                                        cfg_changed = true;
                                    }
                                    let has_height =
                                        crate::state::SurroundUpmixConfig::has_height_channels(
                                            state.hardware_channels,
                                        );

                                    let controls_h = 46.0;
                                    let graph_h = (col.available_height() - controls_h - 4.0).max(56.0);
                                    let (graph_rect, response) = col.allocate_exact_size(
                                        egui::vec2(col.available_width(), graph_h),
                                        egui::Sense::click_and_drag(),
                                    );

                                    let min_freq = 20.0f32;
                                    let max_freq = 20000.0f32;
                                    let log_span = (max_freq / min_freq).ln();
                                    let pad_x = 24.0f32;
                                    let pad_top = 14.0f32;
                                    let pad_bot = 15.0f32;
                                    let plot_rect = egui::Rect::from_min_max(
                                        egui::pos2(graph_rect.left() + pad_x, graph_rect.top() + pad_top),
                                        egui::pos2((graph_rect.right() - 8.0).max(graph_rect.left() + pad_x + 20.0), (graph_rect.bottom() - pad_bot).max(graph_rect.top() + pad_top + 20.0)),
                                    );

                                    let freq_to_x = |f: f32| -> f32 {
                                        let t = ((f.clamp(min_freq, max_freq) / min_freq).ln() / log_span).clamp(0.0, 1.0);
                                        plot_rect.left() + t * plot_rect.width()
                                    };
                                    let x_to_freq = |x: f32| -> f32 {
                                        let t = ((x - plot_rect.left()) / plot_rect.width().max(1.0)).clamp(0.0, 1.0);
                                        min_freq * (max_freq / min_freq).powf(t)
                                    };
                                    let db_to_y = |db: f32| -> f32 {
                                        let norm = ((15.0 - db.clamp(-15.0, 15.0)) / 30.0).clamp(0.0, 1.0);
                                        plot_rect.top() + norm * plot_rect.height()
                                    };
                                    let y_to_db = |y: f32| -> f32 {
                                        let norm = ((y - plot_rect.top()) / plot_rect.height().max(1.0)).clamp(0.0, 1.0);
                                        (15.0 - norm * 30.0).clamp(-12.0, 12.0)
                                    };
                                    let mag_to_y = |mag: f32| -> f32 {
                                        let db = 20.0 * mag.max(0.001).log10();
                                        db_to_y(db)
                                    };

                                    // Pointer interaction for 5 surround routing band nodes (1..=5), Sub Crossover (6), Height Crossover (7, only when has_height)
                                    let drag_id = egui::Id::unique("surround_eq_active_handle");
                                    let mut active_handle: u8 = col.ctx().data_mut(|d| *d.get_temp_mut_or_default::<u8>(drag_id));
                                    if !has_height && active_handle == 7 {
                                        active_handle = 0;
                                    }

                                    let pointer_pos = response
                                        .interact_pointer_pos()
                                        .or_else(|| col.ctx().pointer_latest_pos());

                                    if let Some(pos) = pointer_pos {
                                        if response.double_clicked() || response.secondary_clicked() {
                                            for (i, &bf) in crate::state::SurroundUpmixConfig::EQ_BAND_FREQS.iter().enumerate() {
                                                let node_pos = egui::pos2(freq_to_x(bf), db_to_y(cfg.band_gains_db[i]));
                                                if pos.distance(node_pos) <= 16.0 {
                                                    cfg.band_gains_db[i] = 0.0;
                                                    cfg_changed = true;
                                                }
                                            }
                                        }

                                        if response.drag_started() || (response.clicked() && active_handle == 0) {
                                            let mut best_h = 0u8;
                                            let mut best_dist = 18.0f32;
                                            for (i, &bf) in crate::state::SurroundUpmixConfig::EQ_BAND_FREQS.iter().enumerate() {
                                                let node_pos = egui::pos2(freq_to_x(bf), db_to_y(cfg.band_gains_db[i]));
                                                let d = pos.distance(node_pos);
                                                if d < best_dist {
                                                    best_dist = d;
                                                    best_h = (i + 1) as u8;
                                                }
                                            }
                                            let sub_x = freq_to_x(cfg.sub_crossover_hz);
                                            let sub_d = (pos.x - sub_x).abs();
                                            if sub_d < best_dist.min(13.0) {
                                                best_dist = sub_d;
                                                best_h = 6;
                                            }
                                            if has_height {
                                                let height_x = freq_to_x(cfg.height_crossover_hz);
                                                let height_d = (pos.x - height_x).abs();
                                                if height_d < best_dist.min(13.0) {
                                                    best_h = 7;
                                                }
                                            }
                                            active_handle = best_h;
                                            col.ctx().data_mut(|d| d.insert_temp(drag_id, active_handle));
                                        }

                                        if (response.dragged() || response.is_pointer_button_down_on()) && active_handle != 0 {
                                            col.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                            match active_handle {
                                                1..=5 => {
                                                    let idx = (active_handle - 1) as usize;
                                                    let new_db = (y_to_db(pos.y) * 4.0).round() / 4.0;
                                                    if (cfg.band_gains_db[idx] - new_db).abs() > 1e-3 {
                                                        cfg.band_gains_db[idx] = new_db;
                                                        cfg_changed = true;
                                                    }
                                                }
                                                6 => {
                                                    let new_sub = x_to_freq(pos.x).clamp(40.0, 250.0).round();
                                                    if (cfg.sub_crossover_hz - new_sub).abs() > 0.2 {
                                                        cfg.sub_crossover_hz = new_sub;
                                                        cfg_changed = true;
                                                    }
                                                }
                                                7 if has_height => {
                                                    let new_h = (x_to_freq(pos.x).clamp(800.0, 10000.0) / 10.0).round() * 10.0;
                                                    if (cfg.height_crossover_hz - new_h).abs() > 1.0 {
                                                        cfg.height_crossover_hz = new_h;
                                                        cfg_changed = true;
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }

                                    if (response.drag_stopped()
                                        || !response.is_pointer_button_down_on())
                                        && active_handle != 0
                                    {
                                        active_handle = 0;
                                        col.ctx().data_mut(|d| d.insert_temp(drag_id, 0u8));
                                    }

                                    if response.hovered() && active_handle == 0 {
                                        col.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }

                                    let painter = col.painter().with_clip_rect(graph_rect);
                                    // Studio dark glass backdrop
                                    painter.rect(
                                        graph_rect,
                                        5.0,
                                        egui::Color32::from_rgba_unmultiplied(18, 18, 20, 240),
                                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(110, 92, 74, 110)),
                                        egui::StrokeKind::Inside,
                                    );

                                    // Horizontal dB grid lines (+12, +6, 0, -6, -12 dB)
                                    for &db in &[12.0f32, 6.0, 0.0, -6.0, -12.0] {
                                        let y = db_to_y(db);
                                        let is_zero = db.abs() < 0.1;
                                        let line_col = if is_zero {
                                            egui::Color32::from_rgba_unmultiplied(190, 182, 172, 95)
                                        } else {
                                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18)
                                        };
                                        painter.line_segment(
                                            [egui::pos2(plot_rect.left(), y), egui::pos2(plot_rect.right(), y)],
                                            (if is_zero { 1.2 } else { 1.0 }, line_col),
                                        );
                                        let lbl = if db > 0.0 {
                                            format!("+{:.0}", db)
                                        } else {
                                            format!("{:.0}", db)
                                        };
                                        painter.text(
                                            egui::pos2(plot_rect.left() - 3.0, y),
                                            egui::Align2::RIGHT_CENTER,
                                            lbl,
                                            egui::FontId::proportional(9.5),
                                            egui::Color32::from_rgb(155, 148, 140),
                                        );
                                    }

                                    // Vertical log-frequency grid lines
                                    for &(f_grid, f_lbl) in &[
                                        (60.0f32, "60"),
                                        (120.0, "120"),
                                        (250.0, "250"),
                                        (500.0, "500"),
                                        (1000.0, "1k"),
                                        (2500.0, "2.5k"),
                                        (5000.0, "5k"),
                                        (10000.0, "10k"),
                                    ] {
                                        let x = freq_to_x(f_grid);
                                        painter.line_segment(
                                            [egui::pos2(x, plot_rect.top()), egui::pos2(x, plot_rect.bottom())],
                                            (1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 16)),
                                        );
                                        painter.text(
                                            egui::pos2(x, graph_rect.bottom() - 2.0),
                                            egui::Align2::CENTER_BOTTOM,
                                            f_lbl,
                                            egui::FontId::proportional(9.5),
                                            egui::Color32::from_rgb(155, 148, 140),
                                        );
                                    }

                                    // Live background spectrum bars colored by LFE / Bed / Height routing split
                                    if !state.spectrum_data.is_empty() {
                                        let num_bars = 72usize;
                                        let bar_step = plot_rect.width() / num_bars as f32;
                                        let spec_len = state.spectrum_data.len();
                                        let max_f_spec = state.max_frequency.max(2000.0);
                                        let log_spec_span = (max_f_spec / min_freq).ln();
                                        for b in 0..num_bars {
                                            let x0 = plot_rect.left() + b as f32 * bar_step;
                                            let f_bar = x_to_freq(x0 + bar_step * 0.5);
                                            let spec_t = ((f_bar / min_freq).ln() / log_spec_span).clamp(0.0, 1.0);
                                            let bin_idx = ((spec_t * (spec_len - 1) as f32).round() as usize).min(spec_len - 1);
                                            let val = (state.spectrum_data[bin_idx] / 100.0).clamp(0.0, 1.0);
                                            if val > 0.01 {
                                                let bh = val * plot_rect.height() * 0.85;
                                                let (_, lfe_m, bed_m, h_m) =
                                                    cfg.evaluate_band_routing_for_layout(
                                                        f_bar,
                                                        has_height,
                                                        state.current_sample_rate,
                                                    );
                                                let sum_m = (lfe_m + bed_m + h_m).max(0.01);
                                                let r_c = ((lfe_m * 220.0 + bed_m * 255.0 + h_m * 255.0) / sum_m).clamp(0.0, 255.0) as u8;
                                                let g_c = ((lfe_m * 68.0 + bed_m * 165.0 + h_m * 225.0) / sum_m).clamp(0.0, 255.0) as u8;
                                                let b_c = ((lfe_m * 32.0 + bed_m * 35.0 + h_m * 130.0) / sum_m).clamp(0.0, 255.0) as u8;
                                                let bar_rect = egui::Rect::from_min_max(
                                                    egui::pos2(x0 + 0.5, plot_rect.bottom() - bh),
                                                    egui::pos2(x0 + (bar_step - 0.8).max(1.0), plot_rect.bottom()),
                                                );
                                                painter.rect_filled(
                                                    bar_rect,
                                                    1.0,
                                                    egui::Color32::from_rgba_unmultiplied(r_c, g_c, b_c, 48),
                                                );
                                            }
                                        }
                                    }

                                    // Compute & draw LFE, Bed Surround, Height (if present), and Surround Routing Contour curves
                                    let samples = 112usize;
                                    let mut lfe_pts = Vec::with_capacity(samples);
                                    let mut bed_pts = Vec::with_capacity(samples);
                                    let mut height_pts = Vec::with_capacity(samples);
                                    let mut eq_pts = Vec::with_capacity(samples);

                                    for s in 0..samples {
                                        let t = s as f32 / (samples - 1) as f32;
                                        let x = plot_rect.left() + t * plot_rect.width();
                                        let f = x_to_freq(x);
                                        let (eq_gain, lfe_m, bed_m, height_m) =
                                            cfg.evaluate_band_routing_for_layout(
                                                f,
                                                has_height,
                                                state.current_sample_rate,
                                            );
                                        let y_lfe = mag_to_y(lfe_m);
                                        let y_bed = mag_to_y(bed_m);
                                        let y_height = mag_to_y(height_m);
                                        let y_eq = mag_to_y(eq_gain);

                                        lfe_pts.push(egui::pos2(x, y_lfe));
                                        bed_pts.push(egui::pos2(x, y_bed));
                                        if has_height {
                                            height_pts.push(egui::pos2(x, y_height));
                                        }
                                        eq_pts.push(egui::pos2(x, y_eq));

                                        // Subtle vertical tint columns under each curve for instant visual split clarity
                                        if s + 1 < samples {
                                            let col_w = (plot_rect.width() / (samples - 1) as f32) + 0.5;
                                            if y_bed < plot_rect.bottom() - 1.0 {
                                                painter.rect_filled(
                                                    egui::Rect::from_min_max(egui::pos2(x, y_bed), egui::pos2(x + col_w, plot_rect.bottom())),
                                                    0.0,
                                                    egui::Color32::from_rgba_unmultiplied(255, 160, 35, 18),
                                                );
                                            }
                                            if has_height && y_height < plot_rect.bottom() - 1.0 {
                                                painter.rect_filled(
                                                    egui::Rect::from_min_max(egui::pos2(x, y_height), egui::pos2(x + col_w, plot_rect.bottom())),
                                                    0.0,
                                                    egui::Color32::from_rgba_unmultiplied(255, 225, 130, 20),
                                                );
                                            }
                                            if y_lfe < plot_rect.bottom() - 1.0 {
                                                painter.rect_filled(
                                                    egui::Rect::from_min_max(egui::pos2(x, y_lfe), egui::pos2(x + col_w, plot_rect.bottom())),
                                                    0.0,
                                                    egui::Color32::from_rgba_unmultiplied(220, 65, 30, 26),
                                                );
                                            }
                                        }
                                    }

                                    painter.add(egui::Shape::line(
                                        lfe_pts,
                                        egui::Stroke::new(1.8_f32, egui::Color32::from_rgb(220, 68, 32)),
                                    ));
                                    painter.add(egui::Shape::line(
                                        bed_pts,
                                        egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 165, 35)),
                                    ));
                                    if has_height {
                                        painter.add(egui::Shape::line(
                                            height_pts,
                                            egui::Stroke::new(2.1_f32, egui::Color32::from_rgb(255, 225, 130)),
                                        ));
                                    }
                                    painter.add(egui::Shape::line(
                                        eq_pts,
                                        egui::Stroke::new(1.5_f32, egui::Color32::from_rgba_unmultiplied(220, 215, 208, 215)),
                                    ));

                                    // Draw vertical Crossover Handles (LFE Sub & optional Height Split)
                                    let sub_x = freq_to_x(cfg.sub_crossover_hz);
                                    let sub_col = if active_handle == 6 {
                                        egui::Color32::from_rgb(255, 150, 95)
                                    } else {
                                        egui::Color32::from_rgb(220, 68, 32)
                                    };
                                    painter.line_segment(
                                        [egui::pos2(sub_x, plot_rect.top()), egui::pos2(sub_x, plot_rect.bottom())],
                                        (1.5, sub_col),
                                    );
                                    let sub_badge_text = format!("SUB {:.0}Hz", cfg.sub_crossover_hz);
                                    let sub_galley = painter.layout_no_wrap(
                                        sub_badge_text,
                                        egui::FontId::proportional(10.0),
                                        egui::Color32::WHITE,
                                    );
                                    let sub_badge_rect = egui::Rect::from_center_size(
                                        egui::pos2(sub_x.clamp(plot_rect.left() + 28.0, plot_rect.right() - 28.0), graph_rect.top() + 8.0),
                                        sub_galley.size() + egui::vec2(8.0, 2.0),
                                    );
                                    painter.rect(
                                        sub_badge_rect,
                                        3.0,
                                        egui::Color32::from_rgba_unmultiplied(50, 20, 12, 230),
                                        egui::Stroke::new(1.0_f32, sub_col),
                                        egui::StrokeKind::Inside,
                                    );
                                    painter.galley(sub_badge_rect.min + egui::vec2(4.0, 1.0), sub_galley, egui::Color32::WHITE);

                                    if has_height {
                                        let height_x = freq_to_x(cfg.height_crossover_hz);
                                        let height_col = if active_handle == 7 {
                                            egui::Color32::from_rgb(255, 242, 185)
                                        } else {
                                            egui::Color32::from_rgb(255, 225, 130)
                                        };
                                        painter.line_segment(
                                            [egui::pos2(height_x, plot_rect.top()), egui::pos2(height_x, plot_rect.bottom())],
                                            (1.5, height_col),
                                        );
                                        let height_badge_text = if cfg.height_crossover_hz >= 1000.0 {
                                            format!("HEIGHT ↑ {:.1}kHz", cfg.height_crossover_hz / 1000.0)
                                        } else {
                                            format!("HEIGHT ↑ {:.0}Hz", cfg.height_crossover_hz)
                                        };
                                        let height_galley = painter.layout_no_wrap(
                                            height_badge_text,
                                            egui::FontId::proportional(10.0),
                                            egui::Color32::WHITE,
                                        );
                                        let height_badge_rect = egui::Rect::from_center_size(
                                            egui::pos2(height_x.clamp(plot_rect.left() + 42.0, plot_rect.right() - 42.0), graph_rect.top() + 8.0),
                                            height_galley.size() + egui::vec2(8.0, 2.0),
                                        );
                                        painter.rect(
                                            height_badge_rect,
                                            3.0,
                                            egui::Color32::from_rgba_unmultiplied(46, 36, 16, 235),
                                            egui::Stroke::new(1.0_f32, height_col),
                                            egui::StrokeKind::Inside,
                                        );
                                        painter.galley(height_badge_rect.min + egui::vec2(4.0, 1.0), height_galley, egui::Color32::WHITE);
                                    }

                                    // Draw the 5 interactive Surround Routing Band Nodes
                                    for (i, &bf) in crate::state::SurroundUpmixConfig::EQ_BAND_FREQS.iter().enumerate() {
                                        let gain_db = cfg.band_gains_db[i];
                                        let node_pos = egui::pos2(freq_to_x(bf), db_to_y(gain_db));
                                        let is_active = active_handle == (i + 1) as u8;
                                        let node_color = if bf <= cfg.sub_crossover_hz * 1.1 {
                                            egui::Color32::from_rgb(225, 75, 35)
                                        } else if has_height && bf >= cfg.height_crossover_hz {
                                            egui::Color32::from_rgb(255, 225, 130)
                                        } else {
                                            egui::Color32::from_rgb(255, 165, 35)
                                        };
                                        painter.circle_filled(
                                            node_pos,
                                            if is_active { 7.0 } else { 5.5 },
                                            egui::Color32::from_rgb(24, 22, 20),
                                        );
                                        painter.circle_stroke(
                                            node_pos,
                                            if is_active { 7.0 } else { 5.5 },
                                            egui::Stroke::new(
                                                if is_active { 2.5_f32 } else { 2.0_f32 },
                                                node_color,
                                            ),
                                        );
                                        painter.circle_filled(node_pos, 2.5, egui::Color32::WHITE);

                                        if is_active || gain_db.abs() >= 0.25 {
                                            let sign = if gain_db > 0.0 { "+" } else { "" };
                                            let db_txt = format!("{}{:.1}dB", sign, gain_db);
                                            let lbl_y = if node_pos.y > plot_rect.top() + 18.0 {
                                                node_pos.y - 11.0
                                            } else {
                                                node_pos.y + 11.0
                                            };
                                            painter.text(
                                                egui::pos2(node_pos.x, lbl_y),
                                                egui::Align2::CENTER_CENTER,
                                                db_txt,
                                                egui::FontId::proportional(10.0),
                                                egui::Color32::WHITE,
                                            );
                                        }
                                    }

                                    // Bottom compact speaker routing & level sliders (2 rows x 3 columns),
                                    // tailored to Bed + Height (>8ch) vs Bed-Only (3..=8ch) systems
                                    col.add_space(3.0);
                                    col.columns(3, |cols| {
                                        cols[0].horizontal(|ui| {
                                            ui.spacing_mut().slider_width = (ui.available_width() - 56.0).max(28.0);
                                            ui.label(egui::RichText::new("Front").size(11.0).color(egui::Color32::from_rgb(210, 205, 198)));
                                            if ui
                                                .add(egui::Slider::new(&mut cfg.front_level, 0.0..=1.5).show_value(false))
                                                .on_hover_text(format!("Front L/R Anchor Level (uncolored): {:.0}%", cfg.front_level * 100.0))
                                                .changed()
                                            {
                                                cfg_changed = true;
                                            }
                                        });
                                        cols[1].horizontal(|ui| {
                                            ui.spacing_mut().slider_width = (ui.available_width() - 62.0).max(28.0);
                                            ui.label(egui::RichText::new("Side/Rear").size(11.0).color(egui::Color32::from_rgb(255, 165, 35)));
                                            if ui
                                                .add(egui::Slider::new(&mut cfg.side_rear_level, 0.0..=1.5).show_value(false))
                                                .on_hover_text(format!("Side & Rear Surround Bed Level: {:.0}%", cfg.side_rear_level * 100.0))
                                                .changed()
                                            {
                                                cfg_changed = true;
                                            }
                                        });
                                        cols[2].horizontal(|ui| {
                                            ui.spacing_mut().slider_width = (ui.available_width() - 58.0).max(28.0);
                                            if has_height {
                                                ui.label(egui::RichText::new("Height ↑").size(11.0).color(egui::Color32::from_rgb(255, 225, 130)));
                                                if ui
                                                    .add(egui::Slider::new(&mut cfg.height_level, 0.0..=1.5).show_value(false))
                                                    .on_hover_text(format!("Overhead Height (Top Front / Top Rear) Level: {:.0}%", cfg.height_level * 100.0))
                                                    .changed()
                                                {
                                                    cfg_changed = true;
                                                }
                                            } else {
                                                ui.label(egui::RichText::new("Width").size(11.0).color(egui::Color32::from_rgb(235, 185, 105)));
                                                if ui
                                                    .add(egui::Slider::new(&mut cfg.stereo_width, 0.0..=2.0).show_value(false))
                                                    .on_hover_text(format!("Surround Bed Ambient Width: {:.0}%", cfg.stereo_width * 100.0))
                                                    .changed()
                                                {
                                                    cfg_changed = true;
                                                }
                                            }
                                        });
                                    });
                                    col.columns(3, |cols| {
                                        cols[0].horizontal(|ui| {
                                            ui.spacing_mut().slider_width = (ui.available_width() - 56.0).max(28.0);
                                            ui.label(egui::RichText::new("LFE Sub").size(11.0).color(egui::Color32::from_rgb(225, 75, 35)));
                                            if ui
                                                .add(egui::Slider::new(&mut cfg.lfe_level, 0.0..=1.5).show_value(false))
                                                .on_hover_text(format!("LFE Subwoofer Level: {:.0}%", cfg.lfe_level * 100.0))
                                                .changed()
                                            {
                                                cfg_changed = true;
                                            }
                                        });
                                        cols[1].horizontal(|ui| {
                                            ui.spacing_mut().slider_width = (ui.available_width() - 62.0).max(28.0);
                                            ui.label(egui::RichText::new("Center").size(11.0).color(egui::Color32::from_rgb(245, 195, 85)));
                                            if ui
                                                .add(egui::Slider::new(&mut cfg.center_level, 0.0..=1.5).show_value(false))
                                                .on_hover_text(format!("Center Speaker Mid Level: {:.0}% (0% = Pure Left/Right Side Split)", cfg.center_level * 100.0))
                                                .changed()
                                            {
                                                cfg_changed = true;
                                            }
                                        });
                                        cols[2].horizontal(|ui| {
                                            ui.spacing_mut().slider_width = (ui.available_width() - 58.0).max(28.0);
                                            if has_height {
                                                ui.label(egui::RichText::new("Hi-Bleed").size(11.0).color(egui::Color32::from_rgb(215, 195, 165)));
                                                if ui
                                                    .add(egui::Slider::new(&mut cfg.surround_high_bleed, 0.0..=1.0).show_value(false))
                                                    .on_hover_text(format!("High-Frequency Bleed in Ear-Level Side/Rear Speakers: {:.0}% (0% routes highs exclusively to Height speakers)", cfg.surround_high_bleed * 100.0))
                                                    .changed()
                                                {
                                                    cfg_changed = true;
                                                }
                                            } else {
                                                ui.label(egui::RichText::new("Sub X-O").size(11.0).color(egui::Color32::from_rgb(235, 120, 60)));
                                                if ui
                                                    .add(egui::Slider::new(&mut cfg.sub_crossover_hz, 40.0..=250.0).show_value(false))
                                                    .on_hover_text(format!("LFE Subwoofer Low-Pass Crossover: {:.0} Hz", cfg.sub_crossover_hz))
                                                    .changed()
                                                {
                                                    cfg_changed = true;
                                                }
                                            }
                                        });
                                    });

                                    if cfg_changed {
                                        engine_action.push(EngineAction::SetSurroundUpmix(cfg));
                                    }
                                    return;
                                }

                                let hm_rect = col.available_rect_before_wrap();
                                *out_heatmap_rect = Some(hm_rect);

                                col.painter().rect_filled(hm_rect, 0.0, egui::Color32::TRANSPARENT);

                                let painter = col.painter().with_clip_rect(hm_rect);
                                let chunks = 64;
                                let cell_w = hm_rect.width() / chunks as f32;

                                for c in 0..=chunks {
                                    let x = hm_rect.left() + c as f32 * cell_w;
                                    painter.line_segment([egui::pos2(x, hm_rect.top()), egui::pos2(x, hm_rect.bottom())], (1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 5)));
                                }

                                if let Some(lyrics) = &state.lyrics {
                                    // Layer 1: Backdrop Scrim - Dim heatmap slightly for crisp lyrics readability
                                    painter.rect_filled(
                                        hm_rect,
                                        0.0,
                                        egui::Color32::from_rgba_unmultiplied(10, 10, 14, 175)
                                    );

                                    let display_secs = state.scrub_target_seconds.unwrap_or(state.current_seconds);
                                    let center_y = hm_rect.top() + hm_rect.height() / 2.0;
                                    let row_height = 32.0;
                                    let num_rows_to_draw = (hm_rect.height() / row_height) as i32;
                                    let half_rows = (num_rows_to_draw / 2).max(1);

                                    let active_idx = lyrics.find_current_line_idx(display_secs);
                                    let is_intro = active_idx.is_none();
                                    let base_idx = active_idx.unwrap_or(0);

                                    for offset in -half_rows..=half_rows {
                                        let target_idx = (base_idx as i32) + offset - if is_intro && offset > 0 { 1 } else { 0 };
                                        let y = center_y + (offset as f32) * row_height;

                                        let distance = offset.abs() as f32 / (half_rows as f32);
                                        let alpha = (1.0 - distance * 0.80).clamp(0.0, 1.0);
                                        if alpha <= 0.02 {
                                            continue;
                                        }

                                        if is_intro && offset == 0 {
                                            // Active Intro indicator
                                            let font_id = egui::FontId::proportional(18.0);
                                            let text = "♪   ♪   ♪   (Intro)";
                                            let galley = painter.layout_no_wrap(
                                                text.to_string(),
                                                font_id,
                                                egui::Color32::from_rgba_unmultiplied(225, 225, 240, 240),
                                            );
                                            let pos = egui::pos2(hm_rect.center().x, y);
                                            let rect = egui::Rect::from_center_size(pos, galley.size());
                                            painter.rect(
                                                rect.expand2(egui::vec2(14.0, 4.0)),
                                                5.0,
                                                egui::Color32::from_rgba_unmultiplied(14, 14, 18, 235),
                                                egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 50)),
                                                egui::StrokeKind::Outside,
                                            );
                                            painter.galley(rect.min, galley, egui::Color32::from_rgba_unmultiplied(225, 225, 240, 240));
                                            continue;
                                        }

                                        if is_intro && offset < 0 {
                                            continue;
                                        }

                                        if target_idx >= 0 && (target_idx as usize) < lyrics.lines.len() {
                                            let line = &lyrics.lines[target_idx as usize];
                                            let is_current = !is_intro && offset == 0;
                                            let text_display = if line.text.is_empty() {
                                                "♪   ♪   ♪"
                                            } else {
                                                line.text.as_str()
                                            };

                                            let pos = egui::pos2(hm_rect.center().x, y);

                                            if is_current {
                                                // Active line: prominent large font + high contrast pill with subtle luminous border
                                                let font_id = egui::FontId::proportional(20.0);
                                                let max_w = (hm_rect.width() - 24.0).max(50.0);
                                                let galley = painter.layout(
                                                    text_display.to_string(),
                                                    font_id,
                                                    egui::Color32::WHITE,
                                                    max_w,
                                                );
                                                let rect = egui::Rect::from_center_size(pos, galley.size());

                                                painter.rect(
                                                    rect.expand2(egui::vec2(16.0, 5.0)),
                                                    6.0,
                                                    egui::Color32::from_rgba_unmultiplied(12, 12, 16, 240),
                                                    egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60)),
                                                    egui::StrokeKind::Outside,
                                                );
                                                painter.galley(rect.min, galley, egui::Color32::WHITE);
                                            } else {
                                                // Surrounding lines: readable 16pt font with smooth distance fade + soft dark backdrop for contrast
                                                let font_id = egui::FontId::proportional(16.0);
                                                let max_w = (hm_rect.width() - 24.0).max(50.0);
                                                let text_color = if line.text.is_empty() {
                                                    egui::Color32::from_rgba_unmultiplied(150, 150, 160, (alpha * 140.0) as u8)
                                                } else {
                                                    egui::Color32::from_rgba_unmultiplied(225, 225, 235, (alpha * 200.0) as u8)
                                                };
                                                let galley = painter.layout(
                                                    text_display.to_string(),
                                                    font_id,
                                                    text_color,
                                                    max_w,
                                                );
                                                let rect = egui::Rect::from_center_size(pos, galley.size());

                                                let pill_alpha = (alpha * 180.0) as u8;
                                                if pill_alpha > 15 {
                                                    painter.rect_filled(
                                                        rect.expand2(egui::vec2(10.0, 3.0)),
                                                        4.0,
                                                        egui::Color32::from_rgba_unmultiplied(10, 10, 14, pill_alpha),
                                                    );
                                                }
                                                painter.galley(rect.min, galley, text_color);
                                            }
                                        }
                                    }
                                } else if !state.tracker_patterns_by_order.is_empty() {
                                    let current_order = state.current_tracker_order;
                                    let current_row = state.current_tracker_row;
                                    let center_y = hm_rect.top() + hm_rect.height() / 2.0;
                                    let row_height = 16.0;
                                    let num_rows_to_draw = (hm_rect.height() / row_height) as i32;

                                    let font_id = egui::FontId::monospace(12.0);
                                    let char_width = 7.5; // Monospace char width at 12pt with safety margin
                                    let max_chars = ((hm_rect.width() - 24.0) / char_width).max(10.0) as usize;
                                    let max_text_chars = max_chars.saturating_sub(4);

                                    let mut formatted = String::with_capacity(max_text_chars + 16);

                                    for offset in -(num_rows_to_draw / 2)..=(num_rows_to_draw / 2) {
                                        let mut resolved_order = current_order;
                                        let mut resolved_row = current_row + offset;

                                        if offset < 0 {
                                            // Read exact playback sequence from history
                                            let history_idx = (-offset - 1) as usize;
                                            if history_idx < state.tracker_row_history.len() {
                                                let (hist_order, hist_row) = state.tracker_row_history[history_idx];
                                                resolved_order = hist_order;
                                                resolved_row = hist_row;
                                            } else {
                                                // Fall back to underflow if history hasn't built up yet
                                                while resolved_row < 0 && resolved_order > 0 {
                                                    resolved_order -= 1;
                                                    if (resolved_order as usize) < state.tracker_patterns_by_order.len() {
                                                        resolved_row += state.tracker_patterns_by_order[resolved_order as usize].len() as i32;
                                                    } else {
                                                        break;
                                                    }
                                                }
                                            }
                                        } else if offset > 0 {
                                            // Overflow forwards within the current pattern, clamped
                                            while (resolved_order as usize) < state.tracker_patterns_by_order.len() {
                                                let pattern_len = state.tracker_patterns_by_order[resolved_order as usize].len() as i32;
                                                if resolved_row >= pattern_len {
                                                    resolved_row -= pattern_len;
                                                    resolved_order += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                        }

                                        if resolved_order >= 0 && (resolved_order as usize) < state.tracker_patterns_by_order.len() {
                                            let pattern = &state.tracker_patterns_by_order[resolved_order as usize];
                                            if resolved_row >= 0 && (resolved_row as usize) < pattern.len() {
                                                let row_str = &pattern[resolved_row as usize];

                                                let distance = offset.abs() as f32 / (num_rows_to_draw as f32 / 2.0);
                                                let alpha = (1.0 - distance * 0.75).clamp(0.0, 1.0);
                                                let y = center_y + (offset as f32) * row_height;

                                                // Format tracker row text
                                                formatted.clear();
                                                let text_slice = if row_str.chars().count() > max_text_chars {
                                                    let mut end = row_str.len();
                                                    for (char_count, (byte_idx, _)) in row_str.char_indices().enumerate() {
                                                        if char_count == max_text_chars {
                                                            end = byte_idx;
                                                            break;
                                                        }
                                                    }
                                                    &row_str[..end]
                                                } else {
                                                    row_str.as_str()
                                                };

                                                use std::fmt::Write;
                                                let _ = write!(formatted, "{:02} | {}", resolved_row, text_slice);

                                                let pos = egui::pos2(hm_rect.left() + 10.0, y);
                                                if offset == 0 {
                                                    // Prominent highlight background on the active playback row
                                                    let galley = painter.layout_no_wrap(
                                                        formatted.clone(),
                                                        font_id.clone(),
                                                        egui::Color32::WHITE,
                                                    );
                                                    let rect = egui::Rect::from_min_size(
                                                        pos + egui::vec2(0.0, -galley.size().y / 2.0),
                                                        galley.size(),
                                                    );
                                                    painter.rect_filled(
                                                        rect.expand2(egui::vec2(10.0, 2.0)),
                                                        4.0,
                                                        egui::Color32::from_black_alpha(220)
                                                    );
                                                    painter.galley(rect.min, galley, egui::Color32::WHITE);
                                                } else {
                                                    // Valid unmultiplied alpha color
                                                    let color = egui::Color32::from_rgba_unmultiplied(150, 150, 150, (alpha * 100.0) as u8);

                                                    let galley = painter.layout_no_wrap(
                                                        formatted.clone(),
                                                        font_id.clone(),
                                                        egui::Color32::WHITE,
                                                    );

                                                    let rect = egui::Rect::from_min_size(
                                                        pos + egui::vec2(0.0, -galley.size().y / 2.0),
                                                        galley.size(),
                                                    );
                                                    painter.galley(rect.min, galley, color);
                                                }

                                                // Pattern boundary indicator
                                                if resolved_row == 0 {
                                                    painter.line_segment(
                                                        [egui::pos2(hm_rect.left(), y - row_height / 2.0), egui::pos2(hm_rect.right(), y - row_height / 2.0)],
                                                        (1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (alpha * 150.0) as u8))
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            };

                            let mut render_col_info = |col: &mut egui::Ui, out_track_info_rect: &mut Option<egui::Rect>, engine_action: &mut Vec<EngineAction>| {
                                if state.video_mode == 1 && self.video_state.is_some() {
                                    let available = col.available_size();
                                    let (rect, _) = col.allocate_exact_size(available, egui::Sense::hover());
                                    *out_track_info_rect = Some(rect);
                                } else {
                                    col.style_mut().visuals.override_text_color = Some(egui::Color32::from_gray(235));
                                    let heading_text = if state.playlist.len() > 1 {
                                        format!("Track Info ({}/{})", state.playlist_index + 1, state.playlist.len())
                                    } else {
                                        "Track Info".to_string()
                                    };
                                    col.horizontal(|ui| {
                                        ui.heading(heading_text);
                                        let _ = render_track_info_open_area(ui, is_portrait, engine_action);
                                    });
                                    col.separator();

                                    let render_smooth_marquee = |ui: &mut egui::Ui, text: &str, size: f32, is_title: bool| {
                                        let available_width = ui.available_width();
                                        let font_id = egui::FontId::proportional(size);
                                        let color = ui.style().visuals.override_text_color.unwrap_or(egui::Color32::from_gray(235));
                                        let text_color = if is_title { egui::Color32::WHITE } else { color };

                                        let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, text_color);
                                        let text_width = galley.rect.width();
                                        let height = galley.rect.height();

                                        let (rect, _response) = ui.allocate_exact_size(egui::vec2(available_width, height), egui::Sense::hover());
                                        let painter = ui.painter().with_clip_rect(rect);

                                        if text_width > available_width {
                                            let max_scroll = text_width - available_width;
                                            let scroll_duration = max_scroll / 35.0;
                                            let total_period = 2.0 + scroll_duration + 2.0 + scroll_duration;
                                            let t = (state.current_seconds as f32) % total_period;

                                            let offset = if t < 2.0 {
                                                0.0
                                            } else if t < 2.0 + scroll_duration {
                                                let progress = (t - 2.0) / scroll_duration;
                                                progress * max_scroll
                                            } else if t < 2.0 + scroll_duration + 2.0 {
                                                max_scroll
                                            } else {
                                                let progress = (t - (2.0 + scroll_duration + 2.0)) / scroll_duration;
                                                max_scroll - (progress * max_scroll)
                                            };

                                            painter.galley(rect.min + egui::vec2(-offset, 0.0), galley, text_color);
                                        } else {
                                            painter.galley(rect.min, galley, text_color);
                                        }
                                    };

                                    let strip_android_cache_prefix = |s: &str| -> String {
                                        if let Some(rest) = s.strip_prefix("track_")
                                            && let Some(idx) = rest.find('_')
                                            && idx > 0
                                            && rest[..idx].chars().all(|c| c.is_ascii_digit())
                                        {
                                            return rest[idx + 1..].to_string();
                                        }
                                        s.to_string()
                                    };

                                    let raw_display_title = if state.song_title.is_empty() {
                                        "Unknown Title".to_string()
                                    } else {
                                        let p = std::path::Path::new(&state.song_title);
                                        if p.extension().is_some() || state.song_title.contains('/') || state.song_title.contains('\\') {
                                            p.file_stem().unwrap_or_default().to_string_lossy().to_string()
                                        } else {
                                            state.song_title.clone()
                                        }
                                    };
                                    let display_title = strip_android_cache_prefix(&raw_display_title);

                                    let current_path_str = if state.playlist_index < state.playlist.len() {
                                        state.playlist[state.playlist_index].clone()
                                    } else {
                                        state.song_title.clone()
                                    };
                                    let is_mic_stream = crate::state::is_mic_uri(&current_path_str)
                                        || crate::state::is_mic_uri(&state.song_title)
                                        || state.is_mic_input;
                                    let is_network = current_path_str.starts_with("http://") || current_path_str.starts_with("https://");
                                    let file_path = std::path::Path::new(&current_path_str);
                                    let file_name = if is_network || is_mic_stream {
                                        display_title.clone()
                                    } else {
                                        let raw_fname = file_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                                        strip_android_cache_prefix(&raw_fname)
                                    };
                                    if self.cached_meta_path != current_path_str {
                                        self.cached_meta_path = current_path_str.clone();
                                        if is_network || is_mic_stream {
                                            self.cached_file_dir = current_path_str.clone();
                                            self.cached_file_len = None;
                                        } else {
                                            let abs_path = if std::path::Path::new(&current_path_str).is_absolute() {
                                                std::path::PathBuf::from(&current_path_str)
                                            } else if let Ok(cwd) = std::env::current_dir() {
                                                cwd.join(&current_path_str)
                                            } else {
                                                std::path::PathBuf::from(&current_path_str)
                                            };
                                            let dir_path = abs_path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                                            self.cached_file_dir = if let Ok(home) = std::env::var("HOME") {
                                                if dir_path.starts_with(&home) {
                                                    dir_path.replacen(&home, "~", 1)
                                                } else {
                                                    dir_path
                                                }
                                            } else {
                                                dir_path
                                            };
                                            self.cached_file_len = std::fs::metadata(&current_path_str).ok().map(|m| m.len());
                                        }
                                    }
                                    let file_dir = self.cached_file_dir.clone();
                                    let cached_file_len = self.cached_file_len;

                                    let avail_info_h = col.available_height().max(32.0);
                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .max_height(avail_info_h)
                                        .min_scrolled_height(32.0)
                                        .show(col, |ui| {
                                            egui::Grid::new("track_info_meta_grid")
                                                .num_columns(2)
                                                .spacing([10.0, 4.0])
                                                .show(ui, |ui| {
                                                    // 1. Song Title
                                                    ui.label(egui::RichText::new("Title:").color(egui::Color32::from_rgb(160, 180, 200)).strong());
                                                    render_smooth_marquee(ui, &display_title, 14.0, true);
                                                    ui.end_row();

                                                    // 2. Artist (only display if known)
                                                    let trimmed_artist = state.artist.trim();
                                                    if !trimmed_artist.is_empty()
                                                        && !trimmed_artist.eq_ignore_ascii_case("unknown")
                                                        && !trimmed_artist.eq_ignore_ascii_case("unknown artist")
                                                    {
                                                        ui.label(egui::RichText::new("Artist:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                        render_smooth_marquee(ui, trimmed_artist, 14.0, false);
                                                        ui.end_row();
                                                    }

                                                    // 3. File Name
                                                    if !is_mic_stream {
                                                        ui.label(egui::RichText::new("File:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                        render_smooth_marquee(ui, &file_name, 14.0, false);
                                                        ui.end_row();
                                                    }

                                                    // 4. File Directory / URL / Input Device (skip internal Android cache path)
                                                    if is_mic_stream {
                                                        let input_dev = state
                                                            .selected_audio_device
                                                            .as_deref()
                                                            .or_else(|| crate::state::device_from_mic_uri(&current_path_str))
                                                            .unwrap_or(crate::audio::SYSTEM_DEFAULT_AUDIO_DEVICE);
                                                        ui.label(egui::RichText::new("Input:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                        render_smooth_marquee(ui, input_dev, 14.0, false);
                                                        ui.end_row();
                                                    } else {
                                                        let is_android_cache_dir = file_dir.contains("com.rusttracker.app/cache");
                                                        if !is_android_cache_dir {
                                                            let folder_label = if is_network { "Stream:" } else { "Folder:" };
                                                            ui.label(egui::RichText::new(folder_label).color(egui::Color32::from_rgb(160, 180, 200)));
                                                            render_smooth_marquee(ui, &file_dir, 14.0, false);
                                                            ui.end_row();
                                                        }
                                                    }

                                                    // 5 & 6. Format, Bitrate & Channels
                                                    let effective_bitrate = state.bitrate.or_else(|| {
                                                        if state.duration_seconds > 0.0 && !is_network {
                                                            cached_file_len.map(|len| ((len as f64 * 8.0) / (state.duration_seconds * 1000.0)).round() as u32)
                                                        } else {
                                                            None
                                                        }
                                                    });

                                                    let format_name = if let Some(track) = state.audio_tracks.get(state.selected_audio_track) {
                                                        if !track.codec.is_empty()
                                                            && !state.module_type.to_ascii_uppercase().contains(&track.codec.to_ascii_uppercase())
                                                        {
                                                            format!("{}/{}", track.codec, state.module_type)
                                                        } else {
                                                            state.module_type.clone()
                                                        }
                                                    } else {
                                                        state.module_type.clone()
                                                    };

                                                    let format_name = if let Some(ext) = file_path.extension().and_then(|e| e.to_str()) {
                                                        let ext_upper = ext.to_ascii_uppercase();
                                                        if (ext_upper == "MP4" || ext_upper == "M4A" || ext_upper == "MOV" || ext_upper == "MKV")
                                                            && !format_name.to_ascii_uppercase().contains(&ext_upper)
                                                        {
                                                            format!("{}/{}", format_name, ext_upper)
                                                        } else {
                                                            format_name
                                                        }
                                                    } else {
                                                        format_name
                                                    };

                                                    let ch_info = if state.is_native_atmos {
                                                        "Dolby Atmos (7.1.4)".to_string()
                                                    } else if state.stats.mpegh_bitstream_active {
                                                        let is_atmos_spatial = state
                                                            .stats
                                                            .mpegh_carrier_info
                                                            .contains("SpatialAudio")
                                                            || state.stats.mpegh_carrier_info.contains("Dolby");
                                                        let carrier_tag = if is_atmos_spatial {
                                                            "Dolby Atmos"
                                                        } else {
                                                            "MPEG-H"
                                                        };
                                                        if let Some(tc) = state.tracker_channels {
                                                            format!("11.1 {} (12ch) / {} tracker", carrier_tag, tc)
                                                        } else if state.source_channels >= 12 {
                                                            format!("11.1 {} (7.1.4 / 12ch)", carrier_tag)
                                                        } else {
                                                            let src_label = match state.source_channels {
                                                                1 => "Mono (1ch)".to_string(),
                                                                2 => "Stereo (2ch)".to_string(),
                                                                6 => "5.1 (6ch)".to_string(),
                                                                8 => "7.1 (8ch)".to_string(),
                                                                10 => "7.1.2 (10ch)".to_string(),
                                                                n => format!("{}ch", n.max(1)),
                                                            };
                                                            if state.surround_upmix.is_active_for(state.source_channels, state.hardware_channels) {
                                                                format!("{} -> 7.1.4 {} Upmix (12ch)", src_label, carrier_tag)
                                                            } else {
                                                                format!("{} via {} 7.1.4", src_label, carrier_tag)
                                                            }
                                                        }
                                                    } else if let Some(tc) = state.tracker_channels {
                                                        format!("{} hw / {} tracker", state.hardware_channels, tc)
                                                    } else if state.num_channels == 1 {
                                                        "1-channel".to_string()
                                                    } else {
                                                        format!("{}-channels", state.num_channels)
                                                    };

                                                    if is_portrait {
                                                        // In mobile view: "Audio: AAC/MP4, X-channels, Y bitrate"
                                                        ui.label(egui::RichText::new("Audio:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                        let audio_val = if let Some(br) = effective_bitrate {
                                                            format!("{}, {}, {} kbps", format_name, ch_info, br)
                                                        } else {
                                                            format!("{}, {}", format_name, ch_info)
                                                        };
                                                        ui.label(audio_val);
                                                        ui.end_row();
                                                    } else {
                                                        // Desktop / Wide layout: separate lines
                                                        let format_str = if let Some(br) = effective_bitrate {
                                                            format!("{} ({} kbps)", format_name, br)
                                                        } else {
                                                            format_name
                                                        };
                                                        ui.label(egui::RichText::new("Format:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                        ui.label(format_str);
                                                        ui.end_row();

                                                        ui.label(egui::RichText::new("Channels:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                        ui.label(ch_info);
                                                        ui.end_row();
                                                    }

                                                    // 7. Track Duration
                                                    ui.label(egui::RichText::new("Duration:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                    if is_mic_stream {
                                                        ui.label("Live Input");
                                                    } else if state.duration_seconds <= 0.0 {
                                                        ui.label("Live Stream");
                                                    } else {
                                                        let formatted = crate::state::format_timeline_time(
                                                            state.duration_seconds,
                                                            state.duration_seconds >= 3600.0,
                                                        );
                                                        ui.label(format!("{} ({:.1}s)", formatted, state.duration_seconds));
                                                    }
                                                    ui.end_row();

                                                    // 8. Synchronized Lyrics Source (if loaded)
                                                    if let Some(lyrics) = &state.lyrics {
                                                        let lrc_name = lyrics.file_name.as_deref().unwrap_or("Synchronized LRC");
                                                        ui.label(egui::RichText::new("Lyrics:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                        render_smooth_marquee(ui, lrc_name, 14.0, false);
                                                        ui.end_row();
                                                    }

                                                    // 9. Active Visualizer Module
                                                    let vis_name = crate::state::VISUALIZERS
                                                        .get(state.current_visualizer_idx)
                                                        .map(|v| v.name)
                                                        .unwrap_or("Unknown");
                                                    ui.label(egui::RichText::new("Visualizer:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                    ui.horizontal(|ui| {
                                                        let vis_link = ui.link(egui::RichText::new(vis_name).color(egui::Color32::from_rgb(255, 215, 0)).strong());
                                                        if vis_link.clicked() {
                                                            engine_action.push(EngineAction::ToggleVisPicker);
                                                        }
                                                        vis_link.on_hover_text("Click to open Visualization Modules selector (Shortcut: 'm')");
                                                        if state.visualizer_mode == 31 {
                                                            let (smoke_lbl, smoke_fill, smoke_stroke, smoke_col) = if state.room_volume_smoke {
                                                                (
                                                                    "💨 Smoke: ON",
                                                                    egui::Color32::from_rgb(18, 62, 78),
                                                                    egui::Color32::from_rgb(0, 205, 245),
                                                                    egui::Color32::from_rgb(205, 248, 255),
                                                                )
                                                            } else {
                                                                (
                                                                    "💨 Smoke: OFF",
                                                                    egui::Color32::from_rgb(38, 38, 44),
                                                                    egui::Color32::from_rgb(95, 95, 108),
                                                                    egui::Color32::from_rgb(185, 188, 198),
                                                                )
                                                            };
                                                            let smoke_btn = egui::Button::new(
                                                                egui::RichText::new(smoke_lbl)
                                                                    .size(10.5)
                                                                    .strong()
                                                                    .color(smoke_col),
                                                            )
                                                            .fill(smoke_fill)
                                                            .stroke(egui::Stroke::new(1.0_f32, smoke_stroke))
                                                            .corner_radius(4.0);
                                                            if ui
                                                                .add(smoke_btn)
                                                                .on_hover_text("Toggle volumetric speaker smoke blasts & standing-wave vapor (Shortcut: 'c')")
                                                                .clicked()
                                                            {
                                                                engine_action.push(EngineAction::ToggleRoomVolumeSmoke);
                                                            }
                                                        }
                                                    });
                                                    ui.end_row();
                                                });

                                            if state.audio_tracks.len() > 1 {
                                                ui.add_space(6.0);
                                                ui.horizontal(|ui| {
                                                    ui.label(egui::RichText::new("Audio Tracks:").strong().color(egui::Color32::from_rgb(0, 210, 255)));
                                                });
                                                ui.add_space(3.0);

                                                // Multi-Track Mix Interface
                                                let total_avail_w = ui.available_width();
                                                // Fixed controls width:
                                                // Checkbox: 24.0
                                                // 5 gaps of 4.0: 20.0
                                                // Slider rail: 55.0
                                                // Percentage text: 28.0
                                                // Mute button [M]: 22.0
                                                // Solo button [S]: 22.0
                                                // Total non-title = 24.0 + 20.0 + 55.0 + 28.0 + 22.0 + 22.0 = 171.0
                                                let title_w = (total_avail_w - 171.0).max(60.0);

                                                for (idx, track) in state.audio_tracks.iter().enumerate() {
                                                    let is_in_mix = state.active_audio_tracks.contains(&idx);
                                                    let lower = track.title.to_lowercase();
                                                    let (icon, color) = if lower.contains("vocal") || lower.contains("guide") {
                                                        ("♪", egui::Color32::from_rgb(255, 110, 180))
                                                    } else if lower.contains("instrumental") || lower.contains("karaoke") || lower.contains("music") {
                                                        ("♫", egui::Color32::from_rgb(0, 230, 160))
                                                    } else {
                                                        ("♪", egui::Color32::from_rgb(255, 200, 60))
                                                    };

                                                    let mut current_vol = state.audio_track_volumes.get(idx).copied().unwrap_or(1.0);

                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut().item_spacing.x = 4.0;

                                                        // 1. Checkbox button to toggle track into/out of mix
                                                        let chk_label = if is_in_mix { "☑" } else { "☐" };
                                                        let chk_btn = egui::Button::new(
                                                            egui::RichText::new(chk_label)
                                                                .color(if is_in_mix { color } else { egui::Color32::GRAY })
                                                                .size(14.0)
                                                                .strong(),
                                                        )
                                                        .fill(if is_in_mix {
                                                            egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 35)
                                                        } else {
                                                            egui::Color32::from_rgba_unmultiplied(40, 40, 45, 160)
                                                        })
                                                        .stroke(if is_in_mix {
                                                            egui::Stroke::new(1.0_f32, color)
                                                        } else {
                                                            egui::Stroke::NONE
                                                        });
                                                        if ui.add_sized([24.0, 26.0], chk_btn).clicked() {
                                                            engine_action.push(EngineAction::ToggleAudioTrackInMix(idx));
                                                        }

                                                        // 2. Track Title Button - tapping solos this track!
                                                        let title_label = format!("{} {}", icon, track.title);
                                                        let title_btn_resp = ui.allocate_ui_with_layout(
                                                            egui::vec2(title_w, 26.0),
                                                            egui::Layout::left_to_right(egui::Align::Center).with_main_align(egui::Align::Min),
                                                            |ui| {
                                                                let title_btn = egui::Button::new(
                                                                    egui::RichText::new(title_label)
                                                                        .color(if is_in_mix { color } else { egui::Color32::from_gray(160) })
                                                                        .size(12.0),
                                                                )
                                                                .truncate()
                                                                .min_size(egui::vec2(title_w, 26.0))
                                                                .fill(if is_in_mix {
                                                                    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 25)
                                                                } else {
                                                                    egui::Color32::from_rgba_unmultiplied(40, 40, 45, 160)
                                                                })
                                                                .stroke(if is_in_mix {
                                                                    egui::Stroke::new(1.0_f32, color)
                                                                } else {
                                                                    egui::Stroke::NONE
                                                                });
                                                                ui.add(title_btn)
                                                            },
                                                        ).inner;
                                                        if title_btn_resp.clicked() {
                                                            engine_action.push(EngineAction::SetAudioMixTracks(vec![(idx, 1.0)]));
                                                        }

                                                        // 3. Track Volume Slider (55 pt rail)
                                                        ui.spacing_mut().slider_width = 55.0;
                                                        let slider = egui::Slider::new(&mut current_vol, 0.0..=1.0).show_value(false);
                                                        let slider_resp = ui.add_enabled(is_in_mix, slider);
                                                        if slider_resp.changed() {
                                                            engine_action.push(EngineAction::SetAudioTrackVolume(idx, current_vol));
                                                        }

                                                        // 4. Volume Percentage Label (dedicated 28 pt column, never jitters or overflows)
                                                        let vol_pct = format!("{:>3.0}%", (current_vol * 100.0).round());
                                                        let vol_lbl = egui::Label::new(
                                                            egui::RichText::new(vol_pct)
                                                                .monospace()
                                                                .size(11.0)
                                                                .color(if is_in_mix { egui::Color32::WHITE } else { egui::Color32::from_gray(120) }),
                                                        );
                                                        ui.add_sized([28.0, 26.0], vol_lbl);

                                                        // 5. Mute button [M]
                                                        let is_muted = current_vol == 0.0;
                                                        let m_btn = egui::Button::new(
                                                            egui::RichText::new("M")
                                                                .strong()
                                                                .color(if !is_in_mix {
                                                                    egui::Color32::from_gray(100)
                                                                } else if is_muted {
                                                                    egui::Color32::RED
                                                                } else {
                                                                    egui::Color32::LIGHT_GRAY
                                                                })
                                                                .size(11.0),
                                                        )
                                                        .fill(if !is_in_mix {
                                                            egui::Color32::from_rgba_unmultiplied(35, 35, 40, 120)
                                                        } else if is_muted {
                                                            egui::Color32::from_rgba_unmultiplied(200, 50, 50, 80)
                                                        } else {
                                                            egui::Color32::from_rgba_unmultiplied(50, 50, 55, 180)
                                                        });
                                                        if ui.add_enabled(is_in_mix, m_btn).clicked() {
                                                            let new_vol = if is_muted { 1.0 } else { 0.0 };
                                                            engine_action.push(EngineAction::SetAudioTrackVolume(idx, new_vol));
                                                        }

                                                        // 6. Solo button [S] (always clickable to solo any track)
                                                        let is_solo = state.active_audio_tracks.len() == 1 && state.active_audio_tracks.contains(&idx);
                                                        let s_btn = egui::Button::new(
                                                            egui::RichText::new("S")
                                                                .strong()
                                                                .color(if is_solo { egui::Color32::BLACK } else { egui::Color32::YELLOW })
                                                                .size(11.0),
                                                        )
                                                        .fill(if is_solo {
                                                            egui::Color32::YELLOW
                                                        } else {
                                                            egui::Color32::from_rgba_unmultiplied(50, 50, 55, 180)
                                                        });
                                                        if ui.add_sized([22.0, 26.0], s_btn).clicked() {
                                                            engine_action.push(EngineAction::SetAudioMixTracks(vec![(idx, 1.0)]));
                                                        }
                                                    });
                                                    ui.add_space(2.0);
                                                }

                                                // Quick Mix Presets Bar (Always Available)
                                                ui.add_space(5.0);
                                                ui.label(egui::RichText::new("Quick Mix Presets:").size(12.0).color(egui::Color32::from_rgb(170, 190, 210)).strong());
                                                ui.add_space(2.0);

                                                let num_tracks = state.audio_tracks.len();
                                                let is_all_on = state.active_audio_tracks.len() == num_tracks && num_tracks > 0;
                                                let is_mix_1_2 = state.active_audio_tracks.len() == 2 && state.active_audio_tracks.contains(&0) && state.active_audio_tracks.contains(&1);

                                                 let total_width = ui.available_width();
                                                 let spacing = 6.0;
                                                 let is_compact = cfg!(target_os = "android") || total_width < 420.0;
                                                 let btn_h = 34.0;

                                                 let render_all_on = |ui: &mut egui::Ui, w: f32, h: f32, engine_action: &mut Vec<EngineAction>| {
                                                     let all_btn = egui::Button::new(
                                                         egui::RichText::new("All On").strong().size(12.0).color(if is_all_on { egui::Color32::from_rgb(0, 240, 170) } else { egui::Color32::LIGHT_GRAY })
                                                     )
                                                     .fill(if is_all_on { egui::Color32::from_rgba_unmultiplied(0, 140, 100, 180) } else { egui::Color32::from_rgba_unmultiplied(50, 60, 70, 220) })
                                                     .stroke(if is_all_on { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 240, 170)) } else { egui::Stroke::NONE });

                                                     if ui.add_sized([w, h], all_btn).clicked() {
                                                         let all_mix: Vec<(usize, f32)> = (0..num_tracks).map(|i| (i, 1.0)).collect();
                                                         engine_action.push(EngineAction::SetAudioMixTracks(all_mix));
                                                     }
                                                 };

                                                 let render_mix_1_2 = |ui: &mut egui::Ui, w: f32, h: f32, engine_action: &mut Vec<EngineAction>| {
                                                     let mix_btn = egui::Button::new(
                                                         egui::RichText::new("Mix 1&2").strong().size(12.0).color(if is_mix_1_2 { egui::Color32::from_rgb(0, 240, 170) } else { egui::Color32::from_rgb(0, 210, 160) })
                                                     )
                                                     .fill(if is_mix_1_2 { egui::Color32::from_rgba_unmultiplied(0, 150, 110, 200) } else { egui::Color32::from_rgba_unmultiplied(0, 80, 70, 180) })
                                                     .stroke(if is_mix_1_2 { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 240, 170)) } else { egui::Stroke::NONE });

                                                     if ui.add_sized([w, h], mix_btn).clicked() {
                                                         engine_action.push(EngineAction::SetAudioMixTracks(vec![(0, 1.0), (1, 1.0)]));
                                                     }
                                                 };

                                                 let render_solo = |ui: &mut egui::Ui, track_idx: usize, w: f32, h: f32, engine_action: &mut Vec<EngineAction>| {
                                                     let is_solo = state.active_audio_tracks.len() == 1 && state.active_audio_tracks.contains(&track_idx);
                                                     let label = if num_tracks == 2 {
                                                         let t_opt = state.audio_tracks.get(track_idx);
                                                         let t_lower = t_opt.map(|t| t.title.to_lowercase()).unwrap_or_default();
                                                         if track_idx == 0 && (t_lower.contains("instrumental") || t_lower.contains("karaoke") || t_lower.contains("music")) {
                                                             "Trk 1 (Music)".to_string()
                                                         } else if track_idx == 1 && (t_lower.contains("vocal") || t_lower.contains("guide") || t_lower.contains("full mix") || t_lower.contains("fullmix")) {
                                                             "Trk 2 (Vocals)".to_string()
                                                         } else {
                                                             format!("Trk {}", track_idx + 1)
                                                         }
                                                     } else {
                                                         format!("Trk {}", track_idx + 1)
                                                     };
                                                     let btn = egui::Button::new(
                                                         egui::RichText::new(label).strong().size(12.0).color(if is_solo { egui::Color32::from_rgb(0, 220, 255) } else { egui::Color32::LIGHT_GRAY })
                                                     )
                                                     .fill(if is_solo { egui::Color32::from_rgba_unmultiplied(0, 100, 150, 180) } else { egui::Color32::from_rgba_unmultiplied(50, 60, 70, 220) })
                                                     .stroke(if is_solo { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 220, 255)) } else { egui::Stroke::NONE });

                                                     if ui.add_sized([w, h], btn).clicked() {
                                                         engine_action.push(EngineAction::SetAudioMixTracks(vec![(track_idx, 1.0)]));
                                                     }
                                                 };

                                                 if num_tracks <= 2 {
                                                     if is_compact {
                                                         // 2 rows on mobile/narrow screen:
                                                         // Row 1: [All On] [Mix 1&2] (50% width each)
                                                         let row_w = ((total_width - spacing) / 2.0).max(60.0);
                                                         ui.horizontal(|ui| {
                                                             ui.spacing_mut().item_spacing.x = spacing;
                                                             render_all_on(ui, row_w, btn_h, engine_action);
                                                             if num_tracks >= 2 {
                                                                 render_mix_1_2(ui, row_w, btn_h, engine_action);
                                                             }
                                                         });
                                                         if num_tracks >= 2 {
                                                             ui.add_space(3.0);
                                                             // Row 2: [Trk 1] [Trk 2] (50% width each)
                                                             ui.horizontal(|ui| {
                                                                 ui.spacing_mut().item_spacing.x = spacing;
                                                                 render_solo(ui, 0, row_w, btn_h, engine_action);
                                                                 render_solo(ui, 1, row_w, btn_h, engine_action);
                                                             });
                                                         }
                                                     } else {
                                                         // Wide screen: 4 buttons in a single row
                                                         let btn_count = if num_tracks == 2 { 4.0 } else { 2.0 };
                                                         let btn_w = ((total_width - spacing * (btn_count - 1.0)) / btn_count).max(50.0);
                                                         ui.horizontal(|ui| {
                                                             ui.spacing_mut().item_spacing.x = spacing;
                                                             render_all_on(ui, btn_w, btn_h, engine_action);
                                                             render_solo(ui, 0, btn_w, btn_h, engine_action);
                                                             if num_tracks >= 2 {
                                                                 render_solo(ui, 1, btn_w, btn_h, engine_action);
                                                                 render_mix_1_2(ui, btn_w, btn_h, engine_action);
                                                             }
                                                         });
                                                     }
                                                 } else {
                                                     // 3+ tracks:
                                                     // Row 1: Mix combos [All On] [Mix 1&2] (each 50% width)
                                                     let row1_w = ((total_width - spacing) / 2.0).max(60.0);
                                                     ui.horizontal(|ui| {
                                                         ui.spacing_mut().item_spacing.x = spacing;
                                                         render_all_on(ui, row1_w, btn_h, engine_action);
                                                         render_mix_1_2(ui, row1_w, btn_h, engine_action);
                                                     });

                                                     ui.add_space(3.0);

                                                     let display_count = num_tracks.min(4);
                                                     if is_compact {
                                                         // Solo tracks in rows of 2 for mobile/narrow
                                                         let chunks: Vec<Vec<usize>> = (0..display_count).collect::<Vec<_>>().chunks(2).map(|c| c.to_vec()).collect();
                                                         for (c_idx, chunk) in chunks.iter().enumerate() {
                                                             let chunk_w = ((total_width - spacing * (chunk.len() - 1) as f32) / chunk.len() as f32).max(45.0);
                                                             ui.horizontal(|ui| {
                                                                 ui.spacing_mut().item_spacing.x = spacing;
                                                                 for &idx in chunk {
                                                                     render_solo(ui, idx, chunk_w, 32.0, engine_action);
                                                                 }
                                                             });
                                                             if c_idx + 1 < chunks.len() {
                                                                 ui.add_space(3.0);
                                                             }
                                                         }
                                                     } else {
                                                         // Wide screen: all solo tracks in a single row
                                                         let row2_w = ((total_width - spacing * (display_count - 1) as f32) / display_count as f32).max(45.0);
                                                         ui.horizontal(|ui| {
                                                             ui.spacing_mut().item_spacing.x = spacing;
                                                             for idx in 0..display_count {
                                                                 render_solo(ui, idx, row2_w, 32.0, engine_action);
                                                             }
                                                         });
                                                     }
                                                 }
                                                ui.add_space(4.0);
                                            }

                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("Device:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                let mut current_device = state.selected_audio_device.clone().unwrap_or_else(|| "Default Device".to_string());
                                                let prev_device = current_device.clone();

                                                egui::ComboBox::from_id_salt("audio_device_combo")
                                                    .selected_text(&current_device)
                                                    .width(ui.available_width().max(80.0))
                                                    .show_ui(ui, |ui| {
                                                        for dev in &state.available_audio_devices {
                                                            ui.selectable_value(&mut current_device, dev.clone(), dev);
                                                        }
                                                    });

                                                if current_device != prev_device {
                                                    engine_action.push(EngineAction::SetAudioDevice(current_device));
                                                }
                                            });

                                            #[cfg(any(target_os = "windows", target_os = "linux"))]
                                            ui.horizontal(|ui| {
                                                let mut passthrough = state.passthrough_enabled;
                                                let label = if cfg!(target_os = "windows") {
                                                    "Bitstream Passthrough (WASAPI Exclusive)"
                                                } else {
                                                    "Bitstream Passthrough (PipeWire)"
                                                };
                                                if ui
                                                    .checkbox(
                                                        &mut passthrough,
                                                        egui::RichText::new(label)
                                                            .size(12.0)
                                                            .color(egui::Color32::from_rgb(160, 180, 200)),
                                                    )
                                                    .on_hover_text("Enable raw IEC 61937 hardware bitstreaming (TrueHD/Atmos, E-AC3, DTS, AC3) to an external HDMI AVR. Unchecked (default) decodes in software using Shared Mode so other apps can play audio simultaneously.")
                                                    .changed()
                                                {
                                                    engine_action.push(EngineAction::SetPassthrough(passthrough));
                                                }
                                            });

                                            if state.hardware_channels > 2
                                                || state.stats.mpegh_bitstream_active
                                                || state.surround_upmix.user_overridden
                                            {
                                                ui.horizontal(|ui| {
                                                    let upmix_enabled = state.surround_upmix.enabled;
                                                    let is_native_bypass = state.file_loaded
                                                        && (state.is_native_atmos
                                                            || (state.hardware_channels > 2
                                                                && !crate::state::SurroundUpmixConfig::can_upmix(
                                                                    state.source_channels,
                                                                    state.hardware_channels,
                                                                )));
                                                    let upmix_active = !is_native_bypass
                                                        && state
                                                            .surround_upmix
                                                            .is_active_for(state.source_channels, state.hardware_channels);
                                                    let layout_tag =
                                                        crate::state::SurroundUpmixConfig::short_tag(
                                                            state.hardware_channels,
                                                        );
                                                    let (btn_text, btn_fill, btn_stroke, txt_col) = if upmix_active {
                                                        (
                                                            format!("🔊 Surround Upmix ({layout_tag}): ON"),
                                                            egui::Color32::from_rgb(165, 85, 12),
                                                            egui::Color32::from_rgb(255, 175, 50),
                                                            egui::Color32::from_rgb(255, 240, 200),
                                                        )
                                                    } else if is_native_bypass {
                                                        (
                                                            "🔊 Surround Upmix: OFF (Native)".to_string(),
                                                            egui::Color32::from_rgb(36, 34, 36),
                                                            egui::Color32::from_rgb(90, 84, 76),
                                                            egui::Color32::from_rgb(175, 170, 162),
                                                        )
                                                    } else if upmix_enabled {
                                                        (
                                                            "🔊 Surround Upmix: ON".to_string(),
                                                            egui::Color32::from_rgb(165, 85, 12),
                                                            egui::Color32::from_rgb(255, 175, 50),
                                                            egui::Color32::from_rgb(255, 240, 200),
                                                        )
                                                    } else {
                                                        (
                                                            if state.hardware_channels > 2 {
                                                                format!("🔊 Surround Upmix ({layout_tag}): OFF")
                                                            } else {
                                                                "🔊 Surround Upmix: OFF".to_string()
                                                            },
                                                            egui::Color32::from_rgb(36, 34, 36),
                                                            egui::Color32::from_rgb(90, 84, 76),
                                                            egui::Color32::from_rgb(200, 192, 182),
                                                        )
                                                    };
                                                    let upmix_btn = egui::Button::new(
                                                        egui::RichText::new(btn_text)
                                                            .size(11.5)
                                                            .strong()
                                                            .color(txt_col),
                                                    )
                                                    .fill(btn_fill)
                                                    .stroke(egui::Stroke::new(1.0_f32, btn_stroke))
                                                    .corner_radius(4.0);
                                                    if ui
                                                        .add(upmix_btn)
                                                        .on_hover_text("Upmix the media file's audio channels to fill all remaining available channels on the surround output device. Use the 'SURROUND EQ' selector in the middle panel title bar to view the live Crossover & 5-band Parametric EQ controls. (Shortcut: 'u' / 'e' / R3)")
                                                        .clicked()
                                                    {
                                                        engine_action.push(EngineAction::ToggleSurroundUpmix(!upmix_enabled));
                                                    }
                                                    #[cfg(any(target_os = "windows", target_os = "linux"))]
                                                    if state.stats.mpegh_bitstream_active
                                                        || state.hardware_channels > 8
                                                        || (state.surround_upmix.mpegh_user_overridden
                                                            && !state.surround_upmix.mpegh_bitstream)
                                                    {
                                                        let mpegh_on = state.surround_upmix.mpegh_bitstream;
                                                        let mpegh_live = state.stats.mpegh_bitstream_active;
                                                        let is_atmos_spatial = state
                                                            .stats
                                                            .mpegh_carrier_info
                                                            .contains("SpatialAudio")
                                                            || state.stats.mpegh_carrier_info.contains("Dolby");
                                                        let (m_text, m_fill, m_stroke, m_txt_col) = if mpegh_live {
                                                            (
                                                                if is_atmos_spatial {
                                                                    "ATMOS 7.1.4"
                                                                } else {
                                                                    "MPEG-H 7.1.4"
                                                                },
                                                                egui::Color32::from_rgb(12, 68, 92),
                                                                egui::Color32::from_rgb(0, 210, 255),
                                                                egui::Color32::from_rgb(200, 248, 255),
                                                            )
                                                        } else if state.hardware_channels > 8 && mpegh_on {
                                                            (
                                                                "7.1.4 Height: ON",
                                                                egui::Color32::from_rgb(28, 52, 68),
                                                                egui::Color32::from_rgb(70, 150, 190),
                                                                egui::Color32::from_rgb(185, 225, 240),
                                                            )
                                                        } else {
                                                            (
                                                                "7.1 Bed Only",
                                                                egui::Color32::from_rgb(36, 34, 36),
                                                                egui::Color32::from_rgb(90, 84, 76),
                                                                egui::Color32::from_rgb(200, 192, 182),
                                                            )
                                                        };
                                                        let mpegh_btn = egui::Button::new(
                                                            egui::RichText::new(m_text)
                                                                .size(11.5)
                                                                .strong()
                                                                .color(m_txt_col),
                                                        )
                                                        .fill(m_fill)
                                                        .stroke(egui::Stroke::new(1.0_f32, m_stroke))
                                                        .corner_radius(4.0);
                                                        let hover_tip = if mpegh_live && !state.stats.mpegh_carrier_info.is_empty() {
                                                            format!(
                                                                "Active 7.1.4 carrier: {}\nClick to toggle 12-channel 7.1.4 height output vs. 8-channel 7.1 LPCM bed.",
                                                                state.stats.mpegh_carrier_info
                                                            )
                                                        } else {
                                                            "Toggle 12-channel 7.1.4 (Bed + Height) output vs. 8-channel 7.1 Bed Only.".to_string()
                                                        };
                                                        if ui
                                                            .add(mpegh_btn)
                                                            .on_hover_text(hover_tip)
                                                            .clicked()
                                                        {
                                                            engine_action.push(EngineAction::ToggleMpeghBitstream(!mpegh_on));
                                                        }
                                                    }
                                                });
                                            }

                                            // 8. Next Song (placed at the bottom, smooth marquee if long)
                                            if state.playlist_index + 1 < state.playlist.len() {
                                                let next_path = std::path::Path::new(&state.playlist[state.playlist_index + 1]);
                                                let next_song = next_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                                                ui.horizontal(|ui| {
                                                    ui.label("Next Song:");
                                                    render_smooth_marquee(ui, &next_song, 14.0, false);
                                                });
                                            }
                                        });

                                    *out_track_info_rect = Some(col.max_rect());
                                }
                            };

                            if is_portrait {
                                let has_video = state.has_video_stream || self.video_state.is_some();
                                let progress_height = 20.0;
                                let available_h = ui.available_height();
                                let content_h = (available_h - progress_height).max(40.0);

                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), content_h),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        match state.mobile_hud_tab {
                                            crate::state::MobileHudTab::Channels => {
                                                render_col_channels(ui, out_meters_rect, out_fire_rect, engine_actions);
                                            }
                                            crate::state::MobileHudTab::Heatmap => {
                                                render_col_heatmap(ui, out_heatmap_rect, engine_actions);
                                            }
                                            crate::state::MobileHudTab::Info => {
                                                render_col_info(ui, out_track_info_rect, engine_actions);
                                            }
                                            crate::state::MobileHudTab::Video => {
                                                if has_video {
                                                    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                                                    *out_video_rect = Some(rect);
                                                } else {
                                                    ui.centered_and_justified(|ui| {
                                                        ui.label(egui::RichText::new("No video stream present").italics());
                                                    });
                                                }
                                            }
                                        }
                                    }
                                );

                                ui.add_space(2.0);
                                render_progress_bar(ui, out_fire_rect, engine_actions);
                            } else {
                                ui.columns(3, |columns| {
                                    render_col_channels(&mut columns[0], out_meters_rect, out_fire_rect, engine_actions);
                                    render_col_heatmap(&mut columns[1], out_heatmap_rect, engine_actions);
                                    render_col_info(&mut columns[2], out_track_info_rect, engine_actions);
                                });
                            }
                        }
                    });
            *out_top_panel_rect = Some(panel_resp.response.rect);
        }

        if state.show_hud && state.video_mode != 3 {
            let total_height = ctx.content_rect().height();
            let total_width = ctx.content_rect().width();
            let drag_h = if is_portrait { 24.0 } else { 8.0 };

            let drag_y = out_top_panel_rect
                .map(|r| r.bottom())
                .unwrap_or(total_height * state.panel_split_ratio);
            let drag_top_y = if is_portrait {
                drag_y - drag_h * 0.5
            } else {
                drag_y - 2.0
            };

            egui::Area::new(egui::Id::unique("split_drag_area"))
                .fixed_pos(egui::pos2(0.0, drag_top_y))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let drag_rect = egui::Rect::from_min_size(
                        ui.min_rect().min,
                        egui::vec2(total_width, drag_h),
                    );
                    let response = ui.allocate_rect(drag_rect, egui::Sense::click_and_drag());
                    if is_portrait {
                        let pill_color = if response.dragged() {
                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 180)
                        } else {
                            egui::Color32::from_rgba_unmultiplied(200, 210, 225, 70)
                        };
                        let pill_rect =
                            egui::Rect::from_center_size(drag_rect.center(), egui::vec2(38.0, 4.0));
                        ui.painter().rect_filled(pill_rect, 2.0, pill_color);
                    }
                    if response.hovered() || response.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if response.dragged()
                        && let Some(mouse_pos) = response.interact_pointer_pos()
                    {
                        let min_h = 120.0f32;
                        let min_r = min_h / total_height;
                        let max_r = (total_height - min_h) / total_height;
                        let new_ratio = mouse_pos.y / total_height;
                        engine_actions.push(EngineAction::SetSplitRatio(
                            new_ratio.clamp(min_r.min(max_r), min_r.max(max_r)),
                        ));
                    }
                });
        }

        let frame = egui::Frame::NONE.fill(egui::Color32::TRANSPARENT);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            *central_rect = rect;

            // Draw OSD text from keyboard/gamepad/touch actions
            if let Some(osd) = &state.osd_text
                && state.osd_timer > 0.0
            {
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::unique("osd_notification"),
                ));
                let alpha = (state.osd_timer.min(0.5) * 2.0 * 255.0) as u8;
                let screen = ui.ctx().viewport_rect();
                let is_portrait = screen.width() < screen.height();

                let max_width = if is_portrait {
                    (screen.width() - 40.0).max(100.0)
                } else {
                    (screen.width() * 0.85).max(200.0)
                };

                let font_size = if is_portrait {
                    if osd.len() > 32 {
                        17.0
                    } else if osd.len() > 18 {
                        21.0
                    } else {
                        26.0
                    }
                } else {
                    if osd.len() > 35 { 24.0 } else { 32.0 }
                };

                let text_color = egui::Color32::from_rgba_premultiplied(255, 255, 255, alpha);
                let galley = painter.layout(
                    osd.clone(),
                    egui::FontId::proportional(font_size),
                    text_color,
                    max_width,
                );

                let top_y = if is_portrait {
                    screen.top() + 65.0
                } else {
                    screen.top() + 40.0
                };
                let galley_pos = egui::pos2(screen.center().x - galley.rect.width() * 0.5, top_y);

                let bg_rect = galley
                    .rect
                    .translate(galley_pos.to_vec2())
                    .expand2(egui::vec2(14.0, 8.0));
                let bg_alpha = (alpha as f32 * 0.65) as u8;
                painter.rect_filled(
                    bg_rect,
                    8.0,
                    egui::Color32::from_rgba_unmultiplied(10, 10, 15, bg_alpha),
                );

                painter.galley(galley_pos, galley, text_color);
            }

            if state.visualizer_mode == 0 && state.show_hud && state.video_mode != 3 {
                let painter = ui.painter();
                let is_portrait =
                    ui.ctx().viewport_rect().width() < ui.ctx().viewport_rect().height();
                let bottom_margin = if is_portrait { 36.0 } else { 20.0 };
                let y = rect.bottom() - bottom_margin;

                let max_freq = state.max_frequency;
                let min_freq = 20.0_f32;
                let x_at = |f: f32| -> f32 { (f / min_freq).ln() / (max_freq / min_freq).ln() };

                let max_freq_key = max_freq.round().max(0.0) as u32;
                if self.stats_cache.max_freq_key != max_freq_key
                    || self.stats_cache.max_freq_label.is_empty()
                {
                    self.stats_cache.max_freq_label = format!("{:.0}kHz", max_freq / 1000.0);
                    self.stats_cache.max_freq_key = max_freq_key;
                }

                let side_margin = if is_portrait { 28.0 } else { 16.0 };
                let usable_width = (rect.width() - 2.0 * side_margin).max(10.0);

                let labels: [(f32, &str, egui::Align2); 5] = [
                    (0.0_f32, "20Hz", egui::Align2::LEFT_BOTTOM),
                    (x_at(100.0), "100Hz", egui::Align2::CENTER_BOTTOM),
                    (x_at(1000.0), "1kHz", egui::Align2::CENTER_BOTTOM),
                    (x_at(5000.0), "5kHz", egui::Align2::CENTER_BOTTOM),
                    (
                        1.0_f32,
                        self.stats_cache.max_freq_label.as_str(),
                        egui::Align2::RIGHT_BOTTOM,
                    ),
                ];

                for &(x_pct, text, align) in &labels {
                    let x = rect.left() + side_margin + usable_width * x_pct;
                    painter.text(
                        egui::pos2(x, y),
                        align,
                        text,
                        egui::FontId::proportional(15.0),
                        egui::Color32::from_gray(230),
                    );
                }
            }
        });

        let pointer_pos = state
            .drag_cursor_pos
            .map(|[x, y]| egui::pos2(x, y))
            .or_else(|| ctx.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos())));
        let is_over_track_info =
            if let (Some(pos), Some(ti_rect)) = (pointer_pos, *out_track_info_rect) {
                ti_rect.contains(pos)
            } else {
                false
            };

        if (!ctx.input(|i| i.raw.hovered_files.is_empty()) || state.hovered_file.is_some())
            && state.file_loaded
        {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("drag_overlay"),
            ));
            if is_over_track_info && state.has_active_playback() {
                if let Some(ti_rect) = *out_track_info_rect {
                    painter.rect(
                        ti_rect.expand(2.0),
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(20, 80, 40, 100),
                        egui::Stroke::new(2.5_f32, egui::Color32::from_rgb(80, 240, 120)),
                        egui::StrokeKind::Outside,
                    );
                    let font_id = egui::FontId::proportional(15.0);
                    let galley = painter.layout(
                        "➕ Add to Playlist".to_string(),
                        font_id,
                        egui::Color32::WHITE,
                        ti_rect.width(),
                    );
                    let badge_rect = egui::Rect::from_center_size(
                        ti_rect.center(),
                        galley.size() + egui::vec2(20.0, 10.0),
                    );
                    painter.rect(
                        badge_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(12, 35, 18, 235),
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 230, 130)),
                        egui::StrokeKind::Outside,
                    );
                    painter.galley(
                        badge_rect.min + egui::vec2(10.0, 5.0),
                        galley,
                        egui::Color32::WHITE,
                    );
                }
            } else {
                let screen_rect = ctx.content_rect();
                let font_id = egui::FontId::proportional(16.0);
                let galley = painter.layout(
                    "▶  Drop to Play Immediately".to_string(),
                    font_id,
                    egui::Color32::WHITE,
                    400.0,
                );
                let banner_rect = egui::Rect::from_center_size(
                    egui::pos2(screen_rect.center().x, screen_rect.top() + 45.0),
                    galley.size() + egui::vec2(28.0, 14.0),
                );
                painter.rect(
                    banner_rect,
                    8.0,
                    egui::Color32::from_rgba_unmultiplied(10, 25, 45, 240),
                    egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(80, 180, 255)),
                    egui::StrokeKind::Outside,
                );
                painter.galley(
                    banner_rect.min + egui::vec2(14.0, 7.0),
                    galley,
                    egui::Color32::WHITE,
                );
            }
        }
    }
}
