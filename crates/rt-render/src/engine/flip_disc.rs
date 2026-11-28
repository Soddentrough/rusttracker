use crate::state::FlipDiscMode;
use std::f32::consts::TAU;

pub const FLIP_DISC_GRID_W: u32 = 128;
pub const FLIP_DISC_GRID_H: u32 = 96;
pub const FLIP_DISC_COUNT: u32 = FLIP_DISC_GRID_W * FLIP_DISC_GRID_H;
pub const FLIP_DISC_WORKGROUPS_X: u32 = FLIP_DISC_GRID_W / 16;
pub const FLIP_DISC_WORKGROUPS_Y: u32 = FLIP_DISC_GRID_H / 16;
pub const FLIP_DISC_AGENT_COUNT: usize = 24;
pub const FLIP_DISC_BAND_COUNT: usize = 64;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FlipDiscParams {
    /// Mode-specific circular/arc impulses (Ripples, Sonar sweep params, or Rain splash arcs).
    pub impulses: [[f32; 4]; 4],
    /// Normalized audio metrics: `[bass, mids, treble, overall]`.
    pub audio_levels: [f32; 4],
    /// Simulation metadata: `[dt, time_s, aspect_ratio, vis_width]`.
    pub sim_info: [f32; 4],
    /// Mode metadata: `[mode_id, vis_height, stereo_bias, reset_flag]`.
    pub mode_info: [f32; 4],
    /// 64 log-spaced FFT band levels packed 4 per `vec4`.
    pub bands: [[f32; 4]; 16],
    /// 64 gravity-falling peak-hold levels packed 4 per `vec4`.
    pub peaks: [[f32; 4]; 16],
    /// Up to 24 active boids or falling raindrops: `[gx, gy, size_or_len, energy_or_active]`.
    pub agents: [[f32; 4]; FLIP_DISC_AGENT_COUNT],
}

/// CPU-side onset detector, multi-pattern field simulator, and fixed-60 Hz substep scheduler
/// for the Mechanical Flip-Disc visualizer (ID 32).
#[derive(Clone, Debug)]
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
    pub last_mode: FlipDiscMode,
    pub mode_initialized: bool,
    // Flocking state
    pub boids_pos: [[f32; 2]; FLIP_DISC_AGENT_COUNT],
    pub boids_vel: [[f32; 2]; FLIP_DISC_AGENT_COUNT],
    pub boids_ready: bool,
    pub flock_scatter: f32,
    // Sonar sweep state
    pub sonar_angle: f32,
    pub sonar_bass_pulse: f32,
    pub sonar_bands: [f32; FLIP_DISC_BAND_COUNT],
    // Mechanical gravity spectrum state
    pub grav_bars: [f32; FLIP_DISC_BAND_COUNT],
    pub grav_peaks: [f32; FLIP_DISC_BAND_COUNT],
    pub grav_peak_vel: [f32; FLIP_DISC_BAND_COUNT],
    pub grav_peak_hold: [f32; FLIP_DISC_BAND_COUNT],
    // Kinetic rain state
    pub rain_pos: [[f32; 2]; FLIP_DISC_AGENT_COUNT],
    pub rain_len: [f32; FLIP_DISC_AGENT_COUNT],
    pub rain_speed: [f32; FLIP_DISC_AGENT_COUNT],
    pub rain_active: [bool; FLIP_DISC_AGENT_COUNT],
    pub rain_splashes: [[f32; 4]; 4],
    pub rain_spawn_cd: f32,
    pub rain_bass_swell: f32,
    pub rain_splash_cursor: usize,
}

