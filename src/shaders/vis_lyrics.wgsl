// ============================================================================
// Visualizer ID 23: 3D Beveled Optical Crystal Glass Water Lyrics Visualizer
// Hardware-rasterized 3D extruded beveled letterforms, dynamic FLIP Navier-Stokes
// water basin with planar reflections, chromatic dispersion, and studio lighting.
// ============================================================================

// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var<storage, read> render_grid: array<u32>;
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
    @location(3) char_id: f32,
    @location(4) material_id: f32,
    @location(5) eye_dir: vec3<f32>,
}

fn get_lyric_param(idx: u32) -> f32 {
    let vec_idx = idx / 4u;
    let comp_idx = idx % 4u;
    return audio.fire_heat[vec_idx][comp_idx];
}

fn get_lyrics_eye_pos() -> vec3<f32> {
    let aspect = max(0.25, audio.aspect_ratio);
    let lyrics_scale = max(1.0, 2.2 / aspect);
    return vec3<f32>(0.0, 0.45 + 0.95 * lyrics_scale, 4.2 * lyrics_scale);
}

// Sample smoothed FLIP Navier-Stokes surface height from upper 16 bits of render_grid
fn sample_flip_surface_displacement(pos_xz: vec2<f32>) -> f32 {
    let u = clamp((pos_xz.x + 16.0) / 32.0, 0.002, 0.997);
    let v = clamp((pos_xz.y + 10.0) / 20.0, 0.002, 0.997);
    let cx = i32(u * 512.0);
    let cz = i32(v * 512.0);

    var sum_h = 0.0;
    var count = 0.0;
    for (var dz = -1; dz <= 1; dz++) {
        let gz = clamp(cz + dz, 0, 511);
        for (var dx = -1; dx <= 1; dx++) {
            let gx = clamp(cx + dx, 0, 511);
            let raw = render_grid[u32(gz) * 512u + u32(gx)] >> 16u;
            if (raw > 370u) {
                sum_h += (f32(raw) * 0.001 - 0.37);
                count += 1.0;
            }
        }
    }
    if (count > 0.0) {
        return clamp((sum_h / count) * 0.04, 0.0, 0.06);
    }
    return 0.0;
}

// Continuous hydrodynamic fluid elevation on expansive water basin
fn get_water_surface_height(pos_xz: vec2<f32>) -> f32 {
    let slam_elapsed = get_lyric_param(0u);
    let slam_y = get_lyric_param(1u);
    let bass = get_lyric_param(6u);
    let text_half_w = max(2.5, get_lyric_param(7u));

    var h = sample_flip_surface_displacement(pos_xz);

    // 1. Hydrodynamic Expanding Wake Ripple Rings from Letter Impact
    if (slam_elapsed > 0.25) {
        let dt_impact = slam_elapsed - 0.25;
        // Elliptical wavefront matching elongated typography aspect ratio (use multiplication instead of pow to prevent NaN on x < 0)
        let x_scaled = pos_xz.x / 1.75;
        let r_eff = sqrt(x_scaled * x_scaled + pos_xz.y * pos_xz.y);
        let wave_front = dt_impact * 3.8;
        let d_front = r_eff - wave_front;

        let wave_ripple = sin(d_front * 8.5) * exp(-abs(d_front) * 1.1);
        let wave_decay = exp(-dt_impact * 1.3) * exp(-r_eff * 0.16);
        h += wave_ripple * wave_decay * 0.14 * (1.0 + bass * 0.6);
    }

    // 2. Letterform Displacement Trough directly under submerged letters
    if (slam_y < 0.0) {
        let dx = abs(pos_xz.x);
        let dz = abs(pos_xz.y);
        if (dx < text_half_w + 0.35 && dz < 0.45) {
            let env_x = smoothstep_r(text_half_w + 0.35, text_half_w * 0.8, dx);
            let env_z = smoothstep_r(0.45, 0.1, dz);
            h += slam_y * 0.75 * env_x * env_z;
        }
    }

    // 3. Multi-Octave Acoustic Faraday Surface Ripples
    let t = audio.time;
    let rip1 = sin(pos_xz.x * 2.8 + t * 2.4) * cos(pos_xz.y * 3.0 - t * 1.8);
    let rip2 = sin(pos_xz.x * 6.2 - t * 3.6 + pos_xz.y * 4.5) * 0.32;
    h += (rip1 + rip2) * 0.009 * (1.0 + bass * 1.2);

    return h;
}

