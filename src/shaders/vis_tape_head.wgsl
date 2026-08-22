// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> timeline_data: array<f32>;

struct ThemePalette {
    bg: vec3<f32>,
    grid: vec3<f32>,
    center_axis: vec3<f32>,
    wave_l: vec3<f32>,
    wave_r: vec3<f32>,
    wave_fill: vec3<f32>,
    playhead: vec3<f32>,
    spark_core: vec3<f32>,
    spark_halo: vec3<f32>,
    ember_a: vec3<f32>,
    ember_b: vec3<f32>,
};

fn get_palette_by_id(id: u32) -> ThemePalette {
    var p: ThemePalette;
    if id == 0u {
        // Cyberpunk Neon (Cyan & Synth Magenta)
        p.bg = vec3<f32>(0.015, 0.02, 0.035);
        p.grid = vec3<f32>(0.025, 0.045, 0.065);
        p.center_axis = vec3<f32>(0.04, 0.09, 0.12);
        p.wave_l = vec3<f32>(0.15, 0.95, 1.0);
        p.wave_r = vec3<f32>(1.0, 0.20, 0.60);
        p.wave_fill = vec3<f32>(0.08, 0.35, 0.50);
        p.playhead = vec3<f32>(0.35, 0.85, 1.0);
        p.spark_core = vec3<f32>(1.0, 1.0, 1.0);
        p.spark_halo = vec3<f32>(0.3, 0.85, 1.0);
        p.ember_a = vec3<f32>(0.2, 0.9, 1.0);
        p.ember_b = vec3<f32>(1.0, 0.3, 0.7);
    } else if id == 1u {
        // Vintage Amber CRT
        p.bg = vec3<f32>(0.025, 0.018, 0.01);
        p.grid = vec3<f32>(0.05, 0.035, 0.015);
        p.center_axis = vec3<f32>(0.10, 0.07, 0.025);
        p.wave_l = vec3<f32>(1.0, 0.75, 0.15);
        p.wave_r = vec3<f32>(1.0, 0.45, 0.05);
        p.wave_fill = vec3<f32>(0.35, 0.18, 0.03);
        p.playhead = vec3<f32>(1.0, 0.85, 0.3);
        p.spark_core = vec3<f32>(1.0, 0.98, 0.9);
        p.spark_halo = vec3<f32>(1.0, 0.7, 0.15);
        p.ember_a = vec3<f32>(1.0, 0.85, 0.2);
        p.ember_b = vec3<f32>(1.0, 0.45, 0.05);
    } else if id == 2u {
        // Emerald Phosphor / Matrix
        p.bg = vec3<f32>(0.01, 0.025, 0.015);
        p.grid = vec3<f32>(0.02, 0.05, 0.03);
        p.center_axis = vec3<f32>(0.04, 0.11, 0.06);
        p.wave_l = vec3<f32>(0.2, 1.0, 0.45);
        p.wave_r = vec3<f32>(0.1, 0.85, 0.75);
        p.wave_fill = vec3<f32>(0.06, 0.32, 0.15);
        p.playhead = vec3<f32>(0.4, 1.0, 0.6);
        p.spark_core = vec3<f32>(0.9, 1.0, 0.9);
        p.spark_halo = vec3<f32>(0.3, 1.0, 0.5);
        p.ember_a = vec3<f32>(0.4, 1.0, 0.6);
        p.ember_b = vec3<f32>(0.8, 1.0, 0.3);
    } else {
        // Ultraviolet Vaporwave
        p.bg = vec3<f32>(0.025, 0.015, 0.035);
        p.grid = vec3<f32>(0.045, 0.025, 0.065);
        p.center_axis = vec3<f32>(0.09, 0.04, 0.14);
        p.wave_l = vec3<f32>(0.75, 0.35, 1.0);
        p.wave_r = vec3<f32>(1.0, 0.35, 0.55);
        p.wave_fill = vec3<f32>(0.25, 0.08, 0.32);
        p.playhead = vec3<f32>(0.85, 0.55, 1.0);
        p.spark_core = vec3<f32>(1.0, 0.95, 1.0);
        p.spark_halo = vec3<f32>(0.8, 0.4, 1.0);
        p.ember_a = vec3<f32>(0.85, 0.45, 1.0);
        p.ember_b = vec3<f32>(1.0, 0.4, 0.7);
    }
    return p;
}

