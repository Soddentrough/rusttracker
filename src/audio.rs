use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use openmpt::module::{Logger, Module};
use rustfft::{FftPlanner, num_complex::Complex};
use std::fs::File;
use std::io::Cursor;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use rustysynth::{MidiFile, MidiFileSequencer, SoundFont, Synthesizer, SynthesizerSettings};

use crate::state::AppState;
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

pub enum PlaybackHandle {
    Cpal(cpal::Stream, Arc<std::sync::atomic::AtomicBool>),
    Bitstream(
        Option<std::thread::JoinHandle<()>>,
        Arc<std::sync::atomic::AtomicBool>,
    ),
    Dummy(
        Option<std::thread::JoinHandle<()>>,
        Arc<std::sync::atomic::AtomicBool>,
    ),
}

impl Drop for PlaybackHandle {
    fn drop(&mut self) {
        match self {
            PlaybackHandle::Cpal(stream, stop_token) => {
                let _ = &stream;
                stop_token.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            PlaybackHandle::Bitstream(handle_opt, stop_token) => {
                stop_token.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(handle) = handle_opt.take() {
                    let _ = handle.join();
                }
            }
            PlaybackHandle::Dummy(handle_opt, stop_token) => {
                stop_token.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(handle) = handle_opt.take() {
                    let _ = handle.join();
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct DspMessage {
    pub audio_data: Vec<f32>,
    pub channel_vus: Vec<f32>,
    pub current_order: i32,
    pub current_row: i32,
    pub bpm: i32,
    pub speed: i32,
    pub current_seconds: f64,
    pub current_row_string: String,
    pub channel_audio_data: Vec<Vec<f32>>,
    pub seek_epoch: u64,
}

// ---------------------------------------------------------
// Automatic Surround & Height Upmixer + 5-Band Crossover EQ
// ---------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct BiquadFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for BiquadFilter {
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

impl BiquadFilter {
    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        if !y.is_finite() {
            self.z1 = 0.0;
            self.z2 = 0.0;
            0.0
        } else {
            y
        }
    }

    #[inline(always)]
    pub fn clear_state(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline(always)]
    pub fn update_coeffs(&mut self, coeffs: Self) {
        self.b0 = coeffs.b0;
        self.b1 = coeffs.b1;
        self.b2 = coeffs.b2;
        self.a1 = coeffs.a1;
        self.a2 = coeffs.a2;
        if (self.b0 - 1.0).abs() < 1e-6
            && self.b1.abs() < 1e-6
            && self.b2.abs() < 1e-6
            && self.a1.abs() < 1e-6
            && self.a2.abs() < 1e-6
        {
            self.z1 = 0.0;
            self.z2 = 0.0;
        }
    }

    /// 2nd-order Linkwitz-Riley (Q = 0.5) or general RBJ Low-Pass filter.
    pub fn lowpass(sample_rate: f32, freq_hz: f32, q: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let f = freq_hz.clamp(20.0, sr * 0.45);
        let w0 = 2.0 * std::f32::consts::PI * f / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q.max(0.1));

        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// 2nd-order Linkwitz-Riley (Q = 0.5) or general RBJ High-Pass filter.
    pub fn highpass(sample_rate: f32, freq_hz: f32, q: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let f = freq_hz.clamp(20.0, sr * 0.45);
        let w0 = 2.0 * std::f32::consts::PI * f / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q.max(0.1));

        let b0 = (1.0 + cos_w0) * 0.5;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn peaking_eq(sample_rate: f32, freq_hz: f32, q: f32, gain_db: f32) -> Self {
        if gain_db.abs() < 1e-3 {
            return Self::default();
        }
        let sr = sample_rate.max(8000.0);
        let f = freq_hz.clamp(20.0, sr * 0.45);
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * f / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q.max(0.1));

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn low_shelf(sample_rate: f32, freq_hz: f32, q: f32, gain_db: f32) -> Self {
        if gain_db.abs() < 1e-3 {
            return Self::default();
        }
        let sr = sample_rate.max(8000.0);
        let f = freq_hz.clamp(20.0, sr * 0.45);
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * f / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q.max(0.1));
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn high_shelf(sample_rate: f32, freq_hz: f32, q: f32, gain_db: f32) -> Self {
        if gain_db.abs() < 1e-3 {
            return Self::default();
        }
        let sr = sample_rate.max(8000.0);
        let f = freq_hz.clamp(20.0, sr * 0.45);
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * f / sr;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q.max(0.1));
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

/// Zero-allocation circular delay line for Haas acoustic decorrelation across surround/height speakers.
pub struct HaasDelayLine<const N: usize> {
    buf: [f32; N],
    write_idx: usize,
    delay_samples: usize,
}

impl<const N: usize> HaasDelayLine<N> {
    pub fn new(delay_samples: usize) -> Self {
        Self {
            buf: [0.0; N],
            write_idx: 0,
            delay_samples: delay_samples.clamp(1, N.saturating_sub(1)),
        }
    }

    pub fn set_delay_ms(&mut self, sample_rate: u32, delay_ms: f32) {
        let samples = ((sample_rate as f32 * delay_ms * 0.001).round() as usize)
            .clamp(1, N.saturating_sub(1));
        self.delay_samples = samples;
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.buf.fill(0.0);
        self.write_idx = 0;
    }

    #[inline(always)]
    pub fn process(&mut self, input: f32) -> f32 {
        let read_idx = (self.write_idx + N - self.delay_samples) % N;
        let out = self.buf[read_idx];
        self.buf[self.write_idx] = input;
        self.write_idx = (self.write_idx + 1) % N;
        out
    }
}

/// Real-time 7.1.4 Surround & Height Upmixer with 5-Band EQ and Linkwitz-Riley Crossovers.
pub struct SurroundUpmixerDsp {
    sample_rate: u32,
    last_config: crate::state::SurroundUpmixConfig,
    eq_l: [BiquadFilter; 5],
    eq_r: [BiquadFilter; 5],
    sub_lp: BiquadFilter,
    sub_hp_l: BiquadFilter,
    sub_hp_r: BiquadFilter,
    sub_hp_c: BiquadFilter,
    height_lp_l: BiquadFilter,
    height_hp_l: BiquadFilter,
    height_lp_r: BiquadFilter,
    height_hp_r: BiquadFilter,
    delay_sl: HaasDelayLine<2048>,
    delay_sr: HaasDelayLine<2048>,
    delay_rl: HaasDelayLine<2048>,
    delay_rr: HaasDelayLine<2048>,
    delay_tfl: HaasDelayLine<2048>,
    delay_tfr: HaasDelayLine<2048>,
    delay_trl: HaasDelayLine<2048>,
    delay_trr: HaasDelayLine<2048>,
    blend: f32,
    blend_initialized: bool,
}

impl SurroundUpmixerDsp {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate.max(8000);
        let default_cfg = crate::state::SurroundUpmixConfig::default();
        let mut dsp = Self {
            sample_rate: sr,
            last_config: default_cfg,
            eq_l: [BiquadFilter::default(); 5],
            eq_r: [BiquadFilter::default(); 5],
            sub_lp: BiquadFilter::default(),
            sub_hp_l: BiquadFilter::default(),
            sub_hp_r: BiquadFilter::default(),
            sub_hp_c: BiquadFilter::default(),
            height_lp_l: BiquadFilter::default(),
            height_hp_l: BiquadFilter::default(),
            height_lp_r: BiquadFilter::default(),
            height_hp_r: BiquadFilter::default(),
            delay_sl: HaasDelayLine::new(115),
            delay_sr: HaasDelayLine::new(115),
            delay_rl: HaasDelayLine::new(250),
            delay_rr: HaasDelayLine::new(250),
            delay_tfl: HaasDelayLine::new(158),
            delay_tfr: HaasDelayLine::new(158),
            delay_trl: HaasDelayLine::new(307),
            delay_trr: HaasDelayLine::new(307),
            blend: 1.0,
            blend_initialized: false,
        };
        dsp.rebuild_all_filters(&default_cfg, sr);
        dsp
    }

    fn rebuild_all_filters(&mut self, cfg: &crate::state::SurroundUpmixConfig, sample_rate: u32) {
        let sr = sample_rate.max(8000) as f32;
        self.sample_rate = sample_rate.max(8000);
        self.last_config = *cfg;

        let freqs = crate::state::SurroundUpmixConfig::EQ_BAND_FREQS;
        let q_shelf = std::f32::consts::FRAC_1_SQRT_2;
        let b0 = BiquadFilter::low_shelf(sr, freqs[0], q_shelf, cfg.band_gains_db[0]);
        let b1 = BiquadFilter::peaking_eq(sr, freqs[1], 1.0, cfg.band_gains_db[1]);
        let b2 = BiquadFilter::peaking_eq(sr, freqs[2], 1.0, cfg.band_gains_db[2]);
        let b3 = BiquadFilter::peaking_eq(sr, freqs[3], 1.0, cfg.band_gains_db[3]);
        let b4 = BiquadFilter::high_shelf(sr, freqs[4], q_shelf, cfg.band_gains_db[4]);
        let bands = [b0, b1, b2, b3, b4];
        for ((el, er), &b) in self
            .eq_l
            .iter_mut()
            .zip(self.eq_r.iter_mut())
            .zip(bands.iter())
        {
            el.update_coeffs(b);
            er.update_coeffs(b);
        }

        // Linkwitz-Riley 2nd-order (Q = 0.5) LFE low-pass and Height low-pass/high-pass crossovers
        let sub_lp = BiquadFilter::lowpass(sr, cfg.sub_crossover_hz.clamp(40.0, 250.0), 0.5);
        self.sub_lp.update_coeffs(sub_lp);
        let sub_hp = BiquadFilter::highpass(sr, cfg.sub_crossover_hz.clamp(40.0, 250.0), 0.5);
        self.sub_hp_l.update_coeffs(sub_hp);
        self.sub_hp_r.update_coeffs(sub_hp);
        self.sub_hp_c.update_coeffs(sub_hp);

        let h_fc = cfg.height_crossover_hz.clamp(800.0, 10000.0);
        let h_lp = BiquadFilter::lowpass(sr, h_fc, 0.5);
        let h_hp = BiquadFilter::highpass(sr, h_fc, 0.5);
        self.height_lp_l.update_coeffs(h_lp);
        self.height_hp_l.update_coeffs(h_hp);
        self.height_lp_r.update_coeffs(h_lp);
        self.height_hp_r.update_coeffs(h_hp);

        self.delay_sl.set_delay_ms(self.sample_rate, 2.4);
        self.delay_sr.set_delay_ms(self.sample_rate, 2.4);
        self.delay_rl.set_delay_ms(self.sample_rate, 5.2);
        self.delay_rr.set_delay_ms(self.sample_rate, 5.2);
        self.delay_tfl.set_delay_ms(self.sample_rate, 3.3);
        self.delay_tfr.set_delay_ms(self.sample_rate, 3.3);
        self.delay_trl.set_delay_ms(self.sample_rate, 6.4);
        self.delay_trr.set_delay_ms(self.sample_rate, 6.4);
    }

    #[inline(always)]
    pub fn sync_config(&mut self, cfg: &crate::state::SurroundUpmixConfig, sample_rate: u32) {
        let sr = sample_rate.max(8000);
        if sr != self.sample_rate
            || (cfg.sub_crossover_hz - self.last_config.sub_crossover_hz).abs() > 0.1
            || (cfg.height_crossover_hz - self.last_config.height_crossover_hz).abs() > 0.5
            || cfg.band_gains_db != self.last_config.band_gains_db
        {
            self.rebuild_all_filters(cfg, sr);
        } else {
            self.last_config = *cfg;
        }
    }

    #[inline(always)]
    fn soft_limit(x: f32) -> f32 {
        // Gentle cubic soft knee above |x| = 0.85 to prevent harsh digital clipping on +12dB EQ boosts
        let ax = x.abs();
        if ax <= 0.85 {
            x
        } else {
            let excess = (ax - 0.85) / 0.45;
            let compressed = 0.85 + 0.15 * (excess / (1.0 + excess));
            compressed.copysign(x).clamp(-1.0, 1.0)
        }
    }

    /// Apply the 5-band parametric/shelf routing contour cascade to a stereo `(L, R)` pair.
    /// This shapes only the signal extracted for the surround bed, height, and LFE channels;
    /// it is never applied as a master EQ on stereo/mono devices or on the Front L/R anchor channels.
    #[inline(always)]
    pub fn process_routing_eq(
        &mut self,
        l_in: f32,
        r_in: f32,
        cfg: &crate::state::SurroundUpmixConfig,
    ) -> (f32, f32) {
        if cfg.band_gains_db.iter().all(|&g| g.abs() < 1e-3) {
            return (l_in, r_in);
        }
        let mut l_eq = l_in;
        let mut r_eq = r_in;
        for (el, er) in self.eq_l.iter_mut().zip(self.eq_r.iter_mut()) {
            l_eq = el.process(l_eq);
            r_eq = er.process(r_eq);
        }
        (Self::soft_limit(l_eq), Self::soft_limit(r_eq))
    }

    /// Process a stereo `(L, R)` frame for a **Bed-Only** surround system (`3..=8` channels: 4.0, 5.1, 7.1).
    /// Returns 8 bed channels `[0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:RL, 7:RR]`.
    ///
    /// Because no overhead height speakers exist in a Bed-Only layout:
    /// - Front L/R (`FL, FR`) and Center (`FC`) carry the uncolored front stage.
    /// - LFE (`LFE`) carries sub-bass below `sub_crossover_hz`.
    /// - Side/Rear bed speakers (`SL, SR, RL, RR`) receive the full-bandwidth routing-shaped
    ///   and Haas-decorrelated surround signal without a `height_crossover_hz` split or height fold-down.
    #[inline(always)]
    pub fn process_stereo_to_bed_only(
        &mut self,
        l_in: f32,
        r_in: f32,
        cfg: &crate::state::SurroundUpmixConfig,
    ) -> [f32; 8] {
        let mid_in = (l_in + r_in) * 0.5;
        let fl = Self::soft_limit(l_in * cfg.front_level);
        let fr = Self::soft_limit(r_in * cfg.front_level);
        let fc_hp = -self.sub_hp_c.process(mid_in);
        let fc = Self::soft_limit(fc_hp * cfg.center_level * std::f32::consts::FRAC_1_SQRT_2);

        let (l_routed, r_routed) = self.process_routing_eq(l_in, r_in, cfg);
        let mid_routed = (l_routed + r_routed) * 0.5;
        let diff_routed = (l_routed - r_routed) * 0.5;
        // ITU-R BS.775 / Dolby Surround center-steering: attenuate common-mode phantom center in surrounds
        // so correlated vocals/kick/bass stay anchored in the front stage while panned/stereo content envelops.
        let common_center = (mid_routed.abs() - diff_routed.abs())
            .max(0.0)
            .copysign(mid_routed);
        let l_bed = -self.sub_hp_l.process(l_routed - common_center * 0.60);
        let r_bed = -self.sub_hp_r.process(r_routed - common_center * 0.60);
        let side = (l_routed - r_routed) * 0.5 * cfg.stereo_width.clamp(0.0, 2.0);
        let l_surr = l_bed + side * 0.35;
        let r_surr = r_bed - side * 0.35;

        let lfe_low = self.sub_lp.process(mid_routed);

        let lfe = Self::soft_limit(lfe_low * cfg.lfe_level * std::f32::consts::FRAC_1_SQRT_2);

        let sl_raw = self.delay_sl.process(l_surr);
        let sr_raw = self.delay_sr.process(r_surr);
        let sl = Self::soft_limit(sl_raw * cfg.side_rear_level);
        let sr = Self::soft_limit(sr_raw * cfg.side_rear_level);

        let rl_raw = self.delay_rl.process(l_surr + side * 0.22);
        let rr_raw = self.delay_rr.process(r_surr - side * 0.22);
        let rl = Self::soft_limit(rl_raw * (cfg.side_rear_level * 0.92));
        let rr = Self::soft_limit(rr_raw * (cfg.side_rear_level * 0.92));

        [fl, fr, fc, lfe, sl, sr, rl, rr]
    }

    /// Process a stereo `(L, R)` frame for a **Bed + Height** surround system (`> 8` channels: 7.1.2, 5.1.4, 7.1.4)
    /// into 12 discrete spatial channels:
    /// `[0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:RL, 7:RR, 8:TFL, 9:TFR, 10:TRL, 11:TRR]`.
    #[inline(always)]
    pub fn process_stereo_to_7_1_4(
        &mut self,
        l_in: f32,
        r_in: f32,
        cfg: &crate::state::SurroundUpmixConfig,
    ) -> [f32; 12] {
        // 1. Uncolored Front L/R & Center anchor stage (not passed through surround routing EQ or height crossover)
        let mid_in = (l_in + r_in) * 0.5;
        let fl = Self::soft_limit(l_in * cfg.front_level);
        let fr = Self::soft_limit(r_in * cfg.front_level);
        let fc_hp = -self.sub_hp_c.process(mid_in);
        let fc = Self::soft_limit(fc_hp * cfg.center_level * std::f32::consts::FRAC_1_SQRT_2);

        // 2. 5-band routing contour & Mid/Side width shaping for Surround Bed, Height, and LFE
        let (l_eq, r_eq) = self.process_routing_eq(l_in, r_in, cfg);
        let mid = (l_eq + r_eq) * 0.5;
        let diff = (l_eq - r_eq) * 0.5;
        let common_center = (mid.abs() - diff.abs()).max(0.0).copysign(mid);
        let l_amb = l_eq - common_center * 0.60;
        let r_amb = r_eq - common_center * 0.60;
        let side = (l_eq - r_eq) * 0.5 * cfg.stereo_width.clamp(0.0, 2.0);
        let l_wide = l_amb + side * 0.35;
        let r_wide = r_amb - side * 0.35;

        // 3. LFE Subwoofer low-pass extraction (< sub_crossover_hz)
        let lfe_low = self.sub_lp.process(mid);

        let lfe = Self::soft_limit(lfe_low * cfg.lfe_level * std::f32::consts::FRAC_1_SQRT_2);

        // 4. Linkwitz-Riley 2nd-order (Q = 0.5) crossover at height_crossover_hz splitting Bed vs. Height.
        // Invert high-pass polarity (-height_hp) so LP and HP sum in phase at the crossover frequency.
        let l_bed_hp = -self.sub_hp_l.process(l_amb);
        let r_bed_hp = -self.sub_hp_r.process(r_amb);
        let l_low = self.height_lp_l.process(l_bed_hp);
        let l_high = -self.height_hp_l.process(l_wide);
        let r_low = self.height_lp_r.process(r_bed_hp);
        let r_high = -self.height_hp_r.process(r_wide);

        // 5. Left/Right Hemisphere Bed Routing (< height_crossover_hz + high_bleed) & Height Routing (> height_crossover_hz)
        let high_bleed = cfg.surround_high_bleed.clamp(0.0, 1.0);

        let sl_raw = self.delay_sl.process(l_low + l_high * high_bleed);
        let sr_raw = self.delay_sr.process(r_low + r_high * high_bleed);
        let sl = Self::soft_limit(sl_raw * cfg.side_rear_level);
        let sr = Self::soft_limit(sr_raw * cfg.side_rear_level);

        let rl_raw = self
            .delay_rl
            .process((l_low + side * 0.22) + l_high * (high_bleed * 0.85));
        let rr_raw = self
            .delay_rr
            .process((r_low - side * 0.22) + r_high * (high_bleed * 0.85));
        let rl = Self::soft_limit(rl_raw * (cfg.side_rear_level * 0.92));
        let rr = Self::soft_limit(rr_raw * (cfg.side_rear_level * 0.92));

        let tfl_raw = self.delay_tfl.process(l_high);
        let tfr_raw = self.delay_tfr.process(r_high);
        let tfl = Self::soft_limit(tfl_raw * cfg.height_level);
        let tfr = Self::soft_limit(tfr_raw * cfg.height_level);

        let trl_raw = self.delay_trl.process(l_high + side * 0.28);
        let trr_raw = self.delay_trr.process(r_high - side * 0.28);
        let trl = Self::soft_limit(trl_raw * (cfg.height_level * 0.92));
        let trr = Self::soft_limit(trr_raw * (cfg.height_level * 0.92));

        [fl, fr, fc, lfe, sl, sr, rl, rr, tfl, tfr, trl, trr]
    }

    /// Upmix a stereo `(l_in, r_in)` frame to `hardware_channels` (up to 12 channels):
    /// - For Bed + Height layouts (`> 8` channels, e.g. `10` = 7.1.2 or `12` = 7.1.4): runs the full
    ///   Linkwitz-Riley height crossover split (`process_stereo_to_7_1_4`).
    /// - For Bed-Only layouts (`3..=8` channels, e.g. `4` = 4.0, `6` = 5.1, `8` = 7.1): runs
    ///   `process_stereo_to_bed_only` so surround speakers receive the full high-frequency band
    ///   without losing energy above `height_crossover_hz` to non-existent height speakers.
    #[inline(always)]
    pub fn process_frame_for_hardware(
        &mut self,
        l_in: f32,
        r_in: f32,
        cfg: &crate::state::SurroundUpmixConfig,
        hardware_channels: usize,
    ) -> [f32; 12] {
        if hardware_channels == 1 {
            let mono = ((l_in + r_in) * 0.5).clamp(-1.0, 1.0);
            return [mono, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        }
        if hardware_channels <= 2 {
            return [l_in, r_in, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        }
        // ITU-R BS.775-3 / BS.1770-4 constant-power front/surround energy conservation:
        // Redirect a fraction of common-mode phantom center from FL/FR when FC is active, and scale
        // active front/bed/height speaker groups by 1/sqrt(N_pairs) so total acoustic power matches 2.0 stereo.
        let mid_in = (l_in + r_in) * 0.5;
        let diff_in = (l_in - r_in) * 0.5;
        let common_in = (mid_in.abs() - diff_in.abs()).max(0.0).copysign(mid_in);
        let center_pull = common_in * cfg.front_level * (0.14 * cfg.center_level.clamp(0.0, 2.0));
        let k_s2 = 0.88_f32;

        if crate::state::SurroundUpmixConfig::has_height_channels(hardware_channels as i32) {
            let [fl, fr, fc, lfe, sl, sr, rl, rr, tfl, tfr, trl, trr] =
                self.process_stereo_to_7_1_4(l_in, r_in, cfg);
            let fl_c = ((fl - center_pull) * k_s2).clamp(-1.0, 1.0);
            let fr_c = ((fr - center_pull) * k_s2).clamp(-1.0, 1.0);
            let fc_c = (fc * k_s2).clamp(-1.0, 1.0);
            match hardware_channels {
                9 => [
                    fl_c,
                    fr_c,
                    fc_c,
                    lfe,
                    (sl * 0.64).clamp(-1.0, 1.0),
                    (sr * 0.64).clamp(-1.0, 1.0),
                    (rl * 0.64).clamp(-1.0, 1.0),
                    (rr * 0.64).clamp(-1.0, 1.0),
                    ((tfl + tfr) * 0.45).clamp(-1.0, 1.0),
                    0.0,
                    0.0,
                    0.0,
                ],
                10 => {
                    let h_l = ((tfl + trl * 0.7) * 0.52).clamp(-1.0, 1.0);
                    let h_r = ((tfr + trr * 0.7) * 0.52).clamp(-1.0, 1.0);
                    [
                        fl_c,
                        fr_c,
                        fc_c,
                        lfe,
                        (sl * 0.64).clamp(-1.0, 1.0),
                        (sr * 0.64).clamp(-1.0, 1.0),
                        (rl * 0.64).clamp(-1.0, 1.0),
                        (rr * 0.64).clamp(-1.0, 1.0),
                        h_l,
                        h_r,
                        0.0,
                        0.0,
                    ]
                }
                _ => [
                    fl_c,
                    fr_c,
                    fc_c,
                    lfe,
                    (sl * 0.64).clamp(-1.0, 1.0),
                    (sr * 0.64).clamp(-1.0, 1.0),
                    (rl * 0.64).clamp(-1.0, 1.0),
                    (rr * 0.64).clamp(-1.0, 1.0),
                    (tfl * 0.82).clamp(-1.0, 1.0),
                    (tfr * 0.82).clamp(-1.0, 1.0),
                    (trl * 0.66).clamp(-1.0, 1.0),
                    (trr * 0.66).clamp(-1.0, 1.0),
                ],
            }
        } else {
            let [fl, fr, fc, lfe, sl, sr, rl, rr] =
                self.process_stereo_to_bed_only(l_in, r_in, cfg);
            let fl_c = ((fl - center_pull) * k_s2).clamp(-1.0, 1.0);
            let fr_c = ((fr - center_pull) * k_s2).clamp(-1.0, 1.0);
            let fc_c = (fc * k_s2).clamp(-1.0, 1.0);
            match hardware_channels {
                3 => [
                    ((fl - center_pull) * 0.94).clamp(-1.0, 1.0),
                    ((fr - center_pull) * 0.94).clamp(-1.0, 1.0),
                    (fc * 0.94).clamp(-1.0, 1.0),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
                4 => [
                    (fl * k_s2).clamp(-1.0, 1.0),
                    (fr * k_s2).clamp(-1.0, 1.0),
                    (sl * k_s2).clamp(-1.0, 1.0),
                    (sr * k_s2).clamp(-1.0, 1.0),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
                5 => [
                    fl_c,
                    fr_c,
                    fc_c,
                    (sl * k_s2).clamp(-1.0, 1.0),
                    (sr * k_s2).clamp(-1.0, 1.0),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
                6 => [
                    fl_c,
                    fr_c,
                    fc_c,
                    lfe,
                    (sl * k_s2).clamp(-1.0, 1.0),
                    (sr * k_s2).clamp(-1.0, 1.0),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
                7 => {
                    let rc = ((rl + rr) * 0.5).clamp(-1.0, 1.0);
                    [
                        fl_c,
                        fr_c,
                        fc_c,
                        lfe,
                        (sl * 0.62).clamp(-1.0, 1.0),
                        (sr * 0.62).clamp(-1.0, 1.0),
                        (rc * 0.62).clamp(-1.0, 1.0),
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                    ]
                }
                _ => [
                    fl_c,
                    fr_c,
                    fc_c,
                    lfe,
                    (sl * 0.64).clamp(-1.0, 1.0),
                    (sr * 0.64).clamp(-1.0, 1.0),
                    (rl * 0.64).clamp(-1.0, 1.0),
                    (rr * 0.64).clamp(-1.0, 1.0),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
            }
        }
    }

    /// Upmixes a decoded multi-channel frame `frame_slice` with `source_channels` intrinsic channels
    /// to `hardware_channels`:
    /// - When `source_channels >= hardware_channels` or `hardware_channels <= 2`: passes all discrete
    ///   channels through 1:1 untouched (never folds down native surround to stereo).
    /// - When `source_channels <= 2`: performs full ITU-R BS.775 constant-power mono/stereo-to-surround upmixing
    ///   across all `hardware_channels`.
    /// - When `2 < source_channels < hardware_channels`: preserves all existing discrete channels from
    ///   the media file in their canonical speaker positions and synthesizes the remaining available
    ///   output channels (`FC`, `LFE`, `SL/SR`, `RL/RR`, `TFL..TRR`) so every output speaker is driven.
    #[inline]
    pub fn process_multichannel_frame_for_hardware(
        &mut self,
        frame_slice: &[f32],
        source_channels: usize,
        cfg: &crate::state::SurroundUpmixConfig,
        hardware_channels: usize,
    ) -> [f32; 12] {
        let src_ch = source_channels.max(1);
        if hardware_channels <= 2 || src_ch >= hardware_channels {
            let mut out = [0.0f32; 12];
            let copy_len = frame_slice.len().min(hardware_channels).min(12);
            out[..copy_len].copy_from_slice(&frame_slice[..copy_len]);
            return out;
        }
        if src_ch <= 2 {
            let (l_in, r_in) = extract_stereo_pair_from_frame(frame_slice, src_ch);
            return self.process_frame_for_hardware(l_in, r_in, cfg, hardware_channels);
        }

        let fl = frame_slice.first().copied().unwrap_or(0.0);
        let fr = frame_slice.get(1).copied().unwrap_or(fl);
        const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

        match src_ch {
            // 3.0 (FL, FR, FC) -> 4.0 / 5.0 / 5.1 / 6.1 / 7.1 / 7.1.x:
            // Preserve discrete FL, FR, FC and synthesize LFE + Surround Bed (+ Height if >8ch) from (FL, FR).
            3 => {
                let fc = frame_slice.get(2).copied().unwrap_or(0.0);
                let mut out = self.process_frame_for_hardware(fl, fr, cfg, hardware_channels);
                if hardware_channels == 4 {
                    // 4.0 Quad has [FL, FR, SL, SR] (no physical Center speaker)
                    out[0] = (fl + fc * INV_SQRT2).clamp(-1.0, 1.0);
                    out[1] = (fr + fc * INV_SQRT2).clamp(-1.0, 1.0);
                } else {
                    out[0] = fl.clamp(-1.0, 1.0);
                    out[1] = fr.clamp(-1.0, 1.0);
                    out[2] = fc.clamp(-1.0, 1.0);
                }
                out
            }
            // 4.0 Quad (FL, FR, SL, SR) -> 5.0 / 5.1 / 6.1 / 7.1 / 7.1.x:
            // Preserve discrete FL, FR, SL, SR; synthesize FC + LFE from (FL, FR) and RL/RR (+ Height) from (SL, SR).
            4 => {
                let sl = frame_slice.get(2).copied().unwrap_or(0.0);
                let sr = frame_slice.get(3).copied().unwrap_or(0.0);
                let l_comb = (fl + sl * INV_SQRT2) * 0.75;
                let r_comb = (fr + sr * INV_SQRT2) * 0.75;
                let mut out =
                    self.process_frame_for_hardware(l_comb, r_comb, cfg, hardware_channels);
                out[0] = fl.clamp(-1.0, 1.0);
                out[1] = fr.clamp(-1.0, 1.0);
                if hardware_channels == 5 {
                    // 5.0: [FL, FR, FC, SL, SR]
                    out[3] = sl.clamp(-1.0, 1.0);
                    out[4] = sr.clamp(-1.0, 1.0);
                } else if hardware_channels == 6 {
                    // 5.1: [FL, FR, FC, LFE, SL, SR]
                    out[4] = sl.clamp(-1.0, 1.0);
                    out[5] = sr.clamp(-1.0, 1.0);
                } else {
                    // >= 7 channels (6.1 / 7.1 / 7.1.x): scale discrete SL/SR by 1/sqrt(2) as rear pair is added
                    out[4] = (sl * INV_SQRT2).clamp(-1.0, 1.0);
                    out[5] = (sr * INV_SQRT2).clamp(-1.0, 1.0);
                }
                out
            }
            // 5.0 (FL, FR, FC, SL, SR) -> 5.1 / 6.1 / 7.1 / 7.1.x:
            // Preserve discrete FL, FR, FC, SL, SR; synthesize LFE and RL/RR (+ Height).
            5 => {
                let fc = frame_slice.get(2).copied().unwrap_or(0.0);
                let sl = frame_slice.get(3).copied().unwrap_or(0.0);
                let sr = frame_slice.get(4).copied().unwrap_or(0.0);
                let l_comb = (fl + fc * 0.5 + sl * INV_SQRT2) * 0.7;
                let r_comb = (fr + fc * 0.5 + sr * INV_SQRT2) * 0.7;
                let mut out =
                    self.process_frame_for_hardware(l_comb, r_comb, cfg, hardware_channels);
                out[0] = fl.clamp(-1.0, 1.0);
                out[1] = fr.clamp(-1.0, 1.0);
                out[2] = fc.clamp(-1.0, 1.0);
                if hardware_channels == 6 {
                    out[4] = sl.clamp(-1.0, 1.0);
                    out[5] = sr.clamp(-1.0, 1.0);
                } else {
                    out[4] = (sl * INV_SQRT2).clamp(-1.0, 1.0);
                    out[5] = (sr * INV_SQRT2).clamp(-1.0, 1.0);
                }
                out
            }
            // 5.1 (FL, FR, FC, LFE, SL, SR) -> 6.1 / 7.1 / 7.1.x:
            // Preserve discrete FL, FR, FC, LFE, SL, SR and expand (SL, SR) into RL, RR (+ Height).
            6 => {
                let sl = frame_slice.get(4).copied().unwrap_or(0.0);
                let sr = frame_slice.get(5).copied().unwrap_or(0.0);
                let mut out = self.process_frame_for_hardware(sl, sr, cfg, hardware_channels);
                for (c, out_c) in out.iter_mut().enumerate().take(4) {
                    *out_c = frame_slice.get(c).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
                }
                out[4] = (sl * INV_SQRT2).clamp(-1.0, 1.0);
                out[5] = (sr * INV_SQRT2).clamp(-1.0, 1.0);
                out
            }
            // 6.1 (FL, FR, FC, LFE, BC, SL, SR) -> 7.1 / 7.1.x:
            // Preserve FL, FR, FC, LFE, SL, SR and expand BC + (SL, SR) into RL, RR (+ Height).
            7 => {
                let bc = frame_slice.get(4).copied().unwrap_or(0.0);
                let sl = frame_slice.get(5).copied().unwrap_or(0.0);
                let sr = frame_slice.get(6).copied().unwrap_or(0.0);
                let mut out = self.process_frame_for_hardware(
                    (sl + bc * INV_SQRT2) * INV_SQRT2,
                    (sr + bc * INV_SQRT2) * INV_SQRT2,
                    cfg,
                    hardware_channels,
                );
                for (c, out_c) in out.iter_mut().enumerate().take(4) {
                    *out_c = frame_slice.get(c).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
                }
                out[4] = sl.clamp(-1.0, 1.0);
                out[5] = sr.clamp(-1.0, 1.0);
                out[6] = ((out[6] + bc * INV_SQRT2) * 0.75).clamp(-1.0, 1.0);
                out[7] = ((out[7] + bc * INV_SQRT2) * 0.75).clamp(-1.0, 1.0);
                out
            }
            // >= 8 channels (e.g. 7.1 -> 7.1.2 / 7.1.4):
            // Preserve all existing discrete channels 0..src_ch and expand surround bed into remaining channels.
            _ => {
                let sur_l = (frame_slice.get(4).copied().unwrap_or(0.0)
                    + frame_slice.get(6).copied().unwrap_or(0.0))
                    * INV_SQRT2;
                let sur_r = (frame_slice.get(5).copied().unwrap_or(0.0)
                    + frame_slice.get(7).copied().unwrap_or(0.0))
                    * INV_SQRT2;
                let mut out = self.process_frame_for_hardware(sur_l, sur_r, cfg, hardware_channels);
                let copy_n = src_ch.min(frame_slice.len()).min(12);
                out[..copy_n].copy_from_slice(&frame_slice[..copy_n]);
                out
            }
        }
    }

    /// Clears internal biquad filter states and Haas delay-line buffers so transitioning from
    /// completely bypassed (`blend == 0.0`) into active upmixing never replays stale delay samples.
    pub fn clear_state(&mut self) {
        for (el, er) in self.eq_l.iter_mut().zip(self.eq_r.iter_mut()) {
            el.clear_state();
            er.clear_state();
        }
        self.sub_lp.clear_state();
        self.sub_hp_l.clear_state();
        self.sub_hp_r.clear_state();
        self.sub_hp_c.clear_state();
        self.height_lp_l.clear_state();
        self.height_hp_l.clear_state();
        self.height_lp_r.clear_state();
        self.height_hp_r.clear_state();
        self.delay_sl.clear();
        self.delay_sr.clear();
        self.delay_rl.clear();
        self.delay_rr.clear();
        self.delay_tfl.clear();
        self.delay_tfr.clear();
        self.delay_trl.clear();
        self.delay_trr.clear();
    }

    /// Returns `true` when upmixing is either actively enabled or in the middle of a smooth
    /// 65 ms raised-cosine crossfade transition (`blend > 0.0`).
    #[inline(always)]
    pub fn is_blending_or_active(&self, target_active: bool) -> bool {
        target_active || !self.blend_initialized || self.blend > 0.0
    }

    /// Advances the 65 ms raised-cosine crossfade envelope toward `target_active` (`1.0` when
    /// active, `0.0` when bypassed) and blends between the dry native frame and the upmixed frame.
    #[inline]
    pub fn process_multichannel_frame_for_hardware_blended(
        &mut self,
        frame_slice: &[f32],
        source_channels: usize,
        cfg: &crate::state::SurroundUpmixConfig,
        hardware_channels: usize,
        target_active: bool,
    ) -> [f32; 12] {
        let src_ch = source_channels.max(1);
        let effective_target = target_active && hardware_channels > 2 && src_ch < hardware_channels;
        if !self.blend_initialized {
            self.blend = if effective_target { 1.0 } else { 0.0 };
            self.blend_initialized = true;
        } else {
            let step = 1.0 / ((self.sample_rate.max(8000) as f32) * 0.065);
            if effective_target {
                if self.blend <= 0.0 {
                    self.clear_state();
                }
                self.blend = (self.blend + step).min(1.0);
            } else {
                self.blend = (self.blend - step).max(0.0);
            }
        }

        if self.blend <= 0.0 || hardware_channels <= 2 || src_ch >= hardware_channels {
            let mut out = [0.0f32; 12];
            let copy_len = frame_slice.len().min(hardware_channels).min(12);
            out[..copy_len].copy_from_slice(&frame_slice[..copy_len]);
            return out;
        }

        let mut upmixed = self.process_multichannel_frame_for_hardware(
            frame_slice,
            src_ch,
            cfg,
            hardware_channels,
        );
        if self.blend < 1.0 {
            let w = 0.5 - 0.5 * (std::f32::consts::PI * self.blend).cos();
            let dry_w = 1.0 - w;
            for (c, sample_out) in upmixed.iter_mut().enumerate() {
                let dry = if c < hardware_channels {
                    frame_slice.get(c).copied().unwrap_or(0.0)
                } else {
                    0.0
                };
                *sample_out = (dry * dry_w + *sample_out * w).clamp(-1.0, 1.0);
            }
        }
        upmixed
    }
}

/// Extract a representative `(L, R)` stereo input pair from a decoded frame slice of length `hardware_channels`.
#[inline(always)]
pub fn extract_stereo_pair_from_frame(frame: &[f32], source_channels: usize) -> (f32, f32) {
    if frame.is_empty() {
        return (0.0, 0.0);
    }
    let fl = frame[0];
    if frame.len() == 1 || source_channels <= 1 {
        return (fl, fl);
    }
    let fr = frame[1];
    if frame.len() >= 6 && source_channels >= 6 {
        let fc = frame[2] * std::f32::consts::FRAC_1_SQRT_2;
        let lfe = frame[3] * 0.5;
        let sl = frame[4] * std::f32::consts::FRAC_1_SQRT_2;
        let sr = frame[5] * std::f32::consts::FRAC_1_SQRT_2;
        let (rl, rr) = if frame.len() >= 8 && source_channels >= 8 {
            (frame[6] * 0.5, frame[7] * 0.5)
        } else {
            (0.0, 0.0)
        };
        let l = (fl + fc + lfe + sl + rl) * 0.65;
        let r = (fr + fc + lfe + sr + rr) * 0.65;
        (l, r)
    } else {
        (fl, fr)
    }
}

fn is_relevant_audio_device(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    // Filter out dummy/silent devices
    if name_lower == "null" {
        return false;
    }
    // Filter out virtual multi-channel surround setups on Linux (e.g. surround40/51/71 ALSA aliases)
    #[cfg(target_os = "linux")]
    if name_lower.starts_with("surround4")
        || name_lower.starts_with("surround5")
        || name_lower.starts_with("surround7")
    {
        return false;
    }
    // Filter out internal mixer and snooping plugins
    if name_lower.starts_with("dmix") || name_lower.starts_with("dsnoop") {
        return false;
    }
    // Filter out legacy or internal virtual interfaces/plugins
    if name_lower.starts_with("adsp") {
        return false;
    }
    if name_lower.starts_with("speex") || name_lower.contains("speexrate") {
        return false;
    }
    if name_lower.contains("upmix") || name_lower.contains("vdownmix") {
        return false;
    }
    if name_lower.contains("samplerate") || name_lower.contains("lavrate") {
        return false;
    }
    // Filter out dead virtual mixer endpoints (e.g. ASUS Sonic Studio Virtual Mixer) whose WASAPI
    // shared callback never clocks when the companion vendor utility is not actively routing.
    if name_lower.contains("sonic studio virtual") {
        return false;
    }
    true
}

pub fn format_cpal_device_name(desc: &cpal::DeviceDescription) -> String {
    #[cfg(target_os = "windows")]
    if let Some(friendly) = desc.extended().first()
        && !friendly.trim().is_empty()
    {
        return friendly.clone();
    }
    desc.name().to_string()
}

pub const SYSTEM_DEFAULT_AUDIO_DEVICE: &str = "System Default (Auto)";

pub fn is_default_audio_device_selection(selected: Option<&str>) -> bool {
    match selected {
        None => true,
        Some(s) => {
            let trimmed = s.trim();
            let lower = trimmed.to_lowercase();
            trimmed.is_empty()
                || trimmed.eq_ignore_ascii_case(SYSTEM_DEFAULT_AUDIO_DEVICE)
                || trimmed.eq_ignore_ascii_case("default")
                || trimmed.eq_ignore_ascii_case("default device")
                || lower.starts_with("default alsa output")
                || lower.starts_with("default alsa input")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSinkTopology {
    pub sink_id: String,
    pub description: String,
    pub channels: u16,
}

pub fn parse_wpctl_inspect_topology(output: &str) -> Option<AudioSinkTopology> {
    let mut node_id_fallback = String::new();
    let mut is_valid_node = false;
    let mut sink_id = String::new();
    let mut description = String::new();
    let mut nick = String::new();
    let mut explicit_channels: Option<u16> = None;
    let mut position_channels: Option<u16> = None;

    for (line_idx, raw_line) in output.lines().enumerate() {
        let trimmed = raw_line.trim();
        if line_idx == 0
            && trimmed.starts_with("id ")
            && trimmed.contains("PipeWire:Interface:Node")
        {
            is_valid_node = true;
            if let Some(after_id) = trimmed.strip_prefix("id ")
                && let Some((num_str, _)) = after_id.split_once(',')
            {
                node_id_fallback = format!("pipewire_node_{}", num_str.trim());
            }
            continue;
        }
        let clean = trimmed.strip_prefix('*').map(str::trim).unwrap_or(trimmed);
        let Some((key, raw_val)) = clean.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = raw_val.trim().trim_matches('"').trim();
        match key {
            "node.name" if !val.is_empty() => {
                sink_id = val.to_string();
                is_valid_node = true;
            }
            "node.description" if !val.is_empty() => {
                description = val.to_string();
            }
            "node.nick" if !val.is_empty() => {
                nick = val.to_string();
            }
            "audio.channels" => {
                if let Ok(ch) = val.parse::<u16>()
                    && ch > 0
                {
                    explicit_channels = Some(ch);
                }
            }
            "audio.position" => {
                let inner = val.trim_start_matches('[').trim_end_matches(']').trim();
                if !inner.is_empty() {
                    let count = inner
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("UNKNOWN"))
                        .count() as u16;
                    if count > 0 {
                        position_channels = Some(count);
                    }
                }
            }
            _ => {}
        }
    }

    if !is_valid_node {
        return None;
    }
    if sink_id.is_empty() {
        sink_id = node_id_fallback;
    }
    if sink_id.is_empty() {
        return None;
    }
    let final_desc = if !description.is_empty() {
        description
    } else if !nick.is_empty() {
        nick
    } else {
        sink_id.clone()
    };
    let channels = explicit_channels
        .or(position_channels)
        .unwrap_or(2)
        .clamp(1, 32);

    Some(AudioSinkTopology {
        sink_id,
        description: final_desc,
        channels,
    })
}

pub fn parse_pactl_sink_topology(
    default_sink: &str,
    sinks_output: &str,
) -> Option<AudioSinkTopology> {
    let target = default_sink.trim();
    let mut first_candidate: Option<AudioSinkTopology> = None;

    // 1. Support verbose `pactl list sinks` blocks (`Sink #N`, `Name:`, `Description:`, `Sample Specification:`)
    if sinks_output.contains("Sample Specification:") || sinks_output.contains("Name:") {
        let mut cur_name: Option<String> = None;
        let mut cur_desc: Option<String> = None;
        let mut cur_channels: u16 = 2;

        let flush_block = |name: Option<String>,
                           desc: Option<String>,
                           channels: u16,
                           first: &mut Option<AudioSinkTopology>|
         -> Option<AudioSinkTopology> {
            let sink_id = name?;
            if sink_id.is_empty() {
                return None;
            }
            let description = desc
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| sink_id.clone());
            let topo = AudioSinkTopology {
                sink_id: sink_id.clone(),
                description,
                channels,
            };
            if !target.is_empty() && target != "@DEFAULT_SINK@" && sink_id == target {
                return Some(topo);
            }
            if first.is_none() {
                *first = Some(topo);
            }
            None
        };

        for raw_line in sinks_output.lines() {
            let line = raw_line.trim();
            if line.starts_with("Sink #") {
                if let Some(matched) = flush_block(
                    cur_name.take(),
                    cur_desc.take(),
                    cur_channels,
                    &mut first_candidate,
                ) {
                    return Some(matched);
                }
                cur_channels = 2;
            } else if let Some(rest) = line.strip_prefix("Name:") {
                cur_name = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("Description:") {
                cur_desc = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("Sample Specification:") {
                for token in rest.split_whitespace() {
                    if let Some(ch_str) = token.strip_suffix("ch")
                        && let Ok(ch) = ch_str.parse::<u16>()
                        && ch > 0
                    {
                        cur_channels = ch.clamp(1, 32);
                        break;
                    }
                }
            }
        }
        if let Some(matched) = flush_block(
            cur_name.take(),
            cur_desc.take(),
            cur_channels,
            &mut first_candidate,
        ) {
            return Some(matched);
        }
        if first_candidate.is_some() {
            return first_candidate;
        }
    }

    // 2. Support `pactl list sinks short` tab-separated lines
    for line in sinks_output.lines() {
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        if cols.len() < 4 {
            continue;
        }
        let sink_name = cols[1];
        if sink_name.is_empty() {
            continue;
        }
        let format_col = cols[3];
        let mut channels = 2u16;
        for token in format_col.split_whitespace() {
            if let Some(ch_str) = token.strip_suffix("ch")
                && let Ok(ch) = ch_str.parse::<u16>()
                && ch > 0
            {
                channels = ch.clamp(1, 32);
                break;
            }
        }
        let topo = AudioSinkTopology {
            sink_id: sink_name.to_string(),
            description: sink_name.to_string(),
            channels,
        };
        if !target.is_empty() && target != "@DEFAULT_SINK@" && sink_name == target {
            return Some(topo);
        }
        if first_candidate.is_none() {
            first_candidate = Some(topo);
        }
    }

    if !target.is_empty() && target != "@DEFAULT_SINK@" {
        if let Some(cand) = first_candidate {
            return Some(cand);
        }
        return Some(AudioSinkTopology {
            sink_id: target.to_string(),
            description: target.to_string(),
            channels: 2,
        });
    }

    first_candidate
}

pub fn parse_proc_asound_max_playback_channels(stream_text: &str) -> Option<u16> {
    let mut in_playback = false;
    let mut max_ch: Option<u16> = None;
    for raw_line in stream_text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("Playback:") {
            in_playback = true;
            continue;
        }
        if trimmed.starts_with("Capture:") {
            in_playback = false;
            continue;
        }
        if in_playback
            && let Some(ch_str) = trimmed.strip_prefix("Channels:")
            && let Ok(ch) = ch_str.trim().parse::<u16>()
            && ch > 0
        {
            max_ch = Some(max_ch.map_or(ch, |m| m.max(ch)));
        } else if let Some(ch_str) = trimmed.strip_prefix("channels:")
            && let Ok(ch) = ch_str.trim().parse::<u16>()
            && ch > 0
        {
            max_ch = Some(max_ch.map_or(ch, |m| m.max(ch)));
        }
    }
    max_ch
}

pub fn query_active_audio_sink_topology(mic: bool) -> Option<AudioSinkTopology> {
    #[cfg(target_os = "linux")]
    {
        let target_node = if mic {
            "@DEFAULT_AUDIO_SOURCE@"
        } else {
            "@DEFAULT_AUDIO_SINK@"
        };
        if let Ok(out) = std::process::Command::new("wpctl")
            .args(["inspect", target_node])
            .output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(topo) = parse_wpctl_inspect_topology(&text) {
                return Some(topo);
            }
        }

        if !mic
            && let Ok(def_out) = std::process::Command::new("pactl")
                .arg("get-default-sink")
                .output()
            && def_out.status.success()
            && let Ok(list_out) = std::process::Command::new("pactl")
                .args(["list", "sinks", "short"])
                .output()
            && list_out.status.success()
        {
            let def_str = String::from_utf8_lossy(&def_out.stdout);
            let list_str = String::from_utf8_lossy(&list_out.stdout);
            if let Some(topo) = parse_pactl_sink_topology(&def_str, &list_str) {
                return Some(topo);
            }
        }
    }

    let host = cpal::default_host();
    let default_dev = if mic {
        host.default_input_device()
    } else {
        host.default_output_device()
    }?;
    let desc = default_dev.description().ok()?;
    let name = format_cpal_device_name(&desc);
    let channels = if mic {
        default_dev
            .default_input_config()
            .ok()
            .map(|c| c.channels())
            .unwrap_or(1)
    } else {
        default_dev
            .default_output_config()
            .ok()
            .map(|c| c.channels())
            .unwrap_or(2)
    };
    Some(AudioSinkTopology {
        sink_id: name.clone(),
        description: name,
        channels,
    })
}

pub fn resolve_device_target_channels(
    force_stereo: bool,
    default_device_channels: u16,
    _target_channels: u16,
) -> u16 {
    if force_stereo || default_device_channels <= 2 {
        default_device_channels.clamp(1, 2)
    } else {
        default_device_channels.clamp(1, 12)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyTransitionAction {
    pub reload_stream: bool,
    pub osd_message: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn evaluate_topology_transition(
    prev: Option<&AudioSinkTopology>,
    next: &AudioSinkTopology,
    is_auto_device: bool,
    file_loaded: bool,
    is_mic: bool,
    current_hardware_channels: i32,
    force_stereo_downmix: bool,
    is_dummy_audio_output: bool,
    bitstream_active: bool,
    passthrough_enabled: bool,
    os_auto_reroutes_same_channel_streams: bool,
) -> TopologyTransitionAction {
    if !is_auto_device {
        return TopologyTransitionAction {
            reload_stream: false,
            osd_message: None,
        };
    }

    let expected_hw_channels = if force_stereo_downmix {
        (next.channels as i32).clamp(1, 2)
    } else {
        (next.channels as i32).clamp(1, 12)
    };

    let format_sink_osd = |desc: &str, hw_ch: i32| -> String {
        if hw_ch > 2 {
            let badge = crate::state::SurroundUpmixConfig::layout_badge(hw_ch);
            format!("🔊 Audio Output: {} ({})", desc, badge)
        } else {
            let tag = crate::state::SurroundUpmixConfig::short_tag(hw_ch);
            format!("🎧 Audio Output: {} ({})", desc, tag)
        }
    };

    let Some(prev_topo) = prev else {
        let initial_mismatch = (file_loaded || is_mic)
            && (is_dummy_audio_output
                || (current_hardware_channels > 0
                    && !bitstream_active
                    && current_hardware_channels != expected_hw_channels));
        let osd_message = if initial_mismatch {
            Some(format_sink_osd(&next.description, expected_hw_channels))
        } else {
            None
        };
        return TopologyTransitionAction {
            reload_stream: initial_mismatch,
            osd_message,
        };
    };

    if prev_topo.sink_id == next.sink_id
        && prev_topo.channels == next.channels
        && !is_dummy_audio_output
    {
        return TopologyTransitionAction {
            reload_stream: false,
            osd_message: None,
        };
    }

    let channel_mismatch = current_hardware_channels > 0
        && current_hardware_channels != expected_hw_channels
        && !bitstream_active;
    let sink_changed = prev_topo.sink_id != next.sink_id;
    let reload_stream = (file_loaded || is_mic)
        && (is_dummy_audio_output
            || channel_mismatch
            || (sink_changed
                && (!os_auto_reroutes_same_channel_streams
                    || passthrough_enabled
                    || bitstream_active)));

    let osd_message = Some(format_sink_osd(&next.description, expected_hw_channels));

    TopologyTransitionAction {
        reload_stream,
        osd_message,
    }
}

pub fn spawn_audio_topology_watcher(shared_state: Arc<Mutex<AppState>>, is_mic: bool) {
    std::thread::spawn(move || {
        let mut last_topology: Option<AudioSinkTopology> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(750));

            let Some(next_topo) = query_active_audio_sink_topology(is_mic) else {
                continue;
            };

            let topology_changed = last_topology.as_ref() != Some(&next_topo);
            let refreshed_devices = if topology_changed {
                Some(get_available_audio_devices(is_mic))
            } else {
                None
            };

            if let Ok(mut state) = shared_state.lock() {
                if let Some(devs) = refreshed_devices {
                    state.available_audio_devices = devs;
                }
                let is_auto =
                    is_default_audio_device_selection(state.selected_audio_device.as_deref());
                let exclusive_or_bitstream_active = state.stats.bitstream_active
                    || state.stats.mpegh_bitstream_active
                    || (cfg!(windows)
                        && state.hardware_channels > next_topo.channels as i32
                        && !state.force_stereo_downmix);
                let action = evaluate_topology_transition(
                    last_topology.as_ref(),
                    &next_topo,
                    is_auto,
                    state.file_loaded,
                    is_mic,
                    state.hardware_channels,
                    state.force_stereo_downmix,
                    state.is_dummy_audio_output,
                    exclusive_or_bitstream_active,
                    state.passthrough_enabled || state.surround_upmix.mpegh_bitstream,
                    cfg!(target_os = "linux"),
                );

                state.active_audio_sink_id = Some(next_topo.sink_id.clone());
                state.active_audio_sink_description = Some(next_topo.description.clone());
                state.active_audio_sink_channels = next_topo.channels;

                if action.reload_stream {
                    let msg = action
                        .osd_message
                        .unwrap_or_else(|| format!("Audio Output: {}", next_topo.description));
                    state.audio_topology_reload_request = Some(msg);
                } else if let Some(osd) = action.osd_message {
                    state.osd_text = Some(osd);
                    state.osd_timer = 3.0;
                }
            }

            last_topology = Some(next_topo);
        }
    });
}

pub fn cpal_device_matches_name(desc: &cpal::DeviceDescription, target: &str) -> bool {
    let formatted = format_cpal_device_name(desc);
    formatted == target
        || desc.name() == target
        || formatted.eq_ignore_ascii_case(target)
        || desc.name().eq_ignore_ascii_case(target)
        || desc
            .driver()
            .is_some_and(|drv| drv.eq_ignore_ascii_case(target))
}

pub type OutputConfigRankKey = (
    std::cmp::Reverse<u8>,
    std::cmp::Reverse<bool>,
    std::cmp::Reverse<bool>,
    std::cmp::Reverse<bool>,
    std::cmp::Reverse<u16>,
);

#[allow(clippy::too_many_arguments)]
pub fn rank_output_config_key(
    channels: u16,
    min_rate: u32,
    max_rate: u32,
    sample_format: cpal::SampleFormat,
    target_channels: u16,
    target_rate: u32,
    surround_upmix_enabled: bool,
    force_stereo: bool,
) -> OutputConfigRankKey {
    let ch_rank = if channels == target_channels {
        2u8
    } else if surround_upmix_enabled && !force_stereo && (6..=12).contains(&channels) {
        1u8
    } else {
        0u8
    };
    let stereo = channels >= 2;
    let rate_match = min_rate <= target_rate && max_rate >= target_rate;
    // Prefer IEEE 32-bit float (F32) over 16-bit integer (I16):
    // On Windows WASAPI, cpal maps F32 to WAVE_FORMAT_EXTENSIBLE (KSDATAFORMAT_SUBTYPE_IEEE_FLOAT),
    // which is required by Shared Mode for >2ch (5.1/7.1) endpoints and avoids f32->i16->f32 quantization.
    let is_f32 = sample_format == cpal::SampleFormat::F32;

    (
        std::cmp::Reverse(ch_rank),
        std::cmp::Reverse(stereo),
        std::cmp::Reverse(rate_match),
        std::cmp::Reverse(is_f32),
        std::cmp::Reverse(channels),
    )
}

pub fn get_available_audio_devices(mic: bool) -> Vec<String> {
    let host = cpal::default_host();
    let devices = if mic {
        host.input_devices()
    } else {
        host.output_devices()
    };

    let mut names = vec![SYSTEM_DEFAULT_AUDIO_DEVICE.to_string()];
    if let Ok(devices) = devices {
        for d in devices {
            if let Ok(desc) = d.description() {
                let name = format_cpal_device_name(&desc);
                if is_relevant_audio_device(&name)
                    && !is_default_audio_device_selection(Some(&name))
                    && !names.contains(&name)
                {
                    names.push(name);
                }
            }
        }
    }
    names
}

pub fn get_default_audio_device_name(_mic: bool) -> Option<String> {
    Some(SYSTEM_DEFAULT_AUDIO_DEVICE.to_string())
}

pub fn calculate_power_of_two_window_size(sample_rate: u32) -> usize {
    let target = (sample_rate as f32 * 0.185).round() as usize;
    let window_size = 1 << (target as f32).log2().round() as usize;
    window_size.clamp(2048, 65536)
}

fn dsp_recycle_channel() -> &'static (Sender<DspMessage>, Receiver<DspMessage>) {
    static POOL: std::sync::OnceLock<(Sender<DspMessage>, Receiver<DspMessage>)> =
        std::sync::OnceLock::new();
    POOL.get_or_init(|| bounded(8))
}

pub fn spawn_dsp_thread(
    rx: Receiver<DspMessage>,
    shared_state: Arc<Mutex<AppState>>,
    sample_rate: u32,
    max_frequency: f32,
    window_size: usize,
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

        // Pre-plan FFT (rustfft caches the twiddle factors)
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(window_size);
        let mut complex_buf = vec![
            Complex {
                re: 0.0f32,
                im: 0.0f32
            };
            window_size
        ];
        let mut magnitudes = vec![0.0f32; window_size / 2];

        // Precompute log-spaced bin interpolation mapping for mono & GPU spectrum
        let resolution = sample_rate as f32 / window_size as f32; // Hz per FFT bin
        let min_freq = 20.0f32;
        let max_f = max_frequency.max(min_freq * 2.0);
        let num_bins = 1024;
        let nyquist = window_size / 2;

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
            let idx_clamped = idx.clamp(0.0, (nyquist - 1) as f32);
            let i0 = idx_clamped.floor() as usize;
            let i1 = (i0 + 1).min(nyquist - 1);
            let frac = idx_clamped - i0 as f32;
            log_bin_mappings.push(BinMapping { i0, i1, frac });

            let idx_end = freq_end / resolution;
            let use_range = (idx_end - idx) >= 1.0;
            let start = idx.ceil() as usize;
            let safe_end = ((idx_end.floor() as usize) + 1).min(nyquist);
            let nearest = (((idx + idx_end) * 0.5).round() as usize).min(nyquist.saturating_sub(1));
            mono_bin_ranges.push(MonoBinRange {
                start,
                safe_end,
                nearest,
                use_range,
            });
        }

        let n_sqrt = (window_size as f32).sqrt();
        let mut last_waveform_push = Instant::now();
        let recycle_tx = &dsp_recycle_channel().0;

        while let Ok(msg) = rx.recv() {
            let (need_gpu_spectrum, stale_epoch) = if let Ok(state) = shared_state.lock() {
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

            let num_channels = msg.channel_audio_data.len().min(32);
            let fft_elapsed = if msg.audio_data.is_empty() {
                0.0
            } else {
                let fft_start = Instant::now();

                // 1. Apply Hann window and prepare complex input for mono channel
                for i in 0..window_size {
                    let sample = *msg.audio_data.get(i).unwrap_or(&0.0);
                    complex_buf[i] = Complex {
                        re: sample * hann_window[i],
                        im: 0.0,
                    };
                }

                // Run FFT
                fft.process(&mut complex_buf);

                // Compute magnitudes (only first half — up to Nyquist)
                for i in 0..nyquist {
                    magnitudes[i] = complex_buf[i].norm() / n_sqrt;
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
                                complex_buf[i] = Complex {
                                    re: sample * hann_window[i],
                                    im: 0.0,
                                };
                            }
                            fft.process(&mut complex_buf);

                            for (i, (mapping, range)) in log_bin_mappings
                                .iter()
                                .zip(mono_bin_ranges.iter())
                                .enumerate()
                            {
                                let (re, im) = if range.use_range && range.start < range.safe_end {
                                    let mut best = complex_buf[range.start];
                                    let mut best_norm_sq = best.norm_sqr();
                                    for &val in &complex_buf[range.start + 1..range.safe_end] {
                                        let nsq = val.norm_sqr();
                                        if nsq > best_norm_sq {
                                            best_norm_sq = nsq;
                                            best = val;
                                        }
                                    }
                                    (best.re, best.im)
                                } else {
                                    let c0 = complex_buf[mapping.i0];
                                    let c1 = complex_buf[mapping.i1];
                                    (
                                        c0.re * (1.0 - mapping.frac) + c1.re * mapping.frac,
                                        c0.im * (1.0 - mapping.frac) + c1.im * mapping.frac,
                                    )
                                };

                                let norm_re = re / n_sqrt;
                                let norm_im = im / n_sqrt;

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
                                let mut best = complex_buf[range.start];
                                let mut best_norm_sq = best.norm_sqr();
                                for &val in &complex_buf[range.start + 1..range.safe_end] {
                                    let nsq = val.norm_sqr();
                                    if nsq > best_norm_sq {
                                        best_norm_sq = nsq;
                                        best = val;
                                    }
                                }
                                (best.re, best.im)
                            } else {
                                let c0 = complex_buf[mapping.i0];
                                let c1 = complex_buf[mapping.i1];
                                (
                                    c0.re * (1.0 - mapping.frac) + c1.re * mapping.frac,
                                    c0.im * (1.0 - mapping.frac) + c1.im * mapping.frac,
                                )
                            };

                            let norm_re = re / n_sqrt;
                            let norm_im = im / n_sqrt;

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
                    drop(state);
                    let _ = recycle_tx.try_send(msg);
                    continue;
                }

                if msg.audio_data.is_empty() {
                    state.stats.fft_us = 0.0;
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

                state.raw_channel_vus.clear();
                state.raw_channel_vus.extend_from_slice(&msg.channel_vus);
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

                if !msg.channel_audio_data.is_empty() {
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

                if !msg.audio_data.is_empty() {
                    // --- Waveform extraction (Zero-Crossing Edge Trigger) ---
                    let visual_width = state.visual_width.clamp(128, 2048) as usize;
                    let target_fps = state.target_fps.clamp(30, 500);
                    let waveform_push_interval =
                        std::time::Duration::from_secs_f64(1.0 / target_fps as f64);

                    // Maintain a constant ~23ms time window regardless of sample rate
                    let target_window_samples = (sample_rate as f32 * 0.023).round() as usize;
                    let stride = (target_window_samples as f32 / visual_width as f32).max(1.0);
                    let actual_window_samples = (visual_width as f32 * stride).ceil() as usize;

                    // Restrict search to the last ~33ms to ensure a fresh trigger at >= 30 FPS,
                    // avoiding the visual "freeze" caused by searching the entire history buffer.
                    let search_window = (sample_rate as f32 / 30.0) as usize;
                    let search_start = msg
                        .audio_data
                        .len()
                        .saturating_sub(actual_window_samples + search_window);
                    // Ensure i+1 stays in bounds during zero-crossing search
                    let search_limit = msg
                        .audio_data
                        .len()
                        .saturating_sub(actual_window_samples)
                        .min(msg.audio_data.len().saturating_sub(1));

                    let mut start_idx = search_start;
                    let mut best_slope = 0.0;

                    if msg.audio_data.len() >= 2 {
                        for i in search_start..search_limit {
                            if msg.audio_data[i] <= 0.0 && msg.audio_data[i + 1] > 0.0 {
                                let slope = msg.audio_data[i + 1] - msg.audio_data[i];
                                if slope > best_slope {
                                    best_slope = slope;
                                    start_idx = i;
                                }
                            }
                        }
                    }

                    if state.raw_waveform.len() != visual_width {
                        state.raw_waveform.resize(visual_width, 0.0);
                    }

                    for i in 0..visual_width {
                        let sample_idx = start_idx + (i as f32 * stride) as usize;
                        state.raw_waveform[i] =
                            msg.audio_data[sample_idx.min(msg.audio_data.len() - 1)];
                    }

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
            }
            let _ = recycle_tx.try_send(msg);
        }
    });
}

pub fn update_lookahead_timeline_from_queue(
    lookahead_queue: &std::collections::VecDeque<(f64, [f32; 8])>,
    raw_waveform: &[f32],
    raw_channel_vus: &[f32],
    cur_t: f64,
    lookahead_timeline: &mut Vec<f32>,
) {
    if lookahead_timeline.len() != 600 * 8 {
        lookahead_timeline.resize(600 * 8, 0.0);
    }
    if !lookahead_queue.is_empty() {
        let q_len = lookahead_queue.len();
        let (t_first, d_first) = lookahead_queue[0];
        let (t_last, d_last) = lookahead_queue[q_len - 1];

        let mut idx = 0usize;
        for k in 0..600 {
            let target_t = cur_t + (k as f64 - 100.0) * 0.01;
            let off = k * 8;

            if target_t < t_first {
                let fade = (1.0 - ((t_first - target_t) / 0.5)).max(0.0) as f32;
                for (c, val) in d_first.iter().enumerate() {
                    lookahead_timeline[off + c] = *val * fade;
                }
            } else if target_t > t_last {
                let fade = (1.0 - ((target_t - t_last) / 0.5)).max(0.0) as f32;
                for (c, val) in d_last.iter().enumerate() {
                    lookahead_timeline[off + c] = *val * fade;
                }
            } else {
                while idx < q_len && lookahead_queue[idx].0 < target_t {
                    idx += 1;
                }

                if idx == 0 {
                    lookahead_timeline[off..off + 8].copy_from_slice(&d_first);
                } else if idx >= q_len {
                    lookahead_timeline[off..off + 8].copy_from_slice(&d_last);
                } else {
                    let (t0, d0) = lookahead_queue[idx - 1];
                    let (t1, d1) = lookahead_queue[idx];
                    let dt = t1 - t0;
                    if dt > 0.0001 && dt < 1.0 && target_t >= t0 && target_t <= t1 {
                        let frac = ((target_t - t0) / dt).clamp(0.0, 1.0) as f32;
                        for c in 0..8 {
                            lookahead_timeline[off + c] = d0[c] * (1.0 - frac) + d1[c] * frac;
                        }
                    } else {
                        lookahead_timeline[off..off + 8].copy_from_slice(&d0);
                    }
                }
            }
        }
    } else if !raw_waveform.is_empty() {
        let wave_len = raw_waveform.len();
        let vu_l = raw_channel_vus
            .first()
            .copied()
            .unwrap_or(0.2)
            .clamp(0.05, 1.0);
        let vu_r = raw_channel_vus
            .get(1)
            .copied()
            .unwrap_or(vu_l)
            .clamp(0.05, 1.0);
        for k in 0..600 {
            let off = k * 8;
            let wave_idx = (k * 13) % wave_len;
            let sample = raw_waveform[wave_idx].clamp(-1.0, 1.0);
            let sample_next = raw_waveform[(wave_idx + 1) % wave_len].clamp(-1.0, 1.0);
            let min_s = sample.min(sample_next).min(0.0) * vu_l;
            let max_s = sample.max(sample_next).max(0.0) * vu_l;
            let min_r = sample.min(sample_next).min(0.0) * vu_r;
            let max_r = sample.max(sample_next).max(0.0) * vu_r;
            let rms = (sample.abs() * 0.7 + 0.3 * (vu_l + vu_r) * 0.5).min(1.0);
            lookahead_timeline[off] = min_s;
            lookahead_timeline[off + 1] = max_s;
            lookahead_timeline[off + 2] = min_r;
            lookahead_timeline[off + 3] = max_r;
            lookahead_timeline[off + 4] = rms;
            lookahead_timeline[off + 5] = rms;
            lookahead_timeline[off + 6] = vu_l;
            lookahead_timeline[off + 7] = vu_r;
        }
    } else {
        lookahead_timeline.fill(0.0);
    }
}

pub trait AudioSource: Send {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize;
    fn get_duration_seconds(&mut self) -> f64;
    fn get_position_seconds(&mut self) -> f64;
    fn set_position_seconds(&mut self, pos: f64);
    fn get_num_channels(&mut self) -> i32;
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32;
    fn get_artist(&mut self) -> String;
    fn get_type(&mut self) -> String;
    fn get_tempo(&mut self) -> i32;
    fn get_speed(&mut self) -> i32;
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32>;
    fn get_num_samples(&mut self) -> i32;
    fn get_num_instruments(&mut self) -> i32;
    fn get_num_patterns(&mut self) -> i32;
    fn get_current_order(&mut self) -> i32;
    fn get_current_row(&mut self) -> i32;
    fn get_tracker_channels(&mut self) -> Option<i32> {
        None
    }
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        Vec::new()
    }
    fn get_current_row_string(&mut self) -> String {
        String::new()
    }
    fn get_video_info(&mut self) -> Option<String> {
        None
    }
    #[cfg(not(target_os = "android"))]
    fn attach_video_queue(&mut self, _tx: crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>) {}
    #[cfg(not(target_os = "android"))]
    fn take_video_parameters(
        &mut self,
    ) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational, u32)> {
        None
    }
    fn get_bitrate(&mut self) -> Option<u32> {
        None
    }
    fn get_video_fps(&self) -> f32 {
        0.0
    }
    fn get_video_bitrate(&self) -> Option<u32> {
        None
    }
    fn get_audio_tracks(&self) -> Vec<crate::state::AudioTrackInfo> {
        Vec::new()
    }
    fn get_selected_audio_track(&self) -> usize {
        0
    }
    fn select_audio_track(&mut self, _track_idx: usize) -> Result<()> {
        Ok(())
    }
    fn get_active_audio_tracks(&self) -> Vec<usize> {
        vec![self.get_selected_audio_track()]
    }
    fn set_active_audio_tracks(&mut self, tracks: &[(usize, f32)]) -> Result<()> {
        if let Some(&(idx, _)) = tracks.first() {
            self.select_audio_track(idx)
        } else {
            Ok(())
        }
    }
    fn has_video_stream(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------
// OpenMPT Tracker Decoder
// ---------------------------------------------------------
pub struct SafeModule(pub Module);
unsafe impl Send for SafeModule {}

struct OpenMptSource {
    module: SafeModule,
    left_buf: Vec<f32>,
    right_buf: Vec<f32>,
}

impl AudioSource for OpenMptSource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        let frames_to_render = output.len() / hardware_channels;

        // The openmpt crate uses Vec::capacity() (not len()) to decide how many
        // frames to render. After Vec::resize(), capacity can exceed length due
        // to Vec's doubling growth strategy, causing the C library to write past
        // the Vec's logical length. Allocate exact-size buffers to avoid this.
        self.left_buf = vec![0.0; frames_to_render];
        self.right_buf = vec![0.0; frames_to_render];

        let frames_read = self.module.0.read_float_stereo(
            sample_rate as i32,
            &mut self.left_buf,
            &mut self.right_buf,
        );

        for i in 0..frames_read {
            let l = self.left_buf[i];
            let r = self.right_buf[i];

            output[i * hardware_channels] = l;
            if hardware_channels > 1 {
                output[i * hardware_channels + 1] = r;
                for j in 2..hardware_channels {
                    output[i * hardware_channels + j] = 0.0;
                }
            }
        }

        frames_read
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.module.0.get_duration_seconds()
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.module.0.get_position_seconds()
    }
    fn set_position_seconds(&mut self, pos: f64) {
        self.module.0.set_position_seconds(pos);
    }
    fn get_num_channels(&mut self) -> i32 {
        2
    }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        self.module.0.get_current_channel_vu_mono(channel)
    }

    fn get_artist(&mut self) -> String {
        use openmpt::module::metadata::MetadataKey;
        self.module
            .0
            .get_metadata(MetadataKey::ModuleArtist)
            .unwrap_or("Unknown".to_string())
    }

    fn get_type(&mut self) -> String {
        use openmpt::module::metadata::MetadataKey;
        self.module
            .0
            .get_metadata(MetadataKey::TypeExt)
            .unwrap_or("Tracker".to_string())
    }

    fn get_tempo(&mut self) -> i32 {
        self.module.0.get_current_tempo()
    }
    fn get_speed(&mut self) -> i32 {
        self.module.0.get_current_speed()
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        None
    }
    fn get_num_samples(&mut self) -> i32 {
        self.module.0.get_num_samples()
    }
    fn get_num_instruments(&mut self) -> i32 {
        self.module.0.get_num_instruments()
    }
    fn get_num_patterns(&mut self) -> i32 {
        self.module.0.get_num_patterns()
    }
    fn get_current_order(&mut self) -> i32 {
        self.module.0.get_current_order()
    }
    fn get_current_row(&mut self) -> i32 {
        self.module.0.get_current_row()
    }
    fn get_tracker_channels(&mut self) -> Option<i32> {
        Some(self.module.0.get_num_channels())
    }

    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        let mut patterns_by_order = Vec::new();
        let num_orders = self.module.0.get_num_orders();
        let num_channels = self.module.0.get_num_channels();

        for o in 0..num_orders {
            let mut row_strings = Vec::new();
            if let Some(mut pattern) = self.module.0.get_pattern_by_order(o) {
                let num_rows = pattern.get_num_rows();
                for r in 0..num_rows {
                    if let Some(mut row) = pattern.get_row_by_number(r) {
                        let mut row_str = String::new();
                        for c in 0..num_channels {
                            if let Some(mut cell) = row.get_cell_by_channel(c) {
                                if c != 0 {
                                    row_str.push_str(" | ");
                                }
                                row_str.push_str(&cell.get_formatted(0, false));
                            }
                        }
                        row_strings.push(row_str);
                    }
                }
            }
            patterns_by_order.push(row_strings);
        }

        patterns_by_order
    }

    fn get_current_row_string(&mut self) -> String {
        let mut row_str = String::new();
        let num_channels = self.module.0.get_num_channels();
        let cur_order = self.module.0.get_current_order();
        let cur_row = self.module.0.get_current_row();

        if let Some(mut pattern) = self.module.0.get_pattern_by_order(cur_order)
            && let Some(mut row) = pattern.get_row_by_number(cur_row)
        {
            for c in 0..num_channels {
                if let Some(mut cell) = row.get_cell_by_channel(c) {
                    if c != 0 {
                        row_str.push_str(" | ");
                    }
                    row_str.push_str(&cell.get_formatted(0, false));
                }
            }
        }
        row_str
    }
}

// ---------------------------------------------------------
// MidiTracker Audio Decoder
// ---------------------------------------------------------
#[derive(Clone)]
struct MidiEvent {
    time_sec: f64,
    channel: i32,
    velocity: u8,
    is_note_on: bool,
}

struct MidiSource {
    sequencer: Option<MidiFileSequencer>,
    pos_silent: f64,
    events: Vec<MidiEvent>,
    event_idx: usize,
    channel_vus: Vec<f32>,
    duration: f64,
    artist: String,
    title: String,
    tracker_data: Vec<Vec<String>>,
    tempo: i32,
    tempo_map: Vec<(u32, f64, f64)>,
    ticks_per_beat: f64,
    left_buf: Vec<f32>,
    right_buf: Vec<f32>,
    midi_file: Arc<MidiFile>,
}

unsafe impl Send for MidiSource {}

impl MidiSource {
    pub fn new(file_path: &str, soundfont_path: &str, sample_rate: i32) -> anyhow::Result<Self> {
        type CachedSoundFont = std::sync::Mutex<Option<(String, Arc<SoundFont>)>>;
        static CACHED_SOUNDFONT: std::sync::OnceLock<CachedSoundFont> = std::sync::OnceLock::new();
        let cache = CACHED_SOUNDFONT.get_or_init(|| std::sync::Mutex::new(None));
        let sf2_arc_opt = {
            let mut guard = cache.lock().unwrap();
            if let Some((cached_path, sf2)) = guard.as_ref()
                && cached_path == soundfont_path
            {
                Some(sf2.clone())
            } else if let Ok(mut sf2_file) = File::open(soundfont_path)
                && let Ok(sf2) = SoundFont::new(&mut sf2_file)
            {
                let arc = Arc::new(sf2);
                *guard = Some((soundfont_path.to_string(), arc.clone()));
                Some(arc)
            } else {
                None
            }
        };

        let mut artist_suffix = "";
        let mut sequencer = if let Some(sf2) = sf2_arc_opt {
            let settings = SynthesizerSettings::new(sample_rate);
            if let Ok(synth) = Synthesizer::new(&sf2, &settings) {
                Some(MidiFileSequencer::new(synth))
            } else {
                artist_suffix = " (No SoundFont)";
                None
            }
        } else {
            artist_suffix = " (No SoundFont)";
            None
        };

        let mut midi_file = File::open(file_path)?;
        let midi = Arc::new(
            MidiFile::new(&mut midi_file)
                .map_err(|e| anyhow::anyhow!("Midi parse error: {:?}", e))?,
        );

        let duration = midi.get_length();
        if let Some(seq) = &mut sequencer {
            seq.play(&midi, false);
        }

        let data = std::fs::read(file_path)?;
        let smf = Smf::parse(&data).map_err(|e| anyhow::anyhow!("Midly parse error: {:?}", e))?;

        let mut artist = String::new();
        let mut title = String::new();

        let mut absolute_events = Vec::new();
        let mut raw_tempo_changes: Vec<(u32, f64)> = Vec::new();
        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(ticks) => ticks.as_int() as f64,
            _ => 480.0,
        };

        for track in &smf.tracks {
            let mut current_tick = 0;
            for event in track {
                current_tick += event.delta.as_int();
                match &event.kind {
                    TrackEventKind::Meta(MetaMessage::TrackName(name)) => {
                        if title.is_empty() {
                            title = String::from_utf8_lossy(name).to_string();
                        }
                    }
                    TrackEventKind::Meta(MetaMessage::Text(text)) => {
                        if artist.is_empty() {
                            artist = String::from_utf8_lossy(text).to_string();
                        }
                    }
                    TrackEventKind::Meta(MetaMessage::Tempo(micros)) => {
                        let us = micros.as_int().max(1) as f64;
                        raw_tempo_changes.push((current_tick, us));
                    }
                    TrackEventKind::Midi { channel, message } => match message {
                        MidiMessage::NoteOn { key, vel } => {
                            absolute_events.push((
                                current_tick,
                                channel.as_int(),
                                key.as_int(),
                                vel.as_int(),
                                vel.as_int() > 0,
                            ));
                        }
                        MidiMessage::NoteOff { key, vel } => {
                            absolute_events.push((
                                current_tick,
                                channel.as_int(),
                                key.as_int(),
                                vel.as_int(),
                                false,
                            ));
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        // Permissive SMF fallback for files (e.g., XMI2MID conversions) where midly stops early
        // due to 8-bit controller values like `B1 00 FF`.
        if absolute_events.is_empty() && data.len() >= 14 && &data[0..4] == b"MThd" {
            let mut pos = 8 + u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
            while pos + 8 <= data.len() {
                let chunk_id = &data[pos..pos + 4];
                let chunk_len = u32::from_be_bytes([
                    data[pos + 4],
                    data[pos + 5],
                    data[pos + 6],
                    data[pos + 7],
                ]) as usize;
                pos += 8;
                let track_end = (pos + chunk_len).min(data.len());
                if chunk_id == b"MTrk" {
                    let mut p = pos;
                    let mut tick = 0u32;
                    let mut last_status = 0u8;
                    let read_varlen = |buf: &[u8], idx: &mut usize, end: usize| -> Option<u32> {
                        let mut val = 0u32;
                        for _ in 0..4 {
                            if *idx >= end {
                                return None;
                            }
                            let b = buf[*idx];
                            *idx += 1;
                            val = (val << 7) | ((b & 0x7F) as u32);
                            if (b & 0x80) == 0 {
                                return Some(val);
                            }
                        }
                        Some(val)
                    };
                    while p < track_end {
                        let Some(delta) = read_varlen(&data, &mut p, track_end) else {
                            break;
                        };
                        tick = tick.saturating_add(delta);
                        if p >= track_end {
                            break;
                        }
                        let first = data[p];
                        p += 1;
                        if (first & 0x80) == 0 {
                            let cmd = last_status & 0xF0;
                            let ch = last_status & 0x0F;
                            let d1 = first & 0x7F;
                            if cmd == 0xC0 || cmd == 0xD0 {
                                continue;
                            }
                            if p >= track_end {
                                break;
                            }
                            let d2 = data[p] & 0x7F;
                            p += 1;
                            if cmd == 0x90 {
                                absolute_events.push((tick, ch, d1, d2, d2 > 0));
                            } else if cmd == 0x80 {
                                absolute_events.push((tick, ch, d1, d2, false));
                            }
                        } else if first == 0xF0 || first == 0xF7 {
                            let Some(len) = read_varlen(&data, &mut p, track_end) else {
                                break;
                            };
                            p = (p + len as usize).min(track_end);
                        } else if first == 0xFF {
                            if p >= track_end {
                                break;
                            }
                            let meta_type = data[p];
                            p += 1;
                            let Some(len) = read_varlen(&data, &mut p, track_end) else {
                                break;
                            };
                            let meta_end = (p + len as usize).min(track_end);
                            let slice = &data[p..meta_end];
                            p = meta_end;
                            if meta_type == 0x2F {
                                break;
                            } else if meta_type == 0x51 && slice.len() == 3 {
                                let us = u32::from_be_bytes([0, slice[0], slice[1], slice[2]])
                                    .max(1) as f64;
                                raw_tempo_changes.push((tick, us));
                            } else if meta_type == 0x03 && title.is_empty() {
                                title = String::from_utf8_lossy(slice).to_string();
                            } else if meta_type == 0x01 && artist.is_empty() {
                                artist = String::from_utf8_lossy(slice).to_string();
                            }
                        } else {
                            last_status = first;
                            let cmd = first & 0xF0;
                            let ch = first & 0x0F;
                            if p >= track_end {
                                break;
                            }
                            let d1 = data[p] & 0x7F;
                            p += 1;
                            if cmd == 0xC0 || cmd == 0xD0 {
                                continue;
                            }
                            if p >= track_end {
                                break;
                            }
                            let d2 = data[p] & 0x7F;
                            p += 1;
                            if cmd == 0x90 {
                                absolute_events.push((tick, ch, d1, d2, d2 > 0));
                            } else if cmd == 0x80 {
                                absolute_events.push((tick, ch, d1, d2, false));
                            }
                        }
                    }
                }
                pos = track_end;
            }
        }

        raw_tempo_changes.sort_by_key(|e| e.0);
        let mut tempo_map: Vec<(u32, f64, f64)> = vec![(0, 0.0, 500_000.0)];
        for (tick, us_per_beat) in raw_tempo_changes {
            let &(prev_tick, prev_sec, prev_us) = tempo_map.last().unwrap();
            if tick == prev_tick {
                tempo_map.last_mut().unwrap().2 = us_per_beat;
            } else {
                let delta_beats = (tick - prev_tick) as f64 / ticks_per_beat;
                let sec = prev_sec + delta_beats * (prev_us / 1_000_000.0);
                tempo_map.push((tick, sec, us_per_beat));
            }
        }

        let initial_bpm = (60_000_000.0 / tempo_map[0].2).round() as i32;

        absolute_events.sort_by_key(|e| e.0);

        let mut parsed_events = Vec::with_capacity(absolute_events.len());
        let mut pattern_rows: Vec<Vec<String>> = Vec::new();
        let notes = [
            "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
        ];
        let mut tempo_idx = 0;
        for (tick, channel, key, vel, is_on) in absolute_events {
            while tempo_idx + 1 < tempo_map.len() && tempo_map[tempo_idx + 1].0 <= tick {
                tempo_idx += 1;
            }
            let (base_tick, base_sec, us_per_beat) = tempo_map[tempo_idx];
            let delta_ticks = tick.saturating_sub(base_tick);
            let beats = delta_ticks as f64 / ticks_per_beat;
            let current_time = base_sec + beats * (us_per_beat / 1_000_000.0);

            parsed_events.push(MidiEvent {
                time_sec: current_time,
                channel: channel as i32,
                velocity: vel,
                is_note_on: is_on,
            });

            if is_on && (channel as usize) < 16 {
                let row_idx = (tick as f64 / (ticks_per_beat / 4.0)) as usize;
                while pattern_rows.len() <= row_idx {
                    pattern_rows.push(vec!["... .. ..".to_string(); 16]);
                }
                let octave = key / 12;
                let note = key % 12;
                let note_name = format!("{}{}", notes[note as usize], octave);
                let note_str = format!("{} .. {:02X}", note_name, vel);
                pattern_rows[row_idx][channel as usize] = note_str;
            }
        }

        let mut final_rows = Vec::new();
        for row in pattern_rows {
            final_rows.push(row.join(" | "));
        }
        let tracker_data = vec![final_rows];

        if artist.is_empty() {
            artist = "Unknown MIDI".to_string();
        }
        if !artist_suffix.is_empty() {
            artist.push_str(artist_suffix);
        }

        Ok(Self {
            sequencer,
            pos_silent: 0.0,
            events: parsed_events,
            event_idx: 0,
            channel_vus: vec![0.0; 16],
            duration,
            artist,
            title,
            tracker_data,
            tempo: initial_bpm,
            tempo_map,
            ticks_per_beat,
            left_buf: vec![0.0; 8192],
            right_buf: vec![0.0; 8192],
            midi_file: midi,
        })
    }
}

impl AudioSource for MidiSource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        _sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        let frames_to_render = output.len() / hardware_channels;

        let pos = if let Some(ref mut seq) = self.sequencer {
            if self.left_buf.len() < frames_to_render {
                self.left_buf.resize(frames_to_render, 0.0);
                self.right_buf.resize(frames_to_render, 0.0);
            }
            seq.render(
                &mut self.left_buf[..frames_to_render],
                &mut self.right_buf[..frames_to_render],
            );
            for i in 0..frames_to_render {
                let l = self.left_buf[i];
                let r = self.right_buf[i];
                output[i * hardware_channels] = l;
                if hardware_channels > 1 {
                    output[i * hardware_channels + 1] = r;
                    for j in 2..hardware_channels {
                        output[i * hardware_channels + j] = 0.0;
                    }
                }
            }
            seq.get_position()
        } else {
            output.fill(0.0);
            self.pos_silent = (self.pos_silent + frames_to_render as f64 / _sample_rate as f64)
                .min(self.duration);
            self.pos_silent
        };

        for c in 0..16 {
            self.channel_vus[c] = (self.channel_vus[c] - 0.02).max(0.0);
        }

        while self.event_idx < self.events.len() && self.events[self.event_idx].time_sec <= pos {
            let ev = &self.events[self.event_idx];
            if ev.is_note_on {
                let ch = ev.channel as usize;
                if ch < 16 {
                    self.channel_vus[ch] = (ev.velocity as f32 / 127.0).clamp(0.1, 1.0);
                }
            }
            self.event_idx += 1;
        }

        let is_end = if let Some(ref seq) = self.sequencer {
            seq.end_of_sequence()
        } else {
            pos >= self.duration
        };

        if is_end { 0 } else { frames_to_render }
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        if let Some(ref seq) = self.sequencer {
            seq.get_position()
        } else {
            self.pos_silent
        }
    }

    fn set_position_seconds(&mut self, pos: f64) {
        if let Some(ref mut seq) = self.sequencer {
            if pos < seq.get_position() {
                seq.play(&self.midi_file, false);
            }

            let mut trash_left = vec![0.0; 8192];
            let mut trash_right = vec![0.0; 8192];
            while seq.get_position() < pos && !seq.end_of_sequence() {
                seq.render(&mut trash_left, &mut trash_right);
            }
        } else {
            self.pos_silent = pos.clamp(0.0, self.duration);
        }

        self.event_idx = 0;
        while self.event_idx < self.events.len() && self.events[self.event_idx].time_sec < pos {
            self.event_idx += 1;
        }
        for c in 0..16 {
            self.channel_vus[c] = 0.0;
        }
    }

    fn get_num_channels(&mut self) -> i32 {
        2
    }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        if (0..16).contains(&channel) {
            self.channel_vus[channel as usize]
        } else {
            0.0
        }
    }

    fn get_artist(&mut self) -> String {
        if !self.title.is_empty() {
            format!("{} - {}", self.artist, self.title)
        } else {
            self.artist.clone()
        }
    }

    fn get_type(&mut self) -> String {
        "MIDI".to_string()
    }
    fn get_tempo(&mut self) -> i32 {
        let pos = self.get_position_seconds();
        let mut us = self.tempo_map.first().map(|e| e.2).unwrap_or(500_000.0);
        for &(_, sec, tempo_us) in &self.tempo_map {
            if sec <= pos {
                us = tempo_us;
            } else {
                break;
            }
        }
        self.tempo = (60_000_000.0 / us).round() as i32;
        self.tempo
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        Some(48000)
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        128
    }
    fn get_num_patterns(&mut self) -> i32 {
        1
    }

    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        let pos = self.get_position_seconds();
        let mut base_tick = 0u32;
        let mut base_sec = 0.0;
        let mut us = 500_000.0;
        for &(t, sec, tempo_us) in &self.tempo_map {
            if sec <= pos {
                base_tick = t;
                base_sec = sec;
                us = tempo_us;
            } else {
                break;
            }
        }
        let delta_sec = (pos - base_sec).max(0.0);
        let delta_ticks = (delta_sec / (us / 1_000_000.0)) * self.ticks_per_beat;
        let current_tick = base_tick as f64 + delta_ticks;
        (current_tick / (self.ticks_per_beat / 4.0)) as i32
    }

    fn get_tracker_channels(&mut self) -> Option<i32> {
        Some(16)
    }

    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        self.tracker_data.clone()
    }

    fn get_current_row_string(&mut self) -> String {
        let row = self.get_current_row() as usize;
        if !self.tracker_data.is_empty() && row < self.tracker_data[0].len() {
            self.tracker_data[0][row].clone()
        } else {
            vec!["... .. ..".to_string(); 16].join(" | ")
        }
    }
}

// ---------------------------------------------------------
// Symphonia Standard Audio Decoder
// ---------------------------------------------------------
struct ActiveSymphoniaTrack {
    track_idx: usize,
    track_id: u32,
    decoder: Box<dyn Decoder>,
    sample_buf: SampleBuffer<f32>,
    samples: Vec<f32>,
    buf_pos: usize,
    volume: f32,
    enabled: bool,
    channels: u16,
    time_base: f64,
    current_time: f64,
}

#[inline(always)]
pub(crate) fn calculate_mix_headroom<I>(volumes: I) -> f32
where
    I: IntoIterator<Item = f32>,
{
    let active_power: f32 = volumes.into_iter().map(|v| v * v).sum();
    if active_power > 1.0 {
        1.0 / active_power.sqrt()
    } else {
        1.0
    }
}

#[inline(always)]
pub fn get_downmixed_sample(
    track_channels: u16,
    base_slice: &[f32],
    hardware_channels: usize,
    out_channel: usize,
) -> f32 {
    if base_slice.is_empty() {
        return 0.0;
    }
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const SQRT2: f32 = std::f32::consts::SQRT_2;

    match (track_channels, hardware_channels, out_channel) {
        // Direct 1-to-1 mapping if matching or enough channels
        (tc, hc, c) if c < tc as usize && hc >= tc as usize => {
            base_slice.get(c).copied().unwrap_or(0.0)
        }
        // Mono to any channel (pan to left and right, or mono)
        (1, _, c) if c < 2 => base_slice[0],
        (1, _, _) => 0.0,
        // Stereo to mono
        (2, 1, 0) => {
            let l = base_slice.first().copied().unwrap_or(0.0);
            let r = base_slice.get(1).copied().unwrap_or(0.0);
            (l + r) * 0.5
        }
        // 3.0 / 2.1 (FL, FR, FC/LFE) to Stereo (L, R)
        (3, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc) * INV_SQRT2
        }
        (3, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc) * INV_SQRT2
        }
        // 3.0 / 2.1 to Mono
        (3, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + fr + INV_SQRT2 * fc) * 0.4
        }
        // Quad (FL, FR, BL, BR) to Stereo (L, R)
        (4, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let bl = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * bl) * INV_SQRT2
        }
        (4, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let br = base_slice.get(3).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * br) * INV_SQRT2
        }
        // Quad to Mono
        (4, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let bl = base_slice.get(2).copied().unwrap_or(0.0);
            let br = base_slice.get(3).copied().unwrap_or(0.0);
            (fl + fr + bl + br) * 0.25
        }
        // 5.0 (FL, FR, FC, BL, BR) to Stereo (L, R)
        (5, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(3).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + INV_SQRT2 * bl) * INV_SQRT2
        }
        (5, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let br = base_slice.get(4).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + INV_SQRT2 * br) * INV_SQRT2
        }
        // 5.0 to Mono
        (5, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(3).copied().unwrap_or(0.0);
            let br = base_slice.get(4).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc + bl + br) * 0.35
        }
        // 5.1 (FL, FR, FC, LFE, BL, BR) to Stereo (L, R)
        (6, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + INV_SQRT2 * bl + 0.5 * lfe) * INV_SQRT2
        }
        (6, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + INV_SQRT2 * br + 0.5 * lfe) * INV_SQRT2
        }
        // 5.1 to Mono
        (6, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc + bl + br + lfe) * 0.35
        }
        // 6.1 (FL, FR, FC, LFE, BC, SL, SR) to Stereo (L, R)
        (7, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(5).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + 0.5 * bc + INV_SQRT2 * sl + 0.5 * lfe) * 0.6
        }
        (7, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sr = base_slice.get(6).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + 0.5 * bc + INV_SQRT2 * sr + 0.5 * lfe) * 0.6
        }
        // 6.1 to Mono
        (7, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(6).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc + bc + sl + sr + lfe) * 0.3
        }
        // 7.1 (FL, FR, FC, LFE, BL, BR, SL, SR) to 5.1 (FL, FR, FC, LFE, SL/BL, SR/BR)
        (8, 6, c @ 0..=3) => base_slice.get(c).copied().unwrap_or(0.0),
        (8, 6, 4) => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            (bl + sl) * INV_SQRT2
        }
        (8, 6, 5) => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            (br + sr) * INV_SQRT2
        }
        // 7.1 (FL, FR, FC, LFE, BL, BR, SL, SR) to Stereo
        (8, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + INV_SQRT2 * bl + INV_SQRT2 * sl + 0.5 * lfe) * 0.6
        }
        (8, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + INV_SQRT2 * br + INV_SQRT2 * sr + 0.5 * lfe) * 0.6
        }
        // 7.1 to Mono
        (8, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc + bl + br + sl + sr + lfe) * 0.28
        }
        // >8 channels (7.1.4, 7.1.2, 5.1.4, Ambisonics) to 7.1 (FL, FR, FC, LFE, BL, BR, SL, SR)
        (tc, 8, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let tfl = base_slice.get(8).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * tfl) * 0.85
        }
        (tc, 8, 1) if tc > 8 => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let tfr = base_slice.get(9).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * tfr) * 0.85
        }
        (tc, 8, c @ 2..=3) if tc > 8 => base_slice.get(c).copied().unwrap_or(0.0) * 0.85,
        (tc, 8, 4) if tc > 8 => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            (bl + 0.5 * tbl) * 0.85
        }
        (tc, 8, 5) if tc > 8 => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (br + 0.5 * tbr) * 0.85
        }
        (tc, 8, 6) if tc > 8 => {
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            (sl + 0.5 * tbl) * 0.85
        }
        (tc, 8, 7) if tc > 8 => {
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (sr + 0.5 * tbr) * 0.85
        }
        // >8 channels (7.1.4, 7.1.2, 5.1.4, Ambisonics) to 5.1 (FL, FR, FC, LFE, SL/BL, SR/BR)
        (tc, 6, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let tfl = base_slice.get(8).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * tfl) * 0.8
        }
        (tc, 6, 1) if tc > 8 => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let tfr = base_slice.get(9).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * tfr) * 0.8
        }
        (tc, 6, c @ 2..=3) if tc > 8 => base_slice.get(c).copied().unwrap_or(0.0) * 0.8,
        (tc, 6, 4) if tc > 8 => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            (bl + sl + INV_SQRT2 * tbl) * 0.65
        }
        (tc, 6, 5) if tc > 8 => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (br + sr + INV_SQRT2 * tbr) * 0.65
        }
        // >8 channels (7.1.4, Ambisonics, etc.) to Stereo
        (tc, 2, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let mut sum = fl + INV_SQRT2 * (fc + bl + sl) + 0.5 * lfe;
            for idx in (8..tc as usize).step_by(2) {
                sum += INV_SQRT2 * base_slice.get(idx).copied().unwrap_or(0.0);
            }
            sum * (2.0 / (tc as f32).sqrt()).min(0.6)
        }
        (tc, 2, 1) if tc > 8 => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let lfe = base_slice.get(3).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            let mut sum = fr + INV_SQRT2 * (fc + br + sr) + 0.5 * lfe;
            for idx in (9..tc as usize).step_by(2) {
                sum += INV_SQRT2 * base_slice.get(idx).copied().unwrap_or(0.0);
            }
            sum * (2.0 / (tc as f32).sqrt()).min(0.6)
        }
        // >8 channels to Mono
        (tc, 1, 0) if tc > 8 => {
            let sum: f32 = base_slice.iter().take(tc as usize).copied().sum();
            sum / (tc as f32).sqrt()
        }
        // 7.1 / 7.1.4 (tc >= 8) to 5.1 (hc == 6): fold Back + Side surround pairs into 5.1 surround L/R
        (tc, 6, 4) if tc >= 8 => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            (bl + sl) * INV_SQRT2
        }
        (tc, 6, 5) if tc >= 8 => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            (br + sr) * INV_SQRT2
        }
        // General fallback: if channel index exists in source, take it
        (tc, _, c) if c < tc as usize => base_slice.get(c).copied().unwrap_or(0.0),
        _ => 0.0,
    }
}

struct SymphoniaSource {
    format: Box<dyn FormatReader>,
    active_tracks: std::collections::HashMap<u32, ActiveSymphoniaTrack>,
    primary_track_id: u32,
    time_base: f64,
    current_time: f64,
    duration: f64,
    channels: u16,
    channel_vus: Vec<f32>,
    artist: String,
    ext_type: String,
    intrinsic_sample_rate: Option<u32>,
    video_info: Option<String>,
    audio_tracks: Vec<crate::state::AudioTrackInfo>,
    selected_track_idx: usize,
    has_video: bool,
    bitrate: Option<u32>,
    is_full_mix_guide: bool,
}

impl AudioSource for SymphoniaSource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        _sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        let mut frames_written = 0;
        let frames_needed = output.len() / hardware_channels;
        self.channel_vus.fill(0.0);

        let is_2track = self.audio_tracks.len() == 2;

        // Dynamic energy-cancellation check for 2-track audio
        if !self.is_full_mix_guide && is_2track {
            let t0_opt = self.active_tracks.values().find(|t| t.track_idx == 0);
            let t1_opt = self.active_tracks.values().find(|t| t.track_idx == 1);
            if let (Some(t0), Some(t1)) = (t0_opt, t1_opt) {
                let ch0 = t0.channels as usize;
                let ch1 = t1.channels as usize;
                let avail0 = (t0.samples.len().saturating_sub(t0.buf_pos)) / ch0.max(1);
                let avail1 = (t1.samples.len().saturating_sub(t1.buf_pos)) / ch1.max(1);
                let check_frames = avail0.min(avail1).min(512);
                if check_frames >= 128 {
                    let mut e0 = 0.0f32;
                    let mut e1 = 0.0f32;
                    let mut ediff = 0.0f32;
                    for i in 0..check_frames {
                        let s0 = t0.samples.get(t0.buf_pos + i * ch0).copied().unwrap_or(0.0);
                        let s1 = t1.samples.get(t1.buf_pos + i * ch1).copied().unwrap_or(0.0);
                        e0 += s0 * s0;
                        e1 += s1 * s1;
                        ediff += (s1 - s0) * (s1 - s0);
                    }
                    if e0 > 0.001 && e1 > 0.001 && ediff < 0.35 * e1 {
                        self.is_full_mix_guide = true;
                        eprintln!(
                            "[RustTracker Audio] Detected Full Mix vocal guide stem - enabling phase cancellation"
                        );
                        #[cfg(target_os = "android")]
                        crate::android::log_android(
                            3,
                            "[RustTracker Audio] Detected Full Mix vocal guide stem - enabling phase cancellation",
                        );
                    }
                }
            }
        }

        let need_t0 = is_2track && (self.is_full_mix_guide || self.current_time < 3.0);

        while frames_written < frames_needed {
            let mut need_more = false;
            for track in self.active_tracks.values().filter(|t| {
                t.enabled || t.track_id == self.primary_track_id || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.samples.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                if avail < (frames_needed - frames_written) {
                    need_more = true;
                    break;
                }
            }

            if need_more {
                for _ in 0..512 {
                    let all_satisfied = self
                        .active_tracks
                        .values()
                        .filter(|t| {
                            t.enabled
                                || t.track_id == self.primary_track_id
                                || (need_t0 && t.track_idx == 0)
                        })
                        .all(|t| {
                            let avail = (t.samples.len().saturating_sub(t.buf_pos))
                                / t.channels.max(1) as usize;
                            avail >= (frames_needed - frames_written)
                        });
                    if all_satisfied {
                        break;
                    }

                    match self.format.next_packet() {
                        Ok(packet) => {
                            let pkt_track_id = packet.track_id();
                            if let Some(track) = self.active_tracks.get_mut(&pkt_track_id)
                                && (track.enabled
                                    || track.track_id == self.primary_track_id
                                    || (need_t0 && track.track_idx == 0))
                                && let Ok(decoded) = track.decoder.decode(&packet)
                            {
                                if track.sample_buf.capacity() < decoded.capacity() {
                                    track.sample_buf = SampleBuffer::<f32>::new(
                                        decoded.capacity() as u64,
                                        *decoded.spec(),
                                    );
                                }
                                track.sample_buf.copy_interleaved_ref(decoded);

                                if track.buf_pos > 4096 {
                                    track.samples.drain(0..track.buf_pos);
                                    track.buf_pos = 0;
                                }
                                track.samples.extend_from_slice(track.sample_buf.samples());
                            }
                        }
                        Err(_) => break, // EOF
                    }
                }
            }

            let mut min_available = usize::MAX;
            for track in self.active_tracks.values().filter(|t| {
                t.enabled || t.track_id == self.primary_track_id || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.samples.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                min_available = min_available.min(avail);
            }

            if min_available == 0 || min_available == usize::MAX {
                if let Some(primary) = self.active_tracks.get(&self.primary_track_id) {
                    let avail = (primary.samples.len().saturating_sub(primary.buf_pos))
                        / primary.channels.max(1) as usize;
                    if avail == 0 {
                        break;
                    }
                    min_available = avail;
                } else {
                    break;
                }
            }

            let frames_to_copy = min_available.min(frames_needed - frames_written);
            if frames_to_copy == 0 {
                break;
            }

            let headroom = calculate_mix_headroom(
                self.active_tracks
                    .values()
                    .filter(|t| t.enabled)
                    .map(|t| t.volume),
            );

            let is_full_mix = self.is_full_mix_guide && is_2track;
            let (t0_info, t1_info) = if is_full_mix {
                let mut t0 = None;
                let mut t1 = None;
                for t in self.active_tracks.values() {
                    if t.track_idx == 0 {
                        t0 = Some((t.buf_pos, t.channels, t.volume, t.enabled));
                    } else if t.track_idx == 1 {
                        t1 = Some((t.buf_pos, t.channels, t.volume, t.enabled));
                    }
                }
                (t0, t1)
            } else {
                (None, None)
            };

            if is_full_mix && let (Some(t0), Some(t1)) = (t0_info, t1_info) {
                let v0 = if t0.3 { t0.2 } else { 0.0 };
                let v1 = if t1.3 { t1.2 } else { 0.0 };

                let t0_samples = &self
                    .active_tracks
                    .values()
                    .find(|t| t.track_idx == 0)
                    .unwrap()
                    .samples;
                let t1_samples = &self
                    .active_tracks
                    .values()
                    .find(|t| t.track_idx == 1)
                    .unwrap()
                    .samples;

                for f in 0..frames_to_copy {
                    let out_base = (frames_written + f) * hardware_channels;
                    let base_idx_0 = t0.0 + f * t0.1 as usize;
                    let base_idx_1 = t1.0 + f * t1.1 as usize;
                    let ch_slice_0 = if base_idx_0 < t0_samples.len() {
                        &t0_samples[base_idx_0..]
                    } else {
                        &[]
                    };
                    let ch_slice_1 = if base_idx_1 < t1_samples.len() {
                        &t1_samples[base_idx_1..]
                    } else {
                        &[]
                    };

                    for c in 0..hardware_channels {
                        let s0 = get_downmixed_sample(t0.1, ch_slice_0, hardware_channels, c);
                        let s1 = get_downmixed_sample(t1.1, ch_slice_1, hardware_channels, c);
                        // Extracted Vocals V = s1 - s0
                        // Desired output: v0 * I + v1 * V = (v0 - v1) * s0 + v1 * s1
                        let sum_val = (v0 - v1) * s0 + v1 * s1;
                        let final_val = sum_val.clamp(-1.0, 1.0);
                        output[out_base + c] = final_val;

                        if c < self.channel_vus.len() {
                            self.channel_vus[c] = self.channel_vus[c].max(final_val.abs());
                        }
                    }
                }
            } else {
                for f in 0..frames_to_copy {
                    let out_base = (frames_written + f) * hardware_channels;

                    // Update true channel VUs across all source channels for each active track
                    for track in self.active_tracks.values() {
                        if track.enabled && track.volume > 0.0 {
                            let base_idx = track.buf_pos + f * track.channels as usize;
                            for ch in 0..track.channels as usize {
                                if ch < self.channel_vus.len() {
                                    let s = track
                                        .samples
                                        .get(base_idx + ch)
                                        .copied()
                                        .unwrap_or(0.0)
                                        .abs();
                                    self.channel_vus[ch] =
                                        self.channel_vus[ch].max(s * track.volume);
                                }
                            }
                        }
                    }

                    for c in 0..hardware_channels {
                        let mut sum_val = 0.0f32;
                        for track in self.active_tracks.values() {
                            if track.enabled && track.volume > 0.0 {
                                let base_idx = track.buf_pos + f * track.channels as usize;
                                let ch_slice = if base_idx < track.samples.len() {
                                    &track.samples[base_idx..]
                                } else {
                                    &[]
                                };
                                let s_val = get_downmixed_sample(
                                    track.channels,
                                    ch_slice,
                                    hardware_channels,
                                    c,
                                );
                                sum_val += s_val * track.volume;
                            }
                        }
                        let final_val = (sum_val * headroom).clamp(-1.0, 1.0);
                        output[out_base + c] = final_val;
                    }
                }
            }

            for track in self.active_tracks.values_mut().filter(|t| {
                t.enabled || t.track_id == self.primary_track_id || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.samples.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                let consume = frames_to_copy.min(avail);
                track.buf_pos += consume * track.channels as usize;
                track.current_time +=
                    consume as f64 / self.intrinsic_sample_rate.unwrap_or(44100) as f64;
            }
            if let Some(primary) = self.active_tracks.get(&self.primary_track_id) {
                self.current_time = primary.current_time;
            }
            frames_written += frames_to_copy;
        }

        frames_written
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.current_time
    }

    fn set_position_seconds(&mut self, pos: f64) {
        let ts = (pos / self.time_base) as u64;
        let seek_res = self.format.seek(
            symphonia::core::formats::SeekMode::Coarse,
            symphonia::core::formats::SeekTo::TimeStamp {
                ts,
                track_id: self.primary_track_id,
            },
        );

        if seek_res.is_ok() {
            for track in self.active_tracks.values_mut() {
                track.decoder.reset();
                track.current_time = pos;
                track.samples.clear();
                track.buf_pos = 0;
            }
            self.current_time = pos;
        } else {
            let seek_res_start = self.format.seek(
                symphonia::core::formats::SeekMode::Coarse,
                symphonia::core::formats::SeekTo::TimeStamp {
                    ts: 0,
                    track_id: self.primary_track_id,
                },
            );
            if seek_res_start.is_ok() {
                for track in self.active_tracks.values_mut() {
                    track.decoder.reset();
                    track.current_time = pos;
                    track.samples.clear();
                    track.buf_pos = 0;
                }
                self.current_time = pos;
            }
        }
    }

    fn get_num_channels(&mut self) -> i32 {
        self.channels as i32
    }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        self.channel_vus
            .get(channel as usize)
            .cloned()
            .unwrap_or(0.0)
    }
    fn get_type(&mut self) -> String {
        self.ext_type.clone()
    }
    fn get_tempo(&mut self) -> i32 {
        0
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        self.intrinsic_sample_rate
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        0
    }
    fn get_num_patterns(&mut self) -> i32 {
        0
    }
    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        0
    }
    fn get_tracker_channels(&mut self) -> Option<i32> {
        None
    }
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        Vec::new()
    }
    fn get_current_row_string(&mut self) -> String {
        String::new()
    }
    fn get_artist(&mut self) -> String {
        self.artist.clone()
    }
    fn get_video_info(&mut self) -> Option<String> {
        self.video_info.clone()
    }

    fn get_audio_tracks(&self) -> Vec<crate::state::AudioTrackInfo> {
        self.audio_tracks.clone()
    }
    fn get_selected_audio_track(&self) -> usize {
        self.selected_track_idx
    }

    fn get_active_audio_tracks(&self) -> Vec<usize> {
        let mut list: Vec<usize> = self
            .active_tracks
            .values()
            .filter(|t| t.enabled)
            .map(|t| t.track_idx)
            .collect();
        list.sort();
        list.dedup();
        if list.is_empty() {
            vec![self.selected_track_idx]
        } else {
            list
        }
    }

    fn set_active_audio_tracks(&mut self, tracks: &[(usize, f32)]) -> Result<()> {
        if tracks.is_empty() {
            return Ok(());
        }

        let track_map: std::collections::HashMap<usize, f32> = tracks.iter().copied().collect();
        for track in self.active_tracks.values_mut() {
            if let Some(&vol) = track_map.get(&track.track_idx) {
                track.enabled = true;
                track.volume = vol;
            } else {
                track.enabled = false;
                track.volume = 0.0;
            }
        }

        if let Some(&(first_idx, _)) = tracks.first()
            && first_idx < self.audio_tracks.len()
        {
            self.selected_track_idx = first_idx;
            self.primary_track_id = self.audio_tracks[first_idx].id as u32;
            let container = self.ext_type.split('/').nth(1).map(|s| s.to_string());
            if let Some(container) = container {
                self.ext_type = format!("{}/{}", self.audio_tracks[first_idx].codec, container);
            } else {
                self.ext_type = self.audio_tracks[first_idx].codec.clone();
            }
            self.channels = self.audio_tracks[first_idx].channels;
            self.channel_vus = vec![0.0; self.channels as usize];
            self.intrinsic_sample_rate = Some(self.audio_tracks[first_idx].sample_rate);
            if let Some(t) = self.active_tracks.get(&self.primary_track_id) {
                self.time_base = t.time_base;
            }
        }
        Ok(())
    }

    fn select_audio_track(&mut self, track_idx: usize) -> Result<()> {
        self.set_active_audio_tracks(&[(track_idx, 1.0)])
    }

    fn has_video_stream(&self) -> bool {
        self.has_video
    }

    fn get_bitrate(&mut self) -> Option<u32> {
        self.bitrate
    }
}

// ---------------------------------------------------------
// FFmpeg Native Decoder (including AOMedia IAMF / Eclipsa Audio)
// ---------------------------------------------------------
#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug)]
pub struct IamfSubstreamChannelMap {
    pub stream_index: usize,
    pub substream_channel: usize,
    /// `(target_speaker_idx, linear_gain)` in canonical speaker order:
    /// `[0:FL, 1:FR, 2:FC, 3:LFE, 4:BL, 5:BR, 6:SL, 7:SR, 8:TFL, 9:TFR, 10:TBL, 11:TBR]`
    pub targets: Vec<(usize, f32)>,
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug)]
pub struct IamfTrackGroup {
    pub substream_indices: Vec<usize>,
    pub reconstructed_channels: u16,
    pub layout_label: String,
    pub annotation: Option<String>,
    pub language: Option<String>,
    pub channel_mappings: Vec<IamfSubstreamChannelMap>,
}

#[cfg(not(target_os = "android"))]
#[repr(C)]
struct FfiIamfLayer {
    av_class: *const ffmpeg_next::ffi::AVClass,
    ch_layout: ffmpeg_next::ffi::AVChannelLayout,
    flags: std::ffi::c_uint,
    output_gain_flags: std::ffi::c_uint,
    output_gain: ffmpeg_next::ffi::AVRational,
    ambisonics_mode: std::ffi::c_uint,
    demixing_matrix: *mut ffmpeg_next::ffi::AVRational,
    nb_demixing_matrix: std::ffi::c_uint,
}

#[cfg(not(target_os = "android"))]
#[repr(C)]
struct FfiIamfAudioElement {
    av_class: *const ffmpeg_next::ffi::AVClass,
    layers: *mut *mut FfiIamfLayer,
    nb_layers: std::ffi::c_uint,
    demixing_info: *mut std::ffi::c_void,
    recon_gain_info: *mut std::ffi::c_void,
    audio_element_type: std::ffi::c_uint,
    default_w: std::ffi::c_uint,
}

#[cfg(not(target_os = "android"))]
#[repr(C)]
struct FfiIamfSubmixElement {
    av_class: *const ffmpeg_next::ffi::AVClass,
    audio_element_id: std::ffi::c_uint,
    element_mix_config: *mut std::ffi::c_void,
    default_mix_gain: ffmpeg_next::ffi::AVRational,
    headphones_rendering_mode: std::ffi::c_uint,
    annotations: *mut ffmpeg_next::ffi::AVDictionary,
}

#[cfg(not(target_os = "android"))]
#[repr(C)]
struct FfiIamfSubmix {
    av_class: *const ffmpeg_next::ffi::AVClass,
    elements: *mut *mut FfiIamfSubmixElement,
    nb_elements: std::ffi::c_uint,
    layouts: *mut *mut std::ffi::c_void,
    nb_layouts: std::ffi::c_uint,
    output_mix_config: *mut std::ffi::c_void,
    default_mix_gain: ffmpeg_next::ffi::AVRational,
}

#[cfg(not(target_os = "android"))]
#[repr(C)]
struct FfiIamfMixPresentation {
    av_class: *const ffmpeg_next::ffi::AVClass,
    submixes: *mut *mut FfiIamfSubmix,
    nb_submixes: std::ffi::c_uint,
    annotations: *mut ffmpeg_next::ffi::AVDictionary,
}

#[cfg(not(target_os = "android"))]
fn iamf_rational_db_to_linear(r: ffmpeg_next::ffi::AVRational) -> f32 {
    if r.den == 0 || r.num == 0 {
        return 1.0;
    }
    let db = (r.num as f32 / r.den as f32).clamp(-60.0, 24.0);
    10.0f32.powf(db / 20.0)
}

#[cfg(not(target_os = "android"))]
unsafe fn extract_first_av_dict_entry(
    dict: *const ffmpeg_next::ffi::AVDictionary,
) -> (Option<String>, Option<String>) {
    if dict.is_null() {
        return (None, None);
    }
    let entry = unsafe {
        ffmpeg_next::ffi::av_dict_get(
            dict,
            c"".as_ptr(),
            std::ptr::null(),
            ffmpeg_next::ffi::AV_DICT_IGNORE_SUFFIX,
        )
    };
    if entry.is_null() {
        return (None, None);
    }
    let key = if !unsafe { (*entry).key }.is_null() {
        unsafe { std::ffi::CStr::from_ptr((*entry).key) }
            .to_str()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.to_lowercase() != "und")
    } else {
        None
    };
    let val = if !unsafe { (*entry).value }.is_null() {
        unsafe { std::ffi::CStr::from_ptr((*entry).value) }
            .to_str()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "default_mix_presentation")
    } else {
        None
    };
    (key, val)
}

#[cfg(not(target_os = "android"))]
pub fn map_iamf_av_channels_to_canonical(
    av_channels: &[i32],
    substreams: &[(usize, u16)],
) -> (u16, String, Vec<IamfSubstreamChannelMap>) {
    use ffmpeg_next::ffi::AVChannel::*;
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const AV_CHAN_BINAURAL_LEFT_ID: i32 = 61;
    const AV_CHAN_BINAURAL_RIGHT_ID: i32 = 62;

    let has_height = av_channels.iter().any(|&c| {
        c == AV_CHAN_TOP_CENTER as i32
            || c == AV_CHAN_TOP_FRONT_LEFT as i32
            || c == AV_CHAN_TOP_FRONT_CENTER as i32
            || c == AV_CHAN_TOP_FRONT_RIGHT as i32
            || c == AV_CHAN_TOP_BACK_LEFT as i32
            || c == AV_CHAN_TOP_BACK_CENTER as i32
            || c == AV_CHAN_TOP_BACK_RIGHT as i32
            || c == AV_CHAN_TOP_SIDE_LEFT as i32
            || c == AV_CHAN_TOP_SIDE_RIGHT as i32
            || c == AV_CHAN_TOP_SURROUND_LEFT as i32
            || c == AV_CHAN_TOP_SURROUND_RIGHT as i32
    });
    let has_back = av_channels
        .iter()
        .any(|&c| c == AV_CHAN_BACK_LEFT as i32 || c == AV_CHAN_BACK_RIGHT as i32);
    let has_side = av_channels.iter().any(|&c| {
        c == AV_CHAN_SIDE_LEFT as i32
            || c == AV_CHAN_SIDE_RIGHT as i32
            || c == AV_CHAN_SIDE_SURROUND_LEFT as i32
            || c == AV_CHAN_SIDE_SURROUND_RIGHT as i32
            || c == AV_CHAN_SURROUND_DIRECT_LEFT as i32
            || c == AV_CHAN_SURROUND_DIRECT_RIGHT as i32
    });
    let has_top_back = av_channels
        .iter()
        .any(|&c| c == AV_CHAN_TOP_BACK_LEFT as i32 || c == AV_CHAN_TOP_BACK_RIGHT as i32);

    let nb_ch = av_channels.len();
    let (recon_channels, layout_label) = if has_height {
        let lbl = match nb_ch {
            12 => "7.1.4".to_string(),
            10 if has_back && has_side => "7.1.2".to_string(),
            10 => "5.1.4".to_string(),
            8 => "5.1.2".to_string(),
            6 => "3.1.2".to_string(),
            14 => "9.1.4".to_string(),
            16 => "9.1.6".to_string(),
            n => format!("{} ch", n),
        };
        (12u16, lbl)
    } else if nb_ch >= 7 {
        (
            8u16,
            if nb_ch == 8 {
                "7.1".to_string()
            } else {
                "6.1".to_string()
            },
        )
    } else if nb_ch >= 5 {
        (
            6u16,
            if nb_ch == 6 {
                "5.1".to_string()
            } else {
                "5.0".to_string()
            },
        )
    } else if nb_ch == 4 {
        (4u16, "4.0".to_string())
    } else if nb_ch == 3 {
        (3u16, "3.0".to_string())
    } else if nb_ch == 2 {
        (2u16, "Stereo".to_string())
    } else {
        (1u16, "Mono".to_string())
    };

    let mut mappings = Vec::with_capacity(nb_ch);
    let mut flat_idx = 0usize;
    for &(stream_index, sub_ch_count) in substreams {
        for sub_ch in 0..(sub_ch_count as usize) {
            let chan_id = av_channels
                .get(flat_idx)
                .copied()
                .unwrap_or(AV_CHAN_NONE as i32);
            flat_idx += 1;

            let targets: Vec<(usize, f32)> = if chan_id == AV_CHAN_FRONT_LEFT as i32
                || chan_id == AV_CHAN_STEREO_LEFT as i32
                || chan_id == AV_CHAN_BINAURAL_LEFT_ID
                || chan_id == AV_CHAN_WIDE_LEFT as i32
            {
                vec![(0, 1.0)]
            } else if chan_id == AV_CHAN_FRONT_RIGHT as i32
                || chan_id == AV_CHAN_STEREO_RIGHT as i32
                || chan_id == AV_CHAN_BINAURAL_RIGHT_ID
                || chan_id == AV_CHAN_WIDE_RIGHT as i32
            {
                vec![(1, 1.0)]
            } else if chan_id == AV_CHAN_FRONT_CENTER as i32 {
                if recon_channels == 1 {
                    vec![(0, 1.0)]
                } else if recon_channels == 2 {
                    vec![(0, INV_SQRT2), (1, INV_SQRT2)]
                } else {
                    vec![(2, 1.0)]
                }
            } else if chan_id == AV_CHAN_LOW_FREQUENCY as i32
                || chan_id == AV_CHAN_LOW_FREQUENCY_2 as i32
            {
                if recon_channels >= 4 {
                    vec![(3, 1.0)]
                } else {
                    vec![(0, 0.5), (1, 0.5)]
                }
            } else if chan_id == AV_CHAN_BACK_LEFT as i32 {
                if recon_channels == 6 {
                    vec![(4, 1.0)]
                } else if recon_channels >= 8 {
                    if !has_side {
                        vec![(4, INV_SQRT2), (6, INV_SQRT2)]
                    } else {
                        vec![(4, 1.0)]
                    }
                } else if recon_channels == 4 {
                    vec![(2, 1.0)]
                } else {
                    vec![(0, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_BACK_RIGHT as i32 {
                if recon_channels == 6 {
                    vec![(5, 1.0)]
                } else if recon_channels >= 8 {
                    if !has_side {
                        vec![(5, INV_SQRT2), (7, INV_SQRT2)]
                    } else {
                        vec![(5, 1.0)]
                    }
                } else if recon_channels == 4 {
                    vec![(3, 1.0)]
                } else {
                    vec![(1, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_SIDE_LEFT as i32
                || chan_id == AV_CHAN_SIDE_SURROUND_LEFT as i32
                || chan_id == AV_CHAN_SURROUND_DIRECT_LEFT as i32
            {
                if recon_channels == 6 {
                    vec![(4, 1.0)]
                } else if recon_channels >= 8 {
                    if !has_back {
                        vec![(4, INV_SQRT2), (6, INV_SQRT2)]
                    } else {
                        vec![(6, 1.0)]
                    }
                } else if recon_channels == 4 {
                    vec![(2, 1.0)]
                } else {
                    vec![(0, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_SIDE_RIGHT as i32
                || chan_id == AV_CHAN_SIDE_SURROUND_RIGHT as i32
                || chan_id == AV_CHAN_SURROUND_DIRECT_RIGHT as i32
            {
                if recon_channels == 6 {
                    vec![(5, 1.0)]
                } else if recon_channels >= 8 {
                    if !has_back {
                        vec![(5, INV_SQRT2), (7, INV_SQRT2)]
                    } else {
                        vec![(7, 1.0)]
                    }
                } else if recon_channels == 4 {
                    vec![(3, 1.0)]
                } else {
                    vec![(1, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_BACK_CENTER as i32 {
                if recon_channels >= 6 {
                    vec![(4, INV_SQRT2), (5, INV_SQRT2)]
                } else {
                    vec![(0, 0.5), (1, 0.5)]
                }
            } else if chan_id == AV_CHAN_TOP_FRONT_LEFT as i32
                || chan_id == AV_CHAN_TOP_SIDE_LEFT as i32
                || chan_id == AV_CHAN_TOP_SURROUND_LEFT as i32
            {
                if recon_channels >= 12 {
                    if !has_top_back {
                        vec![(8, INV_SQRT2), (10, INV_SQRT2)]
                    } else {
                        vec![(8, 1.0)]
                    }
                } else {
                    vec![(0, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_FRONT_RIGHT as i32
                || chan_id == AV_CHAN_TOP_SIDE_RIGHT as i32
                || chan_id == AV_CHAN_TOP_SURROUND_RIGHT as i32
            {
                if recon_channels >= 12 {
                    if !has_top_back {
                        vec![(9, INV_SQRT2), (11, INV_SQRT2)]
                    } else {
                        vec![(9, 1.0)]
                    }
                } else {
                    vec![(1, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_BACK_LEFT as i32 {
                if recon_channels >= 12 {
                    vec![(10, 1.0)]
                } else {
                    vec![(4, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_BACK_RIGHT as i32 {
                if recon_channels >= 12 {
                    vec![(11, 1.0)]
                } else {
                    vec![(5, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_FRONT_CENTER as i32 {
                if recon_channels >= 12 {
                    vec![(8, INV_SQRT2), (9, INV_SQRT2)]
                } else {
                    vec![(2, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_BACK_CENTER as i32 {
                if recon_channels >= 12 {
                    vec![(10, INV_SQRT2), (11, INV_SQRT2)]
                } else {
                    vec![(4, 0.5), (5, 0.5)]
                }
            } else if chan_id == AV_CHAN_TOP_CENTER as i32 {
                if recon_channels >= 12 {
                    vec![(8, 0.5), (9, 0.5), (10, 0.5), (11, 0.5)]
                } else {
                    vec![(0, 0.5), (1, 0.5)]
                }
            } else if chan_id == AV_CHAN_FRONT_LEFT_OF_CENTER as i32 {
                if recon_channels >= 3 {
                    vec![(0, 0.8), (2, 0.5)]
                } else {
                    vec![(0, 1.0)]
                }
            } else if chan_id == AV_CHAN_FRONT_RIGHT_OF_CENTER as i32 {
                if recon_channels >= 3 {
                    vec![(1, 0.8), (2, 0.5)]
                } else {
                    vec![(1, 1.0)]
                }
            } else {
                let fallback_ch = (flat_idx - 1).min(recon_channels.saturating_sub(1) as usize);
                vec![(fallback_ch, 1.0)]
            };

            mappings.push(IamfSubstreamChannelMap {
                stream_index,
                substream_channel: sub_ch,
                targets,
            });
        }
    }

    (recon_channels, layout_label, mappings)
}

#[cfg(not(target_os = "android"))]
fn map_iamf_ambisonics_to_canonical(
    substreams: &[(usize, u16)],
    nb_acn_channels: usize,
) -> (u16, String, Vec<IamfSubstreamChannelMap>) {
    // ITU-R BS.2051-3 7.1.4 speaker unit vectors (x=cos(az)*cos(el), y=sin(az)*cos(el), z=sin(el))
    // in canonical speaker order [0:FL, 1:FR, 2:FC, 3:LFE, 4:BL, 5:BR, 6:SL, 7:SR, 8:TFL, 9:TFR, 10:TBL, 11:TBR]
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const SPEAKER_DIRS: [(f32, f32, f32); 12] = [
        (0.8660, 0.5000, 0.0000),      // 0: FL (+30, 0)
        (0.8660, -0.5000, 0.0000),     // 1: FR (-30, 0)
        (1.0000, 0.0000, 0.0000),      // 2: FC (0, 0)
        (0.0000, 0.0000, 0.0000),      // 3: LFE (omni W only)
        (-INV_SQRT2, INV_SQRT2, 0.0),  // 4: BL (+135, 0)
        (-INV_SQRT2, -INV_SQRT2, 0.0), // 5: BR (-135, 0)
        (0.0000, 1.0000, 0.0000),      // 6: SL (+90, 0)
        (0.0000, -1.0000, 0.0000),     // 7: SR (-90, 0)
        (0.5000, 0.5000, INV_SQRT2),   // 8: TFL (+45, +45)
        (0.5000, -0.5000, INV_SQRT2),  // 9: TFR (-45, +45)
        (-0.5000, 0.5000, INV_SQRT2),  // 10: TBL (+135, +45)
        (-0.5000, -0.5000, INV_SQRT2), // 11: TBR (-135, +45)
    ];

    let mut mappings = Vec::new();
    let mut acn_idx = 0usize;
    for &(stream_index, sub_ch_count) in substreams {
        for sub_ch in 0..(sub_ch_count as usize) {
            let mut targets = Vec::new();
            match acn_idx {
                // ACN 0 = W (omnidirectional)
                0 => {
                    for s in 0..12 {
                        let w = if s == 3 { 0.25 } else { 0.35 };
                        targets.push((s, w));
                    }
                }
                // ACN 1 = Y (Left-Right dipole)
                1 => {
                    for (s, &(_, y, _)) in SPEAKER_DIRS.iter().enumerate() {
                        if s != 3 && y.abs() > 0.01 {
                            targets.push((s, 0.35 * y));
                        }
                    }
                }
                // ACN 2 = Z (Up-Down dipole)
                2 => {
                    for (s, &(_, _, z)) in SPEAKER_DIRS.iter().enumerate() {
                        if s != 3 && z.abs() > 0.01 {
                            targets.push((s, 0.35 * z));
                        }
                    }
                }
                // ACN 3 = X (Front-Back dipole)
                3 => {
                    for (s, &(x, _, _)) in SPEAKER_DIRS.iter().enumerate() {
                        if s != 3 && x.abs() > 0.01 {
                            targets.push((s, 0.35 * x));
                        }
                    }
                }
                _ => {}
            }
            mappings.push(IamfSubstreamChannelMap {
                stream_index,
                substream_channel: sub_ch,
                targets,
            });
            acn_idx += 1;
        }
    }

    let order_label = match nb_acn_channels {
        4 => "FOA 7.1.4".to_string(),
        9 => "SOA 7.1.4".to_string(),
        16 => "TOA 7.1.4".to_string(),
        n => format!("Ambisonics {}ch", n),
    };
    (12, order_label, mappings)
}

#[cfg(not(target_os = "android"))]
fn probe_iamf_stream_groups(ictx: &ffmpeg_next::format::context::Input) -> Vec<IamfTrackGroup> {
    let mut result = Vec::new();

    unsafe {
        let fmt_ptr = ictx.as_ptr();
        if fmt_ptr.is_null() {
            return result;
        }
        let nb_groups = (*fmt_ptr).nb_stream_groups as usize;
        let groups_ptr = (*fmt_ptr).stream_groups;
        if nb_groups == 0 || groups_ptr.is_null() {
            return result;
        }

        let mut audio_elements: Vec<(i64, IamfTrackGroup)> = Vec::new();

        // 1. Parse all IAMF Audio Element stream groups
        for g_idx in 0..nb_groups {
            let grp = *groups_ptr.add(g_idx);
            if grp.is_null() {
                continue;
            }
            if (*grp).type_
                != ffmpeg_next::ffi::AVStreamGroupParamsType::AV_STREAM_GROUP_PARAMS_IAMF_AUDIO_ELEMENT
            {
                continue;
            }
            let elem_ptr = (*grp).params.iamf_audio_element as *const FfiIamfAudioElement;
            if elem_ptr.is_null() || (*elem_ptr).nb_layers == 0 || (*elem_ptr).layers.is_null() {
                continue;
            }

            let nb_streams = (*grp).nb_streams as usize;
            if nb_streams == 0 || (*grp).streams.is_null() {
                continue;
            }

            let mut substreams = Vec::with_capacity(nb_streams);
            let mut substream_indices = Vec::with_capacity(nb_streams);
            for s in 0..nb_streams {
                let st_ptr = *(*grp).streams.add(s);
                if st_ptr.is_null() || (*st_ptr).codecpar.is_null() {
                    continue;
                }
                let s_idx = (*st_ptr).index as usize;
                let ch = (*(*st_ptr).codecpar).ch_layout.nb_channels.max(1) as u16;
                substreams.push((s_idx, ch));
                if !substream_indices.contains(&s_idx) {
                    substream_indices.push(s_idx);
                }
            }
            if substreams.is_empty() {
                continue;
            }

            let nb_layers = (*elem_ptr).nb_layers as usize;
            let final_layer = *(*elem_ptr).layers.add(nb_layers - 1);
            if final_layer.is_null() {
                continue;
            }

            let layout_ptr = &(*final_layer).ch_layout;
            let nb_layout_ch = layout_ptr.nb_channels.max(1) as usize;
            let layer_gain = if (*final_layer).output_gain_flags != 0 {
                iamf_rational_db_to_linear((*final_layer).output_gain)
            } else {
                1.0
            };

            let (recon_channels, layout_label, mut mappings) =
                if (*elem_ptr).audio_element_type == 1 {
                    map_iamf_ambisonics_to_canonical(&substreams, nb_layout_ch)
                } else {
                    let mut av_channels = Vec::with_capacity(nb_layout_ch);
                    for c_idx in 0..nb_layout_ch {
                        let ch_enum = ffmpeg_next::ffi::av_channel_layout_channel_from_index(
                            layout_ptr,
                            c_idx as std::ffi::c_uint,
                        );
                        av_channels.push(ch_enum as i32);
                    }
                    map_iamf_av_channels_to_canonical(&av_channels, &substreams)
                };

            if (layer_gain - 1.0).abs() > 1e-4 {
                for m in &mut mappings {
                    for t in &mut m.targets {
                        t.1 *= layer_gain;
                    }
                }
            }

            audio_elements.push((
                (*grp).id,
                IamfTrackGroup {
                    substream_indices,
                    reconstructed_channels: recon_channels,
                    layout_label,
                    annotation: None,
                    language: None,
                    channel_mappings: mappings,
                },
            ));
        }

        if audio_elements.is_empty() {
            return result;
        }

        let mut used_element_ids = std::collections::HashSet::new();

        // 2. Parse IAMF Mix Presentation stream groups referencing the Audio Elements
        for g_idx in 0..nb_groups {
            let grp = *groups_ptr.add(g_idx);
            if grp.is_null() {
                continue;
            }
            if (*grp).type_
                != ffmpeg_next::ffi::AVStreamGroupParamsType::AV_STREAM_GROUP_PARAMS_IAMF_MIX_PRESENTATION
            {
                continue;
            }
            let mix_ptr = (*grp).params.iamf_mix_presentation as *const FfiIamfMixPresentation;
            if mix_ptr.is_null() || (*mix_ptr).nb_submixes == 0 || (*mix_ptr).submixes.is_null() {
                continue;
            }

            let (mix_lang, mix_annot) = extract_first_av_dict_entry((*mix_ptr).annotations);
            let nb_submixes = (*mix_ptr).nb_submixes as usize;

            for sm_idx in 0..nb_submixes {
                let submix_ptr = *(*mix_ptr).submixes.add(sm_idx);
                if submix_ptr.is_null()
                    || (*submix_ptr).nb_elements == 0
                    || (*submix_ptr).elements.is_null()
                {
                    continue;
                }
                let submix_gain = iamf_rational_db_to_linear((*submix_ptr).default_mix_gain);
                let nb_elems = (*submix_ptr).nb_elements as usize;

                let mut combined_substreams = Vec::new();
                let mut combined_mappings = Vec::new();
                let mut max_recon_ch = 1u16;
                let mut primary_layout_label = String::new();
                let mut elem_annot_opt = None;
                let mut elem_lang_opt = None;

                for el_idx in 0..nb_elems {
                    let sub_el = *(*submix_ptr).elements.add(el_idx);
                    if sub_el.is_null() {
                        continue;
                    }
                    let target_id = (*sub_el).audio_element_id as i64;
                    let elem_gain =
                        submix_gain * iamf_rational_db_to_linear((*sub_el).default_mix_gain);
                    let (el_lang, el_annot) = extract_first_av_dict_entry((*sub_el).annotations);
                    if elem_lang_opt.is_none() {
                        elem_lang_opt = el_lang;
                    }
                    if elem_annot_opt.is_none() {
                        elem_annot_opt = el_annot;
                    }

                    if let Some((_, elem_group)) =
                        audio_elements.iter().find(|(id, _)| *id == target_id)
                    {
                        used_element_ids.insert(target_id);
                        for &s_idx in &elem_group.substream_indices {
                            if !combined_substreams.contains(&s_idx) {
                                combined_substreams.push(s_idx);
                            }
                        }
                        if elem_group.reconstructed_channels >= max_recon_ch {
                            max_recon_ch = elem_group.reconstructed_channels;
                            primary_layout_label = elem_group.layout_label.clone();
                        }
                        for mut m in elem_group.channel_mappings.clone() {
                            if (elem_gain - 1.0).abs() > 1e-4 {
                                for t in &mut m.targets {
                                    t.1 *= elem_gain;
                                }
                            }
                            combined_mappings.push(m);
                        }
                    }
                }

                if !combined_substreams.is_empty() {
                    let annot = match (mix_annot.as_deref(), elem_annot_opt.as_deref()) {
                        (Some(ma), Some(ea)) if ma != ea && ea != primary_layout_label => {
                            Some(format!("{} - {}", ma, ea))
                        }
                        (Some(ma), _) if ma != primary_layout_label => Some(ma.to_string()),
                        (_, Some(ea)) if ea != primary_layout_label => Some(ea.to_string()),
                        _ => None,
                    };
                    result.push(IamfTrackGroup {
                        substream_indices: combined_substreams,
                        reconstructed_channels: max_recon_ch,
                        layout_label: primary_layout_label,
                        annotation: annot,
                        language: mix_lang.clone().or(elem_lang_opt),
                        channel_mappings: combined_mappings,
                    });
                }
            }
        }

        // 3. Include any unreferenced IAMF Audio Elements directly
        for (id, elem_group) in audio_elements {
            if !used_element_ids.contains(&id) {
                result.push(elem_group);
            }
        }
    }

    result
}

/// Estimates or scans the duration in seconds of a standalone `.iamf` OBU stream when
/// container header duration is unavailable (`AV_NOPTS_VALUE`).
#[cfg(not(target_os = "android"))]
fn probe_iamf_duration_seconds(file_path: &str) -> Option<f64> {
    let file_len = std::fs::metadata(file_path).ok()?.len();
    if file_len == 0 {
        return None;
    }
    let mut probe_ctx = ffmpeg_next::format::input(&file_path).ok()?;
    let st0 = probe_ctx.stream(0)?;
    let tb = st0.time_base();
    if tb.denominator() <= 0 || tb.numerator() <= 0 {
        return None;
    }
    let tb_f64 = tb.numerator() as f64 / tb.denominator() as f64;

    let mut max_end_pts: i64 = 0;
    let mut first_pos: Option<i64> = None;
    let mut last_pos: i64 = 0;
    let mut packets_seen = 0usize;
    let scan_all = file_len <= 32 * 1024 * 1024;

    for (stream, pkt) in probe_ctx.packets() {
        let pos = pkt.position() as i64;
        if pos > 0 {
            if first_pos.is_none() {
                first_pos = Some(pos);
            }
            if pos > last_pos {
                last_pos = pos;
            }
        }
        if stream.index() == 0 {
            let pts = pkt.pts().or_else(|| pkt.dts()).unwrap_or(0).max(0);
            let end_pts = pts + pkt.duration().max(0);
            if end_pts > max_end_pts {
                max_end_pts = end_pts;
            }
        }
        packets_seen += 1;
        if !scan_all && packets_seen >= 512 {
            break;
        }
    }

    let observed_sec = max_end_pts as f64 * tb_f64;
    if observed_sec <= 0.0 {
        return None;
    }
    if scan_all {
        Some(observed_sec)
    } else if let Some(fp) = first_pos
        && last_pos > fp
    {
        let bytes_span = (last_pos - fp) as f64;
        let total_payload = (file_len as i64 - fp).max(1) as f64;
        Some((observed_sec * total_payload / bytes_span).max(observed_sec))
    } else {
        Some(observed_sec)
    }
}

/// Detects whether a file is a standalone `.iamf` stream or an ISO-BMFF container (`.mp4`, `.m4a`, `.mov`)
/// carrying IAMF / Eclipsa Audio (`iamf` brand or `iacb` configuration box).
pub fn is_iamf_container(file_path: &str) -> bool {
    let lower = file_path.to_lowercase();
    if lower.ends_with(".iamf") {
        return true;
    }
    if !lower.ends_with(".mp4")
        && !lower.ends_with(".m4a")
        && !lower.ends_with(".mov")
        && !lower.ends_with(".m4v")
    {
        return false;
    }
    if let Ok(mut f) = std::fs::File::open(file_path) {
        use std::io::{Read, Seek, SeekFrom};
        let file_len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let head_len = (256 * 1024).min(file_len as usize);
        let mut buf = vec![0u8; head_len];
        if let Ok(n) = f.read(&mut buf) {
            buf.truncate(n);
            if buf.windows(4).any(|w| w == b"iamf" || w == b"iacb") {
                return true;
            }
        }
        if file_len > 256 * 1024 {
            let tail_len = (256 * 1024).min((file_len - 256 * 1024) as usize);
            let mut tail = vec![0u8; tail_len];
            if f.seek(SeekFrom::End(-(tail_len as i64))).is_ok()
                && let Ok(n) = f.read(&mut tail)
            {
                tail.truncate(n);
                if tail.windows(4).any(|w| w == b"iamf" || w == b"iacb") {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(not(target_os = "android"))]
struct ActiveFfmpegTrack {
    track_idx: usize,
    stream_index: usize,
    decoder: ffmpeg_next::decoder::Audio,
    resampler: ffmpeg_next::software::resampling::Context,
    sample_buf: Vec<f32>,
    buf_pos: usize,
    volume: f32,
    enabled: bool,
    channels: u16,
    time_base: f64,
    current_time: f64,
    seek_in_progress: bool,
    substream_bitrate_kbps: Option<u32>,
}

#[cfg(not(target_os = "android"))]
struct FfmpegSource {
    ictx: ffmpeg_next::format::context::Input,
    active_tracks: std::collections::HashMap<usize, ActiveFfmpegTrack>,
    primary_stream_index: usize,
    iamf_groups: Vec<Option<IamfTrackGroup>>,

    channels: u16,
    time_base: f64,
    current_time: f64,
    duration: f64,
    artist: String,
    ext_type: String,
    intrinsic_sample_rate: u32,
    video_info: Option<String>,
    channel_vus: Vec<f32>,
    video_stream_index: Option<usize>,
    video_tx: Option<crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>>,
    video_epoch: u64,
    video_params: Option<ffmpeg_next::codec::Parameters>,
    video_time_base: Option<ffmpeg_next::Rational>,
    video_rotation: u32,
    video_fps: f32,
    video_bitrate_kbps: Option<u32>,

    audio_tracks: Vec<crate::state::AudioTrackInfo>,
    audio_track_bitrates: Vec<Option<u32>>,
    selected_track_idx: usize,
    output_sample_rate: u32,
    is_eof: bool,
    target_seek_time: Option<f64>,
    is_full_mix_guide: bool,
}

#[cfg(not(target_os = "android"))]
fn safe_ffmpeg_channel_layout(
    layout: ffmpeg_next::channel_layout::ChannelLayout,
    channels: u16,
) -> ffmpeg_next::channel_layout::ChannelLayout {
    let ch = channels.max(1) as i32;
    if !layout.is_empty() && layout.channels() == ch {
        layout
    } else {
        ffmpeg_next::channel_layout::ChannelLayout::default(ch)
    }
}

#[cfg(not(target_os = "android"))]
impl FfmpegSource {
    fn fill_buffers(&mut self) -> bool {
        let mut progress = false;
        let target_pos_opt = self.target_seek_time;
        let has_iamf = self.iamf_groups.iter().any(|g| g.is_some());
        let is_2track = self.audio_tracks.len() == 2 && !has_iamf;
        let need_t0 = is_2track && (self.is_full_mix_guide || self.current_time < 3.0);

        // 1. Drain any packets currently in the active decoders
        for track in self.active_tracks.values_mut() {
            if !track.enabled
                && track.stream_index != self.primary_stream_index
                && (!need_t0 || track.track_idx != 0)
            {
                continue;
            }
            let mut decoded = ffmpeg_next::frame::Audio::empty();
            while track.decoder.receive_frame(&mut decoded).is_ok() {
                let dec_ch = decoded.channels().max(1);
                let dec_rate = if decoded.rate() > 0 {
                    decoded.rate()
                } else {
                    self.output_sample_rate
                };
                let dec_layout = safe_ffmpeg_channel_layout(decoded.channel_layout(), dec_ch);
                if (track.channels != dec_ch
                    || track.resampler.input().format != decoded.format()
                    || track.resampler.input().rate != dec_rate
                    || track.resampler.input().channel_layout != dec_layout
                    || track.resampler.output().rate != self.output_sample_rate)
                    && let Ok(new_resamp) = ffmpeg_next::software::resampling::context::Context::get(
                        decoded.format(),
                        dec_layout,
                        dec_rate,
                        ffmpeg_next::format::sample::Sample::F32(
                            ffmpeg_next::format::sample::Type::Packed,
                        ),
                        dec_layout,
                        self.output_sample_rate,
                    )
                {
                    track.resampler = new_resamp;
                    track.channels = dec_ch;
                }
                let mut resampled = ffmpeg_next::frame::Audio::empty();
                if track.resampler.run(&decoded, &mut resampled).is_ok() {
                    let data = resampled.plane::<f32>(0);
                    let actual_len = resampled.samples() * resampled.channels() as usize;
                    let actual_data =
                        unsafe { std::slice::from_raw_parts(data.as_ptr(), actual_len) };
                    if track.buf_pos > 4096 {
                        track.sample_buf.drain(0..track.buf_pos);
                        track.buf_pos = 0;
                    }
                    if let Some(target_pos) = target_pos_opt {
                        let actual_frames = actual_len / track.channels.max(1) as usize;
                        let frame_dur = actual_frames as f64 / self.output_sample_rate as f64;
                        if track.current_time + frame_dur <= target_pos {
                            track.current_time += frame_dur;
                            progress = true;
                        } else if track.current_time < target_pos {
                            let skip_sec = target_pos - track.current_time;
                            let skip_frames =
                                (skip_sec * self.output_sample_rate as f64).round() as usize;
                            let skip_samples =
                                (skip_frames * track.channels as usize).min(actual_len);
                            track
                                .sample_buf
                                .extend_from_slice(&actual_data[skip_samples..]);
                            track.current_time = target_pos;
                            progress = true;
                        } else {
                            track.sample_buf.extend_from_slice(actual_data);
                            progress = true;
                        }
                    } else {
                        track.sample_buf.extend_from_slice(actual_data);
                        progress = true;
                    }
                }
            }
        }

        // 2. Read packets from container if any active track still needs data
        let mut need_packets = false;
        for track in self.active_tracks.values() {
            if !track.enabled
                && track.stream_index != self.primary_stream_index
                && (!need_t0 || track.track_idx != 0)
            {
                continue;
            }
            let avail = (track.sample_buf.len().saturating_sub(track.buf_pos))
                / track.channels.max(1) as usize;
            if avail < 1024 {
                need_packets = true;
                break;
            }
        }

        if need_packets && !self.is_eof {
            let max_packets = if self.target_seek_time.is_some() {
                4096
            } else {
                512
            };
            for _ in 0..max_packets {
                let all_satisfied = self
                    .active_tracks
                    .values()
                    .filter(|t| {
                        t.enabled
                            || t.stream_index == self.primary_stream_index
                            || (need_t0 && t.track_idx == 0)
                    })
                    .all(|t| {
                        let avail = (t.sample_buf.len().saturating_sub(t.buf_pos))
                            / t.channels.max(1) as usize;
                        avail >= 1024
                    });
                if all_satisfied {
                    break;
                }

                if let Some((stream, packet)) = self.ictx.packets().next() {
                    let s_idx = stream.index();
                    if Some(s_idx) == self.video_stream_index {
                        if let Some(tx) = &self.video_tx {
                            let _ = tx.try_send((self.video_epoch, packet.clone()));
                        }
                    } else if let Some(track) = self.active_tracks.get_mut(&s_idx)
                        && (track.enabled
                            || track.stream_index == self.primary_stream_index
                            || (need_t0 && track.track_idx == 0))
                    {
                        if track.seek_in_progress {
                            if let Some(pkt_pts) = packet.pts().or_else(|| packet.dts()) {
                                track.current_time = pkt_pts as f64 * track.time_base;
                            } else {
                                track.current_time = self.current_time;
                            }
                            track.seek_in_progress = false;
                        }
                        let _ = track.decoder.send_packet(&packet);
                        let mut decoded = ffmpeg_next::frame::Audio::empty();
                        while track.decoder.receive_frame(&mut decoded).is_ok() {
                            let dec_ch = decoded.channels().max(1);
                            let dec_rate = if decoded.rate() > 0 {
                                decoded.rate()
                            } else {
                                self.output_sample_rate
                            };
                            if track.substream_bitrate_kbps.is_none() {
                                if track.decoder.bit_rate() > 0 {
                                    track.substream_bitrate_kbps =
                                        Some((track.decoder.bit_rate() / 1000).max(1) as u32);
                                } else if decoded.samples() > 0 && dec_rate > 0 && packet.size() > 0
                                {
                                    let frame_dur = decoded.samples() as f64 / dec_rate as f64;
                                    if frame_dur > 0.0005 {
                                        let kbps = ((packet.size() as f64 * 8.0)
                                            / (frame_dur * 1000.0))
                                            .round()
                                            as u32;
                                        if kbps > 0 {
                                            track.substream_bitrate_kbps = Some(kbps);
                                        }
                                    }
                                }
                            }
                            if !has_iamf
                                && let Some(slot) =
                                    self.audio_track_bitrates.get_mut(track.track_idx)
                                && slot.is_none()
                            {
                                *slot = track.substream_bitrate_kbps;
                            }
                            let dec_layout =
                                safe_ffmpeg_channel_layout(decoded.channel_layout(), dec_ch);
                            if (track.channels != dec_ch
                                || track.resampler.input().format != decoded.format()
                                || track.resampler.input().rate != dec_rate
                                || track.resampler.input().channel_layout != dec_layout
                                || track.resampler.output().rate != self.output_sample_rate)
                                && let Ok(new_resamp) =
                                    ffmpeg_next::software::resampling::context::Context::get(
                                        decoded.format(),
                                        dec_layout,
                                        dec_rate,
                                        ffmpeg_next::format::sample::Sample::F32(
                                            ffmpeg_next::format::sample::Type::Packed,
                                        ),
                                        dec_layout,
                                        self.output_sample_rate,
                                    )
                            {
                                track.resampler = new_resamp;
                                track.channels = dec_ch;
                            }
                            let mut resampled = ffmpeg_next::frame::Audio::empty();
                            if track.resampler.run(&decoded, &mut resampled).is_ok() {
                                let data = resampled.plane::<f32>(0);
                                let actual_len =
                                    resampled.samples() * resampled.channels() as usize;
                                let actual_data = unsafe {
                                    std::slice::from_raw_parts(data.as_ptr(), actual_len)
                                };
                                if track.buf_pos > 4096 {
                                    track.sample_buf.drain(0..track.buf_pos);
                                    track.buf_pos = 0;
                                }
                                if let Some(target_pos) = self.target_seek_time {
                                    let actual_frames = actual_len / track.channels.max(1) as usize;
                                    let frame_dur =
                                        actual_frames as f64 / self.output_sample_rate as f64;
                                    if track.current_time + frame_dur <= target_pos {
                                        track.current_time += frame_dur;
                                        progress = true;
                                    } else if track.current_time < target_pos {
                                        let skip_sec = target_pos - track.current_time;
                                        let skip_frames =
                                            (skip_sec * self.output_sample_rate as f64).round()
                                                as usize;
                                        let skip_samples =
                                            (skip_frames * track.channels as usize).min(actual_len);
                                        track
                                            .sample_buf
                                            .extend_from_slice(&actual_data[skip_samples..]);
                                        track.current_time = target_pos;
                                        progress = true;
                                    } else {
                                        track.sample_buf.extend_from_slice(actual_data);
                                        progress = true;
                                    }
                                } else {
                                    track.sample_buf.extend_from_slice(actual_data);
                                    progress = true;
                                }
                            }
                        }
                    }
                } else {
                    self.is_eof = true;
                    break;
                }
            }
        }

        if has_iamf {
            for (t_idx, grp_opt) in self.iamf_groups.iter().enumerate() {
                if let Some(grp) = grp_opt
                    && let Some(slot) = self.audio_track_bitrates.get_mut(t_idx)
                    && slot.is_none()
                {
                    let mut sum_kbps = 0u32;
                    let mut all_known = true;
                    for &s_idx in &grp.substream_indices {
                        if let Some(st) = self.active_tracks.get(&s_idx)
                            && let Some(br) = st.substream_bitrate_kbps
                        {
                            sum_kbps += br;
                        } else {
                            all_known = false;
                            break;
                        }
                    }
                    if all_known && sum_kbps > 0 {
                        *slot = Some(sum_kbps);
                    }
                }
            }
        } else if let Some(primary) = self.active_tracks.get(&self.primary_stream_index)
            && self.channels != primary.channels
        {
            self.channels = primary.channels;
            if self.channel_vus.len() < self.channels as usize {
                self.channel_vus.resize(self.channels as usize, 0.0);
            }
        }

        if let Some(target_pos) = self.target_seek_time {
            let all_reached = self
                .active_tracks
                .values()
                .filter(|t| t.enabled || t.stream_index == self.primary_stream_index)
                .all(|t| !t.seek_in_progress && t.current_time >= target_pos);
            if all_reached {
                self.target_seek_time = None;
            }
        }

        progress
    }
}

#[cfg(not(target_os = "android"))]
impl AudioSource for FfmpegSource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        if sample_rate > 0 && sample_rate != self.output_sample_rate {
            self.output_sample_rate = sample_rate;
            for track in self.active_tracks.values_mut() {
                let layout =
                    safe_ffmpeg_channel_layout(track.decoder.channel_layout(), track.channels);
                let fmt = if track.decoder.format() == ffmpeg_next::format::sample::Sample::None {
                    track.resampler.input().format
                } else {
                    track.decoder.format()
                };
                let in_rate = if track.decoder.rate() > 0 {
                    track.decoder.rate()
                } else {
                    track.resampler.input().rate
                };
                if let Ok(resamp) = ffmpeg_next::software::resampling::context::Context::get(
                    fmt,
                    layout,
                    in_rate,
                    ffmpeg_next::format::sample::Sample::F32(
                        ffmpeg_next::format::sample::Type::Packed,
                    ),
                    layout,
                    self.output_sample_rate,
                ) {
                    track.resampler = resamp;
                }
            }
        }
        let mut frames_written = 0;
        let frames_needed = output.len() / hardware_channels;
        self.channel_vus.fill(0.0);

        let has_iamf = self.iamf_groups.iter().any(|g| g.is_some());
        let is_2track = self.audio_tracks.len() == 2 && !has_iamf;

        // Dynamic energy-cancellation check for 2-track audio
        if !self.is_full_mix_guide && is_2track {
            let t0_opt = self.active_tracks.values().find(|t| t.track_idx == 0);
            let t1_opt = self.active_tracks.values().find(|t| t.track_idx == 1);
            if let (Some(t0), Some(t1)) = (t0_opt, t1_opt) {
                let ch0 = t0.channels as usize;
                let ch1 = t1.channels as usize;
                let avail0 = (t0.sample_buf.len().saturating_sub(t0.buf_pos)) / ch0.max(1);
                let avail1 = (t1.sample_buf.len().saturating_sub(t1.buf_pos)) / ch1.max(1);
                let check_frames = avail0.min(avail1).min(512);
                if check_frames >= 128 {
                    let mut e0 = 0.0f32;
                    let mut e1 = 0.0f32;
                    let mut ediff = 0.0f32;
                    for i in 0..check_frames {
                        let s0 = t0
                            .sample_buf
                            .get(t0.buf_pos + i * ch0)
                            .copied()
                            .unwrap_or(0.0);
                        let s1 = t1
                            .sample_buf
                            .get(t1.buf_pos + i * ch1)
                            .copied()
                            .unwrap_or(0.0);
                        e0 += s0 * s0;
                        e1 += s1 * s1;
                        ediff += (s1 - s0) * (s1 - s0);
                    }
                    if e0 > 0.001 && e1 > 0.001 && ediff < 0.35 * e1 {
                        self.is_full_mix_guide = true;
                        eprintln!(
                            "[RustTracker Audio] Detected Full Mix vocal guide stem in FFmpegSource - enabling phase cancellation"
                        );
                    }
                }
            }
        }

        let need_t0 = is_2track && (self.is_full_mix_guide || self.current_time < 3.0);

        while frames_written < frames_needed {
            let mut min_available = usize::MAX;
            for track in self.active_tracks.values().filter(|t| {
                t.enabled
                    || t.stream_index == self.primary_stream_index
                    || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.sample_buf.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                min_available = min_available.min(avail);
            }

            if min_available == 0 || min_available == usize::MAX {
                if !self.fill_buffers() {
                    if let Some(primary) = self.active_tracks.get(&self.primary_stream_index) {
                        let avail = (primary.sample_buf.len().saturating_sub(primary.buf_pos))
                            / primary.channels.max(1) as usize;
                        if avail == 0 {
                            if self.is_eof {
                                break;
                            }
                            // Try filling additional batches (especially if skipping video packets during seek)
                            let extra_batches = if self.target_seek_time.is_some() {
                                4
                            } else {
                                1
                            };
                            let mut filled = false;
                            for _ in 0..extra_batches {
                                if self.fill_buffers() || self.is_eof {
                                    filled = !self.is_eof;
                                    break;
                                }
                            }
                            if !filled {
                                break;
                            }
                            continue;
                        }
                        min_available = avail;
                    } else {
                        break;
                    }
                } else {
                    continue;
                }
            }

            let frames_to_copy = min_available.min(frames_needed - frames_written);
            if frames_to_copy == 0 {
                break;
            }

            let headroom = calculate_mix_headroom(
                self.active_tracks
                    .values()
                    .filter(|t| t.enabled)
                    .map(|t| t.volume),
            );

            let is_full_mix = self.is_full_mix_guide && is_2track;
            let (t0_info, t1_info) = if is_full_mix {
                let mut t0 = None;
                let mut t1 = None;
                for t in self.active_tracks.values() {
                    if t.track_idx == 0 {
                        t0 = Some((t.buf_pos, t.channels, t.volume, t.enabled));
                    } else if t.track_idx == 1 {
                        t1 = Some((t.buf_pos, t.channels, t.volume, t.enabled));
                    }
                }
                (t0, t1)
            } else {
                (None, None)
            };

            if is_full_mix && let (Some(t0), Some(t1)) = (t0_info, t1_info) {
                let v0 = if t0.3 { t0.2 } else { 0.0 };
                let v1 = if t1.3 { t1.2 } else { 0.0 };

                let t0_samples = &self
                    .active_tracks
                    .values()
                    .find(|t| t.track_idx == 0)
                    .unwrap()
                    .sample_buf;
                let t1_samples = &self
                    .active_tracks
                    .values()
                    .find(|t| t.track_idx == 1)
                    .unwrap()
                    .sample_buf;

                for f in 0..frames_to_copy {
                    let out_base = (frames_written + f) * hardware_channels;
                    let base_idx_0 = t0.0 + f * t0.1 as usize;
                    let base_idx_1 = t1.0 + f * t1.1 as usize;
                    let ch_slice_0 = if base_idx_0 < t0_samples.len() {
                        &t0_samples[base_idx_0..]
                    } else {
                        &[]
                    };
                    let ch_slice_1 = if base_idx_1 < t1_samples.len() {
                        &t1_samples[base_idx_1..]
                    } else {
                        &[]
                    };

                    for c in 0..hardware_channels {
                        let s0 = get_downmixed_sample(t0.1, ch_slice_0, hardware_channels, c);
                        let s1 = get_downmixed_sample(t1.1, ch_slice_1, hardware_channels, c);
                        // Extracted Vocals V = s1 - s0
                        // Desired output: v0 * I + v1 * V = (v0 - v1) * s0 + v1 * s1
                        let sum_val = (v0 - v1) * s0 + v1 * s1;
                        let final_val = sum_val.clamp(-1.0, 1.0);
                        output[out_base + c] = final_val;

                        if c < self.channel_vus.len() {
                            self.channel_vus[c] = self.channel_vus[c].max(final_val.abs());
                        }
                    }
                }
            } else if let Some(Some(iamf_group)) =
                self.iamf_groups.get(self.selected_track_idx).cloned()
            {
                let recon_ch = (iamf_group.reconstructed_channels as usize).clamp(1, 12);
                let track_vol = self
                    .active_tracks
                    .get(&self.primary_stream_index)
                    .map(|t| if t.enabled { t.volume } else { 1.0 })
                    .unwrap_or(1.0);

                for f in 0..frames_to_copy {
                    let out_idx = (frames_written + f) * hardware_channels;
                    let mut recon = [0.0f32; 12];

                    for mapping in &iamf_group.channel_mappings {
                        if let Some(sub_track) = self.active_tracks.get(&mapping.stream_index) {
                            let in_idx = sub_track.buf_pos + f * sub_track.channels as usize;
                            let sample = sub_track
                                .sample_buf
                                .get(in_idx + mapping.substream_channel)
                                .copied()
                                .unwrap_or(0.0);
                            if sample != 0.0 {
                                for &(dst_ch, weight) in &mapping.targets {
                                    if dst_ch < 12 {
                                        recon[dst_ch] += sample * weight;
                                    }
                                }
                            }
                        }
                    }

                    for (ch, recon_c) in recon.iter_mut().enumerate().take(recon_ch) {
                        *recon_c = (*recon_c * track_vol).clamp(-1.0, 1.0);
                        if ch < self.channel_vus.len() {
                            self.channel_vus[ch] = self.channel_vus[ch].max(recon_c.abs());
                        }
                    }

                    for c in 0..hardware_channels {
                        let s_val = get_downmixed_sample(
                            recon_ch as u16,
                            &recon[..recon_ch],
                            hardware_channels,
                            c,
                        );
                        output[out_idx + c] = s_val.clamp(-1.0, 1.0);
                    }
                }
            } else {
                for f in 0..frames_to_copy {
                    let out_idx = (frames_written + f) * hardware_channels;

                    // Update true channel VUs across all source channels for each active track
                    for track in self.active_tracks.values() {
                        if track.enabled && track.volume > 0.0 {
                            let in_idx = track.buf_pos + f * track.channels as usize;
                            for ch in 0..track.channels as usize {
                                if ch < self.channel_vus.len() {
                                    let s = track
                                        .sample_buf
                                        .get(in_idx + ch)
                                        .copied()
                                        .unwrap_or(0.0)
                                        .abs();
                                    self.channel_vus[ch] =
                                        self.channel_vus[ch].max(s * track.volume);
                                }
                            }
                        }
                    }

                    for c in 0..hardware_channels {
                        let mut sum_val = 0.0f32;
                        for track in self.active_tracks.values() {
                            if track.enabled && track.volume > 0.0 {
                                let in_idx = track.buf_pos + f * track.channels as usize;
                                let ch_slice = if in_idx < track.sample_buf.len() {
                                    &track.sample_buf[in_idx..]
                                } else {
                                    &[]
                                };
                                let s_val = get_downmixed_sample(
                                    track.channels,
                                    ch_slice,
                                    hardware_channels,
                                    c,
                                );
                                sum_val += s_val * track.volume;
                            }
                        }
                        let out_val = (sum_val * headroom).clamp(-1.0, 1.0);
                        output[out_idx + c] = out_val;
                    }
                }
            }

            for track in self.active_tracks.values_mut().filter(|t| {
                t.enabled
                    || t.stream_index == self.primary_stream_index
                    || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.sample_buf.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                let consume = frames_to_copy.min(avail);
                track.buf_pos += consume * track.channels as usize;
                track.current_time += consume as f64 / self.output_sample_rate as f64;
            }
            if let Some(primary) = self.active_tracks.get(&self.primary_stream_index) {
                self.current_time = primary.current_time;
            }
            frames_written += frames_to_copy;
        }

        frames_written
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.current_time
    }

    fn set_position_seconds(&mut self, pos: f64) {
        self.is_eof = false;
        let (stream_idx, pts) =
            if let (Some(v_idx), Some(tb)) = (self.video_stream_index, self.video_time_base) {
                let tb_f64 = tb.numerator() as f64 / tb.denominator() as f64;
                (v_idx as i32, (pos / tb_f64) as i64)
            } else {
                (
                    -1,
                    (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64).max(0.0) as i64,
                )
            };

        let ret = unsafe {
            ffmpeg_next::ffi::av_seek_frame(
                self.ictx.as_mut_ptr(),
                stream_idx,
                pts,
                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
            )
        };
        if ret < 0 && stream_idx != -1 {
            let fallback_pts = (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64).max(0.0) as i64;
            unsafe {
                ffmpeg_next::ffi::av_seek_frame(
                    self.ictx.as_mut_ptr(),
                    -1,
                    fallback_pts,
                    ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                );
            }
        }
        self.target_seek_time = Some(pos);
        for track in self.active_tracks.values_mut() {
            track.decoder.flush();
            track.sample_buf.clear();
            track.buf_pos = 0;
            track.current_time = 0.0;
            track.seek_in_progress = true;
        }
        self.current_time = pos;
        self.video_epoch += 1;
    }

    fn get_num_channels(&mut self) -> i32 {
        self.channels as i32
    }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        self.channel_vus
            .get(channel as usize)
            .cloned()
            .unwrap_or(0.0)
    }
    fn get_artist(&mut self) -> String {
        self.artist.clone()
    }
    fn get_type(&mut self) -> String {
        self.ext_type.clone()
    }
    fn get_tempo(&mut self) -> i32 {
        0
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        Some(self.intrinsic_sample_rate)
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        0
    }
    fn get_num_patterns(&mut self) -> i32 {
        0
    }
    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        0
    }
    fn get_tracker_channels(&mut self) -> Option<i32> {
        None
    }
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        Vec::new()
    }
    fn get_current_row_string(&mut self) -> String {
        String::new()
    }
    fn get_video_info(&mut self) -> Option<String> {
        self.video_info.clone()
    }

    fn get_bitrate(&mut self) -> Option<u32> {
        if let Some(icy_br) = self.ictx.metadata().get("icy-br")
            && let Ok(br) = icy_br.parse::<u32>()
        {
            return Some(br);
        }

        if let Some(Some(track_br)) = self.audio_track_bitrates.get(self.selected_track_idx)
            && *track_br > 0
        {
            return Some(*track_br);
        }

        let br = self.ictx.bit_rate();
        if br > 0 {
            let total_kbps = ((br as f64) / 1000.0).round() as u32;
            if let Some(v_br) = self.video_bitrate_kbps {
                let rem = total_kbps.saturating_sub(v_br);
                if rem > 0 {
                    return Some(rem);
                }
            } else if self.video_stream_index.is_none() {
                return Some(total_kbps);
            }
        }

        None
    }

    fn get_video_fps(&self) -> f32 {
        self.video_fps
    }

    fn get_video_bitrate(&self) -> Option<u32> {
        self.video_bitrate_kbps
    }

    fn attach_video_queue(&mut self, tx: crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>) {
        self.video_tx = Some(tx);
    }

    fn take_video_parameters(
        &mut self,
    ) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational, u32)> {
        if let (Some(p), Some(tb)) = (self.video_params.take(), self.video_time_base) {
            return Some((p, tb, self.video_rotation));
        }
        None
    }

    fn get_audio_tracks(&self) -> Vec<crate::state::AudioTrackInfo> {
        self.audio_tracks.clone()
    }
    fn get_selected_audio_track(&self) -> usize {
        self.selected_track_idx
    }

    fn get_active_audio_tracks(&self) -> Vec<usize> {
        let mut list: Vec<usize> = self
            .active_tracks
            .values()
            .filter(|t| t.enabled)
            .map(|t| t.track_idx)
            .collect();
        list.sort();
        list.dedup();
        if list.is_empty() {
            vec![self.selected_track_idx]
        } else {
            list
        }
    }

    fn set_active_audio_tracks(&mut self, tracks: &[(usize, f32)]) -> Result<()> {
        if tracks.is_empty() {
            return Ok(());
        }

        if self.iamf_groups.iter().any(|g| g.is_some()) {
            for track in self.active_tracks.values_mut() {
                track.enabled = false;
                track.volume = 0.0;
            }
            for &(t_idx, vol) in tracks {
                if let Some(Some(grp)) = self.iamf_groups.get(t_idx) {
                    for &s_idx in &grp.substream_indices {
                        if let Some(st) = self.active_tracks.get_mut(&s_idx) {
                            st.enabled = true;
                            st.volume = vol;
                            st.track_idx = t_idx;
                        }
                    }
                } else {
                    for track in self.active_tracks.values_mut() {
                        if track.track_idx == t_idx {
                            track.enabled = true;
                            track.volume = vol;
                        }
                    }
                }
            }
        } else {
            let track_map: std::collections::HashMap<usize, f32> = tracks.iter().copied().collect();
            for track in self.active_tracks.values_mut() {
                if let Some(&vol) = track_map.get(&track.track_idx) {
                    track.enabled = true;
                    track.volume = vol;
                } else {
                    track.enabled = false;
                    track.volume = 0.0;
                }
            }
        }

        if let Some(&(first_idx, _)) = tracks.first()
            && first_idx < self.audio_tracks.len()
        {
            self.selected_track_idx = first_idx;
            self.primary_stream_index = self.audio_tracks[first_idx].id;
            self.channels = self.audio_tracks[first_idx].channels;
            self.time_base = self
                .ictx
                .stream(self.primary_stream_index)
                .map(|s| {
                    let tb = s.time_base();
                    tb.numerator() as f64 / tb.denominator() as f64
                })
                .unwrap_or(1.0 / 44100.0);
            let container = self.ext_type.split('/').nth(1).map(|s| s.to_string());
            if let Some(container) = container {
                self.ext_type = format!("{}/{}", self.audio_tracks[first_idx].codec, container);
            } else {
                self.ext_type = self.audio_tracks[first_idx].codec.clone();
            }
            self.channel_vus = vec![0.0; self.channels as usize];
        }
        Ok(())
    }

    fn select_audio_track(&mut self, track_idx: usize) -> Result<()> {
        self.set_active_audio_tracks(&[(track_idx, 1.0)])
    }

    fn has_video_stream(&self) -> bool {
        self.video_stream_index.is_some()
    }
}

// ---------------------------------------------------------
// Video Only Source (No Audio)
// ---------------------------------------------------------
#[cfg(not(target_os = "android"))]
struct VideoOnlySource {
    ictx: ffmpeg_next::format::context::Input,
    video_stream_index: usize,
    video_tx: Option<crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>>,
    video_epoch: u64,
    video_params: Option<ffmpeg_next::codec::Parameters>,
    video_time_base: Option<ffmpeg_next::Rational>,
    video_rotation: u32,
    current_time: f64,
    last_video_pts: f64,
    duration: f64,
    video_info: Option<String>,
    video_fps: f32,
    video_bitrate_kbps: Option<u32>,
    ext_type: String,
}

#[cfg(target_os = "android")]
struct AndroidVideoOnlySource {
    current_time: f64,
    duration: f64,
}

#[cfg(target_os = "android")]
impl AudioSource for AndroidVideoOnlySource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        let frames_to_write = output.len() / hardware_channels;
        for v in output.iter_mut() {
            *v = 0.0;
        }
        self.current_time += frames_to_write as f64 / sample_rate as f64;
        frames_to_write
    }
    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.current_time
    }
    fn set_position_seconds(&mut self, pos: f64) {
        self.current_time = pos;
    }
    fn get_num_channels(&mut self) -> i32 {
        2
    }
    fn get_current_channel_vu_mono(&mut self, _channel: i32) -> f32 {
        0.0
    }
    fn get_artist(&mut self) -> String {
        "Video Stream".to_string()
    }
    fn get_type(&mut self) -> String {
        "Video".to_string()
    }
    fn get_tempo(&mut self) -> i32 {
        0
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        Some(48000)
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        0
    }
    fn get_num_patterns(&mut self) -> i32 {
        0
    }
    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        0
    }
    fn has_video_stream(&self) -> bool {
        true
    }
}

#[cfg(not(target_os = "android"))]
impl AudioSource for VideoOnlySource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        let frames_to_write = output.len() / hardware_channels;
        for v in output.iter_mut() {
            *v = 0.0;
        }

        let tb_f64 = self
            .video_time_base
            .map(|tb| tb.numerator() as f64 / tb.denominator().max(1) as f64)
            .unwrap_or(1.0 / 30.0);
        let target_pts = self.current_time + (frames_to_write as f64 / sample_rate as f64) + 0.25;
        let mut packets_read = 0;
        while packets_read < 16 && self.last_video_pts <= target_pts {
            if let Some((stream, packet)) = self.ictx.packets().next() {
                if stream.index() == self.video_stream_index {
                    if let Some(ts) = packet.dts().or_else(|| packet.pts()) {
                        let pkt_sec = ts as f64 * tb_f64;
                        if pkt_sec > self.last_video_pts {
                            self.last_video_pts = pkt_sec;
                        }
                    } else {
                        self.last_video_pts += 1.0 / 30.0;
                    }
                    if let Some(tx) = &self.video_tx {
                        let _ = tx.try_send((self.video_epoch, packet.clone()));
                    }
                    packets_read += 1;
                }
            } else {
                break;
            }
        }

        self.current_time += frames_to_write as f64 / sample_rate as f64;
        frames_to_write
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.current_time
    }

    fn set_position_seconds(&mut self, pos: f64) {
        let (stream_idx, pts) = if let Some(tb) = self.video_time_base {
            let tb_f64 = tb.numerator() as f64 / tb.denominator() as f64;
            (self.video_stream_index as i32, (pos / tb_f64) as i64)
        } else {
            (-1, (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64)
        };

        unsafe {
            ffmpeg_next::ffi::av_seek_frame(
                self.ictx.as_mut_ptr(),
                stream_idx,
                pts,
                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
            );
        }
        self.current_time = pos;
        self.last_video_pts = pos;
        self.video_epoch += 1;
    }

    fn get_num_channels(&mut self) -> i32 {
        2
    }
    fn get_current_channel_vu_mono(&mut self, _channel: i32) -> f32 {
        0.0
    }
    fn get_artist(&mut self) -> String {
        "Video Only".to_string()
    }
    fn get_type(&mut self) -> String {
        self.ext_type.clone()
    }
    fn get_tempo(&mut self) -> i32 {
        0
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        Some(44100)
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        0
    }
    fn get_num_patterns(&mut self) -> i32 {
        0
    }
    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        0
    }
    fn get_video_info(&mut self) -> Option<String> {
        self.video_info.clone()
    }
    fn get_video_fps(&self) -> f32 {
        self.video_fps
    }
    fn get_video_bitrate(&self) -> Option<u32> {
        self.video_bitrate_kbps
    }

    fn attach_video_queue(&mut self, tx: crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>) {
        self.video_tx = Some(tx);
    }

    fn take_video_parameters(
        &mut self,
    ) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational, u32)> {
        if let (Some(p), Some(tb)) = (self.video_params.take(), self.video_time_base) {
            Some((p, tb, self.video_rotation))
        } else {
            None
        }
    }

    fn has_video_stream(&self) -> bool {
        true
    }
}

#[cfg(not(target_os = "android"))]
#[derive(Clone)]
pub struct VideoStreamSetup {
    pub stream_index: usize,
    pub params: ffmpeg_next::codec::Parameters,
    pub time_base: ffmpeg_next::Rational,
    pub rotation: u32,
    pub info: String,
    pub fps: f32,
    pub bitrate_kbps: Option<u32>,
}

#[cfg(not(target_os = "android"))]
pub fn probe_stream_bitrate_kbps(stream: &ffmpeg_next::format::stream::Stream) -> Option<u32> {
    let raw_br = unsafe { (*stream.parameters().as_ptr()).bit_rate };
    if raw_br > 0 {
        return Some((raw_br / 1000).max(1) as u32);
    }
    let meta = stream.metadata();
    for (k, v) in meta.iter() {
        let ku = k.to_uppercase();
        if (ku == "BPS" || ku.starts_with("BPS-") || ku == "BIT_RATE" || ku == "VARIANT_BITRATE")
            && let Ok(bps) = v.trim().parse::<u64>()
            && bps > 0
        {
            return Some((bps / 1000).max(1) as u32);
        }
    }
    None
}

#[cfg(not(target_os = "android"))]
pub fn probe_video_stream(ictx: &ffmpeg_next::format::context::Input) -> Option<VideoStreamSetup> {
    let v_stream = ictx.streams().best(ffmpeg_next::media::Type::Video)?;
    let stream_index = v_stream.index();
    let params = v_stream.parameters();
    let time_base = v_stream.time_base();
    let mut rotation = 0u32;

    let avg_fr = v_stream.avg_frame_rate();
    let r_fr = v_stream.rate();
    let fps = if avg_fr.denominator() > 0 && avg_fr.numerator() > 0 {
        let f = avg_fr.numerator() as f32 / avg_fr.denominator() as f32;
        if (1.0..=1000.0).contains(&f) {
            f
        } else if r_fr.denominator() > 0 && r_fr.numerator() > 0 {
            (r_fr.numerator() as f32 / r_fr.denominator() as f32).clamp(0.0, 1000.0)
        } else {
            0.0
        }
    } else if r_fr.denominator() > 0 && r_fr.numerator() > 0 {
        (r_fr.numerator() as f32 / r_fr.denominator() as f32).clamp(0.0, 1000.0)
    } else {
        0.0
    };

    let bitrate_kbps = probe_stream_bitrate_kbps(&v_stream);

    // 1. Check metadata dictionary for "rotate" tag
    if let Some(rot_str) = v_stream.metadata().get("rotate")
        && let Ok(rot_deg) = rot_str.parse::<i32>()
    {
        let norm = ((rot_deg % 360) + 360) % 360;
        if norm == 90 || norm == 180 || norm == 270 {
            rotation = norm as u32;
        }
    }
    // 2. Check stream side data for DisplayMatrix
    if rotation == 0 {
        for sd in v_stream.side_data() {
            if sd.kind() == ffmpeg_next::codec::packet::side_data::Type::DisplayMatrix {
                let data = sd.data();
                if data.len() >= 36 {
                    let m0 = i32::from_ne_bytes([data[0], data[1], data[2], data[3]]);
                    let m1 = i32::from_ne_bytes([data[4], data[5], data[6], data[7]]);
                    let a = m0 as f64 / 65536.0;
                    let b = m1 as f64 / 65536.0;
                    let rot_deg = (-b.atan2(a).to_degrees()).round() as i32;
                    let norm = ((rot_deg % 360) + 360) % 360;
                    if norm == 90 || norm == 180 || norm == 270 {
                        rotation = norm as u32;
                    }
                }
            }
        }
    }

    let info =
        if let Ok(v_ctx) = ffmpeg_next::codec::context::Context::from_parameters(params.clone()) {
            if let Ok(v_dec) = v_ctx.decoder().video() {
                let (disp_w, disp_h) = if rotation == 90 || rotation == 270 {
                    (v_dec.height(), v_dec.width())
                } else {
                    (v_dec.width(), v_dec.height())
                };
                format!(
                    "{} ({}x{})",
                    v_dec
                        .codec()
                        .map(|c| c.name().to_string())
                        .unwrap_or("H264".to_string())
                        .to_uppercase(),
                    disp_w,
                    disp_h
                )
            } else {
                "Unsupported Codec".to_string()
            }
        } else {
            "Unsupported Codec".to_string()
        };

    Some(VideoStreamSetup {
        stream_index,
        params,
        time_base,
        rotation,
        info,
        fps,
        bitrate_kbps,
    })
}

#[cfg(not(target_os = "android"))]
fn try_ffmpeg(file_path: &str, is_network: bool) -> Result<Box<dyn AudioSource>> {
    let _ = ffmpeg_next::init();
    let mut dict = ffmpeg_next::Dictionary::new();
    dict.set("probesize", "5000000");
    dict.set("analyzeduration", "5000000");
    let ictx = ffmpeg_next::format::input_with_dictionary(&file_path, dict)
        .context("Failed to open file via libavformat")?;

    let mut duration = ictx.duration() as f64 / ffmpeg_next::ffi::AV_TIME_BASE as f64;
    if is_network || duration < 0.0 {
        duration = 0.0;
    }

    if duration <= 0.0 {
        let mut max_stream_duration = 0.0;
        for stream in ictx.streams() {
            let tb = stream.time_base();
            let sd = stream.duration() as f64 * (tb.numerator() as f64 / tb.denominator() as f64);
            if sd > max_stream_duration {
                max_stream_duration = sd;
            }
        }
        if max_stream_duration > 0.0 {
            duration = max_stream_duration;
        }
    }

    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("FFMPEG")
        .to_uppercase();

    let detected_iamf_groups = probe_iamf_stream_groups(&ictx);
    if duration <= 0.0
        && !is_network
        && (!detected_iamf_groups.is_empty() || ext == "IAMF")
        && let Some(probed_dur) = probe_iamf_duration_seconds(file_path)
    {
        duration = probed_dur;
    }

    let mut video_info = None;
    let mut video_stream_index = None;
    let mut video_params = None;
    let mut video_tb = None;
    let mut video_rotation = 0u32;
    let mut video_fps = 0.0f32;
    let mut video_bitrate_kbps = None;
    if let Some(vs) = probe_video_stream(&ictx) {
        video_stream_index = Some(vs.stream_index);
        video_params = Some(vs.params);
        video_tb = Some(vs.time_base);
        video_rotation = vs.rotation;
        video_info = Some(vs.info);
        video_fps = vs.fps;
        video_bitrate_kbps = vs.bitrate_kbps;
    }

    let mut audio_tracks = Vec::new();
    let mut audio_track_bitrates: Vec<Option<u32>> = Vec::new();
    let mut iamf_groups: Vec<Option<IamfTrackGroup>> = Vec::new();
    let mut selected_track_idx = 0;

    let mp4_track_names = if file_path.to_lowercase().ends_with(".mp4")
        || file_path.to_lowercase().ends_with(".m4a")
        || file_path.to_lowercase().ends_with(".mov")
    {
        extract_mp4_track_names(file_path)
    } else {
        Vec::new()
    };

    let mut iamf_substream_set = std::collections::HashSet::new();
    for grp in &detected_iamf_groups {
        for &s_idx in &grp.substream_indices {
            iamf_substream_set.insert(s_idx);
        }
    }

    for grp in detected_iamf_groups {
        let primary_s_idx = grp.substream_indices[0];
        let mut sub_codec_name = "Opus".to_string();
        let mut rate = 48000u32;
        let mut lang = None;
        let mut total_br_kbps = 0u32;
        let mut any_br = false;

        for (i, &s_idx) in grp.substream_indices.iter().enumerate() {
            if let Some(stream) = ictx.stream(s_idx) {
                let p = stream.parameters();
                if i == 0 {
                    let codec_id = p.id();
                    sub_codec_name = match codec_id {
                        ffmpeg_next::codec::Id::OPUS => "Opus".to_string(),
                        ffmpeg_next::codec::Id::FLAC => "FLAC".to_string(),
                        ffmpeg_next::codec::Id::AAC => "AAC".to_string(),
                        ffmpeg_next::codec::Id::PCM_S16LE
                        | ffmpeg_next::codec::Id::PCM_S24LE
                        | ffmpeg_next::codec::Id::PCM_S32LE
                        | ffmpeg_next::codec::Id::PCM_F32LE => "PCM".to_string(),
                        _ => format!("{:?}", codec_id).to_uppercase(),
                    };
                    let meta = stream.metadata();
                    lang = meta
                        .get("language")
                        .or_else(|| meta.get("LANG"))
                        .map(|s| s.to_string())
                        .filter(|l| l.to_lowercase() != "und");
                }
                let mut sub_br = probe_stream_bitrate_kbps(&stream);
                if let Ok(ctx) = ffmpeg_next::codec::context::Context::from_parameters(p)
                    && let Ok(dec) = ctx.decoder().audio()
                {
                    if i == 0 && dec.rate() > 0 {
                        rate = dec.rate();
                    }
                    if sub_br.is_none() && dec.bit_rate() > 0 {
                        sub_br = Some((dec.bit_rate() / 1000).max(1) as u32);
                    }
                }
                if let Some(br) = sub_br {
                    total_br_kbps += br;
                    any_br = true;
                }
            }
        }

        let codec_name = format!("IAMF ({})", sub_codec_name);
        let channels = grp.reconstructed_channels;
        let ch_desc = if !grp.layout_label.is_empty() {
            grp.layout_label.clone()
        } else {
            match channels {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                4 => "4.0".to_string(),
                6 => "5.1".to_string(),
                8 => "7.1".to_string(),
                10 => "7.1.2".to_string(),
                12 => "7.1.4".to_string(),
                n => format!("{} ch", n),
            }
        };

        let lang = grp.language.clone().or(lang);
        let title = grp
            .annotation
            .clone()
            .or_else(|| mp4_track_names.get(audio_tracks.len()).cloned());

        let mut desc_parts = Vec::new();
        desc_parts.push(codec_name.clone());
        desc_parts.push(ch_desc);
        if rate > 0 {
            desc_parts.push(format!("{}Hz", rate));
        }

        let technical = desc_parts.join(" ");
        let display_title = match (lang.as_deref(), title.as_deref()) {
            (Some(l), Some(t)) => format!(
                "Track {}: {} [{} - {}]",
                audio_tracks.len() + 1,
                technical,
                l,
                t
            ),
            (Some(l), None) => {
                format!("Track {}: {} [{}]", audio_tracks.len() + 1, technical, l)
            }
            (None, Some(t)) => {
                format!("Track {}: {} [{}]", audio_tracks.len() + 1, technical, t)
            }
            (None, None) => format!("Track {}: {}", audio_tracks.len() + 1, technical),
        };

        audio_tracks.push(crate::state::AudioTrackInfo {
            id: primary_s_idx,
            title: display_title,
            codec: codec_name,
            channels,
            sample_rate: rate,
            language: lang,
        });
        audio_track_bitrates.push(if any_br { Some(total_br_kbps) } else { None });
        iamf_groups.push(Some(grp));
    }

    for stream in ictx.streams() {
        if stream.parameters().medium() == ffmpeg_next::media::Type::Audio {
            let s_idx = stream.index();
            if iamf_substream_set.contains(&s_idx) {
                continue;
            }
            let p = stream.parameters();
            let codec_id = p.id();
            let codec_name = match codec_id {
                ffmpeg_next::codec::Id::TRUEHD => "TrueHD".to_string(),
                ffmpeg_next::codec::Id::EAC3 => "E-AC3".to_string(),
                ffmpeg_next::codec::Id::AC3 => "AC3".to_string(),
                ffmpeg_next::codec::Id::DTS => "DTS".to_string(),
                ffmpeg_next::codec::Id::FLAC => "FLAC".to_string(),
                ffmpeg_next::codec::Id::AAC => "AAC".to_string(),
                ffmpeg_next::codec::Id::MP3 => "MP3".to_string(),
                ffmpeg_next::codec::Id::VORBIS => "Vorbis".to_string(),
                ffmpeg_next::codec::Id::OPUS => "Opus".to_string(),
                ffmpeg_next::codec::Id::PCM_S16LE
                | ffmpeg_next::codec::Id::PCM_S24LE
                | ffmpeg_next::codec::Id::PCM_S32LE
                | ffmpeg_next::codec::Id::PCM_F32LE => "PCM".to_string(),
                _ => format!("{:?}", codec_id).to_uppercase(),
            };

            let mut track_br = probe_stream_bitrate_kbps(&stream);
            let (channels, rate) = if let Ok(ctx) =
                ffmpeg_next::codec::context::Context::from_parameters(p.clone())
                && let Ok(dec) = ctx.decoder().audio()
            {
                if track_br.is_none() && dec.bit_rate() > 0 {
                    track_br = Some((dec.bit_rate() / 1000).max(1) as u32);
                }
                (
                    dec.channels().max(1),
                    if dec.rate() > 0 { dec.rate() } else { 48000 },
                )
            } else {
                (2, 48000)
            };

            let ch_desc = match channels {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                6 => "5.1".to_string(),
                8 => "7.1".to_string(),
                10 => "7.1.2".to_string(),
                12 => "7.1.4".to_string(),
                n => format!("{} ch", n),
            };

            let meta = stream.metadata();
            let lang = meta
                .get("language")
                .or_else(|| meta.get("LANG"))
                .map(|s| s.to_string())
                .filter(|l| l.to_lowercase() != "und");
            let title = meta
                .get("name")
                .or_else(|| meta.get("NAME"))
                .or_else(|| meta.get("title"))
                .or_else(|| meta.get("TITLE"))
                .or_else(|| meta.get("description"))
                .map(|s| s.to_string())
                .or_else(|| mp4_track_names.get(audio_tracks.len()).cloned());

            let mut desc_parts = Vec::new();
            desc_parts.push(codec_name.clone());
            desc_parts.push(ch_desc);
            if rate > 0 {
                desc_parts.push(format!("{}Hz", rate));
            }

            let technical = desc_parts.join(" ");
            let display_title = match (lang.as_deref(), title.as_deref()) {
                (Some(l), Some(t)) => format!(
                    "Track {}: {} [{} - {}]",
                    audio_tracks.len() + 1,
                    technical,
                    l,
                    t
                ),
                (Some(l), None) => {
                    format!("Track {}: {} [{}]", audio_tracks.len() + 1, technical, l)
                }
                (None, Some(t)) => {
                    format!("Track {}: {} [{}]", audio_tracks.len() + 1, technical, t)
                }
                (None, None) => format!("Track {}: {}", audio_tracks.len() + 1, technical),
            };

            audio_tracks.push(crate::state::AudioTrackInfo {
                id: s_idx,
                title: display_title,
                codec: codec_name,
                channels,
                sample_rate: rate,
                language: lang,
            });
            audio_track_bitrates.push(track_br);
            iamf_groups.push(None);
        }
    }

    let container_kbps = if ictx.bit_rate() > 0 {
        Some((ictx.bit_rate() / 1000).max(1) as u32)
    } else if !is_network
        && duration > 0.0
        && let Ok(meta) = std::fs::metadata(file_path)
    {
        let kbps = ((meta.len() as f64 * 8.0) / (duration * 1000.0)).round() as u32;
        if kbps > 0 { Some(kbps) } else { None }
    } else {
        None
    };

    if video_bitrate_kbps.is_none()
        && video_stream_index.is_some()
        && let Some(c_kbps) = container_kbps
    {
        let audio_sum: u32 = audio_track_bitrates.iter().flatten().copied().sum();
        if c_kbps > audio_sum {
            video_bitrate_kbps = Some(c_kbps - audio_sum);
        } else {
            video_bitrate_kbps = Some(c_kbps);
        }
    }

    if let Some(c_kbps) = container_kbps {
        if video_stream_index.is_none()
            && audio_track_bitrates.len() == 1
            && audio_track_bitrates[0].is_none()
        {
            audio_track_bitrates[0] = Some(c_kbps);
        } else if let Some(v_kbps) = video_bitrate_kbps
            && audio_track_bitrates.len() == 1
            && audio_track_bitrates[0].is_none()
            && c_kbps > v_kbps
        {
            audio_track_bitrates[0] = Some(c_kbps - v_kbps);
        }
    }

    if audio_tracks.is_empty() {
        if let Some(v_idx) = video_stream_index {
            return Ok(Box::new(VideoOnlySource {
                ictx,
                video_stream_index: v_idx,
                video_tx: None,
                video_epoch: 0,
                video_params,
                video_time_base: video_tb,
                video_rotation,
                current_time: 0.0,
                last_video_pts: 0.0,
                duration,
                video_info,
                video_fps,
                video_bitrate_kbps,
                ext_type: ext,
            }));
        } else {
            return Err(anyhow::anyhow!(
                "No audio stream found and no video stream found"
            ));
        }
    }

    let best_audio_stream = ictx.streams().best(ffmpeg_next::media::Type::Audio);
    let stream_index = if iamf_groups.iter().any(|g| g.is_some()) {
        selected_track_idx = 0;
        audio_tracks[0].id
    } else if let Some(ref best) = best_audio_stream {
        if let Some(pos) = audio_tracks.iter().position(|t| t.id == best.index()) {
            selected_track_idx = pos;
        }
        best.index()
    } else {
        audio_tracks[0].id
    };

    let stream = ictx
        .stream(stream_index)
        .context("Audio stream not found")?;
    let channels = audio_tracks
        .get(selected_track_idx)
        .map(|t| t.channels)
        .unwrap_or(2);
    let sample_rate = audio_tracks
        .get(selected_track_idx)
        .map(|t| t.sample_rate)
        .unwrap_or(48000);
    let time_base = stream.time_base();
    let tb = time_base.numerator() as f64 / time_base.denominator() as f64;

    let initial_artist = ictx
        .metadata()
        .get("artist")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let mut active_tracks = std::collections::HashMap::new();
    for (t_idx, track_info) in audio_tracks.iter().enumerate() {
        let stream_ids: Vec<usize> = if let Some(Some(grp)) = iamf_groups.get(t_idx) {
            grp.substream_indices.clone()
        } else {
            vec![track_info.id]
        };

        for st_id in stream_ids {
            if active_tracks.contains_key(&st_id) {
                continue;
            }
            if let Some(st) = ictx.stream(st_id)
                && let Ok(ctx) =
                    ffmpeg_next::codec::context::Context::from_parameters(st.parameters())
                && let Ok(dec) = ctx.decoder().audio()
            {
                let mut sub_br = probe_stream_bitrate_kbps(&st);
                if sub_br.is_none() && dec.bit_rate() > 0 {
                    sub_br = Some((dec.bit_rate() / 1000).max(1) as u32);
                }
                let ch = dec.channels().max(1);
                let layout = safe_ffmpeg_channel_layout(dec.channel_layout(), ch);
                let fmt = if dec.format() == ffmpeg_next::format::sample::Sample::None {
                    ffmpeg_next::format::sample::Sample::F32(
                        ffmpeg_next::format::sample::Type::Planar,
                    )
                } else {
                    dec.format()
                };
                let in_rate = if dec.rate() > 0 {
                    dec.rate()
                } else {
                    sample_rate.max(8000)
                };
                if let Ok(resamp) = ffmpeg_next::software::resampling::context::Context::get(
                    fmt,
                    layout,
                    in_rate,
                    ffmpeg_next::format::sample::Sample::F32(
                        ffmpeg_next::format::sample::Type::Packed,
                    ),
                    layout,
                    sample_rate.max(8000),
                ) {
                    let vol = if t_idx == selected_track_idx {
                        1.0
                    } else {
                        0.0
                    };
                    let enabled = t_idx == selected_track_idx;
                    let track_st_tb = st.time_base();
                    let track_tb_f64 =
                        track_st_tb.numerator() as f64 / track_st_tb.denominator() as f64;
                    active_tracks.insert(
                        st_id,
                        ActiveFfmpegTrack {
                            track_idx: t_idx,
                            stream_index: st_id,
                            decoder: dec,
                            resampler: resamp,
                            sample_buf: Vec::new(),
                            buf_pos: 0,
                            volume: vol,
                            enabled,
                            channels: ch,
                            time_base: track_tb_f64,
                            current_time: 0.0,
                            seek_in_progress: video_stream_index.is_some(),
                            substream_bitrate_kbps: sub_br,
                        },
                    );
                }
            }
        }
    }

    if !active_tracks.contains_key(&stream_index) {
        return Err(anyhow::anyhow!(
            "Failed to initialize FFmpeg audio decoder for primary stream"
        ));
    }

    let is_full_mix_guide = if audio_tracks.len() == 2 {
        let t1_title = audio_tracks[1].title.to_lowercase();
        t1_title.contains("full mix") || t1_title.contains("fullmix")
    } else {
        false
    };

    Ok(Box::new(FfmpegSource {
        ictx,
        active_tracks,
        primary_stream_index: stream_index,
        channels,
        time_base: tb,
        current_time: 0.0,
        duration,
        artist: initial_artist,
        ext_type: audio_tracks
            .get(selected_track_idx)
            .map(|t| t.codec.clone())
            .unwrap_or(ext),
        intrinsic_sample_rate: sample_rate,
        video_info,
        video_fps,
        video_bitrate_kbps,
        audio_track_bitrates,
        channel_vus: vec![0.0; channels as usize],
        video_stream_index,
        video_tx: None,
        video_epoch: 0,
        video_params,
        video_time_base: video_tb,
        video_rotation,
        audio_tracks,
        selected_track_idx,
        output_sample_rate: sample_rate,
        is_eof: false,
        target_seek_time: None,
        is_full_mix_guide,
        iamf_groups,
    }))
}

/// Helper to extract human-readable track names (e.g. "Instrumental (Karaoke)", "Vocals (Guide)")
/// from ISO-BMFF / MP4 containers when present in `udta.name` or `name` atoms.
pub fn extract_mp4_track_names(path: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(mut file) = std::fs::File::open(path) {
        use std::io::{Read, Seek, SeekFrom};
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut buffers = Vec::new();

        // 1. First 2MB (for faststart/web-optimized MP4s where moov is at the start)
        let mut head = vec![0u8; (2 * 1024 * 1024).min(file_len as usize)];
        if let Ok(n) = file.read(&mut head) {
            head.truncate(n);
            buffers.push(head);
        }

        // 2. If file > 2MB, also read last 4MB where moov is placed in standard MP4s
        if file_len > 2 * 1024 * 1024 {
            let tail_len = (4 * 1024 * 1024).min(file_len - 2 * 1024 * 1024) as usize;
            let tail_offset = file_len - tail_len as u64;
            if file.seek(SeekFrom::Start(tail_offset)).is_ok() {
                let mut tail = vec![0u8; tail_len];
                if let Ok(n) = file.read(&mut tail) {
                    tail.truncate(n);
                    buffers.push(tail);
                }
            }
        }

        let name_marker = b"name";
        for data in &buffers {
            let mut i = 0;
            while i + 8 < data.len() {
                if &data[i..i + 4] == name_marker {
                    let start = i + 4;
                    let mut end = start;
                    while end < data.len()
                        && end < start + 64
                        && data[end] >= 0x20
                        && data[end] <= 0x7E
                    {
                        end += 1;
                    }
                    if end > start + 2
                        && let Ok(name_str) = std::str::from_utf8(&data[start..end])
                    {
                        let trimmed = name_str.trim();
                        if !trimmed.is_empty() && !names.contains(&trimmed.to_string()) {
                            names.push(trimmed.to_string());
                        }
                    }
                }
                i += 1;
            }
        }
    }
    names
}

pub fn try_symphonia<R: symphonia::core::io::MediaSource + 'static>(
    file: R,
    probe_ext: &str,
    display_ext: &str,
    video_info: Option<String>,
    file_path: Option<&str>,
) -> Result<Box<dyn AudioSource>> {
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(probe_ext);

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .context("Unsupported audio format")?;

    let format = probed.format;

    let mp4_track_names = if let Some(p) = file_path {
        if p.to_lowercase().ends_with(".mp4")
            || p.to_lowercase().ends_with(".m4a")
            || p.to_lowercase().ends_with(".mov")
        {
            extract_mp4_track_names(p)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let has_video = format
        .tracks()
        .iter()
        .any(|t| t.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_NULL)
        || probe_ext == "mp4"
        || probe_ext == "mkv"
        || probe_ext == "webm"
        || probe_ext == "mov"
        || probe_ext == "avi";

    let mut audio_tracks = Vec::new();
    let mut selected_track = None;
    let mut selected_track_idx = 0;
    let mut audio_track_count = 0;

    for track in format.tracks().iter() {
        let is_false_mp3 = track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_MP3
            && !probe_ext.is_empty()
            && !matches!(probe_ext, "mp3" | "mp2" | "mp1" | "mpga" | "mpeg" | "wav");
        if !is_false_mp3
            && track.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL
            && symphonia::default::get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())
                .is_ok()
        {
            let codec_name = match track.codec_params.codec {
                symphonia::core::codecs::CODEC_TYPE_AAC => "AAC".to_string(),
                symphonia::core::codecs::CODEC_TYPE_MP3 => "MP3".to_string(),
                symphonia::core::codecs::CODEC_TYPE_FLAC => "FLAC".to_string(),
                symphonia::core::codecs::CODEC_TYPE_VORBIS => "Vorbis".to_string(),
                symphonia::core::codecs::CODEC_TYPE_OPUS => "Opus".to_string(),
                symphonia::core::codecs::CODEC_TYPE_ALAC => "ALAC".to_string(),
                _ => format!("{:?}", track.codec_params.codec),
            };
            let channels = if let Some(ch) = track.codec_params.channels {
                ch.count() as u16
            } else if let Some(layout) = track.codec_params.channel_layout {
                layout.into_channels().count() as u16
            } else {
                2
            };
            let rate = track.codec_params.sample_rate.unwrap_or(44100);
            let ch_desc = match channels {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                6 => "5.1".to_string(),
                8 => "7.1".to_string(),
                n => format!("{} ch", n),
            };

            let track_name_opt = mp4_track_names.get(audio_track_count).cloned();
            audio_track_count += 1;

            let title = if let Some(ref name) = track_name_opt {
                format!(
                    "Track {}: {} [{} {}]",
                    audio_tracks.len() + 1,
                    name,
                    codec_name,
                    ch_desc
                )
            } else if let Some(ref lang) = track.language {
                format!(
                    "Track {}: {} [{}] {} {}Hz",
                    audio_tracks.len() + 1,
                    codec_name,
                    lang,
                    ch_desc,
                    rate
                )
            } else {
                format!(
                    "Track {}: {} {} {}Hz",
                    audio_tracks.len() + 1,
                    codec_name,
                    ch_desc,
                    rate
                )
            };

            if selected_track.is_none() {
                selected_track = Some(track.clone());
                selected_track_idx = audio_tracks.len();
            }

            audio_tracks.push(crate::state::AudioTrackInfo {
                id: track.id as usize,
                title,
                codec: codec_name,
                channels,
                sample_rate: rate,
                language: track.language.clone(),
            });
        }
    }

    let track = selected_track.context("No supported audio track found")?;
    let track_id = track.id;
    let channels = if let Some(ch) = track.codec_params.channels {
        ch.count() as u16
    } else if let Some(layout) = track.codec_params.channel_layout {
        layout.into_channels().count() as u16
    } else {
        2
    };
    let intrinsic_sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let time_base = track
        .codec_params
        .time_base
        .map(|t| t.calc_time(1).seconds as f64 + t.calc_time(1).frac)
        .unwrap_or(1.0 / 44100.0);
    let duration = track
        .codec_params
        .n_frames
        .map(|n| n as f64 * time_base)
        .unwrap_or(0.0);

    let mut active_tracks = std::collections::HashMap::new();
    for (t_idx, track_info) in audio_tracks.iter().enumerate() {
        if let Some(t) = format
            .tracks()
            .iter()
            .find(|t| t.id == track_info.id as u32)
            && let Ok(dec) =
                symphonia::default::get_codecs().make(&t.codec_params, &DecoderOptions::default())
        {
            let t_sym_channels = t
                .codec_params
                .channels
                .or_else(|| t.codec_params.channel_layout.map(|l| l.into_channels()))
                .unwrap_or(
                    symphonia::core::audio::Channels::FRONT_LEFT
                        | symphonia::core::audio::Channels::FRONT_RIGHT,
                );
            let t_channels = t_sym_channels.count() as u16;
            let t_sample_rate = t.codec_params.sample_rate.unwrap_or(44100);
            let t_time_base = t
                .codec_params
                .time_base
                .map(|tb| tb.calc_time(1).seconds as f64 + tb.calc_time(1).frac)
                .unwrap_or(1.0 / 44100.0);
            let vol = if t_idx == selected_track_idx {
                1.0
            } else {
                0.0
            };
            let enabled = t_idx == selected_track_idx;
            active_tracks.insert(
                t.id,
                ActiveSymphoniaTrack {
                    track_idx: t_idx,
                    track_id: t.id,
                    decoder: dec,
                    sample_buf: SampleBuffer::<f32>::new(
                        0,
                        symphonia::core::audio::SignalSpec::new(t_sample_rate, t_sym_channels),
                    ),
                    samples: Vec::new(),
                    buf_pos: 0,
                    volume: vol,
                    enabled,
                    channels: t_channels,
                    time_base: t_time_base,
                    current_time: 0.0,
                },
            );
        }
    }

    let bitrate = if let Some(path) = file_path
        && let Ok(meta) = std::fs::metadata(path)
        && duration > 0.0
    {
        let bytes = meta.len();
        let kbps = ((bytes as f64 * 8.0) / (duration * 1000.0)).round() as u32;
        if kbps > 0 { Some(kbps) } else { None }
    } else {
        None
    };

    let ext_type = if display_ext.eq_ignore_ascii_case("mp4")
        || display_ext.eq_ignore_ascii_case("m4a")
        || display_ext.eq_ignore_ascii_case("mov")
    {
        let first_codec = audio_tracks
            .first()
            .map(|t| t.codec.as_str())
            .unwrap_or("AAC");
        format!("{}/{}", first_codec, display_ext.to_uppercase())
    } else if display_ext.is_empty() {
        "UNKNOWN".to_string()
    } else {
        display_ext.to_uppercase()
    };

    let is_full_mix_guide = if audio_tracks.len() == 2 {
        let t1_title = audio_tracks[1].title.to_lowercase();
        t1_title.contains("full mix") || t1_title.contains("fullmix")
    } else {
        false
    };

    Ok(Box::new(SymphoniaSource {
        format,
        active_tracks,
        primary_track_id: track_id,
        time_base,
        current_time: 0.0,
        duration,
        channels,
        channel_vus: vec![0.0; channels as usize],
        artist: "Unknown".to_string(),
        ext_type,
        intrinsic_sample_rate: Some(intrinsic_sample_rate),
        video_info,
        audio_tracks,
        selected_track_idx,
        has_video,
        bitrate,
        is_full_mix_guide,
    }))
}

pub fn load_audio_source(file_path: &str) -> Result<Box<dyn AudioSource>> {
    #[cfg(not(target_os = "android"))]
    {
        unsafe {
            ffmpeg_next::ffi::av_log_set_level(ffmpeg_next::ffi::AV_LOG_FATAL);
        }
        ffmpeg_next::format::network::init();
    }

    if file_path.starts_with("http") {
        let mut actual_path = file_path.to_string();
        if let Ok(mut resp) = ureq::get(&actual_path).call() {
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_lowercase();
            if content_type.contains("audio/x-scpls") || actual_path.ends_with(".pls") {
                let mut body = String::new();
                if resp
                    .body_mut()
                    .as_reader()
                    .read_to_string(&mut body)
                    .is_ok()
                {
                    for line in body.lines() {
                        if line.starts_with("File1=") {
                            actual_path = line.trim_start_matches("File1=").to_string();
                            break;
                        }
                    }
                }
            } else if content_type.contains("mpegurl")
                || actual_path.ends_with(".m3u")
                || actual_path.ends_with(".m3u8")
            {
                let mut body = String::new();
                if resp
                    .body_mut()
                    .as_reader()
                    .read_to_string(&mut body)
                    .is_ok()
                {
                    for line in body.lines() {
                        if !line.starts_with('#') && line.trim().starts_with("http") {
                            actual_path = line.trim().to_string();
                            break;
                        }
                    }
                }
            }
        }

        #[cfg(not(target_os = "android"))]
        if let Ok(source) = try_ffmpeg(&actual_path, true) {
            return Ok(source);
        }

        if let Ok(resp) = ureq::get(&actual_path).call() {
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_lowercase();
            let probe_ext = if content_type.contains("mpeg") || content_type.contains("mp3") {
                "mp3"
            } else if content_type.contains("aac") {
                "aac"
            } else if content_type.contains("ogg")
                || content_type.contains("vorbis")
                || content_type.contains("opus")
            {
                "ogg"
            } else if content_type.contains("flac") {
                "flac"
            } else if content_type.contains("wav") {
                "wav"
            } else {
                std::path::Path::new(actual_path.split('?').next().unwrap_or(&actual_path))
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("mp3")
            };
            let display_ext = probe_ext.to_uppercase();

            struct SyncStreamReader<R>(std::sync::Mutex<R>);
            impl<R: std::io::Read> std::io::Read for SyncStreamReader<R> {
                fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                    self.0.get_mut().unwrap().read(buf)
                }
            }

            let reader = SyncStreamReader(std::sync::Mutex::new(resp.into_body().into_reader()));
            let ro_source = symphonia::core::io::ReadOnlySource::new(reader);
            if let Ok(source) = try_symphonia(ro_source, probe_ext, &display_ext, None, None) {
                return Ok(source);
            }
        }

        return Err(anyhow::anyhow!("Failed to open network stream"));
    }

    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let video_info = None; // Symphonia doesn't do video

    // 1. Prioritize FFmpeg for video containers (MKV, MP4, WEBM, MOV, AVI) and raw surround bitstreams (IAMF, DTS-HD, DTS, AC3, E-AC3, TrueHD) on desktop.
    #[cfg(not(target_os = "android"))]
    if matches!(
        ext.as_str(),
        "mkv"
            | "mp4"
            | "m4v"
            | "webm"
            | "mov"
            | "avi"
            | "flv"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "ogv"
            | "3gp"
            | "mka"
            | "m2ts"
            | "ts"
            | "mts"
            | "iamf"
            | "dtshd"
            | "dts"
            | "ac3"
            | "eac3"
            | "thd"
            | "truehd"
            | "mlp"
    ) || (ext == "m4a" && is_iamf_container(file_path))
    {
        match try_ffmpeg(file_path, false) {
            Ok(source) => return Ok(source),
            Err(err) => {
                eprintln!("FFmpeg load failed for {}: {:?}", file_path, err);
            }
        }
    }

    // 2. Try OpenMPT Tracker module FIRST for tracker module files
    let is_tracker_ext = matches!(
        ext.as_str(),
        "mod"
            | "s3m"
            | "xm"
            | "it"
            | "mptm"
            | "stm"
            | "med"
            | "okt"
            | "669"
            | "mtm"
            | "far"
            | "ult"
            | "amf"
            | "dsm"
            | "psm"
            | "umx"
            | "mo3"
            | "ptm"
            | "mdl"
            | "ams"
            | "dbm"
            | "digi"
            | "dmf"
            | "dtm"
            | "gdm"
            | "imf"
            | "j2b"
            | "m15"
            | "nst"
            | "plm"
            | "sfx"
            | "stk"
            | "wow"
    );
    if is_tracker_ext && let Ok(mut file) = File::open(file_path) {
        let mut data = Vec::new();
        if file.read_to_end(&mut data).is_ok() {
            let mut module_cursor = Cursor::new(data);
            if let Ok(module) = Module::create(&mut module_cursor, Logger::None, &[]) {
                return Ok(Box::new(OpenMptSource {
                    module: SafeModule(module),
                    left_buf: Vec::with_capacity(8192),
                    right_buf: Vec::with_capacity(8192),
                }));
            }
        }
    }

    // 3. Try MIDI FIRST for MIDI files
    if ext == "mid" || ext == "midi" {
        // Look for soundfont in project dir, then fallback to executable-relative paths, system-wide paths, and macOS Resources bundle
        let mut sf_path = "assets/soundfont.sf2".to_string();
        if !std::path::Path::new(&sf_path).exists() {
            if std::path::Path::new("soundfont.sf2").exists() {
                sf_path = "soundfont.sf2".to_string();
            } else {
                let mut found = false;
                if let Ok(exe_path) = std::env::current_exe()
                    && let Some(exe_dir) = exe_path.parent()
                {
                    let test_paths = vec![
                        exe_dir.join("assets/soundfont.sf2"),
                        exe_dir.join("soundfont.sf2"),
                        exe_dir.join("../share/rusttracker/assets/soundfont.sf2"),
                        exe_dir.join("../share/rusttracker/soundfont.sf2"),
                        exe_dir.join("../Resources/soundfont.sf2"),
                        exe_dir.join("../Resources/assets/soundfont.sf2"),
                    ];
                    for tp in test_paths {
                        if tp.exists() {
                            sf_path = tp.to_string_lossy().into_owned();
                            found = true;
                            break;
                        }
                    }
                }

                // Fallback to system-wide paths on Linux/macOS/Android/BSD if not found yet
                if !found {
                    let system_paths = vec![
                        "/data/user/0/com.rusttracker.app/files/soundfont.sf2",
                        "/data/user/0/com.rusttracker.app/cache/soundfont.sf2",
                        "/data/data/com.rusttracker.app/files/soundfont.sf2",
                        "/data/data/com.rusttracker.app/cache/soundfont.sf2",
                        "/Library/Audio/Sounds/Banks/default.sf2",
                        "/opt/homebrew/share/soundfonts/default.sf2",
                        "/usr/local/share/soundfonts/default.sf2",
                        "/usr/share/sounds/sf2/FluidR3_GM.sf2",
                        "/usr/share/sounds/sf2/default.sf2",
                        "/usr/share/soundfonts/FluidR3_GM.sf2",
                        "/usr/share/soundfonts/default.sf2",
                        "/usr/share/soundfonts/FluidR3_GM.sf3",
                        "/usr/share/soundfonts/freepats-general-midi.sf2",
                    ];
                    for sp in system_paths {
                        if std::path::Path::new(sp).exists() {
                            sf_path = sp.to_string();
                            break;
                        }
                    }
                }
            }
        }
        if let Ok(source) = MidiSource::new(file_path, &sf_path, 48000) {
            return Ok(Box::new(source));
        } else {
            return Err(anyhow::anyhow!(
                "Failed to parse MIDI or missing SoundFont ({}). Please place a SoundFont in assets/soundfont.sf2 or bundled resources",
                sf_path
            ));
        }
    }

    // 4. Try Symphonia for supported audio and container formats (FLAC, MP3, WAV, AAC, M4A, OGG, OPUS, etc.)
    let mut symphonia_error = None;
    if let Ok(file) = File::open(file_path) {
        match try_symphonia(file, &ext, &ext, video_info.clone(), Some(file_path)) {
            Ok(source) => return Ok(source),
            Err(e) => {
                println!(
                    "[RustTracker] Symphonia load failed for {}: {:?}",
                    file_path, e
                );
                symphonia_error = Some(e);
            }
        }
    }

    // 4b. Fallback: Try OpenMPT if extension was missing or non-standard (e.g. Amiga prefix "mod.song" or cloud URI without extension)
    if !is_tracker_ext
        && let Ok(mut file) = File::open(file_path)
        && let Ok(meta) = file.metadata()
        && meta.len() > 0
        && meta.len() <= 64 * 1024 * 1024
    {
        let mut data = Vec::new();
        if file.read_to_end(&mut data).is_ok() {
            let mut module_cursor = Cursor::new(data);
            if let Ok(module) = Module::create(&mut module_cursor, Logger::None, &[]) {
                return Ok(Box::new(OpenMptSource {
                    module: SafeModule(module),
                    left_buf: Vec::with_capacity(8192),
                    right_buf: Vec::with_capacity(8192),
                }));
            }
        }
    }

    // 5. Try FFmpeg native bindings on desktop as universal fallback
    #[cfg(not(target_os = "android"))]
    {
        let ffmpeg_result = try_ffmpeg(file_path, false);
        if let Ok(source) = ffmpeg_result {
            return Ok(source);
        } else if let Err(ref err) = ffmpeg_result {
            if let Some(se) = symphonia_error {
                eprintln!(
                    "[RustTracker] Symphonia load failed for {}: {:?}",
                    file_path, se
                );
            }
            eprintln!("FFmpeg fallback load failed for {}: {:?}", file_path, err);
        }

        // If all failed, return the ffmpeg error since it's the most descriptive for standard media
        ffmpeg_result
    }

    // 6. Fallback for Video-Only containers on Android (e.g. camera videos or silent MP4 clips)
    #[cfg(target_os = "android")]
    {
        if ext == "mp4"
            || ext == "mkv"
            || ext == "mov"
            || ext == "webm"
            || ext == "3gp"
            || ext == "m4v"
            || ext == "avi"
            || ext == "ts"
        {
            return Ok(Box::new(AndroidVideoOnlySource {
                current_time: 0.0,
                duration: 0.0,
            }));
        }
        if let Some(se) = symphonia_error {
            crate::android::log_android(
                5,
                &format!(
                    "[RustTracker] Symphonia load failed for {}: {:?}",
                    file_path, se
                ),
            );
            return Err(anyhow::anyhow!(
                "Unsupported or unreadable audio format ({:?}): {}",
                se,
                file_path
            ));
        }
        Err(anyhow::anyhow!(
            "Unsupported or unreadable audio format: {}",
            file_path
        ))
    }
}

pub fn start_audio_thread(
    file_path: &str,
    mic: bool,
    shared_state: Arc<Mutex<AppState>>,
) -> Result<PlaybackHandle> {
    #[allow(unused_mut)]
    let mut preloaded_source: Option<Box<dyn AudioSource>> = None;
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if !mic {
        let ext = std::path::Path::new(file_path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let can_bitstream = matches!(
            ext.as_str(),
            "mkv"
                | "mp4"
                | "m2ts"
                | "ts"
                | "mts"
                | "mov"
                | "avi"
                | "mka"
                | "dtshd"
                | "dts"
                | "ac3"
                | "eac3"
                | "thd"
                | "truehd"
                | "mlp"
        );
        let (passthrough, surround_upmix_enabled, force_stereo) = {
            let s = shared_state.lock().unwrap();
            (
                s.passthrough_enabled,
                s.surround_upmix.enabled,
                s.force_stereo_downmix,
            )
        };
        let should_try_bitstream = if passthrough
            && can_bitstream
            && !surround_upmix_enabled
            && !force_stereo
        {
            true
        } else if can_bitstream {
            match load_audio_source(file_path) {
                Ok(src) => {
                    preloaded_source = Some(src);
                    false
                }
                Err(e) => {
                    eprintln!(
                        "[audio] Shared-mode software decode failed ({}), falling back to exclusive bitstream passthrough...",
                        e
                    );
                    true
                }
            }
        } else {
            false
        };
        if should_try_bitstream {
            {
                let mut state = shared_state.lock().unwrap();
                state.current_seconds = 0.0;
                state.lookahead_queue.clear();
                state.lookahead_sample_buffer.clear();
                state.lookahead_buffer_start_time = 0.0;
                state.lookahead_timeline = vec![0.0; 600 * 8];
                state.waveform_history.clear();
                state.raw_waveform.fill(0.0);
                state.track_ended = false;
            }
            let (tx, rx) = bounded::<DspMessage>(32);
            let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
            if let Ok((handle, decoder_rate, channels, codec_name, has_video, artist, duration)) =
                crate::bitstream::start_bitstream_thread(
                    file_path,
                    shared_state.clone(),
                    tx.clone(),
                    stop_token.clone(),
                )
            {
                let max_frequency = shared_state.lock().unwrap().max_frequency;
                let sample_rate = decoder_rate;
                let window_size = calculate_power_of_two_window_size(sample_rate);

                let is_lpcm = codec_name == "flac"
                    || codec_name == "pcm"
                    || codec_name == "alac"
                    || codec_name == "multichannel_pcm";
                let display_name = match codec_name.as_str() {
                    "truehd" => "TrueHD / Dolby Atmos",
                    "eac3" => "E-AC3 / Dolby Digital Plus",
                    "dts" => "DTS",
                    "ac3" => "AC3 / Dolby Digital",
                    "flac" => "FLAC Multi-Channel LPCM",
                    "pcm" => "Multi-Channel LPCM",
                    "alac" => "ALAC Multi-Channel LPCM",
                    _ => &codec_name,
                }
                .to_string();

                let video_suffix = if has_video {
                    " (Video available: 'v' to view)"
                } else {
                    ""
                };

                let default_artist = if is_lpcm {
                    "WASAPI Exclusive LPCM"
                } else {
                    "Bitstream Active"
                };
                let meta_artist = if !artist.is_empty() {
                    artist
                } else {
                    default_artist.to_string()
                };
                let meta_duration = duration;

                {
                    let mut state = shared_state.lock().unwrap();
                    state.artist = meta_artist;
                    state.module_type = if is_lpcm {
                        "Multi-Channel LPCM".to_string()
                    } else {
                        "Hardware Passthrough".to_string()
                    };
                    state.stats.bitstream_active = true;
                    state.stats.mpegh_bitstream_active = false;
                    state.stats.mpegh_carrier_info.clear();
                    state.is_dummy_audio_output = false;
                    state.current_sample_rate = sample_rate as f32;
                    state.duration_seconds = meta_duration;
                    let ch = (channels as i32).max(2);
                    state.num_channels = ch;
                    state.hardware_channels = ch;
                    state.channel_vus = vec![0.0; ch as usize];
                    state.peak_vus = vec![0.0; ch as usize];
                    state.has_video_stream = has_video;
                    state.video_info = Some(format!("{}{}", display_name, video_suffix));
                }

                spawn_dsp_thread(
                    rx,
                    shared_state.clone(),
                    sample_rate,
                    max_frequency,
                    window_size,
                );
                return Ok(PlaybackHandle::Bitstream(Some(handle), stop_token));
            }
        }
    }

    {
        let mut state = shared_state.lock().unwrap();
        state.stats.bitstream_active = false;
        state.stats.mpegh_bitstream_active = false;
        state.stats.mpegh_carrier_info.clear();
        state.is_dummy_audio_output = false;
        state.stats.audio_buffer_fill_pct = 0.0;
    }

    let host = cpal::default_host();
    let mut audio_source_opt = if mic {
        None
    } else if let Some(src) = preloaded_source.take() {
        Some(src)
    } else {
        Some(load_audio_source(file_path)?)
    };
    let rate = audio_source_opt
        .as_mut()
        .and_then(|a| a.get_intrinsic_sample_rate())
        .unwrap_or(48000);
    let target_rate: cpal::SampleRate = rate;
    #[allow(unused_variables)]
    let (
        force_stereo,
        surround_upmix_enabled,
        surround_upmix_user_overridden,
        passthrough_enabled,
        mpegh_bitstream,
        mpegh_user_overridden,
    ) = {
        let s = shared_state.lock().unwrap();
        (
            s.force_stereo_downmix,
            s.surround_upmix.enabled,
            s.surround_upmix.user_overridden,
            s.passthrough_enabled,
            s.surround_upmix.mpegh_bitstream,
            s.surround_upmix.mpegh_user_overridden,
        )
    };
    let source_channels = audio_source_opt
        .as_mut()
        .map(|a| a.get_num_channels() as u16)
        .unwrap_or(2);
    let target_channels = if force_stereo { 2 } else { source_channels };

    if mic {
        let selected_device_name = {
            let state = shared_state.lock().unwrap();
            state.selected_audio_device.clone()
        };
        let mut dev = None;
        if !is_default_audio_device_selection(selected_device_name.as_deref())
            && let Some(ref name) = selected_device_name
            && let Ok(devices) = host.input_devices()
        {
            for d in devices {
                if let Ok(desc) = d.description()
                    && cpal_device_matches_name(&desc, name)
                {
                    dev = Some(d);
                    break;
                }
            }
        }
        let device = if let Some(d) = dev {
            d
        } else {
            host.default_input_device()
                .context("No input device available")?
        };

        let supported_config = {
            let supported_configs_range = device
                .supported_input_configs()
                .context("error while querying input configs")?;
            let mut input_configs: Vec<_> = supported_configs_range
                .filter(|c| {
                    c.max_sample_rate() >= target_rate
                        && c.min_sample_rate() <= target_rate
                        && (c.sample_format() == cpal::SampleFormat::F32
                            || c.sample_format() == cpal::SampleFormat::I16)
                })
                .collect();
            input_configs
                .sort_by_key(|c| std::cmp::Reverse(c.sample_format() == cpal::SampleFormat::F32));
            input_configs
                .into_iter()
                .next()
                .map(|c| c.with_sample_rate(target_rate))
                .or_else(|| device.default_input_config().ok())
                .context("No supported config?!")?
        };

        let config: cpal::StreamConfig = supported_config.clone().into();
        let (tx, rx) = bounded::<DspMessage>(32);

        {
            let mut state = shared_state.lock().unwrap();
            state.artist = "Microphone".to_string();
            state.module_type = "Hardware Input".to_string();
            state.duration_seconds = 0.0;
            state.num_channels = config.channels as i32;
            state.channel_vus = vec![0.0; state.num_channels as usize];
            state.bpm = 0;
            state.speed = 0;
            state.max_frequency = config.sample_rate as f32 / 2.0;
            state.current_sample_rate = config.sample_rate as f32;
        }

        let max_frequency = { shared_state.lock().unwrap().max_frequency };
        let window_size = calculate_power_of_two_window_size(config.sample_rate);
        spawn_dsp_thread(
            rx,
            shared_state.clone(),
            config.sample_rate,
            max_frequency,
            window_size,
        );

        let stream = match supported_config.sample_format() {
            cpal::SampleFormat::F32 => {
                run_mic::<f32>(&device, &config, shared_state, tx, config.sample_rate)
            }
            cpal::SampleFormat::I16 => {
                run_mic::<i16>(&device, &config, shared_state, tx, config.sample_rate)
            }
            cpal::SampleFormat::U16 => {
                run_mic::<u16>(&device, &config, shared_state, tx, config.sample_rate)
            }
            _ => return Err(anyhow::anyhow!("Unsupported sample format")),
        }?;
        return Ok(PlaybackHandle::Cpal(
            stream,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        ));
    }

    let shared_state_closure = shared_state.clone();
    let cpal_stream_result = (|| -> Result<PlaybackHandle> {
        let selected_device_name = {
            let state = shared_state_closure.lock().unwrap();
            state.selected_audio_device.clone()
        };

        let mut devices_to_try = Vec::new();
        let push_unique_device = |list: &mut Vec<cpal::Device>, dev: cpal::Device| {
            let d_name = dev
                .description()
                .map(|desc| format_cpal_device_name(&desc))
                .unwrap_or_else(|_| "Unknown Device".to_string());
            let already_present = list.iter().any(|existing| {
                existing
                    .description()
                    .map(|desc| format_cpal_device_name(&desc) == d_name)
                    .unwrap_or(false)
            });
            if !already_present {
                list.push(dev);
            }
        };

        if !is_default_audio_device_selection(selected_device_name.as_deref())
            && let Some(ref name) = selected_device_name
        {
            let mut is_default = false;

            // ALSA WORKAROUND: If the user selected the default device by name, ensure we use the REAL default host handle.
            // Matching by name in the output_devices() iterator on Linux can inadvertently pick a raw hardware endpoint
            // that bypasses PulseAudio/PipeWire and fails with ALSA `unable to open slave` lock errors.
            if let Some(default_dev) = host.default_output_device()
                && let Ok(desc) = default_dev.description()
                && cpal_device_matches_name(&desc, name)
            {
                push_unique_device(&mut devices_to_try, default_dev);
                is_default = true;
            }

            if !is_default && let Ok(devices) = host.output_devices() {
                for d in devices {
                    if let Ok(desc) = d.description()
                        && cpal_device_matches_name(&desc, name)
                    {
                        push_unique_device(&mut devices_to_try, d);
                        break;
                    }
                }
            }
        }

        if let Some(default_dev) = host.default_output_device() {
            let is_relevant = default_dev
                .description()
                .map(|desc| is_relevant_audio_device(&format_cpal_device_name(&desc)))
                .unwrap_or(true);
            if is_relevant {
                push_unique_device(&mut devices_to_try, default_dev);
            }
        }
        if let Ok(devices) = host.output_devices() {
            for d in devices {
                let is_relevant = d
                    .description()
                    .map(|desc| is_relevant_audio_device(&format_cpal_device_name(&desc)))
                    .unwrap_or(true);
                if is_relevant {
                    push_unique_device(&mut devices_to_try, d);
                }
            }
        }

        if devices_to_try.is_empty() {
            return Err(anyhow::anyhow!("No audio output devices found."));
        }

        let mut last_err = anyhow::anyhow!("No candidate audio device succeeded");
        for (dev_idx, device) in devices_to_try.into_iter().enumerate() {
            let device_name = device
                .description()
                .map(|desc| format_cpal_device_name(&desc))
                .unwrap_or_else(|_| "Unknown Device".to_string());

            // Reuse initial audio source for the first attempt, or reload if retrying on fallback device
            let mut attempt_audio_source_opt = if mic {
                None
            } else if let Some(src) = audio_source_opt.take() {
                Some(src)
            } else if let Some(src) = preloaded_source.take() {
                Some(src)
            } else {
                Some(load_audio_source(file_path)?)
            };

            let result = (|| -> Result<PlaybackHandle> {
                #[allow(unused_mut)]
                let mut default_device_channels = device
                    .default_output_config()
                    .ok()
                    .map(|c| c.channels())
                    .unwrap_or(2);
                #[cfg(target_os = "linux")]
                {
                    let is_default_cand = dev_idx == 0
                        && is_default_audio_device_selection(selected_device_name.as_deref());
                    let d_lower = device_name.to_lowercase();
                    if (is_default_cand
                        || d_lower == "default"
                        || d_lower.contains("pipewire")
                        || d_lower.contains("pulseaudio"))
                        && let Some(topo) = query_active_audio_sink_topology(false)
                    {
                        default_device_channels = topo.channels.max(1);
                        if let Ok(mut state) = shared_state_closure.lock() {
                            state.active_audio_sink_id = Some(topo.sink_id);
                            state.active_audio_sink_description = Some(topo.description);
                            state.active_audio_sink_channels = topo.channels;
                        }
                    }
                }
                let effective_surround_upmix = if surround_upmix_user_overridden {
                    surround_upmix_enabled
                } else {
                    !force_stereo
                        && !passthrough_enabled
                        && crate::state::SurroundUpmixConfig::should_enable_by_default(
                            source_channels as i32,
                            default_device_channels as i32,
                        )
                };

                // On Windows HDMI/DisplayPort/AVR endpoints where the Windows Shared Mode default
                // mix format is Stereo (default_device_channels <= 2), cpal only exposes
                // `GetMixFormat()` and AudioDg.exe downmixes any Shared Mode stream to 2.0 stereo.
                // Probe and open WASAPI Exclusive Mode multi-channel LPCM (up to 7.1 / 7.1.4) or
                // IEC 61937-13 MPEG-H 3D Audio 7.1.4 (11.1) directly on the endpoint so the HDMI
                // Audio InfoFrame switches to multi-channel surround.
                #[cfg(windows)]
                if !mic && !force_stereo {
                    let want_wasapi_upmix = if surround_upmix_user_overridden {
                        surround_upmix_enabled
                    } else {
                        true
                    };
                    let wasapi_target_dev = if dev_idx == 0
                        && is_default_audio_device_selection(selected_device_name.as_deref())
                    {
                        None
                    } else {
                        Some(device_name.as_str())
                    };
                    match run_wasapi_exclusive_multichannel(
                        wasapi_target_dev,
                        &mut attempt_audio_source_opt,
                        shared_state_closure.clone(),
                        target_rate,
                        target_channels,
                        want_wasapi_upmix,
                        mpegh_bitstream,
                        mpegh_user_overridden,
                        file_path,
                    ) {
                        Ok(handle) => {
                            if dev_idx > 0
                                && !is_default_audio_device_selection(
                                    selected_device_name.as_deref(),
                                )
                                && let Ok(mut state) = shared_state_closure.lock()
                            {
                                eprintln!(
                                    "[audio] Switched to available fallback audio device: '{}'",
                                    device_name
                                );
                                state.selected_audio_device = Some(device_name.clone());
                            }
                            return Ok(handle);
                        }
                        Err(e) => {
                            eprintln!(
                                "[audio] WASAPI Exclusive Multichannel probe on '{}': {:?} (falling back to CPAL)",
                                device_name, e
                            );
                        }
                    }
                }

                #[cfg(target_os = "linux")]
                if !mic
                    && !force_stereo
                    && mpegh_bitstream
                    && (effective_surround_upmix || target_channels >= 9)
                    && (mpegh_user_overridden
                        || (default_device_channels > 2
                            && default_device_channels < 12
                            && device_name.to_lowercase().contains("hdmi")))
                {
                    match run_linux_pipewire_mpegh_714(
                        &mut attempt_audio_source_opt,
                        shared_state_closure.clone(),
                        file_path,
                    ) {
                        Ok(handle) => {
                            return Ok(handle);
                        }
                        Err(e) => {
                            eprintln!(
                                "[audio] Linux PipeWire MPEG-H 3D Audio 7.1.4 probe on '{}': {:?} (falling back to CPAL)",
                                device_name, e
                            );
                        }
                    }
                }

                // Only request >2 output channels for surround upmix when the OS audio endpoint
                // is actually configured for multi-channel surround (>2ch). On standard stereo/mono
                // speakers (default_device_channels <= 2), surround upmix is bypassed completely
                // and hardware_channels stays at the native stereo/mono channel count.
                let upmix_to_multichannel_hw =
                    effective_surround_upmix && default_device_channels > 2;
                let device_target_channels = resolve_device_target_channels(
                    force_stereo,
                    default_device_channels,
                    target_channels,
                );

                let mut candidate_configs: Vec<cpal::SupportedStreamConfig> = Vec::new();
                if let Ok(supported_configs_range) = device.supported_output_configs() {
                    let mut configs: Vec<_> = supported_configs_range
                        .filter(|c| {
                            c.sample_format() == cpal::SampleFormat::F32
                                || c.sample_format() == cpal::SampleFormat::I16
                        })
                        .collect();

                    configs.sort_by_key(|c| {
                        rank_output_config_key(
                            c.channels(),
                            c.min_sample_rate(),
                            c.max_sample_rate(),
                            c.sample_format(),
                            device_target_channels,
                            target_rate,
                            upmix_to_multichannel_hw,
                            force_stereo,
                        )
                    });

                    for c in configs {
                        let rate = if c.min_sample_rate() <= target_rate
                            && c.max_sample_rate() >= target_rate
                        {
                            target_rate
                        } else {
                            c.max_sample_rate()
                        };
                        let resolved = c.with_sample_rate(rate);
                        if !candidate_configs.iter().any(|existing| {
                            existing.channels() == resolved.channels()
                                && existing.sample_rate() == resolved.sample_rate()
                                && existing.sample_format() == resolved.sample_format()
                        }) {
                            candidate_configs.push(resolved);
                            if candidate_configs.len() >= 4 {
                                break;
                            }
                        }
                    }
                }

                if let Ok(default_cfg) = device.default_output_config()
                    && matches!(
                        default_cfg.sample_format(),
                        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
                    )
                    && !candidate_configs.iter().any(|existing| {
                        existing.channels() == default_cfg.channels()
                            && existing.sample_rate() == default_cfg.sample_rate()
                            && existing.sample_format() == default_cfg.sample_format()
                    })
                {
                    candidate_configs.push(default_cfg);
                }

                if candidate_configs.is_empty() {
                    return Err(anyhow::anyhow!("No supported output config on device"));
                }

                let mut last_cfg_err = anyhow::anyhow!("No output config succeeded on device");
                for supported_config in candidate_configs {
                    let config: cpal::StreamConfig = supported_config.clone().into();
                    let (tx, rx) = bounded::<DspMessage>(32);
                    let window_size = calculate_power_of_two_window_size(config.sample_rate);

                    let stream_res = match supported_config.sample_format() {
                        cpal::SampleFormat::F32 => run::<f32>(
                            &device,
                            &config,
                            &mut attempt_audio_source_opt,
                            shared_state_closure.clone(),
                            tx,
                            config.sample_rate,
                            0.0,
                            window_size,
                            file_path,
                        ),
                        cpal::SampleFormat::I16 => run::<i16>(
                            &device,
                            &config,
                            &mut attempt_audio_source_opt,
                            shared_state_closure.clone(),
                            tx,
                            config.sample_rate,
                            0.0,
                            window_size,
                            file_path,
                        ),
                        cpal::SampleFormat::U16 => run::<u16>(
                            &device,
                            &config,
                            &mut attempt_audio_source_opt,
                            shared_state_closure.clone(),
                            tx,
                            config.sample_rate,
                            0.0,
                            window_size,
                            file_path,
                        ),
                        _ => Err(anyhow::anyhow!("Unsupported sample format")),
                    };

                    match stream_res {
                        Ok((stream, stop_token)) => {
                            let max_frequency =
                                { shared_state_closure.lock().unwrap().max_frequency };
                            spawn_dsp_thread(
                                rx,
                                shared_state_closure.clone(),
                                config.sample_rate,
                                max_frequency,
                                window_size,
                            );
                            if dev_idx > 0
                                && !is_default_audio_device_selection(
                                    selected_device_name.as_deref(),
                                )
                                && let Ok(mut state) = shared_state_closure.lock()
                            {
                                eprintln!(
                                    "[audio] Switched to available fallback audio device: '{}'",
                                    device_name
                                );
                                state.selected_audio_device = Some(device_name.clone());
                            }
                            return Ok(PlaybackHandle::Cpal(stream, stop_token));
                        }
                        Err(e) => {
                            last_cfg_err = e;
                        }
                    }
                }

                Err(last_cfg_err)
            })();

            match result {
                Ok(handle) => {
                    return Ok(handle);
                }
                Err(err) => {
                    if audio_source_opt.is_none() {
                        audio_source_opt = attempt_audio_source_opt;
                    }
                    eprintln!(
                        "Warning: Failed to create/play audio stream on device '{}': {:?}",
                        device_name, err
                    );
                    last_err = err;
                }
            }
        }

        Err(last_err)
    })();

    match cpal_stream_result {
        Ok(handle) => Ok(handle),
        Err(err) => {
            if mic {
                return Err(err);
            }
            eprintln!(
                "Warning: CPAL audio stream creation failed: {:?}. Falling back to silent dummy device.",
                err
            );

            if let Some(mut audio_source) = audio_source_opt
                .take()
                .or_else(|| load_audio_source(file_path).ok())
            {
                let sample_rate = target_rate;
                let window_size = calculate_power_of_two_window_size(sample_rate);
                let (tx, rx) = bounded::<DspMessage>(32);

                {
                    let mut state = shared_state.lock().unwrap();
                    state.is_dummy_audio_output = true;
                    populate_state_from_audio_source(
                        &mut state,
                        audio_source.as_mut(),
                        2,
                        sample_rate,
                    );
                }

                let max_frequency = { shared_state.lock().unwrap().max_frequency };
                spawn_dsp_thread(
                    rx,
                    shared_state.clone(),
                    sample_rate,
                    max_frequency,
                    window_size,
                );

                run_dummy(
                    audio_source,
                    shared_state,
                    tx,
                    sample_rate,
                    window_size,
                    file_path,
                )
            } else {
                Err(anyhow::anyhow!(
                    "CPAL audio stream creation failed: {:?}, and no audio source was available for fallback.",
                    err
                ))
            }
        }
    }
}

fn populate_state_from_audio_source(
    state: &mut AppState,
    audio_source: &mut dyn AudioSource,
    hardware_channels: i32,
    sample_rate: u32,
) {
    state.current_seconds = 0.0;
    state.lookahead_queue.clear();
    state.lookahead_sample_buffer.clear();
    state.lookahead_buffer_start_time = 0.0;
    state.lookahead_timeline = vec![0.0; 600 * 8];
    state.waveform_history.clear();
    state.raw_waveform.fill(0.0);
    state.track_ended = false;
    state.artist = audio_source.get_artist();
    state.module_type = audio_source.get_type();
    state.duration_seconds = audio_source.get_duration_seconds();
    let tracker_channels = audio_source.get_tracker_channels();
    state.hardware_channels = hardware_channels;
    let source_channels = audio_source.get_num_channels();
    state.source_channels = source_channels;
    if !crate::state::SurroundUpmixConfig::can_upmix(source_channels, hardware_channels) {
        state.surround_upmix.enabled = false;
    } else if !state.surround_upmix.user_overridden {
        state.surround_upmix.enabled = !state.force_stereo_downmix
            && !state.passthrough_enabled
            && crate::state::SurroundUpmixConfig::should_enable_by_default(
                source_channels,
                hardware_channels,
            );
    }
    state.num_channels = if state
        .surround_upmix
        .is_active_for(source_channels, hardware_channels)
    {
        hardware_channels.min(12)
    } else if let Some(tc) = tracker_channels {
        tc + 2
    } else {
        audio_source.get_num_channels()
    };
    state.channel_vus = vec![0.0; state.num_channels as usize];
    state.peak_vus = vec![0.0; state.num_channels as usize];
    state.bpm = audio_source.get_tempo();
    state.speed = audio_source.get_speed();
    state.video_info = audio_source.get_video_info();
    state.max_frequency = audio_source
        .get_intrinsic_sample_rate()
        .map(|r| r as f32 / 2.0)
        .unwrap_or(10000.0);
    state.current_sample_rate = sample_rate as f32;
    state.num_samples = audio_source.get_num_samples();
    state.num_instruments = audio_source.get_num_instruments();
    state.num_patterns = audio_source.get_num_patterns();
    state.bitrate = audio_source.get_bitrate();
    state.stats.audio_bitrate_kbps = state.bitrate;
    state.stats.video_fps = audio_source.get_video_fps();
    state.stats.video_bitrate_kbps = audio_source.get_video_bitrate();
    state.audio_tracks = audio_source.get_audio_tracks();
    state.selected_audio_track = audio_source.get_selected_audio_track();
    state.active_audio_tracks = audio_source.get_active_audio_tracks();
    state.audio_track_volumes = vec![1.0; state.audio_tracks.len()];
    state.multi_track_mix_mode = state.active_audio_tracks.len() > 1;
    state.audio_track_request = None;
    state.audio_mix_request = None;
    state.has_video_stream = audio_source.has_video_stream();
    state.tracker_channels = tracker_channels;
    state.tracker_patterns_by_order = audio_source.pre_format_tracker_data();
}

fn push_chunk_lookahead_slices(
    state: &mut AppState,
    chunk: &AudioChunk,
    frames_read: usize,
    hardware_channels: usize,
    sample_rate: u32,
) {
    if frames_read == 0 || hardware_channels == 0 || sample_rate == 0 {
        return;
    }
    let slice_frames = ((sample_rate as f64 * 0.01).round() as usize).max(16);
    let valid_samples = frames_read * hardware_channels;

    let existing_end_time = state.lookahead_buffer_start_time
        + (state.lookahead_sample_buffer.len() / hardware_channels) as f64 / sample_rate as f64;
    if state.lookahead_sample_buffer.is_empty()
        || (chunk.current_seconds - existing_end_time).abs() > 0.05
    {
        state.lookahead_sample_buffer.clear();
        state.lookahead_buffer_start_time = chunk.current_seconds;
    }
    state
        .lookahead_sample_buffer
        .extend_from_slice(&chunk.samples[..valid_samples]);

    let buffer_frames = state.lookahead_sample_buffer.len() / hardware_channels;
    let full_slices = buffer_frames / slice_frames;

    for s in 0..full_slices {
        let start_f = s * slice_frames;
        let end_f = start_f + slice_frames;
        let slice_time = state.lookahead_buffer_start_time + (start_f as f64 / sample_rate as f64);

        let mut min_l = 0.0f32;
        let mut max_l = 0.0f32;
        let mut min_r = 0.0f32;
        let mut max_r = 0.0f32;
        let mut sq_l = 0.0f32;
        let mut sq_r = 0.0f32;
        let mut bass_acc = 0.0f32;
        let mut treb_acc = 0.0f32;

        let cnt = slice_frames as f32;
        let mut prev_l = 0.0f32;
        for i in start_f..end_f {
            let frame_slice =
                &state.lookahead_sample_buffer[i * hardware_channels..(i + 1) * hardware_channels];
            let l = get_downmixed_sample(hardware_channels as u16, frame_slice, 2, 0);
            let r = get_downmixed_sample(hardware_channels as u16, frame_slice, 2, 1);

            min_l = min_l.min(l);
            max_l = max_l.max(l);
            min_r = min_r.min(r);
            max_r = max_r.max(r);
            sq_l += l * l;
            sq_r += r * r;

            let mono = (l + r) * 0.5;
            let diff = if i > start_f { (l - prev_l).abs() } else { 0.0 };
            prev_l = l;

            bass_acc += mono.abs();
            treb_acc += diff * 2.0;
        }

        let rms_l = (sq_l / cnt).sqrt().min(1.0);
        let rms_r = (sq_r / cnt).sqrt().min(1.0);
        let bass = (bass_acc / cnt * 2.0).min(1.0);
        let treble = (treb_acc / cnt * 2.0).min(1.0);

        while let Some((t, _)) = state.lookahead_queue.back() {
            if *t >= slice_time - 0.0001 {
                state.lookahead_queue.pop_back();
            } else {
                break;
            }
        }

        state.lookahead_queue.push_back((
            slice_time,
            [
                min_l.clamp(-1.0, 1.0),
                max_l.clamp(-1.0, 1.0),
                min_r.clamp(-1.0, 1.0),
                max_r.clamp(-1.0, 1.0),
                rms_l,
                rms_r,
                bass,
                treble,
            ],
        ));
    }

    let consumed_samples = full_slices * slice_frames * hardware_channels;
    if consumed_samples > 0 {
        state.lookahead_sample_buffer.drain(0..consumed_samples);
        state.lookahead_buffer_start_time +=
            (full_slices * slice_frames) as f64 / sample_rate as f64;
    }

    while state.lookahead_queue.len() > 1800 {
        state.lookahead_queue.pop_front();
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
}

pub fn push_planar_lookahead_slices(
    state: &mut AppState,
    channels: &[Vec<f32>],
    sample_rate: u32,
    current_seconds: f64,
) {
    if channels.is_empty() || sample_rate == 0 {
        return;
    }
    let left = &channels[0];
    if left.is_empty() {
        return;
    }
    let right = if channels.len() > 1 {
        &channels[1]
    } else {
        left
    };
    let center = if channels.len() > 2 {
        Some(&channels[2])
    } else {
        None
    };

    let slice_frames = ((sample_rate as f64 * 0.01).round() as usize).max(16);
    let total_samples = left.len().min(right.len());
    if total_samples == 0 {
        return;
    }

    let existing_end_time = state.lookahead_buffer_start_time
        + (state.lookahead_sample_buffer.len() / 2) as f64 / sample_rate as f64;
    if state.lookahead_sample_buffer.is_empty()
        || (current_seconds - existing_end_time).abs() > 0.05
    {
        state.lookahead_sample_buffer.clear();
        state.lookahead_buffer_start_time = current_seconds;
    }

    state.lookahead_sample_buffer.reserve(total_samples * 2);
    for i in 0..total_samples {
        let mut l = left[i];
        let mut r = right[i];
        if let Some(c_plane) = center
            && i < c_plane.len()
        {
            let c_val = c_plane[i] * std::f32::consts::FRAC_1_SQRT_2;
            l += c_val;
            r += c_val;
        }
        state.lookahead_sample_buffer.push(l);
        state.lookahead_sample_buffer.push(r);
    }

    let buffer_frames = state.lookahead_sample_buffer.len() / 2;
    let full_slices = buffer_frames / slice_frames;
    if full_slices == 0 {
        return;
    }

    for s in 0..full_slices {
        let start_f = s * slice_frames;
        let end_f = start_f + slice_frames;
        let slice_time = state.lookahead_buffer_start_time + (start_f as f64 / sample_rate as f64);

        let mut min_l = 0.0f32;
        let mut max_l = 0.0f32;
        let mut min_r = 0.0f32;
        let mut max_r = 0.0f32;
        let mut sq_l = 0.0f32;
        let mut sq_r = 0.0f32;
        let mut bass_acc = 0.0f32;
        let mut treb_acc = 0.0f32;

        let cnt = slice_frames as f32;
        for i in start_f..end_f {
            let l = state.lookahead_sample_buffer[i * 2];
            let r = state.lookahead_sample_buffer[i * 2 + 1];

            min_l = min_l.min(l);
            max_l = max_l.max(l);
            min_r = min_r.min(r);
            max_r = max_r.max(r);
            sq_l += l * l;
            sq_r += r * r;

            let mono = (l + r) * 0.5;
            let diff = if i > start_f {
                let prev_l = state.lookahead_sample_buffer[(i - 1) * 2];
                (l - prev_l).abs()
            } else {
                0.0
            };

            bass_acc += mono.abs();
            treb_acc += diff * 2.0;
        }

        let rms_l = (sq_l / cnt).sqrt().min(1.0);
        let rms_r = (sq_r / cnt).sqrt().min(1.0);
        let bass = (bass_acc / cnt * 2.0).min(1.0);
        let treble = (treb_acc / cnt * 2.0).min(1.0);

        while let Some((t, _)) = state.lookahead_queue.back() {
            if *t >= slice_time - 0.0001 {
                state.lookahead_queue.pop_back();
            } else {
                break;
            }
        }

        state.lookahead_queue.push_back((
            slice_time,
            [
                min_l.clamp(-1.0, 1.0),
                max_l.clamp(-1.0, 1.0),
                min_r.clamp(-1.0, 1.0),
                max_r.clamp(-1.0, 1.0),
                rms_l,
                rms_r,
                bass,
                treble,
            ],
        ));
    }

    let consumed_samples = full_slices * slice_frames * 2;
    if consumed_samples > 0 {
        state.lookahead_sample_buffer.drain(0..consumed_samples);
        state.lookahead_buffer_start_time +=
            (full_slices * slice_frames) as f64 / sample_rate as f64;
    }

    while state.lookahead_queue.len() > 1800 {
        state.lookahead_queue.pop_front();
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
}

fn process_decoder_state_requests(
    state: &mut AppState,
    audio_source: &mut dyn AudioSource,
    ready_rx: &Receiver<AudioChunk>,
    free_tx: &Sender<AudioChunk>,
    mut clear_video_packets: impl FnMut(),
) -> bool {
    let mut seeked = false;
    if let Some(mix_req) = state.audio_mix_request.take()
        && state.audio_tracks.len() > 1
        && !mix_req.is_empty()
    {
        let mut new_active: Vec<usize> = mix_req.iter().map(|&(idx, _)| idx).collect();
        new_active.sort();
        let current_active = audio_source.get_active_audio_tracks();
        let tracks_changed = new_active != current_active;
        let play_pos = state.current_seconds;
        if audio_source.set_active_audio_tracks(&mix_req).is_ok() {
            if tracks_changed {
                audio_source.set_position_seconds(play_pos);
                state.audio_track_request = None;
            }
            state.active_audio_tracks = audio_source.get_active_audio_tracks();
            if let Some(&first_idx) = state.active_audio_tracks.first() {
                state.selected_audio_track = first_idx;
            }
            let source_channels = audio_source.get_num_channels();
            state.source_channels = source_channels;
            if !crate::state::SurroundUpmixConfig::can_upmix(
                source_channels,
                state.hardware_channels,
            ) {
                state.surround_upmix.enabled = false;
            } else if !state.surround_upmix.user_overridden {
                state.surround_upmix.enabled = !state.force_stereo_downmix
                    && !state.passthrough_enabled
                    && crate::state::SurroundUpmixConfig::should_enable_by_default(
                        source_channels,
                        state.hardware_channels,
                    );
            }
            state.num_channels = if state
                .surround_upmix
                .is_active_for(source_channels, state.hardware_channels)
            {
                state.hardware_channels.min(12)
            } else {
                audio_source.get_num_channels()
            };
            let ch_count = state.num_channels as usize;
            state.channel_vus.resize(ch_count, 0.0);
            state.peak_vus.resize(ch_count, 0.0);
            state.module_type = audio_source.get_type();
            state.bitrate = audio_source.get_bitrate();
            state.stats.audio_bitrate_kbps = state.bitrate;
            state.multi_track_mix_mode = state.active_audio_tracks.len() > 1;

            if state.audio_track_volumes.len() != state.audio_tracks.len() {
                state.audio_track_volumes = vec![1.0; state.audio_tracks.len()];
            }
            for &(idx, vol) in &mix_req {
                if idx < state.audio_track_volumes.len() {
                    state.audio_track_volumes[idx] = vol;
                }
            }

            if tracks_changed {
                let mix_desc = if state.active_audio_tracks.len() > 1 {
                    let track_nums: Vec<String> = state
                        .active_audio_tracks
                        .iter()
                        .map(|idx| (idx + 1).to_string())
                        .collect();
                    format!("🎛 Audio Mix: Tracks {}", track_nums.join("+"))
                } else {
                    let idx = state.selected_audio_track;
                    let track_title = state
                        .audio_tracks
                        .get(idx)
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| format!("Track {}", idx + 1));
                    if track_title.to_lowercase().starts_with("track") {
                        format!("🎛 {}", track_title)
                    } else {
                        format!("🎛 Track {}: {}", idx + 1, track_title)
                    }
                };
                state.osd_text = Some(mix_desc);
                state.osd_timer = 3.0;

                state.lookahead_queue.clear();
                state.lookahead_sample_buffer.clear();
                state.lookahead_timeline.fill(0.0);
                state.lookahead_buffer_start_time = play_pos;
                state.waveform_history.clear();

                while let Ok(c) = ready_rx.try_recv() {
                    let _ = free_tx.try_send(c);
                }
                seeked = true;
            }
        }
    }
    if let Some(track_idx) = state.audio_track_request.take()
        && state.audio_tracks.len() > 1
        && track_idx < state.audio_tracks.len()
        && (audio_source.get_active_audio_tracks() != vec![track_idx])
    {
        let play_pos = state.current_seconds;
        if audio_source.select_audio_track(track_idx).is_ok() {
            audio_source.set_position_seconds(play_pos);
            state.selected_audio_track = track_idx;
            state.active_audio_tracks = vec![track_idx];
            state.multi_track_mix_mode = false;
            let source_channels = audio_source.get_num_channels();
            state.source_channels = source_channels;
            if !crate::state::SurroundUpmixConfig::can_upmix(
                source_channels,
                state.hardware_channels,
            ) {
                state.surround_upmix.enabled = false;
            } else if !state.surround_upmix.user_overridden {
                state.surround_upmix.enabled = !state.force_stereo_downmix
                    && !state.passthrough_enabled
                    && crate::state::SurroundUpmixConfig::should_enable_by_default(
                        source_channels,
                        state.hardware_channels,
                    );
            }
            state.num_channels = if state
                .surround_upmix
                .is_active_for(source_channels, state.hardware_channels)
            {
                state.hardware_channels.min(12)
            } else {
                audio_source.get_num_channels()
            };
            let ch_count = state.num_channels as usize;
            state.channel_vus.resize(ch_count, 0.0);
            state.peak_vus.resize(ch_count, 0.0);
            state.module_type = audio_source.get_type();
            state.bitrate = audio_source.get_bitrate();
            state.stats.audio_bitrate_kbps = state.bitrate;
            let track_title = state
                .audio_tracks
                .get(track_idx)
                .map(|t| t.title.clone())
                .unwrap_or_else(|| format!("Track {}", track_idx + 1));
            let display_title = if track_title.to_lowercase().starts_with("track") {
                format!("🎛 {}", track_title)
            } else {
                format!("🎛 Track {}: {}", track_idx + 1, track_title)
            };
            state.osd_text = Some(display_title);
            state.osd_timer = 3.0;

            state.lookahead_queue.clear();
            state.lookahead_sample_buffer.clear();
            state.lookahead_timeline.fill(0.0);
            state.lookahead_buffer_start_time = play_pos;
            state.waveform_history.clear();

            while let Ok(c) = ready_rx.try_recv() {
                let _ = free_tx.try_send(c);
            }
            seeked = true;
        }
    }
    if let Some(pos) = state.seek_request.take() {
        audio_source.set_position_seconds(pos);
        state.current_seconds = pos;
        state.track_ended = false;
        state.seek_epoch += 1;
        state.lookahead_queue.clear();
        state.lookahead_sample_buffer.clear();
        state.lookahead_timeline.fill(0.0);
        state.lookahead_buffer_start_time = pos;
        state.waveform_history.clear();
        while let Ok(c) = ready_rx.try_recv() {
            let _ = free_tx.try_send(c);
        }
        clear_video_packets();
        seeked = true;
    }
    seeked
}

fn decode_single_audio_chunk(
    audio_source: &mut dyn AudioSource,
    chunk: &mut AudioChunk,
    hardware_channels: usize,
    sample_rate: u32,
    chunk_frames: usize,
) -> (usize, f32, u32) {
    let chunk_start_seconds = audio_source.get_position_seconds();
    let decode_start = Instant::now();
    let frames_read = audio_source.read_frames(
        hardware_channels,
        sample_rate,
        &mut chunk.samples[..chunk_frames * hardware_channels],
    );
    let decode_elapsed = decode_start.elapsed().as_micros() as f32;

    chunk.valid_frames = frames_read;
    chunk.current_order = audio_source.get_current_order();
    chunk.current_row = audio_source.get_current_row();
    chunk.bpm = audio_source.get_tempo();
    chunk.speed = audio_source.get_speed();
    chunk.current_seconds = if audio_source.has_video_stream() && frames_read > 0 {
        (audio_source.get_position_seconds() - (frames_read as f64 / sample_rate.max(1) as f64))
            .max(0.0)
    } else {
        chunk_start_seconds
    };
    chunk.current_row_string.clear();
    chunk
        .current_row_string
        .push_str(&audio_source.get_current_row_string());
    chunk.tracker_channels = audio_source.get_tracker_channels();
    chunk.spatial_channels = audio_source.get_num_channels();
    chunk.track_ended = frames_read == 0;

    let mut left_peak = 0.0_f32;
    let mut right_peak = 0.0_f32;
    let mut clips = 0u32;
    for i in 0..frames_read {
        let l_val = chunk.samples[i * hardware_channels];
        left_peak = left_peak.max(l_val.abs());
        if l_val.abs() > 1.0001 {
            clips += 1;
        }

        if hardware_channels > 1 {
            let r_val = chunk.samples[i * hardware_channels + 1];
            right_peak = right_peak.max(r_val.abs());
            if r_val.abs() > 1.0001 {
                clips += 1;
            }
        } else {
            right_peak = left_peak;
        }
    }
    chunk.left_peak = left_peak;
    chunk.right_peak = right_peak;

    chunk.channel_vus.clear();
    if let Some(num_mod_channels) = chunk.tracker_channels {
        chunk.channel_vus.push(left_peak);
        for i in 0..num_mod_channels {
            chunk
                .channel_vus
                .push(audio_source.get_current_channel_vu_mono(i));
        }
        chunk.channel_vus.push(right_peak);
    } else {
        for i in 0..chunk.spatial_channels {
            chunk
                .channel_vus
                .push(audio_source.get_current_channel_vu_mono(i));
        }
    }

    (frames_read, decode_elapsed, clips)
}

#[cfg(not(target_os = "android"))]
pub type VideoPacketChannel = (
    Sender<(u64, ffmpeg_next::Packet)>,
    Receiver<(u64, ffmpeg_next::Packet)>,
);

#[cfg(not(target_os = "android"))]
fn open_video_decoder(
    params: &ffmpeg_next::codec::Parameters,
    allow_hw: bool,
) -> Result<(ffmpeg_next::decoder::Video, Option<String>), ffmpeg_next::Error> {
    unsafe {
        ffmpeg_next::ffi::av_log_set_level(ffmpeg_next::ffi::AV_LOG_FATAL);
    }
    if allow_hw {
        #[cfg(target_os = "windows")]
        let preferred_hw: &[(ffmpeg_next::ffi::AVHWDeviceType, &str)] = &[
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
                "HW (D3D11VA)",
            ),
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_DXVA2,
                "HW (DXVA2)",
            ),
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN,
                "HW (Vulkan)",
            ),
        ];
        #[cfg(target_os = "linux")]
        let preferred_hw: &[(ffmpeg_next::ffi::AVHWDeviceType, &str)] = &[
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                "HW (VAAPI)",
            ),
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN,
                "HW (Vulkan)",
            ),
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VDPAU,
                "HW (VDPAU)",
            ),
        ];
        #[cfg(target_os = "macos")]
        let preferred_hw: &[(ffmpeg_next::ffi::AVHWDeviceType, &str)] = &[(
            ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
            "HW (VideoToolbox)",
        )];
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        let preferred_hw: &[(ffmpeg_next::ffi::AVHWDeviceType, &str)] = &[];

        let codec_ptr =
            unsafe { ffmpeg_next::ffi::avcodec_find_decoder((*params.as_ptr()).codec_id) };
        if !codec_ptr.is_null() {
            for &(hw_type, label) in preferred_hw {
                let mut hw_pix_fmt = ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE;
                let mut idx = 0;
                loop {
                    let cfg = unsafe { ffmpeg_next::ffi::avcodec_get_hw_config(codec_ptr, idx) };
                    if cfg.is_null() {
                        break;
                    }
                    let methods = unsafe { (*cfg).methods };
                    let dev_type = unsafe { (*cfg).device_type };
                    if (methods
                        & (ffmpeg_next::ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32))
                        != 0
                        && dev_type == hw_type
                    {
                        hw_pix_fmt = unsafe { (*cfg).pix_fmt };
                        break;
                    }
                    idx += 1;
                }

                if hw_pix_fmt == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                    continue;
                }

                let mut hw_device_ctx: *mut ffmpeg_next::ffi::AVBufferRef = std::ptr::null_mut();
                let err = unsafe {
                    ffmpeg_next::ffi::av_hwdevice_ctx_create(
                        &mut hw_device_ctx,
                        hw_type,
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        0,
                    )
                };
                if err == 0 && !hw_device_ctx.is_null() {
                    if let Ok(mut context) =
                        ffmpeg_next::codec::context::Context::from_parameters(params.clone())
                    {
                        unsafe {
                            (*context.as_mut_ptr()).hw_device_ctx =
                                ffmpeg_next::ffi::av_buffer_ref(hw_device_ctx);
                        }
                        if let Ok(mut decoder) = context.decoder().video() {
                            // Verify that the hardware device driver actually supports this codec,
                            // profile, and resolution (e.g. Fedora's stock Mesa VAAPI driver omits
                            // H.264/HEVC profiles unless mesa-va-drivers-freeworld is installed).
                            let frames_ok = unsafe {
                                if (*decoder.as_ptr()).sw_pix_fmt
                                    == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE
                                {
                                    (*decoder.as_mut_ptr()).sw_pix_fmt = if (*decoder.as_ptr())
                                        .pix_fmt
                                        != ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE
                                    {
                                        (*decoder.as_ptr()).pix_fmt
                                    } else {
                                        ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_YUV420P
                                    };
                                }
                                let mut test_frames_ctx: *mut ffmpeg_next::ffi::AVBufferRef =
                                    std::ptr::null_mut();
                                let ret = ffmpeg_next::ffi::avcodec_get_hw_frames_parameters(
                                    decoder.as_mut_ptr(),
                                    hw_device_ctx,
                                    hw_pix_fmt,
                                    &mut test_frames_ctx,
                                );
                                if !test_frames_ctx.is_null() {
                                    ffmpeg_next::ffi::av_buffer_unref(&mut test_frames_ctx);
                                }
                                ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx);
                                ret == 0
                            };
                            if frames_ok {
                                return Ok((decoder, Some(label.to_string())));
                            }
                            #[cfg(target_os = "linux")]
                            if hw_type == ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI {
                                // On Linux, VAAPI and Vulkan/VDPAU share the same GPU decode block
                                // and Mesa codec build config; if VAAPI opened the device and
                                // rejected the codec profile, skip redundant Vulkan/VDPAU probes.
                                break;
                            }
                        } else {
                            unsafe {
                                ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx);
                            }
                        }
                    } else {
                        unsafe {
                            ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx);
                        }
                    }
                }
            }
        }
    }

    let mut context = ffmpeg_next::codec::context::Context::from_parameters(params.clone())?;
    let hw_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
        .clamp(2, 16);
    context.set_threading(ffmpeg_next::codec::threading::Config {
        kind: ffmpeg_next::codec::threading::Type::Frame,
        count: hw_threads,
    });
    let decoder = context.decoder().video()?;
    Ok((decoder, None))
}

#[cfg(not(target_os = "android"))]
pub fn spawn_video_decoder_thread(
    params: ffmpeg_next::codec::Parameters,
    time_base: ffmpeg_next::Rational,
    rotation: u32,
    shared_state: Arc<Mutex<AppState>>,
    stop_token: Arc<std::sync::atomic::AtomicBool>,
) -> VideoPacketChannel {
    let (video_packet_tx, video_packet_rx) = bounded::<(u64, ffmpeg_next::Packet)>(4096);
    let (video_frame_tx, video_frame_rx) = bounded::<crate::state::VideoFrame>(16);
    let (free_video_frame_tx, free_video_frame_rx) = unbounded::<crate::state::VideoFrame>();

    for _ in 0..16 {
        let _ = free_video_frame_tx.try_send(crate::state::VideoFrame {
            pts: 0.0,
            width: 0,
            height: 0,
            rotation,
            y_plane: Vec::new(),
            u_plane: Vec::new(),
            v_plane: Vec::new(),
            y_stride: 0,
            u_stride: 0,
            v_stride: 0,
            bit_depth: 8,
            color_space: 0,
            color_range: 0,
            color_trc: 0,
        });
    }

    if let Ok(mut state) = shared_state.lock() {
        state.video_frame_rx = Some(video_frame_rx);
        state.free_video_frame_tx = Some(free_video_frame_tx.clone());
        state.has_video_stream = true;
    }

    let state_for_video = shared_state;
    let video_packet_rx_for_video = video_packet_rx.clone();
    let stop_token_video = stop_token;
    std::thread::spawn(move || {
        if let Ok((mut decoder, mut hw_label)) = open_video_decoder(&params, true) {
            if let Ok(mut state) = state_for_video.lock() {
                state.stats.video_hw_decoder =
                    hw_label.clone().unwrap_or_else(|| "CPU".to_string());
            }
            let tb = time_base.numerator() as f64 / time_base.denominator() as f64;
            let mut local_epoch = 0;
            let mut fallback_pts_seconds = 0.0;
            // Deliver the very first decoded frame immediately (even while paused at startup)
            let mut is_first_frame_after_seek = true;
            let mut decoded_any_frame = false;
            let mut pkt_bytes_accum: u64 = 0;
            let mut pkt_window_start_pts: Option<f64> = None;
            let mut prev_frame_pts: Option<f64> = None;
            let mut scaler: Option<(
                ffmpeg_next::format::Pixel,
                u32,
                u32,
                ffmpeg_next::software::scaling::Context,
            )> = None;

            while !stop_token_video.load(std::sync::atomic::Ordering::Relaxed) {
                match video_packet_rx_for_video.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok((packet_epoch, packet)) => {
                        {
                            let state = state_for_video.lock().unwrap();
                            if state.seek_epoch > local_epoch {
                                decoder.flush();
                                local_epoch = state.seek_epoch;
                                is_first_frame_after_seek = true;
                                pkt_bytes_accum = 0;
                                pkt_window_start_pts = None;
                                prev_frame_pts = None;
                            }
                        }

                        if packet_epoch < local_epoch {
                            continue;
                        }

                        pkt_bytes_accum = pkt_bytes_accum.saturating_add(packet.size() as u64);

                        let decode_start = std::time::Instant::now();
                        let mut send_ok = decoder.send_packet(&packet).is_ok();
                        if !send_ok
                            && hw_label.is_some()
                            && !decoded_any_frame
                            && let Ok((sw_dec, _)) = open_video_decoder(&params, false)
                        {
                            decoder = sw_dec;
                            hw_label = None;
                            if let Ok(mut state) = state_for_video.lock() {
                                state.stats.video_hw_decoder = "CPU".to_string();
                            }
                            send_ok = decoder.send_packet(&packet).is_ok();
                        }

                        if send_ok {
                            let mut decoded = ffmpeg_next::frame::Video::empty();
                            loop {
                                match decoder.receive_frame(&mut decoded) {
                                    Ok(()) => {}
                                    Err(ffmpeg_next::Error::Other { errno })
                                        if errno == ffmpeg_next::ffi::EAGAIN =>
                                    {
                                        break;
                                    }
                                    Err(ffmpeg_next::Error::Eof) => break,
                                    Err(_) => {
                                        if hw_label.is_some()
                                            && !decoded_any_frame
                                            && let Ok((sw_dec, _)) =
                                                open_video_decoder(&params, false)
                                        {
                                            decoder = sw_dec;
                                            hw_label = None;
                                            if let Ok(mut state) = state_for_video.lock() {
                                                state.stats.video_hw_decoder = "CPU".to_string();
                                            }
                                            if decoder.send_packet(&packet).is_ok()
                                                && decoder.receive_frame(&mut decoded).is_ok()
                                            {
                                                // Recovered via CPU fallback on the first packet
                                            } else {
                                                break;
                                            }
                                        } else {
                                            break;
                                        }
                                    }
                                }

                                if stop_token_video.load(std::sync::atomic::Ordering::Relaxed) {
                                    return;
                                }
                                let mut pts = decoded
                                    .timestamp()
                                    .map(|t| t as f64 * tb)
                                    .or_else(|| decoded.pts().map(|p| p as f64 * tb))
                                    .unwrap_or(-1.0);

                                if pts < 0.0 {
                                    pts = fallback_pts_seconds;
                                    fallback_pts_seconds += 1.0 / 30.0;
                                } else {
                                    fallback_pts_seconds = pts + (1.0 / 30.0);
                                }

                                let mut skip_push = false;
                                if is_first_frame_after_seek {
                                    let (cached_seconds, current_epoch, duration) = {
                                        let state = state_for_video.lock().unwrap();
                                        (
                                            state.current_seconds,
                                            state.seek_epoch,
                                            state.duration_seconds,
                                        )
                                    };
                                    if current_epoch > local_epoch {
                                        skip_push = true;
                                    } else {
                                        let target_t = if duration > 0.25 {
                                            cached_seconds.min((duration - 0.25).max(0.0))
                                        } else {
                                            cached_seconds
                                        };
                                        if pts >= 0.0 && pts < target_t - 0.15 {
                                            // Fast-forward through intermediate GOP reference frames
                                            // between the keyframe and the seek target without GPU transfer
                                            // or snapping `state.current_seconds` backwards.
                                            skip_push = true;
                                        } else {
                                            is_first_frame_after_seek = false;
                                        }
                                    }
                                } else {
                                    loop {
                                        if stop_token_video
                                            .load(std::sync::atomic::Ordering::Relaxed)
                                        {
                                            return;
                                        }
                                        let (cached_seconds, current_epoch, is_paused, track_ended) = {
                                            let state = state_for_video.lock().unwrap();
                                            (
                                                state.current_seconds,
                                                state.seek_epoch,
                                                state.is_paused,
                                                state.track_ended,
                                            )
                                        };

                                        if track_ended || current_epoch > local_epoch {
                                            skip_push = true;
                                            break;
                                        }

                                        if pts < cached_seconds - 0.15 {
                                            skip_push = true;
                                            break;
                                        }

                                        if pts <= cached_seconds + 0.05 {
                                            break;
                                        }

                                        // Only skip if PTS is wildly ahead due to a broken container discontinuity (>30s)
                                        if pts > cached_seconds + 30.0 {
                                            skip_push = true;
                                            break;
                                        }

                                        let sleep_dur = if is_paused { 25 } else { 4 };
                                        std::thread::sleep(std::time::Duration::from_millis(
                                            sleep_dur,
                                        ));
                                    }
                                }

                                if skip_push {
                                    continue;
                                }

                                let mut frame_opt = None;
                                loop {
                                    if stop_token_video.load(std::sync::atomic::Ordering::Relaxed) {
                                        return;
                                    }
                                    let (current_epoch, is_paused, track_ended) = {
                                        let state = state_for_video.lock().unwrap();
                                        (state.seek_epoch, state.is_paused, state.track_ended)
                                    };
                                    if track_ended || current_epoch > local_epoch {
                                        skip_push = true;
                                        break;
                                    }
                                    match free_video_frame_rx
                                        .recv_timeout(std::time::Duration::from_millis(10))
                                    {
                                        Ok(f) => {
                                            frame_opt = Some(f);
                                            break;
                                        }
                                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                                            let sleep_dur = if is_paused { 25 } else { 5 };
                                            std::thread::sleep(std::time::Duration::from_millis(
                                                sleep_dur,
                                            ));
                                        }
                                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                                            return;
                                        }
                                    }
                                }

                                if skip_push {
                                    continue;
                                }

                                if let Some(mut frame) = frame_opt {
                                    let transfer_start = std::time::Instant::now();
                                    let is_hw_frame =
                                        unsafe { !(*decoded.as_ptr()).hw_frames_ctx.is_null() };
                                    let mut hw_sw_frame = ffmpeg_next::frame::Video::empty();
                                    if is_hw_frame {
                                        let ret = unsafe {
                                            ffmpeg_next::ffi::av_hwframe_transfer_data(
                                                hw_sw_frame.as_mut_ptr(),
                                                decoded.as_ptr(),
                                                0,
                                            )
                                        };
                                        if ret < 0 {
                                            let _ = free_video_frame_tx.try_send(frame);
                                            if hw_label.is_some()
                                                && !decoded_any_frame
                                                && let Ok((sw_dec, _)) =
                                                    open_video_decoder(&params, false)
                                            {
                                                decoder = sw_dec;
                                                hw_label = None;
                                                if let Ok(mut state) = state_for_video.lock() {
                                                    state.stats.video_hw_decoder =
                                                        "CPU".to_string();
                                                }
                                            }
                                            continue;
                                        }
                                    } else if hw_label.is_some() && !decoded_any_frame {
                                        hw_label = None;
                                        if let Ok(mut state) = state_for_video.lock() {
                                            state.stats.video_hw_decoder = "CPU".to_string();
                                        }
                                    }
                                    decoded_any_frame = true;

                                    let active_decoded: &ffmpeg_next::frame::Video =
                                        if is_hw_frame { &hw_sw_frame } else { &decoded };

                                    frame.pts = pts;
                                    frame.width = active_decoded.width();
                                    frame.height = active_decoded.height();
                                    frame.rotation = rotation;
                                    frame.color_space = decoded.color_space() as u32;
                                    frame.color_range = decoded.color_range() as u32;
                                    frame.color_trc =
                                        decoded.color_transfer_characteristic() as u32;

                                    let pix_fmt = active_decoded.format();
                                    let is_nv12 = pix_fmt == ffmpeg_next::format::Pixel::NV12
                                        && active_decoded.planes() >= 2;
                                    let is_p010 = pix_fmt == ffmpeg_next::format::Pixel::P010LE
                                        && active_decoded.planes() >= 2;

                                    if is_nv12 {
                                        let w = active_decoded.width() as usize;
                                        let h = active_decoded.height() as usize;
                                        let half_w = w / 2;
                                        let half_h = h / 2;

                                        frame.bit_depth = 8;
                                        frame.y_stride = active_decoded.stride(0);
                                        frame.u_stride = half_w;
                                        frame.v_stride = half_w;

                                        let y_len = frame.y_stride * h;
                                        frame.y_plane.clear();
                                        let y_src = active_decoded.data(0);
                                        let y_copy = y_len.min(y_src.len());
                                        frame.y_plane.extend_from_slice(&y_src[..y_copy]);
                                        if y_copy < y_len {
                                            frame.y_plane.resize(y_len, 0);
                                        }

                                        frame.u_plane.resize(half_w * half_h, 128);
                                        frame.v_plane.resize(half_w * half_h, 128);
                                        let uv_stride = active_decoded.stride(1);
                                        let uv_src = active_decoded.data(1);
                                        for row in 0..half_h {
                                            let src_row_start = row * uv_stride;
                                            let dst_row_start = row * half_w;
                                            if src_row_start + half_w * 2 <= uv_src.len() {
                                                let src_row = &uv_src
                                                    [src_row_start..src_row_start + half_w * 2];
                                                let u_row = &mut frame.u_plane
                                                    [dst_row_start..dst_row_start + half_w];
                                                let v_row = &mut frame.v_plane
                                                    [dst_row_start..dst_row_start + half_w];
                                                for col in 0..half_w {
                                                    u_row[col] = src_row[col * 2];
                                                    v_row[col] = src_row[col * 2 + 1];
                                                }
                                            }
                                        }
                                    } else if is_p010 {
                                        let w = active_decoded.width() as usize;
                                        let h = active_decoded.height() as usize;
                                        let half_w = w / 2;
                                        let half_h = h / 2;
                                        let c_row_bytes = half_w * 2;

                                        // MSB-aligned 10-bit surface (bits [15:6])
                                        frame.bit_depth = 16;
                                        frame.y_stride = active_decoded.stride(0);
                                        frame.u_stride = c_row_bytes;
                                        frame.v_stride = c_row_bytes;

                                        let y_len = frame.y_stride * h;
                                        frame.y_plane.clear();
                                        let y_src = active_decoded.data(0);
                                        let y_copy = y_len.min(y_src.len());
                                        frame.y_plane.extend_from_slice(&y_src[..y_copy]);
                                        if y_copy < y_len {
                                            frame.y_plane.resize(y_len, 0);
                                        }

                                        frame.u_plane.resize(c_row_bytes * half_h, 0);
                                        frame.v_plane.resize(c_row_bytes * half_h, 0);
                                        let uv_stride = active_decoded.stride(1);
                                        let uv_src = active_decoded.data(1);
                                        for row in 0..half_h {
                                            let src_row_start = row * uv_stride;
                                            let dst_row_start = row * c_row_bytes;
                                            if src_row_start + half_w * 4 <= uv_src.len() {
                                                let src_row = &uv_src
                                                    [src_row_start..src_row_start + half_w * 4];
                                                let u_row = &mut frame.u_plane
                                                    [dst_row_start..dst_row_start + c_row_bytes];
                                                let v_row = &mut frame.v_plane
                                                    [dst_row_start..dst_row_start + c_row_bytes];
                                                for col in 0..half_w {
                                                    let s = col * 4;
                                                    let d = col * 2;
                                                    u_row[d] = src_row[s];
                                                    u_row[d + 1] = src_row[s + 1];
                                                    v_row[d] = src_row[s + 2];
                                                    v_row[d + 1] = src_row[s + 3];
                                                }
                                            }
                                        }
                                    } else {
                                        let is_planar_420 = matches!(
                                            pix_fmt,
                                            ffmpeg_next::format::Pixel::YUV420P
                                                | ffmpeg_next::format::Pixel::YUVJ420P
                                                | ffmpeg_next::format::Pixel::YUV420P10LE
                                                | ffmpeg_next::format::Pixel::YUV420P12LE
                                        ) && active_decoded.planes() >= 3;

                                        let mut converted = ffmpeg_next::frame::Video::empty();
                                        let src_frame: &ffmpeg_next::frame::Video = if is_planar_420
                                        {
                                            active_decoded
                                        } else {
                                            let w = active_decoded.width();
                                            let h = active_decoded.height();
                                            let fmt = pix_fmt;
                                            let needs_new_scaler =
                                                scaler.as_ref().is_none_or(|(sf, sw, sh, _)| {
                                                    *sf != fmt || *sw != w || *sh != h
                                                });
                                            if needs_new_scaler {
                                                scaler =
                                                    ffmpeg_next::software::scaling::Context::get(
                                                        fmt,
                                                        w,
                                                        h,
                                                        ffmpeg_next::format::Pixel::YUV420P,
                                                        w,
                                                        h,
                                                        ffmpeg_next::software::scaling::Flags::BILINEAR,
                                                    )
                                                    .ok()
                                                    .map(|ctx| (fmt, w, h, ctx));
                                            }
                                            if let Some((_, _, _, ref mut sws)) = scaler
                                                && sws.run(active_decoded, &mut converted).is_ok()
                                                && converted.planes() >= 3
                                            {
                                                &converted
                                            } else {
                                                let _ = free_video_frame_tx.try_send(frame);
                                                continue;
                                            }
                                        };

                                        let format_name = format!("{:?}", src_frame.format());
                                        frame.bit_depth = if format_name.contains("10LE") {
                                            10
                                        } else if format_name.contains("12LE") {
                                            12
                                        } else {
                                            8
                                        };

                                        frame.y_stride = src_frame.stride(0);
                                        frame.u_stride = src_frame.stride(1);
                                        frame.v_stride = src_frame.stride(2);

                                        let height = src_frame.height() as usize;
                                        let y_len = frame.y_stride * height;
                                        let u_len = frame.u_stride * (height / 2);
                                        let v_len = frame.v_stride * (height / 2);

                                        frame.y_plane.clear();
                                        let y_plane = src_frame.data(0);
                                        let y_copy_len = y_len.min(y_plane.len());
                                        frame.y_plane.extend_from_slice(&y_plane[..y_copy_len]);
                                        if y_copy_len < y_len {
                                            frame.y_plane.resize(y_len, 0);
                                        }

                                        frame.u_plane.clear();
                                        let u_plane = src_frame.data(1);
                                        let u_copy_len = u_len.min(u_plane.len());
                                        frame.u_plane.extend_from_slice(&u_plane[..u_copy_len]);
                                        if u_copy_len < u_len {
                                            frame.u_plane.resize(u_len, 0);
                                        }

                                        frame.v_plane.clear();
                                        let v_plane = src_frame.data(2);
                                        let v_copy_len = v_len.min(v_plane.len());
                                        frame.v_plane.extend_from_slice(&v_plane[..v_copy_len]);
                                        if v_copy_len < v_len {
                                            frame.v_plane.resize(v_len, 0);
                                        }
                                    }

                                    let frame_work_us = transfer_start.elapsed().as_micros() as f32;
                                    if let Err(crossbeam_channel::TrySendError::Full(f)) =
                                        video_frame_tx.try_send(frame)
                                    {
                                        let _ = free_video_frame_tx.try_send(f);
                                    }
                                    if let Ok(mut state) = state_for_video.try_lock() {
                                        let pkt_pct = (video_packet_rx_for_video.len() as f32
                                            / 128.0)
                                            * 100.0;
                                        let frame_pct =
                                            (video_frame_tx.len() as f32 / 16.0) * 100.0;
                                        state.stats.video_buffer_fill_pct =
                                            pkt_pct.max(frame_pct).clamp(0.0, 100.0);
                                        if state.stats.video_fps <= 0.0
                                            && let Some(p_pts) = prev_frame_pts
                                        {
                                            let dt = pts - p_pts;
                                            if (0.001..0.5).contains(&dt) {
                                                state.stats.video_fps = (1.0 / dt) as f32;
                                            }
                                        }
                                        if pts >= 0.0 {
                                            prev_frame_pts = Some(pts);
                                            if let Some(start_pts) = pkt_window_start_pts {
                                                let win_dur = pts - start_pts;
                                                if win_dur >= 0.5 {
                                                    let kbps = ((pkt_bytes_accum as f64 * 8.0)
                                                        / (win_dur * 1000.0))
                                                        .round()
                                                        as u32;
                                                    if kbps > 0
                                                        && state.stats.video_bitrate_kbps.is_none()
                                                    {
                                                        state.stats.video_bitrate_kbps = Some(kbps);
                                                    }
                                                    pkt_window_start_pts = Some(pts);
                                                    pkt_bytes_accum = 0;
                                                }
                                            } else {
                                                pkt_window_start_pts = Some(pts);
                                                pkt_bytes_accum = 0;
                                            }
                                        }
                                        if state.stats.bitstream_active {
                                            state.stats.decode_us =
                                                state.stats.decode_us * 0.9 + frame_work_us * 0.1;
                                        }
                                    }
                                }
                            }
                        }
                        let _ = decode_start;
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        }
    });

    (video_packet_tx, video_packet_rx)
}

#[allow(clippy::too_many_arguments)]
fn spawn_media_decoder_threads(
    mut audio_source: Box<dyn AudioSource>,
    shared_state: Arc<Mutex<AppState>>,
    stop_token: Arc<std::sync::atomic::AtomicBool>,
    ready_tx: Sender<AudioChunk>,
    ready_rx: Receiver<AudioChunk>,
    free_tx: Sender<AudioChunk>,
    free_rx: Receiver<AudioChunk>,
    hardware_channels: usize,
    sample_rate: u32,
    chunk_frames: usize,
    pool_size: usize,
    prime_chunks: usize,
    _current_path: &str,
) {
    #[cfg(target_os = "android")]
    let (_video_packet_tx, video_packet_rx) = bounded::<(u64, ())>(1);

    #[cfg(target_os = "android")]
    if audio_source.has_video_stream() {
        crate::android_video::decoder::start_android_video_thread(
            _current_path,
            shared_state.clone(),
            stop_token.clone(),
        );
    }

    #[cfg(not(target_os = "android"))]
    let video_packet_rx =
        if let Some((params, time_base, rotation)) = audio_source.take_video_parameters() {
            let (video_packet_tx, video_packet_rx) = spawn_video_decoder_thread(
                params,
                time_base,
                rotation,
                shared_state.clone(),
                stop_token.clone(),
            );
            audio_source.attach_video_queue(video_packet_tx);
            video_packet_rx
        } else {
            let (_tx, rx) = bounded::<(u64, ffmpeg_next::Packet)>(1);
            rx
        };

    for _ in 0..pool_size {
        let _ = free_tx.try_send(AudioChunk {
            samples: vec![0.0; chunk_frames * hardware_channels],
            valid_frames: 0,
            channel_vus: Vec::with_capacity(32),
            left_peak: 0.0,
            right_peak: 0.0,
            current_order: 0,
            current_row: 0,
            bpm: 0,
            speed: 0,
            current_seconds: 0.0,
            current_row_string: String::with_capacity(128),
            tracker_channels: None,
            spatial_channels: 0,
            track_ended: false,
            seek_epoch: 0,
        });
    }

    for _ in 0..prime_chunks {
        if let Ok(mut chunk) = free_rx.try_recv() {
            let (frames_read, _, _) = decode_single_audio_chunk(
                audio_source.as_mut(),
                &mut chunk,
                hardware_channels,
                sample_rate,
                chunk_frames,
            );
            if frames_read > 0 {
                if let Ok(mut state) = shared_state.lock() {
                    if state.bitrate.is_none() {
                        state.bitrate = audio_source.get_bitrate();
                        state.stats.audio_bitrate_kbps = state.bitrate;
                    }
                    chunk.seek_epoch = state.seek_epoch;
                    push_chunk_lookahead_slices(
                        &mut state,
                        &chunk,
                        frames_read,
                        hardware_channels,
                        sample_rate,
                    );
                }
                chunk.track_ended = false;
                let _ = ready_tx.try_send(chunk);
            } else {
                let _ = free_tx.try_send(chunk);
                break;
            }
        }
    }

    let state_for_decoder = shared_state.clone();
    let ready_rx_for_decoder = ready_rx.clone();
    let free_tx_for_decoder = free_tx.clone();
    let video_rx_for_decoder = video_packet_rx.clone();
    let stop_token_decoder = stop_token.clone();

    std::thread::spawn(move || {
        let mut just_seeked = false;
        while !stop_token_decoder.load(std::sync::atomic::Ordering::Relaxed) {
            let mut chunk = loop {
                if stop_token_decoder.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                if let Ok(mut state) = state_for_decoder.try_lock()
                    && process_decoder_state_requests(
                        &mut state,
                        audio_source.as_mut(),
                        &ready_rx_for_decoder,
                        &free_tx_for_decoder,
                        || {
                            while video_rx_for_decoder.try_recv().is_ok() {}
                        },
                    )
                {
                    just_seeked = true;
                }
                match free_rx.recv_timeout(std::time::Duration::from_millis(10)) {
                    Ok(c) => break c,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
            };

            let (frames_read, decode_elapsed, clips) = decode_single_audio_chunk(
                audio_source.as_mut(),
                &mut chunk,
                hardware_channels,
                sample_rate,
                chunk_frames,
            );

            let current_artist = audio_source.get_artist();
            let current_bitrate = audio_source.get_bitrate();
            if let Ok(mut state) = state_for_decoder.lock() {
                if just_seeked {
                    if frames_read > 0
                        && (state.current_seconds - chunk.current_seconds).abs() < 0.5
                    {
                        state.current_seconds = chunk.current_seconds;
                    }
                    just_seeked = false;
                }
                chunk.seek_epoch = state.seek_epoch;
                if state.bitrate.is_none() && current_bitrate.is_some() {
                    state.bitrate = current_bitrate;
                    state.stats.audio_bitrate_kbps = current_bitrate;
                }
                if current_artist != "Unknown" && !current_artist.is_empty() {
                    if let Some((artist_part, title_part)) = current_artist.split_once(" - ") {
                        if state.artist != artist_part || state.song_title != title_part {
                            state.artist = artist_part.to_string();
                            state.song_title = title_part.to_string();
                        }
                    } else if state.artist != current_artist {
                        state.artist = current_artist.clone();
                    }
                }
                state.stats.decode_us = state.stats.decode_us * 0.9 + decode_elapsed * 0.1;
                let fill_pct = (ready_rx_for_decoder.len() as f32 / pool_size as f32) * 100.0;
                state.stats.audio_buffer_fill_pct = fill_pct;
                let pkt_pct = (video_rx_for_decoder.len() as f32 / 128.0) * 100.0;
                let frame_pct = state
                    .video_frame_rx
                    .as_ref()
                    .map(|rx| (rx.len() as f32 / 16.0) * 100.0)
                    .unwrap_or(0.0);
                state.stats.video_buffer_fill_pct = pkt_pct.max(frame_pct).clamp(0.0, 100.0);
                state.stats.clipping_events += clips;
                push_chunk_lookahead_slices(
                    &mut state,
                    &chunk,
                    frames_read,
                    hardware_channels,
                    sample_rate,
                );
            }

            let mut chunk_to_send = chunk;
            loop {
                if stop_token_decoder.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let mut seeked = false;
                if let Ok(mut state) = state_for_decoder.try_lock() {
                    seeked = process_decoder_state_requests(
                        &mut state,
                        audio_source.as_mut(),
                        &ready_rx_for_decoder,
                        &free_tx_for_decoder,
                        || {
                            while video_rx_for_decoder.try_recv().is_ok() {}
                        },
                    );
                }
                if seeked {
                    just_seeked = true;
                    let _ = free_tx_for_decoder.try_send(chunk_to_send);
                    break;
                }
                match ready_tx.send_timeout(chunk_to_send, std::time::Duration::from_millis(10)) {
                    Ok(_) => break,
                    Err(crossbeam_channel::SendTimeoutError::Timeout(c)) => {
                        chunk_to_send = c;
                    }
                    Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                        return;
                    }
                }
            }

            if frames_read == 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    });
}

fn run_dummy(
    audio_source: Box<dyn AudioSource>,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    sample_rate: u32,
    window_size: usize,
    current_path: &str,
) -> Result<PlaybackHandle> {
    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_token_clone = stop_token.clone();
    let hardware_channels = 2;
    let chunk_frames = 1024;
    let pool_size = ((sample_rate as usize * 8) / chunk_frames).clamp(384, 1024);

    let (ready_tx, ready_rx) = bounded::<AudioChunk>(pool_size);
    let (free_tx, free_rx) = unbounded::<AudioChunk>();

    spawn_media_decoder_threads(
        audio_source,
        shared_state.clone(),
        stop_token.clone(),
        ready_tx,
        ready_rx.clone(),
        free_tx.clone(),
        free_rx,
        hardware_channels,
        sample_rate,
        chunk_frames,
        pool_size,
        0,
        current_path,
    );

    let shared_state_dummy = shared_state.clone();

    let thread_handle = std::thread::spawn(move || {
        let mut fft_buffer: Vec<f32> = vec![0.0; window_size];
        let mut channel_fft_buffers: Vec<Vec<f32>> =
            vec![vec![0.0; window_size]; hardware_channels.max(12)];
        let mut fft_index = 0;
        let recycle_rx = dsp_recycle_channel().1.clone();

        let mut was_paused = false;
        let mut latest_seek_epoch = 0u64;
        let mut upmix_cfg = crate::state::SurroundUpmixConfig::default();
        let mut upmixer_dsp = SurroundUpmixerDsp::new(sample_rate);
        let mut upmix_peak_vus = [0.0f32; 12];

        while !stop_token_clone.load(std::sync::atomic::Ordering::Relaxed) {
            if let Ok(state) = shared_state_dummy.try_lock() {
                was_paused = state.is_paused;
                latest_seek_epoch = state.seek_epoch;
                upmix_cfg = state.surround_upmix;
            }

            if was_paused {
                if let Ok(mut state) = shared_state_dummy.try_lock() {
                    state.raw_channel_vus.fill(0.0);
                    state.raw_spectrum_data.fill(0.0);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }

            match ready_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(chunk) => {
                    if chunk.seek_epoch < latest_seek_epoch {
                        let _ = free_tx.try_send(chunk);
                        continue;
                    }
                    if chunk.track_ended
                        && let Ok(mut state) = shared_state_dummy.try_lock()
                    {
                        state.track_ended = true;
                    }

                    // Process chunk for FFT and VU levels (dummy device is 2ch stereo, so surround upmix is bypassed)
                    let to_copy = chunk.valid_frames;
                    let upmix_active =
                        upmix_cfg.is_active_for(chunk.spatial_channels, hardware_channels as i32);
                    if upmix_active {
                        upmixer_dsp.sync_config(&upmix_cfg, sample_rate);
                        upmix_peak_vus = [0.0f32; 12];
                        let src_ch = (chunk.spatial_channels as usize).max(1);
                        let active_ch = hardware_channels.min(12);
                        for i in 0..to_copy {
                            let frame_slice =
                                &chunk.samples[i * hardware_channels..(i + 1) * hardware_channels];
                            let out_channels = upmixer_dsp.process_multichannel_frame_for_hardware(
                                frame_slice,
                                src_ch,
                                &upmix_cfg,
                                hardware_channels,
                            );
                            for c in 0..active_ch {
                                let s = out_channels[c];
                                upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                channel_fft_buffers[c][fft_index] = s;
                            }
                            let mono = (out_channels[0] + out_channels[1]) * 0.5;
                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;
                        }
                    } else {
                        for i in 0..to_copy {
                            let mut mono = 0.0;
                            for (c, buf) in channel_fft_buffers
                                .iter_mut()
                                .enumerate()
                                .take(hardware_channels)
                            {
                                let sample =
                                    chunk.samples[i * hardware_channels + c].clamp(-1.0, 1.0);
                                let ch_weight = match c {
                                    0 | 1 => {
                                        if hardware_channels == 1 {
                                            1.0
                                        } else {
                                            0.5
                                        }
                                    }
                                    2 => std::f32::consts::FRAC_1_SQRT_2,
                                    _ => 0.25,
                                };
                                mono += sample * ch_weight;
                                buf[fft_index] = sample;
                            }
                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;
                        }
                    }

                    let (dsp_channels, vu_slice): (usize, &[f32]) = if upmix_active {
                        let active_ch = hardware_channels.min(12);
                        (active_ch, &upmix_peak_vus[..active_ch])
                    } else {
                        (
                            hardware_channels.min(chunk.channel_vus.len().max(1)),
                            &chunk.channel_vus,
                        )
                    };

                    let msg = build_recycled_dsp_message(
                        &recycle_rx,
                        &fft_buffer,
                        &channel_fft_buffers,
                        fft_index,
                        window_size,
                        dsp_channels,
                        vu_slice,
                        chunk.current_order,
                        chunk.current_row,
                        chunk.bpm,
                        chunk.speed,
                        chunk.current_seconds,
                        &chunk.current_row_string,
                        chunk.seek_epoch,
                        true,
                    );

                    let _ = tx.try_send(msg);

                    if let Ok(mut state) = shared_state_dummy.try_lock()
                        && chunk.seek_epoch == state.seek_epoch
                    {
                        state.current_seconds = chunk.current_seconds;
                    }

                    // Sleep to simulate the playback duration of this chunk
                    let chunk_duration_ms = (to_copy as f64 / sample_rate as f64 * 1000.0) as u64;
                    if chunk_duration_ms > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(chunk_duration_ms));
                    }

                    let _ = free_tx.try_send(chunk);
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    });

    Ok(PlaybackHandle::Dummy(Some(thread_handle), stop_token))
}

#[cfg(target_os = "linux")]
#[allow(clippy::needless_range_loop)]
fn run_linux_pipewire_mpegh_714(
    audio_source_opt: &mut Option<Box<dyn AudioSource>>,
    shared_state: Arc<Mutex<AppState>>,
    file_path: &str,
) -> Result<PlaybackHandle> {
    use std::io::Write;

    let mut mpegh_encoder = crate::mpegh::Mpegh714Encoder::new(4, true)?;
    let mut child = std::process::Command::new("pw-play")
        .arg("--properties=node.passthrough=true")
        .arg("-f")
        .arg("s16")
        .arg("-r")
        .arg("192000")
        .arg("-c")
        .arg("2")
        .arg("--raw")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn pw-play for MPEG-H 3D Audio 7.1.4 passthrough")?;

    std::thread::sleep(std::time::Duration::from_millis(80));
    if let Ok(Some(status)) = child.try_wait() {
        return Err(anyhow::anyhow!(
            "pw-play exited immediately with status: {}",
            status
        ));
    }

    let mut pw_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to open pw-play stdin"))?;

    let mut audio_source = audio_source_opt
        .take()
        .ok_or_else(|| anyhow::anyhow!("No audio source available"))?;

    let sample_rate = crate::mpegh::MPEGH_SAMPLE_RATE;
    let hardware_channels = 12usize;
    let max_frequency = {
        let mut state = shared_state.lock().unwrap();
        populate_state_from_audio_source(
            &mut state,
            audio_source.as_mut(),
            hardware_channels as i32,
            sample_rate,
        );
        state.stats.mpegh_bitstream_active = true;
        state.stats.mpegh_carrier_info = "PipeWire IEC 61937-13 MPEG-H (2ch @ 192 kHz)".to_string();
        state.max_frequency
    };

    let (tx, rx) = bounded::<DspMessage>(32);
    let window_size = calculate_power_of_two_window_size(sample_rate);
    spawn_dsp_thread(
        rx,
        shared_state.clone(),
        sample_rate,
        max_frequency,
        window_size,
    );

    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let chunk_frames = 1024;
    let pool_size = ((sample_rate as usize * 8) / chunk_frames).clamp(384, 1024);
    let (ready_tx, ready_rx) = bounded::<AudioChunk>(pool_size);
    let (free_tx, free_rx) = unbounded::<AudioChunk>();

    spawn_media_decoder_threads(
        audio_source,
        shared_state.clone(),
        stop_token.clone(),
        ready_tx,
        ready_rx.clone(),
        free_tx.clone(),
        free_rx,
        hardware_channels,
        sample_rate,
        chunk_frames,
        pool_size,
        4,
        file_path,
    );

    let stop_token_thread = stop_token.clone();
    let shared_state_thread = shared_state.clone();
    let thread_handle = std::thread::spawn(move || {
        let mut fft_buffer = vec![0.0f32; window_size];
        let mut channel_fft_buffers = vec![vec![0.0f32; window_size]; 12];
        let mut fft_index = 0usize;
        let recycle_rx = dsp_recycle_channel().1.clone();
        let mut upmixer_dsp = SurroundUpmixerDsp::new(sample_rate);
        let mut upmix_peak_vus = [0.0f32; 12];
        let (mut latest_seek_epoch, mut upmix_cfg) = {
            let s = shared_state_thread.lock().unwrap();
            (s.seek_epoch, s.surround_upmix)
        };
        let silence_burst = vec![0u8; 4096 * 4];
        let mut iec_byte_queue: std::collections::VecDeque<u8> =
            std::collections::VecDeque::with_capacity(65536);

        while !stop_token_thread.load(std::sync::atomic::Ordering::Relaxed) {
            let mut is_paused = false;
            if let Ok(state) = shared_state_thread.try_lock() {
                is_paused = state.is_paused;
                latest_seek_epoch = state.seek_epoch;
                upmix_cfg = state.surround_upmix;
            }

            if is_paused {
                if let Ok(mut state) = shared_state_thread.try_lock() {
                    state.raw_channel_vus.fill(0.0);
                    state.raw_spectrum_data.fill(0.0);
                }
                if pw_stdin.write_all(&silence_burst).is_err() {
                    break;
                }
                continue;
            }

            match ready_rx.recv_timeout(std::time::Duration::from_millis(25)) {
                Ok(chunk) => {
                    if chunk.seek_epoch < latest_seek_epoch {
                        let _ = free_tx.try_send(chunk);
                        continue;
                    }
                    if chunk.seek_epoch > latest_seek_epoch {
                        latest_seek_epoch = chunk.seek_epoch;
                    }
                    if chunk.track_ended
                        && let Ok(mut state) = shared_state_thread.try_lock()
                    {
                        state.track_ended = true;
                    }

                    let upmix_active = upmix_cfg.is_active_for_hardware(12);
                    if upmix_active {
                        upmixer_dsp.sync_config(&upmix_cfg, sample_rate);
                    }
                    let src_ch = (chunk.spatial_channels as usize).max(1);
                    let target_upmix = upmix_active && src_ch < 12;
                    let use_upmixer =
                        upmixer_dsp.is_blending_or_active(target_upmix) && src_ch < 12;

                    for i in 0..chunk.valid_frames {
                        let frame_slice = &chunk.samples[i * 12..(i + 1) * 12];
                        if use_upmixer {
                            let out_channels = upmixer_dsp
                                .process_multichannel_frame_for_hardware_blended(
                                    frame_slice,
                                    src_ch,
                                    &upmix_cfg,
                                    12,
                                    target_upmix,
                                );
                            for c in 0..12 {
                                let s = out_channels[c].clamp(-1.0, 1.0);
                                upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                channel_fft_buffers[c][fft_index] = s;
                            }
                            let mono = (out_channels[0] + out_channels[1]) * 0.5;
                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;

                            if let Ok(true) = mpegh_encoder
                                .push_rusttracker_714_frame(&out_channels, &mut iec_byte_queue)
                            {
                                let (s1, s2) = iec_byte_queue.as_slices();
                                if pw_stdin.write_all(s1).is_err()
                                    || pw_stdin.write_all(s2).is_err()
                                {
                                    break;
                                }
                                iec_byte_queue.clear();
                            }
                        } else {
                            let mut cicp_frame = [0.0f32; 12];
                            let mut mono = 0.0f32;
                            for c in 0..12 {
                                let s = frame_slice[c].clamp(-1.0, 1.0);
                                cicp_frame[c] = s;
                                let ch_weight = match c {
                                    0 | 1 => 0.5,
                                    2 => std::f32::consts::FRAC_1_SQRT_2,
                                    _ => 0.25,
                                };
                                mono += s * ch_weight;
                                channel_fft_buffers[c][fft_index] = s;
                            }
                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;

                            if let Ok(true) =
                                mpegh_encoder.push_cicp19_frame(&cicp_frame, &mut iec_byte_queue)
                            {
                                let (s1, s2) = iec_byte_queue.as_slices();
                                if pw_stdin.write_all(s1).is_err()
                                    || pw_stdin.write_all(s2).is_err()
                                {
                                    break;
                                }
                                iec_byte_queue.clear();
                            }
                        }
                    }

                    let (dsp_channels, vu_slice): (usize, &[f32]) = if upmix_active && src_ch < 12 {
                        (12, &upmix_peak_vus)
                    } else {
                        (chunk.channel_vus.len().max(1), &chunk.channel_vus)
                    };
                    let msg = build_recycled_dsp_message(
                        &recycle_rx,
                        &fft_buffer,
                        &channel_fft_buffers,
                        fft_index,
                        window_size,
                        dsp_channels,
                        vu_slice,
                        chunk.current_order,
                        chunk.current_row,
                        chunk.bpm,
                        chunk.speed,
                        chunk.current_seconds,
                        &chunk.current_row_string,
                        chunk.seek_epoch,
                        true,
                    );
                    upmix_peak_vus = [0.0f32; 12];
                    let _ = tx.try_send(msg);
                    let _ = free_tx.try_send(chunk);
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }

        drop(pw_stdin);
        let _ = child.kill();
        let _ = child.wait();
    });

    Ok(PlaybackHandle::Dummy(Some(thread_handle), stop_token))
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_wasapi_exclusive_multichannel(
    selected_device_name: Option<&str>,
    audio_source_opt: &mut Option<Box<dyn AudioSource>>,
    shared_state: Arc<Mutex<AppState>>,
    target_rate: u32,
    target_channels: u16,
    surround_upmix_enabled: bool,
    mpegh_bitstream: bool,
    _mpegh_user_overridden: bool,
    file_path: &str,
) -> Result<PlaybackHandle> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY};
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::core::GUID;

    const WAIT_OBJECT_0: u32 = 0;
    const KSDATAFORMAT_SUBTYPE_PCM: GUID = GUID {
        data1: 0x00000001,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };
    const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID {
        data1: 0x00000003,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };
    const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    #[derive(Clone, Copy, Debug)]
    enum WasapiPcmFormat {
        I24In32,
        F32,
        I32,
        I16,
    }

    impl WasapiPcmFormat {
        fn container_bits(self) -> u16 {
            match self {
                Self::I24In32 | Self::F32 | Self::I32 => 32,
                Self::I16 => 16,
            }
        }
        fn valid_bits(self) -> u16 {
            match self {
                Self::I24In32 => 24,
                Self::F32 | Self::I32 => 32,
                Self::I16 => 16,
            }
        }
        fn sub_format(self) -> GUID {
            match self {
                Self::F32 => KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
                Self::I24In32 | Self::I32 | Self::I16 => KSDATAFORMAT_SUBTYPE_PCM,
            }
        }
        fn bytes_per_sample(self) -> usize {
            (self.container_bits() / 8) as usize
        }
        fn label(self) -> &'static str {
            match self {
                Self::I24In32 => "24-bit PCM (in 32-bit container)",
                Self::F32 => "32-bit IEEE Float",
                Self::I32 => "32-bit PCM",
                Self::I16 => "16-bit PCM",
            }
        }
    }

    fn build_wave_format(
        channels: u16,
        rate: u32,
        channel_mask: u32,
        fmt: WasapiPcmFormat,
    ) -> WAVEFORMATEXTENSIBLE {
        let container_bits = fmt.container_bits();
        let block_align = channels * (container_bits / 8);
        let avg_bytes = rate * block_align as u32;
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: 0xFFFE,
                nChannels: channels,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: avg_bytes,
                nBlockAlign: block_align,
                wBitsPerSample: container_bits,
                cbSize: 22,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: fmt.valid_bits(),
            },
            dwChannelMask: channel_mask,
            SubFormat: fmt.sub_format(),
        }
    }

    fn get_wasapi_device_name(device: &IMMDevice) -> Option<String> {
        unsafe {
            let store = device.OpenPropertyStore(STGM_READ).ok()?;
            let propvar = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
            if propvar.Anonymous.Anonymous.vt == VT_LPWSTR {
                let pwstr = propvar.Anonymous.Anonymous.Anonymous.pwszVal;
                if !pwstr.is_null() {
                    pwstr.to_string().ok()
                } else {
                    None
                }
            } else {
                let s = propvar.to_string();
                if s.is_empty() { None } else { Some(s) }
            }
        }
    }

    fn try_open_exclusive_client(
        device: &IMMDevice,
        channels: u16,
        rate: u32,
        channel_mask: u32,
        fmt: WasapiPcmFormat,
    ) -> Result<(IAudioClient, IAudioRenderClient, HANDLE, u32)> {
        const AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED_HR: i32 = -2004287463i32; // 0x88890019

        let wf = build_wave_format(channels, rate, channel_mask, fmt);
        let wf_ptr = &wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX;

        let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        if unsafe { audio_client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, wf_ptr, None) }
            != windows::core::HRESULT(0)
        {
            return Err(anyhow::anyhow!(
                "Format unsupported: {}ch @ {} Hz ({})",
                channels,
                rate,
                fmt.label()
            ));
        }

        let mut default_period = 0i64;
        let mut min_period = 0i64;
        unsafe {
            let _ = audio_client.GetDevicePeriod(Some(&mut default_period), Some(&mut min_period));
        }
        let mut buffer_duration = if default_period > 0 {
            default_period.max(200_000)
        } else if min_period > 0 {
            min_period.max(200_000)
        } else {
            200_000
        };

        let init_res = unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                buffer_duration,
                buffer_duration,
                wf_ptr,
                None,
            )
        };

        let active_client = match init_res {
            Ok(()) => audio_client,
            Err(e) if e.code().0 == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED_HR => {
                let aligned_frames = unsafe { audio_client.GetBufferSize()? };
                buffer_duration =
                    ((10_000_000.0 * aligned_frames as f64 / rate as f64) + 0.5) as i64;
                drop(audio_client);
                let retry_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
                unsafe {
                    retry_client
                        .Initialize(
                            AUDCLNT_SHAREMODE_EXCLUSIVE,
                            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                            buffer_duration,
                            buffer_duration,
                            wf_ptr,
                            None,
                        )
                        .map_err(|e| anyhow::anyhow!(e))?;
                }
                retry_client
            }
            Err(e) => return Err(anyhow::anyhow!(e)),
        };

        let event = unsafe { CreateEventW(None, false, false, None)? };
        if let Err(e) = unsafe { active_client.SetEventHandle(event) } {
            unsafe {
                let _ = CloseHandle(event);
            }
            return Err(anyhow::anyhow!(e));
        }
        let buffer_frames = unsafe { active_client.GetBufferSize()? };
        let render_client: IAudioRenderClient = match unsafe { active_client.GetService() } {
            Ok(rc) => rc,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(event);
                }
                return Err(anyhow::anyhow!(e));
            }
        };
        Ok((active_client, render_client, event, buffer_frames))
    }

    #[allow(dead_code)]
    fn device_supports_multichannel_lpcm(device: &IMMDevice) -> bool {
        let Ok(audio_client) = (unsafe { device.Activate::<IAudioClient>(CLSCTX_ALL, None) })
        else {
            return false;
        };
        for &(ch, mask) in &[(8u16, 0x63Fu32), (6u16, 0x3Fu32)] {
            for &fmt in &[
                WasapiPcmFormat::I24In32,
                WasapiPcmFormat::F32,
                WasapiPcmFormat::I16,
            ] {
                let wf = build_wave_format(ch, 48_000, mask, fmt);
                let wf_ptr = &wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX;
                if unsafe {
                    audio_client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, wf_ptr, None)
                } == windows::core::HRESULT(0)
                {
                    return true;
                }
            }
        }
        false
    }

    /// Negotiate a WASAPI Exclusive Mode IEC 61937-13 bitstream carrier for 12-channel (`7.1.4` / `11.1`)
    /// MPEG-H 3D Audio (`CICP Layout 19`) to an HDMI AVR (such as the Denon AVR-X3800H).
    ///
    /// First probes dedicated MPEG-H IEC 61937 subtype GUIDs (`00000019-0cea-...` and `0000000f-0cea-...`).
    /// When the endpoint is a multichannel HDMI AVR (or the user explicitly enabled MPEG-H 11.1) and the
    /// Windows GPU HDMI driver only registers legacy CEA-861-D/E IEC 61937 subtypes in `IsFormatSupported`,
    /// also probes high-bitrate 192 kHz IEC 61937 carriers (`E-AC-3` 2ch @ 192 kHz = 6.144 Mbps `rate_factor = 4`,
    /// `DTS-HD` / `MLP` 8ch @ 192 kHz = 24.576 Mbps `rate_factor = 16`), because the AVR decodes the stream
    /// from the embedded IEC 61937-13 `Pa/Pb/Pc` burst preamble (`Pc = 0x0019`).
    fn try_open_exclusive_mpegh_iec61937_client(
        device: &IMMDevice,
        allow_cea_carrier_fallback: bool,
    ) -> Result<(IAudioClient, IAudioRenderClient, HANDLE, u32, usize, String)> {
        const AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED_HR: i32 = -2004287463i32; // 0x88890019

        // CEA-861.3 / IEC 61937-13 MPEG-H 3D Audio and compatible 192 kHz HDMI IEC 61937 carriers:
        // (guid_data1, channels, rate_hz, channel_mask, rate_factor, carrier_label, requires_avr_fallback)
        let candidates: &[(u32, u16, u32, u32, usize, &str, bool)] = &[
            (
                0x00000019,
                2,
                192_000,
                0x3,
                4,
                "IEC 61937-13 MPEG-H (2ch @ 192 kHz)",
                false,
            ),
            (
                0x0000000f,
                2,
                192_000,
                0x3,
                4,
                "CEA-861-G MPEG-H 3D Audio (2ch @ 192 kHz)",
                false,
            ),
            (
                0x00000019,
                8,
                192_000,
                0x63F,
                16,
                "IEC 61937-13 MPEG-H HBR (8ch @ 192 kHz)",
                false,
            ),
            (
                0x0000000f,
                8,
                192_000,
                0x63F,
                16,
                "CEA-861-G MPEG-H 3D Audio HBR (8ch @ 192 kHz)",
                false,
            ),
            (
                0x0000000a,
                2,
                192_000,
                0x3,
                4,
                "IEC 61937-13 over HDMI 192 kHz 2ch carrier",
                true,
            ),
            (
                0x0000000b,
                8,
                192_000,
                0x63F,
                16,
                "IEC 61937-13 over HDMI 192 kHz 8ch HBR carrier",
                true,
            ),
            (
                0x0000000c,
                8,
                192_000,
                0x63F,
                16,
                "IEC 61937-13 over HDMI 192 kHz 8ch MAT/MLP carrier",
                true,
            ),
        ];

        #[repr(C, packed)]
        struct WaveFormatExtensibleIec61937 {
            format_ext: WAVEFORMATEXTENSIBLE,
            dw_encoded_samples_per_sec: u32,
            dw_encoded_channel_count: u32,
            dw_average_bytes_per_sec: u32,
        }

        let mut last_err = anyhow::anyhow!("No IEC 61937-13 MPEG-H carrier supported on endpoint");

        for &(
            guid_data1,
            carrier_ch,
            carrier_rate,
            carrier_mask,
            rate_factor,
            label,
            needs_fallback,
        ) in candidates
        {
            if needs_fallback && !allow_cea_carrier_fallback {
                continue;
            }
            let sub_format = GUID {
                data1: guid_data1,
                data2: 0x0cea,
                data3: 0x0010,
                data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
            };
            let block_align = carrier_ch * 2; // 16-bit IEC 61937 words
            let avg_bytes = carrier_rate * block_align as u32;

            // Try both WAVEFORMATEXTENSIBLE_IEC61937 (cbSize = 34, standard for >48 kHz HDMI IEC 61937)
            // and legacy WAVEFORMATEXTENSIBLE (cbSize = 22).
            for &use_iec_ext in &[true, false] {
                let wf_ext = WAVEFORMATEXTENSIBLE {
                    Format: WAVEFORMATEX {
                        wFormatTag: 0xFFFE,
                        nChannels: carrier_ch,
                        nSamplesPerSec: carrier_rate,
                        nAvgBytesPerSec: avg_bytes,
                        nBlockAlign: block_align,
                        wBitsPerSample: 16,
                        cbSize: if use_iec_ext { 34 } else { 22 },
                    },
                    Samples: WAVEFORMATEXTENSIBLE_0 {
                        wValidBitsPerSample: 16,
                    },
                    dwChannelMask: carrier_mask,
                    SubFormat: sub_format,
                };
                let wf_iec = WaveFormatExtensibleIec61937 {
                    format_ext: wf_ext,
                    dw_encoded_samples_per_sec: carrier_rate,
                    dw_encoded_channel_count: carrier_ch as u32,
                    dw_average_bytes_per_sec: avg_bytes,
                };
                let wf_ptr = if use_iec_ext {
                    &wf_iec as *const WaveFormatExtensibleIec61937 as *const WAVEFORMATEX
                } else {
                    &wf_ext as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX
                };

                let audio_client: IAudioClient = match unsafe { device.Activate(CLSCTX_ALL, None) }
                {
                    Ok(c) => c,
                    Err(e) => {
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };
                if unsafe {
                    audio_client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, wf_ptr, None)
                } != windows::core::HRESULT(0)
                {
                    continue;
                }

                let mut default_period = 0i64;
                let mut min_period = 0i64;
                unsafe {
                    let _ = audio_client
                        .GetDevicePeriod(Some(&mut default_period), Some(&mut min_period));
                }
                let mut buffer_duration = if min_period > 0 {
                    min_period
                } else if default_period > 0 {
                    default_period.max(100_000)
                } else {
                    100_000
                };

                let init_res = unsafe {
                    audio_client.Initialize(
                        AUDCLNT_SHAREMODE_EXCLUSIVE,
                        AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                        buffer_duration,
                        buffer_duration,
                        wf_ptr,
                        None,
                    )
                };

                let active_client = match init_res {
                    Ok(()) => audio_client,
                    Err(e)
                        if e.code().0 == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED_HR
                            || e.code() == windows::core::HRESULT(0x80070057u32 as i32) =>
                    {
                        let aligned_frames = match unsafe { audio_client.GetBufferSize() } {
                            Ok(f) if f > 0 => f,
                            _ => {
                                last_err = anyhow::anyhow!(e);
                                continue;
                            }
                        };
                        buffer_duration = ((10_000_000.0 * aligned_frames as f64
                            / carrier_rate as f64)
                            + 0.5) as i64;
                        drop(audio_client);
                        let retry_client: IAudioClient =
                            match unsafe { device.Activate(CLSCTX_ALL, None) } {
                                Ok(c) => c,
                                Err(e2) => {
                                    last_err = anyhow::anyhow!(e2);
                                    continue;
                                }
                            };
                        if let Err(e2) = unsafe {
                            retry_client.Initialize(
                                AUDCLNT_SHAREMODE_EXCLUSIVE,
                                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                                buffer_duration,
                                buffer_duration,
                                wf_ptr,
                                None,
                            )
                        } {
                            last_err = anyhow::anyhow!(e2);
                            continue;
                        }
                        retry_client
                    }
                    Err(e) => {
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };

                let event = match unsafe { CreateEventW(None, false, false, None) } {
                    Ok(ev) => ev,
                    Err(e) => {
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };
                if let Err(e) = unsafe { active_client.SetEventHandle(event) } {
                    unsafe {
                        let _ = CloseHandle(event);
                    }
                    last_err = anyhow::anyhow!(e);
                    continue;
                }
                let buffer_frames = match unsafe { active_client.GetBufferSize() } {
                    Ok(f) => f,
                    Err(e) => {
                        unsafe {
                            let _ = CloseHandle(event);
                        }
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };
                let render_client: IAudioRenderClient = match unsafe { active_client.GetService() }
                {
                    Ok(rc) => rc,
                    Err(e) => {
                        unsafe {
                            let _ = CloseHandle(event);
                        }
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };

                return Ok((
                    active_client,
                    render_client,
                    event,
                    buffer_frames,
                    rate_factor,
                    label.to_string(),
                ));
            }
        }

        Err(last_err)
    }

    struct SpatialAudio714State {
        _spatial_client: ISpatialAudioClient,
        _fmt_enum: IAudioFormatEnumerator,
        stream: ISpatialAudioObjectRenderStream,
        objects: Vec<ISpatialAudioObject>,
        carrier_label: String,
    }

    /// Opens a 12-channel (`7.1.4` / `11.1`) Windows Spatial Audio (`ISpatialAudioClient` ->
    /// `ISpatialAudioObjectRenderStream`) stream with all 12 static 7.1.4 bed objects
    /// (`FL, FR, FC, LFE, SL, SR, BL, BR, TFL, TFR, TBL, TBR`).
    ///
    /// When "Dolby Atmos for Home Theater" (`DolbyMatEnc.dll`) is active on the endpoint
    /// (`GetMaxDynamicObjectCount() > 0`), Windows Spatial Audio encodes all 12 static bed objects
    /// into Dolby MAT 2.0 over HDMI/eARC so the AVR (e.g. Denon AVR-X3800H via LG OLED C4 eARC)
    /// receives discrete 7.1.4 (11.1) with height channels. If Windows Spatial Sound is Off
    /// (`GetMaxDynamicObjectCount() == 0`) and the Shared Mode `MixFormat` has `< 12` channels,
    /// `AudioDg.exe` would fold the 12 objects down to 2ch Stereo, so we decline and let WASAPI
    /// Exclusive 8ch (7.1) LPCM take over instead.
    fn try_open_spatial_audio_714_client(
        device: &IMMDevice,
        mix_channels: u16,
    ) -> Result<(SpatialAudio714State, HANDLE, u32, u32)> {
        use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
        use windows::Win32::System::Variant::VT_BLOB;
        use windows::core::Interface;

        let sac: ISpatialAudioClient = unsafe {
            device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| anyhow::anyhow!(e))?
        };
        let max_dyn = unsafe { sac.GetMaxDynamicObjectCount().unwrap_or(0) };
        if max_dyn == 0 && mix_channels < 12 {
            return Err(anyhow::anyhow!(
                "Windows Spatial Sound ('Dolby Atmos for Home Theater') is Off on endpoint (GetMaxDynamicObjectCount == 0, MixFormat == {}ch); declining ISpatialAudioClient to avoid stereo folddown",
                mix_channels
            ));
        }

        if let Err(e) = unsafe {
            sac.IsSpatialAudioStreamAvailable(&ISpatialAudioObjectRenderStream::IID, None)
        } {
            return Err(anyhow::anyhow!(
                "ISpatialAudioObjectRenderStream not available on endpoint: {}",
                e
            ));
        }

        let fmt_enum = unsafe {
            sac.GetSupportedAudioObjectFormatEnumerator()
                .map_err(|e| anyhow::anyhow!(e))?
        };
        let fmt_count = unsafe { fmt_enum.GetCount().unwrap_or(0) };
        if fmt_count == 0 {
            return Err(anyhow::anyhow!(
                "ISpatialAudioClient reported 0 supported AudioObject formats"
            ));
        }

        // Prefer 48000 Hz 32-bit float mono object format.
        let mut chosen_wf_ptr: *const WAVEFORMATEX = std::ptr::null();
        let mut chosen_rate = 48000u32;
        for idx in 0..fmt_count {
            if let Ok(wf_ptr) = unsafe { fmt_enum.GetFormat(idx) }
                && !wf_ptr.is_null()
            {
                let wf = unsafe { *wf_ptr };
                if wf.wBitsPerSample == 32 && wf.nChannels == 1 {
                    chosen_wf_ptr = wf_ptr;
                    chosen_rate = wf.nSamplesPerSec;
                    if wf.nSamplesPerSec == 48000 {
                        break;
                    }
                }
            }
        }
        if chosen_wf_ptr.is_null() {
            return Err(anyhow::anyhow!(
                "No 32-bit float mono AudioObject format found in ISpatialAudioClient"
            ));
        }

        let max_frames = unsafe {
            sac.GetMaxFrameCount(chosen_wf_ptr)
                .map_err(|e| anyhow::anyhow!(e))?
        };
        let event =
            unsafe { CreateEventW(None, false, false, None).map_err(|e| anyhow::anyhow!(e))? };

        let mask_714 = AudioObjectType_FrontLeft
            | AudioObjectType_FrontRight
            | AudioObjectType_FrontCenter
            | AudioObjectType_LowFrequency
            | AudioObjectType_SideLeft
            | AudioObjectType_SideRight
            | AudioObjectType_BackLeft
            | AudioObjectType_BackRight
            | AudioObjectType_TopFrontLeft
            | AudioObjectType_TopFrontRight
            | AudioObjectType_TopBackLeft
            | AudioObjectType_TopBackRight;

        let mut params = SpatialAudioObjectRenderStreamActivationParams {
            ObjectFormat: chosen_wf_ptr,
            StaticObjectTypeMask: mask_714,
            MinDynamicObjectCount: 0,
            MaxDynamicObjectCount: 0,
            Category: AudioCategory_Media,
            EventHandle: event,
            NotifyObject: std::mem::ManuallyDrop::new(None),
        };
        let mut pv = std::mem::ManuallyDrop::new(PROPVARIANT::default());
        unsafe {
            (*pv.Anonymous.Anonymous).vt = VT_BLOB;
            (*pv.Anonymous.Anonymous).Anonymous.blob = BLOB {
                cbSize: std::mem::size_of::<SpatialAudioObjectRenderStreamActivationParams>()
                    as u32,
                pBlobData: &mut params as *mut _ as *mut u8,
            };
        }

        let stream: ISpatialAudioObjectRenderStream =
            match unsafe { sac.ActivateSpatialAudioStream(&*pv) } {
                Ok(s) => s,
                Err(e) => {
                    unsafe {
                        let _ = CloseHandle(event);
                    }
                    return Err(anyhow::anyhow!(
                        "ActivateSpatialAudioStream(7.1.4) failed: {}",
                        e
                    ));
                }
            };

        // Activate all 12 static 7.1.4 bed objects in RustTracker's canonical 12-channel order:
        // [0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:BL(RL), 7:BR(RR), 8:TFL, 9:TFR, 10:TBL(TRL), 11:TBR(TRR)]
        let static_types = [
            AudioObjectType_FrontLeft,
            AudioObjectType_FrontRight,
            AudioObjectType_FrontCenter,
            AudioObjectType_LowFrequency,
            AudioObjectType_SideLeft,
            AudioObjectType_SideRight,
            AudioObjectType_BackLeft,
            AudioObjectType_BackRight,
            AudioObjectType_TopFrontLeft,
            AudioObjectType_TopFrontRight,
            AudioObjectType_TopBackLeft,
            AudioObjectType_TopBackRight,
        ];
        let mut objects = Vec::with_capacity(12);
        for obj_type in static_types {
            match unsafe { stream.ActivateSpatialAudioObject(obj_type) } {
                Ok(obj) => objects.push(obj),
                Err(e) => {
                    unsafe {
                        let _ = CloseHandle(event);
                    }
                    return Err(anyhow::anyhow!(
                        "ActivateSpatialAudioObject({:?}) failed: {}",
                        obj_type,
                        e
                    ));
                }
            }
        }

        let carrier_label = if max_dyn > 0 {
            format!(
                "ISpatialAudioClient 7.1.4 (Dolby Atmos / MAT 2.0, {} dynamic objects supported)",
                max_dyn
            )
        } else {
            "ISpatialAudioClient 7.1.4 (Static Bed — enable 'Dolby Atmos for Home Theater' in Windows Sound Settings for eARC heights)".to_string()
        };

        Ok((
            SpatialAudio714State {
                _spatial_client: sac,
                _fmt_enum: fmt_enum,
                stream,
                objects,
                carrier_label,
            },
            event,
            max_frames.max(480),
            chosen_rate,
        ))
    }

    fn try_open_shared_client(
        device: &IMMDevice,
        channels: u16,
        rate: u32,
        channel_mask: u32,
        fmt: WasapiPcmFormat,
    ) -> Result<(IAudioClient, IAudioRenderClient, HANDLE, u32)> {
        let wf = build_wave_format(channels, rate, channel_mask, fmt);
        let wf_ptr = &wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX;

        let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let mut default_period = 0i64;
        let mut min_period = 0i64;
        unsafe {
            let _ = audio_client.GetDevicePeriod(Some(&mut default_period), Some(&mut min_period));
        }
        let buffer_duration = if default_period > 0 {
            default_period.max(200_000)
        } else if min_period > 0 {
            min_period.max(200_000)
        } else {
            200_000
        };

        let stream_flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

        unsafe {
            audio_client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    stream_flags,
                    buffer_duration,
                    0,
                    wf_ptr,
                    None,
                )
                .map_err(|e| anyhow::anyhow!(e))?;
        }

        let event = unsafe { CreateEventW(None, false, false, None)? };
        if let Err(e) = unsafe { audio_client.SetEventHandle(event) } {
            unsafe {
                let _ = CloseHandle(event);
            }
            return Err(anyhow::anyhow!(e));
        }
        let buffer_frames = unsafe { audio_client.GetBufferSize()? };
        let render_client: IAudioRenderClient = match unsafe { audio_client.GetService() } {
            Ok(rc) => rc,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(event);
                }
                return Err(anyhow::anyhow!(e));
            }
        };
        Ok((audio_client, render_client, event, buffer_frames))
    }

    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_token_thread = stop_token.clone();
    let shared_state_thread = shared_state.clone();
    let selected_dev_owned = selected_device_name.map(|s| s.to_string());
    let (init_tx, init_rx) =
        bounded::<Result<(u16, u32, WasapiPcmFormat, String, bool, String)>>(1);
    let (pipe_tx, pipe_rx) =
        bounded::<(Receiver<AudioChunk>, Sender<AudioChunk>, Sender<DspMessage>)>(1);

    let thread_handle = std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let enumerator: IMMDeviceEnumerator =
            match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
                Ok(e) => e,
                Err(e) => {
                    let _ = init_tx.send(Err(anyhow::anyhow!(
                        "Failed to create MMDeviceEnumerator: {}",
                        e
                    )));
                    unsafe {
                        CoUninitialize();
                    }
                    return;
                }
            };

        let mut candidate_devices: Vec<(IMMDevice, String)> = Vec::new();
        let push_unique_wasapi_dev = |list: &mut Vec<(IMMDevice, String)>, dev: IMMDevice| {
            let id = unsafe {
                dev.GetId()
                    .ok()
                    .and_then(|s| s.to_string().ok())
                    .unwrap_or_default()
            };
            let already = list.iter().any(|(existing, _)| {
                let existing_id = unsafe {
                    existing
                        .GetId()
                        .ok()
                        .and_then(|s| s.to_string().ok())
                        .unwrap_or_default()
                };
                !id.is_empty() && existing_id == id
            });
            if !already {
                let name = get_wasapi_device_name(&dev).unwrap_or_else(|| "Unknown".to_string());
                list.push((dev, name));
            }
        };

        if let Some(ref target_name) = selected_dev_owned
            && let Ok(collection) =
                unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
        {
            let count = unsafe { collection.GetCount().unwrap_or(0) };
            let target_lower = target_name.to_lowercase();
            for i in 0..count {
                if let Ok(dev) = unsafe { collection.Item(i) } {
                    let name =
                        get_wasapi_device_name(&dev).unwrap_or_else(|| "Unknown".to_string());
                    let name_lower = name.to_lowercase();
                    if name_lower == target_lower
                        || name_lower.contains(&target_lower)
                        || target_lower.contains(&name_lower)
                    {
                        push_unique_wasapi_dev(&mut candidate_devices, dev);
                    }
                }
            }
        }

        if selected_dev_owned.is_none() {
            if let Ok(default_dev) =
                unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            {
                push_unique_wasapi_dev(&mut candidate_devices, default_dev);
            }
            if let Ok(collection) =
                unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            {
                let count = unsafe { collection.GetCount().unwrap_or(0) };
                for i in 0..count {
                    if let Ok(dev) = unsafe { collection.Item(i) } {
                        let name =
                            get_wasapi_device_name(&dev).unwrap_or_else(|| "Unknown".to_string());
                        if is_relevant_audio_device(&name) {
                            push_unique_wasapi_dev(&mut candidate_devices, dev);
                        }
                    }
                }
            }
        }

        let channel_candidates: Vec<(u16, u32)> = if surround_upmix_enabled {
            vec![
                (12, 0x2D63F),
                (8, 0x63F),
                (8, 0xFF),
                (6, 0x3F),
                (6, 0x60F),
                (4, 0x33),
            ]
        } else {
            match target_channels {
                c if c >= 9 => vec![(12, 0x2D63F), (8, 0x63F), (8, 0xFF), (6, 0x3F)],
                7 | 8 => vec![(8, 0x63F), (8, 0xFF), (6, 0x3F)],
                5 | 6 => vec![(6, 0x3F), (6, 0x60F), (8, 0x63F), (8, 0xFF)],
                3 | 4 => vec![(4, 0x33), (6, 0x3F), (8, 0x63F)],
                _ => Vec::new(),
            }
        };

        let mut rate_candidates = Vec::with_capacity(3);
        for &r in &[target_rate.max(44100), 48000, 44100] {
            if !rate_candidates.contains(&r) {
                rate_candidates.push(r);
            }
        }

        let exclusive_fmt_candidates = [
            WasapiPcmFormat::I24In32,
            WasapiPcmFormat::F32,
            WasapiPcmFormat::I32,
            WasapiPcmFormat::I16,
        ];
        let shared_fmt_candidates = [
            WasapiPcmFormat::F32,
            WasapiPcmFormat::I24In32,
            WasapiPcmFormat::I32,
            WasapiPcmFormat::I16,
        ];

        let mut opened: Option<(
            IMMDevice,
            String,
            Option<(IAudioClient, IAudioRenderClient)>,
            HANDLE,
            u32,
            u16,
            u32,
            u32,
            WasapiPcmFormat,
            bool,
            Option<(crate::mpegh::Mpegh714Encoder, usize, String)>,
            Option<SpatialAudio714State>,
        )> = None;
        let mut last_err =
            anyhow::anyhow!("No multichannel WASAPI support detected on candidate endpoints");

        'dev_loop: for (dev, dev_name) in candidate_devices {
            let probe_client: Option<IAudioClient> = unsafe { dev.Activate(CLSCTX_ALL, None).ok() };
            let mix_channels = if let Some(ref pc) = probe_client {
                unsafe {
                    if let Ok(mix_ptr) = pc.GetMixFormat() {
                        let ch = (*mix_ptr).nChannels;
                        CoTaskMemFree(Some(mix_ptr as *const std::ffi::c_void));
                        ch
                    } else {
                        2
                    }
                }
            } else {
                2
            };
            drop(probe_client);

            for &(ch, mask) in &channel_candidates {
                // 0. When 12-channel (7.1.4 / 11.1) output is enabled (`ch == 12 && mpegh_bitstream`),
                // first check if Windows Spatial Audio (`Dolby Atmos for Home Theater`, `GetMaxDynamicObjectCount() > 0`)
                // is active on the endpoint BEFORE probing `AUDCLNT_SHAREMODE_EXCLUSIVE` (since exclusive-mode
                // format/init calls can cause Windows `AudioDg.exe` to reset Spatial Sound to Off).
                if ch == 12 && mpegh_bitstream {
                    match try_open_spatial_audio_714_client(&dev, mix_channels) {
                        Ok((spatial, ev, max_frames, spatial_rate)) => {
                            eprintln!(
                                "[audio] Windows Spatial Audio 7.1.4 (11.1) negotiated on '{}': 12 static bed objects @ {} Hz [{}] (max {} frames/period)",
                                dev_name, spatial_rate, spatial.carrier_label, max_frames
                            );
                            opened = Some((
                                dev,
                                dev_name,
                                None,
                                ev,
                                max_frames,
                                12,
                                0x2D63F,
                                spatial_rate,
                                WasapiPcmFormat::F32,
                                false,
                                None,
                                Some(spatial),
                            ));
                            break 'dev_loop;
                        }
                        Err(e) => {
                            last_err = e;
                        }
                    }
                }

                for &rate in &rate_candidates {
                    // 1. Primary path: WASAPI Exclusive Mode LPCM (bypasses AudioDg.exe's 2ch Shared Mode
                    // mixer and reconfigures the physical HDMI Audio InfoFrame to multi-channel surround).
                    for &fmt in &exclusive_fmt_candidates {
                        match try_open_exclusive_client(&dev, ch, rate, mask, fmt) {
                            Ok((ac, rc, ev, buf_frames)) => {
                                eprintln!(
                                    "[audio] WASAPI Exclusive Multichannel LPCM negotiated on '{}': {}ch (mask 0x{:X}) @ {} Hz [{}] (buffer {} frames)",
                                    dev_name,
                                    ch,
                                    mask,
                                    rate,
                                    fmt.label(),
                                    buf_frames
                                );
                                opened = Some((
                                    dev,
                                    dev_name,
                                    Some((ac, rc)),
                                    ev,
                                    buf_frames,
                                    ch,
                                    mask,
                                    rate,
                                    fmt,
                                    true,
                                    None,
                                    None,
                                ));
                                break 'dev_loop;
                            }
                            Err(e) => {
                                last_err = e;
                            }
                        }
                    }

                    // 2. Fallback path: WASAPI Shared Mode ONLY when the Windows Shared Mode MixFormat
                    // itself has >= `ch` discrete channels (so AudioDg.exe will not downmix to stereo).
                    if mix_channels >= ch {
                        for &fmt in &shared_fmt_candidates {
                            match try_open_shared_client(&dev, ch, rate, mask, fmt) {
                                Ok((ac, rc, ev, buf_frames)) => {
                                    eprintln!(
                                        "[audio] WASAPI Shared Multichannel LPCM negotiated on '{}': {}ch (mask 0x{:X}) @ {} Hz [{}] (buffer {} frames)",
                                        dev_name,
                                        ch,
                                        mask,
                                        rate,
                                        fmt.label(),
                                        buf_frames
                                    );
                                    opened = Some((
                                        dev,
                                        dev_name,
                                        Some((ac, rc)),
                                        ev,
                                        buf_frames,
                                        ch,
                                        mask,
                                        rate,
                                        fmt,
                                        false,
                                        None,
                                        None,
                                    ));
                                    break 'dev_loop;
                                }
                                Err(e) => {
                                    last_err = e;
                                }
                            }
                        }
                    }
                }

                // 3. Direct HDMI AVR fallback: probe native EDID IEC 61937-13 MPEG-H 3D Audio (CICP Layout 19)
                // when connected directly to an HDMI sink that advertises MPEG-H (`0x00000019` / `0x0000000f`).
                if ch == 12 && mpegh_bitstream {
                    match try_open_exclusive_mpegh_iec61937_client(&dev, false) {
                        Ok((ac, rc, ev, buf_frames, rate_factor, carrier_label)) => {
                            match crate::mpegh::Mpegh714Encoder::new(rate_factor as u8, true) {
                                Ok(enc) => {
                                    let carrier_frame_bytes = rate_factor;
                                    eprintln!(
                                        "[audio] WASAPI Exclusive MPEG-H 3D Audio 7.1.4 (11.1, CICP 19) negotiated on '{}': 12ch @ {} Hz via {} (buffer {} carrier frames)",
                                        dev_name,
                                        crate::mpegh::MPEGH_SAMPLE_RATE,
                                        carrier_label,
                                        buf_frames
                                    );
                                    opened = Some((
                                        dev,
                                        dev_name,
                                        Some((ac, rc)),
                                        ev,
                                        buf_frames,
                                        12,
                                        0x2D63F,
                                        crate::mpegh::MPEGH_SAMPLE_RATE,
                                        WasapiPcmFormat::I16,
                                        true,
                                        Some((enc, carrier_frame_bytes, carrier_label)),
                                        None,
                                    ));
                                    break 'dev_loop;
                                }
                                Err(e) => {
                                    unsafe {
                                        let _ = CloseHandle(ev);
                                    }
                                    drop(rc);
                                    drop(ac);
                                    last_err = e;
                                }
                            }
                        }
                        Err(e) => {
                            last_err = e;
                        }
                    }
                }
            }
        }

        let (
            active_device,
            active_device_name,
            wasapi_clients,
            event,
            buffer_frames,
            negotiated_channels,
            negotiated_mask,
            negotiated_rate,
            negotiated_fmt,
            is_exclusive,
            mut mpegh_state,
            mut spatial_state,
        ) = match opened {
            Some(v) => v,
            None => {
                let _ = init_tx.send(Err(last_err));
                drop(enumerator);
                unsafe {
                    CoUninitialize();
                }
                return;
            }
        };

        let is_mpegh_iec_bitstream = mpegh_state.is_some();
        let is_spatial_714 = spatial_state.is_some();
        let is_11_1_active = is_mpegh_iec_bitstream || is_spatial_714;
        let mpegh_carrier_label = if let Some((_, _, ref label)) = mpegh_state {
            label.clone()
        } else if let Some(ref spatial) = spatial_state {
            spatial.carrier_label.clone()
        } else {
            String::new()
        };

        if init_tx
            .send(Ok((
                negotiated_channels,
                negotiated_rate,
                negotiated_fmt,
                active_device_name.clone(),
                is_11_1_active,
                mpegh_carrier_label,
            )))
            .is_err()
        {
            unsafe {
                let _ = CloseHandle(event);
            }
            drop(spatial_state);
            drop(wasapi_clients);
            drop(active_device);
            drop(enumerator);
            unsafe {
                CoUninitialize();
            }
            return;
        }

        let (ready_rx, free_tx, tx) = match pipe_rx.recv() {
            Ok(pipes) => pipes,
            Err(_) => {
                unsafe {
                    let _ = CloseHandle(event);
                }
                drop(spatial_state);
                drop(wasapi_clients);
                drop(active_device);
                drop(enumerator);
                unsafe {
                    CoUninitialize();
                }
                return;
            }
        };

        let hardware_channels = negotiated_channels as usize;
        let bytes_per_sample = negotiated_fmt.bytes_per_sample();
        let frame_bytes = if let Some((_, carrier_fb, _)) = mpegh_state.as_ref() {
            *carrier_fb
        } else {
            hardware_channels * bytes_per_sample
        };
        let window_size = calculate_power_of_two_window_size(negotiated_rate);

        let mut fft_buffer = vec![0.0f32; window_size];
        let mut channel_fft_buffers = vec![vec![0.0f32; window_size]; hardware_channels.max(12)];
        let mut fft_index = 0usize;
        let recycle_rx = dsp_recycle_channel().1.clone();
        let dsp_send_interval = (negotiated_rate as usize / 120).max(1);
        let mut frames_since_last_dsp_send = dsp_send_interval;

        let mut last_channel_vus = Vec::new();
        let mut last_spatial_channels = 2usize;
        let mut last_current_order = 0;
        let mut last_current_row = 0;
        let mut last_bpm = 0;
        let mut last_speed = 0;
        let mut last_current_seconds = 0.0;
        let mut last_current_row_string = String::new();
        let mut last_seek_epoch = 0u64;
        let mut upmixer_dsp = SurroundUpmixerDsp::new(negotiated_rate);
        let mut upmix_peak_vus = [0.0f32; 12];
        let mut current_chunk: Option<AudioChunk> = None;
        let mut chunk_frame_pos = 0usize;
        let mut warmup_silent_buffers =
            if is_exclusive && !is_mpegh_iec_bitstream && !is_spatial_714 {
                1u32
            } else {
                0u32
            };
        let startup_fade_frames = ((negotiated_rate as usize) * 35 / 1000).max(64);
        let mut startup_fade_pos = 0usize;
        let mut iec_byte_queue: std::collections::VecDeque<u8> = if is_mpegh_iec_bitstream {
            std::collections::VecDeque::with_capacity(65536 * 2)
        } else {
            std::collections::VecDeque::new()
        };
        let mut stream_started = !is_mpegh_iec_bitstream;

        let (mut latest_seek_epoch, mut upmix_cfg) = {
            let state = shared_state_thread.lock().unwrap();
            (state.seek_epoch, state.surround_upmix)
        };

        // Start the hardware stream:
        // - For ISpatialAudioObjectRenderStream, call `stream.Start()` before entering the event loop.
        // - For LPCM WASAPI, prime the buffer with silence before calling `audio_client.Start()`.
        // - For IEC 61937-13 MPEG-H bitstreaming, pre-fill the first DMA buffer with real IEC 61937
        //   bursts inside the loop before calling `audio_client.Start()`.
        if let Some(ref spatial) = spatial_state {
            unsafe {
                if let Err(e) = spatial.stream.Start() {
                    eprintln!(
                        "[audio] ISpatialAudioObjectRenderStream::Start() failed: {:?}",
                        e
                    );
                    let _ = CloseHandle(event);
                    drop(spatial_state);
                    drop(wasapi_clients);
                    drop(active_device);
                    drop(enumerator);
                    CoUninitialize();
                    return;
                }
            }
        } else if !is_mpegh_iec_bitstream
            && let Some((ref audio_client, ref render_client)) = wasapi_clients
        {
            unsafe {
                let padding = if is_exclusive {
                    0
                } else {
                    audio_client.GetCurrentPadding().unwrap_or(0)
                };
                let prime_frames = buffer_frames.saturating_sub(padding);
                if prime_frames > 0
                    && let Ok(buf_ptr) = render_client.GetBuffer(prime_frames)
                {
                    std::ptr::write_bytes(buf_ptr, 0, (prime_frames as usize) * frame_bytes);
                    let _ = render_client
                        .ReleaseBuffer(prime_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
                }
                if let Err(e) = audio_client.Start() {
                    eprintln!("[audio] WASAPI Start() failed: {:?}", e);
                    let _ = CloseHandle(event);
                    drop(spatial_state);
                    drop(wasapi_clients);
                    drop(active_device);
                    drop(enumerator);
                    CoUninitialize();
                    return;
                }
            }
        }

        while !stop_token_thread.load(std::sync::atomic::Ordering::Relaxed) {
            let mut is_paused = false;
            if let Ok(state) = shared_state_thread.try_lock() {
                is_paused = state.is_paused;
                latest_seek_epoch = state.seek_epoch;
                upmix_cfg = state.surround_upmix;
            }

            if latest_seek_epoch > last_seek_epoch {
                last_seek_epoch = latest_seek_epoch;
                if let Some(stale) = current_chunk.take() {
                    let _ = free_tx.try_send(stale);
                }
                chunk_frame_pos = 0;
                iec_byte_queue.clear();
                if let Some((ref mut mpegh_encoder, _, _)) = mpegh_state {
                    mpegh_encoder.clear_accumulator();
                }
            } else if let Some(ref c) = current_chunk
                && c.seek_epoch < latest_seek_epoch
            {
                let stale = current_chunk.take().unwrap();
                let _ = free_tx.try_send(stale);
                chunk_frame_pos = 0;
            }

            if !stream_started && is_paused {
                if let Ok(mut state) = shared_state_thread.try_lock() {
                    state.raw_channel_vus.fill(0.0);
                    state.raw_spectrum_data.fill(0.0);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }

            if stream_started {
                let wait_res = unsafe { WaitForSingleObject(event, 50) };
                if wait_res.0 != WAIT_OBJECT_0 {
                    continue;
                }
            }

            let mut frames_written = 0usize;
            let upmix_active = upmix_cfg.is_active_for_hardware(hardware_channels as i32);

            if let Some(ref mut spatial) = spatial_state {
                let mut dyn_count = 0u32;
                let mut avail_frames = 0u32;
                if let Err(e) = unsafe {
                    spatial
                        .stream
                        .BeginUpdatingAudioObjects(&mut dyn_count, &mut avail_frames)
                } {
                    eprintln!(
                        "[audio] ISpatialAudioObjectRenderStream::BeginUpdatingAudioObjects failed: {:?}",
                        e
                    );
                    if let Ok(mut state) = shared_state_thread.lock() {
                        state.audio_device_lost = true;
                    }
                    break;
                }
                if avail_frames == 0 {
                    unsafe {
                        let _ = spatial.stream.EndUpdatingAudioObjects();
                    }
                    continue;
                }

                let frames_to_render = avail_frames as usize;
                let mut obj_ptrs = [std::ptr::null_mut::<f32>(); 12];
                let mut buffer_err = false;
                for (c, obj) in spatial.objects.iter().enumerate().take(12) {
                    let mut ptr: *mut u8 = std::ptr::null_mut();
                    let mut byte_len = 0u32;
                    if unsafe { obj.GetBuffer(&mut ptr, &mut byte_len) }.is_err()
                        || ptr.is_null()
                        || (byte_len as usize) < frames_to_render * 4
                    {
                        buffer_err = true;
                        break;
                    }
                    obj_ptrs[c] = ptr as *mut f32;
                }
                if buffer_err {
                    unsafe {
                        let _ = spatial.stream.EndUpdatingAudioObjects();
                    }
                    continue;
                }

                if is_paused {
                    if let Ok(mut state) = shared_state_thread.try_lock() {
                        state.raw_channel_vus.fill(0.0);
                        state.raw_spectrum_data.fill(0.0);
                    }
                    unsafe {
                        for &ptr in &obj_ptrs {
                            std::ptr::write_bytes(ptr, 0, frames_to_render);
                        }
                        let _ = spatial.stream.EndUpdatingAudioObjects();
                    }
                    continue;
                }

                if upmix_active {
                    upmixer_dsp.sync_config(&upmix_cfg, negotiated_rate);
                }

                while frames_written < frames_to_render {
                    if current_chunk.is_none() {
                        match ready_rx.try_recv() {
                            Ok(chunk) => {
                                if chunk.seek_epoch < latest_seek_epoch {
                                    let _ = free_tx.try_send(chunk);
                                    continue;
                                }
                                last_channel_vus.clear();
                                last_channel_vus.extend_from_slice(&chunk.channel_vus);
                                last_spatial_channels = (chunk.spatial_channels as usize).max(1);
                                last_current_order = chunk.current_order;
                                last_current_row = chunk.current_row;
                                last_bpm = chunk.bpm;
                                last_speed = chunk.speed;
                                last_current_seconds = chunk.current_seconds;
                                last_current_row_string.clear();
                                last_current_row_string.push_str(&chunk.current_row_string);
                                last_seek_epoch = chunk.seek_epoch;
                                if chunk.seek_epoch > latest_seek_epoch {
                                    latest_seek_epoch = chunk.seek_epoch;
                                }

                                if chunk.track_ended
                                    && let Ok(mut state) = shared_state_thread.try_lock()
                                {
                                    state.track_ended = true;
                                }

                                current_chunk = Some(chunk);
                                chunk_frame_pos = 0;
                            }
                            Err(_) => {
                                let rem = frames_to_render - frames_written;
                                unsafe {
                                    for &ptr in &obj_ptrs {
                                        std::ptr::write_bytes(ptr.add(frames_written), 0, rem);
                                    }
                                }
                                break;
                            }
                        }
                    }

                    if let Some(chunk) = current_chunk.as_mut() {
                        let frames_available = chunk.valid_frames.saturating_sub(chunk_frame_pos);
                        let frames_needed = frames_to_render - frames_written;

                        if frames_available == 0 {
                            let c = current_chunk.take().unwrap();
                            let _ = free_tx.try_send(c);
                            continue;
                        }

                        let to_copy = std::cmp::min(frames_available, frames_needed);
                        let src_ch = (chunk.spatial_channels as usize).max(1);
                        let target_upmix = upmix_active && src_ch < hardware_channels;

                        if upmixer_dsp.is_blending_or_active(target_upmix)
                            && src_ch < hardware_channels
                        {
                            for i in 0..to_copy {
                                let src_frame_idx = chunk_frame_pos + i;
                                let dst_frame_idx = frames_written + i;
                                let fade_gain = if startup_fade_pos < startup_fade_frames {
                                    let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                    startup_fade_pos += 1;
                                    0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                } else {
                                    1.0
                                };
                                let frame_slice = &chunk.samples[src_frame_idx * hardware_channels
                                    ..(src_frame_idx + 1) * hardware_channels];
                                let out_channels = upmixer_dsp
                                    .process_multichannel_frame_for_hardware_blended(
                                        frame_slice,
                                        src_ch,
                                        &upmix_cfg,
                                        hardware_channels,
                                        target_upmix,
                                    );

                                // `spatial.objects` are activated in RustTracker's canonical 12-channel order:
                                // [0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:BL(RL), 7:BR(RR), 8:TFL, 9:TFR, 10:TBL(TRL), 11:TBR(TRR)].
                                // Compensate for Windows Spatial Audio (`DolbyMatEnc.dll`) object-summing headroom pad
                                // and AVR Dolby Atmos Dialnorm (~3.5-4.3 dB attenuation relative to 0 dBFS Exclusive LPCM).
                                for c in 0..12 {
                                    let atmos_makeup = if c < 4 { 1.50 } else { 1.65 };
                                    let s = SurroundUpmixerDsp::soft_limit(
                                        out_channels[c] * fade_gain * atmos_makeup,
                                    );
                                    unsafe {
                                        *obj_ptrs[c].add(dst_frame_idx) = s;
                                    }
                                    upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                    channel_fft_buffers[c][fft_index] = s;
                                }

                                let mono = (out_channels[0] + out_channels[1]) * 0.5 * fade_gain;
                                fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                fft_index = (fft_index + 1) % window_size;
                            }
                        } else {
                            for i in 0..to_copy {
                                let src_frame_idx = chunk_frame_pos + i;
                                let dst_frame_idx = frames_written + i;
                                let fade_gain = if startup_fade_pos < startup_fade_frames {
                                    let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                    startup_fade_pos += 1;
                                    0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                } else {
                                    1.0
                                };

                                let mut mono = 0.0f32;
                                for c in 0..12 {
                                    let sample = if c < hardware_channels {
                                        SurroundUpmixerDsp::soft_limit(
                                            chunk.samples[src_frame_idx * hardware_channels + c]
                                                * fade_gain
                                                * 1.50,
                                        )
                                    } else {
                                        0.0
                                    };
                                    unsafe {
                                        *obj_ptrs[c].add(dst_frame_idx) = sample;
                                    }
                                    let ch_weight = match c {
                                        0 | 1 => 0.5,
                                        2 => std::f32::consts::FRAC_1_SQRT_2,
                                        _ => 0.25,
                                    };
                                    mono += sample * ch_weight;
                                    channel_fft_buffers[c][fft_index] = sample;
                                }

                                fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                fft_index = (fft_index + 1) % window_size;
                            }
                        }

                        chunk_frame_pos += to_copy;
                        frames_written += to_copy;
                    }
                }

                if let Err(e) = unsafe { spatial.stream.EndUpdatingAudioObjects() } {
                    eprintln!(
                        "[audio] ISpatialAudioObjectRenderStream::EndUpdatingAudioObjects failed: {:?}",
                        e
                    );
                    if let Ok(mut state) = shared_state_thread.lock() {
                        state.audio_device_lost = true;
                    }
                    break;
                }
            } else if let Some((ref audio_client, ref render_client)) = wasapi_clients {
                let avail_frames = if is_exclusive {
                    buffer_frames
                } else {
                    let padding = match unsafe { audio_client.GetCurrentPadding() } {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!(
                                "[audio] WASAPI Shared GetCurrentPadding failed (device disconnected?): {:?}",
                                e
                            );
                            if let Ok(mut state) = shared_state_thread.lock() {
                                state.audio_device_lost = true;
                            }
                            break;
                        }
                    };
                    buffer_frames.saturating_sub(padding)
                };
                if avail_frames == 0 {
                    continue;
                }

                if warmup_silent_buffers > 0 {
                    warmup_silent_buffers -= 1;
                    if let Ok(buf_ptr) = unsafe { render_client.GetBuffer(avail_frames) } {
                        unsafe {
                            std::ptr::write_bytes(
                                buf_ptr,
                                0,
                                (avail_frames as usize) * frame_bytes,
                            );
                            let _ = render_client
                                .ReleaseBuffer(avail_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
                        }
                    }
                    continue;
                }

                if is_paused {
                    if let Ok(mut state) = shared_state_thread.try_lock() {
                        state.raw_channel_vus.fill(0.0);
                        state.raw_spectrum_data.fill(0.0);
                    }
                    if let Ok(buf_ptr) = unsafe { render_client.GetBuffer(avail_frames) } {
                        unsafe {
                            std::ptr::write_bytes(
                                buf_ptr,
                                0,
                                (avail_frames as usize) * frame_bytes,
                            );
                            let _ = render_client
                                .ReleaseBuffer(avail_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
                        }
                    }
                    continue;
                }

                let buf_ptr = match unsafe { render_client.GetBuffer(avail_frames) } {
                    Ok(ptr) => ptr,
                    Err(e) => {
                        eprintln!(
                            "[audio] WASAPI Shared GetBuffer failed (device disconnected?): {:?}",
                            e
                        );
                        if let Ok(mut state) = shared_state_thread.lock() {
                            state.audio_device_lost = true;
                        }
                        break;
                    }
                };

                let frames_to_render = avail_frames as usize;
                let out_bytes = unsafe {
                    std::slice::from_raw_parts_mut(buf_ptr, frames_to_render * frame_bytes)
                };

                if upmix_active {
                    upmixer_dsp.sync_config(&upmix_cfg, negotiated_rate);
                }

                if let Some((ref mut mpegh_encoder, _, _)) = mpegh_state {
                    // Real-time 12-channel (7.1.4 / 11.1, CICP Layout 19) MPEG-H 3D Audio -> IEC 61937-13
                    // HDMI bitstream encoding path. Each 1024-sample @ 48 kHz audio frame produces one
                    // IEC 61937-13 burst covering 1024 * rate_factor carrier frames @ 192 kHz.
                    let needed_iec_bytes = out_bytes.len();
                    while iec_byte_queue.len() < needed_iec_bytes {
                        if current_chunk.is_none() {
                            let recv_res = if !stream_started {
                                ready_rx
                                    .recv_timeout(std::time::Duration::from_millis(100))
                                    .map_err(|_| ())
                            } else {
                                ready_rx
                                    .recv_timeout(std::time::Duration::from_millis(15))
                                    .map_err(|_| ())
                            };
                            match recv_res {
                                Ok(chunk) => {
                                    if chunk.seek_epoch < latest_seek_epoch {
                                        let _ = free_tx.try_send(chunk);
                                        continue;
                                    }
                                    last_channel_vus.clear();
                                    last_channel_vus.extend_from_slice(&chunk.channel_vus);
                                    last_spatial_channels =
                                        (chunk.spatial_channels as usize).max(1);
                                    last_current_order = chunk.current_order;
                                    last_current_row = chunk.current_row;
                                    last_bpm = chunk.bpm;
                                    last_speed = chunk.speed;
                                    last_current_seconds = chunk.current_seconds;
                                    last_current_row_string.clear();
                                    last_current_row_string.push_str(&chunk.current_row_string);
                                    last_seek_epoch = chunk.seek_epoch;
                                    if chunk.seek_epoch > latest_seek_epoch {
                                        latest_seek_epoch = chunk.seek_epoch;
                                    }

                                    if chunk.track_ended
                                        && let Ok(mut state) = shared_state_thread.try_lock()
                                    {
                                        state.track_ended = true;
                                    }

                                    current_chunk = Some(chunk);
                                    chunk_frame_pos = 0;
                                }
                                Err(_) => {
                                    break;
                                }
                            }
                        }

                        if let Some(chunk) = current_chunk.as_mut() {
                            let frames_available =
                                chunk.valid_frames.saturating_sub(chunk_frame_pos);
                            if frames_available == 0 {
                                let c = current_chunk.take().unwrap();
                                let _ = free_tx.try_send(c);
                                continue;
                            }

                            let src_ch = (chunk.spatial_channels as usize).max(1);
                            let target_upmix = upmix_active && src_ch < hardware_channels;
                            let use_upmixer = upmixer_dsp.is_blending_or_active(target_upmix)
                                && src_ch < hardware_channels;

                            for _ in 0..frames_available {
                                let src_frame_idx = chunk_frame_pos;
                                let fade_gain = if startup_fade_pos < startup_fade_frames {
                                    let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                    startup_fade_pos += 1;
                                    0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                } else {
                                    1.0
                                };
                                let frame_slice = &chunk.samples[src_frame_idx * hardware_channels
                                    ..(src_frame_idx + 1) * hardware_channels];

                                if use_upmixer {
                                    let mut out_channels = upmixer_dsp
                                        .process_multichannel_frame_for_hardware_blended(
                                            frame_slice,
                                            src_ch,
                                            &upmix_cfg,
                                            hardware_channels,
                                            target_upmix,
                                        );
                                    for s in out_channels.iter_mut() {
                                        *s = (*s * fade_gain).clamp(-1.0, 1.0);
                                    }
                                    for c in 0..12 {
                                        let s = out_channels[c];
                                        upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                        channel_fft_buffers[c][fft_index] = s;
                                    }
                                    let mono = (out_channels[0] + out_channels[1]) * 0.5;
                                    fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                    fft_index = (fft_index + 1) % window_size;

                                    let _ = mpegh_encoder.push_rusttracker_714_frame(
                                        &out_channels,
                                        &mut iec_byte_queue,
                                    );
                                } else {
                                    let mut cicp_frame = [0.0f32; 12];
                                    let mut mono = 0.0f32;
                                    for c in 0..12 {
                                        let sample = if c < hardware_channels {
                                            (frame_slice[c] * fade_gain).clamp(-1.0, 1.0)
                                        } else {
                                            0.0
                                        };
                                        cicp_frame[c] = sample;
                                        let ch_weight = match c {
                                            0 | 1 => 0.5,
                                            2 => std::f32::consts::FRAC_1_SQRT_2,
                                            _ => 0.25,
                                        };
                                        mono += sample * ch_weight;
                                        channel_fft_buffers[c][fft_index] = sample;
                                    }
                                    fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                    fft_index = (fft_index + 1) % window_size;

                                    let _ = mpegh_encoder
                                        .push_cicp19_frame(&cicp_frame, &mut iec_byte_queue);
                                }

                                chunk_frame_pos += 1;
                                frames_written += 1;
                                if iec_byte_queue.len() >= needed_iec_bytes {
                                    break;
                                }
                            }

                            if chunk_frame_pos >= chunk.valid_frames {
                                let c = current_chunk.take().unwrap();
                                let _ = free_tx.try_send(c);
                            }
                        }
                    }

                    let copy_bytes = iec_byte_queue.len().min(out_bytes.len());
                    for b in &mut out_bytes[..copy_bytes] {
                        *b = iec_byte_queue.pop_front().unwrap_or(0);
                    }
                    if copy_bytes < out_bytes.len() {
                        out_bytes[copy_bytes..].fill(0);
                    }
                } else {
                    while frames_written < frames_to_render {
                        if current_chunk.is_none() {
                            match ready_rx.try_recv() {
                                Ok(chunk) => {
                                    if chunk.seek_epoch < latest_seek_epoch {
                                        let _ = free_tx.try_send(chunk);
                                        continue;
                                    }
                                    last_channel_vus.clear();
                                    last_channel_vus.extend_from_slice(&chunk.channel_vus);
                                    last_spatial_channels =
                                        (chunk.spatial_channels as usize).max(1);
                                    last_current_order = chunk.current_order;
                                    last_current_row = chunk.current_row;
                                    last_bpm = chunk.bpm;
                                    last_speed = chunk.speed;
                                    last_current_seconds = chunk.current_seconds;
                                    last_current_row_string.clear();
                                    last_current_row_string.push_str(&chunk.current_row_string);
                                    last_seek_epoch = chunk.seek_epoch;
                                    if chunk.seek_epoch > latest_seek_epoch {
                                        latest_seek_epoch = chunk.seek_epoch;
                                    }

                                    if chunk.track_ended
                                        && let Ok(mut state) = shared_state_thread.try_lock()
                                    {
                                        state.track_ended = true;
                                    }

                                    current_chunk = Some(chunk);
                                    chunk_frame_pos = 0;
                                }
                                Err(_) => {
                                    out_bytes[frames_written * frame_bytes..].fill(0);
                                    break;
                                }
                            }
                        }

                        if let Some(chunk) = current_chunk.as_mut() {
                            let frames_available =
                                chunk.valid_frames.saturating_sub(chunk_frame_pos);
                            let frames_needed = frames_to_render - frames_written;

                            if frames_available == 0 {
                                let c = current_chunk.take().unwrap();
                                let _ = free_tx.try_send(c);
                                continue;
                            }

                            let to_copy = std::cmp::min(frames_available, frames_needed);

                            let src_ch = (chunk.spatial_channels as usize).max(1);
                            let target_upmix = upmix_active && src_ch < hardware_channels;
                            if upmixer_dsp.is_blending_or_active(target_upmix)
                                && src_ch < hardware_channels
                            {
                                let active_ch = hardware_channels.min(12);
                                for i in 0..to_copy {
                                    let src_frame_idx = chunk_frame_pos + i;
                                    let dst_frame_idx = frames_written + i;
                                    let dst_byte_base = dst_frame_idx * frame_bytes;
                                    let fade_gain = if startup_fade_pos < startup_fade_frames {
                                        let t =
                                            startup_fade_pos as f32 / startup_fade_frames as f32;
                                        startup_fade_pos += 1;
                                        0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                    } else {
                                        1.0
                                    };
                                    let frame_slice = &chunk.samples[src_frame_idx
                                        * hardware_channels
                                        ..(src_frame_idx + 1) * hardware_channels];
                                    let out_channels = upmixer_dsp
                                        .process_multichannel_frame_for_hardware_blended(
                                            frame_slice,
                                            src_ch,
                                            &upmix_cfg,
                                            hardware_channels,
                                            target_upmix,
                                        );

                                    // Map upmixer output channels:
                                    //   out_channels = [0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:RL, 7:RR, 8:TFL, 9:TFR, 10:TRL, 11:TRR]
                                    // to physical WAVEFORMATEXTENSIBLE channel bit-order:
                                    //   for 0x63F (7.1) / 0x2D63F (7.1.4): [0:FL, 1:FR, 2:FC, 3:LFE, 4:BL(RL), 5:BR(RR), 6:SL, 7:SR, ...].
                                    for c in 0..hardware_channels {
                                        let raw_s = if hardware_channels >= 8
                                            && (negotiated_mask & 0x630) == 0x630
                                        {
                                            match c {
                                                0..=3 => out_channels[c],
                                                4 => out_channels[6],
                                                5 => out_channels[7],
                                                6 => out_channels[4],
                                                7 => out_channels[5],
                                                8..=11 => out_channels[c],
                                                _ => 0.0,
                                            }
                                        } else if c < 12 {
                                            out_channels[c]
                                        } else {
                                            0.0
                                        };
                                        let out_s = raw_s * fade_gain;
                                        write_wasapi_sample(
                                            &mut out_bytes[dst_byte_base + c * bytes_per_sample
                                                ..dst_byte_base + (c + 1) * bytes_per_sample],
                                            out_s.clamp(-1.0, 1.0),
                                            negotiated_fmt,
                                        );
                                    }

                                    for c in 0..active_ch {
                                        let s = out_channels[c] * fade_gain;
                                        upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                        channel_fft_buffers[c][fft_index] = s;
                                    }

                                    let mono =
                                        (out_channels[0] + out_channels[1]) * 0.5 * fade_gain;
                                    fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                    fft_index = (fft_index + 1) % window_size;
                                }
                            } else {
                                for i in 0..to_copy {
                                    let src_frame_idx = chunk_frame_pos + i;
                                    let dst_frame_idx = frames_written + i;
                                    let dst_byte_base = dst_frame_idx * frame_bytes;
                                    let fade_gain = if startup_fade_pos < startup_fade_frames {
                                        let t =
                                            startup_fade_pos as f32 / startup_fade_frames as f32;
                                        startup_fade_pos += 1;
                                        0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                    } else {
                                        1.0
                                    };

                                    let mut mono = 0.0f32;
                                    for c in 0..hardware_channels {
                                        let sample = (chunk.samples
                                            [src_frame_idx * hardware_channels + c]
                                            * fade_gain)
                                            .clamp(-1.0, 1.0);
                                        write_wasapi_sample(
                                            &mut out_bytes[dst_byte_base + c * bytes_per_sample
                                                ..dst_byte_base + (c + 1) * bytes_per_sample],
                                            sample,
                                            negotiated_fmt,
                                        );
                                        let ch_weight = match c {
                                            0 | 1 => {
                                                if hardware_channels == 1 {
                                                    1.0
                                                } else {
                                                    0.5
                                                }
                                            }
                                            2 => std::f32::consts::FRAC_1_SQRT_2,
                                            _ => 0.25,
                                        };
                                        mono += sample * ch_weight;
                                        channel_fft_buffers[c][fft_index] = sample;
                                    }

                                    fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                    fft_index = (fft_index + 1) % window_size;
                                }
                            }

                            chunk_frame_pos += to_copy;
                            frames_written += to_copy;
                        }
                    }
                }

                unsafe {
                    if let Err(e) = render_client.ReleaseBuffer(avail_frames, 0) {
                        eprintln!("[audio] WASAPI ReleaseBuffer failed: {:?}", e);
                        if let Ok(mut state) = shared_state_thread.lock() {
                            state.audio_device_lost = true;
                        }
                        break;
                    }
                    if !stream_started {
                        if let Err(e) = audio_client.Start() {
                            eprintln!("[audio] WASAPI Start() failed: {:?}", e);
                            if let Ok(mut state) = shared_state_thread.lock() {
                                state.audio_device_lost = true;
                            }
                            break;
                        }
                        stream_started = true;
                    }
                }
            }

            frames_since_last_dsp_send += frames_written;
            if frames_since_last_dsp_send >= dsp_send_interval
                && frames_written > 0
                && last_seek_epoch >= latest_seek_epoch
            {
                frames_since_last_dsp_send = 0;
                let play_time =
                    last_current_seconds + (chunk_frame_pos as f64 / negotiated_rate as f64);
                let (dsp_channels, vu_slice): (usize, &[f32]) =
                    if upmix_active && last_spatial_channels < hardware_channels {
                        let active_ch = hardware_channels.min(12);
                        (active_ch, &upmix_peak_vus[..active_ch])
                    } else {
                        (last_channel_vus.len().max(1), &last_channel_vus)
                    };

                let msg = build_recycled_dsp_message(
                    &recycle_rx,
                    &fft_buffer,
                    &channel_fft_buffers,
                    fft_index,
                    window_size,
                    dsp_channels,
                    vu_slice,
                    last_current_order,
                    last_current_row,
                    last_bpm,
                    last_speed,
                    play_time,
                    &last_current_row_string,
                    last_seek_epoch,
                    true,
                );
                if upmix_active {
                    upmix_peak_vus = [0.0f32; 12];
                }
                let _ = tx.try_send(msg);
            }
        }

        if let Some(ref spatial) = spatial_state {
            unsafe {
                let _ = spatial.stream.Stop();
                let _ = spatial.stream.Reset();
            }
        }
        if let Some((ref audio_client, _)) = wasapi_clients {
            unsafe {
                let _ = audio_client.Stop();
                let _ = audio_client.Reset();
            }
        }
        unsafe {
            let _ = CloseHandle(event);
        }
        drop(spatial_state);
        drop(wasapi_clients);
        drop(active_device);
        drop(enumerator);
        unsafe {
            CoUninitialize();
        }
    });

    #[inline(always)]
    fn write_wasapi_sample(dst: &mut [u8], sample: f32, fmt: WasapiPcmFormat) {
        match fmt {
            WasapiPcmFormat::I24In32 => {
                // 24-bit signed PCM left-aligned in 32-bit container (MSB at bit 31, LSB at bit 8)
                let v = ((sample * 8_388_607.0).round() as i32).clamp(-8_388_608, 8_388_607) << 8;
                dst.copy_from_slice(&v.to_le_bytes());
            }
            WasapiPcmFormat::F32 => {
                dst.copy_from_slice(&sample.to_le_bytes());
            }
            WasapiPcmFormat::I32 => {
                let v = (sample as f64 * 2_147_483_647.0).round() as i32;
                dst.copy_from_slice(&v.to_le_bytes());
            }
            WasapiPcmFormat::I16 => {
                let v = (sample * 32_767.0).round().clamp(-32_768.0, 32_767.0) as i16;
                dst.copy_from_slice(&v.to_le_bytes());
            }
        }
    }

    let (
        negotiated_channels,
        negotiated_rate,
        _negotiated_fmt,
        _matched_dev_name,
        is_mpegh_bitstream,
        mpegh_carrier_label,
    ) = match init_rx.recv() {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => {
            let _ = thread_handle.join();
            return Err(e);
        }
        Err(e) => {
            let _ = thread_handle.join();
            return Err(anyhow::anyhow!(
                "WASAPI Exclusive init channel closed unexpectedly: {}",
                e
            ));
        }
    };

    let mut audio_source = audio_source_opt
        .take()
        .ok_or_else(|| anyhow::anyhow!("No audio source available"))?;

    let max_frequency = {
        let mut state = shared_state.lock().unwrap();
        populate_state_from_audio_source(
            &mut state,
            audio_source.as_mut(),
            negotiated_channels as i32,
            negotiated_rate,
        );
        state.stats.mpegh_bitstream_active = is_mpegh_bitstream;
        state.stats.mpegh_carrier_info = mpegh_carrier_label;
        state.max_frequency
    };

    let (tx, rx) = bounded::<DspMessage>(32);
    let window_size = calculate_power_of_two_window_size(negotiated_rate);
    spawn_dsp_thread(
        rx,
        shared_state.clone(),
        negotiated_rate,
        max_frequency,
        window_size,
    );

    let hardware_channels = negotiated_channels as usize;
    let chunk_frames = 1024;
    let pool_size = ((negotiated_rate as usize * 8) / chunk_frames).clamp(384, 1024);

    let (ready_tx, ready_rx) = bounded::<AudioChunk>(pool_size);
    let (free_tx, free_rx) = unbounded::<AudioChunk>();

    spawn_media_decoder_threads(
        audio_source,
        shared_state.clone(),
        stop_token.clone(),
        ready_tx,
        ready_rx.clone(),
        free_tx.clone(),
        free_rx,
        hardware_channels,
        negotiated_rate,
        chunk_frames,
        pool_size,
        4,
        file_path,
    );

    if pipe_tx.send((ready_rx, free_tx, tx)).is_err() {
        stop_token.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = thread_handle.join();
        return Err(anyhow::anyhow!(
            "WASAPI Exclusive render thread exited before receiving audio pipes"
        ));
    }

    Ok(PlaybackHandle::Dummy(Some(thread_handle), stop_token))
}

#[inline(always)]
fn copy_ring_buffer(dst: &mut [f32], src: &[f32], split_idx: usize) {
    let window_size = dst.len();
    let split = split_idx.min(window_size);
    let tail_len = window_size - split;
    dst[..tail_len].copy_from_slice(&src[split..window_size]);
    dst[tail_len..window_size].copy_from_slice(&src[..split]);
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn build_recycled_dsp_message(
    recycle_rx: &Receiver<DspMessage>,
    fft_buffer: &[f32],
    channel_fft_buffers: &[Vec<f32>],
    fft_index: usize,
    window_size: usize,
    hardware_channels: usize,
    channel_vus: &[f32],
    current_order: i32,
    current_row: i32,
    bpm: i32,
    speed: i32,
    current_seconds: f64,
    current_row_string: &str,
    seek_epoch: u64,
    copy_audio: bool,
) -> DspMessage {
    let mut msg = recycle_rx.try_recv().unwrap_or_else(|_| DspMessage {
        audio_data: vec![0.0; window_size],
        channel_vus: Vec::with_capacity(hardware_channels.max(16)),
        current_order: 0,
        current_row: 0,
        bpm: 0,
        speed: 0,
        current_seconds: 0.0,
        current_row_string: String::with_capacity(256),
        channel_audio_data: vec![vec![0.0; window_size]; hardware_channels],
        seek_epoch: 0,
    });

    if copy_audio {
        if msg.audio_data.len() != window_size {
            msg.audio_data.resize(window_size, 0.0);
        }
        copy_ring_buffer(&mut msg.audio_data, fft_buffer, fft_index);

        if msg.channel_audio_data.len() != hardware_channels {
            msg.channel_audio_data
                .resize_with(hardware_channels, || vec![0.0; window_size]);
        }
        for (dst_ch, src_ch) in msg
            .channel_audio_data
            .iter_mut()
            .zip(channel_fft_buffers.iter())
            .take(hardware_channels)
        {
            if dst_ch.len() != window_size {
                dst_ch.resize(window_size, 0.0);
            }
            copy_ring_buffer(dst_ch, src_ch, fft_index);
        }
    }

    msg.channel_vus.clear();
    msg.channel_vus.extend_from_slice(channel_vus);
    msg.current_order = current_order;
    msg.current_row = current_row;
    msg.bpm = bpm;
    msg.speed = speed;
    msg.current_seconds = current_seconds;
    msg.current_row_string.clear();
    msg.current_row_string.push_str(current_row_string);
    msg.seek_epoch = seek_epoch;
    msg
}

struct AudioChunk {
    samples: Vec<f32>,
    valid_frames: usize,
    channel_vus: Vec<f32>,
    left_peak: f32,
    right_peak: f32,
    current_order: i32,
    current_row: i32,
    bpm: i32,
    speed: i32,
    current_seconds: f64,
    current_row_string: String,
    tracker_channels: Option<i32>,
    spatial_channels: i32,
    track_ended: bool,
    seek_epoch: u64,
}

#[allow(clippy::too_many_arguments)]
fn run<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    audio_source_opt: &mut Option<Box<dyn AudioSource>>,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    sample_rate: u32,
    _max_frequency: f32,
    window_size: usize,
    current_path: &str,
) -> Result<(cpal::Stream, Arc<std::sync::atomic::AtomicBool>)>
where
    T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample,
{
    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hardware_channels = config.channels as usize;

    let chunk_frames = 1024;
    let pool_size = ((sample_rate as usize * 8) / chunk_frames).clamp(384, 1024);

    let (ready_tx, ready_rx) = bounded::<AudioChunk>(pool_size);
    let (free_tx, free_rx) = unbounded::<AudioChunk>();

    let mut current_chunk: Option<AudioChunk> = None;
    let mut chunk_frame_pos = 0;

    let mut fft_buffer: Vec<f32> = vec![0.0; window_size];
    let mut channel_fft_buffers: Vec<Vec<f32>> =
        vec![vec![0.0; window_size]; hardware_channels.max(12)];
    let mut fft_index = 0;
    let recycle_rx = dsp_recycle_channel().1.clone();
    let dsp_send_interval = (sample_rate as usize / 120).max(1);
    let mut frames_since_last_dsp_send = dsp_send_interval;

    let mut last_channel_vus = Vec::new();
    let mut last_spatial_channels = 2usize;
    let mut last_current_order = 0;
    let mut last_current_row = 0;
    let mut last_bpm = 0;
    let mut last_speed = 0;
    let mut last_current_seconds = 0.0;
    let mut last_current_row_string = String::new();
    let mut last_seek_epoch = 0u64;
    let initial_src_ch = audio_source_opt
        .as_mut()
        .map(|s| s.get_num_channels())
        .unwrap_or(2);
    let (mut latest_seek_epoch, mut was_paused, mut upmix_cfg) = {
        let state = shared_state.lock().unwrap();
        let mut cfg = state.surround_upmix;
        if !crate::state::SurroundUpmixConfig::can_upmix(initial_src_ch, hardware_channels as i32) {
            cfg.enabled = false;
        } else if !cfg.user_overridden {
            cfg.enabled = !state.force_stereo_downmix
                && !state.passthrough_enabled
                && crate::state::SurroundUpmixConfig::should_enable_by_default(
                    initial_src_ch,
                    hardware_channels as i32,
                );
        }
        (state.seek_epoch, state.is_paused, cfg)
    };
    let mut upmixer_dsp = SurroundUpmixerDsp::new(sample_rate);
    let mut upmix_peak_vus = [0.0f32; 12];
    let startup_fade_frames = ((sample_rate as usize) * 35 / 1000).max(64);
    let mut startup_fade_pos = 0usize;

    let shared_state_err = shared_state.clone();
    let shared_state_cb = shared_state.clone();
    let ready_rx_cb = ready_rx.clone();
    let free_tx_cb = free_tx.clone();
    let tx_cb = tx.clone();

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            if let Ok(state) = shared_state_cb.try_lock() {
                was_paused = state.is_paused;
                latest_seek_epoch = state.seek_epoch;
                upmix_cfg = state.surround_upmix;
            }

            if let Some(ref c) = current_chunk
                && c.seek_epoch < latest_seek_epoch
            {
                let stale = current_chunk.take().unwrap();
                let _ = free_tx_cb.try_send(stale);
                chunk_frame_pos = 0;
            }

            if was_paused {
                for sample in data.iter_mut() {
                    *sample = T::from_sample(0.0);
                }
                if let Ok(mut state) = shared_state_cb.try_lock() {
                    state.raw_channel_vus.fill(0.0);
                    state.raw_spectrum_data.fill(0.0);
                }
                return;
            }

            let upmix_active = upmix_cfg.is_active_for_hardware(hardware_channels as i32);
            if upmix_active {
                upmixer_dsp.sync_config(&upmix_cfg, sample_rate);
            }

            let frames_to_render = data.len() / hardware_channels;
            let mut frames_written = 0;

            while frames_written < frames_to_render {
                if current_chunk.is_none() {
                    match ready_rx_cb.try_recv() {
                        Ok(chunk) => {
                            if chunk.seek_epoch < latest_seek_epoch {
                                let _ = free_tx_cb.try_send(chunk);
                                continue;
                            }
                            last_channel_vus.clear();
                            last_channel_vus.extend_from_slice(&chunk.channel_vus);
                            last_spatial_channels = (chunk.spatial_channels as usize).max(1);
                            last_current_order = chunk.current_order;
                            last_current_row = chunk.current_row;
                            last_bpm = chunk.bpm;
                            last_speed = chunk.speed;
                            last_current_seconds = chunk.current_seconds;
                            last_current_row_string.clear();
                            last_current_row_string.push_str(&chunk.current_row_string);
                            last_seek_epoch = chunk.seek_epoch;
                            if chunk.seek_epoch > latest_seek_epoch {
                                latest_seek_epoch = chunk.seek_epoch;
                            }

                            if chunk.track_ended
                                && let Ok(mut state) = shared_state_cb.try_lock()
                            {
                                state.track_ended = true;
                            }

                            current_chunk = Some(chunk);
                            chunk_frame_pos = 0;
                        }
                        Err(_) => {
                            for frame in data[frames_written * hardware_channels..]
                                .chunks_mut(hardware_channels)
                            {
                                for sample in frame.iter_mut() {
                                    *sample = T::from_sample(0.0);
                                }
                            }
                            break;
                        }
                    }
                }

                if let Some(chunk) = current_chunk.as_mut() {
                    let frames_available = chunk.valid_frames.saturating_sub(chunk_frame_pos);
                    let frames_needed = frames_to_render - frames_written;

                    if frames_available == 0 {
                        let c = current_chunk.take().unwrap();
                        let _ = free_tx_cb.try_send(c);
                        continue;
                    }

                    let to_copy = std::cmp::min(frames_available, frames_needed);

                    let src_ch = (chunk.spatial_channels as usize).max(1);
                    let target_upmix = upmix_active && src_ch < hardware_channels;
                    if upmixer_dsp.is_blending_or_active(target_upmix) && src_ch < hardware_channels
                    {
                        let active_ch = hardware_channels.min(12);
                        for i in 0..to_copy {
                            let src_frame_idx = chunk_frame_pos + i;
                            let dst_frame_idx = frames_written + i;
                            let fade_gain = if startup_fade_pos < startup_fade_frames {
                                let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                startup_fade_pos += 1;
                                0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                            } else {
                                1.0
                            };
                            let frame_slice = &chunk.samples[src_frame_idx * hardware_channels
                                ..(src_frame_idx + 1) * hardware_channels];
                            let out_channels = upmixer_dsp
                                .process_multichannel_frame_for_hardware_blended(
                                    frame_slice,
                                    src_ch,
                                    &upmix_cfg,
                                    hardware_channels,
                                    target_upmix,
                                );

                            for c in 0..hardware_channels {
                                let out_s = if c < 12 {
                                    out_channels[c] * fade_gain
                                } else {
                                    0.0
                                };
                                data[dst_frame_idx * hardware_channels + c] = T::from_sample(out_s);
                            }

                            for c in 0..active_ch {
                                let s = out_channels[c] * fade_gain;
                                upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                channel_fft_buffers[c][fft_index] = s;
                            }

                            let mono = (out_channels[0] + out_channels[1]) * 0.5 * fade_gain;
                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;
                        }
                    } else {
                        for i in 0..to_copy {
                            let src_frame_idx = chunk_frame_pos + i;
                            let dst_frame_idx = frames_written + i;
                            let fade_gain = if startup_fade_pos < startup_fade_frames {
                                let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                startup_fade_pos += 1;
                                0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                            } else {
                                1.0
                            };

                            let mut mono = 0.0;
                            for c in 0..hardware_channels {
                                let sample = (chunk.samples[src_frame_idx * hardware_channels + c]
                                    * fade_gain)
                                    .clamp(-1.0, 1.0);
                                data[dst_frame_idx * hardware_channels + c] =
                                    T::from_sample(sample);
                                let ch_weight = match c {
                                    0 | 1 => {
                                        if hardware_channels == 1 {
                                            1.0
                                        } else {
                                            0.5
                                        }
                                    }
                                    2 => std::f32::consts::FRAC_1_SQRT_2,
                                    _ => 0.25,
                                };
                                mono += sample * ch_weight;
                                channel_fft_buffers[c][fft_index] = sample;
                            }

                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;
                        }
                    }

                    chunk_frame_pos += to_copy;
                    frames_written += to_copy;
                }
            }

            frames_since_last_dsp_send += frames_written;
            if frames_since_last_dsp_send >= dsp_send_interval
                && frames_written > 0
                && last_seek_epoch >= latest_seek_epoch
            {
                frames_since_last_dsp_send = 0;
                let play_time =
                    last_current_seconds + (chunk_frame_pos as f64 / sample_rate as f64);
                let (dsp_channels, vu_slice): (usize, &[f32]) =
                    if upmix_active && last_spatial_channels < hardware_channels {
                        let active_ch = hardware_channels.min(12);
                        (active_ch, &upmix_peak_vus[..active_ch])
                    } else {
                        (last_channel_vus.len().max(1), &last_channel_vus)
                    };
                let msg = build_recycled_dsp_message(
                    &recycle_rx,
                    &fft_buffer,
                    &channel_fft_buffers,
                    fft_index,
                    window_size,
                    dsp_channels,
                    vu_slice,
                    last_current_order,
                    last_current_row,
                    last_bpm,
                    last_speed,
                    play_time,
                    &last_current_row_string,
                    last_seek_epoch,
                    true,
                );
                if upmix_active {
                    upmix_peak_vus = [0.0f32; 12];
                }
                let _ = tx_cb.try_send(msg);
            }
        },
        move |err| {
            match &err {
                cpal::StreamError::DeviceNotAvailable => {
                    eprintln!("Audio device disconnected: {}", err);
                    if let Ok(mut state) = shared_state_err.try_lock() {
                        state.audio_device_lost = true;
                    }
                }
                cpal::StreamError::BufferUnderrun => {
                    // Recoverable buffer underrun/overrun; do NOT trigger audio device reconnect/reload!
                }
                _ => {
                    let s = err.to_string().to_lowercase();
                    if s.contains("underrun") || s.contains("overrun") {
                        // Recoverable xrun; do NOT trigger audio device reconnect/reload!
                    } else {
                        eprintln!("an error occurred on stream: {}", err);
                        if let Ok(mut state) = shared_state_err.try_lock() {
                            state.audio_device_lost = true;
                        }
                    }
                }
            }
        },
        None,
    )?;

    // Populate state and pre-buffer initial prime chunks BEFORE calling stream.play()
    // so the real-time audio callback never underruns or misses the initial upmix config lock.
    let mut audio_source = audio_source_opt
        .take()
        .ok_or_else(|| anyhow::anyhow!("No audio source available"))?;
    {
        let mut state = shared_state.lock().unwrap();
        populate_state_from_audio_source(
            &mut state,
            audio_source.as_mut(),
            config.channels as i32,
            config.sample_rate,
        );
    }

    spawn_media_decoder_threads(
        audio_source,
        shared_state,
        stop_token.clone(),
        ready_tx,
        ready_rx,
        free_tx,
        free_rx,
        hardware_channels,
        sample_rate,
        chunk_frames,
        pool_size,
        4,
        current_path,
    );

    if let Err(e) = stream.play().context("Failed to play stream") {
        stop_token.store(true, std::sync::atomic::Ordering::Relaxed);
        return Err(e);
    }

    Ok((stream, stop_token))
}

fn run_mic<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    _sample_rate: u32,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample + Into<f32>,
{
    let channels = config.channels as usize;
    let window_size = calculate_power_of_two_window_size(config.sample_rate);

    let mut fft_buffer: Vec<f32> = vec![0.0; window_size];
    let mut channel_fft_buffers: Vec<Vec<f32>> = vec![vec![0.0; window_size]; channels];
    let mut fft_index = 0;
    let recycle_rx = dsp_recycle_channel().1.clone();
    let dsp_send_interval = (config.sample_rate as usize / 120).max(1);
    let mut frames_since_last_dsp_send = dsp_send_interval;

    let mut was_paused = false;

    let shared_state_err = shared_state.clone();
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if let Ok(state) = shared_state.try_lock() {
                was_paused = state.is_paused;
            }

            if was_paused {
                if let Ok(mut state) = shared_state.try_lock() {
                    state.raw_channel_vus.fill(0.0);
                    state.raw_spectrum_data.fill(0.0);
                }
                return;
            }

            let mut left_peak = 0.0_f32;
            let mut right_peak = 0.0_f32;
            let mut frames_written = 0;

            for frame in data.chunks(channels) {
                let left = frame[0].into();
                let right = if channels >= 2 { frame[1].into() } else { left };

                left_peak = left_peak.max(left.abs());
                right_peak = right_peak.max(right.abs());

                let mono = (left + right) / 2.0;
                fft_buffer[fft_index] = mono;
                for c in 0..channels {
                    channel_fft_buffers[c][fft_index] = if c < frame.len() {
                        frame[c].into()
                    } else {
                        0.0
                    };
                }
                fft_index = (fft_index + 1) % window_size;
                frames_written += 1;
            }

            frames_since_last_dsp_send += frames_written;
            if frames_since_last_dsp_send >= dsp_send_interval {
                frames_since_last_dsp_send = 0;
                let mut channel_vus = [0.0f32; 2];
                let vu_slice = if channels >= 2 {
                    channel_vus[0] = left_peak;
                    channel_vus[1] = right_peak;
                    &channel_vus[..2]
                } else if channels == 1 {
                    channel_vus[0] = left_peak;
                    &channel_vus[..1]
                } else {
                    &channel_vus[..0]
                };

                let msg = build_recycled_dsp_message(
                    &recycle_rx,
                    &fft_buffer,
                    &channel_fft_buffers,
                    fft_index,
                    window_size,
                    channels,
                    vu_slice,
                    0,
                    0,
                    0,
                    0,
                    0.0,
                    "",
                    0,
                    true,
                );

                let _ = tx.try_send(msg);
            }
        },
        move |err| match &err {
            cpal::StreamError::DeviceNotAvailable => {
                eprintln!("Audio input device disconnected: {}", err);
                if let Ok(mut state) = shared_state_err.try_lock() {
                    state.audio_device_lost = true;
                }
            }
            cpal::StreamError::BufferUnderrun => {}
            _ => {
                let s = err.to_string().to_lowercase();
                if !s.contains("underrun") && !s.contains("overrun") {
                    eprintln!("an error occurred on input stream: {}", err);
                    if let Ok(mut state) = shared_state_err.try_lock() {
                        state.audio_device_lost = true;
                    }
                }
            }
        },
        None,
    )?;

    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_mix_headroom_single_track() {
        // Single track at full volume should have no attenuation
        let headroom = calculate_mix_headroom([1.0]);
        assert_eq!(headroom, 1.0);

        // Single track at reduced volume should not boost headroom above 1.0
        let headroom_half = calculate_mix_headroom([0.5]);
        assert_eq!(headroom_half, 1.0);

        // Zero tracks
        let headroom_empty = calculate_mix_headroom([]);
        assert_eq!(headroom_empty, 1.0);
    }

    #[test]
    fn test_calculate_mix_headroom_multi_track_equal_power() {
        // Two equal tracks at 1.0: active power = 2.0 -> headroom = 1 / sqrt(2) ≈ 0.7071068
        let headroom_two = calculate_mix_headroom([1.0, 1.0]);
        let expected_two = 1.0 / (2.0f32).sqrt();
        assert!((headroom_two - expected_two).abs() < 1e-6);

        // Three equal tracks at 1.0: active power = 3.0 -> headroom = 1 / sqrt(3) ≈ 0.5773503
        let headroom_three = calculate_mix_headroom([1.0, 1.0, 1.0]);
        let expected_three = 1.0 / (3.0f32).sqrt();
        assert!((headroom_three - expected_three).abs() < 1e-6);

        // Four tracks at 0.5: active power = 4 * 0.25 = 1.0 -> headroom = 1.0
        let headroom_four_half = calculate_mix_headroom([0.5, 0.5, 0.5, 0.5]);
        assert_eq!(headroom_four_half, 1.0);
    }

    #[test]
    fn test_calculate_mix_headroom_muted_and_asymmetric_tracks() {
        // Track 1 at 1.0 and Track 2 at 0.0 (muted)
        // Muted track must NOT attenuate the active track
        let headroom_muted = calculate_mix_headroom([1.0, 0.0]);
        assert_eq!(headroom_muted, 1.0);

        // Track 1 at 1.0 and Track 2 at 0.1 (low volume)
        // Active power = 1.0 + 0.01 = 1.01 -> headroom ≈ 0.995037
        // (Previously based on track count, this caused an abrupt 30% drop to 0.707)
        let headroom_asymmetric = calculate_mix_headroom([1.0, 0.1]);
        let expected = 1.0 / (1.01f32).sqrt();
        assert!((headroom_asymmetric - expected).abs() < 1e-6);
        assert!(headroom_asymmetric > 0.99);
    }

    #[test]
    fn test_get_downmixed_sample_mono_to_stereo() {
        let mono_sample = [0.8f32];
        // Mono duplicated to L and R
        let l = get_downmixed_sample(1, &mono_sample, 2, 0);
        let r = get_downmixed_sample(1, &mono_sample, 2, 1);
        assert_eq!(l, 0.8);
        assert_eq!(r, 0.8);
    }

    #[test]
    fn test_get_downmixed_sample_stereo_passthrough_and_folddown() {
        let stereo_samples = [0.6f32, -0.4f32];
        // Stereo to Stereo passthrough
        let l = get_downmixed_sample(2, &stereo_samples, 2, 0);
        let r = get_downmixed_sample(2, &stereo_samples, 2, 1);
        assert_eq!(l, 0.6);
        assert_eq!(r, -0.4);

        // Stereo to Mono folddown: (L + R) * 0.5
        let mono = get_downmixed_sample(2, &stereo_samples, 1, 0);
        assert!((mono - (0.6 - 0.4) * 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_get_downmixed_sample_quad() {
        // FL, FR, BL, BR
        let quad_samples = [0.6f32, -0.6f32, 0.4f32, -0.4f32];
        const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

        let left = get_downmixed_sample(4, &quad_samples, 2, 0);
        let right = get_downmixed_sample(4, &quad_samples, 2, 1);

        let expected_left = (0.6 + INV_SQRT2 * 0.4) * INV_SQRT2;
        let expected_right = (-0.6 + INV_SQRT2 * -0.4) * INV_SQRT2;

        assert!((left - expected_left).abs() < 1e-6);
        assert!((right - expected_right).abs() < 1e-6);
    }

    #[test]
    fn test_get_downmixed_sample_5_1_surround() {
        // FL, FR, FC, LFE, BL, BR
        let surround_samples = [0.5f32, 0.5f32, 0.4f32, 0.2f32, 0.3f32, 0.3f32];
        const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

        let left = get_downmixed_sample(6, &surround_samples, 2, 0);
        let right = get_downmixed_sample(6, &surround_samples, 2, 1);

        let expected_left = (0.5 + INV_SQRT2 * 0.4 + INV_SQRT2 * 0.3 + 0.5 * 0.2) * INV_SQRT2;
        let expected_right = (0.5 + INV_SQRT2 * 0.4 + INV_SQRT2 * 0.3 + 0.5 * 0.2) * INV_SQRT2;

        assert!((left - expected_left).abs() < 1e-6);
        assert!((right - expected_right).abs() < 1e-6);
    }

    #[test]
    fn test_multi_track_active_tracks_filter_enabled() {
        struct MockTrack {
            track_idx: usize,
            volume: f32,
            enabled: bool,
        }
        let tracks = [
            MockTrack {
                track_idx: 0,
                volume: 1.0,
                enabled: true,
            },
            MockTrack {
                track_idx: 1,
                volume: 0.0,
                enabled: true,
            }, // Muted but enabled
            MockTrack {
                track_idx: 2,
                volume: 0.0,
                enabled: false,
            }, // Omitted / disabled
        ];

        // Active tracks filter checks `track.enabled`
        let mut active: Vec<usize> = tracks
            .iter()
            .filter(|t| t.enabled)
            .map(|t| t.track_idx)
            .collect();
        active.sort();
        // Track 1 must remain active in the UI controls despite 0.0 volume
        assert_eq!(active, vec![0, 1]);

        // Prior buggy filter checked `track.volume > 0.0`, erroneously dropping muted tracks
        let buggy_active: Vec<usize> = tracks
            .iter()
            .filter(|t| t.volume > 0.0)
            .map(|t| t.track_idx)
            .collect();
        assert_eq!(buggy_active, vec![0]);
    }

    #[test]
    fn test_multi_track_mix_mode_flag_transitions() {
        let single_track = [0];
        let multi_track = [0, 1];
        let empty_track: [usize; 0] = [];

        assert!(!(single_track.len() > 1));
        assert!(multi_track.len() > 1);
        assert!(!(empty_track.len() > 1));
    }

    #[test]
    fn test_dsp_thread_skips_fft_in_fullscreen_video_and_preserves_bitstream_buffer_pct() {
        let state = Arc::new(Mutex::new(AppState::new("Test".to_string())));
        {
            let mut s = state.lock().unwrap();
            s.has_video_stream = true;
            s.video_mode = 3; // Fullscreen video mode (no visualization)
            s.stats.bitstream_active = true;
            s.stats.audio_buffer_fill_pct = 82.5;
            s.stats.fft_us = 999.0;
        }

        let (tx, rx) = bounded(4);
        let window_size = 2048;
        spawn_dsp_thread(rx, Arc::clone(&state), 48000, 20000.0, window_size);

        // Send a DSP message while in fullscreen video mode
        tx.send(DspMessage {
            audio_data: Vec::new(),
            channel_audio_data: Vec::new(),
            channel_vus: vec![0.4, 0.5],
            current_order: 1,
            current_row: 2,
            bpm: 120,
            speed: 6,
            current_seconds: 42.5,
            current_row_string: String::new(),
            seek_epoch: 0,
        })
        .unwrap();

        // Wait briefly for DSP thread to process
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            let s = state.lock().unwrap();
            if (s.current_seconds - 42.5).abs() < 1e-6 {
                break;
            }
        }

        {
            let mut s = state.lock().unwrap();
            assert!((s.current_seconds - 42.5).abs() < 1e-6);
            assert_eq!(
                s.stats.fft_us, 0.0,
                "CPU FFT time must be 0.0 when skipped in Fullscreen Video mode"
            );
            assert_eq!(
                s.stats.audio_buffer_fill_pct, 82.5,
                "DSP thread must not overwrite bitstream audio_buffer_fill_pct with 0.0%"
            );
            // Switch to normal visualization mode (video_mode = 2 = Split View)
            s.video_mode = 2;
        }

        // Send a DSP message with a 440Hz tone in Split View mode
        let samples: Vec<f32> = (0..window_size)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin())
            .collect();
        tx.send(DspMessage {
            audio_data: samples.clone(),
            channel_audio_data: vec![samples.clone(), samples],
            channel_vus: vec![0.8, 0.8],
            current_order: 1,
            current_row: 3,
            bpm: 120,
            speed: 6,
            current_seconds: 43.0,
            current_row_string: String::new(),
            seek_epoch: 0,
        })
        .unwrap();

        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            let s = state.lock().unwrap();
            if (s.current_seconds - 43.0).abs() < 1e-6 {
                break;
            }
        }

        {
            let s = state.lock().unwrap();
            assert!((s.current_seconds - 43.0).abs() < 1e-6);
            assert!(
                s.stats.fft_us > 0.0,
                "CPU FFT must run when not in Fullscreen Video mode"
            );
            assert_eq!(
                s.stats.audio_buffer_fill_pct, 82.5,
                "DSP thread must preserve bitstream audio_buffer_fill_pct across frames"
            );
        }
    }

    #[test]
    fn test_dsp_thread_discards_stale_seek_epoch_and_preserves_post_seek_lookahead_queue() {
        let state = Arc::new(Mutex::new(AppState::new("Test".to_string())));
        {
            let mut s = state.lock().unwrap();
            s.seek_epoch = 5;
            s.current_seconds = 100.0;
            // Populate post-seek lookahead queue around 100.0s .. 103.0s
            for i in 0..300 {
                s.lookahead_queue.push_back((
                    100.0 + (i as f64) * 0.01,
                    [0.75, 0.5, 0.75, 0.5, 0.0, 0.0, 0.0, 0.0],
                ));
            }
        }

        let (tx, rx) = bounded(4);
        let window_size = 2048;
        spawn_dsp_thread(rx, Arc::clone(&state), 48000, 20000.0, window_size);

        // Send a stale pre-seek message from epoch 4 at timestamp 250.0s (which previously
        // would have overwritten current_seconds = 250.0 and pruned all 100.0..103.0s slices!)
        tx.send(DspMessage {
            audio_data: vec![0.25; window_size],
            channel_audio_data: vec![vec![0.25; window_size], vec![0.25; window_size]],
            channel_vus: vec![0.25, 0.25],
            current_order: 0,
            current_row: 0,
            bpm: 120,
            speed: 6,
            current_seconds: 250.0,
            current_row_string: String::new(),
            seek_epoch: 4,
        })
        .unwrap();

        // Follow with a fresh post-seek message from epoch 5 at timestamp 100.05s
        tx.send(DspMessage {
            audio_data: vec![0.25; window_size],
            channel_audio_data: vec![vec![0.25; window_size], vec![0.25; window_size]],
            channel_vus: vec![0.25, 0.25],
            current_order: 0,
            current_row: 0,
            bpm: 120,
            speed: 6,
            current_seconds: 100.05,
            current_row_string: String::new(),
            seek_epoch: 5,
        })
        .unwrap();

        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            let s = state.lock().unwrap();
            if (s.current_seconds - 100.05).abs() < 1e-6 {
                break;
            }
        }

        let s = state.lock().unwrap();
        assert!(
            (s.current_seconds - 100.05).abs() < 1e-6,
            "Stale seek_epoch message must not overwrite current_seconds"
        );
        assert!(
            s.lookahead_queue.len() >= 295,
            "Post-seek lookahead_queue slices must not be pruned by stale pre-seek DspMessage (len={})",
            s.lookahead_queue.len()
        );
    }

    #[test]
    fn test_push_planar_lookahead_slices_accumulates_sub_slice_frames_and_remainders() {
        let mut s = AppState::new("Test".to_string());
        s.current_seconds = 10.0;

        // Simulate TrueHD-style 40-sample frames at 48kHz (12 frames = 480 samples = 1 slice of 10ms)
        let channels = vec![vec![0.6f32; 40], vec![0.4f32; 40]];
        for i in 0..24 {
            let pts = 10.0 + (i as f64) * (40.0 / 48_000.0);
            push_planar_lookahead_slices(&mut s, &channels, 48_000, pts);
        }

        assert_eq!(
            s.lookahead_queue.len(),
            2,
            "24 frames of 40 samples @ 48kHz (960 samples) must produce exactly 2 10ms lookahead slices"
        );
        assert_eq!(s.lookahead_sample_buffer.len(), 0);
        assert!((s.lookahead_queue[0].0 - 10.0).abs() < 1e-4);
        assert!((s.lookahead_queue[1].0 - 10.01).abs() < 1e-4);
        assert!((s.lookahead_queue[0].1[1] - 0.6).abs() < 1e-4);
        assert!((s.lookahead_queue[0].1[3] - 0.4).abs() < 1e-4);
        assert!((s.lookahead_queue[0].1[4] - 0.6).abs() < 1e-4);
        assert!((s.lookahead_queue[0].1[5] - 0.4).abs() < 1e-4);

        // Also verify update_lookahead_timeline_from_queue populates the tape timeline at +0.0s (slice 100)
        let mut timeline = vec![0.0f32; 600 * 8];
        update_lookahead_timeline_from_queue(&s.lookahead_queue, &[], &[], 10.0, &mut timeline);
        assert!((timeline[100 * 8 + 1] - 0.6).abs() < 1e-4);
        assert!((timeline[100 * 8 + 3] - 0.4).abs() < 1e-4);
    }

    #[test]
    fn test_surround_upmixer_left_right_hemisphere_and_height_crossover() {
        let sample_rate = 48_000;
        let mut dsp = SurroundUpmixerDsp::new(sample_rate);
        let cfg = crate::state::SurroundUpmixConfig {
            enabled: true,
            sub_crossover_hz: 90.0,
            height_crossover_hz: 2500.0,
            surround_high_bleed: 0.0, // Pure split: highs only to height speakers
            stereo_width: 0.0,        // Disable Mid/Side cross-feed to test strict L/R isolation
            ..Default::default()
        };
        dsp.sync_config(&cfg, sample_rate);

        // 1. Feed a low-mid 250 Hz sine wave strictly on the LEFT channel (R = 0.0)
        let mut peak_low = [0.0f32; 12];
        for n in 0..2048 {
            let l =
                (2.0 * std::f32::consts::PI * 250.0 * n as f32 / sample_rate as f32).sin() * 0.7;
            let out = dsp.process_stereo_to_7_1_4(l, 0.0, &cfg);
            if n > 512 {
                for (p, &s) in peak_low.iter_mut().zip(out.iter()) {
                    *p = p.max(s.abs());
                }
            }
        }
        // Left bed speakers (FL=0, SL=4, RL=6) must be strongly active, Right bed (FR=1, SR=5, RR=7) silent
        assert!(peak_low[0] > 0.4, "FL must carry 250Hz left signal");
        assert!(peak_low[4] > 0.35, "SL must carry 250Hz left signal");
        assert!(peak_low[6] > 0.30, "RL must carry 250Hz left signal");
        assert!(
            peak_low[1] < 1e-4,
            "FR must remain silent for pure left input"
        );
        assert!(
            peak_low[5] < 1e-4,
            "SR must remain silent for pure left input"
        );
        assert!(
            peak_low[7] < 1e-4,
            "RR must remain silent for pure left input"
        );
        // At 250 Hz (well below 2500 Hz height crossover), height speakers (TFL=8) are strongly attenuated
        assert!(
            peak_low[8] < peak_low[4] * 0.15,
            "TFL must be attenuated at 250Hz (< 2.5kHz height crossover)"
        );

        // 2. Feed a high-frequency 8000 Hz sine wave strictly on the RIGHT channel (L = 0.0)
        let mut dsp_hi = SurroundUpmixerDsp::new(sample_rate);
        dsp_hi.sync_config(&cfg, sample_rate);
        let mut peak_hi = [0.0f32; 12];
        for n in 0..2048 {
            let r =
                (2.0 * std::f32::consts::PI * 8000.0 * n as f32 / sample_rate as f32).sin() * 0.7;
            let out = dsp_hi.process_stereo_to_7_1_4(0.0, r, &cfg);
            if n > 512 {
                for (p, &s) in peak_hi.iter_mut().zip(out.iter()) {
                    *p = p.max(s.abs());
                }
            }
        }
        // Right height speakers (TFR=9, TRR=11) must be strongly active, and significantly louder than ear-level SR/RR
        assert!(
            peak_hi[9] > 0.45,
            "TFR must carry 8kHz right high-frequency signal"
        );
        assert!(
            peak_hi[11] > 0.40,
            "TRR must carry 8kHz right high-frequency signal"
        );
        assert!(
            peak_hi[9] > peak_hi[5] * 3.0,
            "At 8kHz (> 2.5kHz crossover), TFR height speaker ({}) must dominate SR bed speaker ({})",
            peak_hi[9],
            peak_hi[5]
        );
        assert!(
            peak_hi[8] < 1e-4,
            "TFL must remain silent for pure right input"
        );
    }

    #[test]
    fn test_rank_output_config_key_prioritizes_f32_over_i16_on_wasapi_configs() {
        // Simulate cpal WASAPI supported_formats() enumeration order: I16 appears before F32
        let mut wasapi_configs = [
            (6u16, 48_000u32, 48_000u32, cpal::SampleFormat::I16),
            (6u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
            (6u16, 44_100u32, 44_100u32, cpal::SampleFormat::I16),
            (6u16, 44_100u32, 44_100u32, cpal::SampleFormat::F32),
        ];
        wasapi_configs.sort_by_key(|&(ch, min_r, max_r, fmt)| {
            rank_output_config_key(ch, min_r, max_r, fmt, 6, 48_000, false, false)
        });
        assert_eq!(
            wasapi_configs[0],
            (6, 48_000, 48_000, cpal::SampleFormat::F32),
            "F32 must be selected before I16 so Windows WASAPI uses WAVE_FORMAT_EXTENSIBLE on 5.1/7.1 endpoints"
        );

        // Also verify stereo (2ch) endpoint with surround upmix enabled prefers F32 at target rate
        let mut stereo_configs = [
            (2u16, 48_000u32, 48_000u32, cpal::SampleFormat::I16),
            (2u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
        ];
        stereo_configs.sort_by_key(|&(ch, min_r, max_r, fmt)| {
            rank_output_config_key(ch, min_r, max_r, fmt, 2, 48_000, false, false)
        });
        assert_eq!(
            stereo_configs[0].3,
            cpal::SampleFormat::F32,
            "Stereo endpoint must also prefer F32 over I16"
        );

        // On a 5.1 (6ch) Bed-Only endpoint whose driver also advertises virtual 12ch,
        // exact target_channels (6) must beat 12 so Bed-Only 5.1 upmixing is used.
        let mut bed_configs = [
            (2u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
            (6u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
            (12u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
        ];
        bed_configs.sort_by_key(|&(ch, min_r, max_r, fmt)| {
            rank_output_config_key(ch, min_r, max_r, fmt, 6, 48_000, true, false)
        });
        assert_eq!(
            bed_configs[0].0, 6,
            "5.1 (6ch) Bed-Only target must be preferred over virtual 12ch"
        );
    }

    #[test]
    fn test_surround_upmixer_stereo_bypass_and_bed_only_vs_bed_plus_height() {
        let sample_rate = 48_000;
        let mut dsp_stereo = SurroundUpmixerDsp::new(sample_rate);
        // Even with extreme +12 dB / -12 dB routing curve gains, stereo/mono (<=2ch) and Front L/R
        // must NEVER be equalized or colored (this is a surround routing shaper, not a master EQ).
        let cfg = crate::state::SurroundUpmixConfig {
            enabled: true,
            band_gains_db: [9.0, -9.0, 8.0, -8.0, 10.0],
            front_level: 1.0,
            surround_high_bleed: 0.0,
            stereo_width: 0.0,
            ..Default::default()
        };
        assert!(!cfg.is_active_for_hardware(1));
        assert!(!cfg.is_active_for_hardware(2));
        assert!(cfg.is_active_for_hardware(6));
        assert!(cfg.is_active_for_hardware(8));
        assert!(cfg.is_active_for_hardware(12));
        assert!(!crate::state::SurroundUpmixConfig::has_height_channels(2));
        assert!(!crate::state::SurroundUpmixConfig::has_height_channels(6));
        assert!(!crate::state::SurroundUpmixConfig::has_height_channels(8));
        assert!(crate::state::SurroundUpmixConfig::has_height_channels(10));
        assert!(crate::state::SurroundUpmixConfig::has_height_channels(12));

        dsp_stereo.sync_config(&cfg, sample_rate);

        // 1. Stereo (2ch) hardware: 100% bit-identical to input even with non-zero eq_gains_db
        let mut crossover_peak_fl = 0.0f32;
        for n in 0..2048 {
            let t = n as f32 / sample_rate as f32;
            let l_in = (2.0 * std::f32::consts::PI * cfg.height_crossover_hz * t).sin() * 0.85;
            let r_in = (2.0 * std::f32::consts::PI * 440.0 * t).cos() * 0.85;
            let stereo_out = dsp_stereo.process_frame_for_hardware(l_in, r_in, &cfg, 2);
            assert_eq!(
                stereo_out[0].to_bits(),
                l_in.to_bits(),
                "Stereo 2ch Left output must be 100% untouched even when routing curve is non-flat"
            );
            assert_eq!(
                stereo_out[1].to_bits(),
                r_in.to_bits(),
                "Stereo 2ch Right output must be 100% untouched even when routing curve is non-flat"
            );

            // And on 7.1.4 with front_gain = 1.0, FL/FR must also remain uncolored by eq_gains_db
            let spatial = dsp_stereo.process_stereo_to_7_1_4(l_in, r_in, &cfg);
            assert!(
                (spatial[0] - l_in).abs() < 1e-6 && (spatial[1] - r_in).abs() < 1e-6,
                "Front L/R anchor must not be colored by surround routing EQ bands"
            );
            if n > 512 {
                crossover_peak_fl = crossover_peak_fl.max(spatial[0].abs());
            }
        }
        assert!(
            crossover_peak_fl > 0.80,
            "FL must not have a phase-cancellation notch at height_crossover_hz (got peak {})",
            crossover_peak_fl
        );

        // 2. Compare Bed-Only (5.1 = 6ch / 7.1 = 8ch) vs Bed + Height (7.1.4 = 12ch) at 8 kHz:
        //    - On Bed-Only (5.1/7.1), there are NO overhead height speakers, so SL/SR receive the
        //      full-bandwidth surround signal without height crossover rolloff, and channels 8..11 are 0.
        //    - On Bed + Height (7.1.4) with surround_high_bleed = 0.0, 8 kHz (> 2.8 kHz crossover)
        //      is routed to overhead height speakers (TFR/TRR) while ear-level SR is low-passed.
        let flat_cfg = crate::state::SurroundUpmixConfig {
            enabled: true,
            surround_high_bleed: 0.0,
            stereo_width: 0.0,
            ..Default::default()
        };
        let mut dsp_bed = SurroundUpmixerDsp::new(sample_rate);
        let mut dsp_atmos = SurroundUpmixerDsp::new(sample_rate);
        dsp_bed.sync_config(&flat_cfg, sample_rate);
        dsp_atmos.sync_config(&flat_cfg, sample_rate);

        let mut peak_sr_bed_only = 0.0f32;
        let mut peak_tfr_bed_only = 0.0f32;
        let mut peak_sr_bed_height = 0.0f32;
        let mut peak_tfr_bed_height = 0.0f32;

        for n in 0..2048 {
            let r =
                (2.0 * std::f32::consts::PI * 8000.0 * n as f32 / sample_rate as f32).sin() * 0.7;
            let out_bed = dsp_bed.process_frame_for_hardware(0.0, r, &flat_cfg, 6);
            let out_atmos = dsp_atmos.process_frame_for_hardware(0.0, r, &flat_cfg, 12);
            if n > 512 {
                peak_sr_bed_only = peak_sr_bed_only.max(out_bed[5].abs());
                peak_tfr_bed_only = peak_tfr_bed_only.max(out_bed[9].abs());
                peak_sr_bed_height = peak_sr_bed_height.max(out_atmos[5].abs());
                peak_tfr_bed_height = peak_tfr_bed_height.max(out_atmos[9].abs());
            }
        }

        assert!(
            peak_sr_bed_only > 0.35,
            "Bed-Only 5.1 SR must retain full-bandwidth 8kHz surround energy (got {})",
            peak_sr_bed_only
        );
        assert!(
            peak_tfr_bed_only < 1e-6,
            "Bed-Only 5.1 must not populate overhead height channels (got {})",
            peak_tfr_bed_only
        );
        assert!(
            peak_sr_bed_only > peak_sr_bed_height * 3.0,
            "Bed-Only SR ({}) must retain high frequencies that Bed+Height SR ({}) routes to overhead speakers",
            peak_sr_bed_only,
            peak_sr_bed_height
        );
        assert!(
            peak_tfr_bed_height > 0.45,
            "Bed+Height 7.1.4 TFR must receive the >2.8kHz height band (got {})",
            peak_tfr_bed_height
        );
    }

    #[test]
    fn test_populate_state_default_upmix_matching_vs_non_matching_channels() {
        struct MockAudioSource {
            channels: i32,
            tracker_channels: Option<i32>,
        }
        impl AudioSource for MockAudioSource {
            fn read_frames(&mut self, _hw_ch: usize, _sr: u32, _out: &mut [f32]) -> usize {
                0
            }
            fn get_duration_seconds(&mut self) -> f64 {
                120.0
            }
            fn get_position_seconds(&mut self) -> f64 {
                0.0
            }
            fn set_position_seconds(&mut self, _pos: f64) {}
            fn get_num_channels(&mut self) -> i32 {
                self.channels
            }
            fn get_current_channel_vu_mono(&mut self, _ch: i32) -> f32 {
                0.0
            }
            fn get_artist(&mut self) -> String {
                "Test".to_string()
            }
            fn get_type(&mut self) -> String {
                "Mock".to_string()
            }
            fn get_tempo(&mut self) -> i32 {
                125
            }
            fn get_speed(&mut self) -> i32 {
                6
            }
            fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
                Some(48_000)
            }
            fn get_num_samples(&mut self) -> i32 {
                0
            }
            fn get_num_instruments(&mut self) -> i32 {
                0
            }
            fn get_num_patterns(&mut self) -> i32 {
                0
            }
            fn get_current_order(&mut self) -> i32 {
                0
            }
            fn get_current_row(&mut self) -> i32 {
                0
            }
            fn get_tracker_channels(&mut self) -> Option<i32> {
                self.tracker_channels
            }
        }

        let mut state = AppState::new("Test".to_string());
        state.force_stereo_downmix = false;

        // 1. 2ch stereo media on 6ch (5.1) output device -> non-matching -> Upmix ON by default
        let mut src_2ch = MockAudioSource {
            channels: 2,
            tracker_channels: None,
        };
        populate_state_from_audio_source(&mut state, &mut src_2ch, 6, 48_000);
        assert!(state.surround_upmix.enabled);
        assert!(state.surround_upmix.is_active_for_hardware(6));
        assert_eq!(state.num_channels, 6);

        // 2. 6ch (5.1) media on 6ch (5.1) output device -> matching -> Upmix OFF by default (discrete 5.1)
        let mut src_6ch = MockAudioSource {
            channels: 6,
            tracker_channels: None,
        };
        populate_state_from_audio_source(&mut state, &mut src_6ch, 6, 48_000);
        assert!(!state.surround_upmix.enabled);
        assert!(!state.surround_upmix.is_active_for_hardware(6));
        assert_eq!(state.num_channels, 6);

        // 3. 6ch (5.1) media on 8ch (7.1) output device -> non-matching -> Upmix ON by default (5.1 -> 7.1)
        populate_state_from_audio_source(&mut state, &mut src_6ch, 8, 48_000);
        assert!(state.surround_upmix.enabled);
        assert!(state.surround_upmix.is_active_for_hardware(8));
        assert_eq!(state.num_channels, 8);

        // 4. 8ch (7.1) media on 8ch (7.1) output device -> matching -> Upmix OFF by default (discrete 7.1)
        let mut src_8ch = MockAudioSource {
            channels: 8,
            tracker_channels: None,
        };
        populate_state_from_audio_source(&mut state, &mut src_8ch, 8, 48_000);
        assert!(!state.surround_upmix.enabled);
        assert!(!state.surround_upmix.is_active_for_hardware(8));
        assert_eq!(state.num_channels, 8);

        // 5. 2ch stereo media on 2ch stereo output device -> matching -> Upmix OFF by default
        populate_state_from_audio_source(&mut state, &mut src_2ch, 2, 48_000);
        assert!(!state.surround_upmix.enabled);
        assert!(!state.surround_upmix.is_active_for_hardware(2));
        assert_eq!(state.num_channels, 2);

        // (covered by test_populate_state_default_upmix_matching_vs_non_matching_channels)
        populate_state_from_audio_source(&mut state, &mut src_2ch, 6, 48_000);
        assert!(state.surround_upmix.enabled);
        state.set_surround_upmix(false);
        assert!(state.surround_upmix.user_overridden);
        assert!(!state.surround_upmix.enabled);
        populate_state_from_audio_source(&mut state, &mut src_2ch, 6, 48_000);
        assert!(!state.surround_upmix.enabled);
        assert_eq!(state.num_channels, 2);
    }

    #[test]
    fn test_surround_upmixer_itu_constant_power_and_sub_bass_management() {
        // 7. Verify ITU-R BS.775 / BS.1770 constant-power loudness conservation and complementary sub-bass crossover
        let sr = 48_000;
        let cfg = crate::state::SurroundUpmixConfig::default();
        let mut dsp_2ch = SurroundUpmixerDsp::new(sr);
        let mut dsp_6ch = SurroundUpmixerDsp::new(sr);
        let mut dsp_8ch = SurroundUpmixerDsp::new(sr);
        let mut dsp_12ch = SurroundUpmixerDsp::new(sr);
        dsp_2ch.sync_config(&cfg, sr);
        dsp_6ch.sync_config(&cfg, sr);
        dsp_8ch.sync_config(&cfg, sr);
        dsp_12ch.sync_config(&cfg, sr);

        let mut power_2ch = 0.0f64;
        let mut power_6ch = 0.0f64;
        let mut power_8ch = 0.0f64;
        let mut power_12ch = 0.0f64;
        let total_samples = 9600;
        let warmup = 1024;
        for n in 0..total_samples {
            let t = n as f32 / sr as f32;
            let two_pi = 2.0 * std::f32::consts::PI;
            // Composite stereo music mix: correlated center vocal/bass + panned mid chords + high shimmer
            let center_vocal = (two_pi * 240.0 * t).sin() * 0.28 + (two_pi * 65.0 * t).sin() * 0.16;
            let l_chord = (two_pi * 440.0 * t).sin() * 0.22 + (two_pi * 4200.0 * t).cos() * 0.14;
            let r_chord = (two_pi * 554.37 * t).cos() * 0.22 + (two_pi * 5600.0 * t).sin() * 0.14;
            let l_in = center_vocal + l_chord;
            let r_in = center_vocal + r_chord;

            let out_2 = dsp_2ch.process_frame_for_hardware(l_in, r_in, &cfg, 2);
            let out_6 = dsp_6ch.process_frame_for_hardware(l_in, r_in, &cfg, 6);
            let out_8 = dsp_8ch.process_frame_for_hardware(l_in, r_in, &cfg, 8);
            let out_12 = dsp_12ch.process_frame_for_hardware(l_in, r_in, &cfg, 12);

            if n >= warmup {
                power_2ch += out_2[..2]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>();
                power_6ch += out_6[..6]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>();
                power_8ch += out_8[..8]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>();
                power_12ch += out_12[..12]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>();
            }
        }
        let db_6ch = 10.0 * (power_6ch / power_2ch).log10();
        let db_8ch = 10.0 * (power_8ch / power_2ch).log10();
        let db_12ch = 10.0 * (power_12ch / power_2ch).log10();
        assert!(
            db_6ch.abs() < 1.25,
            "5.1 upmix total acoustic power must match 2.0 stereo within 1.25 dB (got {:+.2} dB)",
            db_6ch
        );
        assert!(
            db_8ch.abs() < 1.25,
            "7.1 upmix total acoustic power must match 2.0 stereo within 1.25 dB (got {:+.2} dB)",
            db_8ch
        );
        assert!(
            db_12ch.abs() < 1.25,
            "7.1.4 upmix total acoustic power must match 2.0 stereo within 1.25 dB (got {:+.2} dB)",
            db_12ch
        );

        // 8. Verify complementary Linkwitz-Riley sub-bass crossover at 40 Hz (< 90 Hz sub_crossover_hz):
        // LFE must carry strong sub-bass while SL/SR bed and FC are attenuated by the high-pass filter.
        let mut dsp_sub = SurroundUpmixerDsp::new(sr);
        dsp_sub.sync_config(&cfg, sr);
        let mut peak_lfe_40hz = 0.0f32;
        let mut peak_sl_40hz = 0.0f32;
        for n in 0..4096 {
            let t = n as f32 / sr as f32;
            let sub_wave = (2.0 * std::f32::consts::PI * 40.0 * t).sin() * 0.7;
            let out = dsp_sub.process_frame_for_hardware(sub_wave, sub_wave, &cfg, 6);
            if n > 1024 {
                peak_lfe_40hz = peak_lfe_40hz.max(out[3].abs());
                peak_sl_40hz = peak_sl_40hz.max(out[4].abs());
            }
        }
        assert!(
            peak_lfe_40hz > peak_sl_40hz * 4.0,
            "At 40Hz (< 90Hz sub crossover), LFE ({}) must dominate high-passed surround bed SL ({})",
            peak_lfe_40hz,
            peak_sl_40hz
        );

        // 9. Verify native discrete 8-channel (7.1) on 8-channel hardware cannot be upmixed
        // and passes all 8 discrete channels 1:1 untouched without fold-down attenuation.
        assert!(!crate::state::SurroundUpmixConfig::can_upmix(8, 8));
        assert!(!crate::state::SurroundUpmixConfig::can_upmix(6, 6));
        assert!(crate::state::SurroundUpmixConfig::can_upmix(2, 8));
        assert!(crate::state::SurroundUpmixConfig::can_upmix(6, 8));
        let discrete_8ch = [0.8f32, -0.7, 0.5, 0.4, -0.6, 0.6, -0.35, 0.35];
        let out_discrete =
            dsp_8ch.process_multichannel_frame_for_hardware(&discrete_8ch, 8, &cfg, 8);
        assert_eq!(&out_discrete[..8], &discrete_8ch[..]);

        let mut state_8ch = AppState::new("8ch Test".to_string());
        state_8ch.file_loaded = true;
        state_8ch.hardware_channels = 8;
        state_8ch.source_channels = 8;
        state_8ch.surround_upmix.enabled = false;
        state_8ch.set_surround_upmix(true);
        assert!(
            !state_8ch.surround_upmix.enabled,
            "Enabling upmix on an 8-channel source with 8-channel hardware must be blocked"
        );
        assert!(
            state_8ch
                .osd_text
                .as_deref()
                .unwrap_or("")
                .contains("Upmix Not Applicable")
        );

        // 10. Verify toggling Upmix OFF/ON on a stereo track with 8ch hardware never requests
        // a stream reload (`passthrough_change_request` stays None) and crossfades smoothly.
        let mut state_2ch_on_8ch = AppState::new("2ch on 8ch Test".to_string());
        state_2ch_on_8ch.file_loaded = true;
        state_2ch_on_8ch.hardware_channels = 8;
        state_2ch_on_8ch.source_channels = 2;
        state_2ch_on_8ch.passthrough_enabled = false;
        state_2ch_on_8ch.surround_upmix.enabled = true;
        state_2ch_on_8ch.set_surround_upmix(false);
        assert!(!state_2ch_on_8ch.surround_upmix.enabled);
        assert!(
            state_2ch_on_8ch.passthrough_change_request.is_none(),
            "Toggling upmix OFF on 8ch hardware must happen in-place without tearing down the stream"
        );
        state_2ch_on_8ch.set_surround_upmix(true);
        assert!(state_2ch_on_8ch.surround_upmix.enabled);
        assert!(
            state_2ch_on_8ch.passthrough_change_request.is_none(),
            "Toggling upmix ON on 8ch hardware must happen in-place without tearing down the stream"
        );

        let mut dsp_fade = SurroundUpmixerDsp::new(sr);
        dsp_fade.sync_config(&cfg, sr);
        let dry_frame = [0.6f32, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let mut prev_out =
            dsp_fade.process_multichannel_frame_for_hardware_blended(&dry_frame, 2, &cfg, 8, true);
        for _ in 0..512 {
            prev_out = dsp_fade
                .process_multichannel_frame_for_hardware_blended(&dry_frame, 2, &cfg, 8, true);
        }
        // Now toggle target_active = false and verify the first step has no abrupt discontinuity
        let next_out =
            dsp_fade.process_multichannel_frame_for_hardware_blended(&dry_frame, 2, &cfg, 8, false);
        assert!(
            (next_out[0] - prev_out[0]).abs() < 0.005 && (next_out[4] - prev_out[4]).abs() < 0.005,
            "Switching upmix OFF must crossfade smoothly with zero step discontinuity"
        );
        // After 65ms (3120 frames at 48kHz), blend reaches 0.0 (pure dry stereo)
        for _ in 0..3200 {
            prev_out = dsp_fade
                .process_multichannel_frame_for_hardware_blended(&dry_frame, 2, &cfg, 8, false);
        }
        assert!((prev_out[0] - 0.6).abs() < 1e-6 && prev_out[4].abs() < 1e-6);
        assert!(!dsp_fade.is_blending_or_active(false));
    }

    #[test]
    fn test_surround_upmixer_flexible_n_to_m_channel_matrix() {
        let sr = 48_000;
        let cfg = crate::state::SurroundUpmixConfig::default();

        // Test every source_channels (1..=8) against every larger hardware_channels (3..=12)
        for src_ch in 1..=8usize {
            for hw_ch in [3usize, 4, 5, 6, 7, 8, 9, 10, 12] {
                if src_ch >= hw_ch {
                    assert!(!crate::state::SurroundUpmixConfig::can_upmix(
                        src_ch as i32,
                        hw_ch as i32
                    ));
                    continue;
                }
                assert!(crate::state::SurroundUpmixConfig::can_upmix(
                    src_ch as i32,
                    hw_ch as i32
                ));

                let mut dsp = SurroundUpmixerDsp::new(sr);
                dsp.sync_config(&cfg, sr);
                let mut peaks = [0.0f32; 12];

                for n in 0..2048 {
                    let t = n as f32 / sr as f32;
                    let two_pi = 2.0 * std::f32::consts::PI;
                    // Multi-band test signal (55 Hz sub-bass + 440 Hz mid + 5200 Hz high)
                    let l_sig = (two_pi * 55.0 * t).sin() * 0.25
                        + (two_pi * 440.0 * t).sin() * 0.35
                        + (two_pi * 5200.0 * t).cos() * 0.25;
                    let r_sig = (two_pi * 55.0 * t).sin() * 0.25
                        + (two_pi * 554.37 * t).cos() * 0.35
                        + (two_pi * 6400.0 * t).sin() * 0.25;

                    let mut frame = [0.0f32; 12];
                    for (c, f_c) in frame.iter_mut().enumerate().take(src_ch) {
                        *f_c = if c % 2 == 0 { l_sig } else { r_sig };
                    }
                    let out = dsp.process_multichannel_frame_for_hardware(
                        &frame[..hw_ch],
                        src_ch,
                        &cfg,
                        hw_ch,
                    );
                    if n >= 512 {
                        for c in 0..12 {
                            peaks[c] = peaks[c].max(out[c].abs());
                        }
                    }
                }

                // Every active hardware channel (0..hw_ch) must be driven with non-zero audio
                for (c, &p) in peaks.iter().enumerate().take(hw_ch) {
                    assert!(
                        p > 0.02,
                        "Upmix {}ch -> {}ch must populate channel {} (got peak {})",
                        src_ch,
                        hw_ch,
                        c,
                        p
                    );
                }
                // Unused channels beyond hw_ch must remain silent
                for (c, &p) in peaks.iter().enumerate().skip(hw_ch) {
                    assert!(
                        p < 1e-6,
                        "Upmix {}ch -> {}ch must leave channel {} silent (got {})",
                        src_ch,
                        hw_ch,
                        c,
                        p
                    );
                }
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn test_iamf_channel_mapping_and_downmix() {
        // 1. Verify IAMF 7.1.4 AVChannel -> Canonical [0..12] mapping
        // IAMF 7.1.4 substream order: (FL,FR), (SL,SR), (BL,BR), (TFL,TFR), (TBL,TBR), (FC), (LFE)
        // AVChannel IDs: FL=0, FR=1, FC=2, LFE=3, BL=4, BR=5, SL=9, SR=10, TFL=12, TFR=14, TBL=15, TBR=17
        let av_7_1_4 = [0, 1, 9, 10, 4, 5, 12, 14, 15, 17, 2, 3];
        let substreams_7_1_4 = [(0, 2), (1, 2), (2, 2), (3, 2), (4, 2), (5, 1), (6, 1)];
        let (reconstructed_ch, layout_label, map_7_1_4) =
            map_iamf_av_channels_to_canonical(&av_7_1_4, &substreams_7_1_4);
        assert_eq!(reconstructed_ch, 12);
        assert_eq!(layout_label, "7.1.4");
        let expected_canonical = [0, 1, 6, 7, 4, 5, 8, 9, 10, 11, 2, 3];
        for (k, &expected_dst) in expected_canonical.iter().enumerate() {
            assert_eq!(map_7_1_4[k].targets.len(), 1);
            assert_eq!(map_7_1_4[k].targets[0].0, expected_dst);
            assert!((map_7_1_4[k].targets[0].1 - 1.0).abs() < 1e-6);
        }

        // 2. Verify IAMF 5.1 AVChannel -> Canonical [0..6] mapping
        // IAMF 5.1 substream order: (FL,FR), (SL,SR), (FC), (LFE)
        let av_5_1 = [0, 1, 9, 10, 2, 3];
        let substreams_5_1 = [(0, 2), (1, 2), (2, 1), (3, 1)];
        let (reconstructed_5_1, label_5_1, map_5_1) =
            map_iamf_av_channels_to_canonical(&av_5_1, &substreams_5_1);
        assert_eq!(reconstructed_5_1, 6);
        assert_eq!(label_5_1, "5.1");
        let expected_5_1 = [0, 1, 4, 5, 2, 3];
        for (k, &expected_dst) in expected_5_1.iter().enumerate() {
            assert_eq!(map_5_1[k].targets[0].0, expected_dst);
        }

        // 3. Verify Ambisonics FOA (4ch ACN/SN3D) -> 12ch (7.1.4) mapping
        let substreams_foa = [(0, 1), (1, 1), (2, 1), (3, 1)];
        let (ambi_ch, _ambi_label, ambi_map) = map_iamf_ambisonics_to_canonical(&substreams_foa, 4);
        assert_eq!(ambi_ch, 12);
        assert_eq!(ambi_map.len(), 4);
        // W (omnidirectional ACN 0) contributes to all 12 speakers
        assert_eq!(ambi_map[0].targets.len(), 12);

        // 4. Verify 12-channel (7.1.4) downmixing to 8ch (7.1), 6ch (5.1), and 2ch (Stereo)
        // Put unit impulse solely on TFL (ch 8) and TBR (ch 11)
        let mut frame_12 = [0.0f32; 12];
        frame_12[8] = 1.0; // TFL
        frame_12[11] = 1.0; // TBR

        // Downmix to 8ch (7.1): TFL folds into FL (ch 0), TBR folds into BR (ch 5) and SR (ch 7)
        let fl_8 = get_downmixed_sample(12, &frame_12, 8, 0);
        let br_8 = get_downmixed_sample(12, &frame_12, 8, 5);
        let sr_8 = get_downmixed_sample(12, &frame_12, 8, 7);
        assert!(
            fl_8 > 0.5,
            "TFL must fold into FL in 7.1 downmix (got {})",
            fl_8
        );
        assert!(
            br_8 > 0.4,
            "TBR must fold into BR in 7.1 downmix (got {})",
            br_8
        );
        assert!(
            sr_8 > 0.2,
            "TBR must fold into SR in 7.1 downmix (got {})",
            sr_8
        );

        // Downmix to 6ch (5.1): TFL folds into FL (ch 0), TBR folds into BR/LS-RS (ch 5)
        let fl_6 = get_downmixed_sample(12, &frame_12, 6, 0);
        let br_6 = get_downmixed_sample(12, &frame_12, 6, 5);
        assert!(
            fl_6 > 0.5,
            "TFL must fold into FL in 5.1 downmix (got {})",
            fl_6
        );
        assert!(
            br_6 > 0.45,
            "TBR must fold into BR in 5.1 downmix (got {})",
            br_6
        );

        // Downmix to 2ch (Stereo): TFL folds into L (ch 0), TBR folds into R (ch 1)
        let l_2 = get_downmixed_sample(12, &frame_12, 2, 0);
        let r_2 = get_downmixed_sample(12, &frame_12, 2, 1);
        assert!(
            l_2 > 0.4,
            "TFL must fold into L in stereo downmix (got {})",
            l_2
        );
        assert!(
            r_2 > 0.35,
            "TBR must fold into R in stereo downmix (got {})",
            r_2
        );
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn test_iamf_7_1_4_demux_and_decode() {
        let ffmpeg_bin = if std::path::Path::new(r"C:\msys64\ucrt64\bin\ffmpeg.exe").exists() {
            r"C:\msys64\ucrt64\bin\ffmpeg.exe"
        } else if std::path::Path::new("/opt/homebrew/opt/ffmpeg@7/bin/ffmpeg").exists() {
            "/opt/homebrew/opt/ffmpeg@7/bin/ffmpeg"
        } else {
            "ffmpeg"
        };

        let tmp_dir = std::env::temp_dir();
        let iamf_path = tmp_dir.join("rusttracker_test_7_1_4.iamf");
        let mp4_path = tmp_dir.join("rusttracker_test_7_1_4_iamf.mp4");

        let sg_ae =
            "type=iamf_audio_element:id=1:st=0:st=1:st=2:st=3:st=4:st=5:st=6,layer=ch_layout=7.1.4";
        let sg_mp = "type=iamf_mix_presentation:id=2:stg=0:annotations=en-us=default_mix_presentation,submix=parameter_id=100:parameter_rate=48000:default_mix_gain=0.0|element=stg=0:parameter_id=101:parameter_rate=48000:annotations=en-us=7.1.4:default_mix_gain=0.0|layout=sound_system=7.1.4:integrated_loudness=0.0:digital_peak=0.0";

        for path in [&iamf_path, &mp4_path] {
            let status = std::process::Command::new(ffmpeg_bin)
                .args([
                    "-y",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=550:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=660:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=770:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=880:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=330:sample_rate=48000:duration=0.5",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=110:sample_rate=48000:duration=0.5",
                    "-streamid",
                    "0:0",
                    "-streamid",
                    "1:1",
                    "-streamid",
                    "2:2",
                    "-streamid",
                    "3:3",
                    "-streamid",
                    "4:4",
                    "-streamid",
                    "5:5",
                    "-streamid",
                    "6:6",
                    "-stream_group",
                    sg_ae,
                    "-stream_group",
                    sg_mp,
                    "-map",
                    "0:a",
                    "-map",
                    "1:a",
                    "-map",
                    "2:a",
                    "-map",
                    "3:a",
                    "-map",
                    "4:a",
                    "-map",
                    "5:a",
                    "-map",
                    "6:a",
                    "-c:a",
                    "libopus",
                    "-b:a",
                    "64k",
                    path.to_str().unwrap(),
                ])
                .status();

            if let Ok(st) = status
                && st.success()
            {
                let mut src = load_audio_source(path.to_str().unwrap())
                    .expect("Failed to load 7.1.4 IAMF file via load_audio_source");
                assert_eq!(src.get_num_channels(), 12);
                assert_eq!(src.get_type(), "IAMF (Opus)");
                assert_eq!(src.get_active_audio_tracks().len(), 1);
                assert!(
                    src.get_duration_seconds() >= 0.45 && src.get_duration_seconds() <= 0.65,
                    "Expected ~0.5s duration for {:?}, got {}",
                    path,
                    src.get_duration_seconds()
                );

                let mut out_12 = vec![0.0f32; 4800 * 12];
                let frames = src.read_frames(12, 48000, &mut out_12);
                assert!(
                    frames > 1000,
                    "Expected >1000 decoded frames, got {}",
                    frames
                );

                let mut ch_peaks = [0.0f32; 12];
                for f in 0..frames {
                    for c in 0..12 {
                        ch_peaks[c] = ch_peaks[c].max(out_12[f * 12 + c].abs());
                    }
                }
                for (c, &peak) in ch_peaks.iter().enumerate() {
                    assert!(
                        peak > 0.05,
                        "Expected non-zero audio on reconstructed IAMF 7.1.4 channel {} in {:?}, got {}",
                        c,
                        path,
                        peak
                    );
                }
                assert!(out_12.iter().all(|x| x.is_finite()));

                let _ = std::fs::remove_file(path);
            }
        }
    }

    #[test]
    fn test_parse_wpctl_inspect_and_pactl_topology_steamdeck_scenarios() {
        // 1. Steam Deck built-in speakers (2ch stereo)
        let wpctl_speakers = r#"id 46, type PipeWire:Interface:Node
    alsa.card_name = "acp5x"
    audio.channels = "2"
    audio.position = "[ FL FR ]"
    node.description = "Speaker"
    node.name = "alsa_output.pci-0000_04_00.5-platform-acp5x_mach.0.HiFi__hw_acp5x_1__sink"
    media.class = "Audio/Sink"
"#;
        let speaker_topo = parse_wpctl_inspect_topology(wpctl_speakers).expect("speaker topology");
        assert_eq!(
            speaker_topo.sink_id,
            "alsa_output.pci-0000_04_00.5-platform-acp5x_mach.0.HiFi__hw_acp5x_1__sink"
        );
        assert_eq!(speaker_topo.description, "Speaker");
        assert_eq!(speaker_topo.channels, 2);

        // 2. Bluetooth headphones (2ch stereo)
        let wpctl_bt = r#"id 78, type PipeWire:Interface:Node
    api.bluez5.address = "14:3F:A6:88:99:AA"
    audio.channels = "2"
    audio.position = "[ FL FR ]"
    node.description = "WH-1000XM5"
    node.name = "bluez_output.14_3F_A6_88_99_AA.1"
    media.class = "Audio/Sink"
"#;
        let bt_topo = parse_wpctl_inspect_topology(wpctl_bt).expect("bt topology");
        assert_eq!(bt_topo.sink_id, "bluez_output.14_3F_A6_88_99_AA.1");
        assert_eq!(bt_topo.description, "WH-1000XM5");
        assert_eq!(bt_topo.channels, 2);

        // 3. USB-C stereo headset (2ch stereo)
        let wpctl_usbc = r#"id 92, type PipeWire:Interface:Node
    audio.channels = "2"
    audio.position = "[ FL FR ]"
    node.description = "USB-C Headset Analog Stereo"
    node.name = "alsa_output.usb-Google_Inc._USB-C_Headset-00.analog-stereo"
    media.class = "Audio/Sink"
"#;
        let usbc_topo = parse_wpctl_inspect_topology(wpctl_usbc).expect("usbc topology");
        assert_eq!(
            usbc_topo.sink_id,
            "alsa_output.usb-Google_Inc._USB-C_Headset-00.analog-stereo"
        );
        assert_eq!(usbc_topo.description, "USB-C Headset Analog Stereo");
        assert_eq!(usbc_topo.channels, 2);

        // 4. USB Surround sound system (7.1 / 8ch)
        let wpctl_usb_71 = r#"id 105, type PipeWire:Interface:Node
  * audio.channels = "8"
    audio.position = "[ FL FR FC LFE RL RR SL SR ]"
    node.description = "USB Audio 7.1 Surround"
    node.name = "alsa_output.usb-Generic_USB_Surround_Audio-00.analog-surround-71"
    media.class = "Audio/Sink"
"#;
        let usb_71_topo = parse_wpctl_inspect_topology(wpctl_usb_71).expect("usb 7.1 topology");
        assert_eq!(
            usb_71_topo.sink_id,
            "alsa_output.usb-Generic_USB_Surround_Audio-00.analog-surround-71"
        );
        assert_eq!(usb_71_topo.description, "USB Audio 7.1 Surround");
        assert_eq!(usb_71_topo.channels, 8);

        // 5. Fallback to audio.position when audio.channels is omitted (both comma and space separated)
        let wpctl_pos_only = r#"id 106, type PipeWire:Interface:Node
    audio.position = "[ FL, FR, FC, LFE, RL, RR ]"
    node.nick = "USB 5.1 DAC"
    node.name = "alsa_output.usb-DAC_51.analog-surround-51"
"#;
        let usb_51_topo = parse_wpctl_inspect_topology(wpctl_pos_only).expect("usb 5.1 topology");
        assert_eq!(usb_51_topo.channels, 6);
        assert_eq!(usb_51_topo.description, "USB 5.1 DAC");

        let wpctl_space_pos_only = r#"id 107, type PipeWire:Interface:Node
    audio.position = "[ FL FR FC LFE RL RR SL SR ]"
    node.nick = "USB 7.1 DAC"
    node.name = "alsa_output.usb-DAC_71.analog-surround-71"
"#;
        let usb_71_space_topo =
            parse_wpctl_inspect_topology(wpctl_space_pos_only).expect("usb 7.1 space topology");
        assert_eq!(usb_71_space_topo.channels, 8);

        // 6. pactl fallback parser
        let pactl_sinks = r#"Sink #46
	State: SUSPENDED
	Name: alsa_output.pci-0000_04_00.5.Speaker
	Description: Speaker
	Sample Specification: s16le 2ch 48000Hz
	Channel Map: front-left,front-right

Sink #105
	State: RUNNING
	Name: alsa_output.usb-Generic_USB_Surround_Audio-00.analog-surround-71
	Description: USB Audio 7.1 Surround
	Sample Specification: s32le 8ch 48000Hz
	Channel Map: front-left,front-right,front-center,lfe,rear-left,rear-right,side-left,side-right
"#;
        let pactl_topo = parse_pactl_sink_topology(
            "alsa_output.usb-Generic_USB_Surround_Audio-00.analog-surround-71",
            pactl_sinks,
        )
        .expect("pactl 7.1 topology");
        assert_eq!(pactl_topo.channels, 8);
        assert_eq!(pactl_topo.description, "USB Audio 7.1 Surround");

        // 7. /proc/asound/card*/pcm*p/sub0/hw_params parser
        let proc_hw_params = "access: MMAP_INTERLEAVED\nformat: S32_LE\nsubformat: STD\nchannels: 6\nrate: 48000 (48000/1)\n";
        assert_eq!(
            parse_proc_asound_max_playback_channels(proc_hw_params),
            Some(6)
        );
    }

    #[test]
    fn test_steamdeck_dynamic_routing_and_surround_topology_lifecycle() {
        let speaker_topo = AudioSinkTopology {
            sink_id: "alsa_output.acp5x.Speaker".to_string(),
            description: "Speaker".to_string(),
            channels: 2,
        };
        let bt_topo = AudioSinkTopology {
            sink_id: "bluez_output.WH_1000XM5".to_string(),
            description: "WH-1000XM5".to_string(),
            channels: 2,
        };
        let usbc_topo = AudioSinkTopology {
            sink_id: "alsa_output.usb-TypeC_Headphones".to_string(),
            description: "USB-C Headphones".to_string(),
            channels: 2,
        };
        let usb_surround_6ch = AudioSinkTopology {
            sink_id: "alsa_output.usb-Surround_51".to_string(),
            description: "USB 5.1 Audio System".to_string(),
            channels: 6,
        };
        let usb_surround_8ch = AudioSinkTopology {
            sink_id: "alsa_output.usb-Surround_71".to_string(),
            description: "USB 7.1 Audio System".to_string(),
            channels: 8,
        };

        // Stage 1: Built-in Speakers (2ch)
        // Even if playing a 5.1 (6ch) or 7.1 (8ch) file, a 2ch sink resolves to 2 hardware channels
        // so RustTracker downmixes to stereo cleanly without needing force_stereo_downmix.
        assert_eq!(resolve_device_target_channels(false, 2, 6), 2);
        assert_eq!(resolve_device_target_channels(false, 2, 8), 2);
        assert_eq!(resolve_device_target_channels(false, 2, 2), 2);

        // Stage 2: Connect Bluetooth headphones (2ch -> 2ch)
        // On Linux (PipeWire), stream moves live without reload; OSD updates to show Bluetooth headphones.
        let step_bt_linux = evaluate_topology_transition(
            Some(&speaker_topo),
            &bt_topo,
            true,
            true,
            false,
            2,
            false,
            false,
            false,
            false,
            true,
        );
        assert!(!step_bt_linux.reload_stream);
        assert_eq!(
            step_bt_linux.osd_message.as_deref(),
            Some("🎧 Audio Output: WH-1000XM5 (2.0)")
        );

        // On Windows/macOS (non-PipeWire), changing default sink triggers stream reload onto the new device.
        let step_bt_other = evaluate_topology_transition(
            Some(&speaker_topo),
            &bt_topo,
            true,
            true,
            false,
            2,
            false,
            false,
            false,
            false,
            false,
        );
        assert!(step_bt_other.reload_stream);

        // Stage 3: Disconnect Bluetooth and connect USB-C stereo headphones (2ch -> 2ch)
        let step_usbc = evaluate_topology_transition(
            Some(&bt_topo),
            &usbc_topo,
            true,
            true,
            false,
            2,
            false,
            false,
            false,
            false,
            true,
        );
        assert!(!step_usbc.reload_stream);
        assert_eq!(
            step_usbc.osd_message.as_deref(),
            Some("🎧 Audio Output: USB-C Headphones (2.0)")
        );

        // Stage 4a: Disconnect USB-C headphones and connect to USB 5.1 Surround System (2ch -> 6ch)
        let step_surround_51 = evaluate_topology_transition(
            Some(&usbc_topo),
            &usb_surround_6ch,
            true,
            true,
            false,
            2,
            false,
            false,
            false,
            false,
            true,
        );
        assert!(
            step_surround_51.reload_stream,
            "Switching from 2ch USB-C headphones to 6ch USB surround must reload stream to unlock 6 channels"
        );
        assert_eq!(
            step_surround_51.osd_message.as_deref(),
            Some("🔊 Audio Output: USB 5.1 Audio System (5.1 Surround — Bed Only)")
        );
        // Verify channel resolution on the 6ch surround sink:
        // - 5.1 source -> 6ch discrete surround
        // - 2.0 source with surround_upmix enabled -> 6ch surround upmix
        assert_eq!(resolve_device_target_channels(false, 6, 6), 6);
        assert_eq!(resolve_device_target_channels(false, 6, 2), 6);

        // Stage 4b: Connect to USB 7.1 Surround System (6ch -> 8ch)
        let step_surround_71 = evaluate_topology_transition(
            Some(&usb_surround_6ch),
            &usb_surround_8ch,
            true,
            true,
            false,
            6,
            false,
            false,
            false,
            false,
            true,
        );
        assert!(step_surround_71.reload_stream);
        assert_eq!(
            step_surround_71.osd_message.as_deref(),
            Some("🔊 Audio Output: USB 7.1 Audio System (7.1 Surround — Bed Only)")
        );
        assert_eq!(resolve_device_target_channels(false, 8, 8), 8);
        assert_eq!(resolve_device_target_channels(false, 8, 2), 8);

        // Stage 5: Unplug USB surround system -> fallback to built-in stereo speakers (8ch -> 2ch)
        let step_back_to_speakers = evaluate_topology_transition(
            Some(&usb_surround_8ch),
            &speaker_topo,
            true,
            true,
            false,
            8,
            false,
            false,
            false,
            false,
            true,
        );
        assert!(
            step_back_to_speakers.reload_stream,
            "Switching from 8ch surround back to 2ch speakers must reload stream to downmix to stereo"
        );
        assert_eq!(
            step_back_to_speakers.osd_message.as_deref(),
            Some("🎧 Audio Output: Speaker (2.0)")
        );

        // Stage 6: Recovery from dummy output when a real sink appears
        let step_from_dummy = evaluate_topology_transition(
            Some(&speaker_topo),
            &speaker_topo,
            true,
            true,
            false,
            2,
            false,
            true,
            false,
            false,
            true,
        );
        assert!(step_from_dummy.reload_stream);
    }

    #[test]
    fn test_downmix_7_1_to_5_1_and_stereo() {
        // 8ch frame: FL=0.5, FR=-0.5, FC=0.2, LFE=0.1, BL=0.4, BR=-0.4, SL=0.3, SR=-0.3
        let frame_8ch = [0.5f32, -0.5, 0.2, 0.1, 0.4, -0.4, 0.3, -0.3];

        // When outputting to a 5.1 (6ch) sink:
        // Ch 0..4 pass directly; Ch 4 folds BL + SL; Ch 5 folds BR + SR
        assert!((get_downmixed_sample(8, &frame_8ch, 6, 0) - 0.5).abs() < 1e-6);
        assert!((get_downmixed_sample(8, &frame_8ch, 6, 1) - (-0.5)).abs() < 1e-6);
        assert!((get_downmixed_sample(8, &frame_8ch, 6, 2) - 0.2).abs() < 1e-6);
        assert!((get_downmixed_sample(8, &frame_8ch, 6, 3) - 0.1).abs() < 1e-6);
        let expected_ls = (0.4 + 0.3) * std::f32::consts::FRAC_1_SQRT_2;
        let expected_rs = (-0.4 - 0.3) * std::f32::consts::FRAC_1_SQRT_2;
        assert!((get_downmixed_sample(8, &frame_8ch, 6, 4) - expected_ls).abs() < 1e-6);
        assert!((get_downmixed_sample(8, &frame_8ch, 6, 5) - expected_rs).abs() < 1e-6);

        // When outputting to a 2.0 (2ch) stereo sink: ITU-R BS.775 fold-down
        let l_stereo = get_downmixed_sample(8, &frame_8ch, 2, 0);
        let r_stereo = get_downmixed_sample(8, &frame_8ch, 2, 1);
        assert!(l_stereo > 0.4 && r_stereo < -0.2);
    }

    #[test]
    fn test_smoke_2nd_pm_mpegh_714_upmix_and_live_playback() {
        let user_profile = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        let candidates = [
            format!("{}\\Music\\2nd_pm.s3m", user_profile),
            format!("{}/Music/2nd_pm.s3m", user_profile),
            format!("{}\\Music\\2nd_pm.mod", user_profile),
            format!("{}/Music/2nd_pm.mod", user_profile),
        ];
        let Some(track_path) = candidates
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
        else {
            eprintln!("Skipping 2nd_pm smoke test: file not found in ~/Music");
            return;
        };

        eprintln!(
            "=== [SMOKE TEST PART A] Offline 2nd_pm -> 7.1.4 Upmixer -> MPEG-H 3D Audio (CICP 19) -> IEC 61937-13 ==="
        );
        let file_bytes = std::fs::read(&track_path).expect("read 2nd_pm module");
        let mut cursor = std::io::Cursor::new(&file_bytes);
        let module = Module::create(&mut cursor, openmpt::module::Logger::None, &[])
            .expect("OpenMPT load 2nd_pm");
        let mut src = OpenMptSource {
            module: SafeModule(module),
            left_buf: Vec::new(),
            right_buf: Vec::new(),
        };
        let duration = src.get_duration_seconds();
        let tracker_ch = src.get_tracker_channels().unwrap_or(0);
        let mod_type = src.get_type();
        eprintln!(
            "Loaded '{}' | Type: {} | Tracker Channels: {} | Duration: {:.2}s",
            track_path, mod_type, tracker_ch, duration
        );
        assert!(duration > 60.0, "Expected 2nd_pm duration > 60s");
        assert!(tracker_ch >= 4, "Expected tracker channels >= 4");

        // Seek into the active main section (15.0s) so all frequency bands have strong musical content
        src.set_position_seconds(15.0);
        let sample_rate = crate::mpegh::MPEGH_SAMPLE_RATE;
        let mut upmixer = SurroundUpmixerDsp::new(sample_rate);
        let upmix_cfg = crate::state::SurroundUpmixConfig {
            enabled: true,
            mpegh_bitstream: true,
            user_overridden: true,
            mpegh_user_overridden: true,
            ..crate::state::SurroundUpmixConfig::default()
        };
        upmixer.sync_config(&upmix_cfg, sample_rate);

        let mut encoder = crate::mpegh::Mpegh714Encoder::new(4, true)
            .expect("Mpegh714Encoder::new(rate_factor=4)");
        let mut iec_out: std::collections::VecDeque<u8> =
            std::collections::VecDeque::with_capacity(65536 * 2);

        let total_frames = (sample_rate as usize) * 3; // 3.0 seconds = 144,000 frames @ 48 kHz
        let chunk_frames = 1024usize;
        let mut decode_buf = vec![0.0f32; chunk_frames * 12];
        let mut ch_peak = [0.0f32; 12];
        let mut ch_sum_sq = [0.0f64; 12];
        let mut frames_processed = 0usize;
        let mut bursts_verified = 0usize;
        let mut total_iec_bytes = 0usize;
        // Also simulate 192 kHz WASAPI Exclusive carrier DMA buffer drains (4608 frames * 4 bytes = 18,432 bytes)
        let mut simulated_wasapi_dma = vec![0u8; 4608 * 4];
        let mut dma_pulls = 0usize;

        while frames_processed < total_frames {
            let n = src.read_frames(12, sample_rate, &mut decode_buf);
            assert!(n > 0, "Expected non-zero decoded frames from 2nd_pm");
            for chunk_frame_pos in 0..n {
                let src_frame_idx = chunk_frame_pos;
                let frame_slice = &decode_buf[src_frame_idx * 12..(src_frame_idx + 1) * 12];
                let out_12ch = upmixer.process_multichannel_frame_for_hardware_blended(
                    frame_slice,
                    2,
                    &upmix_cfg,
                    12,
                    true,
                );
                for c in 0..12 {
                    let s = out_12ch[c];
                    assert!(s.is_finite(), "Channel {} produced non-finite sample", c);
                    ch_peak[c] = ch_peak[c].max(s.abs());
                    ch_sum_sq[c] += (s as f64) * (s as f64);
                }
                let prev_len = iec_out.len();
                let emitted = encoder
                    .push_rusttracker_714_frame(&out_12ch, &mut iec_out)
                    .expect("push_rusttracker_714_frame");
                if emitted {
                    let burst_len = iec_out.len() - prev_len;
                    assert_eq!(
                        burst_len, 16_384,
                        "Each 4x IEC 61937-13 burst must be 16,384 bytes"
                    );
                    let b0 = iec_out[prev_len];
                    let b1 = iec_out[prev_len + 1];
                    let b2 = iec_out[prev_len + 2];
                    let b3 = iec_out[prev_len + 3];
                    let b4 = iec_out[prev_len + 4];
                    let b5 = iec_out[prev_len + 5];
                    let pa = u16::from_le_bytes([b0, b1]);
                    let pb = u16::from_le_bytes([b2, b3]);
                    let pc = u16::from_le_bytes([b4, b5]);
                    assert_eq!(pa, 0xF872, "Invalid IEC 61937 Pa syncword");
                    assert_eq!(pb, 0x4E1F, "Invalid IEC 61937 Pb syncword");
                    assert_eq!(pc, 0x0839, "Invalid IEC 61937-13 MPEG-H Pc word");
                    total_iec_bytes += burst_len;
                    bursts_verified += 1;
                }
                while iec_out.len() >= simulated_wasapi_dma.len() {
                    for b in &mut simulated_wasapi_dma {
                        *b = iec_out.pop_front().unwrap_or(0);
                    }
                    dma_pulls += 1;
                }
                frames_processed += 1;
            }
        }
        assert!(
            dma_pulls >= 120,
            "Expected >= 120 simulated 192 kHz WASAPI DMA pulls, got {}",
            dma_pulls
        );

        let ch_names = [
            "FL", "FR", "FC", "LFE", "SL", "SR", "RL", "RR", "TFL", "TFR", "TRL", "TRR",
        ];
        eprintln!(
            "Encoded {} frames ({} IEC 61937-13 bursts, {} bytes):",
            frames_processed, bursts_verified, total_iec_bytes
        );
        for c in 0..12 {
            let rms = (ch_sum_sq[c] / frames_processed as f64).sqrt();
            eprintln!(
                "  Ch {:2} ({:3}): peak = {:.4}, rms = {:.4}",
                c, ch_names[c], ch_peak[c], rms
            );
            assert!(
                ch_peak[c] > 0.001,
                "Expected active audio on upmixed channel {} ({}), got peak={}",
                c,
                ch_names[c],
                ch_peak[c]
            );
        }
        assert!(
            bursts_verified >= 140,
            "Expected >= 140 IEC 61937-13 bursts for 3.0s @ 48 kHz, got {}",
            bursts_verified
        );

        eprintln!(
            "=== [SMOKE TEST PART B1] Live WASAPI Exclusive MPEG-H 11.1 Playback of 2nd_pm (35.0s..42.0s) ==="
        );
        eprintln!(
            "Available audio output devices: {:?}",
            get_available_audio_devices(false)
        );
        let shared_state = std::sync::Arc::new(std::sync::Mutex::new(crate::state::AppState::new(
            "SmokeTest".to_string(),
        )));
        {
            let mut st = shared_state.lock().unwrap();
            st.file_loaded = true;
            st.is_paused = false;
            st.set_mpegh_bitstream(true);
            st.set_surround_upmix(true);
            st.seek_request = Some(35.0);
        }
        spawn_audio_topology_watcher(shared_state.clone(), false);
        let playback_handle_b1 = start_audio_thread(&track_path, false, shared_state.clone())
            .expect("start_audio_thread B1");

        let mut max_sec = 0.0f64;
        let mut min_sec_after_seek = f64::MAX;
        let mut saw_12ch_vus = false;
        let mut saw_spectrum = false;
        for step in 1..=14 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let st = shared_state.lock().unwrap();
            let vus: Vec<String> = st
                .raw_channel_vus
                .iter()
                .map(|v| format!("{:.3}", v))
                .collect();
            let spec_max = st.raw_spectrum_data.iter().copied().fold(0.0f32, f32::max);
            eprintln!(
                "[B1 MPEG-H] t={:.1}s: sec={:.3} ord={} row={} hw_ch={} num_ch={} mpegh_active={} carrier='{}' reload_req={:?} spec_max={:.3} vus=[{}]",
                step as f32 * 0.5,
                st.current_seconds,
                st.current_tracker_order,
                st.current_tracker_row,
                st.hardware_channels,
                st.num_channels,
                st.stats.mpegh_bitstream_active,
                st.stats.mpegh_carrier_info,
                st.audio_topology_reload_request,
                spec_max,
                vus.join(", ")
            );
            assert!(
                st.audio_topology_reload_request.is_none(),
                "Topology watcher must not falsely request stream reload during MPEG-H 11.1 playback"
            );
            if st.current_seconds >= 34.5 {
                min_sec_after_seek = min_sec_after_seek.min(st.current_seconds);
                max_sec = max_sec.max(st.current_seconds);
            }
            if st.raw_channel_vus.len() == 12 && st.raw_channel_vus.iter().any(|&v| v > 0.001) {
                saw_12ch_vus = true;
            }
            if spec_max > 0.01 {
                saw_spectrum = true;
            }
        }

        {
            let final_st = shared_state.lock().unwrap();
            assert!(final_st.file_loaded, "2nd_pm must be loaded");
            assert!(
                max_sec > 37.0 && max_sec > min_sec_after_seek + 1.5,
                "Live playback position must advance smoothly in real time (got min={:.3}s, max={:.3}s)",
                min_sec_after_seek,
                max_sec
            );
            if final_st.stats.mpegh_bitstream_active {
                assert_eq!(final_st.hardware_channels, 12);
                assert_eq!(final_st.num_channels, 12);
                assert!(
                    saw_12ch_vus,
                    "Expected active 12-channel VUs during live MPEG-H 11.1 upmix playback"
                );
            }
            assert!(
                saw_spectrum,
                "Expected active FFT spectrum data during live playback"
            );
        }
        drop(playback_handle_b1);
        std::thread::sleep(std::time::Duration::from_millis(250));

        eprintln!("=== [SMOKE TEST PART B2] Live LPCM Playback of 2nd_pm (42.0s..49.0s) ===");
        {
            let mut st = shared_state.lock().unwrap();
            st.is_paused = false;
            st.set_mpegh_bitstream(false);
            st.set_surround_upmix(true);
            st.seek_request = Some(42.0);
            st.audio_topology_reload_request = None;
        }
        let _playback_handle_b2 = start_audio_thread(&track_path, false, shared_state.clone())
            .expect("start_audio_thread B2");
        for step in 1..=14 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let st = shared_state.lock().unwrap();
            let vus: Vec<String> = st
                .raw_channel_vus
                .iter()
                .map(|v| format!("{:.3}", v))
                .collect();
            let spec_max = st.raw_spectrum_data.iter().copied().fold(0.0f32, f32::max);
            eprintln!(
                "[B2 LPCM]   t={:.1}s: sec={:.3} ord={} row={} hw_ch={} num_ch={} mpegh_active={} reload_req={:?} spec_max={:.3} vus=[{}]",
                step as f32 * 0.5,
                st.current_seconds,
                st.current_tracker_order,
                st.current_tracker_row,
                st.hardware_channels,
                st.num_channels,
                st.stats.mpegh_bitstream_active,
                st.audio_topology_reload_request,
                spec_max,
                vus.join(", ")
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn test_spatial_audio_probe() {
        use windows::Win32::Foundation::{CloseHandle, PROPERTYKEY};
        use windows::Win32::Media::Audio::*;
        use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
        use windows::Win32::System::Com::*;
        use windows::Win32::System::Threading::CreateEventW;
        use windows::Win32::System::Variant::{VT_BLOB, VT_LPWSTR};
        use windows::core::{GUID, Interface};

        const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
            fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
            pid: 14,
        };

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let collection = enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .unwrap();
            let count = collection.GetCount().unwrap_or(0);
            for i in 0..count {
                let dev = collection.Item(i).unwrap();
                let store = dev.OpenPropertyStore(STGM_READ).unwrap();
                let propvar = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).unwrap();
                let name = if propvar.Anonymous.Anonymous.vt == VT_LPWSTR {
                    propvar
                        .Anonymous
                        .Anonymous
                        .Anonymous
                        .pwszVal
                        .to_string()
                        .unwrap_or_default()
                } else {
                    propvar.to_string()
                };
                eprintln!("=== Probing ISpatialAudioClient on '{}' ===", name);
                match dev.Activate::<ISpatialAudioClient>(CLSCTX_ALL, None) {
                    Ok(sac) => {
                        let static_mask = sac.GetNativeStaticObjectTypeMask();
                        let max_dyn = sac.GetMaxDynamicObjectCount();
                        let avail = sac.IsSpatialAudioStreamAvailable(
                            &ISpatialAudioObjectRenderStream::IID,
                            None,
                        );
                        eprintln!(
                            "  GetNativeStaticObjectTypeMask: {:?}, GetMaxDynamicObjectCount: {:?}, IsSpatialAudioStreamAvailable: {:?}",
                            static_mask, max_dyn, avail
                        );
                        if let Ok(fmt_enum) = sac.GetSupportedAudioObjectFormatEnumerator() {
                            let fmt_count = fmt_enum.GetCount().unwrap_or(0);
                            eprintln!("  Supported AudioObject formats: {}", fmt_count);
                            for f_idx in 0..fmt_count {
                                if let Ok(wf_ptr) = fmt_enum.GetFormat(f_idx) {
                                    let wf = *wf_ptr;
                                    let tag = wf.wFormatTag;
                                    let ch = wf.nChannels;
                                    let rate = wf.nSamplesPerSec;
                                    let bits = wf.wBitsPerSample;
                                    let align = wf.nBlockAlign;
                                    let max_frames = sac.GetMaxFrameCount(wf_ptr).unwrap_or(0);
                                    eprintln!(
                                        "    Format[{}]: tag=0x{:X} ch={} rate={} bits={} align={} max_frames={}",
                                        f_idx, tag, ch, rate, bits, align, max_frames
                                    );
                                    let mask_714 = AudioObjectType_FrontLeft
                                        | AudioObjectType_FrontRight
                                        | AudioObjectType_FrontCenter
                                        | AudioObjectType_LowFrequency
                                        | AudioObjectType_SideLeft
                                        | AudioObjectType_SideRight
                                        | AudioObjectType_BackLeft
                                        | AudioObjectType_BackRight
                                        | AudioObjectType_TopFrontLeft
                                        | AudioObjectType_TopFrontRight
                                        | AudioObjectType_TopBackLeft
                                        | AudioObjectType_TopBackRight;
                                    let ev = CreateEventW(None, false, false, None).unwrap();
                                    let mut params =
                                        SpatialAudioObjectRenderStreamActivationParams {
                                            ObjectFormat: wf_ptr,
                                            StaticObjectTypeMask: mask_714,
                                            MinDynamicObjectCount: 0,
                                            MaxDynamicObjectCount: 0,
                                            Category: AudioCategory_Media,
                                            EventHandle: ev,
                                            NotifyObject: std::mem::ManuallyDrop::new(None),
                                        };
                                    let mut pv =
                                        std::mem::ManuallyDrop::new(PROPVARIANT::default());
                                    (*pv.Anonymous.Anonymous).vt = VT_BLOB;
                                    (*pv.Anonymous.Anonymous).Anonymous.blob = BLOB {
                                        cbSize: std::mem::size_of::<
                                            SpatialAudioObjectRenderStreamActivationParams,
                                        >() as u32,
                                        pBlobData: &mut params as *mut _ as *mut u8,
                                    };
                                    let stream_res = sac
                                        .ActivateSpatialAudioStream::<ISpatialAudioObjectRenderStream>(
                                            &*pv,
                                        );
                                    eprintln!(
                                        "    ActivateSpatialAudioStream(7.1.4 static mask) => {:?}",
                                        stream_res.as_ref().map(|_| "OK")
                                    );
                                    if let Ok(stream) = stream_res {
                                        let static_types = [
                                            ("FL", AudioObjectType_FrontLeft),
                                            ("FR", AudioObjectType_FrontRight),
                                            ("FC", AudioObjectType_FrontCenter),
                                            ("LFE", AudioObjectType_LowFrequency),
                                            ("SL", AudioObjectType_SideLeft),
                                            ("SR", AudioObjectType_SideRight),
                                            ("BL", AudioObjectType_BackLeft),
                                            ("BR", AudioObjectType_BackRight),
                                            ("TFL", AudioObjectType_TopFrontLeft),
                                            ("TFR", AudioObjectType_TopFrontRight),
                                            ("TBL", AudioObjectType_TopBackLeft),
                                            ("TBR", AudioObjectType_TopBackRight),
                                        ];
                                        let mut objects = Vec::with_capacity(12);
                                        for (label, obj_type) in static_types {
                                            let obj = stream
                                                .ActivateSpatialAudioObject(obj_type)
                                                .expect("ActivateSpatialAudioObject");
                                            objects.push((label, obj));
                                        }
                                        eprintln!(
                                            "      Successfully activated all {} static 7.1.4 AudioObjects!",
                                            objects.len()
                                        );
                                        stream
                                            .Start()
                                            .expect("ISpatialAudioObjectRenderStream::Start");
                                        for tick in 0..5 {
                                            let wait_res = windows::Win32::System::Threading::WaitForSingleObject(ev, 100);
                                            let mut dyn_count = 0u32;
                                            let mut frame_count = 0u32;
                                            stream
                                                .BeginUpdatingAudioObjects(
                                                    &mut dyn_count,
                                                    &mut frame_count,
                                                )
                                                .expect("BeginUpdatingAudioObjects");
                                            for (_, obj) in &objects {
                                                let mut buf_ptr: *mut u8 = std::ptr::null_mut();
                                                let mut buf_bytes = 0u32;
                                                obj.GetBuffer(&mut buf_ptr, &mut buf_bytes)
                                                    .expect("ISpatialAudioObject::GetBuffer");
                                                if !buf_ptr.is_null() && buf_bytes > 0 {
                                                    std::ptr::write_bytes(
                                                        buf_ptr,
                                                        0,
                                                        buf_bytes as usize,
                                                    );
                                                }
                                            }
                                            stream
                                                .EndUpdatingAudioObjects()
                                                .expect("EndUpdatingAudioObjects");
                                            eprintln!(
                                                "      Tick {}: wait={} dyn_count={} frame_count={}",
                                                tick, wait_res.0, dyn_count, frame_count
                                            );
                                        }
                                        let _ = stream.Stop();
                                    }
                                    let _ = CloseHandle(ev);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("  Activate::<ISpatialAudioClient> failed: {:?}", e);
                    }
                }
            }
            CoUninitialize();
        }
    }
}
