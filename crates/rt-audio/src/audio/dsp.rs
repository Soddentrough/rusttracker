use super::*;
use crate::state::AppState;
use crossbeam_channel::{Receiver, Sender, bounded};
use realfft::RealFftPlanner;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub fn calculate_power_of_two_window_size(sample_rate: u32) -> usize {
    let target = (sample_rate as f32 * 0.185).round() as usize;
    let window_size = 1 << (target as f32).log2().round() as usize;
    window_size.clamp(2048, 65536)
}

pub(in crate::audio) fn dsp_recycle_channel() -> &'static (Sender<DspMessage>, Receiver<DspMessage>)
{
    static POOL: std::sync::OnceLock<(Sender<DspMessage>, Receiver<DspMessage>)> =
        std::sync::OnceLock::new();
    POOL.get_or_init(|| bounded(8))
}

const TRIGGER_CORR_POINTS: usize = 64;

#[inline]
fn zero_phase_lowpass(buf: &mut [f32], alpha: f32) {
    if buf.len() < 2 {
        return;
    }
    let a = alpha.clamp(0.001, 1.0);
    // Two forward-backward passes yield a 4th-order zero-phase low-pass filter
    // with -24 dB/octave rolloff and strictly zero phase shift at all frequencies.
    for _ in 0..2 {
        let mut acc = buf[0];
        for v in buf.iter_mut() {
            acc += a * (*v - acc);
            *v = acc;
        }
        let mut acc_b = *buf.last().unwrap_or(&0.0);
        for v in buf.iter_mut().rev() {
            acc_b += a * (*v - acc_b);
            *v = acc_b;
        }
    }
}

#[inline]
fn sample_cubic_catmull_rom(data: &[f32], pos: f32) -> f32 {
    let len = data.len();
    if len == 0 {
        return 0.0;
    }
    if len == 1 {
        return data[0];
    }
    let max_idx = (len - 1) as f32;
    let clamped = pos.clamp(0.0, max_idx);
    let i1 = clamped.floor() as usize;
    let t = clamped - i1 as f32;
    if t <= 1e-6 {
        return data[i1];
    }
    let i0 = i1.saturating_sub(1);
    let i2 = (i1 + 1).min(len - 1);
    let i3 = (i1 + 2).min(len - 1);

    let p0 = data[i0];
    let p1 = data[i1];
    let p2 = data[i2];
    let p3 = data[i3];

    let a0 = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let a1 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let a2 = -0.5 * p0 + 0.5 * p2;
    let a3 = p1;

    ((a0 * t + a1) * t + a2) * t + a3
}

pub(in crate::audio) struct OscilloscopeTriggerState {
    trig_low: Vec<f32>,
    trig_mid: Vec<f32>,
    trig_combined: Vec<f32>,
    candidates: Vec<(f32, f32, f32)>, // (center_f, swing, recency)
    reference_wave: [f32; TRIGGER_CORR_POINTS],
    corr_window: [f32; TRIGGER_CORR_POINTS],
    has_reference: bool,
    last_seek_epoch: u64,
}

impl OscilloscopeTriggerState {
    pub(in crate::audio) fn new() -> Self {
        let mut corr_window = [0.0f32; TRIGGER_CORR_POINTS];
        for (m, w) in corr_window.iter_mut().enumerate() {
            let phase =
                2.0 * std::f32::consts::PI * (m as f32) / ((TRIGGER_CORR_POINTS - 1) as f32);
            // Center-weighted Hann envelope so central standing-wave nodes dominate phase lock
            *w = 0.15 + 0.85 * (0.5 - 0.5 * phase.cos());
        }
        Self {
            trig_low: Vec::with_capacity(4096),
            trig_mid: Vec::with_capacity(4096),
            trig_combined: Vec::with_capacity(4096),
            candidates: Vec::with_capacity(128),
            reference_wave: [0.0; TRIGGER_CORR_POINTS],
            corr_window,
            has_reference: false,
            last_seek_epoch: 0,
        }
    }