// Sample splash droplet density from lower 16 bits of render grid for volumetric impact mist
fn sample_flip_splash_density(pos_xz: vec2<f32>) -> f32 {
    let u = clamp((pos_xz.x + 16.0) / 32.0, 0.0, 0.999);
    let v = clamp((pos_xz.y + 10.0) / 20.0, 0.0, 0.999);
    let gx = u32(u * 512.0);
    let gz = u32(v * 512.0);
    let idx = gz * 512u + gx;
    return f32(render_grid[idx] & 0xFFFFu);
}

// High-precision central difference surface normal for distortion-free glass reflections
fn get_water_surface_normal(pos_xz: vec2<f32>) -> vec3<f32> {
    let eps = 0.06;
    let h_l = get_water_surface_height(pos_xz - vec2<f32>(eps, 0.0));
    let h_r = get_water_surface_height(pos_xz + vec2<f32>(eps, 0.0));
    let h_d = get_water_surface_height(pos_xz - vec2<f32>(0.0, eps));
    let h_u = get_water_surface_height(pos_xz + vec2<f32>(0.0, eps));

    let dx = (h_r - h_l) / (2.0 * eps);
    let dz = (h_u - h_d) / (2.0 * eps);
    return normalize(vec3<f32>(-dx, 1.0, -dz));
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    let char_id = in.tex_coords.x;
    let mat_id = in.tex_coords.y;
    
    let slam_elapsed = get_lyric_param(0u);
    let slam_y = get_lyric_param(1u);
    let bass = get_lyric_param(6u);
    
    var world_pos = in.position;
    var world_normal = in.normal;

    if (mat_id < 1.5) {
        // --- 1. 3D Glass Letterforms (mat = 1.0) ---
        // Vertical slam drop & continuous buoyancy bob
        world_pos.y += slam_y;

        // Subtle per-character fluid wave oscillation (individual floating glyphs)
        if (slam_elapsed > 0.28) {
            let char_phase = char_id * 0.45;
            let wave_bob = sin(audio.time * 2.2 + char_phase) * 0.016 * (1.0 + bass * 0.6);
            let wave_tilt = cos(audio.time * 1.8 + char_phase) * 0.010;
            world_pos.y += wave_bob;
            world_pos.z += wave_tilt * (world_pos.y - 0.16);
        }
    } else if (mat_id >= 1.5 && mat_id < 2.5) {
        // --- 2. 3D Live FLIP Navier-Stokes Water Basin Plane (mat = 2.0) ---
        world_pos.y = get_water_surface_height(world_pos.xz);
        world_normal = get_water_surface_normal(world_pos.xz);
    }
    // mat_id == 3.0: Overhead Softbox Emitter Quad remains fixed at (0.0, 5.2, -1.5)

    out.world_pos = world_pos;
    out.world_normal = world_normal;
    out.uv = in.tex_coords;
    out.char_id = char_id;
    out.material_id = mat_id;

    // View-projection transform
    let view_pos = camera.view_matrix * vec4<f32>(world_pos, 1.0);
    out.clip_position = camera.proj_matrix * view_pos;

    // Dynamic eye vector in world space matching engine aspect-ratio camera
    let eye_pos = get_lyrics_eye_pos();
    out.eye_dir = normalize(eye_pos - world_pos);

    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let mat_id = in.material_id;
    let P = in.world_pos;
    let N = normalize(in.world_normal);
    let V = normalize(in.eye_dir);
    let NdotV = max(0.0, dot(N, V));
    let bass = get_lyric_param(6u);
    let slam_elapsed = get_lyric_param(0u);
    let eye_pos = get_lyrics_eye_pos();

    // Primary Overhead Softbox: rectangular emitter at (0, 5.2, -1.5), width 16.0, height 3.6
    let softbox_pos = vec3<f32>(0.0, 5.2, -1.5);
    let softbox_w = 16.0;
    let softbox_h = 3.6;
    let softbox_radiance = vec3<f32>(3.4, 3.6, 4.0);

    var col = vec3<f32>(0.0);

    if (mat_id >= 2.5) {
        // ====================================================================
        // 1. Overhead Physical Softbox Area Light Emitter (mat = 3.0)
        // ====================================================================
        col = softbox_radiance;
    } else if (mat_id >= 1.5) {
        // ====================================================================
        // 2. Tessellated 3D Water Basin Surface with Live FLIP Waves (mat = 2.0)
        // ====================================================================
        let fresnel_water = 0.04 + 0.96 * pow(1.0 - NdotV, 4.0);
        let R = reflect(-V, N);

        // Compute softbox area light specular reflection on water
        let t_sb = (softbox_pos.y - P.y) / max(0.01, R.y);
        var spec_softbox = 0.0;
        if (t_sb > 0.0 && R.y > 0.0) {
            let hit_sb = P + R * t_sb;
            let dx = abs(hit_sb.x - softbox_pos.x);
            let dz = abs(hit_sb.z - softbox_pos.z);
            if (dx < softbox_w * 0.5 && dz < softbox_h * 0.7) {
                let edge_fade = smoothstep_r(softbox_w * 0.5, softbox_w * 0.35, dx) *
                                smoothstep_r(softbox_h * 0.7, softbox_h * 0.35, dz);
                spec_softbox = edge_fade;
            }
        }

        // Planar letterform reflection into the liquid water surface
        let line_progress = get_lyric_param(2u);
        var letter_refl = vec3<f32>(0.0);
        if (P.z > 0.0 && R.z < -0.01) {
            let t_letter = -P.z / R.z;
            let hit_letter = P + R * t_letter;
            let half_w = max(2.5, get_lyric_param(7u));
            if (abs(hit_letter.x) < half_w + 0.35 && hit_letter.y > -0.10 && hit_letter.y < 1.45) {
                let fade_x = smoothstep_r(half_w + 0.35, half_w * 0.85, abs(hit_letter.x));
                let fade_y = smoothstep(-0.10, 0.05, hit_letter.y) * smoothstep_r(1.45, 0.95, hit_letter.y);
                let refl_u = clamp((hit_letter.x + half_w) / (2.0 * half_w), 0.0, 1.0);
                let laser_sweep = exp(-pow((refl_u - line_progress) * 7.0, 2.0)) * 0.85;
                let crystal_tint = mix(vec3<f32>(0.40, 0.80, 1.1), vec3<f32>(0.75, 0.45, 1.35), laser_sweep) * (1.2 + bass * 1.2 + laser_sweep * 0.6);
                letter_refl = crystal_tint * fade_x * fade_y * 2.8;
            }
        }

        // Deep obsidian-sapphire water base + underwater caustic glow
        let water_depth_tint = vec3<f32>(0.008, 0.025, 0.055);
        let wave_curvature = abs(N.x) + abs(N.z);
        let caustic_pulse = pow(wave_curvature * 3.5, 2.0) * (1.0 + bass * 1.4);
        let underwater_glow = vec3<f32>(0.03, 0.12, 0.24) * (1.0 + bass * 1.2) * exp(-length(P.xz) * 0.08)
                            + vec3<f32>(0.15, 0.45, 0.70) * caustic_pulse * 0.65;
        let water_base = water_depth_tint + underwater_glow;
        let sky_ambient = vec3<f32>(0.10, 0.22, 0.35) * (0.6 + 0.4 * max(0.0, R.y));
        let water_reflection = softbox_radiance * spec_softbox * 1.6 + letter_refl + sky_ambient;

        col = mix(water_base, water_reflection, fresnel_water * 0.90 + 0.10);

        // Volumetric splash mist around impact zone
        let splash_dense = sample_flip_splash_density(P.xz);
        if (splash_dense > 0.0) {
            col += vec3<f32>(0.35, 0.70, 0.95) * min(splash_dense * 0.02, 0.40);
        }
    } else {
        // ====================================================================
        // 3. True 3D Beveled Optical Crystal Glass Letters (mat = 1.0)
        // ====================================================================
        let line_progress = get_lyric_param(2u);
        let char_count = max(1.0, get_lyric_param(3u));
        let char_norm = (in.char_id + 0.5) / char_count;

        // 3-Channel Chromatic Dispersion Refraction (RGB wavelength splitting)
        let refr_R = refract(-V, N, 1.0 / 1.49);
        let refr_G = refract(-V, N, 1.0 / 1.52);
        let refr_B = refract(-V, N, 1.0 / 1.55);

        // Internal optical crystal volume with depth transmission
        let internal_R = (vec3<f32>(0.03, 0.09, 0.16) + vec3<f32>(0.18, 0.38, 0.65) * max(0.0, -refr_R.y) * 1.3).r;
        let internal_G = (vec3<f32>(0.06, 0.18, 0.28) + vec3<f32>(0.28, 0.58, 0.90) * max(0.0, -refr_G.y) * 1.5).g;
        let internal_B = (vec3<f32>(0.09, 0.28, 0.44) + vec3<f32>(0.45, 0.80, 1.15) * max(0.0, -refr_B.y) * 1.7).b;
        var internal_dispersed = vec3<f32>(internal_R, internal_G, internal_B);

        // Audio-reactive crystal core luminescence + progressive karaoke laser sweep
        let core_glow = vec3<f32>(0.15, 0.45, 0.85) * pow(bass, 1.3) * 0.85;
        let sung_mask = smoothstep(line_progress + 0.04, line_progress - 0.02, char_norm);
        let active_laser = exp(-pow((char_norm - line_progress) * max(4.0, char_count * 0.45), 2.0));
        let karaoke_glow = vec3<f32>(0.08, 0.32, 0.55) * sung_mask
                         + vec3<f32>(0.45, 0.85, 1.35) * active_laser * (0.65 + bass * 0.55);
        internal_dispersed += core_glow + karaoke_glow;

        // Area Light Specular on Beveled Chamfers & Polished Faces
        let L_unnorm = softbox_pos - P;
        let R_letter = reflect(-V, N);
        let t_proj = dot(R_letter, L_unnorm) / max(0.001, dot(R_letter, R_letter));
        let P_refl = P + R_letter * clamp(t_proj, 0.1, 10.0);
        let closest_sb_x = clamp(P_refl.x, -softbox_w * 0.45, softbox_w * 0.45);
        let closest_sb_pt = vec3<f32>(closest_sb_x, softbox_pos.y, softbox_pos.z);

        let L_area = normalize(closest_sb_pt - P);
        let H_area = normalize(L_area + V);
        let NdotH = max(0.0, dot(N, H_area));

        let spec_broad = pow(NdotH, 16.0);
        let spec_sharp = pow(NdotH, 128.0);
        let top_ridge = pow(max(0.0, dot(N, vec3<f32>(0.0, 0.707, -0.707))), 8.0);
        let bot_ridge = pow(max(0.0, dot(N, vec3<f32>(0.0, 0.707, 0.707))), 12.0);
        let spec_total = softbox_radiance * (spec_broad * 0.5 + spec_sharp * 4.0 + (top_ridge + bot_ridge) * 2.5);

        // Dual Colored Studio Rim Lights (Cool Cyan Left + Warm Champagne Gold Right)
        let L_rim1 = normalize(vec3<f32>(-8.0, 3.2, -2.5) - P);
        let rim1 = pow(max(0.0, dot(R_letter, L_rim1)), 32.0) * vec3<f32>(0.25, 0.75, 1.0) * 2.2;

        let L_rim2 = normalize(vec3<f32>(8.0, 2.8, -2.0) - P);
        let rim2 = pow(max(0.0, dot(R_letter, L_rim2)), 40.0) * vec3<f32>(1.0, 0.80, 0.45) * 1.8;
        let rim_total = rim1 + rim2;

        // Prismatic Rainbow Flares on Beveled Chamfers (using shared iridescence_cos)
        let is_chamfer = step(0.08, abs(N.z)) * step(0.08, length(N.xy));
        let rainbow = iridescence_cos(fract(N.x + dot(N, V) * 0.5));
        let prism_glint = rainbow * pow(NdotH, 64.0) * (3.5 + active_laser * 2.0) * is_chamfer;

        // Dielectric Fresnel blend (F0 = 0.04 for optical glass)
        let fresnel = 0.04 + 0.96 * pow(1.0 - NdotV, 5.0);
        col = mix(internal_dispersed, spec_total + rim_total + prism_glint, fresnel);
    }

    // Atmospheric studio depth falloff and soft horizon haze
    let dist_cam = length(P - eye_pos);
    let haze = smoothstep(12.0, 32.0, dist_cam);
    let haze_col = vec3<f32>(0.003, 0.008, 0.018);
    col = mix(col, haze_col, haze * 0.75);

    // ACES Tonemapping
    col = aces_tonemap(col);

    return vec4<f32>(col, 1.0);
}
