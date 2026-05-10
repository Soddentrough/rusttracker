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
    display_order: array<vec4<u32>, 4>,
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

// Hash for analog noise
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Classic 8-color DOS demoscene fire palette (hard-stepped, no interpolation)
fn demoscene_palette(h: f32) -> vec3<f32> {
    if h < 0.04 { return vec3<f32>(0.0,  0.0,  0.0);  }  // black
    if h < 0.12 { return vec3<f32>(0.20, 0.02, 0.0);  }  // ember
    if h < 0.22 { return vec3<f32>(0.45, 0.04, 0.0);  }  // dark red
    if h < 0.35 { return vec3<f32>(0.75, 0.10, 0.0);  }  // red
    if h < 0.50 { return vec3<f32>(1.0,  0.25, 0.0);  }  // orange-red
    if h < 0.65 { return vec3<f32>(1.0,  0.55, 0.0);  }  // orange
    if h < 0.80 { return vec3<f32>(1.0,  0.85, 0.08); }  // yellow
    return vec3<f32>(1.0, 1.0, 0.45);                     // bright yellow
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // --- CRT barrel distortion ---
    let crt_uv = in.uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    let distorted = crt_uv * (1.0 + r2 * 0.08);
    let uv = distorted * 0.5 + 0.5;

    // Outside the CRT tube → pure black
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // --- Pixelate to virtual resolution (nearest-neighbor, chunky pixels) ---
    let virt_res = vec2<f32>(256.0, 144.0);
    let pixel_coord = floor(uv * virt_res);
    let pixel_uv = pixel_coord / virt_res;

    // Read heat from compute grid (nearest-neighbor, deliberately blocky)
    let tex_coord = vec2<i32>(
        clamp(i32(pixel_uv.x * 1024.0), 0, 1023),
        clamp(i32(pixel_uv.y * 576.0), 0, 575)
    );
    let heat = textureLoad(fire_grid_tex, tex_coord, 0).r;

    // Apply the hard-stepped palette
    var color = demoscene_palette(clamp(heat, 0.0, 1.0));

    // --- Pixel grid gap (dark lines between virtual pixels) ---
    let cell_frac = fract(uv * virt_res);
    let grid_x = smoothstep(0.0, 0.08, cell_frac.x) * smoothstep(1.0, 0.92, cell_frac.x);
    let grid_y = smoothstep(0.0, 0.08, cell_frac.y) * smoothstep(1.0, 0.92, cell_frac.y);
    color *= 0.7 + 0.3 * grid_x * grid_y;

    // --- CRT scanlines (prominent, every virtual pixel row) ---
    let scanline_phase = fract(uv.y * virt_res.y);
    let scanline = 0.65 + 0.35 * smoothstep(0.0, 0.35, scanline_phase)
                                 * smoothstep(1.0, 0.65, scanline_phase);
    color *= scanline;

    // --- RGB phosphor sub-pixel tint ---
    let sub_pixel = fract(uv.x * virt_res.x * 3.0);
    var phosphor: vec3<f32>;
    if sub_pixel < 0.333 {
        phosphor = vec3<f32>(1.3, 0.85, 0.85);
    } else if sub_pixel < 0.666 {
        phosphor = vec3<f32>(0.85, 1.3, 0.85);
    } else {
        phosphor = vec3<f32>(0.85, 0.85, 1.3);
    }
    color *= phosphor;

    // --- Vignette (darken edges of CRT) ---
    let vignette = smoothstep(1.5, 0.85, length(crt_uv));
    color *= vignette;

    // --- Analog noise / static ---
    let noise = hash12(in.clip_position.xy + fract(audio.smooth_time) * 137.0);
    color += vec3<f32>(noise * 0.025 * vignette);

    // --- CRT flicker (subtle 60Hz-ish wobble) ---
    let flicker = 0.97 + 0.03 * sin(audio.smooth_time * 377.0);
    color *= flicker;

    // --- Bezel edge glow (faint amber reflection near corners) ---
    let bezel_dist = max(abs(crt_uv.x) - 0.85, 0.0) + max(abs(crt_uv.y) - 0.85, 0.0);
    let bezel_glow = exp(-bezel_dist * 30.0) * 0.03;
    color += vec3<f32>(1.0, 0.6, 0.2) * bezel_glow * clamp(heat * 3.0, 0.0, 1.0);

    return vec4<f32>(color, 1.0);
}
