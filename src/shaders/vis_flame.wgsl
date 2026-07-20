// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;


@group(0) @binding(1)
var<storage, read> waveform_history: array<f32>;

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
    let uv = crt_distort_uv(in.uv, 0.03);

    // Hard check slightly outside the tube to save performance
    if uv.x < -0.015 || uv.x > 1.015 || uv.y < -0.015 || uv.y > 1.015 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // Soft bezel/vignette boundary fade (blends/blurs the hard edges nicely)
    let bezel_fade_x = smoothstep(0.0, 0.015, uv.x) * smoothstep_r(1.0, 0.985, uv.x);
    let bezel_fade_y = smoothstep(0.0, 0.015, uv.y) * smoothstep_r(1.0, 0.985, uv.y);
    let bezel = bezel_fade_x * bezel_fade_y;

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
    let grid_x = smoothstep(0.0, 0.08, cell_frac.x) * smoothstep_r(1.0, 0.92, cell_frac.x);
    let grid_y = smoothstep(0.0, 0.08, cell_frac.y) * smoothstep_r(1.0, 0.92, cell_frac.y);
    color *= 0.7 + 0.3 * grid_x * grid_y;

    // --- CRT scanlines (prominent, every virtual pixel row) ---
    let scanline_phase = fract(uv.y * virt_res.y);
    let scanline = 0.65 + 0.35 * smoothstep(0.0, 0.35, scanline_phase)
                                 * smoothstep_r(1.0, 0.65, scanline_phase);
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

    // --- Bezel edge glow (faint amber reflection near corners) ---
    let crt_uv = uv * 2.0 - 1.0;
    let bezel_dist = max(abs(crt_uv.x) - 0.85, 0.0) + max(abs(crt_uv.y) - 0.85, 0.0);
    let bezel_glow = exp(-bezel_dist * 30.0) * 0.03;
    color += vec3<f32>(1.0, 0.6, 0.2) * bezel_glow * clamp(heat * 3.0, 0.0, 1.0);

    var crt_settings = get_default_crt();
    crt_settings.scanline_intensity = 0.0; // Use vis_flame's custom scanlines instead
    crt_settings.vignette_scale = 1.5;
    crt_settings.vignette_softness = 0.85;
    crt_settings.noise_intensity = 0.025;
    crt_settings.flicker_intensity = 0.03;
    color = apply_crt_effects(color, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);

    // Apply soft bezel edge fade
    color *= bezel;

    return vec4<f32>(color, 1.0);
}
