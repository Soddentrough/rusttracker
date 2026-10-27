// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(1) var<storage, read> waveform_history: array<f32>;
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
    @location(1) line_amber_scaled: vec3<f32>,
    @location(2) glass_and_ambient: vec3<f32>,
    @location(3) coc_inv: vec4<f32>, // x: inv_coc_10, y: inv_coc_05, z: inv_coc_02, w: inv_coc_01 * bloom_boost
    @location(4) local_z: f32,
    @location(5) fade_params: vec3<f32>, // x: alpha, y: bezel * fade_sq, z: bloom_boost / inv_coc_01 ratio
}

fn hash21_fast_crt(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;

    let res = max(audio.waveform_resolution, 128u);
    let history_size = max(audio.waveform_history_size, 2u);
    let max_hist_f = f32(history_size - 1u);

    // tex_coords.x goes from 0.0 to 1.0 (sample index)
    // tex_coords.y goes from 0.0 to 1.0 (history row: 0 is oldest, 1 is newest)
    let u_coord = in.tex_coords.x;
    let v_coord = in.tex_coords.y;

    // Integer history index locked to its buffer row while scrolling smoothly in Z via step_fraction
    let hist_idx = u32(round(clamp(v_coord * max_hist_f, 0.0, max_hist_f)));
    let row_offset = hist_idx * 2048u;
    let max_sample = i32(min(res, 2048u)) - 1i;
    let center_idx = clamp(i32(round(u_coord * f32(res - 1u))), 0i, max_sample);
    let center_val = waveform_history[row_offset + u32(center_idx)];
    let prev_val = waveform_history[row_offset + u32(max(center_idx - 1i, 0i))];
    let next_val = waveform_history[row_offset + u32(min(center_idx + 1i, max_sample))];
    var peak_val = select(prev_val, center_val, abs(center_val) >= abs(prev_val));
    peak_val = select(peak_val, next_val, abs(next_val) > abs(peak_val));
    let wave_val = mix(center_val, peak_val, 0.65);

    // Sub-row fractional Z glide eliminates stepped waterfall jumps between history pushes
    let v_scrolled = clamp(v_coord - audio.step_fraction / max_hist_f, 0.0, 1.0);

    // Taper spike height only at the very outer perimeter so the entire foreground
    // focal zone (v_coord in 0.75..0.96) displays full, undamped 3D wave amplitude
    let front_taper = smoothstep_r(0.998, 0.960, v_coord);
    let back_taper = smoothstep(0.0, 0.08, v_scrolled);
    let side_taper = smoothstep_r(0.5, 0.46, abs(u_coord - 0.5));
    let height_taper = front_taper * back_taper * side_taper;

    // Perceptual soft-knee gain so moderate musical/dialogue waves produce prominent 3D ridges
    // while full-scale 0 dBFS peaks saturate cleanly inside the CRT frame
    let abs_w = abs(wave_val);
    let boosted_w = sign(wave_val) * (abs_w * 1.65 / (1.0 + abs_w * 0.65));

    // Scale coordinates into 3D world coordinates
    let x = (u_coord - 0.5) * 19.2;
    let y = boosted_w * 2.25 * height_taper;
    let z = mix(9.48, -2.6, v_scrolled);

    let p3 = vec3<f32>(x, y, z);

    // Use smooth_time so seeking never snaps the camera
    let rot_angle = sin(audio.smooth_time * 0.2) * 0.15;
    let cam_dist_val = 4.6;
    let cam_height = 2.2;
    let ro = vec3<f32>(sin(rot_angle) * cam_dist_val, cam_height, -cos(rot_angle) * cam_dist_val);
    let cam_target = vec3<f32>(0.0, 0.0, 1.2);

    let f = normalize(cam_target - ro);
    let s = normalize(cross(f, vec3<f32>(0.0, 1.0, 0.0)));
    let u = cross(s, f);

    let view_matrix = mat4x4<f32>(
        vec4<f32>(s.x, u.x, -f.x, 0.0),
        vec4<f32>(s.y, u.y, -f.y, 0.0),
        vec4<f32>(s.z, u.z, -f.z, 0.0),
        vec4<f32>(-dot(s, ro), -dot(u, ro), dot(f, ro), 1.0)
    );

    let view_pos = view_matrix * vec4<f32>(p3, 1.0);
    let p_00 = 1.0 / (0.9 * max(audio.aspect_ratio, 0.1));
    let p_11 = 1.11111111;
    let p_22 = -1.0001;
    let p_32 = -0.10001;
    let clip_pos = vec4<f32>(
        view_pos.x * p_00,
        view_pos.y * p_11,
        view_pos.z * p_22 + p_32,
        -view_pos.z
    );

    // Apply barrel distortion in clip space (NDC) to match the curved CRT glass
    let ndc = clip_pos.xy / max(clip_pos.w, 0.0001);
    let r2 = dot(ndc, ndc);
    let distorted_ndc = ndc * (1.0 + r2 * 0.035);
    let final_clip_pos = vec4<f32>(distorted_ndc * clip_pos.w, clip_pos.z, clip_pos.w);

    out.clip_position = final_clip_pos;
    out.uv = vec2<f32>(u_coord, v_scrolled);
    out.local_z = v_coord;

    // Precompute smooth per-vertex optical/CRT lighting & DOF reciprocals to save 9 SFU ops per pixel
    let r = sqrt(dot(distorted_ndc, distorted_ndc));
    let bezel = smoothstep_r(1.3, 0.9, r);
    let cam_dist = length(p3 - ro);
    let coc = abs(cam_dist - 3.6) * 0.35;
    let total_coc = coc + r * r * 0.75;

    let hit_val = boosted_w * height_taper;
    let wave_height = clamp(abs(hit_val) * 1.85, 0.0, 1.0);
    let bloom_boost = 1.0 + wave_height * 1.15;

    out.coc_inv = vec4<f32>(
        1.0 / (1.0 + total_coc),
        1.0 / (1.0 + total_coc * 0.5),
        (1.0 / (1.0 + total_coc * 0.2)) * bloom_boost,
        (1.0 / (1.0 + total_coc * 0.1)) * bloom_boost
    );

    let v_mesh = max(v_scrolled, v_coord);
    let age_fade = mix(0.08, 1.0, v_scrolled);
    let edge_fade = side_taper;
    let depth_fade = smoothstep(0.0, 0.12, v_scrolled);
    let front_fade = smoothstep_r(0.998, 0.955, v_mesh);
    let boundary_fade = edge_fade * depth_fade * front_fade;
    let fade_sq = boundary_fade * boundary_fade;
    let bezel_fade_sq = bezel * fade_sq;

    let amber_ember = vec3<f32>(0.38, 0.07, 0.004);
    let amber_lo = mix(amber_ember, vec3<f32>(0.58, 0.17, 0.02), boundary_fade);
    let amber_hi = vec3<f32>(1.08, 0.62, 0.12);
    let amber_peak = vec3<f32>(1.45, 0.98, 0.42);
    let front_glow = smoothstep(0.76, 0.95, v_scrolled) * front_fade;
    let line_amber = mix(
        mix(amber_lo, amber_hi, wave_height),
        amber_peak,
        wave_height * wave_height * 0.72 + front_glow * 0.22
    );
    out.line_amber_scaled = line_amber * (age_fade * boundary_fade * bezel * 1.75);

    let norm = normalize(vec3<f32>(distorted_ndc * 0.18, 1.0));
    let light_source = vec3<f32>(-0.40406, 0.70711, -0.60609);
    let ndotl = max(dot(norm, light_source), 0.0);
    let nl2 = ndotl * ndotl;
    let nl4 = nl2 * nl2;
    let nl8 = nl4 * nl4;
    let nl16 = nl8 * nl8;
    let reflection_spec = (nl16 * nl16) * 0.08;
    let reflection_ambient = (1.0 - r * 0.4) * 0.015;
    let glass_reflection = vec3<f32>(0.85, 0.92, 1.0) * (reflection_spec + reflection_ambient);
    let phosphor_warmth = vec3<f32>(0.8, 0.35, 0.05) * (0.004 * bezel_fade_sq);
    out.glass_and_ambient = (vec3<f32>(0.02, 0.01, 0.003) + glass_reflection) * (bezel_fade_sq * bezel) + phosphor_warmth;
    out.fade_params = vec3<f32>(clamp(boundary_fade * bezel, 0.0, 1.0), bezel_fade_sq, 0.0);

    return out;
}

