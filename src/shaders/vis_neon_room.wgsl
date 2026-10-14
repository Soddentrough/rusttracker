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
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) channel_id: f32,
    @location(4) material_id: f32,
    @location(5) cone_center_weight: f32,
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

// Speaker source positions in room (7.1.4 Dolby Atmos spatial layout)
const SPEAKER_POS: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(-3.8, 0.2, 5.5),   // FL (0)
    vec3<f32>( 3.8, 0.2, 5.5),   // FR (1)
    vec3<f32>( 0.0, -0.6, 6.8),  // C (2)
    vec3<f32>( 0.0, -1.0, 5.2),  // Sub (3)
    vec3<f32>(-6.2, 0.5, 0.5),   // SL (4)
    vec3<f32>( 6.2, 0.5, 0.5),   // SR (5)
    vec3<f32>(-4.2, 0.2, -4.5),  // RL (6)
    vec3<f32>( 4.2, 0.2, -4.5),  // RR (7)
    vec3<f32>(-3.2, 4.05, 3.5),  // TFL (8)
    vec3<f32>( 3.2, 4.05, 3.5),  // TFR (9)
    vec3<f32>(-3.2, 4.05, -2.5), // TRL (10)
    vec3<f32>( 3.2, 4.05, -2.5)  // TRR (11)
);

fn get_spatial_color(ch: u32) -> vec3<f32> {
    return SPEAKER_COLORS[min(ch, 11u)];
}

