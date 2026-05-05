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
    _pad0: u32,
    _pad1: u32,
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



@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // 512 bins across X axis
    let freq_idx = u32(in.uv.x * 512.0);
    let clamped_idx = clamp(freq_idx, 0u, 511u);
    
    let vec_idx = clamped_idx / 4u;
    let component_idx = clamped_idx % 4u;
    
    let spec_vec = audio.spectrum[vec_idx];
    var amplitude = 0.0;
    if component_idx == 0u { amplitude = spec_vec.x; }
    else if component_idx == 1u { amplitude = spec_vec.y; }
    else if component_idx == 2u { amplitude = spec_vec.z; }
    else { amplitude = spec_vec.w; }
    
    // Create bars with small gaps
    let bar_uv_x = fract(in.uv.x * 512.0);
    if bar_uv_x < 0.1 || bar_uv_x > 0.9 {
        return vec4<f32>(0.02, 0.02, 0.03, 1.0); // Gap color
    }
    
    // Calculate aspect ratio dynamically using screen-space derivatives!
    // dpdx(uv.x) = 1.0 / window_width
    // dpdy(uv.y) = 1.0 / window_height
    // This allows us to make perfectly square cells that adapt to any window size.
    let aspect = dpdx(in.uv.x) / dpdy(in.uv.y);
    let num_leds = 512.0 * aspect;
    
    let led_y = fract((1.0 - in.uv.y) * num_leds);
    if led_y < 0.15 || led_y > 0.85 {
        return vec4<f32>(0.02, 0.02, 0.03, 1.0); // LED gap
    }
    
    let intensity = amplitude; // 0.0 to 100.0
    var color = vec3<f32>(0.0);
    
    if (1.0 - in.uv.y) < (intensity / 100.0) {
        color = val_to_color(intensity);
    } else {
        color = vec3<f32>(0.05, 0.05, 0.06); // LED off
    }
    
    return vec4<f32>(color, 1.0);
}
