// =====================================================
// RustTracker — 3D Multi-Speaker Acoustic Wave FDTD Compute
// 64x32x64 room grid simulating 7.1.4 bed speakers + 3 dynamic
// 3D object monopoles, standing-wave RMS pressure <p^2>, and
// destructive nodal particle-velocity gradient <|grad p|^2>.
// =====================================================

// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(1) var<storage, read_write> wave_a: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> wave_b: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> packed_vol: array<u32>;

const NX: u32 = 64u;
const NY: u32 = 32u;
const NZ: u32 = 64u;
const STRIDE_Y: u32 = 64u;
const STRIDE_Z: u32 = 2048u; // 64 * 32

const ROOM_MIN: vec3<f32> = vec3<f32>(-10.8, -1.5, -8.9);
const ROOM_MAX: vec3<f32> = vec3<f32>(10.8, 4.9, 8.9);
const ROOM_SIZE: vec3<f32> = vec3<f32>(21.6, 6.4, 17.8);
const GRID_DIM_F: vec3<f32> = vec3<f32>(64.0, 32.0, 64.0);

// Metric-isotropic Laplacian weights normalized so wx + wy + wz == 3.0 (CFL: c2 <= 0.333)
const LAPLACIAN_W: vec3<f32> = vec3<f32>(0.5639, 1.6058, 0.8303);
const WAVE_C2: f32 = 0.24;

// Match speaker locations in vis_neon_room.wgsl
const SPEAKER_POS: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(-3.8, -0.25,  5.5), // 0: FL
    vec3<f32>( 3.8, -0.25,  5.5), // 1: FR
    vec3<f32>( 0.0, -0.02,  5.32), // 2: C
    vec3<f32>( 0.0, -0.90,  5.50), // 3: LFE
    vec3<f32>(-6.2,  0.50,  0.5), // 4: SL
    vec3<f32>( 6.2,  0.50,  0.5), // 5: SR
    vec3<f32>(-4.2,  0.10, -4.8), // 6: RL
    vec3<f32>( 4.2,  0.10, -4.8), // 7: RR
    vec3<f32>(-3.2,  4.32,  3.5), // 8: TFL
    vec3<f32>( 3.2,  4.32,  3.5), // 9: TFR
    vec3<f32>(-3.2,  4.32, -2.5), // 10: TRL
    vec3<f32>( 3.2,  4.32, -2.5)  // 11: TRR
);

// Normalized spectral color-role coordinate [0..1] per 7.1.4 speaker role
const SPEAKER_BAND_TINT: array<f32, 12> = array<f32, 12>(
    0.72, // 0: FL (Cyan-blue)
    0.78, // 1: FR (Cyan-magenta)
    0.42, // 2: C (Warm gold)
    0.08, // 3: LFE (Deep warm amber/crimson)
    0.84, // 4: SL (Electric cobalt)
    0.88, // 5: SR (Violet)
    0.66, // 6: RL (Aqua)
    0.92, // 7: RR (Magenta)
    0.58, // 8: TFL (Mint/emerald)
    0.52, // 9: TFR (Lime/gold)
    0.64, // 10: TRL (Teal)
    0.95  // 11: TRR (Pink-violet)
);

fn voxel_idx(ix: u32, iy: u32, iz: u32) -> u32 {
    return ix + iy * STRIDE_Y + iz * STRIDE_Z;
}

fn voxel_world_pos(ix: u32, iy: u32, iz: u32) -> vec3<f32> {
    let uvw = (vec3<f32>(f32(ix), f32(iy), f32(iz)) + vec3<f32>(0.5)) / GRID_DIM_F;
    return ROOM_MIN + uvw * ROOM_SIZE;
}

fn read_phase(idx: u32) -> f32 {
    return audio.channel_phases[idx >> 2u][idx & 3u];
}