    pub(in crate::audio) fn extract_triggered_waveform(
        &mut self,
        audio_data: &[f32],
        sample_rate: u32,
        visual_width: usize,
        seek_epoch: u64,
        out_waveform: &mut Vec<f32>,
    ) {
        let visual_width = visual_width.clamp(128, 2048);
        if out_waveform.len() != visual_width {
            out_waveform.resize(visual_width, 0.0);
        }
        if audio_data.is_empty() {
            out_waveform.fill(0.0);
            return;
        }
        if seek_epoch != self.last_seek_epoch {
            self.has_reference = false;
            self.last_seek_epoch = seek_epoch;
        }

        let sr_f = (sample_rate.max(8000)) as f32;
        // Maintain a constant ~23ms time window regardless of sample rate or display width,
        // scaling down gracefully only if the input buffer is shorter than ~35ms (e.g. unit tests).
        let max_avail_window = (audio_data.len().saturating_sub(8) as f32 * 0.62).max(16.0);
        let target_window_samples_f = (sr_f * 0.023).min(max_avail_window).max(16.0);
        let stride = target_window_samples_f / (visual_width.saturating_sub(1).max(1) as f32);
        let half_window_f = 0.5 * target_window_samples_f;
        let half_window_margin = (half_window_f.ceil() as usize) + 2;

        if audio_data.len() <= half_window_margin * 2 + 4 {
            let start_f = (audio_data.len() as f32 - target_window_samples_f).max(0.0);
            for (i, dst) in out_waveform.iter_mut().enumerate() {
                let pos_f = start_f + (i as f32) * stride;
                *dst = sample_cubic_catmull_rom(audio_data, pos_f);
            }
            return;
        }

        // Search up to ~36ms for the center-screen trigger point (covers >= 1 full period down to 28Hz)
        let search_span = ((sr_f * 0.036).round() as usize).max(64);
        let max_center = audio_data.len().saturating_sub(half_window_margin + 1);
        let min_center = half_window_margin.min(max_center);
        let search_limit = max_center;
        let search_start = search_limit.saturating_sub(search_span).max(min_center);

        let look_narrow = ((sr_f * 0.0022).round() as usize).clamp(4, 256);
        let look_wide = ((sr_f * 0.0065).round() as usize).clamp(12, 768);
        let filter_pad = ((sr_f * 0.008).round() as usize).clamp(16, 512);

        let filt_start = search_start.saturating_sub(look_wide + filter_pad);
        let filt_end = (search_limit + look_wide + filter_pad + 2).min(audio_data.len());
        let filt_len = filt_end.saturating_sub(filt_start);

        let mut best_center_f = search_limit as f32;

        if filt_len >= 8 && search_limit > search_start {
            let slice = &audio_data[filt_start..filt_end];
            let mut weighted_sum = 0.0f32;
            let mut weight_total = 0.0f32;
            let mut max_abs = 0.0f32;
            let inv_filt_m1 = 1.0 / ((filt_len.saturating_sub(1).max(1)) as f32);
            for (idx, &v) in slice.iter().enumerate() {
                // Hann-windowed DC estimator suppresses partial-cycle boundary oscillation
                let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * (idx as f32) * inv_filt_m1).cos();
                weighted_sum += v * w;
                weight_total += w;
                max_abs = max_abs.max(v.abs());
            }

            if max_abs < 1e-5 {
                self.has_reference = false;
            } else {
                let dc_mean = weighted_sum / weight_total.max(1e-6);
                self.trig_low.clear();
                self.trig_mid.clear();
                self.trig_low.reserve(filt_len);
                self.trig_mid.reserve(filt_len);
                for &v in slice {
                    let centered = v - dc_mean;
                    self.trig_low.push(centered);
                    self.trig_mid.push(centered);
                }

                // Zero-phase fundamental low-pass (~280 Hz) & mid-band (~1350 Hz) trigger filters
                let alpha_low = 1.0 - (-2.0 * std::f32::consts::PI * 280.0 / sr_f).exp();
                let alpha_mid = 1.0 - (-2.0 * std::f32::consts::PI * 1350.0 / sr_f).exp();
                zero_phase_lowpass(&mut self.trig_low, alpha_low);
                zero_phase_lowpass(&mut self.trig_mid, alpha_mid);

                let mut peak_low = 0.0f32;
                let mut peak_mid = 0.0f32;
                for i in (search_start - filt_start)..=(search_limit - filt_start) {
                    peak_low = peak_low.max(self.trig_low[i].abs());
                    peak_mid = peak_mid.max(self.trig_mid[i].abs());
                }

                // Prefer fundamental bass/low-mid carrier whenever present; smoothly transition
                // to the mid-band zero-phase carrier for pure high-frequency tones.
                let low_weight = if peak_low > 0.16 * peak_mid { 2.6 } else { 1.2 };
                let mid_weight = if peak_low > 0.16 * peak_mid {
                    0.30
                } else {
                    1.0
                };

                self.trig_combined.clear();
                self.trig_combined.reserve(filt_len);
                for i in 0..filt_len {
                    self.trig_combined
                        .push(self.trig_low[i] * low_weight + self.trig_mid[i] * mid_weight);
                }

                self.candidates.clear();
                let mut max_swing = 0.0f32;
                let span_f = (search_limit - search_start).max(1) as f32;

                for k in search_start..search_limit {
                    let idx = k - filt_start;
                    let v0 = self.trig_combined[idx];
                    let v1 = self.trig_combined[idx + 1];
                    if v0 <= 0.0 && v1 > 0.0 {
                        let dv = (v1 - v0).max(1e-12);
                        let frac = (-v0 / dv).clamp(0.0, 1.0);
                        let center_f = (k as f32) + frac;

                        // Measure wide-aperture peak-to-trough swing around the crossing
                        // (proportional to true cycle amplitude A rather than high-pass A*f)
                        let back_n = idx.saturating_sub(look_narrow);
                        let fwd_n = (idx + 1 + look_narrow).min(filt_len - 1);
                        let mut min_before = v0;
                        for &s in &self.trig_combined[back_n..=idx] {
                            min_before = min_before.min(s);
                        }
                        let mut max_after = v1;
                        for &s in &self.trig_combined[(idx + 1)..=fwd_n] {
                            max_after = max_after.max(s);
                        }
                        let swing_narrow = (max_after - min_before).max(0.0);

                        let back_w = idx.saturating_sub(look_wide);
                        let fwd_w = (idx + 1 + look_wide).min(filt_len - 1);
                        let mut min_before_w = min_before;
                        for &s in &self.trig_combined[back_w..back_n] {
                            min_before_w = min_before_w.min(s);
                        }
                        let mut max_after_w = max_after;
                        for &s in &self.trig_combined[(fwd_n + 1).min(fwd_w)..=fwd_w] {
                            max_after_w = max_after_w.max(s);
                        }
                        let swing_wide = (max_after_w - min_before_w).max(0.0);
                        let swing = swing_narrow.max(swing_wide * 0.85);

                        if swing > max_swing {
                            max_swing = swing;
                        }
                        let recency = (k - search_start) as f32 / span_f;
                        self.candidates.push((center_f, swing, recency));
                    }
                }

                if !self.candidates.is_empty() && max_swing > 1e-6 {
                    // Hysteresis threshold rejects minor harmonic ripples
                    let swing_thresh = max_swing * 0.28;
                    let corr_span_f = target_window_samples_f * 0.75;
                    let corr_half_f = 0.5 * corr_span_f;
                    let corr_step_f = corr_span_f / ((TRIGGER_CORR_POINTS - 1) as f32);

                    let mut ref_energy = 0.0f32;
                    if self.has_reference {
                        for m in 0..TRIGGER_CORR_POINTS {
                            let r = self.reference_wave[m];
                            ref_energy += self.corr_window[m] * r * r;
                        }
                    }
                    let use_corr = self.has_reference && ref_energy > 1e-6;

                    // Keep evaluation bounded if many high-frequency crossings exist
                    let active_count = self
                        .candidates
                        .iter()
                        .filter(|&&(_, sw, _)| sw >= swing_thresh)
                        .count();
                    let eval_stride = (active_count / 40).max(1);

                    let mut best_score = -1.0f32;
                    let mut best_corr = 0.0f32;
                    let mut best_feat = [0.0f32; TRIGGER_CORR_POINTS];
                    let mut active_idx = 0usize;

                    for &(center_f, swing, recency) in &self.candidates {
                        if swing < swing_thresh {
                            continue;
                        }
                        let should_eval = active_idx.is_multiple_of(eval_stride)
                            || active_idx + 1 == active_count;
                        active_idx += 1;
                        if !should_eval {
                            continue;
                        }

                        let swing_norm = (swing / max_swing).clamp(0.0, 1.0);
                        let mut cand_feat = [0.0f32; TRIGGER_CORR_POINTS];
                        let mut dot = 0.0f32;
                        let mut cand_energy = 0.0f32;

                        let c_start = center_f - corr_half_f;
                        for (m, dst_feat) in cand_feat.iter_mut().enumerate() {
                            let sample_pos = c_start + (m as f32) * corr_step_f;
                            let val = sample_cubic_catmull_rom(audio_data, sample_pos) - dc_mean;
                            *dst_feat = val;
                            let w = self.corr_window[m];
                            cand_energy += w * val * val;
                            if use_corr {
                                dot += w * val * self.reference_wave[m];
                            }
                        }

                        let corr = if use_corr && cand_energy > 1e-7 {
                            (dot / (cand_energy * ref_energy).sqrt()).clamp(-1.0, 1.0)
                        } else {
                            0.0
                        };

                        let corr_boost = if use_corr {
                            1.0 + 2.8 * ((corr + 1.0) * 0.5).powi(2)
                        } else {
                            1.0
                        };
                        let score = swing_norm.powf(0.55) * corr_boost * (0.93 + 0.07 * recency);

                        if score > best_score {
                            best_score = score;
                            best_center_f = center_f;
                            best_corr = corr;
                            best_feat = cand_feat;
                        }
                    }

                    if best_score >= 0.0 {
                        // Sub-sample parabolic correlation refinement around the winning zero-crossing
                        // absorbs residual phase perturbation from non-harmonic transients while staying
                        // tethered to the rising zero-crossing node.
                        if use_corr && best_corr > 0.65 {
                            let eval_corr_at =
                                |center_cand: f32| -> (f32, [f32; TRIGGER_CORR_POINTS]) {
                                    let mut feat = [0.0f32; TRIGGER_CORR_POINTS];
                                    let mut dot = 0.0f32;
                                    let mut energy = 0.0f32;
                                    let c_start = center_cand - corr_half_f;
                                    for (m, dst_feat) in feat.iter_mut().enumerate() {
                                        let sample_pos = c_start + (m as f32) * corr_step_f;
                                        let val = sample_cubic_catmull_rom(audio_data, sample_pos)
                                            - dc_mean;
                                        *dst_feat = val;
                                        let w = self.corr_window[m];
                                        energy += w * val * val;
                                        dot += w * val * self.reference_wave[m];
                                    }
                                    let c = if energy > 1e-7 {
                                        (dot / (energy * ref_energy).sqrt()).clamp(-1.0, 1.0)
                                    } else {
                                        0.0
                                    };
                                    (c, feat)
                                };

                            for &step_rad in &[1.5f32, 0.5f32] {
                                let (c_m, feat_m) = eval_corr_at(best_center_f - step_rad);
                                let (c_p, feat_p) = eval_corr_at(best_center_f + step_rad);
                                if c_m > best_corr && c_m >= c_p {
                                    best_center_f -= step_rad * 0.65;
                                    best_corr = c_m;
                                    best_feat = feat_m;
                                } else if c_p > best_corr && c_p > c_m {
                                    best_center_f += step_rad * 0.65;
                                    best_corr = c_p;
                                    best_feat = feat_p;
                                } else {
                                    let denom = 2.0 * best_corr - c_m - c_p;
                                    if denom > 1e-6 {
                                        let delta = 0.5 * (c_p - c_m) / denom;
                                        let shift = delta.clamp(-0.5, 0.5) * step_rad * 0.75;
                                        let (c_ref, feat_ref) = eval_corr_at(best_center_f + shift);
                                        if c_ref >= best_corr {
                                            best_center_f += shift;
                                            best_corr = c_ref;
                                            best_feat = feat_ref;
                                        }
                                    }
                                }
                            }
                        }

                        let adapt = if !self.has_reference || best_corr < 0.15 {
                            1.0
                        } else {
                            0.20
                        };
                        for (r_dst, &f_src) in self.reference_wave.iter_mut().zip(best_feat.iter())
                        {
                            *r_dst = *r_dst * (1.0 - adapt) + f_src * adapt;
                        }
                        self.has_reference = true;
                    }
                }
            }
        }

