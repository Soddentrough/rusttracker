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
    @location(1) energy: f32,
    @location(2) depth: f32,
    @location(3) @interpolate(linear) ndc: vec2<f32>,
}

@vertex
fn vs_main_3d(in: VertexInput, @builtin(instance_index) inst_idx: u32) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    let p = particles[inst_idx];
    
    // Discard/hide particle if life is finished, energy is extremely low,
    // or if this is not the front face of the cube (to render as a single billboarded quad)
    if (p.pos.w <= 0.0 || p.vel.w < 0.001 || in.normal.z < 0.9) {
        out.clip_position = vec4<f32>(0.0, 0.0, -10.0, 1.0); // behind the camera
        out.uv = vec2<f32>(0.0);
        out.energy = 0.0;
        out.depth = 0.0;
        out.ndc = vec2<f32>(0.0);
        return out;
    }
    
    // Scale particle based on energy (pulsing effect) and clamp to prevent excessive blowouts
    let size = 0.035 + clamp(p.vel.w, 0.0, 1.5) * 0.09;
    
    // Billboard the vertex: offset particle center in view space
    var view_pos = camera.view_matrix * vec4<f32>(p.pos.xyz, 1.0);
    view_pos.x += in.position.x * size;
    view_pos.y += in.position.y * size;
    
    let depth = -view_pos.z;
    
    // Discard/hide particle if it is behind or too close to the camera to prevent huge screen-filling discs
    if (depth < 1.2) {
        out.clip_position = vec4<f32>(0.0, 0.0, -10.0, 1.0); // behind the camera
        out.uv = vec2<f32>(0.0);
        out.energy = 0.0;
        out.depth = 0.0;
        out.ndc = vec2<f32>(0.0);
        return out;
    }
    
    var clip_pos = camera.proj_matrix * view_pos;
    
    // Apply barrel distortion in clip space (NDC)
    let ndc = clamp(clip_pos.xy / clip_pos.w, vec2<f32>(-2.0), vec2<f32>(2.0));
    let r2 = min(2.0, dot(ndc, ndc));
    let distorted_ndc = ndc * (1.0 + r2 * 0.055);
    clip_pos = vec4<f32>(distorted_ndc * clip_pos.w, clip_pos.z, clip_pos.w);
    
    out.clip_position = clip_pos;
    out.uv = in.tex_coords;
    out.energy = p.vel.w;
    out.depth = depth;
    out.ndc = distorted_ndc;
    
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    // 1. Bezel boundary check
    if (abs(in.ndc.x) > 1.0 || abs(in.ndc.y) > 1.0) {
        discard;
    }
    
    let border_dist = min(1.0 - abs(in.ndc.x), 1.0 - abs(in.ndc.y));
    let bezel_mask = smoothstep(0.0, 0.03, border_dist);

    // 2. Circle clip on each face using UV coordinate distance from center
    let dist = length(in.uv - vec2<f32>(0.5, 0.5));
    if (dist > 0.5) {
        discard;
    }
    
    // Bioluminescent color gradient
    let core_col = vec3<f32>(0.7, 1.0, 1.0); // Bright cyan-white core
    let mid_col  = vec3<f32>(0.0, 0.75, 1.0); // Vivid electric cyan
    let edge_col = vec3<f32>(0.0, 0.15, 0.7); // Deep sea indigo/blue edge
    
    // Radial color interpolation
    let t = dist * 2.0; // scale to 0.0 - 1.0
    var color = vec3<f32>(0.0);
    if (t < 0.3) {
        color = mix(core_col, mid_col, t / 0.3);
    } else {
        color = mix(mid_col, edge_col, (t - 0.3) / 0.7);
    }
    
    // Brightness profile
    let intensity = (0.2 + in.energy * 2.5) * (1.0 - smoothstep(0.0, 0.5, dist));

    // Add distance fog to match the deep ocean depth feel
    let fog_factor = smoothstep(5.0, 35.0, in.depth);
    var final_color = mix(color * intensity, vec3<f32>(0.001, 0.003, 0.008), fog_factor);
    
    // Apply bezel vignette mask
    final_color = final_color * bezel_mask;
    
    // CRT Filter: Scanlines
    let scanline = 0.86 + 0.14 * cos(in.clip_position.y * 3.14159);
    final_color = final_color * scanline;
    
    // CRT Filter: Flicker
    let flicker = 0.98 + 0.02 * sin(audio.time * 115.0);
    final_color = final_color * flicker;
    
    // CRT Filter: Analog static noise
    let noise_val = hash21(in.clip_position.xy + fract(audio.smooth_time) * 149.0);
    let static_noise = noise_val * 0.022 * bezel_mask;
    final_color = final_color + vec3<f32>(static_noise);
    
    return vec4<f32>(final_color, 1.0);
}

