// ---------------------------------------------------------
// Automatic Surround & Height Upmixer + 5-Band Crossover EQ
// ---------------------------------------------------------

pub use rt_core::biquad::{BiquadFilter, Lr4Filter};

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

/// Real-time 7.1.4 Surround & Height Upmixer with 5-Band EQ and 4th-Order Linkwitz-Riley Crossovers.
pub struct SurroundUpmixerDsp {
    sample_rate: u32,
    last_config: crate::state::SurroundUpmixConfig,
    eq_l: [BiquadFilter; 5],
    eq_r: [BiquadFilter; 5],
    eq_mc: [[BiquadFilter; 5]; 12],
    sub_lp: Lr4Filter,
    mc_sub_lp: Lr4Filter,
    sub_hp_l: Lr4Filter,
    sub_hp_r: Lr4Filter,
    sub_hp_c: Lr4Filter,
    height_lp_l: Lr4Filter,
    height_hp_l: Lr4Filter,
    height_lp_r: Lr4Filter,
    height_hp_r: Lr4Filter,
    mc_height_hp: [Lr4Filter; 4],
    delay_sl: HaasDelayLine<2048>,
    delay_sr: HaasDelayLine<2048>,
    delay_rl: HaasDelayLine<2048>,
    delay_rr: HaasDelayLine<2048>,
    delay_tfl: HaasDelayLine<2048>,
    delay_tfr: HaasDelayLine<2048>,
    delay_trl: HaasDelayLine<2048>,
    delay_trr: HaasDelayLine<2048>,
    mid_env: f32,
    side_env: f32,
    steer_gain: f32,
    steer_alpha: f32,
    blend: f32,
    blend_initialized: bool,
}

impl SurroundUpmixerDsp {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate.max(8000);
        let default_cfg = crate::state::SurroundUpmixConfig::default();
        let steer_alpha = 1.0 - (-1.0 / (sr as f32 * 0.012)).exp();
        let mut dsp = Self {
            sample_rate: sr,
            last_config: default_cfg,
            eq_l: [BiquadFilter::default(); 5],
            eq_r: [BiquadFilter::default(); 5],
            eq_mc: [[BiquadFilter::default(); 5]; 12],
            sub_lp: Lr4Filter::default(),
            mc_sub_lp: Lr4Filter::default(),
            sub_hp_l: Lr4Filter::default(),
            sub_hp_r: Lr4Filter::default(),
            sub_hp_c: Lr4Filter::default(),
            height_lp_l: Lr4Filter::default(),
            height_hp_l: Lr4Filter::default(),
            height_lp_r: Lr4Filter::default(),
            height_hp_r: Lr4Filter::default(),
            mc_height_hp: [Lr4Filter::default(); 4],
            delay_sl: HaasDelayLine::new(322),
            delay_sr: HaasDelayLine::new(322),
            delay_rl: HaasDelayLine::new(485),
            delay_rr: HaasDelayLine::new(485),
            delay_tfl: HaasDelayLine::new(226),
            delay_tfr: HaasDelayLine::new(226),
            delay_trl: HaasDelayLine::new(398),
            delay_trr: HaasDelayLine::new(398),
            mid_env: 0.0,
            side_env: 0.0,
            steer_gain: 0.0,
            steer_alpha,
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
        self.steer_alpha = 1.0 - (-1.0 / (sr * 0.012)).exp();

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
        for ch_eq in self.eq_mc.iter_mut() {
            for (f, &b) in ch_eq.iter_mut().zip(bands.iter()) {
                f.update_coeffs(b);
            }
        }

        // Linkwitz-Riley 4th-order (-24 dB/octave) LFE low-pass and Height low-pass/high-pass crossovers
        let sub_fc = cfg.sub_crossover_hz.clamp(40.0, 250.0);
        let sub_lp = Lr4Filter::lowpass(sr, sub_fc);
        self.sub_lp.update_coeffs(sub_lp);
        self.mc_sub_lp.update_coeffs(sub_lp);
        let sub_hp = Lr4Filter::highpass(sr, sub_fc);
        self.sub_hp_l.update_coeffs(sub_hp);
        self.sub_hp_r.update_coeffs(sub_hp);
        self.sub_hp_c.update_coeffs(sub_hp);

        let h_fc = cfg.height_crossover_hz.clamp(800.0, 10000.0);
        let h_lp = Lr4Filter::lowpass(sr, h_fc);
        let h_hp = Lr4Filter::highpass(sr, h_fc);
        self.height_lp_l.update_coeffs(h_lp);
        self.height_hp_l.update_coeffs(h_hp);
        self.height_lp_r.update_coeffs(h_lp);
        self.height_hp_r.update_coeffs(h_hp);
        for hp in self.mc_height_hp.iter_mut() {
            hp.update_coeffs(h_hp);
        }

        self.delay_sl.set_delay_ms(self.sample_rate, 6.7);
        self.delay_sr.set_delay_ms(self.sample_rate, 6.7);
        self.delay_rl.set_delay_ms(self.sample_rate, 10.1);
        self.delay_rr.set_delay_ms(self.sample_rate, 10.1);
        self.delay_tfl.set_delay_ms(self.sample_rate, 4.7);
        self.delay_tfr.set_delay_ms(self.sample_rate, 4.7);
        self.delay_trl.set_delay_ms(self.sample_rate, 8.3);
        self.delay_trr.set_delay_ms(self.sample_rate, 8.3);
    }