fn get_speaker_level(ch: u32, bass_e: f32, lowmid_e: f32, mid_e: f32, high_e: f32) -> f32 {
    let idx = min(ch, 11u);
    if (audio.num_spatial_channels == 4u) {
        let t0 = audio.spatial_channels[0].x;
        let t1 = audio.spatial_channels[0].y;
        let t2 = audio.spatial_channels[0].z;
        let t3 = audio.spatial_channels[0].w;
        switch idx {
            case 0u: { return t0; }
            case 1u: { return t1; }
            case 2u: { return clamp((t0 + t1 + t2 + t3) * 0.25 + mid_e * 0.25, 0.0, 1.0); }
            case 3u: { return clamp(bass_e * 0.85 + max(max(t0, t1), max(t2, t3)) * 0.25, 0.0, 1.0); }
            case 4u: { return t2; }
            case 5u: { return t3; }
            case 6u: { return clamp(t2 * 0.85 + t0 * 0.20, 0.0, 1.0); }
            case 7u: { return clamp(t3 * 0.85 + t1 * 0.20, 0.0, 1.0); }
            case 8u: { return t0; }
            case 9u: { return t1; }
            case 10u: { return t2; }
            default: { return t3; }
        }
    }
    if (audio.num_spatial_channels > 2u) {
        if (audio.num_spatial_channels == 6u && audio.num_channels == 6u) {
            if (idx < 6u) {
                return audio.spatial_channels[idx >> 2u][idx & 3u];
            } else if (idx == 6u) {
                return audio.spatial_channels[1].x;
            } else if (idx == 7u) {
                return audio.spatial_channels[1].y;
            } else {
                let base_idx = idx - 8u;
                return clamp(audio.spatial_channels[0][base_idx] * 0.65 + high_e * 0.35, 0.0, 1.0);
            }
        }
        let actual_ch = min(audio.num_spatial_channels, 16u);
        if (idx >= actual_ch && actual_ch < 12u) {
            let src_ch = idx % actual_ch;
            return audio.spatial_channels[src_ch >> 2u][src_ch & 3u];
        }
        return audio.spatial_channels[idx >> 2u][idx & 3u];
    }
    let ch_l = audio.spatial_channels[0].x;
    let ch_r = select(ch_l, audio.spatial_channels[0].y, audio.num_spatial_channels >= 2u);
    let phase_l = read_phase(0u);
    let phase_r = read_phase(1u);
    // Reduce synthesized center energy when L/R are out of phase so center nodal plane stays crisp
    let in_phase_coh = 0.5 + 0.5 * cos(phase_l - phase_r);
    let raw_upmix = stereo_upmix_12(idx, ch_l, ch_r, phase_l, phase_r, bass_e, lowmid_e, mid_e, high_e);
    if (idx == 2u) {
        return raw_upmix * in_phase_coh;
    }
    return raw_upmix;
}

fn get_speaker_phase(ch: u32) -> f32 {
    let idx = min(ch, 11u);
    if (audio.num_spatial_channels == 4u) {
        switch idx {
            case 0u, 8u: { return read_phase(0u); }
            case 1u, 9u: { return read_phase(1u); }
            case 4u, 6u, 10u: { return read_phase(2u); }
            case 5u, 7u, 11u: { return read_phase(3u); }
            default: { return 0.0; }
        }
    }
    if (audio.num_spatial_channels > 2u) {
        let actual_ch = min(audio.num_spatial_channels, 16u);
        let src_ch = idx % actual_ch;
        return read_phase(src_ch);
    }
    // Stereo/mono: assign Left-side speakers phase_L, Right-side phase_R, Center/Sub median phase
    let phase_l = read_phase(0u);
    let phase_r = select(phase_l, read_phase(1u), audio.num_spatial_channels >= 2u);
    switch idx {
        case 0u, 4u, 6u, 8u, 10u: { return phase_l; }
        case 1u, 5u, 7u, 9u, 11u: { return phase_r; }
        default: { return 0.0; }
    }
}

struct SourceInjection {
    pressure: f32,
    band_weight: f32,
    band_sum: f32,
}

