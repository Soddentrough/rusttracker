// =====================================================
// RustTracker Shared Shader Header — AudioUniforms + Fullscreen Quad Vertex
// This is the single source of truth for the AudioUniforms struct layout.
// Any field changes MUST be made here — all shaders include this file.
// =====================================================

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
    channel_phases: array<vec4<f32>, 8>,
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
    waveform_resolution: u32,
    waveform_history_size: u32,
    frame_count: u32,
    step_fraction: f32,
    steps_to_fill: u32,
    aspect_ratio: f32,
    // Real frame delta in seconds (clamped) — for framerate-independent sims
    frame_dt: f32,
    // (push_count + step_fraction) * 0.5 — world Z locked to history rows
    history_cam_z: f32,
    fire_intensity: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
};

// Narkowicz ACES fitted tonemapping curve.
// Shared by all visualizers so HDR->SDR mapping is consistent everywhere.
// Input is linear HDR color; output is tonemapped, clamped to [0,1].
fn aces_tonemap(col: vec3<f32>) -> vec3<f32> {
    let mapped = (col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14);
    return clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn hash21_crt(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Reversed-edge smoothstep: equivalent to smoothstep(hi, lo, x) with hi > lo,
// which is indeterminate per the WGSL spec. Use this instead.
fn smoothstep_r(hi: f32, lo: f32, x: f32) -> f32 {
    return 1.0 - smoothstep(lo, hi, x);
}

fn crt_distort_uv(uv: vec2<f32>, distortion: f32) -> vec2<f32> {
    let crt_uv = uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    return (crt_uv * (1.0 + r2 * distortion)) * 0.5 + 0.5;
}

struct CRTSettings {
    scanline_intensity: f32,
    vignette_scale: f32,
    vignette_softness: f32,
    noise_intensity: f32,
    flicker_intensity: f32,
    phosphor_tint: vec3<f32>,
}

fn get_default_crt() -> CRTSettings {
    var settings: CRTSettings;
    settings.scanline_intensity = 0.15;
    settings.vignette_scale = 1.4;
    settings.vignette_softness = 0.85;
    settings.noise_intensity = 0.025;
    settings.flicker_intensity = 0.03;
    settings.phosphor_tint = vec3<f32>(1.0, 1.0, 1.0);
    return settings;
}

fn apply_crt_effects(color: vec3<f32>, uv: vec2<f32>, clip_pos: vec2<f32>, time: f32, settings: CRTSettings) -> vec3<f32> {
    var final_color = color;
    
    // CRT scanlines
    let scanline = 1.0 - settings.scanline_intensity + settings.scanline_intensity * cos(clip_pos.y * 3.14159);
    final_color *= scanline;
    
    // Vignette
    let crt_uv = uv * 2.0 - 1.0;
    let r = length(crt_uv);
    let bezel = smoothstep_r(settings.vignette_scale, settings.vignette_softness, r);
    final_color *= bezel;
    
    // Flicker
    let flicker = 1.0 - settings.flicker_intensity + settings.flicker_intensity * sin(time * 377.0);
    final_color *= flicker;
    
    // Analog noise
    let noise_val = hash21_crt(clip_pos + fract(time) * 137.0);
    let static_noise = noise_val * settings.noise_intensity * bezel;
    final_color = final_color + static_noise;
    
    return final_color * settings.phosphor_tint;
}
