// =====================================================
// RustTracker — Acoustic Wave Chamber Visualizer
// Real-time 3D multi-speaker acoustic wave propagation and
// semi-transparent volumetric interference cloud showing
// standing wave antinodes, destructive nodal lanes, and
// floor cymatic Chladni resonance patterns.
// =====================================================

// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var acoustic_vol_tex: texture_3d<f32>;
@group(1) @binding(1) var acoustic_vol_sampler: sampler;

const ROOM_MIN: vec3<f32> = vec3<f32>(-10.8, -1.5, -8.9);
const ROOM_MAX: vec3<f32> = vec3<f32>(10.8, 4.9, 8.9);
const ROOM_SIZE: vec3<f32> = vec3<f32>(21.6, 6.4, 17.8);

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

const SPEAKER_COLORS: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(0.05, 0.85, 1.00), // 0: FL
    vec3<f32>(1.00, 0.14, 0.72), // 1: FR
    vec3<f32>(1.00, 0.78, 0.14), // 2: C
    vec3<f32>(1.00, 0.30, 0.06), // 3: LFE
    vec3<f32>(0.28, 0.44, 1.00), // 4: SL
    vec3<f32>(0.80, 0.24, 1.00), // 5: SR
    vec3<f32>(0.08, 0.96, 0.76), // 6: RL
    vec3<f32>(0.96, 0.10, 0.48), // 7: RR
    vec3<f32>(0.10, 1.00, 0.38), // 8: TFL
    vec3<f32>(0.78, 1.00, 0.08), // 9: TFR
    vec3<f32>(0.06, 0.90, 0.92), // 10: TRL
    vec3<f32>(1.00, 0.10, 0.78)  // 11: TRR
);

const VOL_STEPS: u32 = 24u;

fn world_to_uvw(p: vec3<f32>) -> vec3<f32> {
    return clamp((p - ROOM_MIN) / ROOM_SIZE, vec3<f32>(0.008), vec3<f32>(0.992));
}

fn get_speaker_level_vis(ch: u32, bass_e: f32, lowmid_e: f32, mid_e: f32, high_e: f32) -> f32 {
    let idx = min(ch, 11u);
    if (audio.num_spatial_channels > 2u) {
        let actual_ch = min(audio.num_spatial_channels, 16u);
        let src_ch = idx % actual_ch;
        return audio.spatial_channels[src_ch >> 2u][src_ch & 3u];
    }
    let ch_l = audio.spatial_channels[0].x;
    let ch_r = select(ch_l, audio.spatial_channels[0].y, audio.num_spatial_channels >= 2u);
    return stereo_upmix_12(
        idx,
        ch_l,
        ch_r,
        audio.channel_phases[0].x,
        audio.channel_phases[0].y,
        bass_e,
        lowmid_e,
        mid_e,
        high_e
    );
}

fn interleaved_gradient_noise(pixel_xy: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel_xy, vec2<f32>(0.06711056, 0.00583715))));
}

