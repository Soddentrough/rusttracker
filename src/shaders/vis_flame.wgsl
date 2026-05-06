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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Prevent floating-point precision loss over hours of playback
    let t = (audio.smooth_time % 10000.0) * 1.5; 
    let ember_t = (audio.smooth_time % 10000.0) * 2.5;
    
    // Y coordinates: 0.0 at the bottom, 1.0 at the top
    let y = 1.0 - in.uv.y;
    
    // --- 1. AUDIO ANALYSIS ---
    var bass_energy = 0.0;
    for (var i = 0u; i < 2u; i = i + 1u) {
        let v = audio.spectrum[i];
        bass_energy += (v.x + v.y + v.z + v.w);
    }
    bass_energy = clamp(bass_energy / (8.0 * 100.0), 0.0, 1.0);

    var treble_energy = 0.0;
    for (var i = 25u; i < 30u; i = i + 1u) {
        let v = audio.spectrum[i];
        treble_energy += (v.x + v.y + v.z + v.w);
    }
    treble_energy = clamp(treble_energy / (20.0 * 100.0), 0.0, 1.0);

    // --- 2. MULTI-CHANNEL FUEL MAP ---
    // Distribute channels across the X axis
    let n_ch = max(1u, min(audio.num_channels, 32u));
    let channel_width = 1.0 / f32(n_ch);
    
    var local_fuel = 0.0;
    for (var i = 0u; i < n_ch; i = i + 1u) {
        // Calculate the center x-coordinate for this channel
        let center_x = (f32(i) + 0.5) * channel_width;
        let dist = abs(in.uv.x - center_x);
        
        let vec_idx = i / 4u;
        let comp = i % 4u;
        let ch_vu = audio.channels[vec_idx][comp];
        
        // Create an overlapping gaussian-like bell curve for each channel
        let spread = channel_width * 1.2;
        let influence = smoothstep(spread, 0.0, dist);
        
        local_fuel += ch_vu * influence;
    }
    
    // Add a baseline of fuel from the global bass energy so the fire never completely dies if there's audio
    local_fuel = clamp(local_fuel * 1.5 + bass_energy * 0.2, 0.0, 1.0);
    
    // --- 3. PROCEDURAL COOLING MAP ---
    // We simulate the "cooling map" of fluid fire by generating upward-scrolling noise.
    
    // Wind drift based on treble
    let wind = treble_energy * 1.5;
    let x_drift = wind * y;
    
    // Convective acceleration: noise moves faster at the top
    let convection_speed = 1.0 + y * 2.0; 
    let convective_y = y * 6.0 - t * convection_speed;
    
    // Turbulence field
    let turbulence = 0.5 + treble_energy * 1.0; 
    let warp_uv = vec2<f32>(in.uv.x * 4.0 + x_drift, convective_y * 0.5);
    let warp_dx = fbm(warp_uv + vec2<f32>(t * 0.2, 0.0)) * 2.0 - 1.0;
    let warp_dy = fbm(warp_uv + vec2<f32>(100.0, t * 0.2)) * 2.0 - 1.0;
    let warp_offset = vec2<f32>(warp_dx, warp_dy) * turbulence;
    
    // Read the cooling map (noise)
    let px = in.uv.x * 12.0 + x_drift + warp_offset.x * 2.0;
    let py = convective_y + warp_offset.y * 2.0;
    let cooling_noise = fbm(vec2<f32>(px, py));
    
    // Calculate flame height based on fuel
    // Higher fuel -> fire reaches higher before cooling map extinguishes it
    let height_multiplier = mix(4.0, 0.5, local_fuel); // Y falloff multiplier
    let cooling = (y * height_multiplier) * (cooling_noise * 1.5 + 0.5);
    
    // Subtract cooling from fuel (classic algorithm)
    let heat = max(local_fuel * 1.2 - cooling, 0.0);
    
    // --- 4. COLOR AND SMOKE MAPPING ---
    let color_bg       = vec3<f32>(0.0, 0.0, 0.0);
    let color_smoke    = vec3<f32>(0.05, 0.05, 0.06);
    let color_dark_red = vec3<f32>(0.8, 0.1, 0.0);
    let color_orange   = vec3<f32>(1.0, 0.5, 0.0);
    let color_yellow   = vec3<f32>(1.0, 0.9, 0.2);
    let color_white    = vec3<f32>(1.0, 1.0, 1.0);
    
    // Non-linear heat curve to emphasize bright cores
    let h = pow(heat, 1.2);
    
    var color = color_bg;
    
    // Smoke generation: Smoke appears where heat has died off but was recently hot
    // We use a broader, slower noise for smoke
    let smoke_py = (y * 4.0) - t * 0.5;
    let smoke_noise = fbm(vec2<f32>(in.uv.x * 6.0 + x_drift * 2.0, smoke_py));
    let smoke_density = smoothstep(0.0, 0.6, smoke_noise) * smoothstep(0.8, 0.0, heat) * smoothstep(0.0, 0.5, y) * smoothstep(0.0, 0.5, local_fuel);
    
    color = mix(color_bg, color_smoke, smoke_density);
    
    // Fire color ramp
    color = mix(color, color_dark_red, smoothstep(0.05, 0.3, h));
    color = mix(color, color_orange,   smoothstep(0.3,  0.6, h));
    color = mix(color, color_yellow,   smoothstep(0.6,  0.85, h));
    color = mix(color, color_white,    smoothstep(0.85, 1.0, h));
    
    // --- 5. EMBERS & SPARKS ---
    // Embers follow the turbulence but move faster
    let spark_px = in.uv.x * 60.0 + x_drift * 5.0 + warp_offset.x * 8.0;
    let spark_py = (y * 40.0) - ember_t * (convection_speed * 1.2) + warp_offset.y * 8.0;
    
    let spark_n = fbm(vec2<f32>(spark_px, spark_py));
    
    // Twinkle modulation
    let twinkle_phase = (in.uv.x * 123.4 + y * 456.7);
    let twinkle = sin(audio.time * 30.0 + twinkle_phase) * 0.5 + 0.5;
    
    // Embers only appear in or just above the fire (guided by fuel and y)
    // They burst more when treble is high
    let spark_burst = treble_energy * 0.5;
    let spark_mask = smoothstep(0.85 - spark_burst, 1.0, spark_n) * smoothstep(0.0, local_fuel + 0.3, 1.0 - y) * twinkle;
    
    let spark_color = vec3<f32>(1.0, 0.8, 0.4) * spark_mask * 2.5;
    
    color = color + spark_color;
    
    return vec4<f32>(color, 1.0);
}
