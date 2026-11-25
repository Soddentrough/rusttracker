pub const FLIP_DISC_GRID_W: u32 = 96;
pub const FLIP_DISC_GRID_H: u32 = 64;
pub const FLIP_DISC_COUNT: u32 = FLIP_DISC_GRID_W * FLIP_DISC_GRID_H;
pub const FLIP_DISC_WORKGROUPS_X: u32 = FLIP_DISC_GRID_W / 16;
pub const FLIP_DISC_WORKGROUPS_Y: u32 = FLIP_DISC_GRID_H / 16;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FlipDiscParams {
    /// Four circular ripple impulses: `[gx, gy, radius, strength]`.
    pub impulses: [[f32; 4]; 4],
    /// Normalized audio metrics: `[bass, mids, treble, overall]`.
    pub audio_levels: [f32; 4],
    /// Simulation metadata: `[dt, time_s, aspect_ratio, vis_width]`.
    pub sim_info: [f32; 4],
}

/// CPU-side onset detector, stereo ripple spawner, and fixed-60 Hz wave substep scheduler
/// for the Mechanical Flip-Disc visualizer (ID 32).
#[derive(Clone, Debug, Default)]
pub struct FlipDiscSimState {
    pub bass_env: f32,
    pub mid_env: f32,
    pub treble_env: f32,
    pub overall_env: f32,
    pub left_env: f32,
    pub right_env: f32,
    pub bass_cd: f32,
    pub mid_cd: f32,
    pub treble_cd: f32,
    pub ambient_cd: f32,
    pub wave_accum: f32,
    pub emit_seq: u32,
    pub pending_impulses: [[f32; 4]; 4],
}

#[inline]
fn r2_offset(seq: u32) -> (f32, f32) {
    let s = (seq as f32) + 1.0;
    let fx = (0.5 + s * 0.754_877_67).fract() * 2.0 - 1.0;
    let fy = (0.5 + s * 0.569_840_29).fract() * 2.0 - 1.0;
    (fx, fy)
}

#[inline]
fn band_energy(spectrum: &[f32], start: usize, end_exclusive: usize) -> f32 {
    if spectrum.is_empty() || start >= spectrum.len() {
        return 0.0;
    }
    let hi = end_exclusive.min(spectrum.len());
    if hi <= start {
        return 0.0;
    }
    let slice = &spectrum[start..hi];
    let mut peak = 0.0f32;
    let mut sum = 0.0f32;
    for &v in slice {
        let clamped = v.max(0.0);
        peak = peak.max(clamped);
        sum += clamped;
    }
    let avg = sum / slice.len() as f32;
    peak * 0.55 + avg * 0.45
}

impl FlipDiscSimState {
    /// Computes the visible grid span `(vis_w, vis_h)` centered at `(48.0, 32.0)` matching
    /// `vis_flip_disc.wgsl` so all spawned ripples land inside the active viewport.
    #[inline]
    pub fn visible_grid_span(aspect_ratio: f32) -> (f32, f32) {
        let aspect = aspect_ratio.max(0.35);
        let vis_h = (22.0 / aspect.sqrt()).clamp(12.5, 30.0);
        let vis_w = (vis_h * aspect).min(80.0);
        (vis_w, vis_h)
    }

