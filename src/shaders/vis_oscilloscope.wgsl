// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;


@group(0) @binding(1)
var<storage, read> waveform_history: array<f32>;

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Waveforms are pre-smoothed on CPU, so we can read directly
fn get_waveform(hist_idx: u32, idx: u32) -> f32 {
    let res = max(audio.waveform_resolution, 128u);
    let clamped_idx = clamp(idx, 0u, res - 1u);
    // engine.rs places each frame at a 2048 stride
    return waveform_history[hist_idx * 2048u + clamped_idx];
}

// Linear interpolation between integer sample indices for sub-pixel accuracy
fn get_waveform_lerp(hist_idx: u32, x: f32) -> f32 {
    let res = f32(max(audio.waveform_resolution, 128u));
    let float_idx = clamp(x, 0.0, 0.999) * (res - 1.0);
    let idx0 = u32(float_idx);
    let idx1 = min(idx0 + 1u, u32(res) - 1u);
    let frac = fract(float_idx);
    return mix(get_waveform(hist_idx, idx0), get_waveform(hist_idx, idx1), frac);
}

fn sdLine(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn get_wave_dist(hist_idx: u32, uv: vec2<f32>, aspect: f32) -> f32 {
    let res = f32(max(audio.waveform_resolution, 128u));
    let clamped_x = clamp(uv.x, 0.0, 0.999);
    let float_idx = clamped_x * (res - 1.0);
    let idx = u32(float_idx);

    var min_dist = 1000.0;

    // Check local neighborhood for proper line segment coverage
    let start_idx = max(0i, i32(idx) - 1);
    let end_idx = min(i32(res) - 2i, i32(idx) + 1);

    let p = vec2<f32>(uv.x * aspect, uv.y);

    for (var i = start_idx; i <= end_idx; i = i + 1) {
        let u_idx0 = u32(i);
        let u_idx1 = u_idx0 + 1u;

        let x0 = f32(u_idx0) / (res - 1.0);
        let x1 = f32(u_idx1) / (res - 1.0);

        let v0 = get_waveform(hist_idx, u_idx0);
        let v1 = get_waveform(hist_idx, u_idx1);

        let y0 = v0 * 0.4 + 0.5;
        let y1 = v1 * 0.4 + 0.5;

        let a = vec2<f32>(x0 * aspect, y0);
        let b = vec2<f32>(x1 * aspect, y1);

        let d = sdLine(p, a, b);
        min_dist = min(min_dist, d);
    }

    return min_dist;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // CRT barrel distortion
    let crt_uv = in.uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    let distorted_uv = crt_uv * (1.0 + r2 * 0.05);
    let final_uv = distorted_uv * 0.5 + 0.5;

    let aspect = dpdx(in.uv.x) / dpdy(in.uv.y);
    var final_color = vec3<f32>(0.0);

    let r = length(crt_uv);
    let edge_blur = smoothstep(0.2, 1.5, r);

    // Warm amber phosphor color
    let amber = vec3<f32>(1.0, 0.45, 0.05);

    var wave_intensity = 0.0;

    // Performance: skip every other history frame when count exceeds 72,
    // compensating with doubled contribution to maintain total brightness.
    let hist_count = min(audio.waveform_history_size, 144u);
    let step = select(1u, 2u, hist_count > 72u);
    let step_scale = f32(step);

    for (var i = 0u; i < hist_count; i = i + step) {
        let true_dist = get_wave_dist(i, final_uv, aspect);

        // Exponential phosphor decay (frame 0 is oldest = most faded)
        let frames_old = f32(hist_count - 1u - i);
        // decay scaled for 144 frames (previously 0.6 for 8 frames = ~12% per frame)
        // 0.05 gives a long 1-second smooth trail.
        let age = exp(-frames_old * 0.06);

        // Beam thickness scales down with dynamic resolution to prevent huge blobs
        let thickness = 0.002 + edge_blur * 0.006;
        let core = smoothstep(thickness, 0.0, true_dist) * 0.4;

        // Tighter bloom (reduced spread and intensity)
        let bloom = 0.0002 / (true_dist * true_dist + 0.001) * 0.03;

        // Tighter halation (faster falloff)
        let halation = exp(-true_dist * 80.0) * 0.015;

        let frame_intensity = (core + bloom + halation) * age * step_scale;

        wave_intensity = wave_intensity + frame_intensity;
    }

    // ACES tonemapping (consistent with other visualizers)
    let mapped = wave_intensity * amber;
    var tonemapped = (mapped * (2.51 * mapped + 0.03)) / (mapped * (2.43 * mapped + 0.59) + 0.14);

    final_color = tonemapped;

    // CRT scanlines
    let scanline = 0.85 + 0.15 * cos(in.clip_position.y * 3.14159);
    final_color *= scanline;

    // Smooth CRT bezel fade (radial, no rectangular edges)
    let bezel = smoothstep(1.4, 0.9, r);

    // Analog noise / static (like vis_flame)
    let noise_val = hash21(in.clip_position.xy + fract(audio.smooth_time) * 137.0);
    let static_noise = noise_val * 0.035 * bezel;
    
    // Add extra faint glow from the waveform onto the noise
    let noise_glow = amber * noise_val * 0.015 * bezel * clamp(wave_intensity * 0.8, 0.0, 1.0);

    final_color = final_color * bezel;
    final_color = final_color + static_noise + noise_glow;

    // Output Linear RGB. WGPU Srgb surface will apply the sRGB gamma curve automatically.
    return vec4<f32>(final_color, 1.0);
}

