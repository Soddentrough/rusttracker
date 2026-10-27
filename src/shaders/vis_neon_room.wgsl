// =====================================================
// RustTracker — 3D Interactive Spatial Listening Room
// Hardware-rasterized 3D acoustic room with multi-channel
// speaker towers, reactive woofer excursion, dynamic
// point-light casting, and acoustic wavefront rings.
// =====================================================

// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(2) @binding(0) var<uniform> camera: CameraUniforms;

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
    @location(6) ch_levels_1_xy: vec2<f32>,
}

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
    vec3<f32>( 0.0, -0.85,  6.8), // C (2)
    vec3<f32>( 0.0, -1.05,  5.2), // Sub (3)
    vec3<f32>(-6.2,  0.50,  0.5), // SL (4)
    vec3<f32>( 6.2,  0.50,  0.5), // SR (5)
    vec3<f32>(-4.2,  0.10, -4.8), // RL (6)
    vec3<f32>( 4.2,  0.10, -4.8), // RR (7)
    vec3<f32>(-3.2,  4.32,  3.5), // TFL (8)
    vec3<f32>( 3.2,  4.32,  3.5), // TFR (9)
    vec3<f32>(-3.2,  4.32, -2.5), // TRL (10)
    vec3<f32>( 3.2,  4.32, -2.5)  // TRR (11)
);

fn get_spatial_color(ch: u32) -> vec3<f32> {
    return SPEAKER_COLORS[min(ch, 11u)];
}

