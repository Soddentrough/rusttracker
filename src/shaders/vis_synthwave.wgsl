// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;
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
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) hit_val: f32,
    @location(3) world_normal: vec3<f32>,
    @location(4) is_sky: f32,
}

fn hash1(n: f32) -> f32 { return fract(sin(n) * 43758.5453); }

fn hash2d(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise2d(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2d(i);
    let b = hash2d(i + vec2<f32>(1.0, 0.0));
    let c = hash2d(i + vec2<f32>(0.0, 1.0));
    let d = hash2d(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn ridge(p: vec2<f32>) -> f32 {
    return 1.0 - abs(noise2d(p) * 2.0 - 1.0);
}

fn terrain_h(wx: f32, wz: f32, cam_z: f32) -> f32 {
    let road_half = 6.0;
    let dx = max(abs(wx) - road_half, 0.0);
    let slope = dx * 0.7;
    let q = vec2<f32>(wx, wz);
    
    let r1 = ridge(q * 0.04) * 1.0;
    let r2 = ridge(q * 0.1) * 0.45;
    
    let detail_mask = smoothstep(0.2, 1.0, r1 + r2);
    let n3 = noise2d(q * 0.25) * 0.3 * detail_mask;
    let n4 = noise2d(q * 0.6) * 0.15 * detail_mask;
    let n5 = noise2d(q * 1.5) * 0.08 * detail_mask;
    let n6 = noise2d(q * 3.2) * 0.03 * detail_mask;
    let h = r1 + r2 + n3 + n4 + n5 + n6;
    
    let dist_z = max((wz - cam_z), 0.0);
    let horizon_fade = smoothstep(220.0, 60.0, dist_z);
    
    let mtn_height = slope * h * 0.5 * horizon_fade;
    return mtn_height - 0.5;
}

fn terrain_normal(wx: f32, wz: f32, cam_z: f32) -> vec3<f32> {
    let e = 0.15;
    let hc = terrain_h(wx, wz, cam_z);
    let hx = terrain_h(wx + e, wz, cam_z);
    let hz = terrain_h(wx, wz + e, cam_z);
    return normalize(vec3<f32>(hc - hx, e, hc - hz));
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    let cam_z = audio.smooth_time * 20.0;
    
    // Use a cubic expansion to keep the center dense but flare the edges massively
    // This perfectly covers ultrawide aspect ratios without losing road resolution
    let norm_x = in.position.x / 100.0;
    let local_x = (norm_x * 400.0) + (norm_x * norm_x * norm_x * 1200.0);
    
    var local_z = (in.position.z + 100.0) * 2.0; // 0 to 400
    
    let is_backdrop = in.position.z > 99.0;
    if is_backdrop {
        local_z -= 2.0; // Make it perfectly vertical by sharing Z with the previous row
    }
    
    let world_x = local_x;
    let world_z = local_z + cam_z;
    
    var h = 0.0;
    var norm = vec3<f32>(0.0, 1.0, 0.0);
    var hit_val = 0.0;
    
    if is_backdrop {
        h = 400.0;
    } else {
        h = terrain_h(world_x, world_z, cam_z);
        norm = terrain_normal(world_x, world_z, cam_z);
        
        // Audio reactivity
        let x_idx = clamp(u32(abs(world_x) * 2.0), 0u, 255u);
        let t_idx = u32(abs(world_z)) % 120u;
        let tex_y = (audio.heatmap_row + 120u - t_idx) % 120u;
        hit_val = textureLoad(history_tex, vec2<i32>(i32(x_idx), i32(tex_y)), 0).x;
    }
    
    let view_pos = vec3<f32>(local_x, h, local_z);
    let world_pos = vec3<f32>(world_x, h, world_z);
    
    out.world_pos = world_pos;
    out.hit_val = hit_val;
    out.world_normal = norm;
    out.uv = in.tex_coords;
    out.is_sky = select(0.0, 1.0, is_backdrop);
    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(view_pos, 1.0);
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    if in.is_sky > 0.01 {
        // --- Backdrop (Sky and Sun) ---
        // Calculate a pseudo-screen UV from the world position of the backdrop wall
        let p = vec2<f32>(in.world_pos.x * 0.015, in.world_pos.y * 0.015 - 0.2);
        
        let sky_t = clamp(p.y * 1.2 + 0.3, 0.0, 1.0);
        var color = mix(vec3<f32>(0.01, 0.0, 0.02),
                        mix(vec3<f32>(0.03, 0.0, 0.06), vec3<f32>(0.08, 0.01, 0.06), sky_t), sky_t);
                        
        // Sun
        let sun_pos = vec2<f32>(0.0, 0.2);
        let sun_dist = length(p - sun_pos);
        let sun_radius = 0.35;
        
        // Sun glow
        color += vec3<f32>(0.4, 0.1, 0.2) * exp(-sun_dist * 2.5) * 0.25;
        
        if (sun_dist < sun_radius && p.y > -0.05) {
            let cut = fract((p.y - sun_pos.y) * 20.0 - audio.smooth_time * 0.8);
            let cut_width = 0.3 + (p.y - sun_pos.y) * 0.5;
            if (cut > cut_width) {
                let glow = exp(-(cut - cut_width) * 10.0) * vec3<f32>(1.0, 0.5, 0.0);
                color = mix(vec3<f32>(1.0, 0.8, 0.2), vec3<f32>(1.0, 0.1, 0.5), p.y + 0.2) + glow * 0.5;
            }
        }
        
        // Stars
        let star_uv = p * 80.0;
        let star_id = floor(star_uv);
        let star_rnd = hash1(star_id.x * 127.1 + star_id.y * 311.7);
        let star_b = step(0.97, star_rnd) * smoothstep(0.04, 0.0, length(fract(star_uv) - 0.5)) * clamp(p.y - 0.1, 0.0, 1.0);
        color += vec3<f32>(star_b);
        
        return vec4<f32>(color, 1.0);
    }
    
    // --- Terrain ---
    let road_half = 6.0;
    let hit_road = abs(in.world_pos.x) < road_half + 0.5;
    
    var color = vec3<f32>(0.0);
    
    if hit_road {
        let grid_x = smoothstep(0.45, 0.5, abs(fract(in.world_pos.x * 2.0) - 0.5));
        let grid_z = smoothstep(0.45, 0.5, abs(fract(in.world_pos.z * 2.0) - 0.5));
        let grid = max(grid_x, grid_z);
        let speed_stripe = smoothstep(0.9, 1.0, fract(in.world_pos.z * 0.1));
        let base_c = vec3<f32>(0.02, 0.0, 0.05);
        let line_c = mix(vec3<f32>(0.0, 0.5, 1.0), vec3<f32>(1.0, 0.0, 0.8), speed_stripe);
        color = mix(base_c, line_c, grid);
        
        let edge = smoothstep(road_half - 0.5, road_half + 0.5, abs(in.world_pos.x));
        color += vec3<f32>(1.0, 0.2, 0.8) * edge * 2.0;
    } else {
        let n = in.world_normal;
        let l = normalize(vec3<f32>(-0.5, 0.8, -0.3));
        let diff = max(dot(n, l), 0.0);
        
        let grid_x = smoothstep(0.48, 0.5, abs(fract(in.world_pos.x) - 0.5));
        let grid_z = smoothstep(0.48, 0.5, abs(fract(in.world_pos.z) - 0.5));
        let grid = max(grid_x, grid_z);
        
        let mtn_c = vec3<f32>(0.01, 0.0, 0.02);
        let line_c = vec3<f32>(1.0, 0.0, 0.8);
        
        let beat = in.hit_val;
        let beat_c = vec3<f32>(0.0, 1.0, 1.0) * beat;
        
        color = mix(mtn_c, line_c + beat_c, grid * (0.3 + diff * 0.7));
    }
    
    // Depth fog
    let cam_z = audio.smooth_time * 20.0;
    let dist = length(in.world_pos - vec3<f32>(0.0, 1.5, cam_z - 2.0));
    let fog_f = smoothstep(40.0, 200.0, dist);
    let fog_c = vec3<f32>(0.02, 0.0, 0.05);
    color = mix(color, fog_c, fog_f);
    
    return vec4<f32>(color, 1.0);
}
