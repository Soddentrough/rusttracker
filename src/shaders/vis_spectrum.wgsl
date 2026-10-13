// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

// 80s/90s Amiga Copperbar & Demoscene Raster Palette
// Smoothly interpolates vertical bar height (0 = bottom, 1 = top) with audio heat boost
fn copperbar_color(height_norm: f32, col_energy: f32) -> vec3<f32> {
    let h = clamp(height_norm + col_energy * 0.10, 0.0, 1.0);
    let c0 = vec3<f32>(0.08, 0.16, 0.75); // Deep Amiga Cobalt Blue
    let c1 = vec3<f32>(0.05, 0.82, 0.96); // Electric Demoscene Cyan
    let c2 = vec3<f32>(0.18, 0.95, 0.42); // Phosphor Emerald
    let c3 = vec3<f32>(1.00, 0.78, 0.10); // Copper Gold / Amber
    let c4 = vec3<f32>(1.00, 0.16, 0.38); // Raster Crimson / Magenta
    let c5 = vec3<f32>(1.35, 1.30, 1.45); // Overdriven White-Hot Tip

    if h < 0.22 {
        return mix(c0, c1, h / 0.22);
    } else if h < 0.45 {
        return mix(c1, c2, (h - 0.22) / 0.23);
    } else if h < 0.68 {
        return mix(c2, c3, (h - 0.45) / 0.23);
    } else if h < 0.88 {
        return mix(c3, c4, (h - 0.68) / 0.20);
    } else {
        return mix(c4, c5, (h - 0.88) / 0.12);
    }
}

fn get_spectrum_bin(idx: u32) -> f32 {
    let c_idx = min(idx, 1023u);
    return audio.spectrum[c_idx >> 2u][c_idx & 3u];
}

fn get_peak_bin(idx: u32) -> f32 {
    let c_idx = min(idx, 1023u);
    return audio.fire_heat[c_idx >> 2u][c_idx & 3u];
}