fn get_channel_level(ch: u32) -> f32 {
    let idx = min(ch, 11u);
    if (audio.num_spatial_channels > 2u) {
        let actual_ch = min(audio.num_spatial_channels, 16u);
        let src_ch = idx % actual_ch;
        return audio.spatial_channels[src_ch / 4u][src_ch % 4u];
    }
    let ch_l = audio.spatial_channels[0].x;
    let ch_r = select(ch_l, audio.spatial_channels[0].y, audio.num_spatial_channels >= 2u);
    let phase_l = audio.channel_phases[0].x;
    let phase_r = audio.channel_phases[0].y;
    let bass_e = clamp((audio.spectrum[0].x + audio.spectrum[1].x) / 140.0, 0.0, 1.0);
    let lowmid_e = clamp((audio.spectrum[3].x + audio.spectrum[5].x) / 130.0, 0.0, 1.0);
    let mid_e = clamp((audio.spectrum[10].x + audio.spectrum[16].x) / 120.0, 0.0, 1.0);
    let high_e = clamp((audio.spectrum[32].x + audio.spectrum[64].x) / 110.0, 0.0, 1.0);
    return stereo_upmix_12(idx, ch_l, ch_r, phase_l, phase_r, bass_e, lowmid_e, mid_e, high_e);
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    let ch_idx = u32(round(in.tex_coords.x));
    let raw_mat = in.tex_coords.y;
    // Cone vertices encode base material (2.0, 3.0, 4.0) + 0.25 at the inner apex
    let is_cone = raw_mat >= 1.8 && raw_mat <= 4.4;
    let mat_id = select(raw_mat, floor(raw_mat + 0.05), is_cone);
    let cone_center_weight = select(0.0, clamp((raw_mat - mat_id) * 4.0, 0.0, 1.0), is_cone);
    
    let ch_energy = get_channel_level(ch_idx);
    // Normalize audio.spectrum[1].x from 0..100 scale to 0..1.2 so cones don't fly across the room
    let bass_pulse = clamp((audio.spectrum[0].x + audio.spectrum[1].x) / 140.0, 0.0, 1.2);
    
    // Woofer / mid cone physical displacement along normal:
    // Outer surround rim (cone_center_weight == 0.0) stays anchored to the cabinet baffle,
    // while the inner cone and dustcap (cone_center_weight == 1.0) pump dynamically!
    var displacement = 0.0;
    if (mat_id >= 1.8 && mat_id <= 3.5) {
        let pump_osc = 0.60 + 0.40 * sin(audio.smooth_time * 36.0 + f32(ch_idx) * 1.2);
        let radial_excursion = 0.10 + 0.90 * cone_center_weight;
        let drive = max(ch_energy, 0.0);
        if (ch_idx == 3u) {
            // LFE subwoofer has deep physical excursion
            displacement = (drive * 0.85 + bass_pulse * 0.65) * 0.42 * pump_osc * radial_excursion;
        } else if (mat_id < 2.5) {
            // Bass woofers
            displacement = (drive * 0.90 + bass_pulse * 0.45) * 0.32 * pump_osc * radial_excursion;
        } else {
            // Midrange cones
            displacement = (drive * 0.80) * 0.22 * pump_osc * radial_excursion;
        }
    }
    
    let displaced_pos = in.position + in.normal * displacement;
    out.world_pos = displaced_pos;
    out.world_normal = in.normal;
    out.uv = in.tex_coords;
    out.channel_id = in.tex_coords.x;
    out.material_id = mat_id;
    out.cone_center_weight = cone_center_weight;
    
    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(displaced_pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let P = in.world_pos;
    let N = normalize(in.world_normal);
    let mat_id = in.material_id;
    let ch_idx = min(u32(round(in.channel_id)), 11u);
    let ch_col = get_spatial_color(ch_idx);
    
    // Pre-fetch all 12 spatial speaker levels once per fragment
    var ch_levels: array<f32, 12>;
    for (var k = 0u; k < 12u; k = k + 1u) {
        ch_levels[k] = get_channel_level(k);
    }
    let ch_energy = ch_levels[ch_idx];
    let bass_norm = clamp((audio.spectrum[0].x + audio.spectrum[1].x) / 140.0, 0.0, 1.2);
    let treble_norm = clamp((audio.spectrum[55].x + audio.spectrum[65].x) / 130.0, 0.0, 1.2);

    // Camera position for specular / fresnel
    let t_cam = audio.smooth_time * 0.18;
    let cam_x = sin(t_cam) * 9.8 * 0.55;
    let cam_z = -cos(t_cam) * 9.8 * 0.6 - 2.8;
    let ro = vec3<f32>(cam_x, 2.4 + sin(t_cam * 0.7) * 0.6, cam_z);
    let V = normalize(ro - P);
    
    // Dynamic multi-point lighting accumulated from all 12 active speakers
    var point_light_diffuse = vec3<f32>(0.0);
    var point_light_specular = vec3<f32>(0.0);
    let spec_power = select(24.0, 64.0, mat_id < 1.0);
    
    for (var i = 0u; i < 12u; i = i + 1u) {
        let s_pos = SPEAKER_POS[i];
        let s_col = SPEAKER_COLORS[i];
        let s_energy = ch_levels[i];
        let L_vec = s_pos - P;
        let dist_sq = dot(L_vec, L_vec);
        let L_dir = L_vec * inverseSqrt(max(dist_sq, 1e-6));
        
        let atten = (s_energy * 2.8 + 0.15) / (dist_sq * 0.45 + 1.2);
        let NdotL = max(dot(N, L_dir), 0.0);
        point_light_diffuse += s_col * NdotL * atten;
        
        let H = normalize(L_dir + V);
        let NdotH = max(dot(N, H), 0.0);
        point_light_specular += s_col * pow(NdotH, spec_power) * atten;
    }
    
    var albedo = vec3<f32>(0.03, 0.03, 0.04);
    var emission = vec3<f32>(0.0);
    var roughness = 0.6;
    
    if (mat_id < 0.2) {
        // === Material 0.0: Floor Stage Grid & Travelling Signal Traces ===
        let grid_size = 1.0;
        let gx = abs(fract(P.x / grid_size) - 0.5);
        let gz = abs(fract(P.z / grid_size) - 0.5);
        let line_w = 0.03;
        let grid_line = smoothstep_r(line_w, 0.0, min(gx, gz));
        
        // Travelling neon light packets racing along the floor grid lines from stage to surrounds
        let packet_z = smoothstep(0.82, 0.98, sin(P.z * 1.4 - audio.smooth_time * 7.5));
        let packet_x = smoothstep(0.82, 0.98, cos(abs(P.x) * 1.4 - audio.smooth_time * 6.5));
        let trace_pulse = grid_line * max(packet_z, packet_x) * (0.25 + bass_norm * 0.95);

        let floor_base = vec3<f32>(0.015, 0.015, 0.025);
        let floor_grid_col = vec3<f32>(0.08, 0.18, 0.35);
        albedo = mix(floor_base, floor_grid_col, grid_line * 0.7);
        roughness = 0.22;
        emission += mix(vec3<f32>(0.0, 0.85, 1.0), vec3<f32>(1.0, 0.15, 0.75), clamp(abs(P.x) / 9.0, 0.0, 1.0)) * trace_pulse * 0.55;
        
        // Acoustic wavefront rings expanding across the floor from active stage & surround speakers
        for (var i = 0u; i < 8u; i = i + 1u) {
            let s_pos = SPEAKER_POS[i];
            let s_col = SPEAKER_COLORS[i];
            let s_energy = ch_levels[i];
            let d_sp = length(P.xz - s_pos.xz);
            let wave_time = fract(audio.smooth_time * 1.8 - d_sp * 0.25);
            let wave_ring = smoothstep_r(0.22, 0.0, abs(wave_time - 0.5)) * smoothstep_r(9.0, 0.0, d_sp);
            emission += s_col * wave_ring * s_energy * 0.42;
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
        
        // Subtle acoustic fabric texture & panel bevels
        let panel_pattern = sin(uv_wall.x * 3.14159) * sin(uv_wall.y * 3.14159) * 0.015;
        let wall_base = vec3<f32>(0.018, 0.02, 0.03) + vec3<f32>(panel_pattern);
        let wall_neon_accent = vec3<f32>(0.08, 0.16, 0.35);
        albedo = mix(wall_base, wall_neon_accent, grid_lines * 0.7);
        roughness = 0.65;
        
    } else if (mat_id < 1.5) {
        // === Material 1.0: Speaker Cabinet / Pedestals (Sleek Matte Black) ===
        albedo = vec3<f32>(0.025, 0.025, 0.03);
        roughness = 0.35;
        
    } else if (mat_id < 2.5) {
        // === Material 2.0: Active Bass Woofer Diaphragms (Ribbed Cone + Dustcap + Rubber Surround) ===
        let cw = in.cone_center_weight;
        let is_surround = smoothstep_r(0.16, 0.06, cw);
        let is_dustcap = smoothstep(0.68, 0.76, cw);
        let cone_ribs = 0.85 + 0.15 * sin(cw * 36.0);
        let cone_base = vec3<f32>(0.035, 0.035, 0.045) * cone_ribs;
        let glow_intensity = (ch_energy * 2.4 + bass_norm * 1.1) * (1.0 - is_surround * 0.7) * (0.65 + 0.65 * is_dustcap);
        emission = ch_col * glow_intensity;
        albedo = mix(cone_base, ch_col * 0.35, clamp(ch_energy, 0.0, 1.0));
        roughness = mix(0.22, 0.08, is_dustcap);
        
    } else if (mat_id < 3.5) {
        // === Material 3.0: Active Midrange Diaphragms (Ribbed Kevlar/Carbon Cone + Phase Plug) ===
        let cw = in.cone_center_weight;
        let is_plug = smoothstep(0.70, 0.78, cw);
        let cone_ribs = 0.88 + 0.12 * sin(cw * 28.0);
        let mid_glow = ch_energy * (2.2 + 0.8 * is_plug) * cone_ribs;
        emission = ch_col * mid_glow;
        albedo = vec3<f32>(0.05) * cone_ribs;
        roughness = mix(0.28, 0.10, is_plug);
        
    } else if (mat_id < 4.5) {
        // === Material 4.0: Titanium / Beryllium Tweeter Dome ===
        let cw = in.cone_center_weight;
        let dome_glow = 0.7 + 0.6 * cw;
        emission = ch_col * (ch_energy * 2.2 + treble_norm * 1.4) * dome_glow;
        albedo = vec3<f32>(0.10, 0.11, 0.13);
        roughness = 0.08;
        
    } else if (mat_id < 5.5) {
        // === Material 5.0: Emissive Neon Halo / Guide Rails ===
        emission = ch_col * (1.6 + ch_energy * 2.4);
        albedo = ch_col;
        roughness = 0.1;
        
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
        let fresnel = pow(1.0 - max(dot(N, V), 0.0), 3.2);
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
    let dist_to_cam = length(P - ro);
    let fog = smoothstep(12.0, 45.0, dist_to_cam);
    let fog_col = vec3<f32>(0.006, 0.008, 0.015);
    final_color = mix(final_color, fog_col, fog);
    
    // Tone mapping
    let tonemapped = aces_tonemap(final_color);
    return vec4<f32>(tonemapped, 1.0);
}
