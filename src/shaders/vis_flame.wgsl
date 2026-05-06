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
    spectrum: array<vec4<f32>, 128>,
    fire_heat: array<vec4<f32>, 128>,
    channels: array<vec4<f32>, 8>,
    channel_peaks: array<vec4<f32>, 8>,
    num_channels: u32,
    mode: u32,
    time: f32,
    duration: f32,
    smooth_time: f32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
    ui_meters_rect: vec4<f32>,
    ui_heatmap_rect: vec4<f32>,
    ui_fire_rect: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> waveform_history: array<vec4<f32>>;

// Simplex noise functions for fire
fn hash(p: vec2<f32>) -> f32 {
    let p3  = fract(vec3<f32>(p.xyx) * 0.1031);
    var p3_mut = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3_mut.x + p3_mut.y) * p3_mut.z);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i + vec2<f32>(0.0, 0.0)), 
                   hash(i + vec2<f32>(1.0, 0.0)), u.x),
               mix(hash(i + vec2<f32>(0.0, 1.0)), 
                   hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var pp = p;
    let rot = mat2x2<f32>(0.87758, 0.47942, -0.47942, 0.87758);
    for (var i = 0; i < 5; i = i + 1) {
        v = v + a * noise(pp);
        pp = rot * pp * 2.0 + vec2<f32>(100.0, 100.0);
        a = a * 0.5;
    }
    return v;
}

fn get_fire_heat(x: f32) -> f32 {
    if (x < 0.0 || x >= 1.0) {
        return 0.0;
    }
    let freq_idx = u32(x * 512.0);
    let clamped_idx = clamp(freq_idx, 0u, 511u);
    let vec_idx = clamped_idx / 4u;
    let component_idx = clamped_idx % 4u;
    
    let spec_vec = audio.fire_heat[vec_idx];
    if component_idx == 0u { return spec_vec.x; }
    else if component_idx == 1u { return spec_vec.y; }
    else if component_idx == 2u { return spec_vec.z; }
    else { return spec_vec.w; }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    
    // Extract global energies
    var bass_energy = 0.0;
    for (var i = 0u; i < 2u; i = i + 1u) { // Average first 8 bands
        let v = audio.spectrum[i];
        bass_energy += (v.x + v.y + v.z + v.w);
    }
    bass_energy = clamp(bass_energy / (8.0 * 100.0), 0.0, 1.0);

    var treble_energy = 0.0;
    for (var i = 25u; i < 30u; i = i + 1u) { // Average some upper bands
        let v = audio.spectrum[i];
        treble_energy += (v.x + v.y + v.z + v.w);
    }
    treble_energy = clamp(treble_energy / (20.0 * 100.0), 0.0, 1.0);

    var volume = 0.0;
    let n_ch = min(audio.num_channels, 32u);
    for (var i = 0u; i < n_ch; i = i + 1u) {
        let vec_idx = i / 4u;
        let comp = i % 4u;
        let v = audio.channels[vec_idx];
        if (comp == 0u) { volume += v.x; }
        else if (comp == 1u) { volume += v.y; }
        else if (comp == 2u) { volume += v.z; }
        else { volume += v.w; }
    }
    if (n_ch > 0u) {
        volume = clamp(volume / f32(n_ch), 0.0, 1.0);
    }

    let y = 1.0 - in.uv.y;

    // Smooth out the incoming local heat
    let fuel1 = get_fire_heat(in.uv.x - 0.005) / 100.0;
    let fuel2 = get_fire_heat(in.uv.x) / 100.0;
    let fuel3 = get_fire_heat(in.uv.x + 0.005) / 100.0;
    let local_heat = clamp((fuel1 + fuel2 * 2.0 + fuel3) / 4.0, 0.0, 1.0);
    
    // Fuel combines global bass foundation with local frequency spikes
    let fuel = clamp(bass_energy * 0.8 + local_heat * 0.6, 0.0, 1.0);
    
    // Wind based on treble
    let wind = treble_energy * 3.0;
    let x_drift = wind * y;
    let px = in.uv.x + x_drift;
    
    // Base shape tapers off vertically
    // Height is modulated by overall volume!
    let height_mod = mix(1.5, 0.3, volume); 
    let base_mask = pow(1.0 - y, height_mod);
    
    // Two layers of noise moving at different speeds for organic licks
    let t = audio.smooth_time * 1.5;
    let n1 = fbm(vec2<f32>(px * 12.0, y * 6.0 - t * 1.8));
    let n2 = fbm(vec2<f32>(px * 25.0 - t * 0.5, y * 12.0 - t * 3.0));
    let noise_mask = (n1 * 0.65 + n2 * 0.35);
    
    // Intensity combines fuel, vertical falloff, and noise.
    let intensity = (fuel * 1.5 + 0.1) * noise_mask * base_mask * 2.5;
    let final_heat = clamp(intensity - (y * 0.8), 0.0, 1.2);
    
    // Procedural Fire Gradient Mapping
    let color_smoke    = vec3<f32>(0.02, 0.02, 0.03);
    let color_dark_red = vec3<f32>(0.5, 0.05, 0.0);
    let color_orange   = vec3<f32>(1.0, 0.35, 0.0);
    let color_yellow   = vec3<f32>(1.0, 0.85, 0.1);
    let color_white    = vec3<f32>(1.0, 1.0, 1.0);
    
    var color = color_smoke;
    color = mix(color, color_dark_red, smoothstep(0.05, 0.3, final_heat));
    color = mix(color, color_orange,   smoothstep(0.3,  0.55, final_heat));
    color = mix(color, color_yellow,   smoothstep(0.55, 0.8, final_heat));
    color = mix(color, color_white,    smoothstep(0.8,  1.0, final_heat));
    
    // Embers / Sparks
    let spark_t = audio.smooth_time * 2.5;
    let spark_n = fbm(vec2<f32>(px * 60.0 + spark_t * 0.2, y * 40.0 - spark_t));
    let spark_mask = smoothstep(0.85, 1.0, spark_n) * smoothstep(0.0, fuel + 0.3, 1.0 - y);
    let spark_color = vec3<f32>(1.0, 0.7, 0.2) * spark_mask * 2.0;
    
    color = color + spark_color;
    
    return vec4<f32>(color, 1.0);
}
