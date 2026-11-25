// =====================================================
// RustTracker — Neon Acoustic Studio Visualizer
// Hardware-rasterized 3D listening studio (speaker towers,
// reactive cone piston excursion, LED meters, observation
// EQ window) merged with the 64x32x64 3D FDTD acoustic
// wave simulation as subtle illuminated smoke-machine blasts
// and standing-wave vapor.
// =====================================================

// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var<uniform> camera: CameraUniforms;
@group(2) @binding(0) var acoustic_vol_tex: texture_3d<f32>;
@group(2) @binding(1) var acoustic_vol_sampler: sampler;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
}

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) pos_mat: vec4<f32>,
    @location(1) norm_ch: vec4<f32>,
    @location(2) fill_diff_cw: vec4<f32>,
    @location(3) fill_spec_che: vec4<f32>,
    @location(4) ro_fog: vec4<f32>,
    @location(5) ch_levels_0: vec4<f32>,
    @location(6) ch_levels_1: vec4<f32>,
    @location(7) ch_levels_2: vec4<f32>,
}

const ROOM_MIN: vec3<f32> = vec3<f32>(-10.8, -1.5, -8.9);
const ROOM_MAX: vec3<f32> = vec3<f32>(10.8, 4.9, 8.9);
const ROOM_SIZE: vec3<f32> = vec3<f32>(21.6, 6.4, 17.8);
const VOL_STEPS: u32 = 14u;
const JET_STEPS: u32 = 5u;

// Distinct neon colors for 7.1.4 spatial speaker roles (module-scope LUT)
const SPEAKER_COLORS: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(0.0, 0.85, 1.0),   // 0: Front Left (Electric Cyan)
    vec3<f32>(1.0, 0.08, 0.75),  // 1: Front Right (Vivid Magenta)
    vec3<f32>(1.0, 0.75, 0.10),  // 2: Center (Amber Gold)
    vec3<f32>(1.0, 0.22, 0.02),  // 3: LFE Subwoofer (Intense Neon Orange/Red)
    vec3<f32>(0.25, 0.40, 1.0),  // 4: Surround Left (Royal Blue)
    vec3<f32>(0.80, 0.20, 1.0),  // 5: Surround Right (Electric Purple)
    vec3<f32>(0.0, 1.0, 0.75),   // 6: Rear Left (Neon Mint / Teal)
    vec3<f32>(0.95, 0.0, 0.45),  // 7: Rear Right (Deep Rose)
    vec3<f32>(0.35, 1.0, 0.35),  // 8: Top Front Left (Emerald)
    vec3<f32>(0.85, 1.0, 0.20),  // 9: Top Front Right (Lime)
    vec3<f32>(0.20, 0.90, 0.90), // 10: Top Rear Left (Aquamarine)
    vec3<f32>(1.0, 0.45, 0.85)   // 11: Top Rear Right (Hot Pink)
);

// Speaker source positions in room (matching exact centers in generate_neon_room_mesh)
const SPEAKER_POS: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(-3.8, -0.25,  5.5), // FL (0)
    vec3<f32>( 3.8, -0.25,  5.5), // FR (1)
    vec3<f32>( 0.0, -0.02,  5.32), // C (2) — stacked on top of Sub
    vec3<f32>( 0.0, -0.90,  5.50), // Sub (3) — base on stage riser
    vec3<f32>(-6.2,  0.50,  0.5), // SL (4)
    vec3<f32>( 6.2,  0.50,  0.5), // SR (5)
    vec3<f32>(-4.2,  0.10, -4.8), // RL (6)
    vec3<f32>( 4.2,  0.10, -4.8), // RR (7)
    vec3<f32>(-3.2,  4.32,  3.5), // TFL (8)
    vec3<f32>( 3.2,  4.32,  3.5), // TFR (9)
    vec3<f32>(-3.2,  4.32, -2.5), // TRL (10)
    vec3<f32>( 3.2,  4.32, -2.5)  // TRR (11)
);

// Grounded anchor Y coordinate for each speaker cabinet (prevents floor/plinth penetration)
const SPEAKER_ANCHOR_Y: array<f32, 12> = array<f32, 12>(
    -1.35, // 0: FL tower base on pedestal
    -1.35, // 1: FR tower base on pedestal
    -0.35, // 2: C bottom on isolation plinth
    -1.36, // 3: LFE sub base on stage riser
    -0.11, // 4: SL monitor bottom on rigid stand
    -0.11, // 5: SR monitor bottom on rigid stand
    -1.00, // 6: RL tower base on pedestal
    -1.00, // 7: RR tower base on pedestal
     4.55, // 8: TFL ceiling mount
     4.55, // 9: TFR ceiling mount
     4.55, // 10: TRL ceiling mount
     4.55  // 11: TRR ceiling mount
);

// Speaker front baffle nozzles & normalized inward firing directions for smoke blasts
const SPEAKER_NOZZLE: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(-3.63, -0.45,  5.07), // 0: FL
    vec3<f32>( 3.63, -0.45,  5.07), // 1: FR
    vec3<f32>( 0.00, -0.02,  4.96), // 2: C
    vec3<f32>( 0.00, -0.82,  4.92), // 3: LFE dual subwoofer port / drivers
    vec3<f32>(-5.89,  0.38,  0.34), // 4: SL
    vec3<f32>( 5.89,  0.38,  0.34), // 5: SR
    vec3<f32>(-3.90,  0.05, -4.45), // 6: RL
    vec3<f32>( 3.90,  0.05, -4.45), // 7: RR
    vec3<f32>(-3.20,  4.04,  3.50), // 8: TFL
    vec3<f32>( 3.20,  4.04,  3.50), // 9: TFR
    vec3<f32>(-3.20,  4.04, -2.50), // 10: TRL
    vec3<f32>( 3.20,  4.04, -2.50)  // 11: TRR
);

