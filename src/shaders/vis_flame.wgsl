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

struct VisualizerStorage {
    history: array<array<f32, 64>, 120>,
    fire_grid: array<array<f32, 1024>, 144>,
};

@group(0) @binding(2)
var<storage, read> vis_storage: VisualizerStorage;

fn get_heat(x: f32, y: f32) -> f32 {
    let x_idx = clamp(u32(x * 1024.0), 0u, 1023u);
    let y_idx = clamp(u32(y * 144.0), 0u, 143u);
    return vis_storage.fire_grid[y_idx][x_idx];
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // We smooth out the grid by sampling continuously
    let heat = get_heat(in.uv.x, in.uv.y);
    
    let color_bg       = vec3<f32>(0.0, 0.0, 0.0);
    let color_dark_red = vec3<f32>(0.5, 0.0, 0.0);
    let color_red      = vec3<f32>(1.0, 0.1, 0.0);
    let color_orange   = vec3<f32>(1.0, 0.5, 0.0);
    let color_yellow   = vec3<f32>(1.0, 0.9, 0.1);
    let color_white    = vec3<f32>(1.0, 1.0, 1.0);
    
    var color = color_bg;
    
    if (heat > 0.0) {
        // Classic demoscene palette mapping
        let h = clamp(heat, 0.0, 1.0);
        
        var fire_color = color_dark_red;
        fire_color = mix(fire_color, color_red,    smoothstep(0.0,  0.2, h));
        fire_color = mix(fire_color, color_orange, smoothstep(0.2,  0.4, h));
        fire_color = mix(fire_color, color_yellow, smoothstep(0.4,  0.7, h));
        fire_color = mix(fire_color, color_white,  smoothstep(0.7,  1.0, h));
        
        color = fire_color;
    }
    
    return vec4<f32>(color, 1.0);
}
