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
    @location(2) normal: vec3<f32>,
    @location(3) audio_hit: f32,
    @location(4) local_u: f32,
}

// HSV to RGB helper for rainbow effects
fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + k.xyz) * 6.0 - k.www);
    return c.z * mix(k.xxx, clamp(p - k.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

fn get_knot_path(t: f32) -> vec3<f32> {
    // 3 and 7 loops around torus
    let p = 3.0; 
    let q = 7.0; 
    
    let time = audio.smooth_time * 0.4;
    
    // Scale down the overall size to fit perfectly inside the viewport.
    // Base radius: r1 ~ 2.0, r2 ~ 0.8
    // Bass swells the whole torus knot dynamically!
    let bass = clamp(audio.spectrum[2].x * 0.8, 0.0, 1.0);
    let r1 = 1.8 + sin(time * 0.7) * 0.3 + bass * 0.3;
    let r2 = 0.7 + cos(time * 0.9) * 0.15 + bass * 0.1;
    
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
    
    // Sample audio history for pulsing thickness
    let history_depth = u32(fract(u * 2.0) * 119.0);
    let tex_y = (audio.heatmap_row + 1024u - history_depth) % 1024u;
    
    // Sample low-mid frequency for expansion pulse
    let hit_val = textureLoad(history_tex, vec2<i32>(10, i32(tex_y)), 0).x;
    let clamped_hit = clamp(hit_val, 0.0, 1.5);
    
    // Treble frequency for jitter/vibration (laser oscilloscope glitch)
    let treble_val = textureLoad(history_tex, vec2<i32>(120, i32(tex_y)), 0).x;
    let treble_clamped = clamp(treble_val, 0.0, 1.0);
    
    // Base tube radius pulses with the music
    let radius = 0.08 + clamped_hit * 0.15;
    
    // tangent and normal calculations for tube extrusion
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
    
    // Dynamic high-frequency laser vibration/glitch
    let jitter_phase = u * 800.0 + audio.smooth_time * 80.0;
    let jitter_val = sin(jitter_phase) * treble_clamped * 0.04;
    
    let tube_offset = (normal_dir * cos(theta) + binormal * sin(theta));
    let final_pos = p0 + tube_offset * (radius + jitter_val);
    
    // Rotate the knot slowly in 3D
    let rot_time = audio.smooth_time * 0.15;
    let c_y = cos(rot_time);
    let s_y = sin(rot_time);
    let rot_y = mat3x3<f32>(
        c_y, 0.0, s_y,
        0.0, 1.0, 0.0,
        -s_y, 0.0, c_y
    );
    
    let rot_time2 = audio.smooth_time * 0.08;
    let c_x = cos(rot_time2);
    let s_x = sin(rot_time2);
    let rot_x = mat3x3<f32>(
        1.0, 0.0, 0.0,
        0.0, c_x, -s_x,
        0.0, s_x, c_x
    );
    
    let rotated_pos = rot_x * rot_y * final_pos;
    
    // Position the knot at Z = +6.0 in front of the camera (which is at Z = -2.0)
    // Camera-to-object distance is 8.0 units, which fits perfectly within the viewport!
    let world_pos = rotated_pos + vec3<f32>(0.0, 1.5, 6.0);
    
    out.world_pos = world_pos;
    out.normal = normalize(rot_x * rot_y * tube_offset);
    out.uv = in.tex_coords;
    out.audio_hit = hit_val;
    out.local_u = u;
    
    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(world_pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let cam_eye = vec3<f32>(0.0, 1.5, -2.0);
    let view_dir = normalize(cam_eye - in.world_pos);
    
    // Lighting
    let l1 = normalize(vec3<f32>(1.0, 1.0, 1.0));
    let l2 = normalize(vec3<f32>(-1.0, -0.5, 0.5));
    
    let diff1 = max(dot(n, l1), 0.0);
    let diff2 = max(dot(n, l2), 0.0);
    
    let half1 = normalize(l1 + view_dir);
    let spec1 = pow(max(dot(n, half1), 0.0), 32.0);
    
    // Fresnel rim glow
    let fresnel = pow(1.0 - max(dot(n, view_dir), 0.0), 4.0);
    
    // Grid lines on the tube to create a vector laser scanline style
    let u_lines = smoothstep(0.3, 0.5, abs(fract(in.uv.x * 120.0) - 0.5));
    let v_lines = smoothstep(0.3, 0.5, abs(fract(in.uv.y * 16.0) - 0.5));
    let grid = max(u_lines, v_lines);
    
    // Rainbow hue shifts along the curve of the torus knot
    let hue = fract(in.local_u * 1.5 + audio.smooth_time * 0.06);
    let laser_color = hsv2rgb(vec3<f32>(hue, 0.95, 1.0));
    let hot_color = vec3<f32>(1.0, 1.0, 1.0); // White hot peaks
    
    let safe_hit = clamp(in.audio_hit, 0.0, 1.2);
    let hit_color = mix(laser_color, hot_color, smoothstep(0.4, 1.2, safe_hit));
    
    // Dark core color
    let base_color = vec3<f32>(0.005, 0.005, 0.008);
    
    // Combine base material with grid lines
    var color = mix(base_color, hit_color, grid * 0.85);
    
    // Add specular highlights and rim lighting
    color += hit_color * spec1 * 1.8;
    color += hit_color * fresnel * (0.8 + safe_hit * 1.5);
    
    // Laser pulse scanning along the wireframe
    let pulse_speed = audio.smooth_time * 8.0;
    let pulse = smoothstep(0.9, 1.0, sin(in.uv.x * 250.0 - pulse_speed));
    color += hit_color * pulse * 2.0;
    
    // Elegant distance fog to fade out in the distance against pitch black background
    let dist = length(in.world_pos - cam_eye);
    let fog = smoothstep(5.0, 18.0, dist);
    color = mix(color, vec3<f32>(0.0, 0.0, 0.0), fog);
    
    // High-quality ACES-like tone mapping to enhance brightness and bloom
    color = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    
    return vec4<f32>(color, 1.0);
}
