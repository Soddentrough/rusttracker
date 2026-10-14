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
    let has_right = audio.fft_channels >= 2u;
    let band_t = f32(line_idx) / 31.0;
    let base_cycles = 3.2 + 8.8 * pow(band_t, 1.25);
    let x_l = x_norm - 0.25;
    let x_r = x_norm - 0.75;

    var wave_sum_l = 0.0;
    var wave_sum_r = 0.0;
    var weight_sum_l = 0.0;
    var weight_sum_r = 0.0;
    var peak_l = 0.0;
    var peak_r = 0.0;

    // Pool all 32 bins across the band using fixed symmetric standing-wave phase (no horizontal scroll, no diagonal skew)
    for (var b = start_bin; b < end_bin; b = b + 1u) {
        let c_l = gpu_spectrum[b];
        let c_r = select(c_l, gpu_spectrum[1024u + b], has_right);

        // Convert complex FFT magnitude to the standard [0..100] spectrum scale (matching audio.rs line 272)
        let raw_l = length(c_l) * 100.0;
        let raw_r = length(c_r) * 100.0;
        let spec_bin = audio.spectrum[b >> 2u][b & 3u];
        let lr_avg = max((raw_l + raw_r) * 0.5, 0.001);
        let ratio_l = select(1.0, clamp(raw_l / lr_avg, 0.15, 1.85), raw_l + raw_r > 0.01);
        let ratio_r = select(1.0, clamp(raw_r / lr_avg, 0.15, 1.85), raw_l + raw_r > 0.01);

        let base_l = max(raw_l, spec_bin * ratio_l * 0.55);
        let base_r = max(raw_r, spec_bin * ratio_r * 0.55);

        let t = f32(b) * 0.0009765625;
        let freq_hz = 20.0 * exp2(t * 8.965784);

        // Gentle perceptual tilt so mid and treble bands translate to balanced physical wave height alongside bass
        let eq_boost = pow(freq_hz / 65.0, 0.20);

        let mag_l = max(base_l * eq_boost - 1.2, 0.0);
        let mag_r = max(raw_r * eq_boost - 1.2, max(base_r * eq_boost - 1.2, 0.0));

        let bin_t = f32(b - start_bin) / 31.0;
        let spatial_cycles = base_cycles + (bin_t - 0.5) * 0.55;

        // Symmetric standing phase around Left (0.25) and Right (0.75) channel centers with zero cross-row skew
        let phase_l = x_l * spatial_cycles * 6.2831853;
        let phase_r = x_r * spatial_cycles * 6.2831853;

        wave_sum_l += mag_l * cos(phase_l);
        wave_sum_r += mag_r * cos(phase_r);
        weight_sum_l += mag_l;
        weight_sum_r += mag_r;
        peak_l = max(peak_l, mag_l);
        peak_r = max(peak_r, mag_r);
    }

    let mean_l = weight_sum_l / 32.0;
    let mean_r = weight_sum_r / 32.0;

    let carrier_l = wave_sum_l / max(weight_sum_l, 0.0001);
    let carrier_r = wave_sum_r / max(weight_sum_r, 0.0001);

    // Map [0..100] audio energy with natural dynamic contrast so broadband background noise
    // stays low while active musical notes rise clearly without pegging all 32 rows at a ceiling
    let band_energy_l = mix(mean_l, peak_l, 0.72);
    let band_energy_r = mix(mean_r, peak_r, 0.72);
    let env_l = pow(clamp(band_energy_l / 34.0, 0.0, 1.15), 0.95);
    let env_r = pow(clamp(band_energy_r / 34.0, 0.0, 1.15), 0.95);

    let val_l = tanh(carrier_l * 1.12) * env_l;
    let val_r = tanh(carrier_r * 1.12) * env_r;

    let base_offset = 16u * 1024u;
    let flat_idx = line_idx * 512u + point_idx;
    gpu_spectrum[base_offset + flat_idx] = vec2<f32>(val_l, val_r);
}
