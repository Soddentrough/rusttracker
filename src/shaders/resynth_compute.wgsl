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

    // Local string coordinate s in [0.0, 1.0] for Left half [0.0, 0.5] and Right half [0.5, 1.0].
    // Fixed boundary nodes at s = 0.0 (u = 0.0, 0.5) and s = 1.0 (u = 0.5, 1.0).
    let s_l = clamp(x_norm * 2.0, 0.0, 1.0);
    let s_r = clamp((x_norm - 0.5) * 2.0, 0.0, 1.0);
    let pi = 3.14159265;

    // Integer base eigenmode harmonic n_base (number of standing half-wave loops per channel):
    // 2 half-waves at 20Hz sub-bass (back) up to 14 half-waves at 10kHz treble (front).
    let n_base = 2u + u32(round(12.0 * pow(band_t, 1.20)));

    // Partition the 32 bins of this band into 4 harmonic eigenmode sub-bands (8 bins each)
    var sub_sum_l = vec4<f32>(0.0);
    var sub_sum_r = vec4<f32>(0.0);
    var sub_peak_l = vec4<f32>(0.0);
    var sub_peak_r = vec4<f32>(0.0);
    var weight_sum_l = 0.0;
    var weight_sum_r = 0.0;
    var peak_l = 0.0;
    var peak_r = 0.0;

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

        let sub_idx = (b - start_bin) >> 3u; // 0..3 (8 bins per harmonic eigenmode)
        if sub_idx == 0u {
            sub_sum_l.x += mag_l;
            sub_sum_r.x += mag_r;
            sub_peak_l.x = max(sub_peak_l.x, mag_l);
            sub_peak_r.x = max(sub_peak_r.x, mag_r);
        } else if sub_idx == 1u {
            sub_sum_l.y += mag_l;
            sub_sum_r.y += mag_r;
            sub_peak_l.y = max(sub_peak_l.y, mag_l);
            sub_peak_r.y = max(sub_peak_r.y, mag_r);
        } else if sub_idx == 2u {
            sub_sum_l.z += mag_l;
            sub_sum_r.z += mag_r;
            sub_peak_l.z = max(sub_peak_l.z, mag_l);
            sub_peak_r.z = max(sub_peak_r.z, mag_r);
        } else {
            sub_sum_l.w += mag_l;
            sub_sum_r.w += mag_r;
            sub_peak_l.w = max(sub_peak_l.w, mag_l);
            sub_peak_r.w = max(sub_peak_r.w, mag_r);
        }

        weight_sum_l += mag_l;
        weight_sum_r += mag_r;
        peak_l = max(peak_l, mag_l);
        peak_r = max(peak_r, mag_r);
    }

    // Sub-band mode energies (combining 8-bin mean and peak)
    let mode_e_l = mix(sub_sum_l * 0.125, sub_peak_l, 0.72);
    let mode_e_r = mix(sub_sum_r * 0.125, sub_peak_r, 0.72);

    // 4 integer harmonic eigenmodes per string: n_base, n_base+1, n_base+2, n_base+4
    let modes_n = vec4<f32>(
        f32(n_base),
        f32(n_base + 1u),
        f32(n_base + 2u),
        f32(n_base + 4u)
    );

    // Coherent sub-Nyquist temporal standing-wave oscillation rate:
    // Nodes sin(n * pi * s) == 0 remain 100% stationary in X, while antinodes oscillate vertically.
    // Adding the quadrature octave overtone 0.32 * sin(2 * n * pi * s) * sin(theta) shares the EXACT
    // same stationary nodes while preventing the string from flattening to zero when cos(theta) == 0.
    let base_omega = 5.4 + 9.2 * pow(band_t, 0.85);
    let omega_vec = base_omega * vec4<f32>(1.00, 1.21, 1.47, 1.79);
    let phase_offset = f32(line_idx) * 0.45 + vec4<f32>(0.0, 1.1, 2.3, 3.7);
    let theta_l = audio.smooth_time * omega_vec + phase_offset;
    let theta_r = audio.smooth_time * omega_vec + phase_offset + vec4<f32>(0.35, -0.40, 0.55, -0.30);

    let k_l = modes_n * (pi * s_l);
    let k_r = modes_n * (pi * s_r);

    let eig_l = sin(k_l) * cos(theta_l) + 0.32 * sin(2.0 * k_l) * sin(theta_l);
    let eig_r = sin(k_r) * cos(theta_r) + 0.32 * sin(2.0 * k_r) * sin(theta_r);

    // Normalize mode weights by RMS mode energy so single-mode notes form pure textbook standing
    // waves and multi-mode chords form rich harmonic interference patterns.
    let rms_mode_l = max(length(mode_e_l), 0.0001);
    let rms_mode_r = max(length(mode_e_r), 0.0001);
    let norm_w_l = mode_e_l / rms_mode_l;
    let norm_w_r = mode_e_r / rms_mode_r;

    // Natural string bridge envelope sin(pi * s)^0.45 keeps fixed nodes at s=0 and s=1 razor-sharp
    let bridge_env_l = pow(max(sin(pi * s_l), 0.0), 0.45);
    let bridge_env_r = pow(max(sin(pi * s_r), 0.0), 0.45);

    let carrier_l = dot(norm_w_l, eig_l) * bridge_env_l;
    let carrier_r = dot(norm_w_r, eig_r) * bridge_env_r;

    let mean_l = weight_sum_l / 32.0;
    let mean_r = weight_sum_r / 32.0;

    // Map [0..100] audio energy with natural dynamic contrast so broadband background noise
    // stays low while active musical notes rise clearly without pegging all 32 rows at a ceiling
    let band_energy_l = mix(mean_l, peak_l, 0.72);
    let band_energy_r = mix(mean_r, peak_r, 0.72);
    let env_l = pow(clamp(band_energy_l / 34.0, 0.0, 1.15), 0.95);
    let env_r = pow(clamp(band_energy_r / 34.0, 0.0, 1.15), 0.95);

    let val_l = tanh(carrier_l * 1.18) * env_l;
    let val_r = tanh(carrier_r * 1.18) * env_r;

    let base_offset = 16u * 1024u;
    let flat_idx = line_idx * 512u + point_idx;
    gpu_spectrum[base_offset + flat_idx] = vec2<f32>(val_l, val_r);
}
