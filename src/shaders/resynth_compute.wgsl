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

    // Standing-wave spatial cycles across the full unbroken string [0.0, 1.0]:
    // Deep bass strings at the back (line_idx = 0) form wide 1.5-cycle standing waves,
    // scaling progressively to tighter 10.5-cycle standing waves on treble strings at the front.
    let base_cycles = 1.5 + 9.0 * pow(band_t, 1.20);
    let x_center = x_norm - 0.5;

    var centroid_num_l = 0.0;
    var centroid_num_r = 0.0;
    var mode_weight_l = 0.0;
    var mode_weight_r = 0.0;
    var weight_sum_l = 0.0;
    var weight_sum_r = 0.0;
    var peak_l = 0.0;
    var peak_r = 0.0;

    // Pool all 32 bins across the band using temporally-smoothed audio.spectrum magnitudes
    // so sustained notes and chords remain calm and stationary without frame-to-frame FFT jitter.
    for (var b = start_bin; b < end_bin; b = b + 1u) {
        let c_l = gpu_spectrum[b];
        let c_r = select(c_l, gpu_spectrum[1024u + b], has_right);

        // Convert complex FFT magnitude to the standard [0..100] spectrum scale (matching audio.rs line 272)
        let raw_l = length(c_l) * 100.0;
        let raw_r = length(c_r) * 100.0;
        let spec_bin = audio.spectrum[b >> 2u][b & 3u];
        let lr_avg = max((raw_l + raw_r) * 0.5, 0.05);
        let ratio_l = select(1.0, clamp(raw_l / lr_avg, 0.25, 1.75), (raw_l + raw_r) > 0.15);
        let ratio_r = select(1.0, clamp(raw_r / lr_avg, 0.25, 1.75), (raw_l + raw_r) > 0.15);

        let smooth_l = spec_bin * ratio_l;
        let smooth_r = spec_bin * ratio_r;
        let base_l = mix(smooth_l, max(smooth_l, raw_l), 0.15);
        let base_r = mix(smooth_r, max(smooth_r, raw_r), 0.15);

        let t = f32(b) * 0.0009765625;
        let freq_hz = 20.0 * exp2(t * 8.965784);

        // Gentle perceptual tilt so mid and treble bands translate to balanced physical wave height alongside bass
        let eq_boost = pow(freq_hz / 65.0, 0.20);

        let mag_l = max(base_l * eq_boost - 1.2, 0.0);
        let mag_r = max(base_r * eq_boost - 1.2, 0.0);

        let bin_t = f32(b - start_bin) / 31.0;

        // Power weighting (mag^2) locks the intra-band pitch centroid onto dominant musical notes
        // while ignoring low-level FFT sidelobe leakage.
        let w_l = mag_l * mag_l;
        let w_r = mag_r * mag_r;

        centroid_num_l += w_l * bin_t;
        centroid_num_r += w_r * bin_t;
        mode_weight_l += w_l;
        mode_weight_r += w_r;
        weight_sum_l += mag_l;
        weight_sum_r += mag_r;
        peak_l = max(peak_l, mag_l);
        peak_r = max(peak_r, mag_r);
    }

    let mean_l = weight_sum_l / 32.0;
    let mean_r = weight_sum_r / 32.0;

    // Smoothly anchored intra-band centroid in [0.0, 1.0]
    let anchor_w = 8.0;
    let cent_l = (centroid_num_l + anchor_w * 0.5) / (mode_weight_l + anchor_w);
    let cent_r = (centroid_num_r + anchor_w * 0.5) / (mode_weight_r + anchor_w);

    let cycles_l = base_cycles + (cent_l - 0.5) * 0.45;
    let cycles_r = base_cycles + (cent_r - 0.5) * 0.45;

    // Pure symmetric standing wave centered at x_norm = 0.5 across the full string span [0.0, 1.0]:
    // Guarantees clean sinusoidal loops with maximum antinode amplitude at the string center (u = 0.5).
    let carrier_l = cos(x_center * cycles_l * 6.2831853);
    let carrier_r = cos(x_center * cycles_r * 6.2831853);

    // Map [0..100] audio energy with natural dynamic contrast so broadband background noise
    // stays low while active musical notes rise clearly without pegging all 32 rows at a ceiling
    let band_energy_l = mix(mean_l, peak_l, 0.72);
    let band_energy_r = mix(mean_r, peak_r, 0.72);
    let env_l = pow(clamp(band_energy_l / 34.0, 0.0, 1.15), 0.95);
    let env_r = pow(clamp(band_energy_r / 34.0, 0.0, 1.15), 0.95);

    let val_l = carrier_l * env_l;
    let val_r = carrier_r * env_r;

    let base_offset = 16u * 1024u;
    let flat_idx = line_idx * 512u + point_idx;
    gpu_spectrum[base_offset + flat_idx] = vec2<f32>(val_l, val_r);
}
