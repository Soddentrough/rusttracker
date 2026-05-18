// INCLUDE: common
//
// Lissajous Laser Projector — XY oscilloscope with analog laser aesthetic
//
// Performance strategy:
//   - Only 1 current frame + 2 fading trail frames (not 48!)
//   - Downsample to 64 line segments max per frame
//   - Use wider bloom to compensate for fewer segments
//   - Total: 3 frames × 64 segments = 192 sdLine calls per pixel (vs 12,288 before)


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
    @location(2) normal: vec3<f32>,
    @location(3) audio_hit: f32,
}

fn get_knot_path(t: f32) -> vec3<f32> {
    let p = 3.0; // Number of loops around the axis of rotational symmetry
    let q = 7.0; // Number of loops through the hole of the torus
    
    // Animate the knot structure subtly
    let time = audio.smooth_time * 0.5;
    let r1 = 3.0 + sin(time * 0.7) * 0.5;
    let r2 = 1.2 + cos(time * 0.9) * 0.3;
    
    let phi = p * t + time;
    let theta = q * t;
    
    let r = r1 + r2 * cos(theta);
    let x = r * cos(phi);
    let z = r * sin(phi);
    let y = r2 * sin(theta);
    
    return vec3<f32>(x, y, z);
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    let u = in.tex_coords.x; // 0 to 1 along the knot
    let v = in.tex_coords.y; // 0 to 1 around the tube
    
    let t = u * 3.14159265 * 2.0;
    let theta = v * 3.14159265 * 2.0;
    
    // Sample audio history based on position along the knot
    let history_depth = u32(fract(u * 3.0) * 119.0); // wrap history 3 times around knot
    let tex_y = (audio.heatmap_row + 120u - history_depth) % 120u;
    
    // Sample a low-mid frequency bin for the pulse
    let hit_val = textureLoad(history_tex, vec2<i32>(5, i32(tex_y)), 0).x;
    let clamped_hit = clamp(hit_val, 0.0, 1.5);
    
    // Tube radius pulses with the music safely
    let radius = 0.15 + clamped_hit * 0.4;
    
    // Calculate tangent, normal, binormal for tube extrusion
    let e = 0.01;
    let p0 = get_knot_path(t);
    let p1 = get_knot_path(t + e);
    let tangent = normalize(p1 - p0);
    
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(tangent.y) > 0.99) {
        up = vec3<f32>(1.0, 0.0, 0.0);
    }
    
    let normal_dir = normalize(cross(tangent, up));
    let binormal = cross(normal_dir, tangent);
    
    let tube_offset = (normal_dir * cos(theta) + binormal * sin(theta));
    let final_pos = p0 + tube_offset * radius;
    
    out.world_pos = final_pos;
    out.normal = normalize(tube_offset);
    out.uv = in.tex_coords;
    out.audio_hit = hit_val;
    
    // Rotate the whole knot in front of the camera
    let rot_time = audio.smooth_time * 0.1;
    let c = cos(rot_time);
    let s = sin(rot_time);
    let rot_mat = mat3x3<f32>(
        c, 0.0, s,
        0.0, 1.0, 0.0,
        -s, 0.0, c
    );
    
    // Push the knot into the view space
    let world_pos = rot_mat * final_pos + vec3<f32>(0.0, 1.5, -6.0); // Center in front of camera
    
    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(world_pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    // Laser oscilloscope aesthetic
    let n = normalize(in.normal);
    let cam_eye = vec3<f32>(0.0, 1.5, -2.0);
    let view_dir = normalize(cam_eye - in.world_pos);
    
    // Lighting
    let l1 = normalize(vec3<f32>(1.0, 1.0, 1.0));
    let l2 = normalize(vec3<f32>(-1.0, -0.5, 0.5));
    
    let diff1 = max(dot(n, l1), 0.0);
    let diff2 = max(dot(n, l2), 0.0);
    
    let half1 = normalize(l1 + view_dir);
    let spec1 = pow(max(dot(n, half1), 0.0), 64.0);
    
    // Fresnel edge glow
    let fresnel = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0);
    
    // Laser wireframe grid
    let u_lines = smoothstep(0.3, 0.5, abs(fract(in.uv.x * 50.0) - 0.5));
    let v_lines = smoothstep(0.3, 0.5, abs(fract(in.uv.y * 12.0) - 0.5));
    let grid = max(u_lines, v_lines);
    
    let base_color = vec3<f32>(0.01, 0.05, 0.02);
    let laser_color = vec3<f32>(0.1, 1.0, 0.3); // Bright neon green
    let hot_color = vec3<f32>(1.0, 1.0, 0.8);   // White hot peaks
    
    let safe_hit = clamp(in.audio_hit, 0.0, 1.0);
    let hit_color = mix(laser_color, hot_color, safe_hit);
    
    // Combine surface colors
    var color = mix(base_color, hit_color, grid * 0.8);
    
    // Add specular highlights and rim lighting
    color += hit_color * spec1 * 2.0;
    color += hit_color * fresnel * (0.5 + safe_hit);
    
    // Inner pulse
    let pulse = smoothstep(0.8, 1.0, sin(in.uv.x * 100.0 - audio.smooth_time * 10.0));
    color += laser_color * pulse * 2.0;
    
    // Distance fog (relative to camera eye)
    let dist = length(in.world_pos - cam_eye);
    let fog = smoothstep(3.0, 12.0, dist);
    color = mix(color, vec3<f32>(0.0, 0.0, 0.0), fog);
    
    // Tonemapping for bloom-like oversaturation
    color = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    
    return vec4<f32>(color, 1.0);
}
