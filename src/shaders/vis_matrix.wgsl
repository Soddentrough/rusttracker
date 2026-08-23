// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

// --- Helper Functions for Procedural Randomness ---

fn hash11(p: f32) -> f32 {
    let p_fract = fract(p * 0.1031);
    let p_scaled = p_fract * (p_fract + 33.33);
    return fract(p_scaled * (p_scaled + p_scaled));
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash31(p: vec3<f32>) -> f32 {
    var p3 = fract(p * 0.1031);
    p3 = p3 + dot(p3, p3.zyx + 31.32);
    return fract((p3.x + p3.y) * p3.z);
}

fn bellrand(seed: f32, range: f32) -> f32 {
    let a = hash11(seed * 7.13);
    let b = hash11(seed * 13.71);
    let c = hash11(seed * 31.37);
    return (a + b + c) / 3.0 * range;
}

// --- Procedural 3x5 Bitmap Font Table ---
// Authentic Matrix rain character set: half-width katakana + digits.

fn matrix_glyph_bitmap(ch: u32) -> u32 {
    switch ch {
        case 0u  { return 31599u; } // 0
        case 1u  { return 11415u; } // 1
        case 2u  { return 29671u; } // 2
        case 3u  { return 29647u; } // 3
        case 4u  { return 23497u; } // 4
        case 5u  { return 31183u; } // 5
        case 6u  { return 31215u; } // 6
        case 7u  { return 29330u; } // 7
        case 8u  { return 31727u; } // 8
        case 9u  { return 31695u; } // 9
        case 10u { return 11860u; } // ｱ
        case 11u { return 17557u; } // ｲ
        case 12u { return 12110u; } // ｳ
        case 13u { return 29847u; } // ｴ
        case 14u { return 11988u; } // ｵ
        case 15u { return 7764u; }  // ｶ
        case 16u { return 11962u; } // ｷ
        case 17u { return 17609u; } // ｸ
        case 18u { return 17866u; } // ｹ
        case 19u { return 29263u; } // ｺ
        case 20u { return 11927u; } // ｻ
        case 21u { return 20558u; } // ｼ
        case 22u { return 29333u; } // ｽ
        case 23u { return 12073u; } // ｾ
        case 24u { return 20564u; } // ｿ
        case 25u { return 10954u; } // ﾀ
        case 26u { return 30162u; } // ﾁ
        case 27u { return 21518u; } // ﾂ
        case 28u { return 29140u; } // ﾃ
        case 29u { return 18852u; } // ﾄ
        case 30u { return 11924u; } // ﾅ
        case 31u { return 28679u; } // ﾆ
        case 32u { return 21973u; } // ﾇ
        case 33u { return 11946u; } // ﾈ
        case 34u { return 4756u; }  // ﾉ
        case 35u { return 23040u; } // ﾊ
        case 36u { return 19311u; } // ﾋ
        case 37u { return 29332u; } // ﾌ
        case 38u { return 2560u; }  // ﾍ
        case 39u { return 30061u; } // ﾎ
        case 40u { return 29397u; } // ﾏ
        case 41u { return 24966u; } // ﾐ
        case 42u { return 9464u; }  // ﾑ
        case 43u { return 23188u; } // ﾒ
        case 44u { return 30163u; } // ﾓ
        case 45u { return 20114u; } // ﾔ
        case 46u { return 13760u; } // ﾕ
        case 47u { return 29647u; } // ﾖ
        case 48u { return 29134u; } // ﾗ
        case 49u { return 23401u; } // ﾘ
        case 50u { return 23403u; } // ﾙ
        case 51u { return 18797u; } // ﾚ
        case 52u { return 31599u; } // ﾛ
        case 53u { return 31087u; } // ﾜ
        case 54u { return 16468u; } // ﾝ
        default  { return 0u; }     // space
    }
}

fn draw_matrix_char(glyph_idx: u32, cell_frac: vec2<f32>) -> f32 {
    let margin_x = 0.12;
    let margin_y = 0.12;
    if cell_frac.x < margin_x || cell_frac.x > 1.0 - margin_x || cell_frac.y < margin_y || cell_frac.y > 1.0 - margin_y {
        return 0.0;
    }
    let col = u32(floor((cell_frac.x - margin_x) / (1.0 - 2.0 * margin_x) * 3.0));
    let row = u32(floor((cell_frac.y - margin_y) / (1.0 - 2.0 * margin_y) * 5.0));
    
    if col >= 3u || row >= 5u { return 0.0; }
    let bit = (4u - row) * 3u + (2u - col);
    return f32((matrix_glyph_bitmap(glyph_idx) >> bit) & 1u);
}

// --- Audio Data Sampling Helpers ---

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

fn get_channel_vu(ch_idx: u32) -> f32 {
    let vec_idx = ch_idx / 4u;
    let comp_idx = ch_idx % 4u;
    if vec_idx >= 8u { return 0.0; }
    let v = audio.channels[vec_idx];
    if comp_idx == 0u { return v.x; }
    if comp_idx == 1u { return v.y; }
    if comp_idx == 2u { return v.z; }
    return v.w;
}

fn get_column_energy(norm_x: f32, col_i: u32) -> f32 {
    // Map normalized screen X (0..1) logarithmically across spectrum bins (1..180)
    let bin_f = pow(norm_x, 1.25) * 160.0 + 1.0;
    let bin0 = u32(clamp(floor(bin_f), 0.0, 255.0));
    let bin1 = bin0 + 1u;
    let frac = fract(bin_f);
    let spec_val = mix(get_spectrum_bin(bin0), get_spectrum_bin(bin1), frac);

    // If tracker/spatial multichannel is playing, also blend corresponding channel VU
    var energy = spec_val * 0.9;
    if audio.num_channels > 2u {
        let ch_idx = col_i % audio.num_channels;
        let ch_vu = get_channel_vu(ch_idx);
        energy = mix(energy, ch_vu * 2.5, 0.45);
    } else if audio.num_channels == 2u {
        let ch_idx = select(0u, 1u, norm_x > 0.5);
        let ch_vu = get_channel_vu(ch_idx);
        energy = mix(energy, ch_vu * 2.0, 0.35);
    }
    return clamp(energy, 0.0, 3.5);
}

const CELL_ASPECT: f32 = 1.45; // Height-to-width ratio for code cells

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = max(audio.aspect_ratio, 0.1);
    let time = audio.smooth_time;

    // --- Global Frequency Analysis ---
    let bass = clamp(max(get_spectrum_bin(0u), get_spectrum_bin(1u)) * 1.3 + get_spectrum_bin(2u) * 0.7, 0.0, 2.5);
    let mid = clamp((get_spectrum_bin(8u) + get_spectrum_bin(16u) + get_spectrum_bin(28u)) * 0.4, 0.0, 2.0);
    let treble = clamp(max(get_spectrum_bin(48u), get_spectrum_bin(80u)) * 1.6, 0.0, 2.5);

    // --- Ambient Cyberspace Background & Sub-Bass Depth Haze ---
    let center_uv = in.uv - vec2<f32>(0.5, 0.5);
    let center_dist = length(center_uv * vec2<f32>(aspect, 1.0));
    let vignette = smoothstep_r(1.6, 0.2, center_dist);

    var ambient_bg = vec3<f32>(0.003, 0.018, 0.008) * (1.0 + bass * 0.8) * vignette;

    // Floor impact bounce glow (bottom of screen)
    let floor_glow = smoothstep(0.75, 1.0, in.uv.y) * vec3<f32>(0.01, 0.06, 0.02) * (1.0 + bass * 1.5);
    ambient_bg += floor_glow;

    var total_color = ambient_bg;

    // --- 3-Layer Parallax Digital Rain Curtain ---
    // Layer 0: Distant background (dense, small, slow drift, soft emerald)
    // Layer 1: Midground (medium, balanced speed, strong frequency response)
    // Layer 2: Foreground (large, fast, white-hot laser heads, intense bloom)
    for (var layer = 0; layer < 3; layer = layer + 1) {
        var base_cols = 40.0;
        var layer_density = 0.55;
        var layer_speed_base = 3.5;
        var layer_speed_react = 4.0;
        var layer_alpha_mult = 1.0;
        var color_tint = vec3<f32>(0.04, 0.85, 0.15);

        if layer == 0 {
            // Distant Background
            base_cols = 60.0;
            layer_density = 0.65;
            layer_speed_base = 2.0;
            layer_speed_react = 2.0;
            layer_alpha_mult = 0.35;
            color_tint = vec3<f32>(0.015, 0.40, 0.08);
        } else if layer == 1 {
            // Midground
            base_cols = 44.0;
            layer_density = 0.55;
            layer_speed_base = 3.8;
            layer_speed_react = 4.5;
            layer_alpha_mult = 0.70;
            color_tint = vec3<f32>(0.05, 0.88, 0.18);
        } else {
            // Foreground Focus
            base_cols = 28.0;
            layer_density = 0.40;
            layer_speed_base = 6.0;
            layer_speed_react = 7.0;
            layer_alpha_mult = 1.0;
            color_tint = vec3<f32>(0.12, 1.0, 0.28);
        }

        let num_cols = floor(base_cols * aspect);
        let num_rows = max(10.0, floor(num_cols / (aspect * CELL_ASPECT)));

        // Grid coordinates with sub-layer parallax offsets
        let offset_x = hash11(f32(layer) * 51.37) * 0.7;
        let offset_y = hash11(f32(layer) * 83.19) * 0.5;
        let grid_x = in.uv.x * num_cols + offset_x;
        let grid_y = in.uv.y * num_rows + offset_y;

        let col_i = u32(floor(grid_x));
        let row_i = floor(grid_y);
        let cell_frac = vec2<f32>(fract(grid_x), fract(grid_y));

        let norm_col_x = clamp(f32(col_i) / num_cols, 0.0, 1.0);
        let col_energy = get_column_energy(norm_col_x, col_i);

        let col_seed = f32(col_i) * 137.19 + f32(layer) * 283.41;

        // Density check: audio energy dynamically excites dormant columns
        let col_active_thresh = hash11(col_seed + 0.3);
        let active_boost = col_energy * 0.25 + bass * 0.15;
        if col_active_thresh > (layer_density + active_boost) {
            continue;
        }

        // Drop fall timing & speed
        let col_speed_rand = bellrand(col_seed + 1.7, 3.5);
        let drop_speed = layer_speed_base + col_speed_rand + col_energy * layer_speed_react;
        let drop_offset = hash11(col_seed + 3.1) * 300.0;

        // Stream cycle loop (falling down the screen)
        let trail_len = 6.0 + hash11(col_seed + 5.9) * 8.0 + col_energy * 8.0 + bass * 3.0;
        let cycle_len = num_rows + trail_len + 4.0;
        let stream_head = (time * drop_speed + drop_offset) % cycle_len;

        let head_y = stream_head - 2.0;
        let dist_to_head = head_y - row_i;

        // Visibility test for stream
        if dist_to_head < 0.0 || dist_to_head > trail_len || row_i < 0.0 || row_i >= num_rows {
            continue;
        }

        let is_lead_head = dist_to_head < 1.0;

        // --- Glyph Selection & Audio Glitch / Cipher Scramble ---
        var glyph_idx = 0u;
        if is_lead_head {
            // Rapidly spinning lead glyph
            let head_spin_rate = 12.0 + col_energy * 10.0 + treble * 15.0;
            let spin_step = floor(time * head_spin_rate);
            let g_hash = hash31(vec3<f32>(f32(col_i), f32(layer), spin_step));
            glyph_idx = u32(g_hash * 55.0);
        } else {
            // Stable trailing glyph with cipher scramble on treble transients
            let scramble_chance = hash21(vec2<f32>(f32(col_i) * 11.3 + row_i * 29.7, f32(layer)));
            let is_scrambling = (treble > 0.6 && col_energy > 0.8) || (scramble_chance < 0.03);

            if is_scrambling {
                let spin_rate = 8.0 + hash11(col_seed + 9.1) * 12.0;
                let spin_step = floor(time * spin_rate);
                let g_hash = hash31(vec3<f32>(f32(col_i), row_i, spin_step + f32(layer) * 13.0));
                glyph_idx = u32(g_hash * 55.0);
            } else {
                let stream_cycle = floor((time * drop_speed + drop_offset) / cycle_len);
                let g_hash = hash31(vec3<f32>(f32(col_i), row_i, stream_cycle * 17.0 + f32(layer) * 31.0));
                glyph_idx = u32(g_hash * 55.0);
            }
        }

        // Draw glyph shape from procedural font bitmap
        let glyph_shape = draw_matrix_char(glyph_idx, cell_frac);

        // Soft phosphor glow inside the character cell
        let cell_center_dist = length(cell_frac - vec2<f32>(0.5, 0.5));
        let cell_halo = smoothstep_r(0.65, 0.0, cell_center_dist) * 0.18;

        // --- Luminance & Reactive Color Output ---
        var cell_emission = vec3<f32>(0.0);

        if is_lead_head {
            // White-hot HDR lead laser head flaring intensely with audio energy
            let head_intensity = (2.2 + col_energy * 3.5 + bass * 1.5) * layer_alpha_mult;
            let head_color = mix(
                vec3<f32>(0.7, 1.0, 0.75),
                vec3<f32>(2.2, 3.2, 2.2),
                clamp(col_energy * 0.6 + bass * 0.4, 0.0, 1.0)
            );
            cell_emission = head_color * (glyph_shape * 1.2 + cell_halo * 1.8) * head_intensity;
        } else {
            // Exponential trail decay with vibrant audio-driven illumination pulses
            let decay_rate = 0.16 / (1.0 + col_energy * 0.4);
            let trail_decay = exp(-dist_to_head * decay_rate);
            let trail_pulse = (1.0 + col_energy * 1.4 + bass * 0.6) * layer_alpha_mult;

            let trail_brightness = (glyph_shape + cell_halo * trail_decay) * trail_decay * trail_pulse;
            cell_emission = color_tint * trail_brightness * 1.6;
        }

        total_color += cell_emission;
    }

    // --- High-Quality ACES Tonemapping (Soft HDR to SDR mapping) ---
    var final_color = aces_tonemap(total_color);

    // --- CRT Scanlines, Phosphor Noise & Vignette Post-Processing ---
    var crt_settings = get_default_crt();
    crt_settings.scanline_intensity = 0.20;
    crt_settings.noise_intensity = 0.012;
    crt_settings.flicker_intensity = 0.015;
    crt_settings.vignette_scale = 1.35;
    crt_settings.vignette_softness = 0.75;
    crt_settings.phosphor_tint = vec3<f32>(0.92, 1.05, 0.95);

    final_color = apply_crt_effects(final_color, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);

    return vec4<f32>(final_color, 1.0);
}

