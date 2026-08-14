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
    @location(5) energy: f32,
}

// Channel palette: distinct neon colors for spatial roles
fn get_spatial_color(ch: u32) -> vec3<f32> {
    switch (ch) {
        case 0u: { return vec3<f32>(0.0, 0.85, 1.0); }    // Front Left: Electric Cyan
        case 1u: { return vec3<f32>(1.0, 0.08, 0.75); }   // Front Right: Vivid Magenta
        case 2u: { return vec3<f32>(1.0, 0.75, 0.10); }   // Center: Amber Gold
        case 3u: { return vec3<f32>(1.0, 0.22, 0.02); }   // LFE Subwoofer: Intense Neon Orange/Red
        case 4u: { return vec3<f32>(0.25, 0.40, 1.0); }   // Surround Left: Royal Blue
        case 5u: { return vec3<f32>(0.80, 0.20, 1.0); }   // Surround Right: Electric Purple
        case 6u: { return vec3<f32>(0.0, 1.0, 0.75); }    // Rear Left: Neon Mint / Teal
        case 7u: { return vec3<f32>(0.95, 0.0, 0.45); }   // Rear Right: Deep Rose
        case 8u: { return vec3<f32>(0.35, 1.0, 0.35); }   // Top Front Left: Emerald
        case 9u: { return vec3<f32>(0.85, 1.0, 0.20); }   // Top Front Right: Lime
        case 10u: { return vec3<f32>(0.20, 0.90, 0.90); } // Top Rear Left: Aquamarine
        case 11u: { return vec3<f32>(1.0, 0.45, 0.85); }  // Top Rear Right: Hot Pink
        default: { return vec3<f32>(0.5, 0.7, 1.0); }
    }
}

