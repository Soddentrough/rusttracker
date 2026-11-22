use super::{AudioUniforms, VulkanEngine};

impl VulkanEngine {
    pub const VU_PHOSPHOR_FADE_SECS: f32 = 0.35;

    /// Updates the per-segment phosphor persistence / afterglow envelope for all 30 blocks across
    /// up to 32 channels. Whenever a block ceases to be active (`segment_idx >= active_segments`),
    /// it smoothly fades from `1.0` back to `0.0` (regular unlit background color) over
    /// `VU_PHOSPHOR_FADE_SECS` (350 ms), packed at 8 bits per segment (`0..=255`) into
    /// `AudioUniforms::vu_segment_glows` (8 `u32` words per channel).
    pub fn step_vu_phosphor_ghosts(
        prev_active: &mut [u32; 32],
        segment_glows: &mut [[f32; 30]; 32],
        channels: &[f32; 32],
        num_channels: u32,
        dt: f32,
    ) -> [u32; 256] {
        let mut packed = [0u32; 256];
        let count = (num_channels as usize).min(32);
        let dt_clamped = dt.clamp(0.0, 0.1);
        let decay = dt_clamped / Self::VU_PHOSPHOR_FADE_SECS;

        for i in 0..count {
            let vu = channels[i].clamp(0.0, 1.0);
            let cur_active = ((vu * 30.0).ceil() as usize).min(30);
            let old_active = (prev_active[i] as usize).min(30);

            for (s, glow) in segment_glows[i].iter_mut().enumerate() {
                if s < cur_active {
                    // Currently active block stays fully energized, but we keep stored glow at 0.0
                    // while rising so only blocks that actually transition from active -> inactive fade.
                    *glow = 0.0;
                } else if s < old_active {
                    // Block was active on the previous frame and just turned off: ignite afterglow at 1.0
                    *glow = 1.0;
                } else if *glow > 0.0 {
                    // Recently active block: smoothly decay back toward unlit background (0.0)
                    *glow = (*glow - decay).max(0.0);
                }
            }

            prev_active[i] = cur_active as u32;

            let base_word = i * 8;
            for (s, &g) in segment_glows[i].iter().enumerate() {
                if g > 0.0 && s >= cur_active {
                    let byte_val = (g.clamp(0.0, 1.0) * 255.0).round() as u32;
                    packed[base_word + (s >> 2)] |= byte_val << ((s & 3) * 8);
                }
            }
        }

        for i in count..32 {
            prev_active[i] = 0;
            segment_glows[i] = [0.0; 30];
        }

        packed
    }

    pub fn step_fire_intensity(current: f32, state: &crate::state::AppState, dt: f32) -> f32 {
        let is_at_end =
            state.duration_seconds > 0.0 && state.current_seconds >= state.duration_seconds - 0.05;
        let is_playing = !state.is_paused && state.file_loaded && !state.track_ended && !is_at_end;
        let target_intensity = if is_playing { 1.0f32 } else { 0.0f32 };
        let fire_rate = if is_playing { 5.0f32 } else { 3.5f32 };
        let dt_clamped = dt.clamp(0.001, 0.1);
        if current < target_intensity {
            (current + fire_rate * dt_clamped).min(target_intensity)
        } else if current > target_intensity {
            (current - fire_rate * dt_clamped).max(target_intensity)
        } else {
            current
        }
    }

