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

// Returns vec3(min_dist, screen_space_slope_dy_dx, y_at_u) with hoisted endpoint reads
fn get_wave_dist_and_slope(hist_idx: u32, u_sweep: f32, uv: vec2<f32>, aspect: f32, search_radius: i32) -> vec3<f32> {
    let res = f32(max(audio.waveform_resolution, 128u));
    let inv_res = 1.0 / (res - 1.0);
    let clamped_u = clamp(u_sweep, 0.0, 0.9999);
    let float_idx = clamped_u * (res - 1.0);
    let idx = u32(float_idx);
    let frac = fract(float_idx);

    var min_dist = 1000.0;
    var best_slope = 0.0;

    let start_idx = max(0i, i32(idx) - search_radius);
    let end_idx = min(i32(res) - 2i, i32(idx) + search_radius);

    let p = vec2<f32>(uv.x * aspect, uv.y);

    // Active sweep spans x in [0.03, 0.97] (width 0.94) and y amplitude 0.38 around 0.5
    let sweep_left = 0.03;
    let sweep_width = 0.94;
    let dx_seg = inv_res * sweep_width * aspect;
    let inv_dx_seg = 1.0 / max(dx_seg, 1e-5);

    let start_u = u32(start_idx);
    var v0 = get_waveform(hist_idx, start_u);
    var y0 = v0 * 0.38 + 0.5;
    var a = vec2<f32>((sweep_left + f32(start_u) * inv_res * sweep_width) * aspect, y0);
    var y_at_u = y0;

    for (var i = start_idx; i <= end_idx; i = i + 1) {
        let u_idx0 = u32(i);
        let u_idx1 = u_idx0 + 1u;
        let v1 = get_waveform(hist_idx, u_idx1);
        let y1 = v1 * 0.38 + 0.5;
        let b = vec2<f32>((sweep_left + f32(u_idx1) * inv_res * sweep_width) * aspect, y1);

        if u_idx0 == idx {
            y_at_u = mix(y0, y1, frac);
        }

        let d = sd_line_segment(p, a, b);
        if d < min_dist {
            min_dist = d;
            // True geometric screen-space derivative |dy / dx|
            best_slope = abs(y1 - y0) * inv_dx_seg;
        }
        v0 = v1;
        y0 = y1;
        a = b;
    }

    return vec3<f32>(min_dist, best_slope, y_at_u);
}

// Wide 2D radial glass faceplate halation sampled across a broad horizontal span of the live trace
fn eval_glass_halation(hist_idx: u32, u_sweep: f32, uv: vec2<f32>, aspect: f32) -> f32 {
    let res = f32(max(audio.waveform_resolution, 128u));
    let sweep_left = 0.03;
    let sweep_width = 0.94;
    let p = vec2<f32>(uv.x * aspect, uv.y);

    var halo_accum = 0.0;
    // 7 coarse taps spanning [-0.075, +0.075] horizontally for true 2D radial halation
    for (var k = -3i; k <= 3i; k = k + 1i) {
        let u_tap = clamp(u_sweep + f32(k) * 0.025, 0.0, 0.9999);
        let tap_idx = u32(u_tap * (res - 1.0));
        let tap_y = get_waveform(hist_idx, tap_idx) * 0.38 + 0.5;
        let tap_x = (sweep_left + u_tap * sweep_width) * aspect;
        let d2 = dot(p - vec2<f32>(tap_x, tap_y), p - vec2<f32>(tap_x, tap_y));
        halo_accum += exp(-sqrt(d2) * 18.0) + 0.0008 / (d2 + 0.0045);
    }
    return halo_accum / 7.0;
}

