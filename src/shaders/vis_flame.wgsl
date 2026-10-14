// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(3) var fire_grid_tex: texture_2d<f32>;

// ============================================================================
// Canonical 37-color DOOM Fire Palette (Filipe Deschamps / PSX DOOM Reference)
// Hoisted to module-scope const so it is never constructed on the per-pixel stack.
// ============================================================================
const DOOM_PALETTE: array<vec3<f32>, 37> = array<vec3<f32>, 37>(
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

fn doom_palette(h: f32) -> vec3<f32> {
    if (h < 0.005) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    let scaled = clamp(h * 36.0, 0.0, 36.0);
    let idx0 = u32(floor(scaled));
    let idx1 = min(idx0 + 1u, 36u);
    let frac = fract(scaled);
    return mix(DOOM_PALETTE[idx0], DOOM_PALETTE[idx1], frac);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;

    // --- Aspect-Ratio-Adaptive Square Retro Pixels ---
    let aspect = max(audio.aspect_ratio, 0.1);
    let v_height = 240.0;
    let v_width = max(floor(v_height * aspect), 1.0);
    let virt_res = vec2<f32>(v_width, v_height);

    let pixel_coord = floor(uv * virt_res);
    let pixel_uv = (pixel_coord + 0.5) / virt_res;

    // Bilinear blend of nearby simulation grid cells (1024 x 576)
    let grid_uv = pixel_uv * vec2<f32>(1023.0, 575.0);
    let base_coord = vec2<i32>(floor(grid_uv));
    let frac_uv = fract(grid_uv);
    let c00 = textureLoad(fire_grid_tex, clamp(base_coord, vec2<i32>(0), vec2<i32>(1023, 575)), 0).r;
    let c10 = textureLoad(fire_grid_tex, clamp(base_coord + vec2<i32>(1, 0), vec2<i32>(0), vec2<i32>(1023, 575)), 0).r;
    let c01 = textureLoad(fire_grid_tex, clamp(base_coord + vec2<i32>(0, 1), vec2<i32>(0), vec2<i32>(1023, 575)), 0).r;
    let c11 = textureLoad(fire_grid_tex, clamp(base_coord + vec2<i32>(1, 1), vec2<i32>(0), vec2<i32>(1023, 575)), 0).r;
    let heat = mix(mix(c00, c10, frac_uv.x), mix(c01, c11, frac_uv.x), frac_uv.y);

    var color = doom_palette(clamp(heat, 0.0, 1.0));

    // --- CRT Scanlines (subtle analog feel, no harsh checkerboard grid) ---
    let scanline_phase = fract(uv.y * virt_res.y);
    let scanline = 0.90 + 0.10 * cos(scanline_phase * 6.2831853);
    color *= scanline;

    // Subtle phosphor triad sheen
    let sub_pixel = fract(uv.x * virt_res.x * 3.0);
    let phosphor = 0.97 + 0.03 * vec3<f32>(
        select(0.0, 1.0, sub_pixel < 0.333),
        select(0.0, 1.0, sub_pixel >= 0.333 && sub_pixel < 0.666),
        select(0.0, 1.0, sub_pixel >= 0.666)
    );
    color *= phosphor;

    // Warm incandescent bloom for white-hot flame cores
    if (heat > 0.82) {
        let white_core = (heat - 0.82) / 0.18;
        color += vec3<f32>(0.28, 0.20, 0.08) * white_core;
    }

    // --- CRT Post-Processing Effects ---
    var crt_settings = get_default_crt();
    crt_settings.scanline_intensity = 0.0;
    crt_settings.vignette_scale = 1.8;
    crt_settings.vignette_softness = 0.95;
    crt_settings.noise_intensity = 0.02;
    crt_settings.flicker_intensity = 0.015;
    color = apply_crt_effects(color, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);

    return vec4<f32>(color, 1.0);
}
