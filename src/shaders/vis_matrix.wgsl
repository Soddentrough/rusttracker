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
// (At 3x5 resolution ﾖ/3 and ﾛ/0 share shapes, as they do in the film's font.)

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
    let margin_x = 0.15;
    let margin_y = 0.15;
    if cell_frac.x < margin_x || cell_frac.x > 1.0 - margin_x || cell_frac.y < margin_y || cell_frac.y > 1.0 - margin_y {
        return 0.0;
    }
    let col = u32(floor((cell_frac.x - margin_x) / (1.0 - 2.0 * margin_x) * 3.0));
    let row = u32(floor((cell_frac.y - margin_y) / (1.0 - 2.0 * margin_y) * 5.0));
    
    if col >= 3u || row >= 5u { return 0.0; }
    let bit = (4u - row) * 3u + (2u - col);
    return f32((matrix_glyph_bitmap(glyph_idx) >> bit) & 1u);
}

// --- Active Channel VU Meter Lookup ---

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

fn get_channel_phase(ch_idx: u32) -> f32 {
    let vec_idx = ch_idx / 4u;
    let comp_idx = ch_idx % 4u;
    if vec_idx >= 8u { return 0.0; }
    let v = audio.channel_phases[vec_idx];
    if comp_idx == 0u { return v.x; }
    if comp_idx == 1u { return v.y; }
    if comp_idx == 2u { return v.z; }
    return v.w;
}

// --- Constants ---