fn add_monopole_source(
    wpos: vec3<f32>,
    src_pos: vec3<f32>,
    energy: f32,
    multi_norm: f32,
    osc: f32,
    phase1: f32,
    phase2: f32,
    band_tint: f32,
    p_inj: ptr<function, f32>,
    w_band: ptr<function, f32>,
    b_sum: ptr<function, f32>
) {
    if (energy <= 0.01) {
        return;
    }
    let d = wpos - src_pos;
    let d2 = dot(d, d);
    if (d2 >= 1.96) {
        return;
    }
    let kern = exp(-d2 * 2.5);
    let drive = energy * multi_norm * kern * 0.15;
    *p_inj += drive * (0.65 * sin(osc + phase1) + 0.55 * sin(2.0 * osc + phase2));
    let w = energy * kern;
    *w_band += w;
    *b_sum += w * band_tint;
}

fn compute_source_injection(wpos: vec3<f32>, osc: f32) -> SourceInjection {
    let bass_e = clamp((audio.spectrum[0].y + audio.spectrum[0].z + audio.spectrum[0].w + audio.spectrum[1].x) / 260.0, 0.0, 1.0);
    let lowmid_e = clamp((audio.spectrum[2].x + audio.spectrum[3].x + audio.spectrum[4].x) / 195.0, 0.0, 1.0);
    let mid_e = clamp((audio.spectrum[6].x + audio.spectrum[10].x + audio.spectrum[14].x) / 180.0, 0.0, 1.0);
    let high_e = clamp((audio.spectrum[20].x + audio.spectrum[34].x + audio.spectrum[50].x) / 165.0, 0.0, 1.0);

    let f_vis = clamp(audio.channel_phases[4].x, 0.0, 1.0);
    let bass_kick = clamp(audio.channel_phases[7].z, 0.0, 1.2);
    let sim_intensity = clamp(audio.channel_phases[7].w, 0.0, 1.0);

    var sp_amps: array<f32, 12>;
    var total_sp_amp = 0.0;
    for (var ch_a = 0u; ch_a < 12u; ch_a = ch_a + 1u) {
        let lvl = get_speaker_level(ch_a, bass_e, lowmid_e, mid_e, high_e);
        sp_amps[ch_a] = lvl;
        total_sp_amp += lvl;
    }
    // Soft sqrt(N) gain normalization keeps 12-channel Atmos bass tuttis well-modulated without whiteout
    let multi_norm = 1.0 / sqrt(max(1.20, total_sp_amp * 0.70));

    // Strictly integer harmonics (1.0*osc, 2.0*osc) for exact CPU 2*PI wrap continuity.
    // The 2*osc term preserves exact sign flip when phi == PI via (cos(phi), sin(phi)) rotation,
    // guaranteeing complete destructive cancellation (p == 0) across symmetric opposing channels.
    let w1_base = clamp(0.68 + 0.22 * bass_e - 0.16 * high_e, 0.35, 0.92);
    let w2_base = clamp(0.26 + 0.50 * (0.45 * mid_e + 0.75 * high_e + 0.35 * f_vis), 0.18, 0.88);
    let s1_osc = sin(osc);
    let c1_osc = cos(osc);
    let s2_osc = sin(2.0 * osc + 0.45);
    let c2_osc = cos(2.0 * osc + 0.45);

    var p_inj = 0.0;
    var w_band = 0.0;
    var b_sum = 0.0;

    // 12 spatial bed speakers
    for (var ch = 0u; ch < 12u; ch = ch + 1u) {
        let d = wpos - SPEAKER_POS[ch];
        let d2 = dot(d, d);
        if (d2 < 1.96) {
            let amp = sp_amps[ch];
            if (amp > 0.004) {
                let phi = get_speaker_phase(ch);
                let cp = cos(phi);
                let sp = sin(phi);
                let fund = s1_osc * cp + c1_osc * sp;
                let harm = s2_osc * cp - c2_osc * sp;

                var w1 = w1_base;
                var w2 = w2_base;
                var boost = 1.0;
                if (ch == 3u) {
                    // LFE excites multi-ring room modes & bounded kick pulse without cavity inflation
                    w1 = 0.82;
                    w2 = 0.24;
                    boost = 0.78 + 0.24 * bass_kick;
                } else if (ch >= 8u) {
                    // Height tweeters favor sharper harmonic interference fringes
                    w1 = w1_base * 0.55;
                    w2 = w2_base * 1.15;
                }

                let kern = exp(-d2 * 2.7);
                let drive = amp * boost * multi_norm * kern * 0.25 * (0.35 + 0.65 * sim_intensity);
                p_inj += drive * (w1 * fund + w2 * harm);

                let mag = amp * kern;
                w_band += mag;
                b_sum += mag * SPEAKER_BAND_TINT[ch];
            }
        }
    }

    // 3 dynamic 3D object / stem monopoles packed in channel_phases[18..29]
    let obj0_pos = vec3<f32>(audio.channel_phases[4].z, audio.channel_phases[4].w, audio.channel_phases[5].x);
    let obj1_pos = vec3<f32>(audio.channel_phases[5].z, audio.channel_phases[5].w, audio.channel_phases[6].x);
    let obj2_pos = vec3<f32>(audio.channel_phases[6].z, audio.channel_phases[6].w, audio.channel_phases[7].x);
    add_monopole_source(wpos, obj0_pos, audio.channel_phases[5].y, multi_norm, osc, 0.5, 1.1, 0.88, &p_inj, &w_band, &b_sum);
    add_monopole_source(wpos, obj1_pos, audio.channel_phases[6].y, multi_norm, osc, 2.1, 2.7, 0.55, &p_inj, &w_band, &b_sum);
    add_monopole_source(wpos, obj2_pos, audio.channel_phases[7].y, multi_norm, osc, 3.14159, 4.8, 0.18, &p_inj, &w_band, &b_sum);

    var out: SourceInjection;
    out.pressure = p_inj;
    out.band_weight = w_band;
    out.band_sum = b_sum;
    return out;
}

