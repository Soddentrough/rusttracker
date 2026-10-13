// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

fn rotate2d(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

fn hash_f(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }

fn get_spatial_vu(i: u32) -> f32 {
    if audio.num_spatial_channels > 2u {
        let v = audio.spatial_channels[i / 4u][i % 4u];
        return clamp(v, 0.0, 1.0);
    }
    if audio.num_channels > 2u {
        let idx = i % audio.num_channels;
        return clamp(audio.channels[idx / 4u][idx % 4u], 0.0, 1.0);
    }
    let ch_l = audio.channels[0].x;
    let ch_r = select(ch_l, audio.channels[0].y, audio.num_channels >= 2u);
    let ph_l = audio.channel_phases[0].x;
    let ph_r = audio.channel_phases[0].y;
    let bass_e = clamp((audio.spectrum[0].x + audio.spectrum[1].x + audio.spectrum[2].x) / 210.0, 0.0, 1.0);
    let lowmid_e = clamp((audio.spectrum[6].x + audio.spectrum[10].x) / 140.0, 0.0, 1.0);
    let mid_e = clamp((audio.spectrum[20].x + audio.spectrum[32].x) / 130.0, 0.0, 1.0);
    let high_e = clamp((audio.spectrum[64].x + audio.spectrum[96].x) / 120.0, 0.0, 1.0);
    return stereo_upmix_12(i, ch_l, ch_r, ph_l, ph_r, bass_e, lowmid_e, mid_e, high_e);
}

fn project_3d(p3: vec3<f32>, ro: vec3<f32>, cu: vec3<f32>, cv: vec3<f32>, cw: vec3<f32>) -> vec3<f32> {
    let dir = p3 - ro;
    let dist_w = dot(dir, cw);
    if dist_w <= 0.001 { return vec3<f32>(999.0, 999.0, dist_w); }

    let proj_x = dot(dir, cu) / dist_w;
    let proj_y = dot(dir, cv) / dist_w;
    return vec3<f32>(proj_x, -proj_y, dist_w);
}

// INCLUDE: glyph_font

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);

    // Enable all 12 speakers via stereo_upmix_12 when source is stereo/tracker, or true spatial count for 5.1/7.1
    let render_channels = select(min(audio.num_spatial_channels, 12u), 12u, audio.num_spatial_channels <= 2u);
    let time = audio.smooth_time;

    var lfe_vu = 0.0;
    if render_channels > 3u {
        lfe_vu = get_spatial_vu(3u);
    }

    var total_volume = 0.0;
    for (var i = 0u; i < render_channels; i = i + 1u) {
        total_volume += get_spatial_vu(i);
    }
    total_volume = total_volume / max(f32(render_channels), 1.0);

    // --- Wide panoramic camera with LFE micro-shake ---
    let fov_scale = 0.75;
    let p = vec2<f32>(uv.x * aspect, uv.y) * fov_scale;

    let cam_dist = 2.4;
    let cam_height = 0.65;
    let shake_x = sin(time * 7.3) * lfe_vu * 0.012;
    let shake_z = sin(time * 5.1 + 1.3) * lfe_vu * 0.008;
    let ro = vec3<f32>(shake_x, -cam_dist, cam_height + shake_z);
    let cam_target = vec3<f32>(0.0, 0.3, 0.15);

    let cw = normalize(cam_target - ro);
    let cu = normalize(cross(cw, vec3<f32>(0.0, 0.0, 1.0)));
    let cv = cross(cu, cw);

    let rd = normalize(cw + p.x * cu - p.y * cv);

    // Gentle panoramic orbital sway so 3D spatial vectors exhibit parallax depth
    let room_angle = sin(time * 0.18) * 0.12;

    let sky_grad = smoothstep(-1.0, 1.0, uv.y) * 0.008;
    var color = vec3<f32>(0.012 + sky_grad, 0.012 + sky_grad * 0.5, 0.018 + sky_grad * 1.5);

    // --- Raytraced Perspective Floor ---
    var floor_xy = vec2<f32>(0.0, 0.0);
    var floor_t = 0.0;
    if rd.z < 0.0 {
        floor_t = -ro.z / rd.z;
        let hit = ro + floor_t * rd;
        floor_xy = rotate2d(hit.xy, -room_angle);

        let dome_r = length(floor_xy);
        let warp_dir = floor_xy / max(dome_r, 0.01);
        let warp_strength = lfe_vu * 0.08 / max(dome_r, 0.2);
        let warped_xy = floor_xy + warp_dir * warp_strength;

        let cell_size = 0.4;
        let grid_x = smoothstep_r(0.015, 0.0, abs(fract(warped_xy.x / cell_size + 0.5) - 0.5) * cell_size);
        let grid_y = smoothstep_r(0.015, 0.0, abs(fract(warped_xy.y / cell_size + 0.5) - 0.5) * cell_size);
        let grid_lines = max(grid_x, grid_y) * 0.4;

        let grid_mask = smoothstep_r(5.5, 1.5, floor_t);
        color = color + vec3<f32>(0.04, 0.07, 0.1) * grid_lines * grid_mask;

        // --- LFE Pressure Dome ---
        let dome_pulse = smoothstep_r(2.8 + lfe_vu * 1.5, 0.0, dome_r);
        let dome_breathe = 1.0 + sin(time * 6.0) * lfe_vu * 0.15;
        let lfe_color = vec3<f32>(1.0, 0.2, 0.4);
        color = color + lfe_color * dome_pulse * lfe_vu * lfe_vu * dome_breathe * 0.4;

        let ring_phase = time * 3.0;
        let ring1 = smoothstep_r(0.06, 0.0, abs(dome_r - fract(ring_phase) * 3.5)) * lfe_vu;
        let ring2 = smoothstep_r(0.06, 0.0, abs(dome_r - fract(ring_phase + 0.5) * 3.5)) * lfe_vu;
        color = color + lfe_color * (ring1 + ring2) * 0.2 * grid_mask;

        let bass = clamp((audio.spectrum[1].x * 2.0 + audio.spectrum[2].x) / 180.0, 0.0, 1.0);
        color = color + vec3<f32>(0.05, 0.02, 0.08) * bass * smoothstep_r(3.5, 0.0, dome_r);
    }

    var speaker_data = array<vec3<f32>, 12>(
        vec3<f32>(-2.5, 2.0, 0.0),    // 0: L
        vec3<f32>(2.5, 2.0, 0.0),     // 1: R
        vec3<f32>(0.0, 2.5, 0.0),     // 2: C
        vec3<f32>(0.0, 0.5, 0.0),     // 3: LFE
        vec3<f32>(-2.2, -0.5, 0.0),   // 4: Ls
        vec3<f32>(2.2, -0.5, 0.0),    // 5: Rs
        vec3<f32>(-1.2, -1.2, 0.0),   // 6: Lrs
        vec3<f32>(1.2, -1.2, 0.0),    // 7: Rrs
        vec3<f32>(-2.2, 1.5, 0.7),    // 8: Ltf
        vec3<f32>(2.2, 1.5, 0.7),     // 9: Rtf
        vec3<f32>(-1.2, -1.2, 0.7),   // 10: Ltr
        vec3<f32>(1.2, -1.2, 0.7)     // 11: Rtr
    );

    if render_channels <= 6u {
        speaker_data[4] = vec3<f32>(-1.2, -1.2, 0.0);
        speaker_data[5] = vec3<f32>(1.2, -1.2, 0.0);
    }

    // Listener origin node in 3D space for Demoscene Vectorball / Acoustic Ray connections
    let listener_3d = vec3<f32>(0.0, 0.0, 0.08);
    let proj_listener = project_3d(listener_3d, ro, cu, cv, cw);

    for (var i = 0u; i < 12u; i = i + 1u) {
        let room_xy = vec2<f32>(speaker_data[i].x, speaker_data[i].y);
        var height = speaker_data[i].z;
        let world_xy = rotate2d(room_xy, room_angle);

        let is_active = i < render_channels;
        let is_lfe = i == 3u;

        var vu = 0.0;
        if is_active { vu = get_spatial_vu(i); }
        if is_lfe { vu = vu * 0.3; }

        height = height + (vu * 0.08);

        let p3_base = vec3<f32>(world_xy.x, world_xy.y, 0.0);
        let p3_speaker = vec3<f32>(world_xy.x, world_xy.y, height);

        let proj_b = project_3d(p3_base, ro, cu, cv, cw);
        let proj_s = project_3d(p3_speaker, ro, cu, cv, cw);

        let proj_base_2d = proj_b.xy;
        let proj_speaker_2d = proj_s.xy;
        let depth = proj_s.z;

        var base_color = vec3<f32>(0.15, 0.15, 0.18);
        if is_active {
            if i <= 2u { base_color = vec3<f32>(0.0, 0.85, 1.0); }
            else if is_lfe { base_color = vec3<f32>(1.0, 0.2, 0.45); }
            else if i <= 7u { base_color = vec3<f32>(0.15, 0.95, 0.78); }
            else { base_color = vec3<f32>(1.0, 0.72, 0.12); }
        }

        let depth_fade = smoothstep_r(6.0, 1.5, depth);
        let ch_color = base_color * (0.3 + 0.7 * depth_fade);

        var size_factor = 1.0;
        if i <= 2u { size_factor = 5.0; }
        else if is_lfe { size_factor = 0.8; }
        else if i <= 5u { size_factor = 2.0; }
        else if i <= 7u { size_factor = 1.0; }
        else { size_factor = 0.8; }

        let raw_scale = 2.0 / max(depth, 0.1);
        let scale = clamp(raw_scale, 0.3, 1.5);

        // Demoscene Vector Beams: connect active speakers to the central acoustic focus node
        // and draw vertical elevation struts for height speakers
        if is_active && !is_lfe && vu > 0.05 && proj_listener.z > 0.01 {
            let beam_d = sd_line_segment(p, proj_speaker_2d, proj_listener.xy);
            let beam_glow = smoothstep_r(0.0035 * scale, 0.0, beam_d) * vu * 0.28;
            color += ch_color * beam_glow;
        }

        if height > 0.0 {
            // Vertical vector strut from floor anchor to elevated speaker orb
            let strut_d = sd_line_segment(p, proj_base_2d, proj_speaker_2d);
            let strut_line = smoothstep_r(0.0025 * scale, 0.0, strut_d) * (0.25 + vu * 0.5);
            let dist_base = length(p - proj_base_2d);
            let anchor_s = size_factor * 0.5 + 0.5;
            let base_core = smoothstep_r(0.03 * scale * anchor_s, 0.0, dist_base) * 0.4;
            let base_ring = smoothstep_r(0.005 * scale, 0.0, abs(dist_base - 0.04 * scale * anchor_s)) * 0.4;
            color = color + ch_color * (base_core + base_ring + strut_line);
        }

        if rd.z < 0.0 && is_active && !is_lfe {
            let time_offset = time * 8.0;
            let dist_room = length(floor_xy - room_xy);
            let ripple = sin(dist_room * 20.0 - time_offset) * 0.5 + 0.5;
            let ripple_radius = 0.8 + (size_factor - 1.0) * 0.5;
            let ripple_mask = smoothstep_r(ripple_radius, 0.0, dist_room) * vu * 0.4;
            let grid_fade = smoothstep_r(7.0, 2.0, length(floor_xy));
            color = color + ch_color * ripple * ripple_mask * grid_fade;
        }

        // --- Speaker Vectorball Orb ---
        let dist = length(p - proj_speaker_2d);
        let noise_phase = hash_f(f32(i) * 17.3 + (time % 341.0) * 3.0) * 0.004 * vu * size_factor;
        let organic_dist = dist + noise_phase;

        let dot_size = (0.03 + vu * 0.04) * scale * size_factor;
        let active_mult = select(0.4, 1.0, is_active);

        let core_hot = smoothstep_r(dot_size * 0.3, 0.0, dist) * 1.2 * active_mult;
        let core = smoothstep_r(dot_size, dot_size * 0.5, dist);

        let rim_inner = smoothstep(dot_size * 0.6, dot_size * 0.85, dist);
        let rim_outer = smoothstep_r(dot_size * 1.1, dot_size * 0.85, dist);
        let rim = rim_inner * rim_outer * max(vu, 0.15) * 1.5 * active_mult;

        let glow = smoothstep_r(dot_size * 3.0, 0.0, organic_dist) * max(vu, 0.08) * 0.5;

        let shock_radius = dot_size + (0.01 + vu * 0.05) * scale * size_factor;
        let shockwave = smoothstep_r(0.006 * scale, 0.0, abs(dist - shock_radius)) * (vu * vu);

        color = color + vec3<f32>(1.0) * core_hot;
        color = color + ch_color * core * (0.8 + 1.2 * active_mult);
        color = color + ch_color * rim;
        color = color + ch_color * glow;
        if is_active { color = color + ch_color * shockwave; }

        // --- SMPTE Channel Label below speaker (gated by bounding box early-out) ---
        let px_size = 0.004 * scale;
        let label_y = proj_speaker_2d.y + dot_size + px_size * 3.0;
        if abs(p.x - proj_speaker_2d.x) < px_size * 7.0 && abs(p.y - (label_y + px_size * 2.5)) < px_size * 4.0 {
            let smpte = get_smpte_channel_label(i);
            let lbl = array<u32, 3>(smpte.x, smpte.y, smpte.z);
            let lbl_len = smpte.w;
            let char_w = px_size * 4.0;
            let total_w = char_w * f32(lbl_len) - px_size;
            let label_origin = vec2<f32>(proj_speaker_2d.x - total_w * 0.5, label_y);

            var label_alpha = 0.0;
            for (var c = 0u; c < lbl_len; c = c + 1u) {
                let char_origin = label_origin + vec2<f32>(char_w * f32(c), 0.0);
                label_alpha = max(label_alpha, draw_label_char(lbl[c], p, char_origin, px_size));
            }
            let label_color = select(vec3<f32>(0.35, 0.35, 0.4), ch_color, is_active);
            color = color + label_color * label_alpha * 0.85;
        }
    }

    if render_channels > 3u {
        let lfe_color = vec3<f32>(1.0, 0.2, 0.4);
        let edge_x = smoothstep(0.6, 1.0, abs(uv.x));
        let edge_y = smoothstep(0.7, 1.0, abs(uv.y));
        let edge_glow = max(edge_x, edge_y) * lfe_vu * lfe_vu;
        color = color + lfe_color * edge_glow * 0.15;
    }

    let fog_color = vec3<f32>(0.025, 0.02, 0.035);
    let fog_clear = smoothstep(0.0, 0.8, total_volume);
    let radial_dist = length(uv);
    let fog_edge = smoothstep(0.4 + fog_clear * 0.6, 1.8, radial_dist);
    let fog_turb = sin(uv.x * 3.0 + time * 0.5) * cos(uv.y * 2.0 + time * 0.3) * 0.08;
    let fog_density = clamp(fog_edge + fog_turb, 0.0, 1.0) * (1.0 - fog_clear * 0.7);
    color = mix(color, fog_color, fog_density);

    let vignette = smoothstep_r(2.0, 0.5, length(uv));
    color = color * vignette;

    let final_col = aces_tonemap(color);
    return vec4<f32>(final_col, 1.0);
}
