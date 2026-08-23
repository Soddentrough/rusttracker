// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> waveform_history: array<f32>;

fn get_spectrum_bin(bin_idx: u32) -> f32 {
    let safe_idx = min(bin_idx, 1023u);
    let vec_idx = safe_idx / 4u;
    let comp = safe_idx % 4u;
    let v = audio.spectrum[vec_idx];
    if comp == 0u { return v.x; }
    if comp == 1u { return v.y; }
    if comp == 2u { return v.z; }
    return v.w;
}

fn get_peak_bin(bin_idx: u32) -> f32 {
    let safe_idx = min(bin_idx, 1023u);
    let vec_idx = safe_idx / 4u;
    let comp = safe_idx % 4u;
    let v = audio.fire_heat[vec_idx];
    if comp == 0u { return v.x; }
    if comp == 1u { return v.y; }
    if comp == 2u { return v.z; }
    return v.w;
}

fn sample_column_energy(norm_x: f32) -> vec2<f32> {
    // Logarithmic mapping: bin 1 (20Hz) to bin 400 (~18kHz)
    let bin_f = pow(clamp(norm_x, 0.0, 1.0), 1.35) * 360.0 + 1.0;
    let bin0 = u32(clamp(floor(bin_f), 0.0, 1020.0));
    let frac = fract(bin_f);

    let amp0 = get_spectrum_bin(bin0);
    let amp1 = get_spectrum_bin(bin0 + 1u);
    let amp = mix(amp0, amp1, frac);

    let pk0 = get_peak_bin(bin0);
    let pk1 = get_peak_bin(bin0 + 1u);
    let peak = mix(pk0, pk1, frac);

    // Normalize: nominal range 0.0 to 1.0 with soft-knee boost
    let norm_amp = clamp(sqrt(amp * 0.045), 0.0, 1.0);
    let norm_peak = clamp(sqrt(peak * 0.045), 0.0, 1.0);
    return vec2<f32>(norm_amp, norm_peak);
}

fn get_led_color(led_height_frac: f32) -> vec3<f32> {
    // Studio Meter Color Standard:
    // 0.0 - 0.60: Emerald Green
    // 0.60 - 0.82: Amber / Warm Gold
    // 0.82 - 1.00: Studio Red (+3dB peak warning)
    if led_height_frac < 0.60 {
        return vec3<f32>(0.08, 0.92, 0.20);
    } else if led_height_frac < 0.82 {
        return vec3<f32>(1.0, 0.72, 0.05);
    } else {
        return vec3<f32>(1.0, 0.12, 0.08);
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = max(audio.aspect_ratio, 0.2);

    // Adaptively scale column count based on aspect ratio (portrait mobile vs square vs ultrawide desktop)
    let num_cols = clamp(floor(48.0 * aspect), 24.0, 112.0);
    let num_leds = 32.0;

    // Margins to frame the display cleanly
    let margin_x = 0.015;
    let margin_y = 0.03;
    if in.uv.x < margin_x || in.uv.x > (1.0 - margin_x) || in.uv.y < margin_y || in.uv.y > (1.0 - margin_y) {
        return vec4<f32>(0.005, 0.006, 0.008, 1.0);
    }

    let inner_uv = vec2<f32>(
        (in.uv.x - margin_x) / (1.0 - 2.0 * margin_x),
        (in.uv.y - margin_y) / (1.0 - 2.0 * margin_y)
    );

    // Column & Row Grid Indices
    let col_f = inner_uv.x * num_cols;
    let col_i = floor(col_f);
    let col_frac = fract(col_f);

    let row_f = (1.0 - inner_uv.y) * num_leds;
    let row_i = floor(row_f);
    let row_frac = fract(row_f);

    let norm_col_x = (col_i + 0.5) / num_cols;
    let energy = sample_column_energy(norm_col_x);
    let amp_level = energy.x;
    let peak_level = energy.y;

    // Segment bevel borders inside each LED block
    let pad_x = 0.14;
    let pad_y = 0.20;
    let is_led_body = col_frac >= pad_x && col_frac <= (1.0 - pad_x) && row_frac >= pad_y && row_frac <= (1.0 - pad_y);

    let led_norm_y = (row_i + 0.5) / num_leds;
    let is_lit = led_norm_y <= amp_level;
    let is_peak_cap = abs(led_norm_y - peak_level) < (0.85 / num_leds) && peak_level > 0.05;

    let base_led_col = get_led_color(led_norm_y);

    // Bevel shading for tactile hardware depth
    let center_dx = abs(col_frac - 0.5) / (0.5 - pad_x);
    let center_dy = abs(row_frac - 0.5) / (0.5 - pad_y);
    let bevel = 1.0 - 0.25 * max(center_dx, center_dy);

    var color = vec3<f32>(0.008, 0.010, 0.014); // Dark chassis background

    if is_led_body {
        if is_peak_cap {
            // Bright white-hot floating peak hold cap
            color = vec3<f32>(1.8, 1.9, 2.0) * bevel;
        } else if is_lit {
            // Lit LED segment with core intensity
            color = base_led_col * 1.35 * bevel;
        } else {
            // Unlit dark LED slot with subtle phosphor outline
            color = base_led_col * 0.045 + vec3<f32>(0.012, 0.014, 0.018) * bevel;
        }
    } else {
        // Inter-LED gap with subtle bloom bleed from lit neighbors
        if is_lit || is_peak_cap {
            color = base_led_col * 0.12;
        }
    }

    // ACES tonemapping for rich HDR glow
    color = aces_tonemap(color);

    // Studio CRT scanlines & phosphor vignette
    var crt = get_default_crt();
    crt.scanline_intensity = 0.18;
    crt.noise_intensity = 0.008;
    crt.vignette_scale = 1.45;
    crt.vignette_softness = 0.85;
    crt.phosphor_tint = vec3<f32>(0.96, 1.02, 0.98);

    color = apply_crt_effects(color, in.uv, in.clip_position.xy, audio.smooth_time, crt);

    return vec4<f32>(color, 1.0);
}
