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

fn hash11_common(p: f32) -> f32 {
    var p1 = fract(p * 0.1031);
    p1 *= p1 + 33.33;
    p1 *= p1 + p1;
    return fract(p1);
}

fn hash21_crt(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22_common(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.xyx) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

fn pcg_hash_common(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn value_noise2d(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash21_crt(i + vec2<f32>(0.0, 0.0)), hash21_crt(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash21_crt(i + vec2<f32>(0.0, 1.0)), hash21_crt(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y
    );
}

fn fbm2d_4(p_in: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var p = p_in;
    let rot = mat2x2<f32>(0.87758, 0.47942, -0.47942, 0.87758);
    for (var i = 0; i < 4; i++) {
        v += a * value_noise2d(p);
        p = rot * p * 2.02 + vec2<f32>(100.0, 100.0);
        a *= 0.5;
    }
    return v;
}

fn sd_line_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h);
}

fn smin_poly(a: f32, b: f32, k: f32) -> f32 {
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

fn smax_poly(a: f32, b: f32, k: f32) -> f32 {
    let h = max(k - abs(a - b), 0.0) / k;
    return max(a, b) + h * h * k * 0.25;
}

fn iridescence_cos(t: f32) -> vec3<f32> {
    let a = vec3<f32>(0.5, 0.5, 0.5);
    let b = vec3<f32>(0.5, 0.5, 0.5);
    let c = vec3<f32>(1.0, 1.0, 1.0);
    let d = vec3<f32>(0.00, 0.33, 0.67);
    return a + b * cos(6.2831853 * (c * t + d));
}

// Synthesizes a 12-channel (7.1.4) spatial VU level for speaker/pole `idx` (0..11)
// when the source audio is stereo/mono (`num_spatial <= 2u`), using L/R VUs,
// mid/side difference, phase correlation, and 4 frequency bands so all 12 speakers/poles
// react dynamically instead of leaving 10 channels dead or cloned.
fn stereo_upmix_12(
    idx: u32,
    ch_l: f32,
    ch_r: f32,
    phase_l: f32,
    phase_r: f32,
    bass_e: f32,
    lowmid_e: f32,
    mid_e: f32,
    high_e: f32
) -> f32 {
    let _unused_phase = phase_l + phase_r;
    let mid_vu = (ch_l + ch_r) * 0.5;
    let side_vu = abs(ch_l - ch_r);
    switch idx {
        case 0u: { return ch_l; } // FL
        case 1u: { return ch_r; } // FR
        case 2u: { return clamp(mid_vu * 0.92 + lowmid_e * 0.25, 0.0, 1.0); } // C
        case 3u: { return clamp(bass_e * 1.15 + mid_vu * 0.35, 0.0, 1.0); } // LFE
        case 4u: { return clamp(ch_l * 0.72 + side_vu * 0.45 + mid_e * 0.25, 0.0, 1.0); } // Ls
        case 5u: { return clamp(ch_r * 0.72 + side_vu * 0.45 + mid_e * 0.25, 0.0, 1.0); } // Rs
        case 6u: { return clamp(ch_l * 0.58 + side_vu * 0.60 + lowmid_e * 0.30, 0.0, 1.0); } // Lrs
        case 7u: { return clamp(ch_r * 0.58 + side_vu * 0.60 + lowmid_e * 0.30, 0.0, 1.0); } // Rrs
        case 8u: { return clamp(ch_l * 0.65 + high_e * 0.55, 0.0, 1.0); } // Ltf
        case 9u: { return clamp(ch_r * 0.65 + high_e * 0.55, 0.0, 1.0); } // Rtf
        case 10u: { return clamp(ch_l * 0.50 + side_vu * 0.40 + high_e * 0.60, 0.0, 1.0); } // Ltr
        default: { return clamp(ch_r * 0.50 + side_vu * 0.40 + high_e * 0.60, 0.0, 1.0); } // Rtr
    }
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
    settings.flicker_intensity = 0.02;
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
    
    // Subtle analog power-supply hum modulation (avoids high-frequency display refresh strobing)
    let flicker = 1.0 - settings.flicker_intensity * 0.5 + settings.flicker_intensity * 0.5 * sin(time * 24.0);
    final_color *= flicker;
    
    // Analog noise
    let noise_val = hash21_crt(clip_pos + fract(time) * 137.0);
    let static_noise = noise_val * settings.noise_intensity * bezel;
    final_color = final_color + static_noise;
    
    return final_color * settings.phosphor_tint;
}