fn shade_room_surface(
    hit_pos: vec3<f32>,
    rd: vec3<f32>,
    nrm: vec3<f32>,
    surf_kind: u32, // 0 = floor, 1 = ceiling, 2 = wall
    bass_e: f32,
    mid_e: f32,
    t_wall: f32
) -> vec3<f32> {
    // Sample volumetric field slightly inside the surface along the interior normal
    let sample_pos = hit_pos + nrm * 0.28;
    let vol = textureSampleLevel(acoustic_vol_tex, acoustic_vol_sampler, world_to_uvw(sample_pos), 0.0);
    let standing = vol.r;
    let nodal_v = vol.g;
    let p_inst = vol.b * 2.0 - 1.0;
    let band = vol.a;

    let dist_fade = smoothstep_r(30.0, 6.0, t_wall);

    if (surf_kind == 0u) {
        // Dark matte studio floor with crisp perspective meter grid + live Chladni/Cymatic heatmap
        let grid_uv = fract(hit_pos.xz * 0.5 + 0.5) - 0.5;
        let line_d = min(abs(grid_uv.x), abs(grid_uv.y));
        let minor_grid = smoothstep_r(0.032, 0.008, line_d) * 0.14;

        let major_uv = fract(hit_pos.xz * 0.125 + 0.5) - 0.5;
        let major_d = min(abs(major_uv.x), abs(major_uv.y));
        let major_grid = smoothstep_r(0.022, 0.006, major_d) * 0.25;

        // Sweet-spot listening ring at center
        let r_center = length(hit_pos.xz);
        let sweet_ring = smoothstep_r(0.16, 0.02, abs(r_center - 1.25)) * (0.28 + 0.45 * mid_e);

        var col = vec3<f32>(0.015, 0.019, 0.030);
        col += vec3<f32>(0.04, 0.08, 0.14) * minor_grid;
        col += vec3<f32>(0.06, 0.14, 0.22) * major_grid;
        col += vec3<f32>(0.18, 0.72, 0.95) * sweet_ring;

        // Live Floor Cymatic Standing-Wave & Destructive Nodal Contours
        let antinode_raw = smoothstep(0.18, 0.68, standing);
        let antinode_floor = antinode_raw * antinode_raw;
        let crest_ripple = 0.50 + 0.50 * abs(p_inst);
        let bass_floor = smoothstep_r(0.36, 0.06, band);
        let cymatic_col = mix(vec3<f32>(0.04, 0.62, 0.96), vec3<f32>(0.96, 0.34, 0.08), bass_floor);

        // Destructive nodal cancellation ridge on the floor where pressure nulls intersect velocity peak
        let null_zone = smoothstep_r(0.26, 0.04, standing);
        let nodal_line = null_zone * smoothstep(0.22, 0.64, nodal_v);

        col += cymatic_col * antinode_floor * crest_ripple * 0.38;
        col += vec3<f32>(0.44, 0.16, 0.98) * nodal_line * 0.32;

        // Subtle Fresnel studio sheen toward horizon
        let fresnel = pow(1.0 - max(dot(-rd, nrm), 0.0), 3.0);
        col += vec3<f32>(0.02, 0.04, 0.08) * fresnel;
        return col * mix(0.45, 1.0, dist_fade);
    }

    if (surf_kind == 1u) {
        // Acoustic baffle ceiling with subtle recessed grid and height-channel illumination
        let baffle_uv = fract(hit_pos.xz * 0.35 + 0.5) - 0.5;
        let slat = smoothstep_r(0.06, 0.01, min(abs(baffle_uv.x), abs(baffle_uv.y)));
        var col = vec3<f32>(0.012, 0.015, 0.024) + vec3<f32>(0.015, 0.024, 0.040) * slat;
        let ceil_glow = smoothstep(0.18, 0.75, standing) * (0.4 + 0.6 * abs(p_inst));
        col += mix(vec3<f32>(0.10, 0.52, 0.90), vec3<f32>(0.55, 0.95, 0.75), band) * ceil_glow * 0.32;
        return col * mix(0.4, 1.0, dist_fade);
    }

    // Studio acoustic diffuser walls + neon perimeter plinths + boundary standing-wave eigenmodes
    let wall_x_coord = select(hit_pos.x, hit_pos.z, abs(nrm.x) > 0.5);
    let diffuser_slats = 0.5 + 0.5 * cos(wall_x_coord * 7.85398);
    let panel_seam = smoothstep_r(0.08, 0.01, abs(fract(hit_pos.y * 0.35) - 0.5));
    var col = vec3<f32>(0.018, 0.022, 0.034) * (0.78 + 0.22 * diffuser_slats);
    col += vec3<f32>(0.02, 0.04, 0.07) * panel_seam;

    // Perimeter glowing baseboard & crown accent strips
    let y_floor_dist = abs(hit_pos.y - ROOM_MIN.y);
    let y_ceil_dist = abs(hit_pos.y - ROOM_MAX.y);
    let trim = smoothstep_r(0.11, 0.015, y_floor_dist) * (0.35 + 0.85 * bass_e)
             + smoothstep_r(0.09, 0.015, y_ceil_dist) * (0.25 + 0.65 * mid_e);
    col += vec3<f32>(0.06, 0.52, 0.98) * trim;

    // Live acoustic pressure & nodal modes painted subtly onto room boundary walls
    let wall_mode = smoothstep(0.16, 0.78, standing) * (0.55 + 0.45 * abs(p_inst));
    let wall_null = smoothstep_r(0.26, 0.04, standing) * smoothstep(0.24, 0.66, nodal_v);
    col += mix(vec3<f32>(0.08, 0.48, 0.88), vec3<f32>(1.00, 0.68, 0.25), smoothstep(0.5, 0.9, standing)) * wall_mode * 0.34;
    col += vec3<f32>(0.42, 0.14, 0.92) * wall_null * 0.22;

    return col * mix(0.45, 1.0, dist_fade);
}