fn get_channel_level(ch: u32) -> f32 {
    let num_sp = max(audio.num_spatial_channels, 1u);
    let idx = min(ch, num_sp - 1u);
    let sp = audio.spatial_channels[idx / 4u][idx % 4u];
    let ch_val = audio.channels[min(ch, max(audio.num_channels, 1u) - 1u) / 4u][min(ch, max(audio.num_channels, 1u) - 1u) % 4u];
    return max(sp, ch_val);
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    let ch_idx = u32(round(in.tex_coords.x));
    let mat_id = in.tex_coords.y;
    
    let ch_energy = get_channel_level(ch_idx);
    let bass_pulse = audio.spectrum[1].x * 1.5;
    
    // Woofer / mid cone physical displacement along normal
    var displacement = 0.0;
    if (mat_id >= 1.8 && mat_id <= 3.2) {
        if (ch_idx == 3u) {
            // LFE subwoofer has massive physical excursion
            displacement = (ch_energy * 0.45 + bass_pulse * 0.55) * 0.42;
        } else if (mat_id < 2.5) {
            // Bass woofers
            displacement = (ch_energy * 0.6 + bass_pulse * 0.4) * 0.22;
        } else {
            // Midrange cones
            displacement = ch_energy * 0.12;
        }
    }
    
    let displaced_pos = in.position + in.normal * displacement;
    out.world_pos = displaced_pos;
    out.world_normal = in.normal;
    out.uv = in.tex_coords;
    out.channel_id = in.tex_coords.x;
    out.material_id = mat_id;
    out.energy = ch_energy;
    
    // Interactive dynamic camera
    let t_cam = audio.smooth_time * 0.18;
    let cam_radius = 9.8;
    let cam_h = 2.4 + sin(t_cam * 0.7) * 0.6;
    let cam_x = sin(t_cam) * cam_radius * 0.55;
    let cam_z = -cos(t_cam) * cam_radius * 0.6 - 2.8;
    let ro = vec3<f32>(cam_x, cam_h, cam_z);
    let ta = vec3<f32>(0.0, 0.4, 2.5); // Focus toward stage center
    
    let cw = normalize(ta - ro);
    let cu = normalize(cross(cw, vec3<f32>(0.0, 1.0, 0.0)));
    let cv = cross(cu, cw);
    
    let view_matrix = mat4x4<f32>(
        vec4<f32>(cu.x, cv.x, -cw.x, 0.0),
        vec4<f32>(cu.y, cv.y, -cw.y, 0.0),
        vec4<f32>(cu.z, cv.z, -cw.z, 0.0),
        vec4<f32>(-dot(cu, ro), -dot(cv, ro), dot(cw, ro), 1.0)
    );
    
    let aspect = max(audio.aspect_ratio, 0.01);
    let fov_y = 1.15; // ~66 degree vertical FOV
    let f = 1.0 / tan(fov_y * 0.5);
    let z_near = 0.2;
    let z_far = 80.0;
    
    let proj_matrix = mat4x4<f32>(
        vec4<f32>(f / aspect, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, f, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, z_far / (z_near - z_far), -1.0),
        vec4<f32>(0.0, 0.0, (z_near * z_far) / (z_near - z_far), 0.0)
    );
    
    out.clip_position = proj_matrix * view_matrix * vec4<f32>(displaced_pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let P = in.world_pos;
    let N = normalize(in.world_normal);
    let mat_id = in.material_id;
    let ch_idx = u32(round(in.channel_id));
    let ch_energy = get_channel_level(ch_idx);
    let ch_col = get_spatial_color(ch_idx);
    
    // Camera position for specular / fresnel
    let t_cam = audio.smooth_time * 0.18;
    let cam_x = sin(t_cam) * 9.8 * 0.55;
    let cam_z = -cos(t_cam) * 9.8 * 0.6 - 2.8;
    let ro = vec3<f32>(cam_x, 2.4 + sin(t_cam * 0.7) * 0.6, cam_z);
    let V = normalize(ro - P);
    
    // Speaker source positions in room (for point light calculation)
    var speaker_pos = array<vec3<f32>, 8>(
        vec3<f32>(-3.8, 0.2, 5.5),  // FL (0)
        vec3<f32>( 3.8, 0.2, 5.5),  // FR (1)
        vec3<f32>( 0.0, -0.6, 6.8), // C (2)
        vec3<f32>( 0.0, -1.0, 5.2), // Sub (3)
        vec3<f32>(-6.2, 0.5, 0.5),  // SL (4)
        vec3<f32>( 6.2, 0.5, 0.5),  // SR (5)
        vec3<f32>(-4.2, 0.2, -4.5), // RL (6)
        vec3<f32>( 4.2, 0.2, -4.5)  // RR (7)
    );
    
    // Dynamic multi-point lighting accumulated from active speakers
    var point_light_diffuse = vec3<f32>(0.0);
    var point_light_specular = vec3<f32>(0.0);
    
    for (var i = 0u; i < 8u; i = i + 1u) {
        let s_pos = speaker_pos[i];
        let s_col = get_spatial_color(i);
        let s_energy = get_channel_level(i);
        let L_vec = s_pos - P;
        let dist = length(L_vec);
        let L_dir = normalize(L_vec);
        
        let atten = (s_energy * 2.8 + 0.15) / (dist * dist * 0.45 + 1.2);
        let NdotL = max(dot(N, L_dir), 0.0);
        point_light_diffuse += s_col * NdotL * atten;
        
        let H = normalize(L_dir + V);
        let NdotH = max(dot(N, H), 0.0);
        let spec_power = select(24.0, 64.0, mat_id < 1.0);
        point_light_specular += s_col * pow(NdotH, spec_power) * atten;
    }
    
    var albedo = vec3<f32>(0.03, 0.03, 0.04);
    var emission = vec3<f32>(0.0);
    var roughness = 0.6;
    
    if (mat_id < 0.2) {
        // === Material 0.0: Floor Stage Grid ===
        let grid_size = 1.0;
        let gx = abs(fract(P.x / grid_size) - 0.5);
        let gz = abs(fract(P.z / grid_size) - 0.5);
        let line_w = 0.03;
        let grid_line = smoothstep_r(line_w, 0.0, min(gx, gz));
        
        let floor_base = vec3<f32>(0.015, 0.015, 0.025);
        let floor_grid_col = vec3<f32>(0.08, 0.18, 0.35);
        albedo = mix(floor_base, floor_grid_col, grid_line * 0.7);
        roughness = 0.25;
        
        // Acoustic wavefront rings expanding across the floor
        for (var i = 0u; i < 4u; i = i + 1u) {
            let s_pos = speaker_pos[i];
            let s_col = get_spatial_color(i);
            let s_energy = get_channel_level(i);
            let d_sp = length(P.xz - s_pos.xz);
            let wave_time = fract(audio.smooth_time * 1.8 - d_sp * 0.25);
            let wave_ring = smoothstep_r(0.25, 0.0, abs(wave_time - 0.5)) * smoothstep_r(9.0, 0.0, d_sp);
            emission += s_col * wave_ring * s_energy * 0.35;
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
        // === Material 2.0: Active Bass Woofer Diaphragms ===
        let cone_base = vec3<f32>(0.04, 0.04, 0.05);
        let glow_intensity = ch_energy * 3.5 + audio.spectrum[1].x * 2.0;
        emission = ch_col * glow_intensity;
        albedo = mix(cone_base, ch_col * 0.3, clamp(ch_energy, 0.0, 1.0));
        roughness = 0.2;
        
    } else if (mat_id < 3.5) {
        // === Material 3.0: Active Midrange Diaphragms ===
        let mid_glow = ch_energy * 3.0;
        emission = ch_col * mid_glow;
        albedo = vec3<f32>(0.05);
        roughness = 0.3;
        
    } else if (mat_id < 4.5) {
        // === Material 4.0: Tweeter Dome / Horn ===
        let treble_boost = audio.spectrum[60].x * 2.0;
        emission = ch_col * (ch_energy * 2.5 + treble_boost);
        albedo = vec3<f32>(0.08);
        roughness = 0.1;
        
    } else if (mat_id < 5.5) {
        // === Material 5.0: Emissive Neon Halo / Guide Rails ===
        emission = ch_col * (1.8 + ch_energy * 2.5);
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
        let wood_grain = fract(sin(dot(uv_wood, vec2<f32>(12.9898, 78.233))) * 43758.5453);
        let slat_ring = sin(uv_wood.y * 6.28 + wood_grain * 0.5) * 0.012;
        albedo = vec3<f32>(0.13, 0.07, 0.035) + vec3<f32>(0.03, 0.02, 0.01) * wood_grain + vec3<f32>(slat_ring);
        roughness = 0.75;
        
    } else {
        // === Material 7.0: Soundproof Studio Control Room Glass Window ===
        let glass_base = vec3<f32>(0.01, 0.025, 0.045);
        let fresnel = pow(1.0 - max(dot(N, V), 0.0), 3.5);
        let glass_reflection = vec3<f32>(0.08, 0.22, 0.45) * fresnel;
        albedo = glass_base;
        emission = glass_reflection * 1.8;
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
