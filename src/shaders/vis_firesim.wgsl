struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let u = f32((in_vertex_index << 1u) & 2u);
    let v = f32(in_vertex_index & 2u);
    out.clip_position = vec4<f32>(u * 2.0 - 1.0, -(v * 2.0 - 1.0), 0.0, 1.0);
    out.uv = vec2<f32>(u, v);
    return out;
}

struct AudioUniforms {
    spectrum: array<vec4<f32>, 256>,
    fire_heat: array<vec4<f32>, 256>,
    channels: array<vec4<f32>, 8>,
    channel_peaks: array<vec4<f32>, 8>,
    spatial_channels: array<vec4<f32>, 4>,
    display_order: array<vec4<u32>, 4>,
    num_channels: u32,
    mode: u32,
    time: f32,
    duration: f32,
    smooth_time: f32,
    heatmap_row: u32,
    fft_channels: u32,
    num_spatial_channels: u32,
    ui_meters_rect: vec4<f32>,
    ui_heatmap_rect: vec4<f32>,
    ui_fire_rect: vec4<f32>,
};

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(1) var<storage, read> waveform_history: array<vec4<f32>>;
@group(0) @binding(3) var fire_grid_tex: texture_2d<f32>;
@group(0) @binding(4) var<storage, read> multi_spectrum: array<vec2<f32>>;

// --- 3x5 bitmap font for debug labels ---
fn glyph_bitmap(ch: u32) -> u32 {
    // 3x5 pixel font. 15 bits per glyph, MSB = top-left.
    // Index: 0-9=digits, 10=L, 11=R, 12=C, 13=S, 14=F, 15=E, 16=space
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
        case 10u { return 18727u; } // L: #.. #.. #.. #.. ###
        case 11u { return 31733u; } // R: ### #.# ### ##. #.#
        case 12u { return 31015u; } // C: ### #.. #.. #.. ###
        case 13u { return 31183u; } // S (same as 5)
        case 14u { return 31204u; } // F: ### #.. ### #.. #..
        case 15u { return 31207u; } // E: ### #.. ### #.. ###
        default  { return 0u; }     // space
    }
}

fn draw_label_char(ch: u32, frag: vec2<f32>, origin: vec2<f32>, px: f32) -> f32 {
    let local = frag - origin;
    if local.x < 0.0 || local.x >= px * 3.0 || local.y < 0.0 || local.y >= px * 5.0 { return 0.0; }
    let col = u32(floor(local.x / px));
    let row = u32(floor(local.y / px));
    let bit = (4u - row) * 3u + (2u - col);
    return f32((glyph_bitmap(ch) >> bit) & 1u);
}

// --- Noise primitives ---

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)),
                      dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453) * 2.0 - 1.0;
}

