// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> timeline_data: array<vec4<f32>>;

const PAL_BG: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(0.015, 0.02, 0.035),
    vec3<f32>(0.025, 0.018, 0.01),
    vec3<f32>(0.01, 0.025, 0.015),
    vec3<f32>(0.025, 0.015, 0.035)
);
const PAL_GRID: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(0.025, 0.045, 0.065),
    vec3<f32>(0.05, 0.035, 0.015),
    vec3<f32>(0.02, 0.05, 0.03),
    vec3<f32>(0.045, 0.025, 0.065)
);
const PAL_CENTER_AXIS: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(0.04, 0.09, 0.12),
    vec3<f32>(0.10, 0.07, 0.025),
    vec3<f32>(0.04, 0.11, 0.06),
    vec3<f32>(0.09, 0.04, 0.14)
);
const PAL_WAVE_L: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(0.15, 0.95, 1.0),
    vec3<f32>(1.0, 0.78, 0.18),
    vec3<f32>(0.2, 1.0, 0.45),
    vec3<f32>(0.75, 0.35, 1.0)
);
const PAL_WAVE_R: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(1.0, 0.22, 0.65),
    vec3<f32>(1.0, 0.42, 0.08),
    vec3<f32>(0.12, 0.85, 0.82),
    vec3<f32>(1.0, 0.35, 0.55)
);
const PAL_WAVE_FILL: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(0.08, 0.35, 0.50),
    vec3<f32>(0.35, 0.18, 0.03),
    vec3<f32>(0.06, 0.32, 0.15),
    vec3<f32>(0.25, 0.08, 0.32)
);
const PAL_PLAYHEAD: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(0.35, 0.85, 1.0),
    vec3<f32>(1.0, 0.85, 0.3),
    vec3<f32>(0.4, 1.0, 0.6),
    vec3<f32>(0.85, 0.55, 1.0)
);
const PAL_SPARK_CORE: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(1.0, 1.0, 1.0),
    vec3<f32>(1.0, 0.98, 0.9),
    vec3<f32>(0.9, 1.0, 0.9),
    vec3<f32>(1.0, 0.95, 1.0)
);
const PAL_SPARK_HALO: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(0.3, 0.85, 1.0),
    vec3<f32>(1.0, 0.7, 0.15),
    vec3<f32>(0.3, 1.0, 0.5),
    vec3<f32>(0.8, 0.4, 1.0)
);
const PAL_EMBER_A: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(0.2, 0.9, 1.0),
    vec3<f32>(1.0, 0.85, 0.2),
    vec3<f32>(0.4, 1.0, 0.6),
    vec3<f32>(0.85, 0.45, 1.0)
);
const PAL_EMBER_B: array<vec3<f32>, 4> = array<vec3<f32>, 4>(
    vec3<f32>(1.0, 0.3, 0.7),
    vec3<f32>(1.0, 0.45, 0.05),
    vec3<f32>(0.8, 1.0, 0.3),
    vec3<f32>(1.0, 0.4, 0.7)
);

fn fast_pow055(x: f32) -> f32 {
    let cx = clamp(x, 0.0, 1.0);
    return mix(sqrt(cx), cx, 0.12);
}

fn sample_timeline_peaks(pos: f32) -> vec2<f32> {
    let o0 = min(u32(max(pos + 0.5, 0.0)), 599u) * 2u;
    let p = timeline_data[o0];
    return vec2<f32>(max(p.y, -p.x), max(p.w, -p.z));
}

fn sample_timeline_rms(pos: f32) -> vec2<f32> {
    let o0 = min(u32(max(pos + 0.5, 0.0)), 599u) * 2u + 1u;
    return timeline_data[o0].xy;
}

fn hash11_tape(p: f32) -> f32 {
    var p1 = fract(p * 0.1031);
    p1 *= p1 + 33.33;
    p1 *= p1 + p1;
    return fract(p1);
}

