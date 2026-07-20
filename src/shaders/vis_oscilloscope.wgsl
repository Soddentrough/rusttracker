// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;


@group(0) @binding(1)
var<storage, read> waveform_history: array<f32>;

// Waveforms are pre-smoothed on CPU, so we can read directly
fn get_waveform(hist_idx: u32, idx: u32) -> f32 {
    let res = max(audio.waveform_resolution, 128u);
    let clamped_idx = clamp(idx, 0u, res - 1u);
    // engine.rs places each frame at a 2048 stride
    return waveform_history[hist_idx * 2048u + clamped_idx];
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
    // Since lines are thin, a small search radius is sufficient to prevent horizontal clipping
    let search_radius = clamp(i32(res * 0.003), 2, 8);
    let start_idx = max(0i, i32(idx) - search_radius);
    let end_idx = min(i32(res) - 2i, i32(idx) + search_radius);

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
    let final_uv = crt_distort_uv(in.uv, 0.05);

    let aspect = audio.aspect_ratio;
    var final_color = vec3<f32>(0.0);

    let crt_uv = in.uv * 2.0 - 1.0;
    let r = length(crt_uv);
    let edge_blur = smoothstep(0.2, 1.5, r);

    // Warm amber phosphor color
    let amber = vec3<f32>(1.0, 0.45, 0.05);

    var wave_intensity = 0.0;

    let dy = dpdy(in.uv.y);
    let base_thickness = max(dy * 0.6, 0.0005);
    let blur_thickness = max(dy * 2.0, 0.0015);

    // Performance: skip every other history frame when count exceeds 72,
    // compensating with doubled contribution to maintain total brightness.
    let hist_count = min(audio.waveform_history_size, 144u);
    let step = select(1u, 2u, hist_count > 72u);
    let step_scale = f32(step);
    
    // Lock thickness to physical screen pixels so it doesn't vanish on short windows
    let thickness = base_thickness + edge_blur * blur_thickness;
    // Tighter bloom (reduced spread and intensity)
    let bloom_spread = max(dy * 0.2, 0.0002);

    for (var i = 0u; i < hist_count; i = i + step) {
        let true_dist = get_wave_dist(i, final_uv, aspect);

        // Exponential phosphor decay (frame 0 is oldest = most faded)
        let frames_old = f32(hist_count - 1u - i);
        // decay scaled for 144 frames (previously 0.6 for 8 frames = ~12% per frame)
        // 0.05 gives a long 1-second smooth trail.
        let age = exp(-frames_old * 0.06);

        let core = smoothstep_r(thickness, 0.0, true_dist) * 0.6;
        let bloom = 0.00005 / (true_dist * true_dist + bloom_spread) * 0.02;

        // Tighter halation (faster falloff)
        let halation = exp(-true_dist * 120.0) * 0.01;

        let frame_intensity = (core + bloom + halation) * age * step_scale;

        wave_intensity = wave_intensity + frame_intensity;
    }

    // ACES tonemapping (consistent with other visualizers)
    let mapped = wave_intensity * amber;
    var tonemapped = aces_tonemap(mapped);

    // Keep the phosphor tint WHITE: the color is already amber-tonemapped above,
    // so tinting again would multiply amber by amber and crush the midtones to red.
    var crt_settings = get_default_crt();
    
    // Smooth CRT bezel fade (radial, no rectangular edges)
    let bezel = smoothstep_r(1.4, 0.9, r);
    
    // Analog noise glow
    let noise_val = hash21_crt(in.clip_position.xy + fract(audio.smooth_time) * 137.0);
    let noise_glow = amber * noise_val * 0.015 * bezel * clamp(wave_intensity * 0.8, 0.0, 1.0);

    final_color = apply_crt_effects(tonemapped, in.uv, in.clip_position.xy, audio.smooth_time, crt_settings);
    final_color = final_color + noise_glow;

    // Output Linear RGB. WGPU Srgb surface will apply the sRGB gamma curve automatically.
    return vec4<f32>(final_color, 1.0);
}