fn get_active_palette(t: f32) -> ThemePalette {
    // Smooth palette cycle every ~70 seconds
    let cycle = (t * 0.055) % 4.0;
    let idx = u32(floor(cycle));
    let frac = fract(cycle);
    let s_frac = smoothstep(0.0, 1.0, frac);

    let p0 = get_palette_by_id(idx % 4u);
    let p1 = get_palette_by_id((idx + 1u) % 4u);

    var res: ThemePalette;
    res.bg = mix(p0.bg, p1.bg, s_frac);
    res.grid = mix(p0.grid, p1.grid, s_frac);
    res.center_axis = mix(p0.center_axis, p1.center_axis, s_frac);
    res.wave_l = mix(p0.wave_l, p1.wave_l, s_frac);
    res.wave_r = mix(p0.wave_r, p1.wave_r, s_frac);
    res.wave_fill = mix(p0.wave_fill, p1.wave_fill, s_frac);
    res.playhead = mix(p0.playhead, p1.playhead, s_frac);
    res.spark_core = mix(p0.spark_core, p1.spark_core, s_frac);
    res.spark_halo = mix(p0.spark_halo, p1.spark_halo, s_frac);
    res.ember_a = mix(p0.ember_a, p1.ember_a, s_frac);
    res.ember_b = mix(p0.ember_b, p1.ember_b, s_frac);
    return res;
}

fn get_bayer_4x4(px: u32, py: u32) -> f32 {
    let x = px % 4u;
    let y = py % 4u;
    if y == 0u {
        if x == 0u { return 0.0 / 16.0; }
        else if x == 1u { return 8.0 / 16.0; }
        else if x == 2u { return 2.0 / 16.0; }
        else { return 10.0 / 16.0; }
    } else if y == 1u {
        if x == 0u { return 12.0 / 16.0; }
        else if x == 1u { return 4.0 / 16.0; }
        else if x == 2u { return 14.0 / 16.0; }
        else { return 6.0 / 16.0; }
    } else if y == 2u {
        if x == 0u { return 3.0 / 16.0; }
        else if x == 1u { return 11.0 / 16.0; }
        else if x == 2u { return 1.0 / 16.0; }
        else { return 9.0 / 16.0; }
    } else {
        if x == 0u { return 15.0 / 16.0; }
        else if x == 1u { return 7.0 / 16.0; }
        else if x == 2u { return 13.0 / 16.0; }
        else { return 5.0 / 16.0; }
    }
}

struct TimelineSlice {
    min_l: f32,
    max_l: f32,
    min_r: f32,
    max_r: f32,
    rms_l: f32,
    rms_r: f32,
    bass: f32,
    treble: f32,
};

fn get_timeline_slice(slice_idx: u32) -> TimelineSlice {
    let c_idx = min(slice_idx, 599u);
    let offset = c_idx * 8u;
    var s: TimelineSlice;
    s.min_l = timeline_data[offset + 0u];
    s.max_l = timeline_data[offset + 1u];
    s.min_r = timeline_data[offset + 2u];
    s.max_r = timeline_data[offset + 3u];
    s.rms_l = timeline_data[offset + 4u];
    s.rms_r = timeline_data[offset + 5u];
    s.bass = timeline_data[offset + 6u];
    s.treble = timeline_data[offset + 7u];
    return s;
}

fn sample_timeline_smooth(pos: f32) -> TimelineSlice {
    let c_pos = clamp(pos, 0.0, 598.999);
    let idx0 = u32(floor(c_pos));
    let idx1 = idx0 + 1u;
    let f = fract(c_pos);
    let s0 = get_timeline_slice(idx0);
    let s1 = get_timeline_slice(idx1);
    var res: TimelineSlice;
    res.min_l = mix(s0.min_l, s1.min_l, f);
    res.max_l = mix(s0.max_l, s1.max_l, f);
    res.min_r = mix(s0.min_r, s1.min_r, f);
    res.max_r = mix(s0.max_r, s1.max_r, f);
    res.rms_l = mix(s0.rms_l, s1.rms_l, f);
    res.rms_r = mix(s0.rms_r, s1.rms_r, f);
    res.bass = mix(s0.bass, s1.bass, f);
    res.treble = mix(s0.treble, s1.treble, f);
    return res;
}

