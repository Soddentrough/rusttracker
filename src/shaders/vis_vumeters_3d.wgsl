// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(2) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
};

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) mat: f32,
    @location(4) local_pos: vec3<f32>,
};

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    let mat_id = in.tex_coords.x;
    var pos = in.position;

    let vu_l = clamp(audio.channels[0].x * 1.35, 0.0, 1.15);
    let vu_r = clamp(audio.channels[0].y * 1.35, 0.0, 1.15);

    // Needle Mechanical Rotation around Pivot Base (lx=-2.65, ly=-1.15 and rx=2.65, ry=-1.15)
    if (mat_id >= 3.8 && mat_id <= 4.2) {
        // Left Needle (0.68 rad = left -20dB rest, -0.68 rad = right +3dB red zone)
        let pivot = vec2<f32>(-2.65, -1.15);
        let ang = mix(0.68, -0.68, vu_l);
        let cos_a = cos(ang);
        let sin_a = sin(ang);
        let rel = pos.xy - pivot;
        pos.x = pivot.x + (rel.x * cos_a - rel.y * sin_a);
        pos.y = pivot.y + (rel.x * sin_a + rel.y * cos_a);
    } else if (mat_id >= 4.8 && mat_id <= 5.2) {
        // Right Needle (0.68 rad = left -20dB rest, -0.68 rad = right +3dB red zone)
        let pivot = vec2<f32>(2.65, -1.15);
        let ang = mix(0.68, -0.68, vu_r);
        let cos_a = cos(ang);
        let sin_a = sin(ang);
        let rel = pos.xy - pivot;
        pos.x = pivot.x + (rel.x * cos_a - rel.y * sin_a);
        pos.y = pivot.y + (rel.x * sin_a + rel.y * cos_a);
    }

    out.world_pos = pos;
    out.normal = in.normal;
    out.uv = in.tex_coords;
    out.mat = mat_id;
    out.local_pos = in.position;

    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let mat_id = in.mat;
    let n = normalize(in.normal);
    let v = normalize(vec3<f32>(0.0, 0.0, 7.5) - in.world_pos);

    // Studio ambient & key lighting
    let key_light_dir = normalize(vec3<f32>(0.3, 0.8, 1.0));
    let key_diffuse = max(dot(n, key_light_dir), 0.0);

    var color = vec3<f32>(0.0);

    if (mat_id < 1.5) {
        // =========================================================================
        // 1.0: BRUSHED ALUMINUM RACK FACEPLATE
        // =========================================================================
        let brush = sin(in.world_pos.y * 140.0) * 0.015;
        let base_al = vec3<f32>(0.13, 0.14, 0.16) + brush;
        let spec = pow(max(dot(reflect(-key_light_dir, n), v), 0.0), 16.0) * 0.40;
        let bevel = smoothstep(5.4, 5.5, abs(in.world_pos.x)) * 0.25;
        color = base_al * (0.65 + key_diffuse * 0.35) + vec3<f32>(spec + bevel);

    } else if (mat_id < 2.5) {
        // =========================================================================
        // 2.0: MATTE BLACK RECESSED METER CAVITY BEZELS
        // =========================================================================
        let black_bezel = vec3<f32>(0.02, 0.02, 0.025);
        color = black_bezel * (0.5 + key_diffuse * 0.5);

    } else if (mat_id < 3.5) {
        // =========================================================================
        // 3.0: VINTAGE CREAM PARCHMENT DIAL SCALE (Incandescent Tungsten Backlight)
        // =========================================================================
        let is_left = in.world_pos.x < 0.0;
        let center_x = select(2.65, -2.65, is_left);
        let rel_x = in.world_pos.x - center_x;
        let rel_y = in.world_pos.y - (-1.15); // Pivot origin
        let dial_r = length(vec2<f32>(rel_x, rel_y));
        let dial_ang = atan2(rel_x, rel_y); // -PI to PI

        // Warm vintage parchment cream base
        let parchment = vec3<f32>(0.96, 0.92, 0.80);

        // Incandescent Tungsten Backlight (Glows warmly from top of meter)
        let lamp_dist = length(vec2<f32>(rel_x, in.world_pos.y - 1.25));
        let lamp_glow = 1.0 / (1.0 + lamp_dist * 1.8);
        let vu_val = select(audio.channels[0].y, audio.channels[0].x, is_left);
        let tungsten_col = vec3<f32>(1.0, 0.78, 0.42) * (1.1 + vu_val * 0.35);

        // Printed Logarithmic VU Arc Scale (Black track, red zone > 0dB)
        let arc_dist = abs(dial_r - 2.05);
        let is_arc = smoothstep(0.028, 0.0, arc_dist) * step(abs(dial_ang), 0.65);
        let is_red_zone = dial_ang > 0.12;
        let arc_col = select(vec3<f32>(0.06, 0.06, 0.06), vec3<f32>(0.92, 0.06, 0.06), is_red_zone);

        // Printed Tick Marks along Arc
        let tick_phase = fract((dial_ang + 0.65) * 12.0);
        let is_tick = smoothstep(0.12, 0.0, tick_phase) * smoothstep(0.10, 0.0, abs(dial_r - 2.05)) * step(abs(dial_ang), 0.65);

        // Dial VU branding text logo
        let logo_box = smoothstep(0.45, 0.0, abs(rel_x)) * smoothstep(0.12, 0.0, abs(in.world_pos.y - 0.20));
        let logo_col = vec3<f32>(0.08, 0.08, 0.08) * logo_box * 0.6;

        var dial = parchment * tungsten_col * lamp_glow;
        dial = mix(dial, arc_col, is_arc * 0.88);
        dial = mix(dial, arc_col, is_tick * 0.92);
        dial = dial - logo_col;

        color = dial;

    } else if (mat_id < 5.5) {
        // =========================================================================
        // 4.0 & 5.0: MECHANICAL NEEDLES (Pivoting Pointer Blades)
        // =========================================================================
        let is_tip = in.world_pos.y > 0.40;
        let needle_body = vec3<f32>(0.10, 0.08, 0.08);
        let needle_tip = vec3<f32>(0.95, 0.08, 0.08);
        color = select(needle_body, needle_tip, is_tip);

    } else if (mat_id < 6.5) {
        // =========================================================================
        // 6.0: MACHINED ALUMINUM MASTER GAIN KNOB
        // =========================================================================
        let knob_al = vec3<f32>(0.22, 0.24, 0.26);
        let fluting = sin(atan2(in.world_pos.x, in.world_pos.y + 0.65) * 24.0) * 0.04;
        let spec = pow(max(dot(reflect(-key_light_dir, n), v), 0.0), 32.0) * 0.8;
        color = (knob_al + fluting) * (0.5 + key_diffuse * 0.5) + vec3<f32>(spec);

    } else if (mat_id < 7.5) {
        // =========================================================================
        // 7.0: LED STATUS INDICATORS (Power Amber, Peak Red)
        // =========================================================================
        if (in.world_pos.y < -0.5) {
            // Power LED (Warm Vintage Green / Amber)
            color = vec3<f32>(0.1, 0.95, 0.3) * 2.5;
        } else {
            // Peak Warning LEDs (+3dB Clipping Flash)
            let is_left = in.world_pos.x < 0.0;
            let peak_val = select(audio.channel_peaks[0].y, audio.channel_peaks[0].x, is_left);
            let is_clipping = smoothstep(0.85, 1.0, peak_val);
            color = vec3<f32>(1.0, 0.02, 0.02) * is_clipping * 3.5 + vec3<f32>(0.15, 0.01, 0.01);
        }

    } else {
        // =========================================================================
        // 8.0: BLACK HEX MOUNTING SCREWS
        // =========================================================================
        color = vec3<f32>(0.04, 0.04, 0.05);
    }

    let tonemapped = aces_tonemap(color);
    return vec4<f32>(tonemapped, 1.0);
}
