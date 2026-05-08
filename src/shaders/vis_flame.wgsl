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

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> waveform_history: array<vec4<f32>>;

@group(0) @binding(3) var fire_grid_tex: texture_2d<f32>;

// --- Bilinear heat sampling (eliminates blocky stair-stepping) ---

fn get_heat(uv: vec2<f32>) -> f32 {
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // We smooth out the grid by sampling continuously
    let heat = get_heat(in.uv);
    
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
