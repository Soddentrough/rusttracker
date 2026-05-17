// INCLUDE: common
//
// Lissajous Laser Projector — XY oscilloscope with analog laser aesthetic
//
// Performance strategy:
//   - Only 1 current frame + 2 fading trail frames (not 48!)
//   - Downsample to 64 line segments max per frame
//   - Use wider bloom to compensate for fewer segments
//   - Total: 3 frames × 64 segments = 192 sdLine calls per pixel (vs 12,288 before)

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> waveform_history: array<f32>;

fn get_waveform(hist_idx: u32, idx: u32) -> f32 {
    let res = max(audio.waveform_resolution, 128u);
    let clamped_idx = clamp(idx % res, 0u, res - 1u);
    return waveform_history[hist_idx * 2048u + clamped_idx];
}

// Linearly interpolated waveform read for smoother curves
fn get_waveform_lerp(hist_idx: u32, t: f32, res: u32) -> f32 {
    let fi = t * f32(res - 1u);
    let i0 = u32(fi);
    let i1 = min(i0 + 1u, res - 1u);
    let frac = fract(fi);
    return mix(get_waveform(hist_idx, i0), get_waveform(hist_idx, i1), frac);
}

fn sdLine(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // CRT barrel distortion
    let crt_uv = in.uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    let distorted = crt_uv * (1.0 + r2 * 0.04);
    let final_uv = distorted * 0.5 + 0.5;

    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);
    let safe_aspect = select(1.0, aspect, aspect > 0.0001 || aspect < -0.0001);

    let p = vec2<f32>((final_uv.x * 2.0 - 1.0) * safe_aspect, -(final_uv.y * 2.0 - 1.0));

    let res = max(audio.waveform_resolution, 128u);

    // --- Fixed 64 output segments regardless of waveform resolution ---
    let num_segments = 64u;
    let sample_step = f32(res) / f32(num_segments);

    // Phase offset: quarter-wave with slow evolution for interesting patterns
    let phase_frac = 0.25 + 0.1 * sin(audio.time * 0.2);

    // Cheap adaptive gain: estimate RMS from 16 evenly-spaced samples of current frame
    // This runs once per pixel but only does 16 reads (negligible vs the 192 in the main loop)
    var rms_sum = 0.0;
    for (var s = 0u; s < 16u; s = s + 1u) {
        let v = get_waveform(0u, s * (res / 16u));
        rms_sum += v * v;
    }
    let rms = sqrt(rms_sum / 16.0);
    // Scale up quiet signals, limit loud ones — target ~0.6 screen coverage
    let gain = clamp(0.55 / max(rms, 0.02), 0.4, 6.0);

    var color = vec3<f32>(0.0);

    // --- Only 3 trail frames: current + 2 fading ---
    let trail_count = min(audio.waveform_history_size, 3u);

    for (var frame = 0u; frame < trail_count; frame = frame + 1u) {
        // Phosphor decay: current frame bright, older frames fade fast
        let age = exp(-f32(frame) * 1.2);

        var min_dist = 100.0;

        for (var seg = 0u; seg < num_segments; seg = seg + 1u) {
            let t0 = f32(seg) / f32(num_segments);
            let t1 = f32(seg + 1u) / f32(num_segments);

            // X from phase-shifted waveform, Y from direct waveform
            let phase_off = phase_frac;
            let x0 = get_waveform_lerp(frame, fract(t0 + phase_off), res) * gain;
            let y0 = get_waveform_lerp(frame, t0, res) * gain;
            let x1 = get_waveform_lerp(frame, fract(t1 + phase_off), res) * gain;
            let y1 = get_waveform_lerp(frame, t1, res) * gain;

            let a = vec2<f32>(x0 * safe_aspect, y0);
            let b = vec2<f32>(x1 * safe_aspect, y1);

            let d = sdLine(p, a, b);
            min_dist = min(min_dist, d);
        }

        // Thin bright core + wide bloom
        let core = smoothstep(0.008, 0.0, min_dist) * 1.5;
        let bloom = 0.0003 / (min_dist * min_dist + 0.0003);
        let halation = exp(-min_dist * 30.0) * 0.08;

        let frame_intensity = (core + bloom + halation) * age;

        // Green laser phosphor, older frames shift slightly dimmer/cooler
        let phosphor = mix(vec3<f32>(0.15, 1.0, 0.3), vec3<f32>(0.05, 0.6, 0.2), f32(frame) * 0.3);
        color += phosphor * frame_intensity;
    }

    // --- CRT scanlines ---
    let scanline = 0.9 + 0.1 * cos(in.clip_position.y * 3.14159);
    color *= scanline;

    // --- CRT bezel vignette ---
    let r = length(crt_uv);
    let bezel = smoothstep(1.4, 0.85, r);
    color *= bezel;

    // --- Faint analog static ---
    let noise_val = hash21(in.clip_position.xy + fract(audio.smooth_time) * 137.0);
    color += vec3<f32>(0.03, 0.06, 0.03) * noise_val * 0.015 * bezel;

    // ACES tonemapping
    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));

    return vec4<f32>(final_col, 1.0);
}