    pub fn update_retro_fire_grid(
        cells: &mut [u8; 320 * 180],
        rng: &mut u32,
        intensity: f32,
        state: &crate::state::AppState,
    ) {
        const W: usize = 320;
        const H: usize = 180;

        if intensity < 0.001 {
            // Decay towards 0 when stopped
            let mut all_zero = true;
            for cell in cells.iter_mut() {
                if *cell > 0 {
                    *cell = cell.saturating_sub(1);
                    all_zero = false;
                }
            }
            if all_zero {
                // all zero, nothing further needed
            }
        } else {
            // 1. Audio Analysis: Bass transient detection and column energy mapping
            let bass_raw = if state.spectrum_data.len() >= 16 {
                state.spectrum_data[1..16]
                    .iter()
                    .copied()
                    .fold(0.0f32, f32::max)
            } else {
                0.0
            };
            let bass = (bass_raw / 60.0).clamp(0.0, 1.5);

            let mut col_energies = [0.0f32; W];
            let spec_len = state.spectrum_data.len();

            if let Some(num_tracks) = state.tracker_channels.filter(|&n| n > 0) {
                let n_ch = (num_tracks as usize).min(64);
                let left_master = state.channel_vus.first().copied().unwrap_or(0.0);
                let right_master = state
                    .channel_vus
                    .get(1 + n_ch)
                    .copied()
                    .unwrap_or(left_master);

                for (x, energy) in col_energies.iter_mut().enumerate().take(W) {
                    let track_idx = ((x * n_ch) / W).min(n_ch.saturating_sub(1));
                    let track_vu = state.channel_vus.get(1 + track_idx).copied().unwrap_or(0.0);

                    // Symmetrical center-out spectrum distribution (bass in center, highs on outer edges)
                    let center_dist =
                        (x as f32 - (W - 1) as f32 / 2.0).abs() / ((W - 1) as f32 / 2.0);
                    let spec_idx = if spec_len > 1 {
                        (((center_dist.powf(1.4) * 450.0) as usize) + 1)
                            .clamp(1, spec_len.saturating_sub(1))
                    } else {
                        0
                    };
                    let spec_val = state.spectrum_data.get(spec_idx).copied().unwrap_or(0.0) / 60.0;

                    let frac = x as f32 / (W - 1) as f32;
                    let master_pan = left_master * (1.0 - frac) + right_master * frac;

                    *energy =
                        (track_vu * 0.60 + master_pan * 0.20 + spec_val * 0.20).clamp(0.0, 1.5);
                }
            } else {
                // Non-tracker audio (Stereo MP3, FLAC, AAC, WAV, Radio Stream, Multi-channel 5.1/7.1):
                let ch_len = state.channel_vus.len();
                let display_order: &[usize] = match ch_len {
                    6 => &[4, 0, 2, 3, 1, 5],
                    8 => &[6, 4, 0, 2, 3, 1, 5, 7],
                    16 => &[14, 12, 10, 8, 6, 4, 0, 2, 3, 1, 5, 7, 9, 11, 13, 15],
                    _ => &[0, 1],
                };

                for (x, energy) in col_energies.iter_mut().enumerate().take(W) {
                    let frac = x as f32 / (W - 1) as f32;

                    // Continuous spatial channel distribution across the screen width
                    let pan_vu = if ch_len >= 2 {
                        let order_len = display_order.len().min(ch_len);
                        if order_len > 1 {
                            let pos = frac * (order_len - 1) as f32;
                            let idx0 = (pos.floor() as usize).min(order_len - 1);
                            let idx1 = (pos.ceil() as usize).min(order_len - 1);
                            let blend = pos - idx0 as f32;
                            let vu0 = state
                                .channel_vus
                                .get(display_order[idx0])
                                .copied()
                                .unwrap_or(0.0);
                            let vu1 = state
                                .channel_vus
                                .get(display_order[idx1])
                                .copied()
                                .unwrap_or(vu0);
                            vu0 * (1.0 - blend) + vu1 * blend
                        } else {
                            state.channel_vus.first().copied().unwrap_or(0.0)
                        }
                    } else {
                        state.channel_vus.first().copied().unwrap_or(0.0)
                    };

                    // Symmetrical center-out spectrum distribution (bass in center, highs mirroring outward)
                    let center_dist =
                        (x as f32 - (W - 1) as f32 / 2.0).abs() / ((W - 1) as f32 / 2.0);
                    let bin = if spec_len > 1 {
                        (((center_dist.powf(1.4) * 450.0) as usize) + 1)
                            .clamp(1, spec_len.saturating_sub(1))
                    } else {
                        0
                    };
                    let spec_val = state.spectrum_data.get(bin).copied().unwrap_or(0.0) / 60.0;

                    *energy = (spec_val * 0.60 + pan_vu * 0.40).clamp(0.0, 1.5);
                }
            }

            let bottom_offset = (H - 1) * W;
            let hearth_offset = (H - 2) * W;

            // 2. Multi-step cellular automaton (2 sub-steps per frame for snappy transient physics)
            for _ in 0..2 {
                // Combustion Hearth Row
                for (x, &col_energy) in col_energies.iter().enumerate().take(W) {
                    let col_e = (col_energy * 0.75 + bass * 0.25) * intensity;

                    // Bottom heat scales from quiet ember (11) up to incandescent white-hot (36):
                    let base_heat = 11.0 + col_e * 25.0;

                    *rng ^= *rng << 13;
                    *rng ^= *rng >> 17;
                    *rng ^= *rng << 5;
                    let crackle = (*rng % 5) as i32 - 2;

                    let heat = ((base_heat as i32 + crackle) as u8).clamp(6, 36);
                    cells[bottom_offset + x] = heat;
                    cells[hearth_offset + x] = heat.saturating_sub((*rng % 2) as u8);
                }

                // In-place DOOM fire propagation across columns
                for (x, &col_energy) in col_energies.iter().enumerate().take(W) {
                    let col_e = (col_energy * 0.75 + bass * 0.25) * intensity;

                    // High decay when quiet (0.68), low decay when energetic (0.20):
                    let decay_prob = (0.68 - col_e * 0.44).clamp(0.20, 0.72);
                    let thresh = (decay_prob * 1000.0) as u32;

                    // Dynamic vertical convection on strong transients:
                    *rng ^= *rng << 13;
                    *rng ^= *rng >> 17;
                    *rng ^= *rng << 5;
                    let dy = if col_e > 0.70 && !(*rng).is_multiple_of(4) {
                        2
                    } else {
                        1
                    };

                    for y in dy..H {
                        let from = y * W + x;
                        let pixel = cells[from];
                        if pixel == 0 {
                            cells[from - W * dy] = 0;
                        } else {
                            *rng ^= *rng << 13;
                            *rng ^= *rng >> 17;
                            *rng ^= *rng << 5;
                            let jitter = ((*rng % 3) as i32) - 1;

                            let target_x = (x as i32 + jitter).clamp(0, (W - 1) as i32) as usize;
                            let target_y = y.saturating_sub(dy);
                            let to = target_y * W + target_x;

                            // Altitude cooling: taper smoke above 60% screen height so flame tips dance naturally
                            let alt_decay = if y < 70 && ((*rng >> 8) % 100 < 35) {
                                1
                            } else {
                                0
                            };
                            let decay = if ((*rng >> 4) % 1000) < thresh { 1 } else { 0 };
                            let total_decay = decay.max(alt_decay);
                            cells[to] = pixel.saturating_sub(total_decay);
                        }
                    }
                }
            }
        }
    }