fn perlin_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    
    let a = dot(hash22(i + vec2<f32>(0.0, 0.0)), f - vec2<f32>(0.0, 0.0));
    let b = dot(hash22(i + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(hash22(i + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(hash22(i + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));
    
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var shift = vec2<f32>(100.0);
    var p2 = p;
    let rot = mat2x2<f32>(0.866, -0.5, 0.5, 0.866);
    for (var i = 0; i < 5; i = i + 1) {
        v += a * perlin_noise(p2);
        p2 = rot * p2 * 2.0 + shift;
        a *= 0.5;
    }
    return v;
}

// --- Bilinear heat sampling ---

fn get_base_heat(uv: vec2<f32>) -> f32 {
    let tex_size = vec2<f32>(1024.0, 576.0);
    let p = uv * tex_size - 0.5;
    let ip = floor(p);
    let fp = fract(p);

    let x0 = clamp(i32(ip.x), 0, 1023);
    let x1 = clamp(x0 + 1, 0, 1023);
    let y0 = clamp(i32(ip.y), 0, 575);
    let y1 = clamp(y0 + 1, 0, 575);

    let h00 = textureLoad(fire_grid_tex, vec2<i32>(x0, y0), 0).r;
    let h10 = textureLoad(fire_grid_tex, vec2<i32>(x1, y0), 0).r;
    let h01 = textureLoad(fire_grid_tex, vec2<i32>(x0, y1), 0).r;
    let h11 = textureLoad(fire_grid_tex, vec2<i32>(x1, y1), 0).r;

    return mix(mix(h00, h10, fp.x), mix(h01, h11, fp.x), fp.y);
}

// Blackbody radiation curve (physically motivated)
fn blackbody(temperature: f32) -> vec3<f32> {
    let t = clamp(temperature, 0.0, 1.0);
    let c1 = vec3<f32>(0.0, 0.0, 0.0);
    let c2 = vec3<f32>(0.5, 0.05, 0.0);
    let c3 = vec3<f32>(0.9, 0.25, 0.0);
    let c4 = vec3<f32>(1.0, 0.6, 0.05);
    let c5 = vec3<f32>(1.0, 0.85, 0.3);
    let c6 = vec3<f32>(1.0, 1.0, 0.85);
    
    var color = c1;
    color = mix(color, c2, smoothstep(0.0,  0.15, t));
    color = mix(color, c3, smoothstep(0.15, 0.30, t));
    color = mix(color, c4, smoothstep(0.30, 0.50, t));
    color = mix(color, c5, smoothstep(0.50, 0.75, t));
    color = mix(color, c6, smoothstep(0.75, 1.0,  t));
    return color;
}

// Sparse spectrum sampling
fn get_spectrum_val(idx: u32) -> f32 {
    let vec_idx = idx / 4u;
    let comp = idx % 4u;
    let v = audio.spectrum[vec_idx];
    if comp == 0u { return v.x; }
    else if comp == 1u { return v.y; }
    else if comp == 2u { return v.z; }
    else { return v.w; }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // --- Audio analysis ---
    var bass_sum = 0.0;
    for (var b = 0u; b < 64u; b += 8u) { bass_sum += get_spectrum_val(b); }
    let bass = clamp(bass_sum / 8.0 / 80.0, 0.0, 1.0);

    var mids_sum = 0.0;
    for (var b = 64u; b < 512u; b += 32u) { mids_sum += get_spectrum_val(b); }
    let mids = clamp(mids_sum / 14.0 / 80.0, 0.0, 1.0);

    var highs_sum = 0.0;
    for (var b = 512u; b < 1024u; b += 32u) { highs_sum += get_spectrum_val(b); }
    let highs = clamp(highs_sum / 16.0 / 60.0, 0.0, 1.0);

    let volume = bass * 0.5 + mids * 0.3 + highs * 0.2;

    let time = audio.time;
    let uv = in.uv;
    
    // Center UV and scale for aspect ratio
    let uv_c = vec2<f32>(uv.x * 1.77, uv.y);
    
    // 1. Direct Base Heat Sampling
    // The flame structure and fluid dynamics are entirely driven by the compute shader simulation.
    // Audio affects fuel/wind inside the compute shader, not via visual warping here.
    let base_heat = get_base_heat(uv);
    
    // 2. Procedural Noise Masking for Flame Detail
    // We add high-frequency detail to the boundaries of the simulated heat,
    // but the underlying structure remains stable.
    let detail_uv = uv_c * 12.0 - vec2<f32>(0.0, time * 3.5);
    let flame_detail = fbm(detail_uv);
    
    let core_mask = smoothstep(0.25, 0.75, base_heat);
    let edge_detail = flame_detail * (1.0 - core_mask);
    
    // Active heat forms the sharp, detailed flame boundaries
    let active_heat = smoothstep(0.1, 0.65, base_heat + edge_detail * 0.45);

    // 3. Smoke and Haze (Beer's Law)
    let smoke_uv = uv_c * 2.5 - vec2<f32>(time * 0.15, time * 1.0);
    let smoke_noise = fbm(smoke_uv);
    
    // Smoke appears where heat is dissipating (above flames)
    let smoke_proximity = smoothstep(0.0, 0.35, get_base_heat(uv + vec2<f32>(0.0, 0.15)));
    let smoke_density = smoothstep(0.3, 0.8, smoke_noise) * smoke_proximity * (1.0 - active_heat) * uv.y * 1.8;
    
    // Beer's Law: Transmission through smoke medium
    let absorption_coeff = 3.5; 
    let transmittance = exp(-smoke_density * absorption_coeff);
    
    // In-scattering: Ambient light and fire glow scattered by smoke
    let scatter_color = mix(vec3<f32>(1.0, 0.3, 0.05), vec3<f32>(0.1, 0.12, 0.18), uv.y);
    let in_scattering = scatter_color * smoke_density * (1.0 - transmittance) * 0.4;

    // 4. Emission (Blackbody Radiation)
    var emission = blackbody(active_heat * 1.15);

    // Per-channel spatial FFT reactivity
    let n_ch = max(1u, audio.num_channels);
    let channel_idx = min(u32(uv.x * f32(n_ch)), n_ch - 1u);

    var raw_ch = audio.display_order[channel_idx / 4u][channel_idx % 4u];
    var fft_ch = raw_ch;
    var vu_scale = 1.0;
    if audio.fft_channels < n_ch {
        fft_ch = raw_ch % max(audio.fft_channels, 1u);
        let vec_idx = channel_idx / 4u;
        let elem_idx = channel_idx % 4u;
        var val = 0.0;
        if elem_idx == 0u { val = audio.channels[vec_idx].x; }
        else if elem_idx == 1u { val = audio.channels[vec_idx].y; }
        else if elem_idx == 2u { val = audio.channels[vec_idx].z; }
        else { val = audio.channels[vec_idx].w; }
        vu_scale = max(val * 2.0, 0.05);
    }

    let offset = fft_ch * 1024u;
    var high_energy = 0.0;
    for (var b = 780u; b < 1000u; b = b + 5u) {
        let c = multi_spectrum[offset + b];
        high_energy += clamp(length(c) * 100.0, 0.0, 100.0);
    }
    high_energy = min((high_energy / 44.0) / 100.0 * vu_scale, 1.0);

    // High-frequency spectral tint (hot blue core)
    let tint = vec3<f32>(0.1, 0.35, 1.0) * (high_energy * active_heat * 1.8);
    emission += tint;

    // 5. Particle Systems (Sparks / Embers)
    let spark_speed = time * 2.0;
    let spark_pos = uv_c * vec2<f32>(18.0, 28.0) - vec2<f32>(0.0, spark_speed);
    
    // Add local noise to spark position so they wiggle as they rise organically
    let spark_wiggle = vec2<f32>(
        fbm(uv_c * 5.0 + vec2<f32>(0.0, time * 2.0)),
        fbm(uv_c * 5.0 - vec2<f32>(time * 1.5, 0.0))
    ) * 3.0;
    let displaced_spark_pos = spark_pos + spark_wiggle;
    
    let grid_uv = floor(displaced_spark_pos);
    let local_uv = fract(displaced_spark_pos);
    let spark_hash = hash22(grid_uv).x * 0.5 + 0.5; // [0, 1]
    
    // Add organic jitter to particle position within cell
    let jitter = hash22(grid_uv + vec2<f32>(13.3, 7.1)) * 0.35;
    let spark_dist = length(local_uv - 0.5 - jitter);
    
    // Only ~3% of cells spawn sparks
    let is_spark = step(0.97, spark_hash); 
    let spark_dot = smoothstep(0.2, 0.02, spark_dist) * is_spark;
    
    // Sparks only spawn where the compute shader simulation has heat
    let spark_proximity = smoothstep(0.05, 0.4, get_base_heat(uv + vec2<f32>(0.0, 0.05)));
    let spark_intensity = spark_dot * spark_proximity * (1.0 - active_heat) * 1.5;
    let spark_color = vec3<f32>(1.0, 0.65, 0.15) * spark_intensity * (4.0 + highs * 4.0);

    // 6. Post-Processing: Bloom and Halation
    let halation_intensity = smoothstep(0.15, 0.55, base_heat) * 0.4;
    let halation = vec3<f32>(1.0, 0.1, 0.02) * halation_intensity;
    
    let bloom_intensity = smoothstep(0.4, 0.8, active_heat) * 0.6;
    let bloom = vec3<f32>(1.0, 0.85, 0.25) * bloom_intensity;

    // 7. Final Composition (CGI Compositing Model)
    // Render equation: Color = Emission * Transmittance + In-Scattering
    var final_color = emission * transmittance + in_scattering;
    
    // Add foreground particles
    final_color += spark_color;
    
    // Add optical glow/bloom
    final_color += halation + bloom;
    
    // ACES Filmic Tonemapping
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    final_color = (final_color * (a * final_color + b)) / (final_color * (c * final_color + d) + e);

    // --- Draw Debug Labels for Channels ---
    if (uv.y > 0.95) {
        let n_ch = max(1u, audio.num_channels);
        let channel_width = 1.0 / f32(n_ch);
        
        let hover_idx = min(u32(uv.x * f32(n_ch)), n_ch - 1u);
        let center_x = (f32(hover_idx) + 0.5) * channel_width;
        
        let center_xc = center_x * 1.77;
        let uv_c = vec2<f32>(uv.x * 1.77, uv.y);
        
        let raw_ch = audio.display_order[hover_idx / 4u][hover_idx % 4u];
        
        var lbl = array<u32, 3>(16u, 16u, 16u);
        var lbl_len = 1u;
        switch raw_ch {
            case 0u  { lbl[0]=10u; }                                // L
            case 1u  { lbl[0]=11u; }                                // R
            case 2u  { lbl[0]=12u; }                                // C
            case 3u  { lbl[0]=10u; lbl[1]=14u; lbl[2]=15u; lbl_len=3u; } // LFE
            case 4u  { lbl[0]=10u; lbl[1]=13u; lbl_len=2u; }       // LS
            case 5u  { lbl[0]=11u; lbl[1]=13u; lbl_len=2u; }       // RS
            case 6u  { lbl[0]=10u; lbl[1]=11u; lbl[2]=13u; lbl_len=3u; } // LRS
            case 7u  { lbl[0]=11u; lbl[1]=11u; lbl[2]=13u; lbl_len=3u; } // RRS
            case 8u  { lbl[0]=10u; lbl[1]=14u; lbl_len=2u; }       // LF
            case 9u  { lbl[0]=11u; lbl[1]=14u; lbl_len=2u; }       // RF
            case 10u { lbl[0]=10u; lbl[1]=11u; lbl_len=2u; }       // LR
            case 11u { lbl[0]=11u; lbl[1]=11u; lbl_len=2u; }       // RR
            default  { }
        }
        
        let px_size = 0.003;
        let char_w = px_size * 4.0;
        let total_w = char_w * f32(lbl_len) - px_size;
        let label_origin = vec2<f32>(center_xc - total_w * 0.5, 0.97);
        
        let box_w = total_w + 0.01;
        let box_h = px_size * 5.0 + 0.01;
        let dx = abs(uv_c.x - center_xc);
        let dy = abs(uv_c.y - (0.97 + px_size * 2.5));
        
        if (dx < box_w * 0.5 && dy < box_h * 0.5) {
            final_color = vec3<f32>(0.0); // Black box
            var text_alpha = 0.0;
            for (var c_idx = 0u; c_idx < lbl_len; c_idx = c_idx + 1u) {
                let char_origin = label_origin + vec2<f32>(char_w * f32(c_idx), 0.0);
                text_alpha = max(text_alpha, draw_label_char(lbl[c_idx], uv_c, char_origin, px_size));
            }
            if (text_alpha > 0.0) {
                final_color = vec3<f32>(1.0); // White text
            }
        }
    }

    return vec4<f32>(final_color, 1.0);
}
