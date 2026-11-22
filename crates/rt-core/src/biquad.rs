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

    /// Evaluates `|H(e^{jw})|^2` of the quantized `f32` biquad coefficients on the unit circle
    /// in `f64` precision to avoid catastrophic cancellation near DC (`20..100 Hz`).
    #[inline]
    pub fn magnitude_squared_unit_circle(
        &self,
        cos_w: f64,
        sin_w: f64,
        cos_2w: f64,
        sin_2w: f64,
    ) -> f64 {
        let b0 = self.b0 as f64;
        let b1 = self.b1 as f64;
        let b2 = self.b2 as f64;
        let a1 = self.a1 as f64;
        let a2 = self.a2 as f64;
        let num_re = b0 + b1 * cos_w + b2 * cos_2w;
        let num_im = -(b1 * sin_w + b2 * sin_2w);
        let den_re = 1.0 + a1 * cos_w + a2 * cos_2w;
        let den_im = -(a1 * sin_w + a2 * sin_2w);
        ((num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im).max(1e-24))
            .max(1e-24)
    }
}

/// 4th-order Linkwitz-Riley crossover filter (-24 dB/octave), formed by cascading
/// two identical 2nd-order Butterworth sections (`Q = 1 / sqrt(2)`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Lr4Filter {
    s1: BiquadFilter,
    s2: BiquadFilter,
}

impl Lr4Filter {
    #[inline]
    pub fn lowpass(sample_rate: f32, freq_hz: f32) -> Self {
        let stage = BiquadFilter::lowpass(sample_rate, freq_hz, std::f32::consts::FRAC_1_SQRT_2);
        Self {
            s1: stage,
            s2: stage,
        }
    }

    #[inline]
    pub fn highpass(sample_rate: f32, freq_hz: f32) -> Self {
        let stage = BiquadFilter::highpass(sample_rate, freq_hz, std::f32::consts::FRAC_1_SQRT_2);
        Self {
            s1: stage,
            s2: stage,
        }
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        self.s2.process(self.s1.process(x))
    }

    #[inline]
    pub fn update_coeffs(&mut self, coeffs: Self) {
        self.s1.update_coeffs(coeffs.s1);
        self.s2.update_coeffs(coeffs.s2);
    }

    #[inline]
    pub fn clear_state(&mut self) {
        self.s1.clear_state();
        self.s2.clear_state();
    }

    #[inline]
    pub fn magnitude_squared_unit_circle(
        &self,
        cos_w: f64,
        sin_w: f64,
        cos_2w: f64,
        sin_2w: f64,
    ) -> f64 {
        self.s1
            .magnitude_squared_unit_circle(cos_w, sin_w, cos_2w, sin_2w)
            * self
                .s2
                .magnitude_squared_unit_circle(cos_w, sin_w, cos_2w, sin_2w)
    }
}

#[cfg(test)]
mod tests {
    use super::Lr4Filter;

    #[test]
    fn test_lr4_crossover_flat_sum_and_rolloff() {
        let sr = 48_000.0f32;
        let fc = 1_000.0f32;

        // Complex-valued unit-circle response check isn't needed for steady-state RMS sum
        // when driving steady sine waves through LP and HP in parallel.
        for &freq in &[125.0f32, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0] {
            let mut lp = Lr4Filter::lowpass(sr, fc);
            let mut hp = Lr4Filter::highpass(sr, fc);
            let mut sum_sq = 0.0f64;
            let mut lp_sq = 0.0f64;
            let mut hp_sq = 0.0f64;
            let mut in_sq = 0.0f64;
            let total_samples = 8192usize;
            let settle_samples = 4096usize;
            for n in 0..total_samples {
                let x = (2.0 * std::f32::consts::PI * freq * (n as f32 / sr)).sin();
                let y_lp = lp.process(x);
                let y_hp = hp.process(x);
                if n >= settle_samples {
                    let s = (y_lp + y_hp) as f64;
                    sum_sq += s * s;
                    lp_sq += (y_lp as f64) * (y_lp as f64);
                    hp_sq += (y_hp as f64) * (y_hp as f64);
                    in_sq += (x as f64) * (x as f64);
                }
            }
            let sum_gain = (sum_sq / in_sq).sqrt();
            assert!(
                (sum_gain - 1.0).abs() < 1e-2,
                "LR4 LP+HP sum at {freq} Hz must be flat (~1.0), got {sum_gain}"
            );
            if (freq - fc).abs() < 1e-3 {
                let lp_gain = (lp_sq / in_sq).sqrt();
                let hp_gain = (hp_sq / in_sq).sqrt();
                assert!(
                    (lp_gain - 0.5).abs() < 1e-2 && (hp_gain - 0.5).abs() < 1e-2,
                    "At fc ({fc} Hz), LR4 LP ({lp_gain}) and HP ({hp_gain}) must each be -6 dB (0.5)"
                );
            }
        }

        // Verify -24 dB/octave stopband attenuation (-48 dB two octaves away at 250 Hz / 4000 Hz)
        let w_stop = 2.0 * std::f64::consts::PI * 250.0 / sr as f64;
        let hp_stop = Lr4Filter::highpass(sr, fc).magnitude_squared_unit_circle(
            w_stop.cos(),
            w_stop.sin(),
            (2.0 * w_stop).cos(),
            (2.0 * w_stop).sin(),
        );
        let db_250 = 10.0 * hp_stop.log10();
        assert!(
            db_250 < -46.0 && db_250 > -50.0,
            "LR4 2 octaves below fc must be ~-48 dB (got {db_250:.2} dB)"
        );
    }
}