// Fast wireframe bloom function
fn get_wire(line_val: f32, inv_coc_05: f32, inv_coc_02_b: f32, inv_coc_01_b: f32) -> f32 {
    let core = 1.0 / (1.0 + line_val * line_val * 3.24);
    let glow1 = 0.45 / (1.0 + line_val * 0.15);
    let glow2 = 0.2 / (1.0 + line_val * 0.04);
    return (core * inv_coc_05) + (glow1 * inv_coc_02_b) + (glow2 * inv_coc_01_b);
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    // Match depth wireframe lines to the actual history row count so horizontal lines sit on vertex ridges
    let hist_lines = f32(max(audio.waveform_history_size, 72u) - 1u);
    let grid_res = vec2<f32>(128.0, hist_lines);

    // Lock horizontal wireframe lines to physical vertex rows (in.local_z = v_coord) so lines sit
    // directly on the 3D wave ridges and glide smoothly backward in Z with the mesh
    let uv_g = vec2<f32>(in.uv.x, in.local_z);
    let scaled_uv = uv_g * grid_res;
    let fwidth_uv = max(fwidth(scaled_uv), vec2<f32>(0.0005, 0.0005));

    let grid = abs(fract(scaled_uv - 0.5) - 0.5) / fwidth_uv;
    let line_dist = min(grid.x, grid.y) * in.coc_inv.x;
    let wire = get_wire(line_dist, in.coc_inv.y, in.coc_inv.z, in.coc_inv.w);

    let color = in.line_amber_scaled * wire + in.glass_and_ambient;
    let alpha = in.fade_params.x;
    let final_color = (color * (1.0 + color * 0.18) / (1.0 + color * 0.85)) * alpha;
    return vec4<f32>(final_color, alpha);
}