// Pool across the bin slice belonging to column `col_idx` out of `num_cols`
// so narrow harmonic peaks are never missed between columns.
fn sample_column_spectrum(col_idx: u32, num_cols: u32) -> vec2<f32> {
    let b0 = (col_idx * 1024u) / num_cols;
    let b1 = max(b0 + 1u, ((col_idx + 1u) * 1024u) / num_cols);

    var max_amp = 0.0;
    var sum_amp = 0.0;
    var max_peak = 0.0;
    let count = min(b1 - b0, 16u);
    for (var i = 0u; i < count; i = i + 1u) {
        let b = b0 + i;
        let a = get_spectrum_bin(b);
        let p = get_peak_bin(b);
        max_amp = max(max_amp, a);
        sum_amp += a;
        max_peak = max(max_peak, p);
    }
    let blended_amp = mix(max_amp, sum_amp / f32(max(count, 1u)), 0.35);

    // Perceptual dB-like curve with full [0..100] headroom
    let norm_amp = clamp(pow(clamp(blended_amp / 100.0, 0.0, 1.0), 0.68) * 1.08, 0.0, 1.0);
    let norm_peak = clamp(pow(clamp(max_peak / 100.0, 0.0, 1.0), 0.68) * 1.08, 0.0, 1.0);
    return vec2<f32>(norm_amp, max(norm_peak, norm_amp));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = max(audio.aspect_ratio, 0.25);
    let num_cols_f = clamp(floor(88.0 * aspect), 32.0, 128.0);
    let num_cols = u32(num_cols_f);
    let num_rows = 48.0;

    // Horizon line at y = 0.85 separates the main copperbar analyzer from the glossy mirror floor
    let horizon_y = 0.85;
    let is_reflection = in.uv.y > horizon_y;

    var sample_y = 0.0;
    var refl_fade = 1.0;
    var uv_x = in.uv.x;

    if !is_reflection {
        sample_y = (horizon_y - in.uv.y) / horizon_y;
    } else {
        let floor_depth = (in.uv.y - horizon_y) / (1.0 - horizon_y);
        // Subtle water/mirror perspective ripple on the reflection floor
        let ripple = sin(floor_depth * 42.0 - audio.smooth_time * 4.5) * 0.0018 * floor_depth;
        uv_x = clamp(in.uv.x + ripple, 0.0, 0.9999);
        sample_y = floor_depth * 0.55;
        refl_fade = (1.0 - floor_depth) * (1.0 - floor_depth) * 0.42;
    }

    let col_f = clamp(uv_x, 0.0, 0.9999) * num_cols_f;
    let col_idx = u32(floor(col_f));
    let col_frac = fract(col_f);

    let row_f = sample_y * num_rows;
    let row_idx = floor(row_f);
    let row_frac = fract(row_f);
    let row_norm = (row_idx + 0.5) / num_rows;

    let levels = sample_column_spectrum(col_idx, num_cols);
    let amp = levels.x;
    let peak = levels.y;

    // Subtle background raster sky & graticule
    var bg_col = vec3<f32>(0.010, 0.012, 0.022);
    let grid_line = smoothstep_r(0.008, 0.0, abs(fract(sample_y * 4.0) - 0.5) - 0.492);
    bg_col += vec3<f32>(0.015, 0.025, 0.045) * grid_line;

    // Segment geometry with cylindrical Amiga copperbar vertical shading
    let in_col = col_frac >= 0.13 && col_frac <= 0.87;
    let in_row = row_frac >= 0.16 && row_frac <= 0.84;

    // Copperbar cylindrical roll-off inside each horizontal raster bar
    let bar_cyl = sin(clamp((row_frac - 0.16) / 0.68, 0.0, 1.0) * 3.14159265);
    let col_bevel = 1.0 - 0.28 * abs((col_frac - 0.5) / 0.37);

    let peak_row = min(floor(peak * num_rows), num_rows - 1.0);
    let is_peak_cap = (row_idx == peak_row) && (peak > 0.025);
    let is_lit = row_norm <= amp;

    let seg_color = copperbar_color(row_norm, amp);
    var color = bg_col;

    if in_col && in_row {
        if is_peak_cap {
            // Glowing white-cyan demoscene peak cap
            color = vec3<f32>(1.35, 1.55, 1.80) * (0.65 + 0.45 * bar_cyl) * col_bevel;
        } else if is_lit {
            // Active Amiga copperbar segment
            let tip_boost = smoothstep_r(0.06, 0.0, abs(row_norm - amp)) * 0.45;
            color = seg_color * (0.45 + 0.75 * bar_cyl + tip_boost) * col_bevel;
        } else {
            // Phosphor afterglow trail between active bar top and decaying peak cap
            var trail = 0.0;
            if row_norm > amp && row_norm < peak {
                let trail_t = (row_norm - amp) / max(peak - amp, 0.001);
                trail = (1.0 - trail_t) * 0.22;
            }
            color = bg_col + seg_color * (0.025 + trail) * bar_cyl;
        }
    } else if in_col && (is_lit || is_peak_cap) {
        // Vertical bloom bleed across inter-segment gaps
        color = bg_col + seg_color * 0.14;
    }

    if is_reflection {
        // Floor grid lines & darkened reflection tint
        let floor_grid_x = smoothstep_r(0.06, 0.0, abs(col_frac - 0.5));
        let floor_base = vec3<f32>(0.006, 0.010, 0.020) + vec3<f32>(0.01, 0.03, 0.06) * floor_grid_x * refl_fade;
        color = floor_base + color * refl_fade * vec3<f32>(0.75, 0.90, 1.10);
    }

    // Glowing neon horizon divider line
    let horizon_dist = abs(in.uv.y - horizon_y);
    let horizon_glow = exp(-horizon_dist * 320.0);
    color += vec3<f32>(0.20, 0.65, 1.10) * horizon_glow * 0.65;

    color = aces_tonemap(color);

    var crt = get_default_crt();
    crt.scanline_intensity = 0.12;
    crt.noise_intensity = 0.010;
    crt.vignette_scale = 1.50;
    crt.vignette_softness = 0.88;
    color = apply_crt_effects(color, in.uv, in.clip_position.xy, audio.smooth_time, crt);

    return vec4<f32>(color, 1.0);
}
