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

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn get_waveform_raw(hist_idx: u32, idx: u32) -> f32 {
    let clamped_idx = clamp(idx, 0u, 1023u);
    let vec_idx = clamped_idx / 4u;
    let component_idx = clamped_idx % 4u;
    
    let spec_vec = waveform_history[hist_idx * 256u + vec_idx];
    if component_idx == 0u { return spec_vec.x; }
    else if component_idx == 1u { return spec_vec.y; }
    else if component_idx == 2u { return spec_vec.z; }
    else { return spec_vec.w; }
}

fn get_waveform_smooth(hist_idx: u32, idx: i32) -> f32 {
    let v0 = get_waveform_raw(hist_idx, u32(clamp(idx - 1, 0, 1023)));
    let v1 = get_waveform_raw(hist_idx, u32(clamp(idx, 0, 1023)));
    let v2 = get_waveform_raw(hist_idx, u32(clamp(idx + 1, 0, 1023)));
    return (v0 + v1 * 2.0 + v2) / 4.0;
}

fn get_waveform_interpolated(hist_idx: u32, x: f32) -> f32 {
    let float_idx = clamp(x, 0.0, 0.999) * 1023.0;
    let idx0 = u32(float_idx);
    let idx1 = min(idx0 + 1u, 1023u);
    let fract_part = fract(float_idx);
    
    let val0 = get_waveform_raw(hist_idx, idx0);
    let val1 = get_waveform_raw(hist_idx, idx1);
    
    return mix(val0, val1, fract_part);
}

fn sdLine(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn get_wave_dist(hist_idx: u32, uv: vec2<f32>, aspect: f32) -> f32 {
    let clamped_x = clamp(uv.x, 0.0, 0.999);
    let float_idx = clamped_x * 1023.0;
    let idx = u32(float_idx);
    
    var min_dist = 1000.0;
    
    // Check local neighborhood to ensure proper line joints
    let start_idx = max(0i, i32(idx) - 1);
    let end_idx = min(1022i, i32(idx) + 1);
    
    let p = vec2<f32>(uv.x * aspect, uv.y);
    
    for (var i = start_idx; i <= end_idx; i = i + 1) {
        let u_idx0 = u32(i);
        let u_idx1 = u_idx0 + 1u;
        
        let x0 = f32(u_idx0) / 1023.0;
        let x1 = f32(u_idx1) / 1023.0;
        
        let v0 = get_waveform_smooth(hist_idx, i32(u_idx0));
        let v1 = get_waveform_smooth(hist_idx, i32(u_idx1));
        
        let y0 = v0 * 0.4 + 0.5;
        let y1 = v1 * 0.4 + 0.5;
        
        let a = vec2<f32>(x0 * aspect, y0);
        let b = vec2<f32>(x1 * aspect, y1);
        
        let d = sdLine(p, a, b);
        min_dist = min(min_dist, d);
    }
    
    return min_dist;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Apply radial distortion
    let crt_uv = in.uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    let distorted_uv = crt_uv * (1.0 + r2 * 0.05); // Less curve to fill window better
    let final_uv = distorted_uv * 0.5 + 0.5;
    
    let aspect = dpdx(in.uv.x) / dpdy(in.uv.y);
    var final_color = vec3<f32>(0.0);
    
    let r = length(crt_uv);
    let edge_blur = smoothstep(0.2, 1.5, r);
    
    // Updated color to deeper orange amber
    let amber = vec3<f32>(1.0, 0.45, 0.05);
    
    var wave_intensity = 0.0;
    
    // Accumulate 15 history frames for ghosting (frames older than 15 are fully decayed)
    for (var i = 45u; i < 60u; i = i + 1u) {
        let true_dist = get_wave_dist(i, final_uv, aspect);
        
        // True exponential decay (e^-kx) like a real CRT phosphor
        let frames_old = 59.0 - f32(i);
        let age = exp(-frames_old * 0.8); 
        
        // Softer, thicker core for more blur
        // Adds realistic lens defocus at the edges of the screen
        let thickness = 0.008 + edge_blur * 0.015; 
        let core = smoothstep(thickness, 0.0, true_dist);
        
        // Stronger, wider bloom
        let bloom = 0.0015 / (true_dist * true_dist + 0.0005) * 0.3;
        
        // Halation (soft wider glow simulating glass scattering)
        let halation = exp(-true_dist * 20.0) * 0.2;
        
        let frame_intensity = (core + bloom + halation) * age;
        
        wave_intensity = wave_intensity + frame_intensity;
    }
    
    // Reinhard-like tonemapping
    let mapped_intensity = wave_intensity / (1.0 + wave_intensity * 0.5);
    
    final_color = final_color + amber * mapped_intensity;
    
    // Soft vignette (darkens corners realistically without hard cutoffs)
    let vignette = smoothstep(1.8, 0.4, r);
    
    // Smoothly fade out the edges where the UVs distort past the signal boundaries
    // This prevents the clamped flat-line artifact at the extreme curved edges
    let screen_bounds = smoothstep(0.0, 0.03, final_uv.x) * smoothstep(1.0, 0.97, final_uv.x) *
                        smoothstep(0.0, 0.05, final_uv.y) * smoothstep(1.0, 0.95, final_uv.y);
    
    // Analog Signal Noise (Hash based on pixel coordinates and time)
    // Using in.clip_position.xy (gl_FragCoord) ensures true high-frequency noise and fixes the diagonal banding!
    let noise_val = hash21(in.clip_position.xy + fract(audio.smooth_time) * 100.0);
    
    // Only apply noise primarily to darker areas of the screen
    let noise_color = amber * noise_val * 0.05 * (1.0 - mapped_intensity) * vignette;
    
    final_color = final_color * vignette * screen_bounds;
    final_color = final_color + noise_color;
    
    return vec4<f32>(final_color, 1.0);
}