        // Resample the 23ms window centered on best_center_f with sub-sample precision
        let start_f = (best_center_f - half_window_f).max(0.0);
        let anti_alias_d = 0.35 * stride;
        let use_aa = stride > 1.25;

        for (i, dst) in out_waveform.iter_mut().enumerate() {
            let pos_f = start_f + (i as f32) * stride;
            *dst = if use_aa {
                0.25 * sample_cubic_catmull_rom(audio_data, pos_f - anti_alias_d)
                    + 0.50 * sample_cubic_catmull_rom(audio_data, pos_f)
                    + 0.25 * sample_cubic_catmull_rom(audio_data, pos_f + anti_alias_d)
            } else {
                sample_cubic_catmull_rom(audio_data, pos_f)
            };
        }
    }
}

pub fn spawn_dsp_thread(
    rx: Receiver<DspMessage>,
    shared_state: Arc<Mutex<AppState>>,
    sample_rate: u32,
    max_frequency: f32,
    window_size: usize,
) {
    spawn_dsp_thread_with_recycle(
        rx,
        shared_state,
        sample_rate,
        max_frequency,
        window_size,
        None,
    );
}

pub(in crate::audio) fn spawn_dsp_thread_with_recycle(
    rx: Receiver<DspMessage>,
    shared_state: Arc<Mutex<AppState>>,
    sample_rate: u32,
    max_frequency: f32,
    window_size: usize,
    recycle_tx_override: Option<Sender<DspMessage>>,
) {
    std::thread::spawn(move || {
        let mut binned_data = vec![0.0; 1024];
        let mut gpu_spectrum = vec![0.0f32; 32 * 1024 * 2];

        // Pre-compute Hann window coefficients
        let hann_window: Vec<f32> = (0..window_size)
            .map(|i| {
                0.5 * (1.0
                    - (2.0 * std::f32::consts::PI * i as f32 / (window_size - 1) as f32).cos())
            })
            .collect();

        // Pre-plan real-to-complex FFT (~2x faster than full complex FFT on real audio inputs)
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(window_size);
        let mut real_input = r2c.make_input_vec();
        let mut spectrum_buf = r2c.make_output_vec();
        let mut scratch_buf = r2c.make_scratch_vec();
        let nyquist = window_size / 2;
        let mut magnitudes = vec![0.0f32; nyquist + 1];

        // Precompute log-spaced bin interpolation mapping for mono & GPU spectrum
        let resolution = sample_rate as f32 / window_size as f32; // Hz per FFT bin
        let min_freq = 20.0f32;
        let max_f = max_frequency.max(min_freq * 2.0);
        let num_bins = 1024;

        struct BinMapping {
            i0: usize,
            i1: usize,
            frac: f32,
        }
        struct MonoBinRange {
            start: usize,
            safe_end: usize,
            nearest: usize,
            use_range: bool,
        }
        let mut log_bin_mappings = Vec::with_capacity(num_bins);
        let mut mono_bin_ranges = Vec::with_capacity(num_bins);
        for i in 0..num_bins {
            let t0 = i as f32 / num_bins as f32;
            let t1 = (i + 1) as f32 / num_bins as f32;
            let freq_start = min_freq * (max_f / min_freq).powf(t0);
            let freq_end = min_freq * (max_f / min_freq).powf(t1);

            let idx = freq_start / resolution;
            let idx_clamped = idx.clamp(0.0, nyquist as f32);
            let i0 = (idx_clamped.floor() as usize).min(nyquist);
            let i1 = (i0 + 1).min(nyquist);
            let frac = (idx_clamped - i0 as f32).clamp(0.0, 1.0);
            log_bin_mappings.push(BinMapping { i0, i1, frac });

            let idx_end = freq_end / resolution;
            let use_range = (idx_end - idx) >= 1.0;
            let start = (idx.ceil() as usize).min(nyquist);
            let safe_end = ((idx_end.floor() as usize) + 1).min(nyquist + 1);
            let nearest = (((idx + idx_end) * 0.5).round() as usize).min(nyquist);
            mono_bin_ranges.push(MonoBinRange {
                start,
                safe_end,
                nearest,
                use_range,
            });
        }

        // Sample-rate-invariant normalization matching 1.0 / sqrt(8192) at 44.1/48 kHz (N=8192)
        // and scaling proportionally at 96/192 kHz (N=16384/32768) so the Frequency Analysis
        // Y-axis height is consistent across all sample rates.
        let norm_factor = (8192.0_f32).sqrt() / (window_size as f32);
        let mut last_waveform_push = Instant::now();
        let mut scope_trigger = OscilloscopeTriggerState::new();
        let recycle_tx: &Sender<DspMessage> = recycle_tx_override
            .as_ref()
            .unwrap_or(&dsp_recycle_channel().0);

        while let Ok(msg) = rx.recv() {
            let (need_gpu_spectrum, stale_epoch) = if let Ok(mut state) = shared_state.lock() {
                state.sync_rt_control();
                if msg.seek_epoch < state.seek_epoch {
                    (false, true)
                } else {
                    let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
                    let is_fullscreen_video = state.video_mode == 3
                        && (state.has_video_stream || state.video_frame_rx.is_some());
                    let need_gpu =
                        !is_fullscreen_video && (vis_def.id == 6 || vis_def.requires_resynth);
                    (need_gpu, false)
                }
            } else {
                (false, false)
            };

            if stale_epoch {
                let _ = recycle_tx.try_send(msg);
                continue;
            }

            let num_channels = msg
                .channel_audio_data
                .iter()
                .take_while(|ch| !ch.is_empty())
                .count()
                .min(32);
            let fft_elapsed = if msg.audio_data.is_empty() {
                0.0
            } else {
                let fft_start = Instant::now();

                // 1. Apply Hann window with zero-phase `fftshift` rotation for mono channel
                for i in 0..window_size {
                    let sample = *msg.audio_data.get(i).unwrap_or(&0.0);
                    real_input[i] = sample * hann_window[i];
                }
                real_input.rotate_left(window_size / 2);

                // Run real-to-complex FFT
                let _ =
                    r2c.process_with_scratch(&mut real_input, &mut spectrum_buf, &mut scratch_buf);

                // Compute magnitudes (positive frequencies 0..=nyquist inclusive)
                for i in 0..=nyquist {
                    magnitudes[i] = spectrum_buf[i].norm() * norm_factor;
                }

                // Logarithmic binning into 1024 display bins (mono) using precomputed ranges
                for (bin, range) in binned_data.iter_mut().zip(mono_bin_ranges.iter()) {
                    let mut max_val: f32 = 0.0;
                    if range.use_range {
                        if range.start < range.safe_end {
                            for &mag in &magnitudes[range.start..range.safe_end] {
                                max_val = max_val.max(mag);
                            }
                        }
                    } else {
                        max_val = magnitudes[range.nearest];
                    }
                    *bin = (max_val * 100.0).clamp(0.0, 100.0);
                }

                // 2. Process multi-channel FFT ONLY when active visualizer requires GPU spectrum (ID 6 or requires_resynth)
                if need_gpu_spectrum {
                    gpu_spectrum.fill(0.0);
                    if num_channels > 0 {
                        for c in 0..num_channels {
                            let channel_data = &msg.channel_audio_data[c];
                            for i in 0..window_size {
                                let sample = *channel_data.get(i).unwrap_or(&0.0);
                                real_input[i] = sample * hann_window[i];
                            }
                            real_input.rotate_left(window_size / 2);
                            let _ = r2c.process_with_scratch(
                                &mut real_input,
                                &mut spectrum_buf,
                                &mut scratch_buf,
                            );

                            for (i, (mapping, range)) in log_bin_mappings
                                .iter()
                                .zip(mono_bin_ranges.iter())
                                .enumerate()
                            {
                                let (re, im) = if range.use_range && range.start < range.safe_end {
                                    let mut best = spectrum_buf[range.start];
                                    let mut best_norm_sq = best.norm_sqr();
                                    for &val in &spectrum_buf[range.start + 1..range.safe_end] {
                                        let nsq = val.norm_sqr();
                                        if nsq > best_norm_sq {
                                            best_norm_sq = nsq;
                                            best = val;
                                        }
                                    }
                                    (best.re, best.im)
                                } else {
                                    let c0 = spectrum_buf[mapping.i0];
                                    let c1 = spectrum_buf[mapping.i1];
                                    (
                                        c0.re * (1.0 - mapping.frac) + c1.re * mapping.frac,
                                        c0.im * (1.0 - mapping.frac) + c1.im * mapping.frac,
                                    )
                                };

                                let norm_re = re * norm_factor;
                                let norm_im = im * norm_factor;

                                let out_idx = c * 1024 * 2 + i * 2;
                                gpu_spectrum[out_idx] = norm_re;
                                gpu_spectrum[out_idx + 1] = norm_im;
                            }
                        }
                    } else {
                        // Fallback to mono channel FFT mapped to channel 0
                        for (i, (mapping, range)) in log_bin_mappings
                            .iter()
                            .zip(mono_bin_ranges.iter())
                            .enumerate()
                        {
                            let (re, im) = if range.use_range && range.start < range.safe_end {
                                let mut best = spectrum_buf[range.start];
                                let mut best_norm_sq = best.norm_sqr();
                                for &val in &spectrum_buf[range.start + 1..range.safe_end] {
                                    let nsq = val.norm_sqr();
                                    if nsq > best_norm_sq {
                                        best_norm_sq = nsq;
                                        best = val;
                                    }
                                }
                                (best.re, best.im)
                            } else {
                                let c0 = spectrum_buf[mapping.i0];
                                let c1 = spectrum_buf[mapping.i1];
                                (
                                    c0.re * (1.0 - mapping.frac) + c1.re * mapping.frac,
                                    c0.im * (1.0 - mapping.frac) + c1.im * mapping.frac,
                                )
                            };

                            let norm_re = re * norm_factor;
                            let norm_im = im * norm_factor;

                            let out_idx = i * 2;
                            gpu_spectrum[out_idx] = norm_re;
                            gpu_spectrum[out_idx + 1] = norm_im;
                        }
                    }
                }

                fft_start.elapsed().as_secs_f32() * 1_000_000.0
            };

            // Sync to UI state
            if let Ok(mut state) = shared_state.lock() {
                if msg.seek_epoch < state.seek_epoch {
                    state.sync_rt_control();
                    drop(state);
                    let _ = recycle_tx.try_send(msg);
                    continue;
                }

                if state.rt_control.take_track_ended() {
                    state.track_ended = true;
                }
                if state.rt_control.take_audio_device_lost() {
                    state.audio_device_lost = true;
                }

                let paused_loaded = state.is_paused && state.file_loaded;
                if paused_loaded || msg.audio_data.is_empty() {
                    state.stats.fft_us = 0.0;
                    if paused_loaded {
                        state.raw_spectrum_data.fill(0.0);
                        state.gpu_spectrum_data.fill(0.0);
                        state.raw_channel_vus.fill(0.0);
                    }
                } else {
                    // Decay/smooth the execution stats for readability
                    state.stats.fft_us = state.stats.fft_us * 0.9 + fft_elapsed * 0.1;
                    state.raw_spectrum_data.copy_from_slice(&binned_data);
                    if need_gpu_spectrum {
                        if state.gpu_spectrum_data.len() != gpu_spectrum.len() {
                            state.gpu_spectrum_data.resize(gpu_spectrum.len(), 0.0);
                        }
                        state.gpu_spectrum_data.copy_from_slice(&gpu_spectrum);
                    }
                }

                if !paused_loaded {
                    state.raw_channel_vus.clear();
                    state.raw_channel_vus.extend_from_slice(&msg.channel_vus);
                }
                if !msg.channel_vus.is_empty() {
                    state.num_channels = if state
                        .surround_upmix
                        .is_active_for(state.source_channels, state.hardware_channels)
                    {
                        state.hardware_channels.min(12)
                    } else {
                        msg.channel_vus.len() as i32
                    };
                }

                if !paused_loaded && !msg.channel_audio_data.is_empty() {
                    if state.raw_audio_channels.len() != num_channels {
                        state.raw_audio_channels.resize_with(num_channels, Vec::new);
                    }
                    for (dst, src) in state
                        .raw_audio_channels
                        .iter_mut()
                        .zip(msg.channel_audio_data.iter())
                        .take(num_channels)
                    {
                        dst.clear();
                        dst.extend_from_slice(src);
                    }
                }

                if !paused_loaded && !msg.audio_data.is_empty() {
                    // --- Waveform extraction (Zero-Phase Fundamental + Cross-Correlation Standing-Wave Trigger) ---
                    let visual_width = state.visual_width.clamp(128, 2048) as usize;
                    let target_fps = state.target_fps.clamp(30, 500);
                    let waveform_push_interval =
                        std::time::Duration::from_secs_f64(1.0 / target_fps as f64);

                    scope_trigger.extract_triggered_waveform(
                        &msg.audio_data,
                        sample_rate,
                        visual_width,
                        msg.seek_epoch,
                        &mut state.raw_waveform,
                    );

                    let now = Instant::now();
                    if now.duration_since(last_waveform_push) >= waveform_push_interval {
                        while state.waveform_history.len() >= 144 {
                            state.waveform_history.pop_front();
                        }
                        let wave_clone = state.raw_waveform.clone();
                        state.waveform_history.push_back(wave_clone);
                        state.waveform_history_push_count += 1;
                        last_waveform_push = now;
                    }
                }

                if msg.bpm != 0 {
                    state.bpm = msg.bpm;
                }
                if msg.speed != 0 {
                    state.speed = msg.speed;
                }
                if state.duration_seconds > 0.0 {
                    state.current_seconds = msg.current_seconds.min(state.duration_seconds);
                } else {
                    state.current_seconds = msg.current_seconds;
                }
                state.current_tracker_row_string.clear();
                state
                    .current_tracker_row_string
                    .push_str(&msg.current_row_string);

                // --- Lookahead Timeline Queue Pruning ---
                while let Some(&(t, _)) = state.lookahead_queue.front() {
                    if t < msg.current_seconds - 2.0 {
                        state.lookahead_queue.pop_front();
                    } else {
                        break;
                    }
                }
                let AppState {
                    ref lookahead_queue,
                    ref raw_waveform,
                    ref raw_channel_vus,
                    current_seconds,
                    ref mut lookahead_timeline,
                    ..
                } = *state;
                update_lookahead_timeline_from_queue(
                    lookahead_queue,
                    raw_waveform,
                    raw_channel_vus,
                    current_seconds,
                    lookahead_timeline,
                );

                if state.current_tracker_order != msg.current_order
                    || state.current_tracker_row != msg.current_row
                {
                    let cur_order = state.current_tracker_order;
                    let cur_row = state.current_tracker_row;

                    if cur_order == msg.current_order && msg.current_row > cur_row {
                        for r in cur_row..msg.current_row {
                            state.tracker_row_history.push_front((cur_order, r));
                        }
                    } else {
                        state.tracker_row_history.push_front((cur_order, cur_row));
                    }

                    state.tracker_row_history.truncate(128);

                    state.current_tracker_order = msg.current_order;
                    state.current_tracker_row = msg.current_row;
                }
                state.sync_rt_control();
            }
            let _ = recycle_tx.try_send(msg);
        }
    });
}
