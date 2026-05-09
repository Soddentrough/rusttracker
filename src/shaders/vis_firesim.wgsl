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

    // --- UV perturbation (gentle, for macro shape only) ---
    let uv_scaled = vec2<f32>(in.uv.x * 1.77, in.uv.y);
    let edge_noise = fbm(uv_scaled * 4.0 - vec2<f32>(0.0, audio.time * 0.6));
    let gentle_dist = edge_noise * 0.025 * (1.0 - in.uv.y);
    let sample_uv = in.uv + vec2<f32>(gentle_dist, 0.0);

    let base_heat = get_base_heat(sample_uv);

    // --- Ragged edge technique: add noise TO the heat BEFORE thresholding ---
    // This pushes the fire boundary around irregularly.
    // Where noise is positive, fire extends outward; where negative, it retreats.
    // This is fundamentally different from multiplicative noise (which only changes brightness).
    
    // Medium-frequency tears (large ragged chunks)
    let tear_noise = fbm(uv_scaled * 8.0 - vec2<f32>(0.0, audio.time * 2.0));
    // Fine wisps at the very edge (small tendrils)
    let wisp_noise = perlin_noise(uv_scaled * 22.0 - vec2<f32>(0.0, audio.time * 3.5));
    
    // Scale the noise contribution by height — more ragged at flame tips, smoother at base
    let tip_factor = clamp(1.0 - in.uv.y * 1.3, 0.0, 1.0);
    let noisy_heat = base_heat + tear_noise * 0.18 * tip_factor + wisp_noise * 0.08 * tip_factor;

    // Tight smoothstep: sharp cutoff from fire to void with the noisy boundary
    let sharp_heat = smoothstep(0.05, 0.45, noisy_heat) * pow(max(base_heat, 0.0), 0.7);

    // --- Subtle multiplicative texture (light touch — edge noise does the heavy lifting) ---
    let turb_speed = 1.5 + mids * 1.5;
    let crackle = perlin_noise(uv_scaled * 30.0 - vec2<f32>(0.0, audio.time * turb_speed)) * 0.5 + 0.5;
    var final_heat = sharp_heat * (0.88 + 0.18 * crackle);

    // Bass-driven brightness surge
    final_heat *= 1.0 + bass * 0.5;

    // --- Embers in negative space ---
    var ember_glow = 0.0;
    if base_heat < 0.3 && in.uv.y < 0.90 {
        let ember_speed = 2.0 + highs * 3.0;
        let ember_uv = uv_scaled * 20.0 - vec2<f32>(0.0, audio.time * ember_speed);
        let ember_noise = fbm(ember_uv);
        let proximity = get_base_heat(in.uv + vec2<f32>(0.0, 0.05));
        let threshold = 0.6 - highs * 0.15;
        if ember_noise > threshold {
            ember_glow = smoothstep(threshold, 1.0, ember_noise) * proximity * 6.0;
        }
    }

    // --- Per-channel spatial FFT reactivity ---
    let n_ch = max(1u, audio.num_channels);
    let channel_idx = min(u32(in.uv.x * f32(n_ch)), n_ch - 1u);

    var fft_ch = channel_idx;
    var vu_scale = 1.0;
    if audio.fft_channels < n_ch {
        fft_ch = channel_idx % max(audio.fft_channels, 1u);
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

    // --- Final color ---
    let fire_color = blackbody(final_heat) * (1.0 + volume * 0.4);

    // High-frequency spectral tint (subtle blue in hot zones)
    let tint = vec3<f32>(0.2, 0.35, 0.9) * (high_energy * final_heat * 2.5);

    // Embers
    let ember_color = vec3<f32>(1.0, 0.5, 0.1) * ember_glow * (1.0 + highs * 3.0);

    // Soft bloom halo around bright areas
    let bloom_intensity = max(final_heat - 0.6, 0.0) * 2.0;
    let bloom = vec3<f32>(1.0, 0.6, 0.2) * bloom_intensity * 0.25 * (1.0 + bass * 0.5);

    let final_color = fire_color + tint + ember_color + bloom;

    return vec4<f32>(final_color, 1.0);
}
