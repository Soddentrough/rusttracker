// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> waveform_history: array<f32>;

@group(0) @binding(3) var fire_grid_tex: texture_2d<f32>;

// ============================================================================
// Canonical 37-color DOOM Fire Palette (Filipe Deschamps / PSX DOOM Reference)
// Color entries 0 to 36 mapping temperature index to RGB color:
// Black -> Dark Red -> Bright Orange -> Golden Yellow -> Incandescent White
// ============================================================================
fn doom_palette(h: f32) -> vec3<f32> {
    if (h < 0.005) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    let idx = clamp(u32(h * 36.0), 0u, 36u);
    var pal = array<vec3<f32>, 37>(
        vec3<f32>(0.027, 0.027, 0.027), // 0:  #070707
        vec3<f32>(0.122, 0.027, 0.027), // 1:  #1f0707
        vec3<f32>(0.184, 0.059, 0.027), // 2:  #2f0f07
        vec3<f32>(0.278, 0.059, 0.027), // 3:  #470f07
        vec3<f32>(0.341, 0.090, 0.027), // 4:  #571707
        vec3<f32>(0.404, 0.122, 0.027), // 5:  #671f07
        vec3<f32>(0.467, 0.122, 0.027), // 6:  #771f07
        vec3<f32>(0.561, 0.153, 0.027), // 7:  #8f2707
        vec3<f32>(0.624, 0.184, 0.027), // 8:  #9f2f07
        vec3<f32>(0.686, 0.247, 0.027), // 9:  #af3f07
        vec3<f32>(0.749, 0.278, 0.027), // 10: #bf4707
        vec3<f32>(0.780, 0.278, 0.027), // 11: #c74707
        vec3<f32>(0.875, 0.310, 0.027), // 12: #df4f07
        vec3<f32>(0.875, 0.341, 0.027), // 13: #df5707
        vec3<f32>(0.875, 0.341, 0.027), // 14: #df5707
        vec3<f32>(0.843, 0.373, 0.027), // 15: #d75f07
        vec3<f32>(0.843, 0.373, 0.027), // 16: #d75f07
        vec3<f32>(0.843, 0.404, 0.059), // 17: #d7670f
        vec3<f32>(0.812, 0.435, 0.059), // 18: #cf6f0f
        vec3<f32>(0.812, 0.467, 0.059), // 19: #cf770f
        vec3<f32>(0.812, 0.498, 0.059), // 20: #cf7f0f
        vec3<f32>(0.812, 0.529, 0.090), // 21: #cf8717
        vec3<f32>(0.780, 0.529, 0.090), // 22: #c78717
        vec3<f32>(0.780, 0.561, 0.090), // 23: #c78f17
        vec3<f32>(0.780, 0.592, 0.122), // 24: #c7971f
        vec3<f32>(0.749, 0.624, 0.122), // 25: #bf9f1f
        vec3<f32>(0.749, 0.624, 0.122), // 26: #bf9f1f
        vec3<f32>(0.749, 0.655, 0.153), // 27: #bfa727
        vec3<f32>(0.749, 0.655, 0.153), // 28: #bfa727
        vec3<f32>(0.749, 0.686, 0.184), // 29: #bfaf2f
        vec3<f32>(0.718, 0.686, 0.184), // 30: #b7af2f
        vec3<f32>(0.718, 0.718, 0.184), // 31: #b7b72f
        vec3<f32>(0.718, 0.718, 0.216), // 32: #b7b737
        vec3<f32>(0.812, 0.812, 0.435), // 33: #cfcf6f
        vec3<f32>(0.875, 0.875, 0.624), // 34: #dfdf9f
        vec3<f32>(0.937, 0.937, 0.780), // 35: #efefc7
        vec3<f32>(1.000, 1.000, 1.000)  // 36: #ffffff
    );
    return pal[idx];
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;

    // --- Aspect-Ratio-Adaptive Square Retro Pixels ---
    // Maintain uniform square chunky pixels matching classic DOOM height (~180 rows).
    // Dynamically scales column count with audio.aspect_ratio to fill the entire screen
    // without black borders, stretching, or letterboxing on any display geometry.
    let aspect = max(audio.aspect_ratio, 0.1);
    let v_height = 180.0;
    let v_width = max(floor(v_height * aspect), 1.0);
    let virt_res = vec2<f32>(v_width, v_height);

    let pixel_coord = floor(uv * virt_res);
    let pixel_uv = (pixel_coord + 0.5) / virt_res;

    // Sample temperature from simulation compute grid (1024 x 576)
    let tex_coord = vec2<i32>(
        clamp(i32(pixel_uv.x * 1024.0), 0, 1023),
        clamp(i32(pixel_uv.y * 576.0), 0, 575)
    );
    let heat = textureLoad(fire_grid_tex, tex_coord, 0).r;

    // Map heat intensity through the authentic 37-color DOOM palette
    var color = doom_palette(clamp(heat, 0.0, 1.0));

    // --- Retro Pixel Grid Separation ---
    // Fine dark gap between virtual pixels for sharp retro definition
    let cell_frac = fract(uv * virt_res);
    let grid_x = smoothstep(0.0, 0.07, cell_frac.x) * smoothstep_r(1.0, 0.93, cell_frac.x);
    let grid_y = smoothstep(0.0, 0.07, cell_frac.y) * smoothstep_r(1.0, 0.93, cell_frac.y);
    color *= 0.82 + 0.18 * grid_x * grid_y;

    // --- CRT Scanlines (per virtual pixel row) ---
    let scanline_phase = fract(uv.y * virt_res.y);
    let scanline = 0.80 + 0.20 * smoothstep(0.0, 0.32, scanline_phase)
                                 * smoothstep_r(1.0, 0.68, scanline_phase);
    color *= scanline;

    // --- RGB Phosphor Triad Sub-Pixel Tint ---
    let sub_pixel = fract(uv.x * virt_res.x * 3.0);
    var phosphor: vec3<f32>;
    if sub_pixel < 0.333 {
        phosphor = vec3<f32>(1.08, 0.96, 0.96);
    } else if sub_pixel < 0.666 {
        phosphor = vec3<f32>(0.96, 1.08, 0.96);
    } else {
        phosphor = vec3<f32>(0.96, 0.96, 1.08);
    }
    color *= phosphor;

    // Warm incandescent bloom for white-hot flame cores
    if (heat > 0.85) {
        let white_core = (heat - 0.85) / 0.15;
        color += vec3<f32>(0.25, 0.18, 0.06) * white_core;
    }

    // --- CRT Post-Processing Effects ---
    // Subtle electron beam flicker, gentle vignette, and analog noise
    var crt_settings = get_default_crt();
    crt_settings.scanline_intensity = 0.0; // Handled by row scanlines above
    crt_settings.vignette_scale = 1.8;     // Wide vignette preserving full-screen coverage
    crt_settings.vignette_softness = 0.95;
    crt_settings.noise_intensity = 0.02;   // Authentic analog noise
    crt_settings.flicker_intensity = 0.015;// Subtle CRT electron beam flicker
    color = apply_crt_effects(color, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);

    return vec4<f32>(color, 1.0);
}
