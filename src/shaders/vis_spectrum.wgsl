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

fn get_amplitude(x: f32) -> f32 {
    let freq_idx = u32(x * 1024.0);
    let clamped_idx = clamp(freq_idx, 0u, 1023u);
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
    let amplitude = get_amplitude(in.uv.x);
    
    let bar_uv_x = fract(in.uv.x * 1024.0);
    if bar_uv_x < 0.1 || bar_uv_x > 0.9 {
        return vec4<f32>(0.02, 0.02, 0.03, 1.0);
    }
    
    let aspect = dpdx(in.uv.x) / dpdy(in.uv.y);
    let num_leds = 1024.0 * aspect;
    
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