impl Default for FlipDiscSimState {
    fn default() -> Self {
        Self {
            bass_env: 0.0,
            mid_env: 0.0,
            treble_env: 0.0,
            overall_env: 0.0,
            left_env: 0.0,
            right_env: 0.0,
            bass_cd: 0.0,
            mid_cd: 0.0,
            treble_cd: 0.0,
            ambient_cd: 0.0,
            wave_accum: 0.0,
            emit_seq: 0,
            pending_impulses: [[0.0; 4]; 4],
            last_mode: FlipDiscMode::Ripples,
            mode_initialized: false,
            boids_pos: [[0.0; 2]; FLIP_DISC_AGENT_COUNT],
            boids_vel: [[0.0; 2]; FLIP_DISC_AGENT_COUNT],
            boids_ready: false,
            flock_scatter: 0.0,
            sonar_angle: std::f32::consts::FRAC_PI_2,
            sonar_bass_pulse: 0.0,
            sonar_bands: [0.0; FLIP_DISC_BAND_COUNT],
            grav_bars: [0.0; FLIP_DISC_BAND_COUNT],
            grav_peaks: [0.0; FLIP_DISC_BAND_COUNT],
            grav_peak_vel: [0.0; FLIP_DISC_BAND_COUNT],
            grav_peak_hold: [0.0; FLIP_DISC_BAND_COUNT],
            rain_pos: [[0.0; 2]; FLIP_DISC_AGENT_COUNT],
            rain_len: [0.0; FLIP_DISC_AGENT_COUNT],
            rain_speed: [0.0; FLIP_DISC_AGENT_COUNT],
            rain_active: [false; FLIP_DISC_AGENT_COUNT],
            rain_splashes: [[0.0; 4]; 4],
            rain_spawn_cd: 0.0,
            rain_bass_swell: 0.0,
            rain_splash_cursor: 0,
        }
    }
}

