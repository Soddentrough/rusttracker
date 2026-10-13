// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

fn get_spectrum_bin(bin_idx: u32) -> f32 {
    let safe_idx = min(bin_idx, 1023u);
    return audio.spectrum[safe_idx >> 2u][safe_idx & 3u];
}

fn get_peak_bin(bin_idx: u32) -> f32 {
    let safe_idx = min(bin_idx, 1023u);
    return audio.fire_heat[safe_idx >> 2u][safe_idx & 3u];
}

// Pool across the full 1024 logarithmically-binned spectrum (20Hz..10kHz)
// and map with a studio logarithmic dB curve that uses the full [0..100] range without early clipping.
fn sample_column_energy(col_idx: u32, num_cols: u32) -> vec2<f32> {
    let b0 = (col_idx * 1024u) / num_cols;
    let b1 = max(b0 + 1u, ((col_idx + 1u) * 1024u) / num_cols);

    var max_amp = 0.0;
    var avg_amp = 0.0;
    var max_pk = 0.0;
    let count = min(b1 - b0, 24u);
    for (var i = 0u; i < count; i = i + 1u) {
        let b = b0 + i;
        let a = get_spectrum_bin(b);
        let p = get_peak_bin(b);
        max_amp = max(max_amp, a);
        avg_amp += a;
        max_pk = max(max_pk, p);
    }
    let amp = mix(max_amp, avg_amp / f32(max(count, 1u)), 0.30);

    // Studio logarithmic dB compression across [0..100]: sensitive at low levels, never clips at 22%
    let log_denom = log2(1.0 + 36.0);
    let norm_amp = clamp(log2(1.0 + amp * 0.36) / log_denom, 0.0, 1.0);
    let norm_peak = clamp(log2(1.0 + max_pk * 0.36) / log_denom, 0.0, 1.0);
    return vec2<f32>(norm_amp, max(norm_peak, norm_amp));
}

