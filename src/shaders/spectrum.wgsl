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
    channels: array<vec4<f32>, 8>,
    num_channels: u32,
    mode: u32,
    time: f32,
    _pad2: u32,
};

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

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
        // --- MODE 1: FIRE EFFECT ---
        let amplitude = get_amplitude(in.uv.x);
        let heat = clamp(amplitude / 100.0, 0.0, 1.0);
        let y = 1.0 - in.uv.y;
        
        // Base shape of fire based on frequency amplitude
        let shape = smoothstep(heat + 0.1, heat - 0.2, y);
        
        // Flowing noise
        let t = audio.time * 3.0;
        let n = fbm(vec2<f32>(in.uv.x * 15.0, y * 8.0 - t));
        
        // Combine shape and noise. The fire "licks" upwards
        let final_heat = shape * n * 2.0;
        
        var color = vec3<f32>(0.0, 0.0, 0.0);
        if final_heat > 0.8 {
            color = vec3<f32>(1.0, 1.0, 1.0); // White
        } else if final_heat > 0.5 {
            color = vec3<f32>(1.0, 0.8, 0.1); // Yellow
        } else if final_heat > 0.2 {
            color = vec3<f32>(1.0, 0.3, 0.0); // Orange
        } else if final_heat > 0.05 {
            color = vec3<f32>(0.6, 0.05, 0.0); // Dark red
        } else if final_heat > 0.01 {
            color = vec3<f32>(0.1, 0.1, 0.12); // Smoke
        }
        
        return vec4<f32>(color, 1.0);
    } 
    else {
        // --- MODE 2: AMBER CRT ---
        // Apply radial distortion
        let crt_uv = in.uv * 2.0 - 1.0;
        let r2 = dot(crt_uv, crt_uv);
        let distorted_uv = crt_uv * (1.0 + r2 * 0.15); // Curve
        let final_uv = distorted_uv * 0.5 + 0.5;
        
        if final_uv.x < 0.0 || final_uv.x > 1.0 || final_uv.y < 0.0 || final_uv.y > 1.0 {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        
        let amplitude = get_amplitude(final_uv.x);
        let y = 1.0 - final_uv.y;
        let target_h = amplitude / 100.0;
        
        var intensity = 0.0;
        if y < target_h {
            intensity = 1.0;
        } else {
            // Soft glow falloff above the bar
            intensity = 0.05 / (y - target_h + 0.05);
            intensity = clamp(intensity, 0.0, 0.5);
        }
        
        let vignette = smoothstep(1.5, 0.2, length(distorted_uv));
        
        // Scanlines
        let aspect = dpdx(in.uv.x) / dpdy(in.uv.y);
        let scanlines = sin(final_uv.y * 400.0 * aspect) * 0.15 + 0.85;
        
        // CRT Flicker
        let flicker = sin(audio.time * 60.0) * 0.03 + 0.97;
        
        let amber = vec3<f32>(1.0, 0.65, 0.1);
        let final_color = amber * intensity * vignette * scanlines * flicker;
        
        return vec4<f32>(final_color, 1.0);
    }
}
