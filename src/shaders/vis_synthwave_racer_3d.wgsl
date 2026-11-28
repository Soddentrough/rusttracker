// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;
@group(1) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
}

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) mat: f32,
    @location(4) local_z: f32,
}

fn hash1(n: f32) -> f32 {
    var u = bitcast<u32>(floor(n) + 0.0) * 1597334677u;
    u = (u ^ (u >> 16u)) * 2654435769u;
    return f32((u ^ (u >> 16u)) & 0x00FFFFFFu) / f32(0x01000000);
}
fn hash2(p: vec2<f32>) -> f32 {
    let q = floor(p) + vec2<f32>(0.0);
    var u = bitcast<u32>(q.x) * 1597334677u ^ bitcast<u32>(q.y) * 3812015801u;
    u = (u ^ (u >> 16u)) * 2654435769u;
    return f32((u ^ (u >> 16u)) & 0x00FFFFFFu) / f32(0x01000000);
}

// Symmetric OutRun highway S-curve ahead and behind the car (straight near car at z = 5.2)
fn road_curve_offset(z: f32, t: f32) -> f32 {
    let onset = smoothstep(8.0, 110.0, abs(z - 5.2));
    return onset * (sin(z * 0.013 - t * 0.75) * 5.5 + cos(z * 0.007 - t * 0.42) * 2.5);
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;

    let bass_pulse = clamp((audio.spectrum[1].x + audio.spectrum[2].x + audio.spectrum[3].x) / 180.0, 0.0, 1.2);
    let mat_id = in.tex_coords.x;

    var pos = in.position;
    var norm = in.normal;
    var uv_out = in.tex_coords;

    // Smooth lane steering sway with dynamic roll banking, pitch squat & spinning wheels
    if (mat_id >= 2.5 && mat_id <= 9.5) {
        // Spin front & rear wheels (7.0 = tire tread, 7.5 = 5-spoke rim) around grounded hub axles
        if (mat_id >= 6.8 && mat_id <= 7.8) {
            let is_front = pos.z > 5.2;
            let y_axle = select(0.335, 0.315, is_front);
            let z_axle = select(4.15, 6.35, is_front);
            let wheel_angle = audio.smooth_time * 24.0;
            let cw = cos(wheel_angle);
            let sw = sin(wheel_angle);

            let wy = pos.y - y_axle;
            let wz = pos.z - z_axle;
            let rot_wy = wy * cw - wz * sw;
            let rot_wz = wy * sw + wz * cw;
            pos.y = y_axle + rot_wy;
            pos.z = z_axle + rot_wz;
            // Export local wheel-plane coords for crisp rotating 5-spoke turbine rim shader
            uv_out = vec2<f32>(rot_wy, rot_wz);

            let ny = norm.y;
            let nz = norm.z;
            norm.y = ny * cw - nz * sw;
            norm.z = ny * sw + nz * cw;
        }

        let sway_phase = audio.smooth_time * 0.45;
        let sway = sin(sway_phase) * 0.35 + sin(sway_phase * 0.5) * 0.15;

        // Roll banking (leaning naturally into turns)
        let roll = -cos(sway_phase) * 0.065;
        let cos_r = cos(roll);
        let sin_r = sin(roll);
        let rel_xy = vec2<f32>(pos.x, pos.y - 0.375);
        let rot_x = rel_xy.x * cos_r - rel_xy.y * sin_r;
        var rot_y = rel_xy.x * sin_r + rel_xy.y * cos_r;

        let n_rx = norm.x * cos_r - norm.y * sin_r;
        var n_ry = norm.x * sin_r + norm.y * cos_r;

        // Pitch squat (front lifts/rear squats on bass acceleration)
        let pitch = -bass_pulse * 0.024;
        let cos_p = cos(pitch);
        let sin_p = sin(pitch);
        let rel_yz = vec2<f32>(rot_y, pos.z - 5.2);
        rot_y = rel_yz.x * cos_p - rel_yz.y * sin_p;
        let rot_z = rel_yz.x * sin_p + rel_yz.y * cos_p;

        let n_rz = n_ry * sin_p + norm.z * cos_p;
        n_ry = n_ry * cos_p - norm.z * sin_p;

        pos.x = sway + rot_x;
        pos.y = 0.375 + rot_y;
        pos.z = 5.2 + rot_z;
        norm = normalize(vec3<f32>(n_rx, n_ry, n_rz));
    }

    // Roadside Palm Trees & Streetlamps Streaming Continuously Across [-140, +328] Loop
    if (mat_id >= 9.8 && mat_id <= 11.8) {
        let anchor_z = in.tex_coords.y;
        let loop_len = 468.0; // 18 props * 26.0 spacing
        let offset_from_anchor = pos.z - anchor_z;

        let travel_z = (audio.smooth_time * 75.0) % 4680.0;
        let phase = fract((anchor_z - travel_z) / loop_len);
        let looped_anchor_z = -140.0 + phase * loop_len;

        pos.z = looped_anchor_z + offset_from_anchor;
        pos.x += road_curve_offset(looped_anchor_z, audio.smooth_time);
    }

    // Apply OutRun highway S-curve to roadbed, curbs, and concrete barriers
    if (mat_id < 2.2 && abs(mat_id - 0.5) > 0.2) {
        pos.x += road_curve_offset(pos.z, audio.smooth_time);
    }

    out.world_pos = pos;
    out.normal = norm;
    out.uv = uv_out;
    out.mat = mat_id;
    out.local_z = pos.z;

    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let bass_pulse = clamp((audio.spectrum[1].x + audio.spectrum[2].x + audio.spectrum[3].x) / 180.0, 0.0, 1.2);
    let mid_pulse = clamp((audio.spectrum[20].x + audio.spectrum[28].x) / 130.0, 0.0, 1.2);
    let treble_pulse = clamp((audio.spectrum[70].x + audio.spectrum[85].x) / 120.0, 0.0, 1.2);
    let mat_id = in.mat;

    // Derive actual 3D camera world position from view_matrix so speculars, Fresnel & fog match every angle
    let vm = camera.view_matrix;
    let cam_pos = -vec3<f32>(
        dot(vm[0].xyz, vm[3].xyz),
        dot(vm[1].xyz, vm[3].xyz),
        dot(vm[2].xyz, vm[3].xyz),
    );
    let view_vec = cam_pos - in.world_pos;
    let cam_dist = length(view_vec);
    let v = view_vec / max(cam_dist, 1e-4);
    let n_raw = normalize(in.normal);
    // Double-sided safety: ensure surface normal always faces toward active camera
    let n = select(-n_raw, n_raw, dot(n_raw, v) >= 0.0);
    let sun_dir = normalize(vec3<f32>(0.0, 0.19, 0.98));

    var color = vec3<f32>(0.0);

    if (mat_id < 0.2) {
        let curve_x = road_curve_offset(in.world_pos.z, audio.smooth_time);
        let road_x = in.world_pos.x - curve_x;
        let dx = abs(road_x);
        // =========================================================================
        // 0.0: WET SPECULAR ASPHALT HIGHWAY (Forward Motion + Dynamic Lights)
        // =========================================================================
        let travel_z = (audio.smooth_time * 75.0) % 4680.0;
        let speed_z = in.world_pos.z + travel_z;

        let grain = hash2(vec2<f32>(floor(road_x * 20.0), floor(speed_z * 20.0))) * 0.015;
        var asphalt = vec3<f32>(0.012, 0.010, 0.020) + vec3<f32>(grain);

        let is_center = smoothstep_r(0.24, 0.0, dx);
        let is_center_dash = smoothstep(0.45, 0.85, sin(speed_z * 0.65));
        let yellow_line = vec3<f32>(1.0, 0.75, 0.05) * is_center * is_center_dash * (1.8 + treble_pulse * 0.8);

        let is_lane_l = smoothstep_r(0.14, 0.0, abs(dx - 4.5));
        let is_lane_dash = smoothstep(0.45, 0.85, sin(speed_z * 0.65));
        let white_line = vec3<f32>(0.95, 0.95, 1.0) * is_lane_l * is_lane_dash * 1.2;

        // View-dependent specular sunset mirror sheen along wet asphalt
        let refl_horiz = normalize(vec2<f32>(-v.x, -v.z));
        let view_sun_align = pow(max(refl_horiz.y, 0.0), 6.0);
        let sun_refl_x = smoothstep_r(7.2, 0.0, abs(road_x * 0.72));
        let sun_refl_z = smoothstep(14.0, 210.0, in.world_pos.z);
        let refl_tint = mix(vec3<f32>(0.82, 0.12, 0.40), vec3<f32>(1.0, 0.45, 0.25), sun_refl_z);
        let sun_refl_c = refl_tint * sun_refl_x * max(sun_refl_z * 0.42, view_sun_align * 0.28) * (0.65 + bass_pulse * 0.35);

        let sway_phase = audio.smooth_time * 0.45;
        let car_sway = sin(sway_phase) * 0.35 + sin(sway_phase * 0.5) * 0.15;
        let rel_car_x = in.world_pos.x - car_sway;
        let rel_car_z = in.world_pos.z - 5.2;

        // Dynamic Neon Chassis Underglow & Tire Contact Grounding Pool
        let underglow_mask = smoothstep_r(1.85, 0.25, length(vec2<f32>(rel_car_x * 1.25, rel_car_z * 0.45)));
        let underglow_c = mix(vec3<f32>(0.0, 0.90, 1.0), vec3<f32>(1.0, 0.04, 0.62), clamp(bass_pulse * 0.85, 0.0, 1.0)) * (1.15 + bass_pulse * 0.85);
        let chassis_shadow = smoothstep_r(1.02, 0.72, abs(rel_car_x)) * smoothstep_r(2.00, 1.40, abs(rel_car_z));
        let underglow = underglow_c * underglow_mask * (1.0 - chassis_shadow * 0.50);

        // Smoothly faded nitrous exhaust glow & twin red taillight trails (no harsh rectangular cutout)
        let rear_dist = max(-rel_car_z - 1.5, 0.0);
        let rear_start_fade = smoothstep(-1.4, -2.5, rel_car_z);
        let nitrous_pool = vec3<f32>(0.0, 0.88, 1.0)
            * exp(-rear_dist * 0.42)
            * smoothstep_r(0.85, 0.0, abs(rel_car_x))
            * rear_start_fade
            * (0.60 + bass_pulse * 0.75);

        let lamp_twin_x = smoothstep_r(0.52, 0.0, abs(abs(rel_car_x) - 0.56));
        let taillight_streak = vec3<f32>(0.95, 0.02, 0.04)
            * exp(-rear_dist * 0.14)
            * lamp_twin_x
            * rear_start_fade
            * (0.55 + bass_pulse * 0.35);

        var headlight_cone = vec3<f32>(0.0);
        if (rel_car_z > 1.7 && rel_car_z < 55.0) {
            let fwd_z = rel_car_z - 1.7;
            let beam_w = 0.65 + fwd_z * 0.08;
            let twin_hl = max(
                smoothstep_r(beam_w, 0.0, abs(rel_car_x - 0.56)),
                smoothstep_r(beam_w, 0.0, abs(rel_car_x + 0.56)),
            );
            let beam_f = twin_hl * smoothstep(0.0, 1.8, fwd_z) * smoothstep_r(55.0, 2.0, fwd_z);
            headlight_cone = vec3<f32>(1.0, 0.94, 0.78) * beam_f * 0.26;
        }

        // Euclidean fract() streetlamp pool so behind-car z < 0 renders seamlessly too
        let lamp_period = 26.0;
        let lamp_rel_z = (fract((in.world_pos.z + travel_z + 13.0) / lamp_period) - 0.5) * lamp_period;
        let lamp_dist = length(vec2<f32>(road_x - 7.5, lamp_rel_z));
        let lamp_spot = smoothstep_r(6.5, 0.0, lamp_dist) * vec3<f32>(1.0, 0.92, 0.75) * (0.42 + bass_pulse * 0.25);

        color = asphalt + yellow_line + white_line + sun_refl_c + underglow + nitrous_pool + taillight_streak + headlight_cone + lamp_spot;
    } else if (mat_id < 0.8) {
        // =========================================================================
        // 0.5: 360° SYNTHWAVE OCEAN EXPANSE (Symmetric Parallax & Sun Reflections)
        // =========================================================================
        let dist_xz = length(in.world_pos.xz - vec2<f32>(0.0, 5.2));
        let ground_dark = vec3<f32>(0.012, 0.004, 0.024);
        let ground_horizon = vec3<f32>(0.16, 0.025, 0.17);
        let ground_base = mix(ground_dark, ground_horizon, smoothstep(14.0, 340.0, dist_xz));

        let water_z = in.world_pos.z + audio.smooth_time * 15.0;
        let wave1 = sin(in.world_pos.x * 0.08 + audio.smooth_time * 1.0) * cos(water_z * 0.06);
        let wave2 = sin(in.world_pos.x * 0.20 - water_z * 0.12 + audio.smooth_time * 1.6);
        let wave_h = wave1 * 0.6 + wave2 * 0.4;

        let refl_falloff = smoothstep_r(120.0, 9.0, abs(in.world_pos.x));
        let forward_sheen = smoothstep(-40.0, 220.0, in.world_pos.z);
        let sunset_spec = mix(vec3<f32>(0.72, 0.06, 0.42), vec3<f32>(1.0, 0.65, 0.15), forward_sheen);
        let ocean_sheen = sunset_spec * (0.10 + 0.34 * smoothstep(-0.2, 0.8, wave_h)) * refl_falloff * (0.75 + bass_pulse * 0.65);

        let crest_fade = smoothstep_r(250.0, 15.0, dist_xz);
        let crest = smoothstep(0.60, 0.95, wave_h) * vec3<f32>(0.98, 0.02, 0.52) * 0.62 * crest_fade;

        color = ground_base + ocean_sheen + crest;
    } else if (mat_id < 1.5) {
        // 1.0: Elevated Bevel Curbs (Euclidean Alternating Magenta / Cyan Rumble Strips)
        let travel_z = (audio.smooth_time * 75.0) % 4680.0;
        let speed_z = in.world_pos.z + travel_z;
        let curb_seg = fract(speed_z * 0.225) < 0.5;
        let c1 = vec3<f32>(0.98, 0.02, 0.52) * (1.5 + bass_pulse * 0.9);
        let c2 = vec3<f32>(0.0, 0.90, 0.98) * (1.4 + treble_pulse * 0.6);
        color = select(c1, c2, curb_seg);
    } else if (mat_id < 2.5) {
        // 2.0: Concrete Barriers with Neon Edge Strip
        let concrete = vec3<f32>(0.08, 0.06, 0.10);
        let neon_strip = smoothstep_r(0.18, 0.0, abs(in.world_pos.y - 0.70)) * vec3<f32>(0.0, 0.92, 1.0) * (1.4 + mid_pulse * 0.6);
        color = concrete + neon_strip;
    } else if (mat_id < 3.5) {
        // =========================================================================
        // 3.0: SUPERCAR CORSA RED CLEARCOAT GLOSS PAINT (Rich Deep Red + Crisp Coat)
        // =========================================================================
        let key_diff = max(dot(n, sun_dir), 0.0);
        let sky_fill = max(dot(n, normalize(vec3<f32>(-0.45, 0.72, -0.52))), 0.0);
        let rear_fill = max(dot(n, normalize(vec3<f32>(0.35, 0.35, -0.87))), 0.0);
        let up_hemi = n.y * 0.5 + 0.5;

        let h_sun = normalize(sun_dir + v);
        let h_sky = normalize(normalize(vec3<f32>(0.0, 0.85, -0.35)) + v);
        let spec_clear = pow(max(dot(n, h_sun), 0.0), 84.0) * 0.95 + pow(max(dot(n, h_sky), 0.0), 36.0) * 0.30;
        let fresnel = pow(1.0 - max(dot(n, v), 0.0), 3.4);

        let body_base = vec3<f32>(0.78, 0.014, 0.012);
        let diffuse_term = body_base * (0.22 + up_hemi * 0.22 + key_diff * 0.42 + sky_fill * 0.24 + rear_fill * 0.22);
        let spec_highlight = vec3<f32>(1.0, 0.86, 0.82) * spec_clear;
        let rim_glow = vec3<f32>(1.0, 0.14, 0.45) * fresnel * (0.28 + bass_pulse * 0.22);
        let under_bounce = smoothstep_r(0.38, 0.12, in.world_pos.y) * mix(vec3<f32>(0.0, 0.85, 1.0), vec3<f32>(1.0, 0.08, 0.62), bass_pulse * 0.75) * 0.12;
        color = diffuse_term + spec_highlight + rim_glow + under_bounce;
    } else if (mat_id < 4.5) {
        // 4.0: Cockpit Canopy Glass with Glossy Sunset Reflection & Vector Green HUD
        let fresnel = pow(1.0 - max(dot(n, v), 0.0), 2.5);
        let sky_refl = mix(vec3<f32>(0.18, 0.04, 0.26), vec3<f32>(1.0, 0.42, 0.28), fresnel) * (0.28 + fresnel * 0.65);
        let glass_base = vec3<f32>(0.018, 0.025, 0.045) + sky_refl;
        let hud_lines = smoothstep(0.85, 0.98, sin(in.world_pos.y * 38.0 - audio.smooth_time * 4.0)) * vec3<f32>(0.0, 1.0, 0.48) * (1.25 + treble_pulse * 0.75);
        color = glass_base + hud_lines;
    } else if (mat_id < 5.5) {
        // =========================================================================
        // 5.0: TWIN FRONT HEADLIGHTS (Warm Xenon Glow)
        // =========================================================================
        let pulse = 2.4 + bass_pulse * 0.9;
        color = vec3<f32>(1.0, 0.95, 0.82) * pulse;
    } else if (mat_id < 6.5) {
        // =========================================================================
        // 6.0: QUAD CIRCULAR GLOWING RED TAILLIGHTS
        // =========================================================================
        let pulse = 1.9 + bass_pulse * 1.4;
        color = vec3<f32>(1.0, 0.02, 0.02) * pulse;
    } else if (mat_id < 7.2) {
        // 7.0: Dark Rubber Tire Treads with rim-facing sheen
        let tread_sheen = max(dot(n, normalize(vec3<f32>(0.2, 0.7, 0.3))), 0.0) * 0.032;
        color = vec3<f32>(0.015, 0.015, 0.018) + vec3<f32>(tread_sheen);
    } else if (mat_id < 7.8) {
        // 7.5: Spinning 5-Spoke Turbine Wheel Rim Stars + Crisp Machined Outer Lip
        let r_len = length(in.uv);
        let ang = atan2(in.uv.x, in.uv.y);
        let spoke_wave = smoothstep(0.42, 0.94, cos(ang * 5.0));
        let outer_lip = smoothstep(0.215, 0.248, r_len) * smoothstep_r(0.275, 0.252, r_len);
        let hub_cap = smoothstep_r(0.068, 0.032, r_len);
        let rim_mask = max(max(spoke_wave, outer_lip), hub_cap);
        let rim_spec = max(dot(n, sun_dir), 0.0);
        let rim_col = mix(vec3<f32>(0.0, 0.92, 1.0), vec3<f32>(1.0, 0.12, 0.72), rim_spec * 0.55);
        let metal_dark = vec3<f32>(0.04, 0.04, 0.06);
        color = mix(metal_dark, rim_col * (1.45 + treble_pulse * 0.95 + bass_pulse * 0.40), rim_mask);
    } else if (mat_id < 8.5) {
        // 8.0: Fastback Engine Deck Louvers (upper rear deck) + Satin Carbon Aero Trim Elsewhere
        let is_rear_deck = n.y > 0.35 && in.world_pos.z < 5.05 && in.world_pos.y > 0.50;
        let louver = select(0.0, step(0.45, fract(in.world_pos.z * 18.0)), is_rear_deck);
        let base_c = vec3<f32>(0.026, 0.026, 0.034);
        let slot_c = vec3<f32>(0.008, 0.008, 0.012);
        let spec_c = pow(max(dot(n, normalize(sun_dir + v)), 0.0), 22.0) * 0.14;
        color = mix(base_c, slot_c, louver) + vec3<f32>(spec_c);
    } else if (mat_id < 9.5) {
        // =========================================================================
        // 9.0: DUAL NITROUS EXHAUST TIPS & CYAN PLASMA FLAME BLAST
        // =========================================================================
        let flame = sin(audio.smooth_time * 65.0) * 0.25 + 0.75;
        let crackle = hash1(fract(audio.smooth_time * 120.0) + in.world_pos.y) * 0.3;
        let plasma_col = mix(vec3<f32>(0.0, 0.95, 1.0), vec3<f32>(1.0, 0.25, 0.85), clamp(bass_pulse - 0.5, 0.0, 0.6));
        color = plasma_col * (2.0 + bass_pulse * 3.0 + crackle) * flame;
    } else if (mat_id < 10.8) {
        // 10.0 / 10.5: Roadside Palm Trees
        let is_crown = mat_id > 10.2;
        if (is_crown) {
            color = vec3<f32>(0.0, 1.0, 0.52) * (1.4 + bass_pulse * 1.2);
        } else {
            let ring_glow = smoothstep(0.85, 0.98, sin(in.world_pos.y * 10.0)) * vec3<f32>(0.95, 0.05, 0.55) * (0.3 + bass_pulse * 0.4);
            color = vec3<f32>(0.14, 0.05, 0.24) + ring_glow;
        }
    } else if (mat_id < 11.8) {
        // 11.0 / 11.5: Cobra-Head Highway Streetlamps (.OBJ Mesh)
        let is_lamp_glow = mat_id > 11.2;
        if (is_lamp_glow) {
            color = vec3<f32>(0.99, 0.95, 0.85) * (3.2 + bass_pulse * 1.8);
        } else {
            color = vec3<f32>(0.15, 0.08, 0.22);
        }
    } else if (mat_id < 13.0) {
        // 12.0 / 12.5: Distant Horizon Skyscrapers with Live Equalizer Windows
        let is_spire = mat_id > 12.2;
        if (is_spire) {
            let blink = step(0.55, sin(audio.smooth_time * 3.0 + hash1(floor(in.world_pos.x * 0.1)) * 6.0));
            color = vec3<f32>(1.0, 0.15, 0.08) * blink * (1.8 + treble_pulse * 1.2);
        } else {
            let win_u = fract(in.world_pos.x * 0.18);
            let win_v = fract(in.world_pos.y * 0.12);
            let bldg_cell = vec2<f32>(floor(in.world_pos.x * 0.18), floor(in.world_pos.y * 0.12));
            let rnd_win = hash2(bldg_cell);

            let win_shape = smoothstep(0.18, 0.28, win_u) * smoothstep_r(0.82, 0.72, win_u)
                          * smoothstep(0.18, 0.30, win_v) * smoothstep_r(0.82, 0.70, win_v);

            let eq_bin = u32(clamp(abs(bldg_cell.x) * 2.5, 0.0, 95.0));
            let eq_amp = clamp(audio.spectrum[eq_bin / 4u][eq_bin % 4u] / 72.0, 0.0, 1.0);
            let norm_bldg_y = clamp((in.world_pos.y + 2.0) / 42.0, 0.0, 1.0);
            let is_eq_active = step(norm_bldg_y, eq_amp * 0.92);

            let is_lit = max(step(0.45, rnd_win), is_eq_active);

            let col_warm = vec3<f32>(1.00, 0.86, 0.55);
            let col_cyan = vec3<f32>(0.40, 0.88, 0.96);
            let col_rose = vec3<f32>(0.96, 0.50, 0.72);

            var win_col = mix(col_warm, col_cyan, step(0.65, rnd_win));
            win_col = mix(win_col, col_rose, step(0.85, rnd_win));
            let eq_col = mix(vec3<f32>(0.0, 0.92, 1.0), vec3<f32>(1.0, 0.05, 0.62), norm_bldg_y);
            win_col = mix(win_col, eq_col * 1.65, is_eq_active * 0.85);

            let gentle_glow = 0.55 + 0.15 * sin(audio.smooth_time * 1.2 + rnd_win * 10.0) + is_eq_active * 0.65;
            let bldg_base = vec3<f32>(0.020, 0.010, 0.035);

            color = bldg_base + win_col * win_shape * is_lit * gentle_glow;
        }
    } else {
        color = vec3<f32>(0.1, 0.1, 0.1);
    }

    // 360° Radial Horizon Fog (Seamless fade ahead and behind across all camera viewpoints)
    let radial_dist = length(in.world_pos.xz - vec2<f32>(0.0, 5.2));
    if (radial_dist > 95.0) {
        let fog_f = smoothstep(95.0, 385.0, radial_dist);
        let dir_xz = normalize(in.world_pos.xz - vec2<f32>(0.0, 5.2));
        let sun_align = pow(max(dir_xz.y, 0.0), 3.0);
        let fog_c = mix(vec3<f32>(0.20, 0.025, 0.28), vec3<f32>(0.76, 0.20, 0.30), sun_align * 0.75);
        color = mix(color, fog_c, fog_f);
    }

    let tonemapped = aces_tonemap(color);
    return vec4<f32>(tonemapped, 1.0);
}