#[inline]
fn r2_offset(seq: u32) -> (f32, f32) {
    let s = (seq as f32) + 1.0;
    let fx = (0.5 + s * 0.754_877_7).fract() * 2.0 - 1.0;
    let fy = (0.5 + s * 0.569_840_3).fract() * 2.0 - 1.0;
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

fn compute_log_bands(spectrum: &[f32]) -> [f32; FLIP_DISC_BAND_COUNT] {
    let mut out = [0.0f32; FLIP_DISC_BAND_COUNT];
    if spectrum.len() < 2 {
        return out;
    }
    let max_bin = (spectrum.len() - 1).min(512) as f32;
    for (i, slot) in out.iter_mut().enumerate() {
        let f0 = i as f32 / FLIP_DISC_BAND_COUNT as f32;
        let f1 = (i + 1) as f32 / FLIP_DISC_BAND_COUNT as f32;
        let b0 = (1.0 * max_bin.powf(f0)).floor() as usize;
        let b1 = ((1.0 * max_bin.powf(f1)).ceil() as usize).max(b0 + 1);
        let raw = band_energy(spectrum, b0, b1) / 56.0;
        let tilt = 1.0 + 0.75 * ((i as f32) / (FLIP_DISC_BAND_COUNT as f32 - 1.0));
        let compressed = 1.0 - (-(raw * tilt).max(0.0) * 1.45).exp();
        *slot = compressed.clamp(0.0, 1.0);
    }
    out
}

#[inline]
fn pack_64_to_vec4(src: &[f32; FLIP_DISC_BAND_COUNT]) -> [[f32; 4]; 16] {
    let mut out = [[0.0f32; 4]; 16];
    for (chunk_idx, dst) in out.iter_mut().enumerate() {
        let base = chunk_idx * 4;
        *dst = [src[base], src[base + 1], src[base + 2], src[base + 3]];
    }
    out
}

impl FlipDiscSimState {
    /// Computes the visible grid span `(vis_w, vis_h)` centered at `(64.0, 48.0)` matching
    /// `vis_flip_disc.wgsl` so all spawned patterns land inside the active viewport.
    #[inline]
    pub fn visible_grid_span(aspect_ratio: f32) -> (f32, f32) {
        let aspect = aspect_ratio.max(0.35);
        let vis_h = (30.0 / aspect.sqrt()).clamp(17.5, 42.0);
        let vis_w = (vis_h * aspect).min(108.0);
        (vis_w, vis_h)
    }

    fn reset_mode_buffers(&mut self) {
        self.pending_impulses = [[0.0; 4]; 4];
        self.boids_ready = false;
        self.flock_scatter = 0.0;
        self.sonar_angle = std::f32::consts::FRAC_PI_2;
        self.sonar_bass_pulse = 0.0;
        self.sonar_bands.fill(0.0);
        self.grav_bars.fill(0.0);
        self.grav_peaks.fill(0.0);
        self.grav_peak_vel.fill(0.0);
        self.grav_peak_hold.fill(0.0);
        self.rain_active.fill(false);
        self.rain_splashes = [[0.0; 4]; 4];
        self.rain_spawn_cd = 0.0;
        self.rain_bass_swell = 0.0;
        self.rain_splash_cursor = 0;
    }

    fn seed_boids(&mut self, cx: f32, cy: f32, vis_w: f32, vis_h: f32) {
        let rx = vis_w * 0.28;
        let ry = vis_h * 0.24;
        for i in 0..FLIP_DISC_AGENT_COUNT {
            let ang = (i as f32 / FLIP_DISC_AGENT_COUNT as f32) * TAU;
            let ring_mod = if i % 2 == 0 { 0.72 } else { 1.05 };
            self.boids_pos[i] = [cx + ang.cos() * rx * ring_mod, cy + ang.sin() * ry * ring_mod];
            let speed = 8.0 + (i % 5) as f32 * 0.8;
            self.boids_vel[i] = [-ang.sin() * speed, ang.cos() * speed];
        }
        self.boids_ready = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn step_boids_substep(
        &mut self,
        dt: f32,
        cx: f32,
        cy: f32,
        vis_w: f32,
        vis_h: f32,
        time_s: f32,
        mids: f32,
        overall: f32,
        stereo_bias: f32,
        bass_kick: bool,
    ) {
        if !self.boids_ready {
            self.seed_boids(cx, cy, vis_w, vis_h);
        }

        if bass_kick {
            self.flock_scatter = 0.35;
        } else {
            self.flock_scatter = (self.flock_scatter - dt).max(0.0);
        }

        let target_x = cx + (time_s * 0.42).sin() * (vis_w * 0.26) + stereo_bias * (vis_w * 0.14);
        let target_y = cy + (time_s * 0.67).cos() * (vis_h * 0.22);
        let target_speed = 6.5 + mids * 14.0 + overall * 7.5;
        let bound_x = vis_w * 0.46;
        let bound_y = vis_h * 0.44;
        let coh_weight = if self.flock_scatter > 0.0 { -1.15 } else { 0.75 };

        let old_pos = self.boids_pos;
        let old_vel = self.boids_vel;

        for i in 0..FLIP_DISC_AGENT_COUNT {
            let p = old_pos[i];
            let mut v = old_vel[i];

            let mut sep = [0.0f32; 2];
            let mut ali = [0.0f32; 2];
            let mut coh = [0.0f32; 2];
            let mut ali_n = 0.0f32;
            let mut coh_n = 0.0f32;

            for j in 0..FLIP_DISC_AGENT_COUNT {
                if i == j {
                    continue;
                }
                let dx = old_pos[j][0] - p[0];
                let dy = old_pos[j][1] - p[1];
                let d2 = dx * dx + dy * dy;
                if d2 < 1e-5 {
                    continue;
                }
                let d = d2.sqrt();
                if d < 3.4 {
                    let push = (3.4 - d) / (d * 3.4);
                    sep[0] -= dx * push;
                    sep[1] -= dy * push;
                }
                if d < 9.0 {
                    ali[0] += old_vel[j][0];
                    ali[1] += old_vel[j][1];
                    ali_n += 1.0;
                }
                if d < 11.0 {
                    coh[0] += dx;
                    coh[1] += dy;
                    coh_n += 1.0;
                }
            }

            let mut acc = [sep[0] * 24.0, sep[1] * 24.0];
            if ali_n > 0.0 {
                acc[0] += ((ali[0] / ali_n) - v[0]) * 1.1;
                acc[1] += ((ali[1] / ali_n) - v[1]) * 1.1;
            }
            if coh_n > 0.0 {
                acc[0] += (coh[0] / coh_n) * coh_weight;
                acc[1] += (coh[1] / coh_n) * coh_weight;
            }

            // Gentle Lissajous attractor & swirl
            let tx = target_x - p[0];
            let ty = target_y - p[1];
            acc[0] += tx * 1.05;
            acc[1] += ty * 1.05;

            if bass_kick {
                let bx = p[0] - cx;
                let by = p[1] - cy;
                let bd = (bx * bx + by * by).sqrt().max(0.5);
                acc[0] += (bx / bd - by / bd * 0.5) * 32.0;
                acc[1] += (by / bd + bx / bd * 0.5) * 32.0;
            }

            // Soft boundary repulsion
            let dx_c = p[0] - cx;
            let dy_c = p[1] - cy;
            if dx_c.abs() > bound_x * 0.78 {
                let over = (dx_c.abs() - bound_x * 0.78) / (bound_x * 0.22).max(0.5);
                acc[0] -= dx_c.signum() * over * 38.0;
            }
            if dy_c.abs() > bound_y * 0.78 {
                let over = (dy_c.abs() - bound_y * 0.78) / (bound_y * 0.22).max(0.5);
                acc[1] -= dy_c.signum() * over * 38.0;
            }

            v[0] += acc[0] * dt;
            v[1] += acc[1] * dt;
            let spd = (v[0] * v[0] + v[1] * v[1]).sqrt();
            if spd > 1e-4 {
                let desired = target_speed * (0.90 + (i % 4) as f32 * 0.07);
                let new_spd = spd + (desired - spd) * (dt * 6.0).min(1.0);
                let scale = new_spd / spd;
                v[0] *= scale;
                v[1] *= scale;
            } else {
                v = [target_speed, 0.0];
            }

            self.boids_vel[i] = v;
            self.boids_pos[i] = [
                (p[0] + v[0] * dt).clamp(cx - bound_x, cx + bound_x),
                (p[1] + v[1] * dt).clamp(cy - bound_y, cy + bound_y),
            ];
        }
    }

    fn spawn_rain_drop(&mut self, cx: f32, cy: f32, vis_w: f32, vis_h: f32, treble: f32, mids: f32, overall: f32) {
        let Some(slot) = self.rain_active.iter().position(|&a| !a) else {
            return;
        };
        self.emit_seq = self.emit_seq.wrapping_add(1);
        let (fx, fy) = r2_offset(self.emit_seq);
        let gx = cx + fx * vis_w * 0.45;
        let gy = cy + vis_h * 0.49 + fy.abs() * 1.5;
        let len = (2.4 + treble * 1.4 + overall * 0.7).clamp(2.4, 4.4);
        let speed = (14.0 + treble * 8.0 + mids * 4.0 + (slot % 3) as f32 * 1.6).clamp(14.0, 27.0);
        self.rain_pos[slot] = [gx, gy];
        self.rain_len[slot] = len;
        self.rain_speed[slot] = speed;
        self.rain_active[slot] = true;
    }

    fn push_rain_splash(&mut self, gx: f32, y_floor: f32, strength: f32) {
        self.rain_splashes[self.rain_splash_cursor] = [gx, y_floor, 0.5, strength];
        self.rain_splash_cursor = (self.rain_splash_cursor + 1) % 4;
    }

    /// Advances audio onset detection and fixed-60 Hz simulation substeps, returning
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
        mode: FlipDiscMode,
    ) -> (FlipDiscParams, u32) {
        const STEP_DT: f32 = 1.0 / 60.0;
        let dt = dt_raw.clamp(0.001, 0.033);
        let aspect = aspect_ratio.max(0.35);
        let (vis_w, vis_h) = Self::visible_grid_span(aspect);

        let mut reset_flag = 0.0f32;
        if !self.mode_initialized {
            self.last_mode = mode;
            self.mode_initialized = true;
        } else if mode != self.last_mode {
            self.last_mode = mode;
            self.reset_mode_buffers();
            reset_flag = 1.0;
        }

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
                    mode_info: [mode.as_u32() as f32, vis_h, stereo_bias, reset_flag],
                    bands: [[0.0; 4]; 16],
                    peaks: [[0.0; 4]; 16],
                    agents: [[0.0; 4]; FLIP_DISC_AGENT_COUNT],
                },
                0,
            );
        }

        self.bass_cd = (self.bass_cd - dt).max(0.0);
        self.mid_cd = (self.mid_cd - dt).max(0.0);
        self.treble_cd = (self.treble_cd - dt).max(0.0);
        self.ambient_cd = (self.ambient_cd - dt).max(0.0);
        self.rain_spawn_cd = (self.rain_spawn_cd - dt).max(0.0);

        let cx = (FLIP_DISC_GRID_W as f32) * 0.5;
        let cy = (FLIP_DISC_GRID_H as f32) * 0.5;
        let max_dx = (vis_w * 0.44).min(cx - 6.0);
        let max_dy = (vis_h * 0.42).min(cy - 6.0);

        let clamp_grid = |dx: f32, dy: f32| -> (f32, f32) {
            (
                cx + dx.clamp(-max_dx, max_dx),
                cy + dy.clamp(-max_dy, max_dy),
            )
        };

        let bass_kick = overall >= 0.015 && bass > (self.bass_env * 1.18 + 0.06) && self.bass_cd <= 0.0;
        let mid_hit = overall >= 0.015 && mids > (self.mid_env * 1.15 + 0.05) && self.mid_cd <= 0.0;
        let treble_hit = overall >= 0.015 && treble > (self.treble_env * 1.16 + 0.045) && self.treble_cd <= 0.0;

        self.wave_accum += dt;
        let mut substeps = 0u32;
        while self.wave_accum + 1e-5 >= STEP_DT && substeps < 2 {
            self.wave_accum = (self.wave_accum - STEP_DT).max(0.0);
            substeps += 1;
        }
        if (!initialized || reset_flag > 0.5) && substeps == 0 {
            substeps = 1;
        }

        let mut out_impulses = [[0.0; 4]; 4];
        let mut out_bands = [[0.0; 4]; 16];
        let mut out_peaks = [[0.0; 4]; 16];
        let mut out_agents = [[0.0; 4]; FLIP_DISC_AGENT_COUNT];
        let gpu_substeps: u32;

        match mode {
            FlipDiscMode::Ripples => {
                let mut any_pending = self.pending_impulses.iter().any(|imp| imp[3] > 0.0);
                if overall >= 0.015 {
                    if bass_kick {
                        self.emit_seq = self.emit_seq.wrapping_add(1);
                        let (fx, fy) = r2_offset(self.emit_seq);
                        let (gx, gy) = clamp_grid(
                            (fx * 0.24 + stereo_bias * 0.14) * vis_w,
                            fy * 0.20 * vis_h,
                        );
                        let radius = (2.6 + bass * 0.5).clamp(2.6, 3.3) * 1.15;
                        let strength = (1.65 + bass * 0.65).clamp(1.65, 2.55);
                        if strength > self.pending_impulses[0][3] {
                            self.pending_impulses[0] = [gx, gy, radius, strength];
                        }
                        self.bass_cd = 0.24;
                        any_pending = true;
                    }
                    if mid_hit {
                        self.emit_seq = self.emit_seq.wrapping_add(1);
                        let (fx, fy) = r2_offset(self.emit_seq);
                        let (gx, gy) = clamp_grid(
                            (fx * 0.28 + stereo_bias * 0.34) * vis_w,
                            fy * 0.32 * vis_h,
                        );
                        let radius = (1.8 + mids * 0.35).clamp(1.8, 2.3) * 1.15;
                        let strength = (1.15 + mids * 0.55).clamp(1.15, 1.85);
                        if strength > self.pending_impulses[1][3] {
                            self.pending_impulses[1] = [gx, gy, radius, strength];
                        }
                        self.mid_cd = 0.17;
                        any_pending = true;
                    }
                    if treble_hit {
                        self.emit_seq = self.emit_seq.wrapping_add(1);
                        let (fx, fy) = r2_offset(self.emit_seq);
                        let (gx, gy) = clamp_grid(
                            (fx * 0.42 + stereo_bias * 0.18) * vis_w,
                            fy * 0.40 * vis_h,
                        );
                        let radius = (1.15 + treble * 0.25).clamp(1.15, 1.50) * 1.15;
                        let strength = (0.85 + treble * 0.40).clamp(0.85, 1.35);
                        if strength > self.pending_impulses[2][3] {
                            self.pending_impulses[2] = [gx, gy, radius, strength];
                        }
                        self.treble_cd = 0.12;
                        any_pending = true;
                    }
                    if !any_pending && overall > 0.07 && self.ambient_cd <= 0.0 {
                        self.emit_seq = self.emit_seq.wrapping_add(1);
                        let (fx, fy) = r2_offset(self.emit_seq);
                        let (gx, gy) = clamp_grid(
                            (fx * 0.34 + stereo_bias * 0.16) * vis_w,
                            fy * 0.32 * vis_h,
                        );
                        let strength = (0.75 + overall * 1.1).clamp(0.75, 1.85);
                        self.pending_impulses[3] = [gx, gy, 2.2 * 1.15, strength];
                        self.ambient_cd = 0.42;
                    }
                }
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
                gpu_substeps = substeps;
            }
            FlipDiscMode::Flocking => {
                if bass_kick {
                    self.bass_cd = 0.24;
                }
                for step_i in 0..substeps {
                    self.step_boids_substep(
                        STEP_DT,
                        cx,
                        cy,
                        vis_w,
                        vis_h,
                        time_s,
                        mids,
                        overall,
                        stereo_bias,
                        bass_kick && step_i == 0,
                    );
                }
                let boid_energy = if overall >= 0.015 {
                    (0.55 + overall * 0.70).clamp(0.55, 1.35)
                } else {
                    0.0
                };
                for i in 0..FLIP_DISC_AGENT_COUNT {
                    let radius = 1.25 + bass * 0.45 + (i % 3) as f32 * 0.12;
                    out_agents[i] = [
                        self.boids_pos[i][0],
                        self.boids_pos[i][1],
                        radius,
                        boid_energy,
                    ];
                }
                gpu_substeps = if substeps > 0 || reset_flag > 0.5 { 1 } else { 0 };
            }
            FlipDiscMode::SonarSweep => {
                let raw_bands = compute_log_bands(spectrum);
                let b_atk = 1.0 - (-dt * 24.0).exp();
                let b_dec = 1.0 - (-dt * 6.5).exp();
                for (dst, &src) in self.sonar_bands.iter_mut().zip(raw_bands.iter()) {
                    let target = if overall >= 0.015 { src } else { 0.0 };
                    let k = if target > *dst { b_atk } else { b_dec };
                    *dst += (target - *dst) * k;
                }
                if bass_kick {
                    self.bass_cd = 0.24;
                    self.sonar_bass_pulse = (0.7 + bass * 0.55).clamp(0.7, 1.3);
                }
                let prev_angle = self.sonar_angle;
                let omega = 1.35 + overall * 1.15 + if bass_kick { 0.55 } else { 0.0 };
                let mut swept_delta = 0.0f32;
                for _ in 0..substeps {
                    let step_ang = omega * STEP_DT;
                    self.sonar_angle = (self.sonar_angle - step_ang).rem_euclid(TAU);
                    swept_delta += step_ang;
                    self.sonar_bass_pulse *= 0.86;
                }
                let outer_radius = (vis_w * 0.48).hypot(vis_h * 0.48);
                out_impulses[0] = [prev_angle, self.sonar_angle, swept_delta, outer_radius];
                out_impulses[1] = [cx, cy, stereo_bias, self.sonar_bass_pulse];
                out_bands = pack_64_to_vec4(&self.sonar_bands);
                gpu_substeps = if substeps > 0 || reset_flag > 0.5 { 1 } else { 0 };
            }
            FlipDiscMode::MechanicalGravity => {
                let raw_bands = compute_log_bands(spectrum);
                let atk_k = 1.0 - (-dt * 26.0).exp();
                let rel_k = 1.0 - (-dt * 7.0).exp();
                for i in 0..FLIP_DISC_BAND_COUNT {
                    let target = if overall < 0.015 {
                        0.0
                    } else {
                        (raw_bands[i] * 0.90).clamp(0.0, 0.90)
                    };
                    let k = if target > self.grav_bars[i] { atk_k } else { rel_k };
                    self.grav_bars[i] += (target - self.grav_bars[i]) * k;

                    if overall < 0.015 {
                        self.grav_peak_hold[i] = 0.0;
                    }
                    if self.grav_bars[i] >= self.grav_peaks[i] {
                        self.grav_peaks[i] = self.grav_bars[i];
                        self.grav_peak_vel[i] = 0.0;
                        self.grav_peak_hold[i] = if overall >= 0.015 { 0.22 } else { 0.0 };
                    } else if self.grav_peak_hold[i] > 0.0 {
                        self.grav_peak_hold[i] = (self.grav_peak_hold[i] - dt).max(0.0);
                    } else {
                        self.grav_peak_vel[i] += 2.4 * dt;
                        self.grav_peaks[i] =
                            (self.grav_peaks[i] - self.grav_peak_vel[i] * dt).max(self.grav_bars[i]);
                    }
                }
                out_bands = pack_64_to_vec4(&self.grav_bars);
                out_peaks = pack_64_to_vec4(&self.grav_peaks);
                gpu_substeps = if substeps > 0 || reset_flag > 0.5 { 1 } else { 0 };
            }
            FlipDiscMode::KineticRain => {
                let y_floor = cy - vis_h * 0.43;
                if treble_hit {
                    self.treble_cd = 0.08;
                    self.spawn_rain_drop(cx, cy, vis_w, vis_h, treble, mids, overall);
                    if treble > 0.55 {
                        self.spawn_rain_drop(cx, cy, vis_w, vis_h, treble, mids, overall);
                    }
                }
                if mid_hit {
                    self.mid_cd = 0.14;
                    self.spawn_rain_drop(cx, cy, vis_w, vis_h, treble, mids, overall);
                }
                if bass_kick {
                    self.bass_cd = 0.22;
                    self.rain_bass_swell = (0.75 + bass * 0.65).clamp(0.75, 1.45);
                    self.emit_seq = self.emit_seq.wrapping_add(1);
                    let (fx, _) = r2_offset(self.emit_seq);
                    self.push_rain_splash(cx + fx * vis_w * 0.28, y_floor, (1.1 + bass * 0.45).clamp(1.1, 1.6));
                }
                if overall >= 0.015 && self.rain_spawn_cd <= 0.0 {
                    let active_count = self.rain_active.iter().filter(|&&a| a).count();
                    let target_drops = (6.0 + overall * 14.0).round() as usize;
                    if active_count < target_drops {
                        self.spawn_rain_drop(cx, cy, vis_w, vis_h, treble, mids, overall);
                        if active_count == 0 {
                            self.spawn_rain_drop(cx, cy, vis_w, vis_h, treble, mids, overall);
                        }
                    }
                    self.rain_spawn_cd = (0.14 - overall * 0.08).clamp(0.04, 0.14);
                }

                for _ in 0..substeps {
                    for i in 0..FLIP_DISC_AGENT_COUNT {
                        if !self.rain_active[i] {
                            continue;
                        }
                        self.rain_pos[i][0] += stereo_bias * 2.5 * STEP_DT;
                        self.rain_pos[i][1] -= self.rain_speed[i] * STEP_DT;
                        if self.rain_pos[i][1] <= y_floor {
                            if overall >= 0.015 {
                                let str_val = (0.95 + overall * 0.45).clamp(0.95, 1.50);
                                self.push_rain_splash(self.rain_pos[i][0], y_floor, str_val);
                            }
                            self.rain_active[i] = false;
                        }
                    }
                    for sp in &mut self.rain_splashes {
                        if sp[3] > 0.0 {
                            sp[2] += 15.0 * STEP_DT;
                            sp[3] *= 0.88;
                            if sp[3] < 0.12 || sp[2] > 6.5 {
                                *sp = [0.0; 4];
                            }
                        }
                    }
                    self.rain_bass_swell *= 0.87;
                }

                for i in 0..FLIP_DISC_AGENT_COUNT {
                    if self.rain_active[i] {
                        out_agents[i] = [
                            self.rain_pos[i][0],
                            self.rain_pos[i][1],
                            self.rain_len[i],
                            1.0,
                        ];
                    }
                }
                out_impulses = self.rain_splashes;
                out_bands[0][0] = self.rain_bass_swell;
                out_bands[0][1] = y_floor;
                gpu_substeps = if substeps > 0 || reset_flag > 0.5 { 1 } else { 0 };
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

        (
            FlipDiscParams {
                impulses: out_impulses,
                audio_levels: [bass, mids, treble, overall],
                sim_info: [dt, time_s, aspect, vis_w],
                mode_info: [mode.as_u32() as f32, vis_h, stereo_bias, reset_flag],
                bands: out_bands,
                peaks: out_peaks,
                agents: out_agents,
            },
            gpu_substeps,
        )
    }
}
