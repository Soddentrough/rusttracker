// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(2) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
}

struct Particle {
    pos: vec4<f32>, // pos.xyz = position, pos.w = life
    vel: vec4<f32>, // vel.xyz = velocity, vel.w = energy
}

@group(3) @binding(0)
var<storage, read> particles: array<Particle>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
}

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) core_col: vec3<f32>,
    @location(2) @interpolate(flat) mid_col: vec3<f32>,
    @location(3) @interpolate(flat) edge_col: vec3<f32>,
    @location(4) @interpolate(flat) particle_params: vec3<f32>, // x: base_intensity, y: fog_factor, z: flicker
    @location(5) @interpolate(linear) ndc: vec2<f32>,
}

fn hash21_fast_biolum(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}

@vertex
fn vs_main_3d(in: VertexInput, @builtin(instance_index) inst_idx: u32) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    let p = particles[inst_idx];
    
    // Discard/hide particle if life is finished or energy is extremely low
    if (p.pos.w <= 0.0 || p.vel.w < 0.001) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.uv = vec2<f32>(0.0);
        out.core_col = vec3<f32>(0.0);
        out.mid_col = vec3<f32>(0.0);
        out.edge_col = vec3<f32>(0.0);
        out.particle_params = vec3<f32>(0.0);
        out.ndc = vec2<f32>(0.0);
        return out;
    }
    
    var view_pos = camera.view_matrix * vec4<f32>(p.pos.xyz, 1.0);
    let depth = -view_pos.z;
    
    // Cull particles behind/too close to camera, beyond the 38m deep-ocean fog horizon,
    // or sub-pixel particles at medium/far distance to cut distant alpha overdraw
    if (depth < 1.2 || depth > 38.0 || (depth > 9.0 && (inst_idx & 1u) != 0u) || (depth > 16.0 && (inst_idx & 3u) != 0u)) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.uv = vec2<f32>(0.0);
        out.core_col = vec3<f32>(0.0);
        out.mid_col = vec3<f32>(0.0);
        out.edge_col = vec3<f32>(0.0);
        out.particle_params = vec3<f32>(0.0);
        out.ndc = vec2<f32>(0.0);
        return out;
    }

    // Scale particle based on energy (pulsing effect) and clamp to prevent excessive blowouts
    let size = 0.035 + clamp(p.vel.w, 0.0, 1.6) * 0.088;
    
    // Billboard the vertex: offset particle center in view space
    view_pos.x += in.position.x * size;
    view_pos.y += in.position.y * size;
    
    var clip_pos = camera.proj_matrix * view_pos;
    
    // Apply barrel distortion in clip space (NDC)
    let ndc = clamp(clip_pos.xy / clip_pos.w, vec2<f32>(-2.0), vec2<f32>(2.0));
    let r2 = min(2.0, dot(ndc, ndc));
    let distorted_ndc = ndc * (1.0 + r2 * 0.055);

    // Frustum-cull off-screen particles in the vertex stage so they never hit the tiler or fragment discard
    if (abs(distorted_ndc.x) > 1.08 || abs(distorted_ndc.y) > 1.08) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.uv = vec2<f32>(0.0);
        out.core_col = vec3<f32>(0.0);
        out.mid_col = vec3<f32>(0.0);
        out.edge_col = vec3<f32>(0.0);
        out.particle_params = vec3<f32>(0.0);
        out.ndc = vec2<f32>(0.0);
        return out;
    }

    clip_pos = vec4<f32>(distorted_ndc * clip_pos.w, clip_pos.z, clip_pos.w);
    
    let energy = p.vel.w;
    let crest_t = clamp((p.pos.y + 0.45) / 1.35, 0.0, 1.0);
    let excitation = clamp((energy - 0.12) / 1.5, 0.0, 1.0);
    let crest_shift = clamp(crest_t * 0.65 + excitation * 0.55, 0.0, 1.0);

    let core_col = mix(vec3<f32>(0.65, 1.0, 0.95), vec3<f32>(0.92, 0.96, 1.0), crest_shift);
    let mid_teal = vec3<f32>(0.0, 0.85, 0.68);
    let mid_cyan = vec3<f32>(0.0, 0.76, 1.0);
    let mid_violet = vec3<f32>(0.42, 0.35, 1.0);
    let mid_col = mix(mix(mid_teal, mid_cyan, smoothstep(0.0, 0.55, crest_shift)), mid_violet, smoothstep(0.65, 1.0, crest_shift));
    let edge_col = mix(vec3<f32>(0.0, 0.16, 0.55), vec3<f32>(0.18, 0.06, 0.72), crest_shift);

    let dist_boost = select(1.0, 1.35, depth > 14.0);
    let base_intensity = (0.2 + energy * 2.5) * dist_boost;
    let fog_factor = smoothstep(5.0, 35.0, depth);
    let flicker = 0.985 + 0.015 * sin(audio.smooth_time * 24.0);

    out.clip_position = clip_pos;
    out.uv = in.tex_coords;
    out.core_col = core_col;
    out.mid_col = mid_col;
    out.edge_col = edge_col;
    out.particle_params = vec3<f32>(base_intensity, fog_factor, flicker);
    out.ndc = distorted_ndc;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    // 1. Bezel boundary check
    let abs_ndc = abs(in.ndc);
    if (abs_ndc.x > 1.0 || abs_ndc.y > 1.0) {
        discard;
    }

    // 2. Circle clip on each face using squared UV distance from center before sqrt
    let d_uv = in.uv - vec2<f32>(0.5, 0.5);
    let dist_sq = dot(d_uv, d_uv);
    if (dist_sq > 0.25) {
        discard;
    }
    let dist = sqrt(dist_sq);
    
    let border_dist = min(1.0 - abs_ndc.x, 1.0 - abs_ndc.y);
    let bezel_mask = smoothstep(0.0, 0.03, border_dist);
    
    // Radial color interpolation from flat vertex palette
    let t = dist * 2.0; // scale to 0.0 - 1.0
    var color = vec3<f32>(0.0);
    if (t < 0.3) {
        color = mix(in.core_col, in.mid_col, t * 3.333333);
    } else {
        color = mix(in.mid_col, in.edge_col, (t - 0.3) * 1.428571);
    }
    
    // Brightness profile & distance fog
    let intensity = in.particle_params.x * (1.0 - smoothstep(0.0, 0.5, dist));
    var final_color = mix(color * intensity, vec3<f32>(0.001, 0.003, 0.008), in.particle_params.y);
    
    // Apply bezel vignette mask, CRT scanlines & 24Hz flicker
    let scanline = 0.86 + 0.14 * cos(in.clip_position.y * 3.14159);
    final_color = final_color * (bezel_mask * scanline * in.particle_params.z);
    
    // CRT Filter: Analog static noise using fast ALU hash
    let noise_val = hash21_fast_biolum(in.clip_position.xy + fract(audio.smooth_time) * 149.0);
    let static_noise = noise_val * 0.022 * bezel_mask;
    final_color = final_color + vec3<f32>(static_noise);
    
    let tonemapped = aces_tonemap(final_color);
    return vec4<f32>(tonemapped, 1.0);
}

