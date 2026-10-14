// INCLUDE: common

@group(0) @binding(0) var<storage, read_write> gpu_spectrum: array<vec2<f32>>;
@group(0) @binding(1) var<uniform> audio: AudioUniforms;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let point_idx = global_id.x; // 0 to 511
    let line_idx = global_id.y;  // 0 to 31 (0 = 20Hz sub-bass at back, 31 = 10kHz treble at front)

    if point_idx >= 512u || line_idx >= 32u {
        return;
    }

    let num_lines = 32u;
    let num_points = 512u;

    let bins_per_line = 1024u / num_lines; // 32
    let start_bin = line_idx * bins_per_line;
    let end_bin = start_bin + bins_per_line;

    let x_norm = f32(point_idx) / f32(num_points - 1u);
    let x_centered = x_norm - 0.5;
    let has_right = audio.fft_channels >= 2u;

    let band_t = f32(line_idx) / 31.0;

    var wave_sum_l = 0.0;
    var wave_sum_r = 0.0;
    var weight_sum_l = 0.0;
    var weight_sum_r = 0.0;
    var peak_l = 0.0;
    var peak_r = 0.0;

    // Sample 8 evenly spaced sub-bins across the 32-bin band (stride 4) for crisp harmonic structure
    for (var b = start_bin; b < end_bin; b = b + 4u) {
        let c_l = gpu_spectrum[b];
        let c_r = select(c_l, gpu_spectrum[1024u + b], has_right);

        // Normalize Hann-windowed FFT magnitude (where full-scale sine is ~12..16 in length(c))
        let raw_l = length(c_l) * 0.085;
        let raw_r = length(c_r) * 0.085;

        let t = f32(b) * 0.0009765625;
        let freq_hz = 20.0 * exp2(t * 8.965784);

        // Gentle perceptual tilt (+1.8dB/oct) so treble bands are visible without blowing out noise
        let eq_boost = pow(freq_hz / 45.0, 0.28);

        let mag_l = max(raw_l * eq_boost - 0.010, 0.0);
        let mag_r = max(raw_r * eq_boost - 0.010, 0.0);

        // Deeper frequencies at the back (line_idx = 0 -> ~1.6 cycles across full field)
        // to higher frequencies at the front (line_idx = 31 -> ~23 cycles across full field)
        let bin_t = f32(b - start_bin) / 31.0;
        let spatial_cycles = 1.6 + 19.5 * pow(band_t, 1.75) + bin_t * (0.45 + 1.8 * band_t);

        let temporal_rate = 1.8 + band_t * 5.2 + bin_t * 0.6;
        let partial_offset = f32(b - start_bin) * 0.37 + f32(line_idx) * 0.58;
        let base_phase = x_centered * spatial_cycles * 6.2831853 - audio.smooth_time * temporal_rate + partial_offset;

        wave_sum_l += mag_l * cos(base_phase);
        wave_sum_r += mag_r * cos(base_phase + 0.25);
        weight_sum_l += mag_l;
        weight_sum_r += mag_r;
        peak_l = max(peak_l, mag_l);
        peak_r = max(peak_r, mag_r);
    }

    let mean_l = weight_sum_l * 0.125;
    let mean_r = weight_sum_r * 0.125;

    // Normalize carrier wave so log-bin oversampling at low frequencies never inflates amplitude
    let carrier_l = wave_sum_l / max(weight_sum_l * 0.45 + peak_l * 0.55, 0.0001);
    let carrier_r = wave_sum_r / max(weight_sum_r * 0.45 + peak_r * 0.55, 0.0001);

    let env_l = tanh(mix(mean_l, peak_l, 0.65) * 1.35);
    let env_r = tanh(mix(mean_r, peak_r, 0.65) * 1.35);

    let val_l = clamp(carrier_l, -1.15, 1.15) * env_l;
    let val_r = clamp(carrier_r, -1.15, 1.15) * env_r;

    let base_offset = 16u * 1024u;
    let flat_idx = line_idx * 512u + point_idx;
    gpu_spectrum[base_offset + flat_idx] = vec2<f32>(val_l, val_r);
}
