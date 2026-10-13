// ============================================================================
// 3D Airborne Water Splash Droplets — Instanced Billboard Quad Shader
// Renders glistening FLIP fluid splash droplets leaping from letter impacts
// ============================================================================

// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(1) var<storage, read> particles: array<Particle>;
@group(2) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
}

struct Particle {
    pos: vec4<f32>,
    vel: vec4<f32>,
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
}

struct SplashVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) local_uv: vec2<f32>,
    @location(2) droplet_r: f32,
    @location(3) eye_dir: vec3<f32>,
}

@vertex
fn vs_splash(in: VertexInput, @builtin(instance_index) inst_id: u32) -> SplashVertexOutput {
    var out: SplashVertexOutput;

    let p = particles[inst_id];

    // Only draw active airborne splash droplets launched around the letter impact stage (|x| < 6.5, |z| < 2.5, y > 0.035)
    if (p.pos.y <= 0.035 || abs(p.pos.x) > 6.5 || abs(p.pos.z) > 2.5) {
        out.clip_position = vec4<f32>(0.0, 0.0, -100.0, 1.0);
        return out;
    }

    // Early view-frustum culling on particle center before billboard expansion
    let center_clip = camera.proj_matrix * (camera.view_matrix * vec4<f32>(p.pos.xyz, 1.0));
    if (center_clip.w <= 0.05 || abs(center_clip.x) > center_clip.w * 1.15 || abs(center_clip.y) > center_clip.w * 1.15) {
        out.clip_position = vec4<f32>(0.0, 0.0, -100.0, 1.0);
        return out;
    }

    // Camera basis vectors from view matrix
    let cam_right = normalize(vec3<f32>(camera.view_matrix[0][0], camera.view_matrix[1][0], camera.view_matrix[2][0]));
    let cam_up = normalize(vec3<f32>(camera.view_matrix[0][1], camera.view_matrix[1][1], camera.view_matrix[2][1]));

    let speed = length(p.vel.xyz);
    let droplet_r = 0.026 + clamp(speed * 0.006, 0.0, 0.032);

    let offset = (in.position.x * cam_right + in.position.y * cam_up) * droplet_r;
    let world_p = p.pos.xyz + offset;

    let aspect = max(0.25, audio.aspect_ratio);
    let lyrics_scale = max(1.0, 2.2 / aspect);
    let eye_pos = vec3<f32>(0.0, 0.45 + 0.95 * lyrics_scale, 4.2 * lyrics_scale);

    out.world_pos = world_p;
    out.local_uv = in.position.xy * 2.0; // [-1.0, 1.0]
    out.droplet_r = droplet_r;
    out.eye_dir = normalize(eye_pos - world_p);

    let view_pos = camera.view_matrix * vec4<f32>(world_p, 1.0);
    out.clip_position = camera.proj_matrix * view_pos;

    return out;
}

@fragment
fn fs_splash(in: SplashVertexOutput) -> @location(0) vec4<f32> {
    let r2 = dot(in.local_uv, in.local_uv);
    if (r2 > 1.0) {
        discard;
    }

    let edge_alpha = smoothstep_r(1.0, 0.82, r2);
    if (edge_alpha <= 0.01) {
        discard;
    }

    let nz = sqrt(max(0.0, 1.0 - r2));
    // Camera basis vectors
    let cam_right = normalize(vec3<f32>(camera.view_matrix[0][0], camera.view_matrix[1][0], camera.view_matrix[2][0]));
    let cam_up = normalize(vec3<f32>(camera.view_matrix[0][1], camera.view_matrix[1][1], camera.view_matrix[2][1]));
    let V = normalize(in.eye_dir);
    let N = normalize(in.local_uv.x * cam_right + in.local_uv.y * cam_up + nz * V);
    let NdotV = max(0.0, dot(N, V));

    // Dielectric Fresnel for water droplet (n = 1.33)
    let fresnel = 0.04 + 0.96 * pow(1.0 - NdotV, 4.0);

    // Overhead Softbox Area Light Specular Highlight: pinpoint specular glint
    let softbox_pos = vec3<f32>(0.0, 5.2, -1.5);
    let L_sb = normalize(softbox_pos - in.world_pos);
    let H_sb = normalize(L_sb + V);
    let NdotH = max(0.0, dot(N, H_sb));
    let spec_softbox = pow(NdotH, 64.0) * 4.2 + pow(NdotH, 16.0) * 0.6;
    let softbox_col = vec3<f32>(3.0, 3.2, 3.6);

    // Sparkling rim reflection catching ambient light around droplet contour
    let contour_rim = pow(1.0 - NdotV, 3.0) * vec3<f32>(0.45, 0.85, 1.1) * 1.8;

    // Dual Studio Rim Lights
    let L_rim1 = normalize(vec3<f32>(-6.0, 2.5, 2.0) - in.world_pos);
    let rim1 = pow(max(0.0, dot(reflect(-V, N), L_rim1)), 28.0) * vec3<f32>(0.35, 0.75, 1.0) * 1.8;

    let L_rim2 = normalize(vec3<f32>(6.0, 2.2, 2.0) - in.world_pos);
    let rim2 = pow(max(0.0, dot(reflect(-V, N), L_rim2)), 28.0) * vec3<f32>(1.0, 0.88, 0.55) * 1.5;

    // Optical water body transmission: clean aquamarine core
    let water_core = vec3<f32>(0.12, 0.35, 0.65) * (0.5 + 0.5 * nz);
    let reflection = softbox_col * spec_softbox + contour_rim + rim1 + rim2;

    var col = mix(water_core, reflection, fresnel);
    col = aces_tonemap(col);

    let alpha = clamp((0.35 + fresnel * 0.65 + spec_softbox * 0.5) * edge_alpha, 0.0, 0.95);
    return vec4<f32>(col, alpha);
}