fn pack_rgba8_unorm(c: vec4<f32>) -> u32 {
    let q = vec4<u32>(clamp(round(c * 255.0), vec4<f32>(0.0), vec4<f32>(255.0)));
    return q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
}

fn fdtd_step_pressure(
    p_c: f32,
    p_prev: f32,
    lap: f32,
    inj_pressure: f32,
    is_wall: bool,
    step_s: f32
) -> f32 {
    let sim_intensity = clamp(audio.channel_phases[7].w, 0.0, 1.0);
    let idle_decay = select(1.0, 0.94, sim_intensity < 0.005);
    let base_damp = select(0.991, 0.938, is_wall) * idle_decay;
    let damp = pow(base_damp, step_s);
    let step_s2 = step_s * step_s;
    // Screened compliance (-0.008 * p_c) drains the sealed Neumann uniform DC room mode while
    // preserving exact odd symmetry on opposing phase cancellation planes.
    let vel = (p_c - p_prev) * damp + (WAVE_C2 * lap - 0.008 * p_c) * step_s2;
    return clamp(p_c + vel + inj_pressure * step_s2, -3.5, 3.5);
}

@compute @workgroup_size(8, 4, 4)
fn cs_clear_waves(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= NX || gid.y >= NY || gid.z >= NZ) {
        return;
    }
    let idx = voxel_idx(gid.x, gid.y, gid.z);
    wave_a[idx] = vec4<f32>(0.0, 0.0, 0.0, 0.5);
    wave_b[idx] = vec4<f32>(0.0, 0.0, 0.0, 0.5);
    packed_vol[idx] = pack_rgba8_unorm(vec4<f32>(0.0, 0.0, 0.5, 0.5));
}