const SPEAKER_DIR: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>( 0.371,  0.020, -0.928), // 0: FL
    vec3<f32>(-0.371,  0.020, -0.928), // 1: FR
    vec3<f32>( 0.000,  0.030, -1.000), // 2: C
    vec3<f32>( 0.000,  0.050, -0.999), // 3: LFE
    vec3<f32>( 0.887, -0.030, -0.460), // 4: SL
    vec3<f32>(-0.887, -0.030, -0.460), // 5: SR
    vec3<f32>( 0.658,  0.020,  0.753), // 6: RL
    vec3<f32>(-0.658,  0.020,  0.753), // 7: RR
    vec3<f32>( 0.000, -1.000,  0.000), // 8: TFL
    vec3<f32>( 0.000, -1.000,  0.000), // 9: TFR
    vec3<f32>( 0.000, -1.000,  0.000), // 10: TRL
    vec3<f32>( 0.000, -1.000,  0.000)  // 11: TRR
);

fn get_spatial_color(ch: u32) -> vec3<f32> {
    return SPEAKER_COLORS[min(ch, 11u)];
}

fn world_to_uvw(p: vec3<f32>) -> vec3<f32> {
    return clamp((p - ROOM_MIN) / ROOM_SIZE, vec3<f32>(0.008), vec3<f32>(0.992));
}

fn interleaved_gradient_noise(pixel_xy: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel_xy, vec2<f32>(0.06711056, 0.00583715))));
}

fn smoke_billow_3d(p: vec3<f32>, t: f32) -> f32 {
    let q1 = p * vec3<f32>(1.15, 1.40, 1.15) + vec3<f32>(t * 0.420, -t * 0.310, t * 0.370);
    let warp = sin(p.yzx * vec3<f32>(1.008, 0.828, 0.828) + vec3<f32>(t * 0.240, -t * 0.190, t * 0.280)) * 0.62;
    let w1 = q1 + warp;
    let g1 = dot(sin(w1), cos(w1.yzx));
    let fold1 = 1.0 - min(abs(g1) * 0.6667, 1.0);

    let q2 = p * vec3<f32>(1.95, 2.15, 1.85) + warp * 1.15 + vec3<f32>(-t * 0.680, -t * 0.510, t * 0.590);
    let g2 = dot(sin(q2), cos(q2.yzx));
    let fold2 = 1.0 - min(abs(g2) * 0.6667, 1.0);

    return smoothstep(0.38, 0.85, fold1 * 0.68 + fold2 * 0.32);
}

fn eval_speaker_jet(
    wpos: vec3<f32>,
    sp_idx: u32,
    jet_str: f32,
    wave_puff: f32,
    billow: f32
) -> f32 {
    if (jet_str <= 0.01) {
        return 0.0;
    }
    let idx = min(sp_idx, 11u);
    let rel = wpos - SPEAKER_NOZZLE[idx];
    let dir = SPEAKER_DIR[idx];
    let along = dot(rel, dir);
    let reach = select(1.4 + 2.8 * jet_str, 1.8 + 3.6 * jet_str, idx == 3u);
    if (along < -0.15 || along > reach) {
        return 0.0;
    }
    let axial_t = clamp(along / max(reach, 1e-4), 0.0, 1.0);
    var perp = rel - dir * along;
    if (idx == 3u) {
        // Wide horizontal dual-15" subwoofer blast fan
        perp.x *= 0.58;
    }
    let rad2 = dot(perp, perp);
    let base_r = select(0.20, 0.32, idx == 3u);
    let cone_r = base_r + along * select(0.28, 0.36, idx == 3u);
    let radial_falloff = exp(-rad2 / max(cone_r * cone_r, 0.02));
    let axial_env = smoothstep(-0.08, 0.26, along) * pow(max(1.0 - axial_t, 1e-4), 1.35);
    let texture_mod = mix(0.75 + 0.55 * billow, 0.15 + 1.35 * billow, smoothstep(0.08, 0.58, axial_t));
    return jet_str * radial_falloff * axial_env * wave_puff * texture_mod;
}

fn integrate_speaker_smoke_plume(
    ro: vec3<f32>,
    rd: vec3<f32>,
    sp_idx: u32,
    jet_str: f32,
    t_a: f32,
    t_b: f32
) -> vec4<f32> {
    if (jet_str <= 0.01 || t_b <= t_a + 0.03) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let idx = min(sp_idx, 11u);
    let dt = (t_b - t_a) / f32(JET_STEPS);
    let smoke_col = mix(vec3<f32>(0.70, 0.75, 0.84), SPEAKER_COLORS[idx], 0.50);

    var accum = vec3<f32>(0.0);
    var trans = 1.0;

    for (var s = 0u; s < JET_STEPS; s = s + 1u) {
        let t = t_a + (f32(s) + 0.5) * dt;
        let wpos = ro + rd * t;
        let vol = textureSampleLevel(acoustic_vol_tex, acoustic_vol_sampler, world_to_uvw(wpos), 0.0);
        let p_inst = vol.b * 2.0 - 1.0;
        let wave_puff = 0.55 + 0.65 * smoothstep(-0.15, 0.45, p_inst);
        let billow = smoke_billow_3d(wpos, audio.smooth_time);
        let j = eval_speaker_jet(wpos, idx, jet_str, wave_puff, billow);
        if (j > 0.002) {
            let near_fade = smoothstep(0.28, 1.45, t);
            let sigma_j = j * 0.22 * near_fade;
            let step_alpha = 1.0 - exp(-sigma_j * dt);
            let in_scatter = smoke_col * (0.42 + 0.85 * j);
            accum += trans * in_scatter * step_alpha;
            trans *= (1.0 - step_alpha);
        }
    }
    return vec4<f32>(accum, trans);
}

