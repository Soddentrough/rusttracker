// ============================================================================
// Visualizer ID 23: 3D Beveled Optical Crystal Glass Water Lyrics Visualizer
// Hardware-rasterized 3D extruded beveled letterforms, dynamic tessellated
// water basin with planar reflections, chromatic dispersion, and studio lighting.
// ============================================================================

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
    @location(3) char_id: f32,
    @location(4) material_id: f32,
    @location(5) eye_dir: vec3<f32>,
}

fn get_lyric_param(idx: u32) -> f32 {
    let vec_idx = idx / 4u;
    let comp_idx = idx % 4u;
    return audio.fire_heat[vec_idx][comp_idx];
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
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
        // --- 2. 3D Water Grid Plane (mat = 2.0) ---
        let r = length(world_pos.xz);
        var shockwave = 0.0;
        var dsw_dr = 0.0;

        // Expanding circular shockwave ripple rings from letter slam impact
        if (slam_elapsed > 0.26) {
            let dt_impact = slam_elapsed - 0.26;
            let wave_front = dt_impact * 4.2;
            let dist_from_front = r - wave_front;
            
            let ring_wave = sin(dist_from_front * 12.0) * exp(-abs(dist_from_front) * 1.4);
            let damp = exp(-dt_impact * 1.8) * exp(-r * 0.25);
            shockwave = ring_wave * damp * 0.15;

            let d_ring = (12.0 * cos(dist_from_front * 12.0) - 1.4 * sign(dist_from_front) * sin(dist_from_front * 12.0)) * exp(-abs(dist_from_front) * 1.4);
            dsw_dr = d_ring * damp * 0.15;
        }

        // Multi-octave audio-reactive liquid wave surface
        let t = audio.time;
        let rip1 = sin(world_pos.x * 2.5 + t * 2.0) * cos(world_pos.z * 2.8 - t * 1.6);
        let rip2 = sin(world_pos.x * 5.2 - t * 2.8 + world_pos.z * 2.0) * 0.35;
        let ambient = (rip1 + rip2) * 0.012 * (1.0 + bass * 1.4);
        world_pos.y = shockwave + ambient;

        // Perturb vertex normal from analytical derivative
        if (r > 0.001) {
            let d_ambient_x = (2.5 * cos(world_pos.x * 2.5 + t * 2.0) * cos(world_pos.z * 2.8 - t * 1.6)
                             + 5.2 * cos(world_pos.x * 5.2 - t * 2.8 + world_pos.z * 2.0) * 0.35) * 0.012 * (1.0 + bass * 1.4);
            let d_ambient_z = (-2.8 * sin(world_pos.x * 2.5 + t * 2.0) * sin(world_pos.z * 2.8 - t * 1.6)
                             + 2.0 * cos(world_pos.x * 5.2 - t * 2.8 + world_pos.z * 2.0) * 0.35) * 0.012 * (1.0 + bass * 1.4);
            let grad_x = dsw_dr * (world_pos.x / r) + d_ambient_x;
            let grad_z = dsw_dr * (world_pos.z / r) + d_ambient_z;
            world_normal = normalize(vec3<f32>(-grad_x, 1.0, -grad_z));
        }
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

    // Eye vector in world space (camera is at (0.0, 1.4, 4.2))
    let eye_pos = vec3<f32>(0.0, 1.4, 4.2);
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
    let eye_pos = vec3<f32>(0.0, 1.4, 4.2);

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
        // 2. Tessellated 3D Water Basin Surface with Planar Reflections (mat = 2.0)
        // ====================================================================
        let fresnel_water = 0.02 + 0.98 * pow(1.0 - NdotV, 5.0);
        let R = reflect(-V, N);

        // Compute softbox area light specular reflection on water
        let t_sb = (softbox_pos.y - P.y) / max(0.01, R.y);
        var spec_softbox = 0.0;
        if (t_sb > 0.0 && R.y > 0.0) {
            let hit_sb = P + R * t_sb;
            let dx = abs(hit_sb.x - softbox_pos.x);
            let dz = abs(hit_sb.z - softbox_pos.z);
            if (dx < softbox_w * 0.5 && dz < softbox_h * 0.7) {
                let edge_fade = smoothstep(softbox_w * 0.5, softbox_w * 0.35, dx) *
                                smoothstep(softbox_h * 0.7, softbox_h * 0.35, dz);
                spec_softbox = edge_fade;
            }
        }

        // Planar letterform reflection into the liquid water surface
        var letter_refl = vec3<f32>(0.0);
        let t_letter = -P.z / max(0.0001, R.z);
        if (t_letter > 0.0 && R.y > 0.0) {
            let hit_letter = P + R * t_letter;
            let half_w = max(2.5, get_lyric_param(7u));
            if (abs(hit_letter.x) < half_w + 0.3 && hit_letter.y > -0.05 && hit_letter.y < 1.45) {
                let fade_x = smoothstep(half_w + 0.3, half_w * 0.85, abs(hit_letter.x));
                let fade_y = smoothstep(-0.05, 0.1, hit_letter.y) * smoothstep(1.45, 0.9, hit_letter.y);
                // Luminous crystal reflection distorted by water surface ripples
                let crystal_tint = vec3<f32>(0.35, 0.75, 1.0) * (1.0 + bass * 1.5);
                let core_glint = pow(max(0.0, dot(R, vec3<f32>(0.0, 0.8, -0.6))), 16.0);
                letter_refl = crystal_tint * fade_x * fade_y * (2.2 + core_glint * 3.0);
            }
        }

        // Deep obsidian-sapphire water base + underwater caustic glow
        let water_depth_tint = vec3<f32>(0.002, 0.006, 0.015);
        let underwater_glow = vec3<f32>(0.02, 0.08, 0.16) * (1.0 + bass * 1.2) * exp(-length(P.xz) * 0.15);
        let water_base = water_depth_tint + underwater_glow;
        let sky_ambient = vec3<f32>(0.08, 0.15, 0.25) * pow(max(0.0, R.y), 3.0);
        let water_reflection = softbox_radiance * spec_softbox * 1.5 + letter_refl + sky_ambient;

        col = mix(water_base, water_reflection, fresnel_water * 0.94);
    } else {
        // ====================================================================
        // 3. True 3D Beveled Optical Crystal Glass Letters (mat = 1.0)
        // ====================================================================
        // 3-Channel Chromatic Dispersion Refraction (RGB wavelength splitting)
        let refr_R = refract(-V, N, 1.0 / 1.49);
        let refr_G = refract(-V, N, 1.0 / 1.52);
        let refr_B = refract(-V, N, 1.0 / 1.55);

        // Internal optical crystal volume with depth transmission
        let internal_R = (vec3<f32>(0.03, 0.09, 0.16) + vec3<f32>(0.18, 0.38, 0.65) * max(0.0, -refr_R.y) * 1.3).r;
        let internal_G = (vec3<f32>(0.06, 0.18, 0.28) + vec3<f32>(0.28, 0.58, 0.90) * max(0.0, -refr_G.y) * 1.5).g;
        let internal_B = (vec3<f32>(0.09, 0.28, 0.44) + vec3<f32>(0.45, 0.80, 1.15) * max(0.0, -refr_B.y) * 1.7).b;
        var internal_dispersed = vec3<f32>(internal_R, internal_G, internal_B);

        // Audio-reactive crystal core luminescence (pulses from within on beat)
        let core_glow = vec3<f32>(0.15, 0.45, 0.85) * pow(bass, 1.4) * 1.8;
        internal_dispersed += core_glow;

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

        // Prismatic Rainbow Flares on Beveled Chamfers
        let is_chamfer = step(0.08, abs(N.z)) * step(0.08, length(N.xy));
        let rainbow = 0.5 + 0.5 * cos(vec3<f32>(0.0, 2.094, 4.188) + N.x * 6.28 + dot(N, V) * 3.14);
        let prism_glint = rainbow * pow(NdotH, 64.0) * 3.5 * is_chamfer;

        // Dielectric Fresnel blend (F0 = 0.04 for optical glass)
        let fresnel = 0.04 + 0.96 * pow(1.0 - NdotV, 5.0);
        col = mix(internal_dispersed, spec_total + rim_total + prism_glint, fresnel);
    }

    // Dynamic splash spray and sparkling water droplets on impact
    if (slam_elapsed > 0.26 && slam_elapsed < 1.1) {
        let dt_drop = slam_elapsed - 0.26;
        let splash_ring_r = dt_drop * 4.0;
        let dist_ring = abs(length(P.xz) - splash_ring_r);
        let splash_beads = exp(-dist_ring * 6.0) * exp(-abs(P.y - 0.25) * 3.5) * exp(-dt_drop * 2.8);
        let sparkle = hash2(vec2<f32>(P.x * 12.0, P.z * 12.0));
        col += vec3<f32>(0.7, 0.9, 1.0) * splash_beads * (0.8 + sparkle * 1.5) * 0.4;
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