fn get_slice_pos_for_px(p: f32, playhead_p: f32, max_w: f32) -> f32 {
    if p < playhead_p {
        let prog = p / max(playhead_p, 1.0);
        return clamp(20.0 + prog * 80.0, 0.0, 100.0);
    } else {
        let prog = (p - playhead_p) / max(max_w - playhead_p, 1.0);
        return clamp(100.0 + prog * 498.0, 100.0, 599.0);
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Aspect-ratio-adaptive square retro pixel grid
    let aspect = max(audio.aspect_ratio, 0.1);
    let v_height = 270.0;
    let v_width = max(floor(v_height * aspect), 1.0);
    let grid_size = vec2<f32>(v_width, v_height);
    let pixel_coord = floor(in.uv * grid_size);
    let px = u32(pixel_coord.x);
    let py = u32(pixel_coord.y);

    let cycle = (audio.smooth_time * 0.055) % 4.0;
    let p_i0 = u32(floor(cycle)) & 3u;
    let p_i1 = (p_i0 + 1u) & 3u;
    let p_f = smoothstep(0.0, 1.0, fract(cycle));

    let playhead_x_norm = 0.15;
    let playhead_px = u32(playhead_x_norm * grid_size.x);
    let mid_py = u32(grid_size.y * 0.5);

    // 1. Base Background Grid, Graticule & Scrolling Studer/Nagra Tape Timecode Sprockets
    let is_major_grid = (px % 32u == 0u) || (py % 32u == 0u);
    let is_dot_grid = (px % 8u == 0u) && (py % 8u == 0u);
    let is_center_axis = (py == mid_py);

    var col: vec3<f32>;
    if is_center_axis {
        col = mix(PAL_CENTER_AXIS[p_i0], PAL_CENTER_AXIS[p_i1], p_f) * 1.35;
    } else if is_major_grid {
        col = mix(PAL_GRID[p_i0], PAL_GRID[p_i1], p_f);
    } else if is_dot_grid {
        col = mix(PAL_GRID[p_i0], PAL_GRID[p_i1], p_f) * 0.75;
    } else {
        col = mix(PAL_BG[p_i0], PAL_BG[p_i1], p_f);
    }

    let wave_l_col = mix(PAL_WAVE_L[p_i0], PAL_WAVE_L[p_i1], p_f);
    let wave_r_col = mix(PAL_WAVE_R[p_i0], PAL_WAVE_R[p_i1], p_f);

    // Scrolling tape transport calibration ticks along top and bottom rails
    let tape_scroll_px = u32(floor(audio.smooth_time * 82.0)) + px;
    if py >= 3u && py <= 6u {
        if (tape_scroll_px % 16u == 0u) || (tape_scroll_px % 4u == 0u && py == 4u) {
            col = mix(col, wave_l_col * 0.55, 0.7);
        }
    } else if py >= u32(grid_size.y) - 7u && py <= u32(grid_size.y) - 4u {
        if (tape_scroll_px % 16u == 0u) || (tape_scroll_px % 4u == 0u && py == u32(grid_size.y) - 5u) {
            col = mix(col, wave_r_col * 0.55, 0.7);
        }
    }

    // 2. Continuous Sub-Pixel Lookahead Mapping
    let is_history = px < playhead_px;
    var history_fade: f32 = 1.0;
    if is_history {
        let hist_prog = f32(px) / f32(max(playhead_px, 1u));
        history_fade = hist_prog * mix(1.0, hist_prog, 0.35);
    }

    let pos_curr = get_slice_pos_for_px(f32(px), f32(playhead_px), grid_size.x);
    let pos_next = get_slice_pos_for_px(f32(px + 1u), f32(playhead_px), grid_size.x);

    let curr_peaks = sample_timeline_peaks(pos_curr);
    let next_peaks = sample_timeline_peaks(pos_next);

    // 3. Dual-Track Stereo Envelopes:
    // Upper track (above mid_py) driven by Left channel (`wave_l`),
    // Lower track (below mid_py) driven by Right channel (`wave_r`),
    // with a subtle secondary ghost trace of the opposite channel for stereo phase comparison!
    let amp_scale = 120.0;

    let l_curr = fast_pow055(curr_peaks.x);
    let r_curr = fast_pow055(curr_peaks.y);
    let l_next = fast_pow055(next_peaks.x);
    let r_next = fast_pow055(next_peaks.y);

    let py_top = u32(clamp(f32(mid_py) - l_curr * amp_scale, 0.0, grid_size.y - 1.0));
    let py_bot = u32(clamp(f32(mid_py) + r_curr * amp_scale, 0.0, grid_size.y - 1.0));
    let next_py_top = u32(clamp(f32(mid_py) - l_next * amp_scale, 0.0, grid_size.y - 1.0));
    let next_py_bot = u32(clamp(f32(mid_py) + r_next * amp_scale, 0.0, grid_size.y - 1.0));

    let min_yt = min(py_top, next_py_top);
    let max_yt = max(py_top, next_py_top);
    let min_yb = min(py_bot, next_py_bot);
    let max_yb = max(py_bot, next_py_bot);

    // 4. Waveform Body Phosphor Fill & 5. Dual Stereo RMS Core Energy (Dense Audio Body)
    if py + 1u >= min_yt && py <= max_yb + 1u {
        let in_body = (py >= min_yt && py <= max_yb);
        if in_body {
            let is_upper = py <= mid_py;
            let track_wave_col = select(wave_r_col, wave_l_col, is_upper);
            let wave_fill_col = mix(PAL_WAVE_FILL[p_i0], PAL_WAVE_FILL[p_i1], p_f);
            let track_fill_col = mix(wave_fill_col, track_wave_col * 0.42, 0.45);

            let d_center = abs(f32(py) - f32(mid_py)) / (grid_size.y * 0.5);
            let fa = clamp(1.0 - d_center * 0.65, 0.0, 1.0);
            let fill_atten = fa * sqrt(fa);
            let fill_col = track_fill_col * fill_atten * 0.52 * history_fade;
            col = max(col, fill_col);

            let curr_rms = sample_timeline_rms(pos_curr);
            let rms_l_scaled = fast_pow055(curr_rms.x * 1.5);
            let rms_r_scaled = fast_pow055(curr_rms.y * 1.5);
            let rms_top = u32(clamp(f32(mid_py) - rms_l_scaled * amp_scale * 0.72, 0.0, grid_size.y - 1.0));
            let rms_bot = u32(clamp(f32(mid_py) + rms_r_scaled * amp_scale * 0.72, 0.0, grid_size.y - 1.0));

            if (py >= rms_top && py <= rms_bot) {
                col = col + track_wave_col * 0.22 * history_fade;
            }
        }

        // Peak Boundary Laser Outlines (Left on top in wave_l, Right on bottom in wave_r)
        if py >= min_yt && py <= max_yt {
            col = max(col, wave_l_col * history_fade);
        } else if abs(i32(py) - i32(py_top)) == 1 {
            col = col + wave_l_col * 0.42 * history_fade;
        }

        if py >= min_yb && py <= max_yb {
            col = max(col, wave_r_col * history_fade);
        } else if abs(i32(py) - i32(py_bot)) == 1 {
            col = col + wave_r_col * 0.42 * history_fade;
        }
    }

    // 6. The Optical Playhead Bar & Magnetic Flux Head Gap + 7. Cathode Spark / Transient Impact Points
    let dx_playhead = abs(i32(px) - i32(playhead_px));
    if dx_playhead <= 66 {
        let now_p = timeline_data[200u];
        let now_m = timeline_data[201u];
        let now_l = fast_pow055(max(abs(now_p.x), abs(now_p.y)));
        let now_r = fast_pow055(max(abs(now_p.z), abs(now_p.w)));
        let now_bass = clamp(now_m.z * 1.5, 0.0, 1.0);
        let now_treble = clamp(now_m.w * 1.5, 0.0, 1.0);

        let dist_mid_norm = abs(f32(py) - f32(mid_py)) / (grid_size.y * 0.5);
        let now_slit_energy = mix(now_bass, now_treble, dist_mid_norm);
        let slit_e12 = now_slit_energy * mix(1.0, now_slit_energy, 0.25);

        let flare_w = i32(3.0 + now_slit_energy * 8.0 + now_bass * 6.0);
        if dx_playhead <= flare_w {
            let playhead_col = mix(PAL_PLAYHEAD[p_i0], PAL_PLAYHEAD[p_i1], p_f);
            let slit_flare = playhead_col * slit_e12 * 1.8;
            if px == playhead_px {
                let is_tick = (py % 6u == 0u);
                let base_slit = select(playhead_col * 0.45, playhead_col, is_tick);
                col = max(col, base_slit + slit_flare);
            } else {
                let t_att = 1.0 - f32(dx_playhead) / f32(flare_w + 1);
                col = col + slit_flare * (t_att * t_att * 0.35);
            }
        }

        let spark_core_col = mix(PAL_SPARK_CORE[p_i0], PAL_SPARK_CORE[p_i1], p_f);
        let spark_halo_col = mix(PAL_SPARK_HALO[p_i0], PAL_SPARK_HALO[p_i1], p_f);

        // Playhead Top & Bottom Caliper Brackets (Studer Tape Head Guides)
        if (py <= 16u || py >= (u32(grid_size.y) - 17u)) && dx_playhead <= 4 {
            let anchor_y = select(u32(grid_size.y) - 12u, 11u, py <= 16u);
            let dy_c = abs(i32(py) - i32(anchor_y));
            if (dx_playhead + dy_c) <= 4 {
                col = spark_core_col;
            }
        }

        // 7. Cathode Spark / Transient Impact Points (Left Track Top, Right Track Bottom)
        let hit_h_l = u32(round(now_l * amp_scale));
        let hit_h_r = u32(round(now_r * amp_scale));
        let hit_py_top = select(mid_py, mid_py - hit_h_l, mid_py >= hit_h_l);
        let hit_py_bot = min(u32(grid_size.y - 1.0), mid_py + hit_h_r);

        let dy_hit_top = abs(i32(py) - i32(hit_py_top));
        let dy_hit_bot = abs(i32(py) - i32(hit_py_bot));
        let spark_rad = 6.0 + now_bass * 16.0;
        let max_spark_d = spark_rad * 2.0;

        if f32(dx_playhead) < max_spark_d && f32(min(dy_hit_top, dy_hit_bot)) < max_spark_d {
            let dist_hit_top = sqrt(f32(dx_playhead * dx_playhead + dy_hit_top * dy_hit_top));
            let dist_hit_bot = sqrt(f32(dx_playhead * dx_playhead + dy_hit_bot * dy_hit_bot));
            let dist_hit = min(dist_hit_top, dist_hit_bot);

            if dist_hit < max_spark_d {
                let norm_d = dist_hit / max_spark_d;
                let one_m_d = 1.0 - norm_d;
                let halo = (one_m_d * one_m_d) * (1.0 + now_bass * 1.5);
                let halo_tint = select(wave_r_col, wave_l_col, dist_hit_top <= dist_hit_bot);
                col = col + mix(spark_halo_col, halo_tint, 0.45) * halo * 0.85;
                if norm_d < 0.22 {
                    let core = (1.0 - norm_d / 0.22) * 2.5;
                    col = col + spark_core_col * core;
                }
            }
        }

        // Diamond Spark Center
        let diamond_size = i32(2.0 + now_bass * 3.0);
        if (dx_playhead + dy_hit_top) <= diamond_size || (dx_playhead + dy_hit_bot) <= diamond_size {
            col = spark_core_col;
        }

        // Horizontal Laser Streak across both sparks
        let streak_len = i32(16.0 + now_treble * 50.0);
        if (dy_hit_top == 0 || dy_hit_bot == 0) && dx_playhead <= streak_len {
            let att = (1.0 - f32(dx_playhead) / f32(streak_len)) * (0.35 + now_treble * 0.75);
            col = col + spark_halo_col * att;
        }

        // Micro Electric Plasma Discharge Arcs (gated to playhead neighborhood)
        if dx_playhead <= 4 && (dy_hit_top <= 5 || dy_hit_bot <= 5) {
            let arc_seed = floor(audio.smooth_time * 24.0);
            for (var a = 0; a < 4; a = a + 1) {
                let f_a = f32(a);
                let arc_rnd_x = i32(hash11_tape(arc_seed + f_a * 13.1) * 6.0) - 3;
                if (i32(px) == i32(playhead_px) + arc_rnd_x) {
                    let arc_rnd_y = i32(hash11_tape(arc_seed + f_a * 29.7) * 8.0) - 4;
                    let is_arc_top = (i32(py) == i32(hit_py_top) + arc_rnd_y);
                    let is_arc_bot = (i32(py) == i32(hit_py_bot) - arc_rnd_y);
                    if is_arc_top || is_arc_bot {
                        col = spark_core_col;
                    }
                }
            }
        }

        // Embers / Sparks Spraying Leftward into History (gated to history side and vertical trajectory band)
        if px < playhead_px && dx_playhead <= 48 && (dy_hit_top <= 24 || dy_hit_bot <= 24) {
            let ember_time = audio.smooth_time * 8.0;
            let base_ember_cycle = floor(ember_time);
            let ember_frac = fract(ember_time);

            let num_embers = 8u;
            for (var e = 0u; e < num_embers; e = e + 1u) {
                let f_e = f32(e);
                let seed = base_ember_cycle + f_e * 17.31;
                let vx = -(hash11_tape(seed * 1.1) * 35.0 + 8.0 + now_treble * 30.0);
                let life = clamp(1.0 - ember_frac + hash11_tape(seed * 3.7) * 0.3, 0.0, 1.0);
                let age = 1.0 - life;

                let ember_x = i32(playhead_px) + i32(vx * age);
                if (i32(px) != ember_x) { continue; }

                let vy = (hash11_tape(seed * 2.3) * 2.0 - 1.0) * (12.0 + now_bass * 14.0);
                let y_disp = i32(vy * age + (vy * vy * 0.015));
                let ember_y_top = i32(hit_py_top) + y_disp;
                let ember_y_bot = i32(hit_py_bot) - y_disp;

                let is_emb_top = (i32(py) == ember_y_top);
                let is_emb_bot = (i32(py) == ember_y_bot);

                if is_emb_top || is_emb_bot {
                    let heat = life * (0.6 + now_treble * 0.6);
                    let ember_col = select(
                        mix(PAL_EMBER_B[p_i0], PAL_EMBER_B[p_i1], p_f),
                        mix(PAL_EMBER_A[p_i0], PAL_EMBER_A[p_i1], p_f),
                        is_emb_top
                    );
                    col = max(col, ember_col * heat);
                }
            }
        }
    }

    // 8. ACES Tonemapping BEFORE CRT post-processing (standard pipeline order)
    var final_color = aces_tonemap(col);

    var crt_settings = get_default_crt();
    crt_settings.scanline_intensity = 0.12;
    crt_settings.vignette_scale = 1.80;
    crt_settings.vignette_softness = 0.95;
    crt_settings.noise_intensity = 0.018;

    final_color = apply_crt_effects(final_color, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);

    return vec4<f32>(final_color, 1.0);
}
