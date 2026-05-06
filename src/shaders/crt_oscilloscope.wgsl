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

fn val_to_color(val: f32) -> vec3<f32> {
    let v = clamp(val, 0.0, 100.0);
    if v < 5.0 {
        return vec3<f32>(0.08, 0.08, 0.1);
    } else if v < 20.0 {
        return vec3<f32>(0.31, 0.08, 0.08);
    } else if v < 40.0 {
        return vec3<f32>(0.7, 0.12, 0.08);
    } else if v < 60.0 {
        return vec3<f32>(1.0, 0.39, 0.08);
    } else if v < 85.0 {
        return vec3<f32>(1.0, 0.78, 0.2);
    } else {
        return vec3<f32>(1.0, 1.0, 1.0);
    }
}

// Simplex noise functions for fire
fn hash(p: vec2<f32>) -> f32 {
    let p3  = fract(vec3<f32>(p.xyx) * 0.1031);
    var p3_mut = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3_mut.x + p3_mut.y) * p3_mut.z);
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
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

fn get_amplitude(x: f32) -> f32 {
    let freq_idx = u32(x * 512.0);
    let clamped_idx = clamp(freq_idx, 0u, 511u);
    let vec_idx = clamped_idx / 4u;
    let component_idx = clamped_idx % 4u;
    
    let spec_vec = audio.spectrum[vec_idx];
    if component_idx == 0u { return spec_vec.x; }
    else if component_idx == 1u { return spec_vec.y; }
    else if component_idx == 2u { return spec_vec.z; }
    else { return spec_vec.w; }
}

fn get_fire_heat(x: f32) -> f32 {
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
    
    if audio.mode == 0u {
        // --- MODE 0: DEFAULT GRID ---
        let amplitude = get_amplitude(in.uv.x);
        
        let bar_uv_x = fract(in.uv.x * 512.0);
        if bar_uv_x < 0.1 || bar_uv_x > 0.9 {
            return vec4<f32>(0.02, 0.02, 0.03, 1.0);
        }
        
        let aspect = dpdx(in.uv.x) / dpdy(in.uv.y);
        let num_leds = 512.0 * aspect;
        
        let led_y = fract((1.0 - in.uv.y) * num_leds);
        if led_y < 0.15 || led_y > 0.85 {
            return vec4<f32>(0.02, 0.02, 0.03, 1.0);
        }
        
        if (1.0 - in.uv.y) < (amplitude / 100.0) {
            return vec4<f32>(val_to_color(amplitude), 1.0);
        } else {
            return vec4<f32>(0.05, 0.05, 0.06, 1.0);
        }
    } 
    else if audio.mode == 1u {
        // --- MODE 1: PROCEDURAL COMBUSTION ---
        // Smooth out the incoming heat (fuel) by sampling slightly around it
        let fuel1 = get_fire_heat(in.uv.x - 0.005) / 100.0;
        let fuel2 = get_fire_heat(in.uv.x) / 100.0;
        let fuel3 = get_fire_heat(in.uv.x + 0.005) / 100.0;
        let fuel = clamp((fuel1 + fuel2 * 2.0 + fuel3) / 4.0, 0.0, 1.0);
        
        let y = 1.0 - in.uv.y;
        
        // Base shape tapers off vertically
        let base_mask = pow(1.0 - y, 2.0);
        
        // Two layers of noise moving at different speeds for organic licks
        let t = audio.time * 1.5;
        let n1 = fbm(vec2<f32>(in.uv.x * 12.0, y * 6.0 - t * 1.8));
        let n2 = fbm(vec2<f32>(in.uv.x * 25.0 - t * 0.5, y * 12.0 - t * 3.0));
        let noise_mask = (n1 * 0.65 + n2 * 0.35);
        
        // Intensity combines fuel, vertical falloff, and noise.
        // We push the flame upwards but tear it apart with noise.
        let intensity = (fuel * 1.2 + 0.1) * noise_mask * base_mask * 2.5;
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
        let spark_t = audio.time * 2.5;
        let spark_n = fbm(vec2<f32>(in.uv.x * 60.0 + spark_t * 0.2, y * 40.0 - spark_t));
        // Sparks appear in high noise areas, and fade out towards the top or where fuel is zero
        let spark_mask = smoothstep(0.85, 1.0, spark_n) * smoothstep(0.0, fuel + 0.3, 1.0 - y);
        let spark_color = vec3<f32>(1.0, 0.7, 0.2) * spark_mask * 2.0;
        
        // Add sparks on top of the fire
        color = color + spark_color;
        
        return vec4<f32>(color, 1.0);
    } 
    else {
        // --- MODE 2: NEW ORANGE AMBER CRT ---
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
}
