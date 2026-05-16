// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

@group(0) @binding(1) var<storage, read> waveform_history: array<f32>;
@group(0) @binding(3) var fire_grid_tex: texture_2d<f32>;
@group(0) @binding(4) var<storage, read> multi_spectrum: array<vec2<f32>>;

// INCLUDE: glyph_font

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
    // 1. Read Raw Physics Simulation State
    let phys_val = get_base_heat(uv);
    
    // Positive values = Heat (Fire & Embers). Negative values = Smoke density.
    let base_heat = max(phys_val, 0.0);
    
    // Procedural edge erosion!
    // The physics simulation provides the distinct columns and vertical advection.
    // We use noise to erode the *edges* of the flame, leaving the core solid, which 
    // creates perfect, crisp plasma filaments!
    let detail_uv = uv_c * 12.0 + vec2<f32>(0.0, time * 4.0);
    let flame_noise = fbm(detail_uv) * 0.5 + 0.5;
    
    // High erosion at the edges (base_heat=0), low erosion at the core (base_heat=1)
    let erosion = mix(0.85, 0.05, smoothstep(0.0, 0.8, base_heat));
    let heat = base_heat * (1.0 - flame_noise * erosion);
    
    // Smoke density is negative values.
    // Removed ambient haze so the background is clean.
    let smoke_density = max(-phys_val, 0.0);
    
    // 2. Render Fire & Embers (Emission)
    var emission = blackbody(min(heat, 1.0) * 1.15);
    
    // Embers: Because the physics engine is now sharpened, embers (heat > 1.2) do not 
    // blur into fireballs. They remain perfectly crisp, physical dots riding the fluid.
    let ember_glow = smoothstep(1.5, 4.0, heat);
    emission += vec3<f32>(1.0, 0.9, 0.4) * ember_glow * 8.0;

    // Per-channel spatial FFT reactivity for core tint
    let n_ch = max(1u, audio.num_channels);
    var lfe_idx = 999u;
    if (n_ch == 6u || n_ch == 8u || n_ch == 16u) && audio.num_spatial_channels == n_ch { lfe_idx = 3u; }
    
    var n_spatial_ch = n_ch;
    if lfe_idx < n_ch { n_spatial_ch = n_ch - 1u; }
    
    let hover_spatial_idx = min(u32(uv.x * f32(n_spatial_ch)), n_spatial_ch - 1u);
    
    var hover_display_idx = 0u;
    var spatial_idx = 0u;
    for (var i = 0u; i < n_ch; i = i + 1u) {
        if i == lfe_idx { continue; }
        if spatial_idx == hover_spatial_idx {
            hover_display_idx = i;
            break;
        }
        spatial_idx = spatial_idx + 1u;
    }

    var raw_ch = audio.display_order[hover_display_idx / 4u][hover_display_idx % 4u];
    var fft_ch = raw_ch;
    var vu_scale = 1.0;
    if audio.fft_channels < n_ch {
        fft_ch = raw_ch % max(audio.fft_channels, 1u);
        let vec_idx = hover_display_idx / 4u;
        let elem_idx = hover_display_idx % 4u;
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
    let tint = vec3<f32>(0.1, 0.35, 1.0) * (high_energy * smoothstep(0.4, 0.8, heat) * 1.8);
    emission += tint;

    // 3. Render Smoke (Beer's Law)
    // Transmission through the physically simulated smoke. Reduced coefficient for lighter smoke.
    let absorption_coeff = 2.0; 
    let transmittance = exp(-smoke_density * absorption_coeff);
    
    // In-scattering: Ambient light and fire glow scattered by smoke
    let scatter_color = mix(vec3<f32>(0.04, 0.05, 0.08), vec3<f32>(1.0, 0.3, 0.05), pow(uv.y, 3.0));
    let in_scattering = scatter_color * (1.0 - transmittance) * 1.5;

    // 4. Post-Processing: Bloom and Halation
    let halation_intensity = smoothstep(0.15, 0.55, min(heat, 1.0)) * 0.4;
    let halation = vec3<f32>(1.0, 0.1, 0.02) * halation_intensity;
    
    let bloom_intensity = smoothstep(0.4, 0.8, min(heat, 1.0)) * 0.6;
    let bloom = vec3<f32>(1.0, 0.85, 0.25) * bloom_intensity;

    // 5. Final Composition
    // Fire is the light source; it should not be blocked by its own smoke in this additive model.
    var final_color = emission + in_scattering + halation + bloom;
    
    // ACES Filmic Tonemapping
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    final_color = (final_color * (a * final_color + b)) / (final_color * (c * final_color + d) + e);

    // --- Draw Debug Labels for Channels ---
    if (uv.y > 0.95) {
        let channel_width = 1.0 / f32(max(n_spatial_ch, 1u));
        let center_x = (f32(hover_spatial_idx) + 0.5) * channel_width;
        
        let center_xc = center_x * 1.77;
        let uv_c = vec2<f32>(uv.x * 1.77, uv.y);
        
        var lbl = array<u32, 3>(19u, 19u, 19u);
        var lbl_len = 1u;
        
        if audio.num_spatial_channels < n_ch {
            let ch_num = raw_ch + 1u;
            if ch_num < 10u {
                lbl[0] = ch_num;
                lbl_len = 1u;
            } else {
                lbl[0] = ch_num / 10u;
                lbl[1] = ch_num % 10u;
                lbl_len = 2u;
            }
        } else {
            switch raw_ch {
                case 0u  { lbl[0]=10u; }                                // L
                case 1u  { lbl[0]=11u; }                                // R
                case 2u  { lbl[0]=12u; }                                // C
                case 3u  { lbl[0]=10u; lbl[1]=14u; lbl[2]=15u; lbl_len=3u; } // LFE
                case 4u  { lbl[0]=10u; lbl[1]=13u; lbl_len=2u; }       // LS
                case 5u  { lbl[0]=11u; lbl[1]=13u; lbl_len=2u; }       // RS
                case 6u  { lbl[0]=10u; lbl[1]=11u; lbl[2]=13u; lbl_len=3u; } // LRS
                case 7u  { lbl[0]=11u; lbl[1]=11u; lbl[2]=13u; lbl_len=3u; } // RRS
                case 8u  { lbl[0]=10u; lbl[1]=18u; lbl_len=2u; }       // LW (Left Wide)
                case 9u  { lbl[0]=11u; lbl[1]=18u; lbl_len=2u; }       // RW (Right Wide)
                case 10u { lbl[0]=10u; lbl[1]=16u; lbl[2]=14u; lbl_len=3u; } // LTF (Left Top Front)
                case 11u { lbl[0]=11u; lbl[1]=16u; lbl[2]=14u; lbl_len=3u; } // RTF (Right Top Front)
                case 12u { lbl[0]=10u; lbl[1]=16u; lbl[2]=17u; lbl_len=3u; } // LTM (Left Top Middle)
                case 13u { lbl[0]=11u; lbl[1]=16u; lbl[2]=17u; lbl_len=3u; } // RTM (Right Top Middle)
                case 14u { lbl[0]=10u; lbl[1]=16u; lbl[2]=11u; lbl_len=3u; } // LTR (Left Top Rear)
                case 15u { lbl[0]=11u; lbl[1]=16u; lbl[2]=11u; lbl_len=3u; } // RTR (Right Top Rear)
                default  { }
            }
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