    /// Advances audio onset detection and fixed-60 Hz wave substep scheduling, returning
    /// `(params, wave_substeps)` for GPU compute dispatch.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        spectrum: &[f32],
        channel_vus: &[f32],
        dt_raw: f32,
        aspect_ratio: f32,
        is_playing: bool,
        time_s: f32,
        initialized: bool,
    ) -> (FlipDiscParams, u32) {
        const STEP_DT: f32 = 1.0 / 60.0;
        let dt = dt_raw.clamp(0.001, 0.033);
        let aspect = aspect_ratio.max(0.35);
        let (vis_w, vis_h) = Self::visible_grid_span(aspect);

        let bass = (band_energy(spectrum, 1, 18) / 55.0).clamp(0.0, 1.5);
        let mids = (band_energy(spectrum, 24, 140) / 50.0).clamp(0.0, 1.5);
        let treble = (band_energy(spectrum, 180, 480) / 42.0).clamp(0.0, 1.5);

        let left_vu = channel_vus.first().copied().unwrap_or(0.0).clamp(0.0, 1.5);
        let right_vu = channel_vus
            .get(1)
            .copied()
            .unwrap_or(left_vu)
            .clamp(0.0, 1.5);
        let stereo_avg = (left_vu + right_vu) * 0.5;
        let overall = (bass * 0.40 + mids * 0.32 + treble * 0.28)
            .max(stereo_avg * 0.85)
            .clamp(0.0, 1.5);
        let stereo_bias = (right_vu - left_vu).clamp(-1.0, 1.0);

        if !is_playing {
            let decay = (-dt * 8.0).exp();
            self.bass_env *= decay;
            self.mid_env *= decay;
            self.treble_env *= decay;
            self.overall_env *= decay;
            self.left_env *= decay;
            self.right_env *= decay;
            self.wave_accum = 0.0;
            self.pending_impulses = [[0.0; 4]; 4];
            return (
                FlipDiscParams {
                    impulses: [[0.0; 4]; 4],
                    audio_levels: [0.0; 4],
                    sim_info: [dt, time_s, aspect, vis_w],
                },
                0,
            );
        }

        self.bass_cd = (self.bass_cd - dt).max(0.0);
        self.mid_cd = (self.mid_cd - dt).max(0.0);
        self.treble_cd = (self.treble_cd - dt).max(0.0);
        self.ambient_cd = (self.ambient_cd - dt).max(0.0);

        let cx = (FLIP_DISC_GRID_W as f32) * 0.5;
        let cy = (FLIP_DISC_GRID_H as f32) * 0.5;
        let max_dx = (vis_w * 0.44).min(cx - 5.0);
        let max_dy = (vis_h * 0.42).min(cy - 5.0);

        let clamp_grid = |dx: f32, dy: f32| -> (f32, f32) {
            (
                cx + dx.clamp(-max_dx, max_dx),
                cy + dy.clamp(-max_dy, max_dy),
            )
        };

        let mut any_pending = self.pending_impulses.iter().any(|imp| imp[3] > 0.0);

        if overall >= 0.015 {
            // Slot 0: Bass kick onset near center, gently nudged by stereo balance
            if bass > (self.bass_env * 1.18 + 0.06) && self.bass_cd <= 0.0 {
                self.emit_seq = self.emit_seq.wrapping_add(1);
                let (fx, fy) = r2_offset(self.emit_seq);
                let (gx, gy) = clamp_grid(
                    (fx * 0.24 + stereo_bias * 0.14) * vis_w,
                    fy * 0.20 * vis_h,
                );
                let radius = (2.6 + bass * 0.5).clamp(2.6, 3.3);
                let strength = (1.65 + bass * 0.65).clamp(1.65, 2.55);
                if strength > self.pending_impulses[0][3] {
                    self.pending_impulses[0] = [gx, gy, radius, strength];
                }
                self.bass_cd = 0.24;
                any_pending = true;
            }

            // Slot 1: Mid / snare / stereo transient biased toward active stereo channel
            if mids > (self.mid_env * 1.15 + 0.05) && self.mid_cd <= 0.0 {
                self.emit_seq = self.emit_seq.wrapping_add(1);
                let (fx, fy) = r2_offset(self.emit_seq);
                let (gx, gy) = clamp_grid(
                    (fx * 0.28 + stereo_bias * 0.34) * vis_w,
                    fy * 0.32 * vis_h,
                );
                let radius = (1.8 + mids * 0.35).clamp(1.8, 2.3);
                let strength = (1.15 + mids * 0.55).clamp(1.15, 1.85);
                if strength > self.pending_impulses[1][3] {
                    self.pending_impulses[1] = [gx, gy, radius, strength];
                }
                self.mid_cd = 0.17;
                any_pending = true;
            }

            // Slot 2: Treble / hi-hat sparkle distributed across the visible disc wall
            if treble > (self.treble_env * 1.16 + 0.045) && self.treble_cd <= 0.0 {
                self.emit_seq = self.emit_seq.wrapping_add(1);
                let (fx, fy) = r2_offset(self.emit_seq);
                let (gx, gy) = clamp_grid(
                    (fx * 0.42 + stereo_bias * 0.18) * vis_w,
                    fy * 0.40 * vis_h,
                );
                let radius = (1.15 + treble * 0.25).clamp(1.15, 1.50);
                let strength = (0.85 + treble * 0.40).clamp(0.85, 1.35);
                if strength > self.pending_impulses[2][3] {
                    self.pending_impulses[2] = [gx, gy, radius, strength];
                }
                self.treble_cd = 0.12;
                any_pending = true;
            }

            // Slot 3: Sustained musical flow ripple for ambient / legato passages
            if !any_pending && overall > 0.07 && self.ambient_cd <= 0.0 {
                self.emit_seq = self.emit_seq.wrapping_add(1);
                let (fx, fy) = r2_offset(self.emit_seq);
                let (gx, gy) = clamp_grid(
                    (fx * 0.34 + stereo_bias * 0.16) * vis_w,
                    fy * 0.32 * vis_h,
                );
                let strength = (0.75 + overall * 1.1).clamp(0.75, 1.85);
                self.pending_impulses[3] = [gx, gy, 2.2, strength];
                self.ambient_cd = 0.42;
            }
        }

        let attack_k = 1.0 - (-dt * 18.0).exp();
        let decay_k = 1.0 - (-dt * 3.8).exp();
        let smooth_follow = |env: &mut f32, val: f32| {
            let k = if val > *env { attack_k } else { decay_k };
            *env += (val - *env) * k;
        };
        smooth_follow(&mut self.bass_env, bass);
        smooth_follow(&mut self.mid_env, mids);
        smooth_follow(&mut self.treble_env, treble);
        smooth_follow(&mut self.overall_env, overall);
        smooth_follow(&mut self.left_env, left_vu);
        smooth_follow(&mut self.right_env, right_vu);

        self.wave_accum += dt;
        let mut substeps = 0u32;
        while self.wave_accum + 1e-5 >= STEP_DT && substeps < 2 {
            self.wave_accum = (self.wave_accum - STEP_DT).max(0.0);
            substeps += 1;
        }
        if !initialized && substeps == 0 {
            substeps = 1;
        }

        let mut out_impulses = [[0.0; 4]; 4];
        if substeps > 0 {
            out_impulses = self.pending_impulses;
            if substeps > 1 {
                let inv = 1.0 / substeps as f32;
                for imp in &mut out_impulses {
                    imp[3] *= inv;
                }
            }
            self.pending_impulses = [[0.0; 4]; 4];
        }

        (
            FlipDiscParams {
                impulses: out_impulses,
                audio_levels: [bass, mids, treble, overall],
                sim_info: [dt, time_s, aspect, vis_w],
            },
            substeps,
        )
    }
}