// Sub-step 1: reads p_curr from wave_a[neighbor].x and p_prev from wave_b[self].x,
// writes p_next to wave_b[self].x. Preserves wave_a untouched for race-free stencil reads.
@compute @workgroup_size(8, 4, 4)
fn cs_step_ab(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= NX || gid.y >= NY || gid.z >= NZ) {
        return;
    }
    let ix = gid.x;
    let iy = gid.y;
    let iz = gid.z;
    let idx = voxel_idx(ix, iy, iz);

    let xm = select(ix - 1u, 0u, ix == 0u);
    let xp = select(ix + 1u, NX - 1u, ix == NX - 1u);
    let ym = select(iy - 1u, 0u, iy == 0u);
    let yp = select(iy + 1u, NY - 1u, iy == NY - 1u);
    let zm = select(iz - 1u, 0u, iz == 0u);
    let zp = select(iz + 1u, NZ - 1u, iz == NZ - 1u);

    let p_c = wave_a[idx].x;
    let p_prev = wave_b[idx].x;

    let p_xm = wave_a[voxel_idx(xm, iy, iz)].x;
    let p_xp = wave_a[voxel_idx(xp, iy, iz)].x;
    let p_ym = wave_a[voxel_idx(ix, ym, iz)].x;
    let p_yp = wave_a[voxel_idx(ix, yp, iz)].x;
    let p_zm = wave_a[voxel_idx(ix, iy, zm)].x;
    let p_zp = wave_a[voxel_idx(ix, iy, zp)].x;

    let lap = LAPLACIAN_W.x * (p_xm + p_xp - 2.0 * p_c)
            + LAPLACIAN_W.y * (p_ym + p_yp - 2.0 * p_c)
            + LAPLACIAN_W.z * (p_zm + p_zp - 2.0 * p_c);

    let is_wall = ix == 0u || ix == NX - 1u || iy == 0u || iy == NY - 1u || iz == 0u || iz == NZ - 1u;
    let step_s = clamp(audio.frame_dt * 60.0, 0.20, 1.08);
    let dt_osc_half = clamp(audio.frame_dt, 0.002, 0.033) * (14.0 + 7.0 * clamp(audio.channel_phases[4].x, 0.0, 1.0));
    let osc_half = audio.channel_phases[4].y - dt_osc_half;
    let wpos = voxel_world_pos(ix, iy, iz);
    let inj = compute_source_injection(wpos, osc_half);

    let p_next = fdtd_step_pressure(p_c, p_prev, lap, inj.pressure, is_wall, step_s);

    var b_self = wave_b[idx];
    b_self.x = p_next;
    wave_b[idx] = b_self;
}

