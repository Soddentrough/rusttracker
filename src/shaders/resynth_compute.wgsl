// INCLUDE: common

@group(0) @binding(0) var<storage, read_write> gpu_spectrum: array<vec2<f32>>;
@group(0) @binding(1) var<uniform> audio: AudioUniforms;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let point_idx = global_id.x; // 0 to 511
    let line_idx = global_id.y;  // 0 to 31

    if point_idx >= 512u || line_idx >= 32u {
        return;
    }

    let num_lines = 32u;
    let num_points = 512u;

    let bins_per_line = 1024u / num_lines; // 32
    let start_bin = line_idx * bins_per_line;
    let end_bin = start_bin + bins_per_line;

    let x_norm = f32(point_idx) / f32(num_points - 1u);
    // Center x_centered in [-0.5, 0.5] so symmetric standing/traveling modes breathe around the screen center
    let x_centered = x_norm - 0.5;
    let has_right = audio.fft_channels >= 2u;

    var val = 0.0;

    for (var b = start_bin; b < end_bin; b = b + 1u) {
        let c_l = gpu_spectrum[b];
        var mag = length(c_l);
        if has_right {
            let c_r = gpu_spectrum[1024u + b];
            mag = max(mag, length(c_r) * 0.9 + mag * 0.35);
        }

        // `b` is already logarithmically spaced in [0..1023] from 20Hz to 10kHz.
        // Map spatial wave number cleanly below the 512-point Nyquist limit (1.5 cycles at bass -> 34 cycles at 10kHz)
        let t = f32(b) * 0.0009765625;
        let freq_hz = 20.0 * exp2(t * 8.965784);
        let spatial_cycles = 1.5 + 32.5 * pow(t, 1.35);

        // Smooth traveling harmonic phase + subtle per-partial phase offset so waves flow organically
        let temporal_rate = 2.2 + t * 5.5;
        let partial_offset = f32(b - start_bin) * 0.42 + f32(line_idx) * 0.65;
        let phase = x_centered * spatial_cycles * 6.2831853 - audio.smooth_time * temporal_rate + partial_offset;

        // +3dB/octave pink-noise tilt compensation so high-frequency strings/synths remain visible
        let eq_boost = sqrt(freq_hz / 20.0);

        val += (mag * eq_boost) * cos(phase);
    }

    // Scale and soft-clip via tanh so HF-heavy passages never clip downstream
    val = tanh((val / 90.5) * 1.55);

    let base_offset = 16u * 1024u;
    let flat_idx = line_idx * 512u + point_idx;
    gpu_spectrum[base_offset + flat_idx] = vec2<f32>(val, 0.0);
}
