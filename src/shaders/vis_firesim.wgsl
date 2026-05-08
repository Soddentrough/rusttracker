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

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(1) var<storage, read> waveform_history: array<vec4<f32>>;
@group(0) @binding(3) var fire_grid_tex: texture_2d<f32>;

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

// Fractal Brownian Motion for high-frequency detail
fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var shift = vec2<f32>(100.0);
    var p2 = p;
    // Rotate to reduce axial artifacts
    let rot = mat2x2<f32>(0.866, -0.5, 0.5, 0.866);
    for (var i = 0; i < 4; i = i + 1) {
        v += a * perlin_noise(p2);
        p2 = rot * p2 * 2.0 + shift;
        a *= 0.5;
    }
    return v;
}

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

// Blackbody radiation curve approximation
fn blackbody(temperature: f32) -> vec3<f32> {
    let t = clamp(temperature, 0.0, 1.0);
    // Dark core mapped to reds, high heat mapped to bright yellow/white
    let c1 = vec3<f32>(0.0, 0.0, 0.0);
    let c2 = vec3<f32>(0.8, 0.1, 0.0);
    let c3 = vec3<f32>(1.0, 0.4, 0.0);
    let c4 = vec3<f32>(1.0, 0.8, 0.1);
    let c5 = vec3<f32>(1.0, 1.0, 1.0);
    
    var color = c1;
    color = mix(color, c2, smoothstep(0.0, 0.2, t));
    color = mix(color, c3, smoothstep(0.2, 0.5, t));
    color = mix(color, c4, smoothstep(0.5, 0.8, t));
    color = mix(color, c5, smoothstep(0.8, 1.0, t));
    return color;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Aspect ratio correction for noise
    let uv_scaled = vec2<f32>(in.uv.x * 1.77, in.uv.y);
    
    // UV Perturbation layer 1 (large slow licks)
    let n1 = fbm(uv_scaled * 4.0 - vec2<f32>(0.0, audio.time * 1.5));
    
    // UV Perturbation layer 2 (small fast tears)
    let n2 = fbm(uv_scaled * 8.0 - vec2<f32>(0.0, audio.time * 3.0));
    
    // Combine noise and distort UVs - horizontal distortion only for licks
    let distortion_x = (n1 * 0.6 + n2 * 0.4) * 0.15 * (1.0 - in.uv.y); 
    let distortion = vec2<f32>(distortion_x, 0.0);
    var sample_uv = in.uv + distortion;
    
    // Read the smooth compute shader simulation
    let base_heat = get_base_heat(sample_uv);
    
    // Increase crispness by applying a noise mask to the heat
    let crisp_mask = fbm(uv_scaled * 15.0 - vec2<f32>(0.0, audio.time * 5.0)) * 0.5 + 0.5;
    var final_heat = base_heat * (0.6 + 0.8 * crisp_mask);
    
    // Add glowing embers in negative space
    var ember_glow = 0.0;
    if (base_heat < 0.25 && in.uv.y < 0.9) {
        let ember_uv = uv_scaled * 30.0 - vec2<f32>(0.0, audio.time * 2.5);
        let ember_noise = fbm(ember_uv);
        if (ember_noise > 0.75) {
            let proximity = get_base_heat(in.uv + vec2<f32>(0.0, 0.05));
            ember_glow = smoothstep(0.75, 1.0, ember_noise) * proximity * 8.0;
        }
    }
    
    // Final color using blackbody
    let fire_color = blackbody(final_heat);
    let final_color = fire_color + vec3<f32>(1.0, 0.5, 0.1) * ember_glow;
    
    return vec4<f32>(final_color, 1.0);
}