    /// Smooth envelope-follower Mid/Side center steering:
    /// Tracks the short-term Mid and Side energy envelopes (`~12 ms` time constant) and derives a
    /// smooth scalar steering gain `steer_gain in [0.0, 1.0]`. Multiplying `mid * self.steer_gain`
    /// preserves linear waveform continuity across zero crossings without generating odd-harmonic
    /// wave-rectifier / crossover hiss.
    #[inline(always)]
    fn update_center_steering(&mut self, mid: f32, diff: f32) -> f32 {
        let a = self.steer_alpha;
        let was_silent = (self.mid_env + self.side_env) <= 1e-9;
        self.mid_env += a * (mid.abs() - self.mid_env);
        self.side_env += a * (diff.abs() - self.side_env);
        let total = self.mid_env + self.side_env;
        if total > 1e-9 {
            let target_ratio = ((self.mid_env - self.side_env).max(0.0) / total).clamp(0.0, 1.0);
            if was_silent {
                self.steer_gain = target_ratio;
            } else {
                self.steer_gain += a * (target_ratio - self.steer_gain);
            }
        }
        mid * self.steer_gain
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
    pub(in crate::audio) fn soft_limit(x: f32) -> f32 {
        // Transparent safety ceiling above |x| = 0.98 (-0.17 dBFS) to prevent harsh digital clipping on +12dB EQ boosts
        // while keeping normal mastered program peaks (up to -0.2 dBFS) 100% linear.
        let ax = x.abs();
        if ax <= 0.98 {
            x
        } else {
            let excess = (ax - 0.98) / 0.10;
            let compressed = 0.98 + 0.02 * (excess / (1.0 + excess));
            compressed.copysign(x).clamp(-1.0, 1.0)
        }
    }

    /// Scales a base/anchor channel linearly without soft-knee compression when `gain <= 1.0`
    /// so that at unity gain (`1.0`) the base tone and transient peaks pass 100% untouched.
    #[inline(always)]
    fn scale_anchor(x: f32, gain: f32) -> f32 {
        let y = x * gain;
        if gain <= 1.0 + 1e-5 {
            y.clamp(-1.0, 1.0)
        } else {
            Self::soft_limit(y)
        }
    }

    #[inline(always)]
    fn process_routing_eq_with_flag(
        &mut self,
        l_in: f32,
        r_in: f32,
        eq_active: bool,
    ) -> (f32, f32) {
        if !eq_active {
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
        let eq_active = cfg.band_gains_db.iter().any(|&g| g.abs() >= 1e-3);
        self.process_routing_eq_with_flag(l_in, r_in, eq_active)
    }

    /// Process a stereo `(L, R)` frame for a **Bed-Only** surround system (`3..=8` channels: 4.0, 5.1, 7.1).
    /// Returns 8 bed channels `[0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:RL, 7:RR]`.
    ///
    /// Because no overhead height speakers exist in a Bed-Only layout:
    /// - Front L/R (`FL, FR`) pass the uncolored stereo anchor untouched at unity (`front_level == 1.0`).
    /// - Center (`FC`) adds in-phase steered center reinforcement above `sub_crossover_hz`.
    /// - LFE (`LFE`) adds sub-bass below `sub_crossover_hz`.
    /// - Side/Rear bed speakers (`SL, SR, RL, RR`) receive the full-bandwidth routing-shaped
    ///   and Haas-decorrelated surround signal without a `height_crossover_hz` split or height fold-down.
    #[inline(always)]
    pub fn process_stereo_to_bed_only(
        &mut self,
        l_in: f32,
        r_in: f32,
        cfg: &crate::state::SurroundUpmixConfig,
    ) -> [f32; 8] {
        let eq_active = cfg.band_gains_db.iter().any(|&g| g.abs() >= 1e-3);
        self.process_stereo_to_bed_only_core(l_in, r_in, cfg, eq_active)
    }

    #[inline(always)]
    fn process_stereo_to_bed_only_core(
        &mut self,
        l_in: f32,
        r_in: f32,
        cfg: &crate::state::SurroundUpmixConfig,
        eq_active: bool,
    ) -> [f32; 8] {
        // Keep Front L/R anchor uncolored by the surround routing EQ and free of soft-clipping at <=1.0 gain
        let fl = Self::scale_anchor(l_in, cfg.front_level);
        let fr = Self::scale_anchor(r_in, cfg.front_level);

        let (l_routed, r_routed) = self.process_routing_eq_with_flag(l_in, r_in, eq_active);
        let mid_routed = (l_routed + r_routed) * 0.5;
        let diff_routed = (l_routed - r_routed) * 0.5;

        // Smooth center-steering: anchor correlated center in FC while rejecting it from surrounds
        let common_center = self.update_center_steering(mid_routed, diff_routed);
        let fc_in = mid_routed * (0.25 + 0.75 * self.steer_gain);
        let fc_hp = self.sub_hp_c.process(fc_in);
        let fc = Self::soft_limit(fc_hp * cfg.center_level * std::f32::consts::FRAC_1_SQRT_2);

        let l_bed = self.sub_hp_l.process(l_routed - common_center * 0.72);
        let r_bed = self.sub_hp_r.process(r_routed - common_center * 0.72);
        let side_bed = (l_bed - r_bed) * 0.5 * cfg.stereo_width.clamp(0.0, 2.0);
        let l_surr = l_bed + side_bed * 0.35;
        let r_surr = r_bed - side_bed * 0.35;

        let lfe_low = self.sub_lp.process(mid_routed);
        let lfe = Self::soft_limit(lfe_low * cfg.lfe_level * std::f32::consts::FRAC_1_SQRT_2);

        let sl_raw = self.delay_sl.process(l_surr);
        let sr_raw = self.delay_sr.process(r_surr);
        let sl = Self::soft_limit(sl_raw * cfg.side_rear_level);
        let sr = Self::soft_limit(sr_raw * cfg.side_rear_level);

        let rl_raw = self.delay_rl.process(l_surr + side_bed * 0.22);
        let rr_raw = self.delay_rr.process(r_surr - side_bed * 0.22);
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
        let eq_active = cfg.band_gains_db.iter().any(|&g| g.abs() >= 1e-3);
        self.process_stereo_to_7_1_4_core(l_in, r_in, cfg, eq_active)
    }

    #[inline(always)]
    fn process_stereo_to_7_1_4_core(
        &mut self,
        l_in: f32,
        r_in: f32,
        cfg: &crate::state::SurroundUpmixConfig,
        eq_active: bool,
    ) -> [f32; 12] {
        // 1. Preserve uncolored Front L/R anchor so enabling upmixing never alters the base stereo tone
        let fl = Self::scale_anchor(l_in, cfg.front_level);
        let fr = Self::scale_anchor(r_in, cfg.front_level);

        // 2. Apply 5-band routing EQ & Mid/Side width shaping to extracted surround/height/LFE stems
        let (l_eq, r_eq) = self.process_routing_eq_with_flag(l_in, r_in, eq_active);
        let mid = (l_eq + r_eq) * 0.5;
        let diff = (l_eq - r_eq) * 0.5;

        let common_center = self.update_center_steering(mid, diff);
        let fc_in = mid * (0.25 + 0.75 * self.steer_gain);
        let fc_hp = self.sub_hp_c.process(fc_in);
        let fc = Self::soft_limit(fc_hp * cfg.center_level * std::f32::consts::FRAC_1_SQRT_2);

        let l_amb = l_eq - common_center * 0.72;
        let r_amb = r_eq - common_center * 0.72;
        let width = cfg.stereo_width.clamp(0.0, 2.0);

        // 3. LFE Subwoofer low-pass extraction (< sub_crossover_hz)
        let lfe_low = self.sub_lp.process(mid);
        let lfe = Self::soft_limit(lfe_low * cfg.lfe_level * std::f32::consts::FRAC_1_SQRT_2);

        // 4. High-pass ambient bed above sub_crossover_hz in positive polarity, then split Bed vs. Height
        // at height_crossover_hz. Blending `l_low * (1 - bleed) + l_bed_hp * bleed` keeps passband polarity
        // positive across all frequencies without a phase-cancellation notch at height_crossover_hz.
        let l_bed_hp = self.sub_hp_l.process(l_amb);
        let r_bed_hp = self.sub_hp_r.process(r_amb);
        let side_bed = (l_bed_hp - r_bed_hp) * 0.5 * width;
        let l_wide_hp = l_bed_hp + side_bed * 0.35;
        let r_wide_hp = r_bed_hp - side_bed * 0.35;
        let l_low = self.height_lp_l.process(l_bed_hp);
        let r_low = self.height_lp_r.process(r_bed_hp);
        let l_high = self.height_hp_l.process(l_wide_hp);
        let r_high = self.height_hp_r.process(r_wide_hp);

        // 5. Left/Right Hemisphere Bed Routing (< height_crossover_hz + high_bleed) & Height Routing (> height_crossover_hz)
        let high_bleed = cfg.surround_high_bleed.clamp(0.0, 1.0);
        let l_bed_band = l_low * (1.0 - high_bleed) + l_bed_hp * high_bleed;
        let r_bed_band = r_low * (1.0 - high_bleed) + r_bed_hp * high_bleed;
        let rl_bleed = high_bleed * 0.85;
        let rl_bed_band = l_low * (1.0 - rl_bleed) + l_bed_hp * rl_bleed;
        let rr_bed_band = r_low * (1.0 - rl_bleed) + r_bed_hp * rl_bleed;
        let side_low = (l_bed_band - r_bed_band) * 0.5 * width;
        let side_high = (l_high - r_high) * 0.5 * width;

        let sl_raw = self.delay_sl.process(l_bed_band);
        let sr_raw = self.delay_sr.process(r_bed_band);
        let sl = Self::soft_limit(sl_raw * cfg.side_rear_level);
        let sr = Self::soft_limit(sr_raw * cfg.side_rear_level);

        let rl_raw = self.delay_rl.process(rl_bed_band + side_low * 0.22);
        let rr_raw = self.delay_rr.process(rr_bed_band - side_low * 0.22);
        let rl = Self::soft_limit(rl_raw * (cfg.side_rear_level * 0.92));
        let rr = Self::soft_limit(rr_raw * (cfg.side_rear_level * 0.92));

        let tfl_raw = self.delay_tfl.process(l_high);
        let tfr_raw = self.delay_tfr.process(r_high);
        let tfl = Self::soft_limit(tfl_raw * cfg.height_level);
        let tfr = Self::soft_limit(tfr_raw * cfg.height_level);

        let trl_raw = self.delay_trl.process(l_high + side_high * 0.16);
        let trr_raw = self.delay_trr.process(r_high - side_high * 0.16);
        let trl = Self::soft_limit(trl_raw * (cfg.height_level * 0.92));
        let trr = Self::soft_limit(trr_raw * (cfg.height_level * 0.92));

        [fl, fr, fc, lfe, sl, sr, rl, rr, tfl, tfr, trl, trr]
    }

    /// Upmix a stereo `(l_in, r_in)` frame to `hardware_channels` (up to 12 channels):
    /// - Preserves the Front L/R anchor (`FL, FR`) untouched at unity gain (`front_level == 1.0`)
    ///   so enabling upmixing never alters the base stereo tone, and adds the extracted Center,
    ///   LFE, Surround Bed, and Height channels around the base mix.
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
        let k_bed_pair = 0.88_f32;

        if crate::state::SurroundUpmixConfig::has_height_channels(hardware_channels as i32) {
            let [fl, fr, fc, lfe, sl, sr, rl, rr, tfl, tfr, trl, trr] =
                self.process_stereo_to_7_1_4(l_in, r_in, cfg);
            let fc_c = (fc * k_bed_pair).clamp(-1.0, 1.0);
            match hardware_channels {
                9 => [
                    fl,
                    fr,
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
                    let h_l = (tfl * 0.82).clamp(-1.0, 1.0);
                    let h_r = (tfr * 0.82).clamp(-1.0, 1.0);
                    [
                        fl,
                        fr,
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
                    fl,
                    fr,
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
            let fc_c = (fc * k_bed_pair).clamp(-1.0, 1.0);
            match hardware_channels {
                3 => [
                    fl,
                    fr,
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
                    fl,
                    fr,
                    (sl * k_bed_pair).clamp(-1.0, 1.0),
                    (sr * k_bed_pair).clamp(-1.0, 1.0),
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
                    fl,
                    fr,
                    fc_c,
                    (sl * k_bed_pair).clamp(-1.0, 1.0),
                    (sr * k_bed_pair).clamp(-1.0, 1.0),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
                6 => [
                    fl,
                    fr,
                    fc_c,
                    lfe,
                    (sl * k_bed_pair).clamp(-1.0, 1.0),
                    (sr * k_bed_pair).clamp(-1.0, 1.0),
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
                        fl,
                        fr,
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
                    fl,
                    fr,
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
    /// - When `hardware_channels <= 2` or `(source_channels >= hardware_channels && cfg.is_flat_identity())`:
    ///   passes all discrete channels through 1:1 untouched.
    /// - When `source_channels <= 2`: performs additive stereo-to-surround upmixing across all
    ///   `hardware_channels` while preserving the uncolored Front L/R anchor (`FL, FR`).
    /// - When `source_channels > 2`: preserves existing discrete base channels at full bandwidth and unity
    ///   gain (at default sliders), maps 7.1 `WAVEFORMATEXTENSIBLE` `0x63F` (`[4:BL, 5:BR, 6:SL, 7:SR]`)
    ///   into canonical `[4:SL, 5:SR, 6:BL, 7:BR]` when expanding to `> 8` channels, and synthesizes only
    ///   the missing surround/height channels using the 5-band routing EQ and crossovers.
    #[inline]
    pub fn process_multichannel_frame_for_hardware(
        &mut self,
        frame_slice: &[f32],
        source_channels: usize,
        cfg: &crate::state::SurroundUpmixConfig,
        hardware_channels: usize,
    ) -> [f32; 12] {
        let src_ch = source_channels.max(1);
        if hardware_channels <= 2 || (src_ch >= hardware_channels && cfg.is_flat_identity()) {
            let mut out = [0.0f32; 12];
            let copy_len = frame_slice.len().min(hardware_channels).min(12);
            out[..copy_len].copy_from_slice(&frame_slice[..copy_len]);
            return out;
        }
        if src_ch <= 2 {
            let (l_in, r_in) = extract_stereo_pair_from_frame(frame_slice, src_ch);
            return self.process_frame_for_hardware(l_in, r_in, cfg, hardware_channels);
        }

        // Keep raw discrete channels for base preservation, and apply 5-band routing EQ for upmixed stems
        let eq_active = cfg.band_gains_db.iter().any(|&g| g.abs() >= 1e-3);
        let mut ch_raw = [0.0f32; 12];
        let mut ch_in = [0.0f32; 12];
        let in_count = src_ch.min(frame_slice.len()).min(12);
        for c in 0..in_count {
            let raw = frame_slice[c];
            ch_raw[c] = raw;
            let mut s = raw;
            if eq_active {
                for bq in self.eq_mc[c].iter_mut() {
                    s = bq.process(s);
                }
                s = Self::soft_limit(s);
            }
            ch_in[c] = s;
        }

        // Normalized discrete stem gains (1.0 at factory default slider positions)
        let front_gain = cfg.front_level.clamp(0.0, 2.0);
        let center_gain = (cfg.center_level / 0.45).clamp(0.0, 2.5);
        let lfe_gain = (cfg.lfe_level / 0.85).clamp(0.0, 2.0);
        let surround_gain = (cfg.side_rear_level / 0.85).clamp(0.0, 2.0);
        let height_gain = (cfg.height_level / 0.95).clamp(0.0, 2.0);

        let fl = Self::scale_anchor(ch_raw[0], front_gain);
        let fr = Self::scale_anchor(ch_raw[1], front_gain);
        let sub_redirect = self.mc_sub_lp.process((ch_in[0] + ch_in[1]) * 0.5);
        let sub_trim = if cfg.sub_crossover_hz > 90.5 {
            sub_redirect * 0.25 * ((cfg.sub_crossover_hz - 90.0) / 160.0)
        } else {
            0.0
        };
        const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

        // Pass pre-EQ'd signals to stereo helper with zeroed band_gains_db and unity stereo_width
        // so it doesn't double-EQ or double-widen before apply_multichannel_surround_width.
        let mut helper_cfg = *cfg;
        helper_cfg.band_gains_db = [0.0; 5];
        helper_cfg.stereo_width = 1.0;

        let mut out = match src_ch {
            // 3.0 (FL, FR, FC) -> 4.0 / 5.0 / 5.1 / 6.1 / 7.1 / 7.1.x:
            3 => {
                let fc = Self::scale_anchor(ch_raw[2], center_gain);
                let mut out = self.process_frame_for_hardware(
                    ch_in[0],
                    ch_in[1],
                    &helper_cfg,
                    hardware_channels,
                );
                if hardware_channels == 4 {
                    out[0] = (fl + fc * INV_SQRT2).clamp(-1.0, 1.0);
                    out[1] = (fr + fc * INV_SQRT2).clamp(-1.0, 1.0);
                } else {
                    out[0] = fl;
                    out[1] = fr;
                    out[2] = fc;
                }
                out
            }
            // 4.0 Quad (FL, FR, SL, SR) -> 4.0 / 5.0 / 5.1 / 6.1 / 7.1 / 7.1.x:
            4 => {
                let sl = Self::scale_anchor(ch_raw[2], surround_gain);
                let sr = Self::scale_anchor(ch_raw[3], surround_gain);
                let l_comb = (ch_in[0] + ch_in[2] * INV_SQRT2) * 0.75;
                let r_comb = (ch_in[1] + ch_in[3] * INV_SQRT2) * 0.75;
                let mut out =
                    self.process_frame_for_hardware(l_comb, r_comb, &helper_cfg, hardware_channels);
                out[0] = fl;
                out[1] = fr;
                if hardware_channels <= 4 {
                    out[2] = sl;
                    out[3] = sr;
                } else if hardware_channels == 5 {
                    out[3] = sl;
                    out[4] = sr;
                } else {
                    out[4] = sl;
                    out[5] = sr;
                }
                out
            }
            // 5.0 (FL, FR, FC, SL, SR) -> 5.0 / 5.1 / 6.1 / 7.1 / 7.1.x:
            5 => {
                let fc = Self::scale_anchor(ch_raw[2], center_gain);
                let sl = Self::scale_anchor(ch_raw[3], surround_gain);
                let sr = Self::scale_anchor(ch_raw[4], surround_gain);
                let l_comb = (ch_in[0] + ch_in[2] * 0.5 + ch_in[3] * INV_SQRT2) * 0.7;
                let r_comb = (ch_in[1] + ch_in[2] * 0.5 + ch_in[4] * INV_SQRT2) * 0.7;
                let mut out =
                    self.process_frame_for_hardware(l_comb, r_comb, &helper_cfg, hardware_channels);
                out[0] = fl;
                out[1] = fr;
                out[2] = fc;
                if hardware_channels <= 5 {
                    out[3] = sl;
                    out[4] = sr;
                } else {
                    out[4] = sl;
                    out[5] = sr;
                }
                out
            }
            // 5.1 (FL, FR, FC, LFE, SL, SR) -> 5.1 / 6.1 / 7.1 / 7.1.x:
            6 => {
                if hardware_channels <= 6 {
                    let mut out = [0.0f32; 12];
                    out[0] = Self::scale_anchor(ch_in[0], front_gain);
                    out[1] = Self::scale_anchor(ch_in[1], front_gain);
                    out[2] = Self::scale_anchor(ch_in[2], center_gain);
                    out[3] = Self::scale_anchor(ch_in[3] + sub_trim, lfe_gain);
                    out[4] = Self::scale_anchor(ch_in[4], surround_gain);
                    out[5] = Self::scale_anchor(ch_in[5], surround_gain);
                    Self::apply_multichannel_surround_width(
                        &mut out,
                        hardware_channels,
                        cfg.stereo_width,
                    );
                    return out;
                }
                let fc = Self::scale_anchor(ch_raw[2], center_gain);
                let lfe = Self::scale_anchor(ch_raw[3] + sub_trim, lfe_gain);
                let sl = Self::scale_anchor(ch_raw[4], surround_gain);
                let sr = Self::scale_anchor(ch_raw[5], surround_gain);
                let l_comb = (ch_in[4] + ch_in[0] * 0.55) * 0.75;
                let r_comb = (ch_in[5] + ch_in[1] * 0.55) * 0.75;
                let mut out =
                    self.process_frame_for_hardware(l_comb, r_comb, &helper_cfg, hardware_channels);
                out[0] = fl;
                out[1] = fr;
                out[2] = fc;
                out[3] = lfe;
                out[4] = sl;
                out[5] = sr;
                out
            }
            // 6.1 (FL, FR, FC, LFE, BC, SL, SR) -> 6.1 / 7.1 / 7.1.x:
            7 => {
                let fc = Self::scale_anchor(ch_raw[2], center_gain);
                let lfe = Self::scale_anchor(ch_raw[3] + sub_trim, lfe_gain);
                let bc = ch_in[4] * surround_gain;
                let sl = Self::scale_anchor(ch_raw[5], surround_gain);
                let sr = Self::scale_anchor(ch_raw[6], surround_gain);
                let mut out = self.process_frame_for_hardware(
                    (ch_in[5] + ch_in[4] * INV_SQRT2) * INV_SQRT2,
                    (ch_in[6] + ch_in[4] * INV_SQRT2) * INV_SQRT2,
                    &helper_cfg,
                    hardware_channels,
                );
                out[0] = fl;
                out[1] = fr;
                out[2] = fc;
                out[3] = lfe;
                out[4] = sl;
                out[5] = sr;
                out[6] = Self::soft_limit((out[6] + bc * INV_SQRT2) * 0.75);
                if hardware_channels >= 8 {
                    out[7] = Self::soft_limit((out[7] + bc * INV_SQRT2) * 0.75);
                }
                out
            }
            // >= 8 channels (e.g. 7.1 on 7.1, 7.1.4 on 7.1.4, or 7.1 -> 7.1.2 / 7.1.4):
            _ => {
                if hardware_channels <= 8 || src_ch >= hardware_channels {
                    let mut out = [0.0f32; 12];
                    out[0] = Self::scale_anchor(ch_in[0], front_gain);
                    out[1] = Self::scale_anchor(ch_in[1], front_gain);
                    out[2] = Self::scale_anchor(ch_in[2], center_gain);
                    out[3] = Self::scale_anchor(ch_in[3] + sub_trim, lfe_gain);
                    if src_ch >= 8 && hardware_channels > 8 {
                        out[4] = Self::scale_anchor(ch_in[6], surround_gain);
                        out[5] = Self::scale_anchor(ch_in[7], surround_gain);
                        out[6] = Self::scale_anchor(ch_in[4], surround_gain);
                        out[7] = Self::scale_anchor(ch_in[5], surround_gain);
                    } else {
                        for c in 4..hardware_channels.min(8) {
                            out[c] = Self::scale_anchor(ch_in[c], surround_gain);
                        }
                    }
                    for c in 8..hardware_channels.min(12) {
                        out[c] = Self::scale_anchor(ch_in[c], height_gain);
                    }
                    Self::apply_multichannel_surround_width(
                        &mut out,
                        hardware_channels,
                        cfg.stereo_width,
                    );
                    return out;
                }

                let fc = Self::scale_anchor(ch_raw[2], center_gain);
                let lfe = Self::scale_anchor(ch_raw[3] + sub_trim, lfe_gain);

                // Decoded 8+ channel (7.1 / 7.1.4) frames follow WAVEFORMATEXTENSIBLE 0x63F order in 0..8:
                // [0:FL, 1:FR, 2:FC, 3:LFE, 4:BL, 5:BR, 6:SL, 7:SR].
                // Map them to canonical Side (SL, SR) and Back/Rear (BL, BR) speaker pairs:
                let (bl_raw, br_raw, sl_raw, sr_raw) = if src_ch >= 8 {
                    (ch_raw[4], ch_raw[5], ch_raw[6], ch_raw[7])
                } else {
                    (ch_raw[6], ch_raw[7], ch_raw[4], ch_raw[5])
                };
                let (bl_in, br_in, sl_in, sr_in) = if src_ch >= 8 {
                    (ch_in[4], ch_in[5], ch_in[6], ch_in[7])
                } else {
                    (ch_in[6], ch_in[7], ch_in[4], ch_in[5])
                };

                // Preserve discrete 7.1 bed channels (SL, SR, BL, BR) at full bandwidth so enabling
                // 7.1.4 upmixing never strips highs out of the ear-level 7.1 bed, and extract
                // positive-polarity high-passed (> height_crossover_hz) stems for the overhead heights.
                let sl_out = Self::scale_anchor(sl_raw, surround_gain);
                let sr_out = Self::scale_anchor(sr_raw, surround_gain);
                let bl_out = Self::scale_anchor(bl_raw, surround_gain);
                let br_out = Self::scale_anchor(br_raw, surround_gain);

                let sl_high = self.mc_height_hp[0].process(sl_in);
                let sr_high = self.mc_height_hp[1].process(sr_in);
                let bl_high = self.mc_height_hp[2].process(bl_in);
                let br_high = self.mc_height_hp[3].process(br_in);

                let f_mid = (ch_in[0] + ch_in[1]) * 0.5;
                let f_diff = (ch_in[0] - ch_in[1]) * 0.5;
                let f_common = self.update_center_steering(f_mid, f_diff);
                let f_l_high = self.height_hp_l.process(ch_in[0] - f_common * 0.78);
                let f_r_high = self.height_hp_r.process(ch_in[1] - f_common * 0.78);

                let tfl = Self::soft_limit((sl_high * 0.82 + f_l_high * 0.28) * height_gain);
                let tfr = Self::soft_limit((sr_high * 0.82 + f_r_high * 0.28) * height_gain);
                let tbl = Self::soft_limit((bl_high * 0.82 + sl_high * 0.18) * height_gain);
                let tbr = Self::soft_limit((br_high * 0.82 + sr_high * 0.18) * height_gain);

                match hardware_channels {
                    9 => [
                        fl,
                        fr,
                        fc,
                        lfe,
                        sl_out,
                        sr_out,
                        bl_out,
                        br_out,
                        ((tfl + tfr) * 0.5).clamp(-1.0, 1.0),
                        0.0,
                        0.0,
                        0.0,
                    ],
                    10 => [
                        fl,
                        fr,
                        fc,
                        lfe,
                        sl_out,
                        sr_out,
                        bl_out,
                        br_out,
                        ((tfl + tbl * 0.7) * 0.65).clamp(-1.0, 1.0),
                        ((tfr + tbr * 0.7) * 0.65).clamp(-1.0, 1.0),
                        0.0,
                        0.0,
                    ],
                    _ => [
                        fl, fr, fc, lfe, sl_out, sr_out, bl_out, br_out, tfl, tfr, tbl, tbr,
                    ],
                }
            }
        };
        Self::apply_multichannel_surround_width(&mut out, hardware_channels, cfg.stereo_width);
        out
    }

    #[inline]
    fn apply_multichannel_surround_width(
        out: &mut [f32; 12],
        hardware_channels: usize,
        stereo_width: f32,
    ) {
        let w = stereo_width.clamp(0.0, 2.0);
        if (w - 1.0).abs() < 1e-4 {
            return;
        }
        let apply_pair = |buf: &mut [f32; 12], li: usize, ri: usize| {
            let mid = (buf[li] + buf[ri]) * 0.5;
            let side = (buf[li] - buf[ri]) * 0.5 * w;
            buf[li] = Self::soft_limit(mid + side);
            buf[ri] = Self::soft_limit(mid - side);
        };
        if hardware_channels == 4 {
            apply_pair(out, 2, 3);
        } else if hardware_channels == 5 {
            apply_pair(out, 3, 4);
        } else if hardware_channels >= 6 {
            apply_pair(out, 4, 5);
            if hardware_channels >= 8 {
                apply_pair(out, 6, 7);
            }
            if hardware_channels >= 10 {
                apply_pair(out, 8, 9);
            }
            if hardware_channels >= 12 {
                apply_pair(out, 10, 11);
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
        for ch_eq in self.eq_mc.iter_mut() {
            for f in ch_eq.iter_mut() {
                f.clear_state();
            }
        }
        self.sub_lp.clear_state();
        self.mc_sub_lp.clear_state();
        self.sub_hp_l.clear_state();
        self.sub_hp_r.clear_state();
        self.sub_hp_c.clear_state();
        self.height_lp_l.clear_state();
        self.height_hp_l.clear_state();
        self.height_lp_r.clear_state();
        self.height_hp_r.clear_state();
        for hp in self.mc_height_hp.iter_mut() {
            hp.clear_state();
        }
        self.delay_sl.clear();
        self.delay_sr.clear();
        self.delay_rl.clear();
        self.delay_rr.clear();
        self.delay_tfl.clear();
        self.delay_tfr.clear();
        self.delay_trl.clear();
        self.delay_trr.clear();
        self.mid_env = 0.0;
        self.side_env = 0.0;
        self.steer_gain = 0.0;
    }

    /// Returns `true` when upmixing is either actively enabled or in the middle of a smooth
    /// 65 ms raised-cosine crossfade transition (`blend > 0.0`).
    #[inline(always)]
    pub fn is_blending_or_active(&self, target_active: bool) -> bool {
        target_active || !self.blend_initialized || self.blend > 0.0
    }

    /// Returns `true` when steady-state active upmixing has finished crossfading (`blend == 1.0`).
    #[inline(always)]
    pub fn is_fully_active(&self, target_active: bool) -> bool {
        target_active && self.blend_initialized && (self.blend - 1.0).abs() < 1e-6
    }

    /// Block-processes interleaved mono/stereo frames (`src_stride` floats per input frame) into
    /// canonical 12-channel output frames, hoisting configuration checks and channel-topology
    /// branching outside the per-sample loop. Matches [`Self::process_frame_for_hardware`] bit-for-bit.
    pub fn process_interleaved_stereo_block(
        &mut self,
        in_frames: &[f32],
        src_stride: usize,
        src_channels: usize,
        out_frames: &mut [[f32; 12]],
        cfg: &crate::state::SurroundUpmixConfig,
        hardware_channels: usize,
    ) {
        if src_stride == 0 || out_frames.is_empty() {
            return;
        }
        let frame_count = out_frames.len().min(in_frames.len() / src_stride);
        let eq_active = cfg.band_gains_db.iter().any(|&g| g.abs() >= 1e-3);
        let mono_src = src_channels <= 1 || src_stride < 2;
        const K_BED_PAIR: f32 = 0.88_f32;

        #[inline(always)]
        fn read_lr(frame: &[f32], mono_src: bool) -> (f32, f32) {
            let l = frame[0];
            let r = if mono_src { l } else { frame[1] };
            (l, r)
        }

        if hardware_channels == 1 {
            for (dst, src) in out_frames[..frame_count]
                .iter_mut()
                .zip(in_frames.chunks_exact(src_stride))
            {
                let (l, r) = read_lr(src, mono_src);
                let mono = ((l + r) * 0.5).clamp(-1.0, 1.0);
                *dst = [mono, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            }
            return;
        }
        if hardware_channels <= 2 {
            for (dst, src) in out_frames[..frame_count]
                .iter_mut()
                .zip(in_frames.chunks_exact(src_stride))
            {
                let (l, r) = read_lr(src, mono_src);
                *dst = [l, r, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            }
            return;
        }

        if crate::state::SurroundUpmixConfig::has_height_channels(hardware_channels as i32) {
            match hardware_channels {
                9 => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, fc, lfe, sl, sr, rl, rr, tfl, tfr, _, _] =
                            self.process_stereo_to_7_1_4_core(l, r, cfg, eq_active);
                        *dst = [
                            fl,
                            fr,
                            (fc * K_BED_PAIR).clamp(-1.0, 1.0),
                            lfe,
                            (sl * 0.64).clamp(-1.0, 1.0),
                            (sr * 0.64).clamp(-1.0, 1.0),
                            (rl * 0.64).clamp(-1.0, 1.0),
                            (rr * 0.64).clamp(-1.0, 1.0),
                            ((tfl + tfr) * 0.45).clamp(-1.0, 1.0),
                            0.0,
                            0.0,
                            0.0,
                        ];
                    }
                }
                10 => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, fc, lfe, sl, sr, rl, rr, tfl, tfr, _, _] =
                            self.process_stereo_to_7_1_4_core(l, r, cfg, eq_active);
                        *dst = [
                            fl,
                            fr,
                            (fc * K_BED_PAIR).clamp(-1.0, 1.0),
                            lfe,
                            (sl * 0.64).clamp(-1.0, 1.0),
                            (sr * 0.64).clamp(-1.0, 1.0),
                            (rl * 0.64).clamp(-1.0, 1.0),
                            (rr * 0.64).clamp(-1.0, 1.0),
                            (tfl * 0.82).clamp(-1.0, 1.0),
                            (tfr * 0.82).clamp(-1.0, 1.0),
                            0.0,
                            0.0,
                        ];
                    }
                }
                _ => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, fc, lfe, sl, sr, rl, rr, tfl, tfr, trl, trr] =
                            self.process_stereo_to_7_1_4_core(l, r, cfg, eq_active);
                        *dst = [
                            fl,
                            fr,
                            (fc * K_BED_PAIR).clamp(-1.0, 1.0),
                            lfe,
                            (sl * 0.64).clamp(-1.0, 1.0),
                            (sr * 0.64).clamp(-1.0, 1.0),
                            (rl * 0.64).clamp(-1.0, 1.0),
                            (rr * 0.64).clamp(-1.0, 1.0),
                            (tfl * 0.82).clamp(-1.0, 1.0),
                            (tfr * 0.82).clamp(-1.0, 1.0),
                            (trl * 0.66).clamp(-1.0, 1.0),
                            (trr * 0.66).clamp(-1.0, 1.0),
                        ];
                    }
                }
            }
        } else {
            match hardware_channels {
                3 => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, fc, ..] =
                            self.process_stereo_to_bed_only_core(l, r, cfg, eq_active);
                        *dst = [
                            fl,
                            fr,
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
                        ];
                    }
                }
                4 => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, _, _, sl, sr, ..] =
                            self.process_stereo_to_bed_only_core(l, r, cfg, eq_active);
                        *dst = [
                            fl,
                            fr,
                            (sl * K_BED_PAIR).clamp(-1.0, 1.0),
                            (sr * K_BED_PAIR).clamp(-1.0, 1.0),
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                        ];
                    }
                }
                5 => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, fc, _, sl, sr, ..] =
                            self.process_stereo_to_bed_only_core(l, r, cfg, eq_active);
                        *dst = [
                            fl,
                            fr,
                            (fc * K_BED_PAIR).clamp(-1.0, 1.0),
                            (sl * K_BED_PAIR).clamp(-1.0, 1.0),
                            (sr * K_BED_PAIR).clamp(-1.0, 1.0),
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                        ];
                    }
                }
                6 => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, fc, lfe, sl, sr, ..] =
                            self.process_stereo_to_bed_only_core(l, r, cfg, eq_active);
                        *dst = [
                            fl,
                            fr,
                            (fc * K_BED_PAIR).clamp(-1.0, 1.0),
                            lfe,
                            (sl * K_BED_PAIR).clamp(-1.0, 1.0),
                            (sr * K_BED_PAIR).clamp(-1.0, 1.0),
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                        ];
                    }
                }
                7 => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, fc, lfe, sl, sr, rl, rr] =
                            self.process_stereo_to_bed_only_core(l, r, cfg, eq_active);
                        let rc = ((rl + rr) * 0.5).clamp(-1.0, 1.0);
                        *dst = [
                            fl,
                            fr,
                            (fc * K_BED_PAIR).clamp(-1.0, 1.0),
                            lfe,
                            (sl * 0.62).clamp(-1.0, 1.0),
                            (sr * 0.62).clamp(-1.0, 1.0),
                            (rc * 0.62).clamp(-1.0, 1.0),
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                        ];
                    }
                }
                _ => {
                    for (dst, src) in out_frames[..frame_count]
                        .iter_mut()
                        .zip(in_frames.chunks_exact(src_stride))
                    {
                        let (l, r) = read_lr(src, mono_src);
                        let [fl, fr, fc, lfe, sl, sr, rl, rr] =
                            self.process_stereo_to_bed_only_core(l, r, cfg, eq_active);
                        *dst = [
                            fl,
                            fr,
                            (fc * K_BED_PAIR).clamp(-1.0, 1.0),
                            lfe,
                            (sl * 0.64).clamp(-1.0, 1.0),
                            (sr * 0.64).clamp(-1.0, 1.0),
                            (rl * 0.64).clamp(-1.0, 1.0),
                            (rr * 0.64).clamp(-1.0, 1.0),
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                        ];
                    }
                }
            }
        }
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
        let effective_target = target_active
            && hardware_channels > 2
            && (src_ch < hardware_channels || !cfg.is_flat_identity());
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

        if self.blend <= 0.0
            || hardware_channels <= 2
            || (src_ch >= hardware_channels && cfg.is_flat_identity())
        {
            let mut out = [0.0f32; 12];
            let copy_len = frame_slice.len().min(hardware_channels).min(12);
            out[..copy_len].copy_from_slice(&frame_slice[..copy_len]);
            if src_ch >= 8 && hardware_channels > 8 && frame_slice.len() >= 8 {
                out.swap(4, 6);
                out.swap(5, 7);
            }
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
                let dry = if c < src_ch && c < hardware_channels {
                    let src_c = if src_ch >= 8 && hardware_channels > 8 {
                        match c {
                            4 => 6,
                            5 => 7,
                            6 => 4,
                            7 => 5,
                            _ => c,
                        }
                    } else {
                        c
                    };
                    frame_slice.get(src_c).copied().unwrap_or(0.0)
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