fn get_channel_level_bands(ch: u32, bass_e: f32, lowmid_e: f32, mid_e: f32, high_e: f32) -> f32 {
    let idx = min(ch, 11u);

    // 4-Channel Tracker Module (MOD) or 4.0 Quad: map the 4 discrete tracks directly to the
    // 4 primary corner/surround speakers (FL=T1, FR=T2, SL/RL=T3, SR/RR=T4), Center=Lead Mix, Sub=Bass!
    if (audio.num_spatial_channels == 4u) {
        let t0 = audio.spatial_channels[0].x;
        let t1 = audio.spatial_channels[0].y;
        let t2 = audio.spatial_channels[0].z;
        let t3 = audio.spatial_channels[0].w;
        switch idx {
            case 0u: { return t0; } // Front Left = Track 1
            case 1u: { return t1; } // Front Right = Track 2
            case 2u: { return clamp((t0 + t1 + t2 + t3) * 0.25 + mid_e * 0.30, 0.0, 1.0); } // Center Stage
            case 3u: { return clamp(bass_e * 0.85 + max(max(t0, t1), max(t2, t3)) * 0.30, 0.0, 1.0); } // Subwoofer
            case 4u: { return t2; } // Surround Left = Track 3
            case 5u: { return t3; } // Surround Right = Track 4
            case 6u: { return clamp(t2 * 0.85 + t0 * 0.25, 0.0, 1.0); } // Rear Left
            case 7u: { return clamp(t3 * 0.85 + t1 * 0.25, 0.0, 1.0); } // Rear Right
            case 8u: { return t0; } // Ceiling Front Left = Track 1
            case 9u: { return t1; } // Ceiling Front Right = Track 2
            case 10u: { return t2; } // Ceiling Rear Left = Track 3
            default: { return t3; } // Ceiling Rear Right = Track 4
        }
    }

    if (audio.num_spatial_channels > 2u) {
        // 5.1 Surround (6 channels): map 0..5 directly, mirror Ls/Rs to RL/RR (6..7) and heights (8..11)
        if (audio.num_spatial_channels == 6u && audio.num_channels == 6u) {
            if (idx < 6u) {
                return audio.spatial_channels[idx / 4u][idx % 4u];
            } else if (idx == 6u) {
                return audio.spatial_channels[1].x; // Mirror Ls to RL
            } else if (idx == 7u) {
                return audio.spatial_channels[1].y; // Mirror Rs to RR
            } else {
                let base_idx = idx - 8u;
                return clamp(audio.spatial_channels[0][base_idx] * 0.65 + high_e * 0.40, 0.0, 1.0);
            }
        }
        // Multi-track tracker (6..16 channels) or 7.1 / 7.1.4 surround
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
    // Cone vertices encode base material (2.0, 3.0, 4.0) + 0.18..0.25 toward inner dustcap apex
    let is_cone = raw_mat >= 1.8 && raw_mat <= 4.4;
    let mat_id = select(raw_mat, floor(raw_mat + 0.05), is_cone);
    let cone_center_weight = select(0.0, clamp((raw_mat - mat_id) * 4.0, 0.0, 1.0), is_cone);
    
    let bass_e = clamp((audio.spectrum[0].x + audio.spectrum[1].x + audio.spectrum[2].x) / 195.0, 0.0, 1.0);
    let lowmid_e = clamp((audio.spectrum[3].x + audio.spectrum[5].x) / 130.0, 0.0, 1.0);
    let mid_e = clamp((audio.spectrum[10].x + audio.spectrum[16].x) / 120.0, 0.0, 1.0);
    let high_e = clamp((audio.spectrum[32].x + audio.spectrum[64].x) / 110.0, 0.0, 1.0);

    let ch_energy_raw = get_channel_level_bands(ch_idx, bass_e, lowmid_e, mid_e, high_e);
    let ch_energy = clamp(ch_energy_raw, 0.0, 1.2);
    let bass_pulse = clamp((audio.spectrum[0].x + audio.spectrum[1].x) / 140.0, 0.0, 1.2);
    
    var pos = in.position;
    let is_speaker = (mat_id > 1.1 && mat_id < 4.5) || (mat_id > 5.1 && mat_id < 5.3);

    // 1. Whole-Speaker Physical Movement (Cabinet Pulse, Radial Expansion & Beat Bounce)
    if (is_speaker) {
        let sp_center = SPEAKER_POS[ch_idx];
        let rel = pos - sp_center;
        let vib = 0.65 + 0.35 * sin(audio.smooth_time * 28.0 + f32(ch_idx) * 1.3);
        let scale_xz = 1.0 + ch_energy * 0.20 * vib;
        let scale_y  = 1.0 + ch_energy * 0.14 * vib;
        let bounce_dir = select(1.0, -1.0, ch_idx >= 8u);
        let hop_y = bounce_dir * ch_energy * 0.15 * abs(sin(audio.smooth_time * 16.0 + f32(ch_idx) * 1.1));
        pos = sp_center + vec3<f32>(rel.x * scale_xz, rel.y * scale_y + hop_y, rel.z * scale_xz);
    }

    // 2. Aggressive Woofer / Midrange / Tweeter Cone Piston Excursion along Normal
    var displacement = 0.0;
    if (is_cone) {
        let pump_osc = 0.55 + 0.45 * sin(audio.smooth_time * 44.0 + f32(ch_idx) * 1.5);
        let radial_excursion = 0.14 + 0.86 * cone_center_weight;
        if (ch_idx == 3u) {
            // LFE dual 15" subwoofer has massive physical excursion
            displacement = (ch_energy * 0.90 + bass_pulse * 0.65) * 0.46 * pump_osc * radial_excursion;
        } else if (mat_id < 2.5) {
            // Bass woofers
            displacement = (ch_energy * 0.95 + bass_pulse * 0.35) * 0.38 * pump_osc * radial_excursion;
        } else if (mat_id < 3.5) {
            // Midrange cones
            displacement = ch_energy * 0.30 * pump_osc * radial_excursion;
        } else {
            // Tweeter domes
            displacement = ch_energy * 0.18 * pump_osc * radial_excursion;
        }
    }
    
    let displaced_pos = pos + in.normal * displacement;

    let t_cam = audio.smooth_time * 0.18;
    let cam_x = sin(t_cam) * 9.8 * 0.55;
    let cam_z = -cos(t_cam) * 9.8 * 0.6 - 2.8;
    let ro_v = vec3<f32>(cam_x, 2.4 + sin(t_cam * 0.7) * 0.6, cam_z);

    let N_v = normalize(in.normal);
    let V_vec = ro_v - displaced_pos;

    // Back-face cull in vertex shader: all room walls, boxes, and speaker cones have exact outward normals
    if (dot(N_v, V_vec) < -0.02) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.pos_mat = vec4<f32>(0.0);
        out.norm_ch = vec4<f32>(0.0);
        out.fill_diff_cw = vec4<f32>(0.0);
        out.fill_spec_che = vec4<f32>(0.0);
        out.ro_fog = vec4<f32>(0.0);
        out.ch_levels_0 = vec4<f32>(0.0);
        out.ch_levels_1_xy = vec2<f32>(0.0);
        return out;
    }

    let dist_to_cam = length(V_vec);
    let V_v = V_vec / max(dist_to_cam, 1e-6);
    let NdotV_v = dot(N_v, V_v);
    let use_spec64 = mat_id < 1.0;

    // Evaluate all 12 spatial speakers (diffuse + specular) per-vertex across the subdivided room mesh
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
    out.ch_levels_1_xy = vec2<f32>(all_levels[4], all_levels[5]);
    
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
    
    var albedo = vec3<f32>(0.03, 0.03, 0.04);
    var emission = vec3<f32>(0.0);
    var roughness = 0.6;
    
    if (mat_id < 0.2) {
        // === Material 0.0: Studio Floor Grid & Channel-Reactive Acoustic Pressure Rings ===
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
            in.ch_levels_1_xy.x, in.ch_levels_1_xy.y
        );
        // Localized acoustic pressure rings around the 6 main stage/surround speakers
        for (var i = 0u; i < 6u; i = i + 1u) {
            let s_energy = floor_levels[i];
            if (s_energy > 0.04) {
                let s_pos = SPEAKER_POS[i];
                let diff_xz = P.xz - s_pos.xz;
                let d_sp2 = dot(diff_xz, diff_xz);
                if (d_sp2 < 30.25) {
                    let s_col = SPEAKER_COLORS[i];
                    let d_sp = sqrt(d_sp2);
                    let ring_phase = fract(d_sp * 0.55 - audio.smooth_time * (1.2 + s_energy * 2.5));
                    let wave_ring = smoothstep_r(0.18, 0.0, abs(ring_phase - 0.5)) * smoothstep_r(5.5, 0.4, d_sp);
                    let base_pool = (1.0 / (1.0 + d_sp2 * 0.65)) * (0.4 + 0.6 * grid_line);
                    emission += s_col * (wave_ring * 0.65 + base_pool * 0.55) * (s_energy * s_energy);
                }
            }
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
        } else if (ch_idx == 2u || ch_idx == 3u) {
            h_u = clamp((P.y - (sp_pos.y - 0.45)) / 0.90, 0.0, 1.0);
        } else if (ch_idx >= 8u) {
            h_u = clamp(1.0 - (P.y - 4.08) / 0.46, 0.0, 1.0);
        }

        // 14-segment LED level meter climbing up the speaker body with ch_energy
        let seg_f = fract(h_u * 14.0);
        let led_seg = smoothstep(0.14, 0.24, seg_f) * smoothstep_r(0.88, 0.78, seg_f);
        let is_lit = smoothstep_r(ch_energy + 0.05, ch_energy - 0.02, h_u) * smoothstep(0.02, 0.08, ch_energy);
        // Shift color from signature channel hue at base to hot white-gold at peak
        let peak_tint = vec3<f32>(1.0, 0.92, 0.60);
        let dynamic_col = mix(ch_col, peak_tint, smoothstep(0.55, 0.98, h_u) * ch_energy);

        let ch_e15 = ch_energy * sqrt(max(ch_energy, 0.0));
        albedo = mix(vec3<f32>(0.020, 0.022, 0.028), dynamic_col * 0.25, ch_energy * 0.7);
        emission = dynamic_col * (led_seg * is_lit * 1.45 + ch_e15 * 0.35);
        roughness = 0.25;
        
    } else if (mat_id < 2.5) {
        // === Material 2.0: Active Bass Woofer Diaphragms (Ribbed Cone + Glowing Dustcap + Travelling Wave) ===
        let cw = in.fill_diff_cw.w;
        let is_surround = smoothstep_r(0.16, 0.04, cw);
        let is_dustcap = smoothstep(0.65, 0.75, cw);
        let cone_ribs = 0.80 + 0.20 * sin(cw * 38.0 - audio.smooth_time * 22.0 * step(0.05, ch_energy));
        let cone_base = vec3<f32>(0.030, 0.032, 0.042) * cone_ribs;
        let hot_core = vec3<f32>(1.0, 0.95, 0.85);
        let active_col = mix(ch_col, hot_core, is_dustcap * clamp(ch_energy * 0.85, 0.0, 0.75));
        let glow_intensity = ch_energy * 3.8 * (1.0 - is_surround * 0.5) * (0.65 + 0.75 * is_dustcap) * cone_ribs;
        emission = active_col * (0.04 + glow_intensity);
        albedo = mix(cone_base, active_col * 0.45, clamp(ch_energy, 0.0, 1.0));
        roughness = mix(0.20, 0.06, is_dustcap);
        
    } else if (mat_id < 3.5) {
        // === Material 3.0: Active Midrange Diaphragms (Ribbed Kevlar Cone + Phase Plug) ===
        let cw = in.fill_diff_cw.w;
        let is_plug = smoothstep(0.68, 0.76, cw);
        let cone_ribs = 0.82 + 0.18 * sin(cw * 30.0 - audio.smooth_time * 26.0 * step(0.05, ch_energy));
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
        // Dim standby ember when silent, blazing neon + white-hot core when active!
        let hot_boost = vec3<f32>(1.0, 0.95, 0.88) * max(ch_energy - 0.55, 0.0) * 2.8;
        emission = ch_col * (0.08 + ch_energy * 4.2) + hot_boost;
        albedo = ch_col;
        roughness = 0.08;
        
    } else if (mat_id < 6.5) {
        // === Material 6.0: Acoustic Diffuser Slats (Rich Walnut Texture, Triplanar) ===
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
        // Shows a glowing 32-column master console LED spectrum analyzer behind the glass!
        let glass_base = vec3<f32>(0.008, 0.020, 0.038);
        let V_win = normalize(ro - P);
        let nv_c = 1.0 - max(dot(N, V_win), 0.0);
        let fresnel = nv_c * nv_c * nv_c;
        let glass_reflection = vec3<f32>(0.08, 0.24, 0.48) * fresnel;

        // Window spans X in [-4.5, 4.5], Y in [0.4, 3.4]
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
    
    // Ambient baseline lighting
    let ambient_col = vec3<f32>(0.015, 0.018, 0.028);
    var final_color = albedo * (ambient_col + point_light_diffuse) + point_light_specular * (1.0 - roughness) + emission;
    
    // Subtle distance fog
    let fog = in.ro_fog.w;
    let fog_col = vec3<f32>(0.006, 0.008, 0.015);
    final_color = mix(final_color, fog_col, fog);
    
    // Tone mapping
    let tonemapped = aces_tonemap(final_color);
    return vec4<f32>(tonemapped, 1.0);
}