fn get_channel_level_bands(ch: u32, bass_e: f32, lowmid_e: f32, mid_e: f32, high_e: f32) -> f32 {
    let idx = min(ch, 11u);

    if (audio.num_spatial_channels == 4u) {
        let t0 = audio.spatial_channels[0].x;
        let t1 = audio.spatial_channels[0].y;
        let t2 = audio.spatial_channels[0].z;
        let t3 = audio.spatial_channels[0].w;
        switch idx {
            case 0u: { return t0; }
            case 1u: { return t1; }
            case 2u: { return clamp((t0 + t1 + t2 + t3) * 0.25 + mid_e * 0.30, 0.0, 1.0); }
            case 3u: { return clamp(bass_e * 0.85 + max(max(t0, t1), max(t2, t3)) * 0.30, 0.0, 1.0); }
            case 4u: { return t2; }
            case 5u: { return t3; }
            case 6u: { return clamp(t2 * 0.85 + t0 * 0.25, 0.0, 1.0); }
            case 7u: { return clamp(t3 * 0.85 + t1 * 0.25, 0.0, 1.0); }
            case 8u: { return t0; }
            case 9u: { return t1; }
            case 10u: { return t2; }
            default: { return t3; }
        }
    }

    if (audio.num_spatial_channels > 2u) {
        if (audio.num_spatial_channels == 6u && audio.num_channels == 6u) {
            if (idx < 6u) {
                return audio.spatial_channels[idx / 4u][idx % 4u];
            } else if (idx == 6u) {
                return audio.spatial_channels[1].x;
            } else if (idx == 7u) {
                return audio.spatial_channels[1].y;
            } else {
                let base_idx = idx - 8u;
                return clamp(audio.spatial_channels[0][base_idx] * 0.65 + high_e * 0.40, 0.0, 1.0);
            }
        }
        let actual_ch = min(audio.num_spatial_channels, 16u);
        let src_ch = idx % actual_ch;
        let ch_val = audio.spatial_channels[src_ch / 4u][src_ch % 4u];
        if (idx == 3u && audio.num_spatial_channels < audio.num_channels) {
            return clamp(ch_val * 0.55 + bass_e * 0.65, 0.0, 1.0);
        }
        return ch_val;
    }

    let ch_l = audio.spatial_channels[0].x;
    let ch_r = select(ch_l, audio.spatial_channels[0].y, audio.num_spatial_channels >= 2u);
    let phase_l = audio.channel_phases[0].x;
    let phase_r = audio.channel_phases[0].y;
    return stereo_upmix_12(idx, ch_l, ch_r, phase_l, phase_r, bass_e, lowmid_e, mid_e, high_e);
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;

    let ch_idx = min(u32(round(in.tex_coords.x)), 11u);
    let raw_mat = in.tex_coords.y;
    let is_cone = raw_mat >= 1.8 && raw_mat <= 4.4;
    let mat_id = select(raw_mat, floor(raw_mat + 0.05), is_cone);
    let cone_center_weight = select(0.0, clamp((raw_mat - mat_id) * 4.0, 0.0, 1.0), is_cone);

    // Skip FFT DC bin 0 (audio.spectrum[0].x) so pure sub-bass never biases piston/room energy
    let bass_e = clamp((audio.spectrum[0].y + audio.spectrum[0].z + audio.spectrum[0].w) / 195.0, 0.0, 1.0);
    let lowmid_e = clamp((audio.spectrum[3].x + audio.spectrum[5].x) / 130.0, 0.0, 1.0);
    let mid_e = clamp((audio.spectrum[10].x + audio.spectrum[16].x) / 120.0, 0.0, 1.0);
    let high_e = clamp((audio.spectrum[32].x + audio.spectrum[64].x) / 110.0, 0.0, 1.0);

    let ch_energy_raw = get_channel_level_bands(ch_idx, bass_e, lowmid_e, mid_e, high_e);
    let ch_energy = clamp(ch_energy_raw, 0.0, 1.0);
    let bass_pulse = clamp((audio.spectrum[0].y + audio.spectrum[0].z) / 135.0, 0.0, 1.2);
    let bass_env = clamp(max(audio.channel_phases[7].z, bass_pulse * 0.75), 0.0, 1.0);

    var pos = in.position;
    let is_speaker = (mat_id > 1.1 && mat_id < 4.5) || (mat_id > 5.1 && mat_id < 5.3);

    // 1. Whole-Speaker Grounded Pressure Swell (anchored at cabinet base/mount, no jittery hop/buzz)
    if (is_speaker) {
        let sp_center = SPEAKER_POS[ch_idx];
        let anchor = vec3<f32>(sp_center.x, SPEAKER_ANCHOR_Y[ch_idx], sp_center.z);
        let rel = pos - anchor;
        let cab_drive = ch_energy * ch_energy;
        let scale_xz = 1.0
            + cab_drive * select(0.032, 0.024, ch_idx == 3u)
            + bass_env * select(0.010, 0.018, ch_idx == 3u);
        let y_expand = select(select(0.025, 0.018, ch_idx == 2u), 0.006, ch_idx == 3u);
        let scale_y = 1.0 + cab_drive * y_expand;
        pos = anchor + vec3<f32>(rel.x * scale_xz, rel.y * scale_y, rel.z * scale_xz);
    }

    // 2. Woofer / Midrange / Tweeter Cone Piston Excursion along Normal (sealed rim, proportional travel)
    var displacement = 0.0;
    if (is_cone) {
        let radial_excursion = cone_center_weight;
        if (ch_idx == 3u) {
            displacement = (ch_energy * 0.55 + bass_env * 0.45) * 0.16 * radial_excursion;
        } else if (mat_id < 2.5) {
            displacement = (ch_energy * 0.70 + bass_env * 0.30) * 0.11 * radial_excursion;
        } else if (mat_id < 3.5) {
            displacement = ch_energy * 0.065 * radial_excursion;
        } else {
            displacement = ch_energy * 0.025 * radial_excursion;
        }
    }

    let displaced_pos = pos + in.normal * displacement;

    let v_t = camera.view_matrix[3].xyz;
    let ro_v = -vec3<f32>(
        dot(camera.view_matrix[0].xyz, v_t),
        dot(camera.view_matrix[1].xyz, v_t),
        dot(camera.view_matrix[2].xyz, v_t)
    );

    let N_v = normalize(in.normal);
    let V_vec = ro_v - displaced_pos;

    if (!is_cone && dot(N_v, V_vec) < -0.02) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.pos_mat = vec4<f32>(0.0);
        out.norm_ch = vec4<f32>(0.0);
        out.fill_diff_cw = vec4<f32>(0.0);
        out.fill_spec_che = vec4<f32>(0.0);
        out.ro_fog = vec4<f32>(0.0);
        out.ch_levels_0 = vec4<f32>(0.0);
        out.ch_levels_1 = vec4<f32>(0.0);
        out.ch_levels_2 = vec4<f32>(0.0);
        return out;
    }

    let dist_to_cam = length(V_vec);
    let V_v = V_vec / max(dist_to_cam, 1e-6);
    let NdotV_v = dot(N_v, V_v);
    let use_spec64 = mat_id < 1.0;

    let all_levels = array<f32, 12>(
        clamp(get_channel_level_bands(0u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(1u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(2u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(3u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(4u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(5u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(6u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(7u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(8u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(9u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(10u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0),
        clamp(get_channel_level_bands(11u, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.0)
    );
    var total_diff = vec3<f32>(0.0);
    var total_spec = vec3<f32>(0.0);
    for (var sp_i = 0u; sp_i < 12u; sp_i = sp_i + 1u) {
        let L_v = SPEAKER_POS[sp_i] - displaced_pos;
        let d2 = dot(L_v, L_v);
        let L_dir = L_v * inverseSqrt(max(d2, 1e-6));
        let ndl = dot(N_v, L_dir);
        let ndlv = ndl + NdotV_v;
        if (ndl > 0.0 || ndlv > 0.0) {
            let e_k = all_levels[sp_i];
            let att = (e_k * e_k * 4.1 + 0.035) / (d2 * 0.42 + 1.1);
            let s_col = SPEAKER_COLORS[sp_i];
            if (ndl > 0.0) {
                total_diff += s_col * (ndl * att);
            }
            if (ndlv > 0.0 && sp_i < 6u) {
                let NdotH = clamp(ndlv * inverseSqrt(max(2.0 + 2.0 * dot(L_dir, V_v), 1e-6)), 0.0, 1.0);
                let nh2 = NdotH * NdotH;
                let nh4 = nh2 * nh2;
                let nh8 = nh4 * nh4;
                let nh16 = nh8 * nh8;
                let spec_val = select(nh16 * nh8, (nh16 * nh16) * (nh16 * nh16), use_spec64);
                total_spec += s_col * (spec_val * att);
            }
        }
    }

    out.pos_mat = vec4<f32>(displaced_pos, mat_id);
    out.norm_ch = vec4<f32>(in.normal, in.tex_coords.x);
    out.fill_diff_cw = vec4<f32>(total_diff, cone_center_weight);
    out.fill_spec_che = vec4<f32>(total_spec, clamp(ch_energy_raw, 0.0, 1.0));
    out.ro_fog = vec4<f32>(ro_v, smoothstep(12.0, 45.0, dist_to_cam));
    out.ch_levels_0 = vec4<f32>(all_levels[0], all_levels[1], all_levels[2], all_levels[3]);
    out.ch_levels_1 = vec4<f32>(all_levels[4], all_levels[5], all_levels[6], all_levels[7]);
    out.ch_levels_2 = vec4<f32>(all_levels[8], all_levels[9], all_levels[10], all_levels[11]);

    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(displaced_pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let P = in.pos_mat.xyz;
    let mat_id = in.pos_mat.w;
    let N = in.norm_ch.xyz;
    let ch_idx = min(u32(round(in.norm_ch.w)), 11u);
    let ch_col = get_spatial_color(ch_idx);
    let ch_energy = in.fill_spec_che.w;

    let ro = in.ro_fog.xyz;
    let point_light_diffuse = in.fill_diff_cw.xyz;
    let point_light_specular = in.fill_spec_che.xyz;
    let smoke_enabled = audio._pad2 < 0.5;

    var albedo = vec3<f32>(0.03, 0.03, 0.04);
    var emission = vec3<f32>(0.0);
    var roughness = 0.6;

    if (mat_id < 0.2) {
        // === Material 0.0: Studio Floor Grid, Speaker Pools & Floor Chladni Vapor ===
        let grid_size = 1.0;
        let gx = abs(fract(P.x / grid_size) - 0.5);
        let gz = abs(fract(P.z / grid_size) - 0.5);
        let line_w = 0.028;
        let grid_line = smoothstep_r(line_w, 0.0, min(gx, gz));

        let floor_base = vec3<f32>(0.012, 0.013, 0.020);
        let floor_grid_col = vec3<f32>(0.06, 0.14, 0.28);
        albedo = mix(floor_base, floor_grid_col, grid_line * 0.65);
        roughness = 0.18;

        let floor_levels = array<f32, 6>(
            in.ch_levels_0.x, in.ch_levels_0.y, in.ch_levels_0.z, in.ch_levels_0.w,
            in.ch_levels_1.x, in.ch_levels_1.y
        );
        for (var i = 0u; i < 6u; i = i + 1u) {
            let s_energy = floor_levels[i];
            if (s_energy > 0.04) {
                let s_pos = SPEAKER_POS[i];
                let diff_xz = P.xz - s_pos.xz;
                let d_sp2 = dot(diff_xz, diff_xz);
                if (d_sp2 < 30.25) {
                    let s_col = SPEAKER_COLORS[i];
                    let base_pool = (1.0 / (1.0 + d_sp2 * 0.65)) * (0.4 + 0.6 * grid_line);
                    emission += s_col * (base_pool * 0.42) * (s_energy * s_energy);
                }
            }
        }

        if (smoke_enabled) {
            let vol_floor = textureSampleLevel(
                acoustic_vol_tex,
                acoustic_vol_sampler,
                world_to_uvw(P + vec3<f32>(0.0, 0.26, 0.0)),
                0.0
            );
            let standing_f = vol_floor.r;
            let nodal_f = vol_floor.g;
            let p_inst_f = vol_floor.b * 2.0 - 1.0;
            let band_f = vol_floor.a;

            let antinode_raw = smoothstep(0.18, 0.68, standing_f);
            let antinode_floor = antinode_raw * antinode_raw * (0.55 + 0.45 * abs(p_inst_f));
            let bass_floor = smoothstep_r(0.36, 0.06, band_f);
            let cymatic_col = mix(vec3<f32>(0.08, 0.62, 0.94), vec3<f32>(0.92, 0.36, 0.12), bass_floor);

            let null_zone = smoothstep_r(0.26, 0.04, standing_f);
            let nodal_line = null_zone * smoothstep(0.22, 0.64, nodal_f);

            emission += cymatic_col * antinode_floor * 0.16;
            emission += vec3<f32>(0.42, 0.16, 0.95) * nodal_line * 0.14;
        }
    } else if (mat_id < 0.8) {
        // === Material 0.5: Acoustic Walls & Ceiling Grid (Triplanar) ===
        var uv_wall: vec2<f32>;
        if (abs(N.x) > 0.5) {
            uv_wall = P.zy;
        } else if (abs(N.z) > 0.5) {
            uv_wall = P.xy;
        } else {
            uv_wall = P.xz;
        }
        let wall_grid_x = abs(fract(uv_wall.x * 0.5) - 0.5);
        let wall_grid_y = abs(fract(uv_wall.y * 0.5) - 0.5);
        let grid_lines = smoothstep_r(0.03, 0.0, min(wall_grid_x, wall_grid_y));

        let panel_pattern = sin(uv_wall.x * 3.14159) * sin(uv_wall.y * 3.14159) * 0.015;
        let wall_base = vec3<f32>(0.018, 0.02, 0.03) + vec3<f32>(panel_pattern);
        let wall_neon_accent = vec3<f32>(0.08, 0.16, 0.35);
        albedo = mix(wall_base, wall_neon_accent, grid_lines * 0.7);
        roughness = 0.65;
    } else if (mat_id < 1.1) {
        // === Material 1.0: Static Stage Platform Riser ===
        albedo = vec3<f32>(0.022, 0.022, 0.028);
        roughness = 0.35;
    } else if (mat_id < 1.5) {
        // === Material 1.2: Active Speaker Cabinets (with Segmented Vertical VU Meter & Color Shift) ===
        let sp_pos = SPEAKER_POS[ch_idx];
        var h_u = clamp((P.y - (sp_pos.y - 1.05)) / 2.1, 0.0, 1.0);
        if (ch_idx == 4u || ch_idx == 5u) {
            h_u = clamp((P.y - (sp_pos.y - 0.58)) / 1.16, 0.0, 1.0);
        } else if (ch_idx == 2u) {
            h_u = clamp((P.y - (sp_pos.y - 0.30)) / 0.60, 0.0, 1.0);
        } else if (ch_idx == 3u) {
            h_u = clamp((P.y - (sp_pos.y - 0.46)) / 0.92, 0.0, 1.0);
        } else if (ch_idx >= 8u) {
            h_u = clamp(1.0 - (P.y - 4.08) / 0.46, 0.0, 1.0);
        }

        let seg_f = fract(h_u * 14.0);
        let led_seg = smoothstep(0.14, 0.24, seg_f) * smoothstep_r(0.88, 0.78, seg_f);
        let is_lit = smoothstep_r(ch_energy + 0.05, ch_energy - 0.02, h_u) * smoothstep(0.02, 0.08, ch_energy);
        let peak_tint = vec3<f32>(1.0, 0.92, 0.60);
        let dynamic_col = mix(ch_col, peak_tint, smoothstep(0.55, 0.98, h_u) * ch_energy);

        let ch_e15 = ch_energy * sqrt(max(ch_energy, 0.0));
        albedo = mix(vec3<f32>(0.020, 0.022, 0.028), dynamic_col * 0.25, ch_energy * 0.7);
        emission = dynamic_col * (led_seg * is_lit * 1.45 + ch_e15 * 0.35);
        roughness = 0.25;
    } else if (mat_id < 2.5) {
        // === Material 2.0: Active Bass Woofer Diaphragms ===
        let cw = in.fill_diff_cw.w;
        let is_surround = smoothstep_r(0.16, 0.04, cw);
        let is_dustcap = smoothstep(0.65, 0.75, cw);
        let cone_ribs = 0.86 + 0.14 * sin(cw * 36.0);
        let cone_base = vec3<f32>(0.030, 0.032, 0.042) * cone_ribs;
        let hot_core = vec3<f32>(1.0, 0.95, 0.85);
        let active_col = mix(ch_col, hot_core, is_dustcap * clamp(ch_energy * 0.85, 0.0, 0.75));
        let glow_intensity = ch_energy * 3.8 * (1.0 - is_surround * 0.5) * (0.65 + 0.75 * is_dustcap) * cone_ribs;
        emission = active_col * (0.04 + glow_intensity);
        albedo = mix(cone_base, active_col * 0.45, clamp(ch_energy, 0.0, 1.0));
        roughness = mix(0.20, 0.06, is_dustcap);
    } else if (mat_id < 3.5) {
        // === Material 3.0: Active Midrange Diaphragms ===
        let cw = in.fill_diff_cw.w;
        let is_plug = smoothstep(0.68, 0.76, cw);
        let cone_ribs = 0.86 + 0.14 * sin(cw * 30.0);
        let hot_core = vec3<f32>(1.0, 0.96, 0.90);
        let active_col = mix(ch_col, hot_core, is_plug * clamp(ch_energy * 0.8, 0.0, 0.7));
        let mid_glow = ch_energy * (3.4 + 1.2 * is_plug) * cone_ribs;
        emission = active_col * (0.04 + mid_glow);
        albedo = mix(vec3<f32>(0.04) * cone_ribs, active_col * 0.4, clamp(ch_energy, 0.0, 1.0));
        roughness = mix(0.24, 0.08, is_plug);
    } else if (mat_id < 4.5) {
        // === Material 4.0: Titanium / Beryllium Tweeter Dome ===
        let cw = in.fill_diff_cw.w;
        let dome_glow = 0.75 + 0.65 * cw;
        let active_col = mix(ch_col, vec3<f32>(1.0, 0.98, 0.95), clamp(ch_energy * 0.65, 0.0, 0.65));
        emission = active_col * (0.05 + ch_energy * 3.6) * dome_glow;
        albedo = vec3<f32>(0.10, 0.11, 0.13);
        roughness = 0.06;
    } else if (mat_id < 5.1) {
        // === Material 5.0: Architectural Room Neon Rails & Panel Frames ===
        emission = ch_col * (0.75 + ch_energy * 1.4);
        albedo = ch_col * 0.5;
        roughness = 0.1;
    } else if (mat_id < 5.5) {
        // === Material 5.2: Speaker Driver Bezels & Cabinet Corner Neon Strips ===
        let hot_boost = vec3<f32>(1.0, 0.95, 0.88) * max(ch_energy - 0.55, 0.0) * 2.8;
        emission = ch_col * (0.08 + ch_energy * 4.2) + hot_boost;
        albedo = ch_col;
        roughness = 0.08;
    } else if (mat_id < 6.5) {
        // === Material 6.0: Acoustic Diffuser Slats (Walnut Texture, Triplanar) ===
        var uv_wood: vec2<f32>;
        if (abs(N.x) > 0.5) {
            uv_wood = P.zy;
        } else {
            uv_wood = P.xy;
        }
        let wood_grain = value_noise2d(vec2<f32>(uv_wood.x * 18.0, uv_wood.y * 3.5));
        let slat_ring = sin(uv_wood.y * 6.28 + wood_grain * 1.2) * 0.015;
        albedo = vec3<f32>(0.13, 0.07, 0.035) + vec3<f32>(0.035, 0.02, 0.01) * wood_grain + vec3<f32>(slat_ring);
        roughness = 0.72;
    } else {
        // === Material 7.0: Soundproof Studio Control Room Observation Window ===
        let glass_base = vec3<f32>(0.008, 0.020, 0.038);
        let V_win = normalize(ro - P);
        let nv_c = 1.0 - max(dot(N, V_win), 0.0);
        let fresnel = nv_c * nv_c * nv_c;
        let glass_reflection = vec3<f32>(0.08, 0.24, 0.48) * fresnel;

        let win_u = clamp((P.x + 4.5) / 9.0, 0.0, 1.0);
        let win_v = clamp((P.y - 0.55) / 2.2, 0.0, 1.0);
        let col_idx = u32(clamp(floor(win_u * 32.0), 0.0, 31.0));
        let col_fract = fract(win_u * 32.0);
        let row_fract = fract(win_v * 18.0);
        let led_cell = smoothstep(0.12, 0.22, col_fract) * smoothstep_r(0.88, 0.78, col_fract)
                     * smoothstep(0.15, 0.25, row_fract) * smoothstep_r(0.85, 0.75, row_fract);
        let bin_idx = col_idx * 3u;
        let eq_val = clamp(audio.spectrum[bin_idx / 4u][bin_idx % 4u] / 72.0, 0.0, 1.0);
        let is_lit = step(win_v, eq_val) * step(0.02, win_v) * step(win_v, 0.96);
        let eq_col = mix(vec3<f32>(0.0, 0.90, 1.0), vec3<f32>(1.0, 0.08, 0.65), win_v);

        albedo = glass_base;
        emission = glass_reflection * 1.8 + eq_col * led_cell * is_lit * 0.85;
        roughness = 0.06;
    }

    let ambient_col = vec3<f32>(0.015, 0.018, 0.028);
    var final_color = albedo * (ambient_col + point_light_diffuse) + point_light_specular * (1.0 - roughness) + emission;

    // === Volumetric Smoke-Machine Blasts + Subtle Acoustic Standing-Wave Vapor ===
    if (smoke_enabled) {
        let ray_vec = P - ro;
        let t_hit = length(ray_vec);
        let rd = ray_vec / max(t_hit, 1e-4);
        let t_start = 0.32;
        let t_end = min(t_hit, 17.5);

        if (t_end > t_start + 0.02) {
            let ch_lvls = array<f32, 12>(
                in.ch_levels_0.x, in.ch_levels_0.y, in.ch_levels_0.z, in.ch_levels_0.w,
                in.ch_levels_1.x, in.ch_levels_1.y, in.ch_levels_1.z, in.ch_levels_1.w,
                in.ch_levels_2.x, in.ch_levels_2.y, in.ch_levels_2.z, in.ch_levels_2.w
            );

            // Pre-loop per-ray bounding-sphere culling: keep up to top 3 closest active jets + analytic chords
            var jet0_idx: u32 = 0u;
            var jet0_str: f32 = 0.0;
            var jet0_score: f32 = 1e9;
            var jet0_ta: f32 = 0.0;
            var jet0_tb: f32 = 0.0;
            var jet1_idx: u32 = 0u;
            var jet1_str: f32 = 0.0;
            var jet1_score: f32 = 1e9;
            var jet1_ta: f32 = 0.0;
            var jet1_tb: f32 = 0.0;
            var jet2_idx: u32 = 0u;
            var jet2_str: f32 = 0.0;
            var jet2_score: f32 = 1e9;
            var jet2_ta: f32 = 0.0;
            var jet2_tb: f32 = 0.0;
            let bass_kick = clamp(audio.channel_phases[7].z, 0.0, 1.0);

            for (var k = 0u; k < 12u; k = k + 1u) {
                let raw_s = smoothstep(0.14, 0.85, ch_lvls[k]);
                let js = select(raw_s, max(raw_s, bass_kick * 0.92), k == 3u);
                if (js > 0.02) {
                    let reach = select(1.4 + 2.8 * js, 1.8 + 3.6 * js, k == 3u);
                    let sphere_c = SPEAKER_NOZZLE[k] + SPEAKER_DIR[k] * (reach * 0.48);
                    let bound_r = select(reach * 0.62 + 0.55, reach * 0.80 + 0.75, k == 3u);
                    let oc = sphere_c - ro;
                    let t_proj = dot(oc, rd);
                    if (t_proj > -bound_r && t_proj < t_hit + bound_r) {
                        let perp_d2 = dot(oc, oc) - t_proj * t_proj;
                        if (perp_d2 < bound_r * bound_r) {
                            let half_chord = sqrt(max(bound_r * bound_r - max(perp_d2, 0.0), 0.0));
                            let ja = max(t_proj - half_chord, 0.28);
                            let jb = min(t_proj + half_chord, t_hit);
                            if (jb > ja + 0.03) {
                                let score = max(perp_d2, 0.0) / max(js * js, 0.05);
                                if (score < jet0_score) {
                                    jet2_idx = jet1_idx;
                                    jet2_str = jet1_str;
                                    jet2_score = jet1_score;
                                    jet2_ta = jet1_ta;
                                    jet2_tb = jet1_tb;
                                    jet1_idx = jet0_idx;
                                    jet1_str = jet0_str;
                                    jet1_score = jet0_score;
                                    jet1_ta = jet0_ta;
                                    jet1_tb = jet0_tb;
                                    jet0_idx = k;
                                    jet0_str = js;
                                    jet0_score = score;
                                    jet0_ta = ja;
                                    jet0_tb = jb;
                                } else if (score < jet1_score) {
                                    jet2_idx = jet1_idx;
                                    jet2_str = jet1_str;
                                    jet2_score = jet1_score;
                                    jet2_ta = jet1_ta;
                                    jet2_tb = jet1_tb;
                                    jet1_idx = k;
                                    jet1_str = js;
                                    jet1_score = score;
                                    jet1_ta = ja;
                                    jet1_tb = jb;
                                } else if (score < jet2_score) {
                                    jet2_idx = k;
                                    jet2_str = js;
                                    jet2_score = score;
                                    jet2_ta = ja;
                                    jet2_tb = jb;
                                }
                            }
                        }
                    }
                }
            }

            let obj0_pos = vec3<f32>(audio.channel_phases[4].z, audio.channel_phases[4].w, audio.channel_phases[5].x);
            let obj0_e = clamp(audio.channel_phases[5].y, 0.0, 1.0);
            let obj1_pos = vec3<f32>(audio.channel_phases[5].z, audio.channel_phases[5].w, audio.channel_phases[6].x);
            let obj1_e = clamp(audio.channel_phases[6].y, 0.0, 1.0);
            let obj2_pos = vec3<f32>(audio.channel_phases[6].z, audio.channel_phases[6].w, audio.channel_phases[7].x);
            let obj2_e = clamp(audio.channel_phases[7].y, 0.0, 1.0);
            let obj0_col = vec3<f32>(0.96, 0.10, 0.88);
            let obj1_col = vec3<f32>(0.06, 0.95, 0.85);
            let obj2_col = vec3<f32>(1.00, 0.52, 0.06);

            // Pass B: Smooth room standing-wave, nodal shear, and 3D object vapor march
            let dt = (t_end - t_start) / f32(VOL_STEPS);
            let jitter = 0.25 + 0.50 * interleaved_gradient_noise(in.clip_position.xy);

            var wave_accum = vec3<f32>(0.0);
            var wave_trans = 1.0;

            for (var step_i = 0u; step_i < VOL_STEPS; step_i = step_i + 1u) {
                let t = t_start + (f32(step_i) + jitter) * dt;
                let wpos = ro + rd * t;
                let vol = textureSampleLevel(acoustic_vol_tex, acoustic_vol_sampler, world_to_uvw(wpos), 0.0);

                let standing = vol.r;
                let nodal_v = vol.g;
                let p_inst = vol.b * 2.0 - 1.0;
                let band = vol.a;

                let wave_lobe = smoothstep(0.14, 0.58, standing);
                let wave_crest = smoothstep(0.08, 0.45, abs(p_inst));
                let wave_env = (wave_lobe * wave_lobe) * (0.12 + 0.88 * wave_crest);
                let pressure_null = smoothstep_r(0.24, 0.04, standing);
                let nodal_shear = pressure_null * smoothstep(0.22, 0.62, nodal_v);

                let has_objs = obj0_e > 0.04 || obj1_e > 0.04 || obj2_e > 0.04;
                if (wave_env > 0.02 || nodal_shear > 0.02 || has_objs) {
                    let billow = smoke_billow_3d(wpos, audio.smooth_time);

                    var obj_puff = 0.0;
                    var obj_col_accum = vec3<f32>(0.0);
                    if (obj0_e > 0.04) {
                        let d2 = dot(wpos - obj0_pos, wpos - obj0_pos);
                        let p0 = exp(-d2 * 1.4) * obj0_e * (0.35 + 0.65 * billow) * 0.65;
                        obj_puff += p0;
                        obj_col_accum += obj0_col * p0;
                    }
                    if (obj1_e > 0.04) {
                        let d2 = dot(wpos - obj1_pos, wpos - obj1_pos);
                        let p1 = exp(-d2 * 1.4) * obj1_e * (0.35 + 0.65 * billow) * 0.65;
                        obj_puff += p1;
                        obj_col_accum += obj1_col * p1;
                    }
                    if (obj2_e > 0.04) {
                        let d2 = dot(wpos - obj2_pos, wpos - obj2_pos);
                        let p2 = exp(-d2 * 1.4) * obj2_e * (0.35 + 0.65 * billow) * 0.65;
                        obj_puff += p2;
                        obj_col_accum += obj2_col * p2;
                    }

                    let ambient_wave_smoke = wave_env * (0.25 + 0.75 * billow) * 0.34;
                    let shear_wisp = nodal_shear * (0.22 + 0.78 * billow) * 0.18;

                    let near_fade = smoothstep(0.35, 1.75, t);
                    let w_wave = ambient_wave_smoke * 0.036;
                    let w_shear = shear_wisp * 0.022;
                    let w_obj = obj_puff * 0.14;
                    let sigma_a = (w_wave + w_shear + w_obj) * near_fade;

                    if (sigma_a > 0.001) {
                        let bass_w = smoothstep_r(0.36, 0.06, band);
                        let crest_mix = smoothstep(-0.22, 0.22, p_inst);
                        let wave_hi = mix(vec3<f32>(0.18, 0.25, 0.95), vec3<f32>(0.05, 0.85, 0.98), crest_mix);
                        let wave_lo = mix(vec3<f32>(0.90, 0.12, 0.28), vec3<f32>(1.00, 0.48, 0.06), crest_mix);
                        let wave_neon = mix(wave_hi, wave_lo, bass_w);
                        let nodal_neon = vec3<f32>(0.46, 0.14, 0.98);

                        let total_w = w_wave + w_shear + w_obj + 1e-4;
                        let neon_tint = (wave_neon * w_wave + nodal_neon * w_shear + obj_col_accum * 0.14) / total_w;
                        let smoke_base = vec3<f32>(0.68, 0.74, 0.82);
                        let smoke_col = mix(smoke_base, neon_tint, 0.50);
                        let in_scatter = smoke_col * (0.36 + 0.34 * ambient_wave_smoke + 0.70 * obj_puff);

                        let step_alpha = 1.0 - exp(-sigma_a * dt);
                        wave_accum += wave_trans * in_scatter * step_alpha;
                        wave_trans *= (1.0 - step_alpha);
                        if (wave_trans < 0.12) {
                            break;
                        }
                    }
                }
            }

            // Pass A: Stipple-free deterministic 5-step speaker smoke-machine plumes
            let j0_res = integrate_speaker_smoke_plume(ro, rd, jet0_idx, jet0_str, jet0_ta, jet0_tb);
            let j1_res = integrate_speaker_smoke_plume(ro, rd, jet1_idx, jet1_str, jet1_ta, jet1_tb);
            let j2_res = integrate_speaker_smoke_plume(ro, rd, jet2_idx, jet2_str, jet2_ta, jet2_tb);

            let jets_trans = j0_res.w * j1_res.w * j2_res.w;
            let total_transmittance = wave_trans * jets_trans;
            let total_smoke = wave_accum * jets_trans
                + j0_res.xyz
                + j1_res.xyz * j0_res.w
                + j2_res.xyz * (j0_res.w * j1_res.w);

            final_color = final_color * total_transmittance + total_smoke;
        }
    }

    // Subtle distance fog
    let fog = in.ro_fog.w;
    let fog_col = vec3<f32>(0.006, 0.008, 0.015);
    final_color = mix(final_color, fog_col, fog);

    let tonemapped = aces_tonemap(final_color);
    return vec4<f32>(tonemapped, 1.0);
}
