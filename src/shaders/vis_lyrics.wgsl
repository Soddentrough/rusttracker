// ============================================================================
// Visualizer ID 23: True 3D Beveled Glass Water Slam Lyrics Visualizer
// Hardware-rasterized 3D extruded beveled letterforms, tessellated dynamic
// water grid, and physical overhead softbox area light emitter.
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
        // Apply vertical gravity drop & spring damping directly to vertices
        world_pos.y += slam_y;
    } else if (mat_id >= 1.5 && mat_id < 2.5) {
        // --- 2. 3D Water Grid Plane (mat = 2.0) ---
        // Expanding circular shockwave ripple rings
        let r = length(world_pos.xz);
        var shockwave = 0.0;
        var dsw_dr = 0.0;
        if (slam_elapsed > 0.26) {
            let dt_impact = slam_elapsed - 0.26;
            let wave_front = dt_impact * 3.6;
            let dist_from_front = r - wave_front;
            
            let ring_wave = sin(dist_from_front * 14.0) * exp(-abs(dist_from_front) * 1.6);
            let damp = exp(-dt_impact * 2.0) * exp(-r * 0.35);
            shockwave = ring_wave * damp * 0.12;

            let d_ring = (14.0 * cos(dist_from_front * 14.0) - 1.6 * sign(dist_from_front) * sin(dist_from_front * 14.0)) * exp(-abs(dist_from_front) * 1.6);
            dsw_dr = d_ring * damp * 0.12;
        }

        // Ambient audio-reactive micro-ripples
        let rip1 = sin(world_pos.x * 3.5 + audio.time * 2.2) * cos(world_pos.z * 3.2 - audio.time * 1.8);
        let ambient = rip1 * 0.008 * (1.0 + bass * 1.5);
        world_pos.y = shockwave + ambient;

        // Perturb vertex normal from ripple gradient
        if (r > 0.001) {
            let grad_x = dsw_dr * (world_pos.x / r) + 3.5 * cos(world_pos.x * 3.5 + audio.time * 2.2) * cos(world_pos.z * 3.2 - audio.time * 1.8) * 0.008 * (1.0 + bass * 1.5);
            let grad_z = dsw_dr * (world_pos.z / r) - 3.2 * sin(world_pos.x * 3.5 + audio.time * 2.2) * sin(world_pos.z * 3.2 - audio.time * 1.8) * 0.008 * (1.0 + bass * 1.5);
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

    // Softbox Light Parameters: rectangular emitter at (0, 5.2, -1.5), width 16.0, height 3.6
    let softbox_pos = vec3<f32>(0.0, 5.2, -1.5);
    let softbox_w = 16.0;
    let softbox_h = 3.6;
    let softbox_radiance = vec3<f32>(3.2, 3.4, 3.8); // Pure brilliant studio white

    var col = vec3<f32>(0.0);

    if (mat_id >= 2.5) {
        // ====================================================================
        // 1. Overhead Physical Softbox Area Light Emitter (mat = 3.0)
        // ====================================================================
        col = softbox_radiance;
    } else if (mat_id >= 1.5) {
        // ====================================================================
        // 2. Tessellated 3D Water Basin Surface (mat = 2.0)
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

        // Deep obsidian water base + softbox and sky reflections
        let water_base = vec3<f32>(0.001, 0.0015, 0.003);
        let water_reflection = softbox_radiance * spec_softbox * 1.5 +
                               vec3<f32>(0.25, 0.45, 0.60) * pow(max(0.0, dot(N, vec3<f32>(0.0, 1.0, 0.2))), 32.0);

        col = mix(water_base, water_reflection, fresnel_water * 0.90);
    } else {
        // ====================================================================
        // 3. True 3D Beveled Dielectric Glass Letterforms (mat = 1.0)
        // ====================================================================
        // Dielectric Fresnel reflectance (F0 = 0.04 for glass)
        let fresnel = 0.04 + 0.96 * pow(1.0 - NdotV, 5.0);

        // --- Area Light Specular on Beveled Chamfers & Top Ridges ---
        let L_unnorm = softbox_pos - P;
        let R = reflect(-V, N);
        let t_proj = dot(R, L_unnorm) / max(0.001, dot(R, R));
        let P_refl = P + R * clamp(t_proj, 0.1, 10.0);
        let closest_sb_x = clamp(P_refl.x, -softbox_w * 0.45, softbox_w * 0.45);
        let closest_sb_pt = vec3<f32>(closest_sb_x, softbox_pos.y, softbox_pos.z);

        let L_area = normalize(closest_sb_pt - P);
        let H_area = normalize(L_area + V);
        let NdotH = max(0.0, dot(N, H_area));

        // Broad softbox highlight + razor-sharp bevel chamfer glint
        let spec_broad = pow(NdotH, 16.0);
        let spec_sharp = pow(NdotH, 128.0);
        
        // Chamfers facing upwards towards the softbox receive intense rim highlights
        let top_chamfer = pow(max(0.0, dot(N, vec3<f32>(0.0, 0.707, -0.707))), 8.0);
        let bot_chamfer = pow(max(0.0, dot(N, vec3<f32>(0.0, 0.707, 0.707))), 12.0);

        let spec_total = softbox_radiance * (spec_broad * 0.6 + spec_sharp * 3.5 + (top_chamfer + bot_chamfer) * 2.8);

        // --- Refraction & Internal Crystalline Transparency ---
        let refr_dir = refract(-V, N, 1.0 / 1.52);
        
        // Sample water reflection / dark floor through the glass interior
        let internal_floor = vec3<f32>(0.005, 0.008, 0.015);
        let internal_water_glow = vec3<f32>(0.08, 0.22, 0.32) * max(0.0, -refr_dir.y);
        let internal_color = internal_floor + internal_water_glow;

        col = mix(internal_color, spec_total, fresnel);
    }

    // Dynamic splash droplet particles
    let slam_elapsed = get_lyric_param(0u);
    if (slam_elapsed > 0.28 && slam_elapsed < 1.0) {
        let dt_drop = slam_elapsed - 0.28;
        let splash_falloff = exp(-abs(P.y - 0.2) * 3.0) * exp(-abs(P.z) * 4.0) * exp(-dt_drop * 3.0);
        col += vec3<f32>(0.6, 0.85, 1.0) * splash_falloff * 0.18;
    }

    // ACES Tonemapping
    col = aces_tonemap(col);

    return vec4<f32>(col, 1.0);
}
