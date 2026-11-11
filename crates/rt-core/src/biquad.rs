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