fn get_slice_pos_for_px(p: f32, playhead_p: f32, max_w: f32) -> f32 {
    if p < playhead_p {
        let prog = p / playhead_p;
        return clamp(20.0 + prog * 80.0, 0.0, 100.0);
    } else {
        let prog = (p - playhead_p) / (max_w - playhead_p);
        return clamp(100.0 + prog * 450.0, 100.0, 599.0);
    }
}

// Pseudo-random hash
fn hash11(p: f32) -> f32 {
    var p3 = fract(p * 0.1031);
    p3 = p3 * (p3 + 33.33);
    p3 = p3 * (p3 + p3);
    return fract(p3);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Virtual 480x270 retro pixel grid
    let grid_size = vec2<f32>(480.0, 270.0);
    let pixel_coord = floor(in.uv * grid_size);
    let px = u32(pixel_coord.x);
    let py = u32(pixel_coord.y);

    let palette = get_active_palette(audio.smooth_time);

    let playhead_x_norm = 0.15;
    let playhead_px = u32(playhead_x_norm * grid_size.x); // ~72 px
    let mid_py = u32(grid_size.y * 0.5);                  // 135 px

    // Extract instantaneous NOW slice (slice 100 = 0.0s offset)
    let now_slice = get_timeline_slice(100u);
    let now_max_linear = max(max(abs(now_slice.min_l), abs(now_slice.max_l)), max(abs(now_slice.min_r), abs(now_slice.max_r)));
    let now_max = pow(clamp(now_max_linear, 0.0, 1.0), 0.55);
    let now_bass = clamp(now_slice.bass * 1.5, 0.0, 1.0);
    let now_treble = clamp(now_slice.treble * 1.5, 0.0, 1.0);

    // 1. Base Background Grid & Graticule
    let is_major_grid = (px % 32u == 0u) || (py % 32u == 0u);
    let is_dot_grid = (px % 8u == 0u) && (py % 8u == 0u);
    let is_center_axis = (py == mid_py);

    var col = palette.bg;
    if is_center_axis {
        col = palette.center_axis;
    } else if is_major_grid {
        col = palette.grid;
    } else if is_dot_grid {
        col = palette.grid * 0.75;
    }

    // 2. Continuous Sub-Pixel Lookahead Mapping
    let is_history = px < playhead_px;
    var history_fade: f32 = 1.0;
    if is_history {
        let hist_prog = f32(px) / f32(playhead_px);
        history_fade = pow(hist_prog, 1.3);
    }

    let pos_curr = get_slice_pos_for_px(f32(px), f32(playhead_px), grid_size.x);
    let pos_next = get_slice_pos_for_px(f32(px + 1u), f32(playhead_px), grid_size.x);

    let slice = sample_timeline_smooth(pos_curr);
    let slice_next = sample_timeline_smooth(pos_next);

    // 3. Dynamic Waveform Amplitudes (Perceptual |v|^0.55 scaling to fill vertical space)
    let amp_scale = 120.0; // Max reach in virtual pixels (120 px out of 135 px half-height)

    let s_max = max(pow(clamp(slice.max_l, 0.0, 1.0), 0.55), pow(clamp(slice.max_r, 0.0, 1.0), 0.55));
    let s_min = max(pow(clamp(-slice.min_l, 0.0, 1.0), 0.55), pow(clamp(-slice.min_r, 0.0, 1.0), 0.55));
    let next_max = max(pow(clamp(slice_next.max_l, 0.0, 1.0), 0.55), pow(clamp(slice_next.max_r, 0.0, 1.0), 0.55));
    let next_min = max(pow(clamp(-slice_next.min_l, 0.0, 1.0), 0.55), pow(clamp(-slice_next.min_r, 0.0, 1.0), 0.55));

    let py_top = u32(clamp(f32(mid_py) - s_max * amp_scale, 0.0, grid_size.y - 1.0));
    let py_bot = u32(clamp(f32(mid_py) + s_min * amp_scale, 0.0, grid_size.y - 1.0));
    let next_py_top = u32(clamp(f32(mid_py) - next_max * amp_scale, 0.0, grid_size.y - 1.0));
    let next_py_bot = u32(clamp(f32(mid_py) + next_min * amp_scale, 0.0, grid_size.y - 1.0));

    // Connect adjacent slices with clean continuous envelope lines
    let min_yt = min(py_top, next_py_top);
    let max_yt = max(py_top, next_py_top);
    let min_yb = min(py_bot, next_py_bot);
    let max_yb = max(py_bot, next_py_bot);

    // 4. Waveform Body Phosphor Fill (Full Waveform Extent)
    let in_body = (py >= min_yt && py <= max_yb);
    if in_body {
        let d_center = abs(f32(py) - f32(mid_py)) / (grid_size.y * 0.5);
        let fill_atten = pow(clamp(1.0 - d_center * 0.65, 0.0, 1.0), 1.5);
        let fill_col = palette.wave_fill * fill_atten * 0.50 * history_fade;
        col = max(col, fill_col);
    }

    // 5. RMS Core Energy (Dense Audio Body)
    let s_rms = max(slice.rms_l, slice.rms_r);
    let s_rms_scaled = pow(clamp(s_rms * 1.5, 0.0, 1.0), 0.55);
    let rms_top = u32(clamp(f32(mid_py) - s_rms_scaled * amp_scale * 0.7, 0.0, grid_size.y - 1.0));
    let rms_bot = u32(clamp(f32(mid_py) + s_rms_scaled * amp_scale * 0.7, 0.0, grid_size.y - 1.0));

    if (py >= rms_top && py <= rms_bot) {
        col = col + palette.wave_l * 0.18 * history_fade;
    }

    // Peak Boundary Laser Outlines
    if (py >= min_yt && py <= max_yt) || (py >= min_yb && py <= max_yb) {
        col = max(col, palette.wave_l * history_fade);
    } else if abs(i32(py) - i32(py_top)) == 1 || abs(i32(py) - i32(py_bot)) == 1 {
        col = col + palette.wave_l * 0.40 * history_fade;
    }

    // 6. The Optical Playhead Bar (Left X = 15%)
    let is_playhead_col = (px == playhead_px);
    let dist_mid_norm = abs(f32(py) - f32(mid_py)) / (grid_size.y * 0.5);
    let now_slit_energy = mix(now_bass, now_treble, dist_mid_norm);

    if is_playhead_col {
        let is_tick = (py % 6u == 0u);
        let base_slit = select(palette.playhead * 0.45, palette.playhead, is_tick);
        let slit_flare = palette.playhead * pow(now_slit_energy, 1.2) * 1.8;
        col = max(col, base_slit + slit_flare);
    } else {
        // Horizontal light bleed from playhead slit
        let dx_playhead = abs(i32(px) - i32(playhead_px));
        let flare_w = i32(3.0 + now_slit_energy * 8.0 + now_bass * 6.0);
        if dx_playhead <= flare_w {
            let att = pow(1.0 - f32(dx_playhead) / f32(flare_w + 1), 2.0);
            let slit_flare = palette.playhead * pow(now_slit_energy, 1.2) * 1.8;
            col = col + slit_flare * att * 0.35;
        }
    }

    // Playhead Top & Bottom Caliper Brackets
    if (py <= 16u || py >= (u32(grid_size.y) - 17u)) {
        let anchor_y = select(u32(grid_size.y) - 12u, 11u, py <= 16u);
        let dy_c = abs(i32(py) - i32(anchor_y));
        let dx_c = abs(i32(px) - i32(playhead_px));
        if (dx_c + dy_c) <= 4 {
            col = palette.spark_core;
        }
    }

    // 7. Cathode Spark / Transient Impact Point (Mirrored Dual Sparks on Top & Bottom)
    let hit_h = u32(round(now_max * amp_scale));
    let hit_py_top = select(mid_py, mid_py - hit_h, mid_py >= hit_h);
    let hit_py_bot = min(u32(grid_size.y - 1.0), mid_py + hit_h);

    let dx_hit = abs(i32(px) - i32(playhead_px));
    let dy_hit_top = abs(i32(py) - i32(hit_py_top));
    let dy_hit_bot = abs(i32(py) - i32(hit_py_bot));
    let dist_hit_top = sqrt(f32(dx_hit * dx_hit + dy_hit_top * dy_hit_top));
    let dist_hit_bot = sqrt(f32(dx_hit * dx_hit + dy_hit_bot * dy_hit_bot));
    let dist_hit = min(dist_hit_top, dist_hit_bot);

    let spark_rad = 6.0 + now_bass * 16.0;
    if dist_hit < spark_rad * 2.0 {
        let norm_d = dist_hit / (spark_rad * 2.0);
        let halo = pow(1.0 - norm_d, 2.2) * (1.0 + now_bass * 1.5);
        col = col + palette.spark_halo * halo * 0.8;
        if norm_d < 0.22 {
            let core = (1.0 - norm_d / 0.22) * 2.5;
            col = col + palette.spark_core * core;
        }
    }

    // Diamond Spark Center (Ignites symmetrically on top & bottom spark nodes)
    let diamond_size = i32(2.0 + now_bass * 3.0);
    if (dx_hit + dy_hit_top) <= diamond_size || (dx_hit + dy_hit_bot) <= diamond_size {
        col = palette.spark_core;
    }

    // Horizontal Laser Streak across both sparks
    let streak_len = i32(16.0 + now_treble * 50.0);
    if (dy_hit_top == 0 || dy_hit_bot == 0) && dx_hit <= streak_len {
        let att = (1.0 - f32(dx_hit) / f32(streak_len)) * (0.35 + now_treble * 0.75);
        col = col + palette.spark_halo * att;
    }

    // Micro Electric Plasma Discharge Arcs
    let arc_seed = floor(audio.smooth_time * 24.0);
    for (var a = 0; a < 4; a = a + 1) {
        let f_a = f32(a);
        let arc_rnd_x = i32(hash11(arc_seed + f_a * 13.1) * 6.0) - 3;
        let arc_rnd_y = i32(hash11(arc_seed + f_a * 29.7) * 8.0) - 4;
        let is_arc_top = (i32(px) == i32(playhead_px) + arc_rnd_x) && (i32(py) == i32(hit_py_top) + arc_rnd_y);
        let is_arc_bot = (i32(px) == i32(playhead_px) + arc_rnd_x) && (i32(py) == i32(hit_py_bot) - arc_rnd_y);
        if is_arc_top || is_arc_bot {
            col = palette.spark_core;
        }
    }

    // Embers / Sparks Spraying Leftward into History from both Top and Bottom
    let ember_time = audio.smooth_time * 8.0;
    let base_ember_cycle = floor(ember_time);
    let ember_frac = fract(ember_time);

    let num_embers = 18u;
    for (var e = 0u; e < num_embers; e = e + 1u) {
        let f_e = f32(e);
        let seed = base_ember_cycle + f_e * 17.31;
        let vx = -(hash11(seed * 1.1) * 35.0 + 8.0 + now_treble * 30.0);
        let vy = (hash11(seed * 2.3) * 2.0 - 1.0) * (12.0 + now_bass * 14.0);
        let life = clamp(1.0 - ember_frac + hash11(seed * 3.7) * 0.3, 0.0, 1.0);
        
        let ember_x = i32(playhead_px) + i32(vx * (1.0 - life));
        let ember_y_top = i32(hit_py_top) + i32(vy * (1.0 - life) + (vy * vy * 0.015));
        let ember_y_bot = i32(hit_py_bot) - i32(vy * (1.0 - life) + (vy * vy * 0.015));
        
        let is_emb_top = (i32(px) == ember_x) && (i32(py) == ember_y_top);
        let is_emb_bot = (i32(px) == ember_x) && (i32(py) == ember_y_bot);

        if is_emb_top || is_emb_bot {
            let heat = life * (0.6 + now_treble * 0.6);
            let ember_col = select(palette.ember_b, palette.ember_a, hash11(seed * 4.9) > 0.5);
            col = max(col, ember_col * heat);
        }
    }

    // 8. Apply CRT Effects & ACES Tonemapping
    var crt_settings = get_default_crt();
    crt_settings.scanline_intensity = 0.12;
    crt_settings.vignette_scale = 1.35;
    crt_settings.vignette_softness = 0.88;
    crt_settings.noise_intensity = 0.018;

    var final_color = apply_crt_effects(col, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);
    final_color = aces_tonemap(final_color);

    return vec4<f32>(final_color, 1.0);
}