fn get_vfd_color(led_height_frac: f32) -> vec3<f32> {
    // 1980s Technics / Sony Hi-Fi Vacuum Fluorescent Display (VFD) Phosphor Standard:
    // 0.00 - 0.62: Ice-Cyan / Aqua VFD Phosphor
    // 0.62 - 0.84: Warm Amber / Gold
    // 0.84 - 1.00: Studio Ruby Red (+3dB Peak Overload)
    if led_height_frac < 0.62 {
        return mix(vec3<f32>(0.06, 0.96, 0.72), vec3<f32>(0.12, 0.90, 0.98), led_height_frac / 0.62);
    } else if led_height_frac < 0.84 {
        return vec3<f32>(1.05, 0.74, 0.08);
    } else {
        return vec3<f32>(1.15, 0.14, 0.10);
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = max(audio.aspect_ratio, 0.2);

    // Adaptively scale column count based on aspect ratio (portrait mobile vs square vs ultrawide desktop)
    let num_cols_f = clamp(floor(48.0 * aspect), 24.0, 112.0);
    let num_cols = u32(num_cols_f);
    let num_leds = 32.0;

    // Smoked acrylic VFD bezel margins
    let margin_x = 0.015;
    let margin_y = 0.03;
    if in.uv.x < margin_x || in.uv.x > (1.0 - margin_x) || in.uv.y < margin_y || in.uv.y > (1.0 - margin_y) {
        return vec4<f32>(0.005, 0.006, 0.009, 1.0);
    }

    let inner_uv = vec2<f32>(
        (in.uv.x - margin_x) / (1.0 - 2.0 * margin_x),
        (in.uv.y - margin_y) / (1.0 - 2.0 * margin_y)
    );

    // Column & Row Grid Indices
    let col_f = clamp(inner_uv.x, 0.0, 0.9999) * num_cols_f;
    let col_i = floor(col_f);
    let col_frac = fract(col_f);

    let row_f = clamp(1.0 - inner_uv.y, 0.0, 0.9999) * num_leds;
    let row_i = floor(row_f);
    let row_frac = fract(row_f);

    let energy = sample_column_energy(u32(col_i), num_cols);
    let amp_level = energy.x;
    let peak_level = energy.y;

    // Segment bevel borders inside each VFD block
    let pad_x = 0.13;
    let pad_y = 0.18;
    let is_led_body = col_frac >= pad_x && col_frac <= (1.0 - pad_x) && row_frac >= pad_y && row_frac <= (1.0 - pad_y);

    let led_norm_y = (row_i + 0.5) / num_leds;
    let is_lit = led_norm_y <= amp_level;

    // Quantized integer row comparison prevents peak-hold cap from flickering between 1 and 2 LEDs thick
    let peak_row = min(floor(peak_level * num_leds), num_leds - 1.0);
    let is_peak_cap = (row_i == peak_row) && (peak_level > 0.03);

    let base_led_col = get_vfd_color(led_norm_y);

    // Bevel & VFD fine anode mesh / tungsten filament wire texture
    let center_dx = abs(col_frac - 0.5) / (0.5 - pad_x);
    let center_dy = abs(row_frac - 0.5) / (0.5 - pad_y);
    let bevel = 1.0 - 0.24 * max(center_dx, center_dy);

    // Subtle VFD cathode filament wire shadow & phosphor grain inside each segment
    let filament_wire = 0.92 + 0.08 * cos(inner_uv.y * num_leds * 18.84955);
    let mesh_pattern = 0.94 + 0.06 * sin(col_frac * 37.7) * sin(row_frac * 25.1);
    let vfd_texture = bevel * filament_wire * mesh_pattern;

    // Subtle studio dB calibration scale lines every 4 rows in background
    let is_db_line = (u32(row_i) % 4u == 0u) && (abs(row_frac - 0.06) < 0.04);
    var color = vec3<f32>(0.007, 0.010, 0.014);
    if is_db_line {
        color += vec3<f32>(0.012, 0.022, 0.028);
    }

    if is_led_body {
        if is_peak_cap {
            // Crisp white-cyan VFD peak hold segment
            let cap_tint = mix(vec3<f32>(1.65, 1.90, 2.00), vec3<f32>(2.00, 1.40, 1.25), step(0.84, led_norm_y));
            color = cap_tint * vfd_texture;
        } else if is_lit {
            // Lit VFD phosphor segment with hot inner core glow
            let core_boost = 1.0 + 0.25 * (1.0 - center_dx * center_dx) * (1.0 - center_dy * center_dy);
            color = base_led_col * 1.38 * vfd_texture * core_boost;
        } else {
            // Unlit VFD anode segment (characteristic dark gray-green phosphor silhouette under glass)
            var afterglow = 0.0;
            if led_norm_y > amp_level && led_norm_y < peak_level {
                afterglow = (1.0 - (led_norm_y - amp_level) / max(peak_level - amp_level, 0.001)) * 0.14;
            }
            color = base_led_col * (0.042 + afterglow) + vec3<f32>(0.012, 0.015, 0.019) * vfd_texture;
        }
    } else {
        // Inter-segment glass bloom bleed from lit segments
        if is_lit || is_peak_cap {
            let gap_glow = exp(-max(center_dx - 1.0, 0.0) * 3.0) * exp(-max(center_dy - 1.0, 0.0) * 3.0);
            color += base_led_col * 0.16 * gap_glow;
        }
    }

    // Subtle smoked-glass diagonal reflection sheen across the VFD faceplate
    let glass_sheen = smoothstep_r(0.18, 0.0, abs((inner_uv.x * 0.65 + inner_uv.y * 0.35) - 0.42)) * 0.018;
    color += vec3<f32>(0.5, 0.75, 0.9) * glass_sheen;

    color = aces_tonemap(color);

    var crt = get_default_crt();
    crt.scanline_intensity = 0.15;
    crt.noise_intensity = 0.007;
    crt.vignette_scale = 1.45;
    crt.vignette_softness = 0.85;
    crt.phosphor_tint = vec3<f32>(0.96, 1.02, 0.99);

    color = apply_crt_effects(color, in.uv, in.clip_position.xy, audio.smooth_time, crt);

    return vec4<f32>(color, 1.0);
}