// Sub-step 2: reads p_curr from wave_b[neighbor].x and p_prev from wave_a[self].x,
// writes p_next + smoothed observables into wave_a[self], and packs RGBA8Unorm voxel output.
@compute @workgroup_size(8, 4, 4)
fn cs_step_ba(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= NX || gid.y >= NY || gid.z >= NZ) {
        return;
    }
    let ix = gid.x;
    let iy = gid.y;
    let iz = gid.z;
    let idx = voxel_idx(ix, iy, iz);

    let xm = select(ix - 1u, 0u, ix == 0u);
    let xp = select(ix + 1u, NX - 1u, ix == NX - 1u);
    let ym = select(iy - 1u, 0u, iy == 0u);
    let yp = select(iy + 1u, NY - 1u, iy == NY - 1u);
    let zm = select(iz - 1u, 0u, iz == 0u);
    let zp = select(iz + 1u, NZ - 1u, iz == NZ - 1u);

    let a_prev = wave_a[idx];
    let p_prev = a_prev.x;
    let p_c = wave_b[idx].x;

    let idx_xm = voxel_idx(xm, iy, iz);
    let idx_xp = voxel_idx(xp, iy, iz);
    let idx_ym = voxel_idx(ix, ym, iz);
    let idx_yp = voxel_idx(ix, yp, iz);
    let idx_zm = voxel_idx(ix, iy, zm);
    let idx_zp = voxel_idx(ix, iy, zp);

    let p_xm = wave_b[idx_xm].x;
    let p_xp = wave_b[idx_xp].x;
    let p_ym = wave_b[idx_ym].x;
    let p_yp = wave_b[idx_yp].x;
    let p_zm = wave_b[idx_zm].x;
    let p_zp = wave_b[idx_zp].x;

    let lap = LAPLACIAN_W.x * (p_xm + p_xp - 2.0 * p_c)
            + LAPLACIAN_W.y * (p_ym + p_yp - 2.0 * p_c)
            + LAPLACIAN_W.z * (p_zm + p_zp - 2.0 * p_c);

    let is_wall = ix == 0u || ix == NX - 1u || iy == 0u || iy == NY - 1u || iz == 0u || iz == NZ - 1u;
    let step_s = clamp(audio.frame_dt * 60.0, 0.20, 1.08);
    let osc = audio.channel_phases[4].y;
    let wpos = voxel_world_pos(ix, iy, iz);
    let inj = compute_source_injection(wpos, osc);

    let p_next = fdtd_step_pressure(p_c, p_prev, lap, inj.pressure, is_wall, step_s);

    // Spatial gradient kinetic energy <|grad p|^2> highlights destructive cancellation nodal planes
    // where p ~ 0 while opposite wavefronts collide steeply across zero.
    let gx = (p_xp - p_xm) * 0.5 * sqrt(LAPLACIAN_W.x);
    let gy = (p_yp - p_ym) * 0.5 * sqrt(LAPLACIAN_W.y);
    let gz = (p_zp - p_zm) * 0.5 * sqrt(LAPLACIAN_W.z);
    let grad2_inst = gx * gx + gy * gy + gz * gz;
    let p2_inst = p_next * p_next;

    let ema_alpha = clamp(0.09 * step_s, 0.02, 0.14);
    let p2_ema = mix(a_prev.y, p2_inst, ema_alpha);
    let grad2_ema = mix(a_prev.z, grad2_inst, ema_alpha);

    // Diffuse normalized spectral role tint cleanly outward from speakers and objects along wave fronts
    let n_band = (
        wave_a[idx_xm].w + wave_a[idx_xp].w +
        wave_a[idx_ym].w + wave_a[idx_yp].w +
        wave_a[idx_zm].w + wave_a[idx_zp].w
    ) * (1.0 / 6.0);
    let diffused_band = mix(a_prev.w, n_band, 0.28);
    let band_blend = select(
        diffused_band,
        mix(diffused_band, inj.band_sum / max(inj.band_weight, 1e-4), clamp(inj.band_weight * 0.65, 0.0, 0.85)),
        inj.band_weight > 0.002
    );

    wave_a[idx] = vec4<f32>(p_next, p2_ema, grad2_ema, band_blend);

    // Soft-shoulder RGBA8Unorm packing preserves standing-wave peaks and nodal contrast under bass:
    // R = RMS standing wave intensity sqrt(<p^2>)
    // G = RMS nodal particle velocity gradient sqrt(<|grad p|^2>)
    // B = Instantaneous signed pressure biased around 0.5 (compression >0.5, rarefaction <0.5)
    // A = Diffused speaker/object spectral role tint [0..1]
    let p_rms = sqrt(max(p2_ema, 0.0));
    let g_rms = sqrt(max(grad2_ema, 0.0));
    let pack_r = clamp((p_rms * 2.05) / (1.0 + p_rms * 1.85), 0.0, 1.0);
    let pack_g = clamp((g_rms * 2.80) / (1.0 + g_rms * 1.50), 0.0, 1.0);
    let p_soft = p_next / (1.0 + abs(p_next) * 0.85);
    let pack_b = clamp(p_soft * 0.48 + 0.5, 0.0, 1.0);
    let pack_a = clamp(band_blend, 0.0, 1.0);
    packed_vol[idx] = pack_rgba8_unorm(vec4<f32>(pack_r, pack_g, pack_b, pack_a));
}