const CELL_ASPECT: f32 = 1.5; // Taller than wide (height / width)

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Audio reactivities
    let bass = max(audio.spectrum[0].x, audio.spectrum[1].x) * 1.5;
    let mid = (audio.spectrum[8].x + audio.spectrum[16].x + audio.spectrum[24].x) * 0.5;
    let treble = max(audio.spectrum[32].x, audio.spectrum[64].x) * 2.0;

    let time = audio.smooth_time * 0.8;
    var color = vec3<f32>(0.0);

    // Loop over 3 depth layers (Background, Midground, Foreground) to simulate 3D depth
    // with strictly controlled column density to prevent overlapping character clutter.
    for (var layer = 0; layer < 3; layer = layer + 1) {
        var scale = 1.0;
        var layer_density = 0.05;
        var layer_audio_mult = 1.0;
        var color_tint = vec3<f32>(0.05, 1.0, 0.05);

        if layer == 0 {
            // Background: tiny, dense, reacts to treble/high frequencies
            scale = 1.8; // 45 columns
            layer_density = 0.07;
            layer_audio_mult = 0.3 + treble * 1.2;
            color_tint = vec3<f32>(0.02, 0.8, 0.02);
        } else if layer == 1 {
            // Midground: medium, reacts to mid frequencies (vocals/melody)
            scale = 1.4; // 35 columns
            layer_density = 0.05;
            layer_audio_mult = 0.5 + mid * 1.0;
            color_tint = vec3<f32>(0.04, 0.95, 0.04);
        } else {
            // Foreground: larger, sparse, reacts to bass hits
            scale = 1.0; // 25 columns
            layer_density = 0.03;
            layer_audio_mult = 0.6 + bass * 1.5;
            color_tint = vec3<f32>(0.08, 1.0, 0.08);
        }

        // Adapt grid to screen size so columns aren't stretched
        let num_cols = floor(25.0 * scale * audio.aspect_ratio);
        let num_rows = max(8.0, floor(num_cols / (audio.aspect_ratio * CELL_ASPECT)));

        // Compute layer UV coordinates
        var layer_uv = in.uv;
        let layer_offset_x = hash11(f32(layer) * 73.17) * 0.5;
        let layer_offset_y = hash11(f32(layer) * 91.31) * 0.3;
        layer_uv.x = (layer_uv.x - 0.5) * num_cols + num_cols * 0.5 + layer_offset_x;
        layer_uv.y = (layer_uv.y - 0.5) * num_rows + num_rows * 0.5 + layer_offset_y;

        let col = floor(layer_uv.x);
        let row = floor(layer_uv.y);
        let cell_frac = fract(layer_uv);

        // Procedural column hash seed
        let col_seed = col * 127.1 + f32(layer) * 311.7;

        // Density Check
        let col_active = hash11(col_seed + 0.5);
        if col_active > layer_density { continue; }

        // Associate column with a tracker channel (guard: no channels when no file loaded)
        let ch_idx = u32(abs(col)) % max(audio.num_channels, 1u);
        let ch_vu = get_channel_vu(ch_idx);

        // Falling speed: must be strictly constant to prevent vertical jitter
        let strip_speed = 2.0 + bellrand(col_seed + 1.0, 6.0);
        let strip_offset = hash11(col_seed + 2.0) * 200.0;

        // Cycle phases (draw, erase, gap)
        let draw_length = num_rows;
        let erase_speed_ratio = 0.5;
        let erase_length = num_rows / erase_speed_ratio;
        let gap_length = 4.0 + hash11(col_seed + 5.0) * 20.0;
        let cycle_length = draw_length + erase_length + gap_length;

        // Linear progression over time using per-channel CPU phase integration
        let ch_phase = get_channel_phase(ch_idx) * 0.8;
        let cycle_pos = (ch_phase * strip_speed + strip_offset) % cycle_length;

        var spinner_y = 0.0;
        var visible = false;
        var is_spinner = false;

        if cycle_pos < draw_length {
            spinner_y = cycle_pos;
            visible = row >= 0.0 && row <= spinner_y && row < num_rows;
            is_spinner = abs(row - floor(spinner_y)) < 1.0;
        } else if cycle_pos < draw_length + erase_length {
            let erase_pos = (cycle_pos - draw_length) * erase_speed_ratio;
            spinner_y = erase_pos;
            visible = row >= 0.0 && row > erase_pos && row < num_rows;
            is_spinner = false;
        }

        if !visible { continue; }

        // Glyph selection
        var glyph_idx = 0u;

        if is_spinner {
            // Leading spinner glyph cycles rapidly, glitching on treble transients
            let spin_time = time * 15.0 + treble * 8.0;
            let glyph_hash = hash31(vec3<f32>(col, f32(layer), floor(spin_time)));
            glyph_idx = u32(glyph_hash * 55.0);
        } else {
            // Mostly static, but occasional slow flips
            let spin_chance = hash21(vec2<f32>(col * 17.3 + row * 31.7, f32(layer)));
            if spin_chance < 0.04 {
                let spin_rate = 2.0 + hash21(vec2<f32>(col, row + f32(layer) * 100.0)) * 4.0;
                let spin_time = time * spin_rate;
                let glyph_hash = hash31(vec3<f32>(col, row, floor(spin_time) + f32(layer)));
                glyph_idx = u32(glyph_hash * 55.0);
            } else {
                let cycle = floor((ch_phase * strip_speed + strip_offset) / cycle_length);
                let glyph_hash = hash31(vec3<f32>(col, row, cycle + f32(layer) * 7.0));
                glyph_idx = u32(glyph_hash * 55.0);
            }
        }

        // Draw glyph shape
        let glyph_shape = draw_matrix_char(glyph_idx, cell_frac);
        if glyph_shape < 0.05 { continue; }

        // Brightness and decay calculation
        var brightness = 1.0;

        // Tail decay relative to drawing head position
        let head_y = select(cycle_pos, num_rows, cycle_pos >= draw_length);
        if is_spinner {
            // Audio intensity only affects the leading spinner head, keeping the trails stable
            let spinner_reactive = 1.0 + ch_vu * 1.5 + (layer_audio_mult - 1.0) * 1.2;
            brightness = brightness * 1.8 * spinner_reactive;
        } else {
            let dist_to_head = head_y - row;
            if dist_to_head > 0.0 {
                // Static decay rate to keep trailing characters consistently green without oversaturating to white
                brightness = brightness * exp(-dist_to_head * 0.15);
            }
        }

        let alpha = glyph_shape * brightness;

        var cell_color = vec3<f32>(0.0);
        if is_spinner {
            // White-hot lead head reacting to beats
            cell_color = vec3<f32>(0.7, 1.0, 0.7) * alpha;
        } else {
            // Classic decaying green trail (completely stable and clean)
            cell_color = color_tint * alpha;
        }

        color = color + cell_color;
    }

    // Clamp values before post-processing
    color = min(color, vec3<f32>(1.0));

    // Apply retro CRT post-processing (scanlines, vignette, screen noise)
    var crt_settings = get_default_crt();
    crt_settings.scanline_intensity = 0.25;
    crt_settings.noise_intensity = 0.015;
    crt_settings.flicker_intensity = 0.02;

    color = apply_crt_effects(color, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);

    return vec4<f32>(color, 1.0);
}
