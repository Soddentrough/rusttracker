// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> waveform_history: array<f32>;

// Waveforms are pre-smoothed on CPU, so we can read directly
fn get_waveform(hist_idx: u32, idx: u32) -> f32 {
    let res = max(audio.waveform_resolution, 128u);
    let clamped_idx = min(idx, res - 1u);
    return waveform_history[hist_idx * 2048u + clamped_idx];
}

// Returns vec2(min_dist, local_slope) with hoisted endpoint reads (1 buffer fetch per segment)
fn get_wave_dist_and_slope(hist_idx: u32, uv: vec2<f32>, aspect: f32) -> vec2<f32> {
    let res = f32(max(audio.waveform_resolution, 128u));
    let inv_res = 1.0 / (res - 1.0);
    let clamped_x = clamp(uv.x, 0.0, 0.999);
    let float_idx = clamped_x * (res - 1.0);
    let idx = u32(float_idx);

    var min_dist = 1000.0;
    var best_slope = 0.0;

    let search_radius = clamp(i32(res * 0.003), 2, 8);
    let start_idx = max(0i, i32(idx) - search_radius);
    let end_idx = min(i32(res) - 2i, i32(idx) + search_radius);

    let p = vec2<f32>(uv.x * aspect, uv.y);

    // Hoist initial vertex read out of the loop to halve storage buffer fetches
    let start_u = u32(start_idx);
    var v0 = get_waveform(hist_idx, start_u);
    var a = vec2<f32>(f32(start_u) * inv_res * aspect, v0 * 0.4 + 0.5);

    for (var i = start_idx; i <= end_idx; i = i + 1) {
        let u_idx1 = u32(i) + 1u;
        let v1 = get_waveform(hist_idx, u_idx1);
        let b = vec2<f32>(f32(u_idx1) * inv_res * aspect, v1 * 0.4 + 0.5);

        let d = sd_line_segment(p, a, b);
        if d < min_dist {
            min_dist = d;
            best_slope = abs(v1 - v0);
        }
        v0 = v1;
        a = b;
    }

    return vec2<f32>(min_dist, best_slope);
}

// Authentic Tektronix 465B etched CRT graticule (10x8 divisions + 5x sub-ticks on center axes)
fn eval_tektronix_graticule(uv: vec2<f32>, aspect: f32) -> f32 {
    if uv.x < 0.02 || uv.x > 0.98 || uv.y < 0.04 || uv.y > 0.96 {
        return 0.0;
    }
    let gx = (uv.x - 0.5) * 10.0;
    let gy = (uv.y - 0.5) * 8.0;

    let dist_x = abs(fract(gx + 0.5) - 0.5) / (10.0 / max(aspect, 0.5));
    let dist_y = abs(fract(gy + 0.5) - 0.5) / 8.0;
    let major_line = smoothstep_r(0.0022, 0.0005, min(dist_x, dist_y));

    // Sub-division tick marks along the central X (gy=0) and Y (gx=0) axes
    let sub_x = abs(fract(gx * 5.0 + 0.5) - 0.5) / (50.0 / max(aspect, 0.5));
    let sub_y = abs(fract(gy * 5.0 + 0.5) - 0.5) / 40.0;
    let on_x_axis = smoothstep_r(0.012, 0.006, abs(uv.y - 0.5));
    let on_y_axis = smoothstep_r(0.010, 0.005, abs(uv.x - 0.5));
    let sub_ticks = (smoothstep_r(0.0018, 0.0004, sub_x) * on_x_axis)
                  + (smoothstep_r(0.0018, 0.0004, sub_y) * on_y_axis);

    return clamp(major_line * 0.65 + sub_ticks * 0.85, 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // CRT barrel distortion
    let final_uv = crt_distort_uv(in.uv, 0.05);

    let aspect = max(audio.aspect_ratio, 0.01);
    let crt_uv = in.uv * 2.0 - 1.0;
    let r = length(crt_uv);
    let edge_blur = smoothstep(0.2, 1.5, r);

    // P31/P39 dual-persistence phosphor palette:
    // Fresh beam hits glow hot cream-gold, cooling through classic amber into deep burnt orange
    let amber_hot = vec3<f32>(1.35, 0.82, 0.28);
    let amber_mid = vec3<f32>(1.00, 0.45, 0.05);
    let amber_old = vec3<f32>(0.72, 0.20, 0.01);

    let dy = dpdy(in.uv.y);
    let base_thickness = max(dy * 0.6, 0.0005);
    let blur_thickness = max(dy * 2.0, 0.0015);

    let hist_count = min(audio.waveform_history_size, 144u);
    let step = select(1u, 2u, hist_count > 72u);
    let step_scale = f32(step);

    let thickness = base_thickness + edge_blur * blur_thickness;
    let bloom_spread = max(dy * 0.2, 0.0002);

    var accum_phosphor = vec3<f32>(0.0);
    var wave_intensity = 0.0;

    // Waveform amplitude is bounded to y in [0.1, 0.9]; outside [0.04, 0.96] only evaluate newest frames for outer bloom
    let in_wave_band = abs(final_uv.y - 0.5) <= 0.46;
    let start_frame = select(max(hist_count, 4u) - 4u, 0u, in_wave_band);

    for (var i = start_frame; i < hist_count; i = i + step) {
        let wave_info = get_wave_dist_and_slope(i, final_uv, aspect);
        let true_dist = wave_info.x;
        let slope = wave_info.y;

        let frames_old = f32(hist_count - 1u - i) + audio.step_fraction;
        let age = exp(-frames_old * 0.06);

        // Authentic analog electron beam dwell modulation: fast vertical slews spread energy thinner
        let dwell = 1.0 / (1.0 + slope * 4.5);

        let core = smoothstep_r(thickness, 0.0, true_dist) * (0.52 + 0.16 * dwell);
        let bloom = 0.00005 / (true_dist * true_dist + bloom_spread) * 0.02;
        let halation = exp(-true_dist * 120.0) * 0.01;

        let frame_intensity = (core + bloom + halation) * age * step_scale;
        wave_intensity += frame_intensity;

        // Phosphor temperature cools with frame age
        let fresh_t = exp(-frames_old * 0.35);
        let cool_t = clamp(frames_old / 48.0, 0.0, 1.0);
        let trace_col = mix(mix(amber_mid, amber_old, cool_t), amber_hot, fresh_t);
        accum_phosphor += trace_col * frame_intensity;
    }

    // Etched Tektronix graticule illuminated by ambient tube glow + active electron beam halation
    let bezel = smoothstep_r(1.4, 0.9, r);
    let graticule = eval_tektronix_graticule(final_uv, aspect) * bezel;
    let grat_color = vec3<f32>(0.45, 0.22, 0.04) * graticule * (0.045 + clamp(wave_intensity * 0.25, 0.0, 0.35));

    let tonemapped = aces_tonemap(accum_phosphor + grat_color);

    var crt_settings = get_default_crt();
    let noise_val = hash21_crt(in.clip_position.xy + fract(audio.smooth_time) * 137.0);
    let noise_glow = amber_mid * noise_val * 0.015 * bezel * clamp(wave_intensity * 0.8, 0.0, 1.0);

    var final_color = apply_crt_effects(tonemapped, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);
    final_color = final_color + noise_glow;

    return vec4<f32>(final_color, 1.0);
}