fn eval_emitter_glow(
    ro: vec3<f32>,
    rd: vec3<f32>,
    t_wall: f32,
    center: vec3<f32>,
    col: vec3<f32>,
    energy: f32,
    core_radius: f32,
    halo_scale: f32
) -> vec3<f32> {
    let oc = center - ro;
    let t_proj = dot(oc, rd);
    if (t_proj <= 0.15 || t_proj >= t_wall) {
        return vec3<f32>(0.0);
    }
    let closest = ro + rd * t_proj;
    let d2 = dot(closest - center, closest - center);
    let dist_atten = 1.0 / (1.0 + 0.06 * t_proj * t_proj);
    let halo = exp(-d2 * halo_scale) * (0.05 + 0.18 * energy) * dist_atten;
    let core = smoothstep_r(core_radius * 1.30, core_radius * 0.35, sqrt(d2)) * (0.22 + 0.42 * energy);
    return col * (halo + core);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // NDC to camera ray
    let aspect = max(audio.aspect_ratio, 0.25);
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, -(in.uv.y * 2.0 - 1.0));
    let screen_uv = vec2<f32>(ndc.x * aspect, ndc.y);

    // Smooth camera orbit strictly inside ROOM_MIN..ROOM_MAX with <= 3-decimal smooth_time factors
    let cam_ang = audio.smooth_time * 0.10;
    let cam_y = 2.55 + sin(audio.smooth_time * 0.06) * 0.72;
    let ro = vec3<f32>(
        sin(cam_ang) * 7.8,
        cam_y,
        -cos(cam_ang) * 6.4
    );
    let cam_target = vec3<f32>(0.0, 0.85, 0.4 + sin(audio.smooth_time * 0.15) * 0.25);

    let forward = normalize(cam_target - ro);
    let right = normalize(cross(forward, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, forward);
    let rd = normalize(forward * 1.48 + right * screen_uv.x + up * screen_uv.y);

    // Analytical ray-AABB exit intersection (ro is always strictly inside ROOM_MIN..ROOM_MAX)
    let inv_rd = vec3<f32>(
        select(1.0 / rd.x, 1e6, abs(rd.x) < 1e-6),
        select(1.0 / rd.y, 1e6, abs(rd.y) < 1e-6),
        select(1.0 / rd.z, 1e6, abs(rd.z) < 1e-6)
    );
    let t0 = (ROOM_MIN - ro) * inv_rd;
    let t1 = (ROOM_MAX - ro) * inv_rd;
    let tmax_v = max(t0, t1);
    let t_wall = max(min(min(tmax_v.x, tmax_v.y), tmax_v.z), 0.25);
    let hit_pos = ro + rd * t_wall;

    var surf_nrm = vec3<f32>(0.0, 0.0, 1.0);
    var surf_kind = 2u;
    if (tmax_v.y <= tmax_v.x && tmax_v.y <= tmax_v.z) {
        if (rd.y < 0.0) {
            surf_nrm = vec3<f32>(0.0, 1.0, 0.0);
            surf_kind = 0u; // floor
        } else {
            surf_nrm = vec3<f32>(0.0, -1.0, 0.0);
            surf_kind = 1u; // ceiling
        }
    } else if (tmax_v.x <= tmax_v.z) {
        surf_nrm = vec3<f32>(select(1.0, -1.0, rd.x > 0.0), 0.0, 0.0);
    } else {
        surf_nrm = vec3<f32>(0.0, 0.0, select(1.0, -1.0, rd.z > 0.0));
    }

    let bass_e = clamp((audio.spectrum[0].y + audio.spectrum[0].z + audio.spectrum[0].w + audio.spectrum[1].x) / 260.0, 0.0, 1.0);
    let lowmid_e = clamp((audio.spectrum[2].x + audio.spectrum[3].x + audio.spectrum[4].x) / 195.0, 0.0, 1.0);
    let mid_e = clamp((audio.spectrum[6].x + audio.spectrum[10].x + audio.spectrum[14].x) / 180.0, 0.0, 1.0);
    let high_e = clamp((audio.spectrum[20].x + audio.spectrum[34].x + audio.spectrum[50].x) / 165.0, 0.0, 1.0);

    var bg_col = shade_room_surface(hit_pos, rd, surf_nrm, surf_kind, bass_e, mid_e, t_wall);

    // Analytical glow beacons for all 12 bed speakers + 3 orbiting 3D object stems
    var emitter_glow = vec3<f32>(0.0);
    for (var ch = 0u; ch < 12u; ch = ch + 1u) {
        let lvl = clamp(get_speaker_level_vis(ch, bass_e, lowmid_e, mid_e, high_e), 0.0, 1.2);
        let rad = select(0.13, 0.20, ch == 3u);
        let halo_k = select(6.2, 4.0, ch == 3u);
        emitter_glow += eval_emitter_glow(ro, rd, t_wall, SPEAKER_POS[ch], SPEAKER_COLORS[ch], lvl, rad, halo_k);
    }

    let obj0_pos = vec3<f32>(audio.channel_phases[4].z, audio.channel_phases[4].w, audio.channel_phases[5].x);
    let obj0_e = clamp(audio.channel_phases[5].y, 0.0, 1.2);
    if (obj0_e > 0.01) {
        emitter_glow += eval_emitter_glow(ro, rd, t_wall, obj0_pos, vec3<f32>(0.96, 0.10, 0.88), obj0_e, 0.15, 5.5);
    }
    let obj1_pos = vec3<f32>(audio.channel_phases[5].z, audio.channel_phases[5].w, audio.channel_phases[6].x);
    let obj1_e = clamp(audio.channel_phases[6].y, 0.0, 1.2);
    if (obj1_e > 0.01) {
        emitter_glow += eval_emitter_glow(ro, rd, t_wall, obj1_pos, vec3<f32>(0.06, 0.95, 0.85), obj1_e, 0.15, 5.5);
    }
    let obj2_pos = vec3<f32>(audio.channel_phases[6].z, audio.channel_phases[6].w, audio.channel_phases[7].x);
    let obj2_e = clamp(audio.channel_phases[7].y, 0.0, 1.2);
    if (obj2_e > 0.01) {
        emitter_glow += eval_emitter_glow(ro, rd, t_wall, obj2_pos, vec3<f32>(1.00, 0.52, 0.06), obj2_e, 0.15, 5.5);
    }
    emitter_glow = min(emitter_glow, vec3<f32>(0.55));

    // 24-step blue-noise-jittered volumetric raymarch across acoustic pressure/nodal volume
    let t_start = 0.30;
    let t_end = min(t_wall, 19.5);
    let march_span = max(t_end - t_start, 0.05);
    let dt = march_span / f32(VOL_STEPS);
    let jitter = interleaved_gradient_noise(in.clip_position.xy);

    var accum_col = vec3<f32>(0.0);
    var transmittance = 1.0;

    for (var step_i = 0u; step_i < VOL_STEPS; step_i = step_i + 1u) {
        let t = t_start + (f32(step_i) + jitter) * dt;
        let wpos = ro + rd * t;
        let uvw = world_to_uvw(wpos);
        let vol = textureSampleLevel(acoustic_vol_tex, acoustic_vol_sampler, uvw, 0.0);

        let standing = vol.r;
        let nodal_v = vol.g;
        let p_inst = vol.b * 2.0 - 1.0;
        let band = vol.a;

        // Carve transparent zero-crossings and nodal corridors so standing-wave shells remain crisp
        let near_fade = smoothstep(0.45, 2.20, t);
        let lobe = smoothstep(0.14, 0.56, standing);
        let wave_crest = abs(p_inst);
        let shell = smoothstep(0.12, 0.48, wave_crest);
        let density_core = (lobe * lobe) * (0.08 + 0.92 * shell) * near_fade;

        // High particle-velocity gradient along zero-pressure cancellation planes renders thin violet contours
        let pressure_null = smoothstep_r(0.24, 0.04, standing);
        let cancel_contour = pressure_null * smoothstep(0.20, 0.60, nodal_v) * near_fade;

        let sigma_a = density_core * 0.105 + cancel_contour * 0.055;
        if (sigma_a > 0.001) {
            let bass_w = smoothstep_r(0.36, 0.06, band);
            let crest_hi = vec3<f32>(0.03, 0.86, 0.98);
            let trough_hi = vec3<f32>(0.20, 0.10, 1.00);
            let crest_lo = vec3<f32>(1.00, 0.46, 0.04);
            let trough_lo = vec3<f32>(0.92, 0.05, 0.24);

            let crest_col = mix(crest_hi, crest_lo, bass_w);
            let trough_col = mix(trough_hi, trough_lo, bass_w);
            let lobe_col = mix(trough_col, crest_col, smoothstep(-0.22, 0.22, p_inst));
            let contour_col = vec3<f32>(0.46, 0.08, 1.00);

            let core_w = density_core / max(density_core + cancel_contour * 0.50, 1e-4);
            let sample_col = mix(contour_col, lobe_col, core_w);
            let emissive = sample_col * (0.55 + 0.85 * density_core + 0.45 * cancel_contour);

            let step_alpha = 1.0 - exp(-sigma_a * dt);
            accum_col += transmittance * emissive * step_alpha;
            transmittance *= (1.0 - step_alpha);
            if (transmittance < 0.05) {
                break;
            }
        }
    }

    // Subtle distance atmospheric haze and vignette inside chamber
    let center_vignette = smoothstep_r(1.55, 0.45, length(ndc));
    let hdr_col = (bg_col * transmittance + emitter_glow * (0.72 * transmittance) + accum_col)
                * mix(0.84, 1.04, center_vignette);

    return vec4<f32>(aces_tonemap(hdr_col), 1.0);
}