// Authentic Tektronix 465B internal etched CRT graticule (10x8 divisions + 5x center sub-ticks)
fn eval_tektronix_graticule(uv: vec2<f32>, aspect: f32) -> f32 {
    let left = 0.03;
    let right = 0.97;
    let top = 0.05;
    let bottom = 0.95;

    if uv.x < left - 0.003 || uv.x > right + 0.003 || uv.y < top - 0.004 || uv.y > bottom + 0.004 {
        return 0.0;
    }

    let norm_x = (uv.x - 0.5) / (right - left);
    let norm_y = (uv.y - 0.5) / (bottom - top);
    let gx = norm_x * 10.0;
    let gy = norm_y * 8.0;

    let dist_x = abs(fract(gx + 0.5) - 0.5) / (10.0 / max(aspect, 0.5));
    let dist_y = abs(fract(gy + 0.5) - 0.5) / 8.0;

    // Outer frame border is slightly bolder than internal division lines
    let border_dist = min(
        min(abs(uv.x - left), abs(uv.x - right)) * max(aspect, 0.5),
        min(abs(uv.y - top), abs(uv.y - bottom))
    );
    let border_line = smoothstep_r(0.0028, 0.0007, border_dist);
    let major_line = smoothstep_r(0.0020, 0.00045, min(dist_x, dist_y));

    // 5x sub-division tick marks along the central X (gy=0) and Y (gx=0) axes
    let sub_x = abs(fract(gx * 5.0 + 0.5) - 0.5) / (50.0 / max(aspect, 0.5));
    let sub_y = abs(fract(gy * 5.0 + 0.5) - 0.5) / 40.0;
    let on_x_axis = smoothstep_r(0.011, 0.005, abs(norm_y));
    let on_y_axis = smoothstep_r(0.009, 0.004, abs(norm_x));
    let sub_ticks = (smoothstep_r(0.0017, 0.0004, sub_x) * on_x_axis)
                  + (smoothstep_r(0.0017, 0.0004, sub_y) * on_y_axis);

    return clamp(max(major_line * 0.70, border_line * 0.90) + sub_ticks * 0.85, 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Subtle spherical CRT faceplate curvature
    let final_uv = crt_distort_uv(in.uv, 0.042);

    let aspect = max(audio.aspect_ratio, 0.01);
    let crt_uv = in.uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    let r = sqrt(r2);

    // Electrostatic deflection astigmatism: spot slightly defocuses toward outer corners
    let edge_blur = smoothstep(0.35, 1.40, r);

    // P19 / P39 long-persistence amber phosphor spectral palette (linear HDR)
    // - Incandescent cream-white/gold core where the electron beam dwells
    // - Rich golden-amber fluorescence corona
    // - Deep burnt-orange/vermilion long-persistence phosphorescence afterglow
    let amber_incandescent = vec3<f32>(2.55, 1.80, 0.88);
    let amber_hot          = vec3<f32>(1.48, 0.74, 0.14);
    let amber_mid          = vec3<f32>(1.04, 0.36, 0.024);
    let amber_afterglow    = vec3<f32>(0.62, 0.12, 0.005);

    let dy = dpdy(in.uv.y);
    let base_thickness = max(dy * 0.75, 0.00065);
    let spot_base = base_thickness * (1.0 + edge_blur * 1.35);

    let hist_count = min(audio.waveform_history_size, 144u);
    // Evaluate the most recent 15 frames at step=1 inside the active vertical wave band
    let in_wave_band = abs(final_uv.y - 0.5) <= 0.42;
    let trail_frames = select(1u, 15u, in_wave_band);
    let start_frame = select(0u, hist_count - trail_frames, hist_count > trail_frames);

    // Map horizontal sweep cleanly to the Tektronix graticule window [0.03, 0.97]
    // and blank the electron beam outside the sweep margins (eliminating edge clamp walls)
    let u_sweep = (final_uv.x - 0.03) / 0.94;
    let sweep_blank = smoothstep(0.020, 0.031, final_uv.x)
                    * smoothstep_r(0.980, 0.969, final_uv.x)
                    * smoothstep(0.015, 0.035, final_uv.y)
                    * smoothstep_r(0.985, 0.965, final_uv.y);

    var accum_phosphor = vec3<f32>(0.0);
    var beam_light = 0.0;

    if sweep_blank > 0.0001 && hist_count > 0u {
        var prev_y = 0.5;
        var has_prev = false;

        for (var i = start_frame; i < hist_count; i = i + 1u) {
            let search_rad = select(1i, 5i, i + 2u >= hist_count);
            let wave_info = get_wave_dist_and_slope(i, u_sweep, final_uv, aspect, search_rad);
            let true_dist = wave_info.x;
            let slope = wave_info.y;
            let curr_y = wave_info.z;

            let frames_old = max(0.0, f32(hist_count - 1u - i) + audio.step_fraction - 0.25);

            // True analog electrostatic beam dwell: I = 1 / sqrt(1 + k * (dy/dx)^2)
            // Horizontal crests/plateaus burn bright & slightly wider; fast vertical edges thin out
            let dwell = inverseSqrt(1.0 + slope * slope * 0.95);
            let turning_point_boost = smoothstep_r(0.65, 0.0, slope) * 0.40;

            if i == hist_count - 1u {
                // --- 1. Active Electron Beam Sweep (Prompt Fluorescence) ---
                let live_radius = spot_base * (0.72 + 0.60 * dwell);
                let core = smoothstep_r(live_radius, 0.0, true_dist) * (0.16 + 1.55 * dwell + turning_point_boost);

                // Tight phosphor scatter corona around the live beam (kept inside exact 2D search window)
                let corona_sigma = spot_base * (1.8 + 1.4 * dwell);
                let corona = exp(-(true_dist * true_dist) / (2.0 * corona_sigma * corona_sigma))
                           * (0.20 + 0.80 * dwell) * 0.42;

                // High-dwell crests heat the phosphor to incandescent cream-gold; fast slews stay warm amber
                let core_temp = clamp(dwell * 1.15, 0.0, 1.0);
                let live_core_col = mix(amber_hot, amber_incandescent, core_temp * core_temp);

                accum_phosphor += live_core_col * core + amber_hot * corona;
                beam_light += core + corona * 1.5;
            } else {
                // --- 2. Long-Persistence Phosphorescence Afterglow (P19 / P39 Decay) ---
                // Bi-exponential phosphor decay:
                // - Fast prompt decay over first ~3 frames
                // - Slow, diffuse deep-amber phosphorescence tail over ~24 frames
                let prompt_decay = exp(-frames_old * 0.65);
                let slow_decay = exp(-frames_old * 0.16);

                // Isotropic 2D Gaussian spot diffusion stays strictly inside the 2D segment search
                // radius so it never clips into vertical columns
                let hist_radius = min(spot_base * (1.15 + frames_old * 0.16), 0.0055);
                let gaussian_trace = exp(-(true_dist * true_dist) / (2.0 * hist_radius * hist_radius))
                                   * (prompt_decay * 0.36 + slow_decay * 0.16);

                // Gentle inter-frame bridge only on slow-moving horizontal segments (dwell^3 suppresses vertical edges)
                var sweep_bridge = 0.0;
                if has_prev {
                    let y_min = min(curr_y, prev_y);
                    let y_max = max(curr_y, prev_y);
                    let dy_span = max(0.0, max(y_min - final_uv.y, final_uv.y - y_max));
                    let bridge_mask = exp(-(dy_span * dy_span) / (2.0 * hist_radius * hist_radius))
                                    * exp(-(true_dist * true_dist) / (2.0 * 0.005 * 0.005));
                    sweep_bridge = bridge_mask * (dwell * dwell * dwell) * slow_decay * 0.12;
                }

                let persistence_dwell = 0.18 + 0.82 * dwell;
                let trail_energy = gaussian_trace * persistence_dwell + sweep_bridge;

                let cool_t = clamp(frames_old / 8.0, 0.0, 1.0);
                let trail_col = mix(amber_hot, mix(amber_mid, amber_afterglow, cool_t), cool_t);

                accum_phosphor += trail_col * trail_energy;
                beam_light += trail_energy;
            }

            prev_y = curr_y;
            has_prev = true;
        }

        // True 2D radial internal glass faceplate halation from the active sweep
        let glass_halo = eval_glass_halation(hist_count - 1u, u_sweep, final_uv, aspect);
        accum_phosphor += amber_mid * glass_halo * 0.095;
        beam_light += glass_halo * 0.35;
    }

    accum_phosphor *= sweep_blank;
    beam_light *= sweep_blank;

    // --- 3. Micro-Crystalline Phosphor Coating Texture (Fixed in Tube UV Space) ---
    // Real vector CRTs have no horizontal TV raster scanlines; instead the phosphor coating
    // has a fine, static micro-granular crystalline structure visible when illuminated
    let grain_uv = final_uv * vec2<f32>(aspect, 1.0) * 720.0;
    let phosphor_grain = value_noise2d(grain_uv) * 0.65 + value_noise2d(grain_uv * 2.3) * 0.35;
    let grain_mod = 0.91 + 0.18 * phosphor_grain;
    accum_phosphor *= grain_mod;

    // --- 4. Unlit Amber Phosphor Cavity & Curved Leaded Glass Faceplate ---
    let tube_mask = smoothstep_r(1.02, 0.96, max(abs(final_uv.x - 0.5) * 2.0, abs(final_uv.y - 0.5) * 2.0));
    let glass_curvature = clamp(1.0 - r2 * 0.32, 0.25, 1.0);
    // Subtle overhead ambient reflection on the curved glass dome
    let dome_spec = exp(-length((final_uv - vec2<f32>(0.5, 0.28)) * vec2<f32>(aspect * 0.9, 1.4)) * 3.2) * 0.008;
    let unlit_phosphor = (vec3<f32>(0.015, 0.0055, 0.0009) * glass_curvature * (0.94 + 0.12 * phosphor_grain)
                        + vec3<f32>(0.022, 0.012, 0.004) * dome_spec) * tube_mask;

    // --- 5. Etched Tektronix Graticule (Edge-Lit by Incandescent Amber Bezel Lamps + Beam Scatter) ---
    let graticule = eval_tektronix_graticule(final_uv, aspect) * tube_mask;
    // Edge illumination is stronger near the top and bottom bezel where the incandescent bulbs sit
    let lamp_falloff = 0.72 + 0.55 * pow(clamp(abs(final_uv.y - 0.5) * 2.1, 0.0, 1.0), 2.0);
    let beam_scatter = clamp(beam_light * 0.45, 0.0, 0.85);
    let grat_color = vec3<f32>(0.58, 0.24, 0.03) * graticule * (0.095 * lamp_falloff + beam_scatter);

    // Subtle 60Hz power-supply hum stability + whisper-quiet warm thermal noise floor
    let power_hum = 0.992 + 0.008 * sin(audio.smooth_time * 24.0);
    let thermal_noise = (hash21_crt(in.clip_position.xy + fract(audio.smooth_time) * 137.0) - 0.5) * 0.003;
    let warm_noise = vec3<f32>(1.0, 0.42, 0.06) * max(thermal_noise, 0.0) * tube_mask;

    let hdr_scene = (unlit_phosphor + accum_phosphor + grat_color) * power_hum + warm_noise;
    let tonemapped = aces_tonemap(hdr_scene) * tube_mask;

    return vec4<f32>(tonemapped, 1.0);
}