    pub fn update_retro_fire(&mut self, state: &crate::state::AppState, uniforms: &AudioUniforms) {
        let dt = uniforms.frame_dt.clamp(0.001, 0.05);
        self.retro_fire_accum = (self.retro_fire_accum + dt * 60.0).min(3.0);
        let steps = (self.retro_fire_accum.floor() as u32).max(1);
        self.retro_fire_accum = (self.retro_fire_accum - steps as f32).max(0.0);

        for _ in 0..steps {
            Self::update_retro_fire_grid(
                &mut self.retro_fire_cells,
                &mut self.retro_fire_rng,
                self.fire_intensity,
                state,
            );
        }

        if self.fire_intensity < 0.001 && self.retro_fire_cells.iter().all(|&c| c == 0) {
            if self.retro_fire_cold_uploaded {
                return;
            }
            self.retro_fire_cold_uploaded = true;
        } else {
            self.retro_fire_cold_uploaded = false;
        }

        // Fast upload to 1024x576 fire_grid_texture using precomputed coordinate maps:
        const PAL_FLOAT: [f32; 37] = {
            let mut p = [0.0f32; 37];
            let mut i = 0;
            while i <= 36 {
                p[i] = i as f32 / 36.0;
                i += 1;
            }
            p
        };

        for py in 0..576 {
            let sy = (py * 180) / 576;
            let row_offset = py * 1024;
            let sim_offset = sy * 320;
            for px in 0..1024 {
                let sx = (px * 320) / 1024;
                let heat_idx = self.retro_fire_cells[sim_offset + sx] as usize;
                self.retro_fire_upload[row_offset + px] = PAL_FLOAT[heat_idx.min(36)];
            }
        }

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.fire_grid_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&self.retro_fire_upload),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1024 * 4),
                rows_per_image: Some(576),
            },
            wgpu::Extent3d {
                width: 1024,
                height: 576,
                depth_or_array_layers: 1,
            },
        );
    }
}
