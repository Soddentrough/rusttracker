// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(4)
var<storage, read> gpu_spectrum: array<vec2<f32>>;

fn get_resynthesized_wave(line_idx: u32, x_norm: f32) -> f32 {
    let point_idx = clamp(u32(x_norm * 511.0), 0u, 511u);
    let base_offset = 16u * 1024u;
    let flat_idx = line_idx * 512u + point_idx;
    return gpu_spectrum[base_offset + flat_idx].x;
}

fn project_3d(p3: vec3<f32>, ro: vec3<f32>, u: vec3<f32>, v_cam: vec3<f32>, w: vec3<f32>) -> vec3<f32> {
    let dir = p3 - ro;
    let dist_w = dot(dir, w);
    if dist_w <= 0.001 { return vec3<f32>(999.0, 999.0, dist_w); }
    let proj_x = dot(dir, u) / dist_w;
    let proj_y = dot(dir, v_cam) / dist_w;
    return vec3<f32>(proj_x, -proj_y, dist_w);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let crt_uv = in.uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    let distorted_uv = crt_uv * (1.0 + r2 * 0.04);

    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);

    let p = vec2<f32>(distorted_uv.x * aspect, distorted_uv.y) * 0.9;

    // Smoothly rotate camera around center using smooth_time
    let rot_angle = sin(audio.smooth_time * 0.2) * 0.15;
    let cam_dist = 3.2;
    let cam_height = 1.7;

    let ro = vec3<f32>(sin(rot_angle) * cam_dist, -cos(rot_angle) * cam_dist, cam_height);
    let cam_target = vec3<f32>(0.0, 0.8, 0.0);

    let w = normalize(cam_target - ro);
    let u = normalize(cross(w, vec3<f32>(0.0, 0.0, 1.0)));
    let v_cam = cross(u, w);

    let rd = normalize(w + p.x * u - p.y * v_cam);

    var accumulated_color = vec3<f32>(0.0);
    let amber_bass = vec3<f32>(0.72, 0.18, 0.02);
    let amber_mid  = vec3<f32>(1.00, 0.52, 0.06);
    let amber_high = vec3<f32>(1.30, 0.85, 0.30);

    let num_lines = 32u;
    let num_points = 512u;
    let r = sqrt(r2);
    let edge_blur = smoothstep(0.3, 1.2, r);
    let thickness = 0.002 + edge_blur * 0.004;

    // Render the 32 frequency planes back-to-front (i=0 is 20Hz sub-bass at back, i=31 is 10kHz treble at front)
    for (var i = 0u; i < num_lines; i = i + 1u) {
        let band_norm = f32(i) / 31.0;
        let y_line = mix(9.48, -1.8, band_norm);

        let t = (y_line - ro.y) / rd.y;
        if t <= 0.0 {
            continue;
        }

        let hit_x = ro.x + rd.x * t;
        let hit_z = ro.z + rd.z * t;

        // Spatial AABB early-out: wave X is in [-8, 8] and wave Z (height) is bounded by tanh * 1.2 within [-1.65, 1.65].
        // Skipping rays that miss the plane's active corridor avoids ~70% of storage buffer reads & 3D projections.
        if abs(hit_x) > 8.35 || abs(hit_z) > 1.75 {
            continue;
        }

        let float_idx = (hit_x + 8.0) / 16.0 * f32(num_points - 1u);
        let idx = i32(round(float_idx));

        let start_idx = max(0, idx - 4);
        let end_idx = min(i32(num_points) - 2, idx + 4);

        var min_dist = 1000.0;

        let j_start_u = u32(start_idx);
        let x0_init = mix(-8.0, 8.0, f32(j_start_u) / f32(num_points - 1u));
        var mask0 = smoothstep_r(8.0, 5.0, abs(x0_init));
        let x_norm0 = f32(j_start_u) / f32(num_points - 1u);
        var v0 = get_resynthesized_wave(i, x_norm0);
        var x0 = x0_init;
        var proj0 = project_3d(vec3<f32>(x0, y_line, v0 * mask0 * 1.2), ro, u, v_cam, w);

        for (var j = start_idx; j <= end_idx; j = j + 1) {
            let j_u = u32(j);
            let x_norm1 = f32(j_u + 1u) / f32(num_points - 1u);
            let x1 = mix(-8.0, 8.0, x_norm1);
            let mask1 = smoothstep_r(8.0, 5.0, abs(x1));
            let v1 = get_resynthesized_wave(i, x_norm1);

            let p3_1 = vec3<f32>(x1, y_line, v1 * mask1 * 1.2);
            let proj1 = project_3d(p3_1, ro, u, v_cam, w);

            if proj0.z > 0.001 && proj1.z > 0.001 {
                let d = sd_line_segment(p, proj0.xy, proj1.xy);
                min_dist = min(min_dist, d);
            }

            v0 = v1;
            x0 = x1;
            mask0 = mask1;
            proj0 = proj1;
        }

        let core = smoothstep_r(thickness, 0.0, min_dist);
        let bloom = 0.0004 / (min_dist * min_dist + 0.0001) * 0.15;

        let depth_fade = exp(-t * 0.19);
        let edge_fade = smoothstep_r(8.0, 5.0, abs(hit_x));

        let hit_x_norm = clamp((hit_x + 8.0) / 16.0, 0.0, 1.0);
        let wave_height = clamp(abs(get_resynthesized_wave(i, hit_x_norm)) * 2.0, 0.0, 1.0);
        let base_band_col = mix(amber_bass, amber_mid, band_norm * 0.75);
        let line_amber = mix(base_band_col, amber_high, wave_height * 0.75 + band_norm * 0.25);

        accumulated_color += line_amber * (core + bloom) * depth_fade * edge_fade;
    }

    var final_color = aces_tonemap(accumulated_color);

    let scanline = 0.85 + 0.15 * cos(in.clip_position.y * 3.14159);
    final_color *= scanline;

    let bezel = smoothstep_r(1.3, 0.9, r);

    let noise_val = hash21_crt(in.clip_position.xy + fract(audio.smooth_time) * 100.0);
    let acc_lum = dot(accumulated_color, vec3<f32>(0.299, 0.587, 0.114));
    let noise_color = vec3<f32>(0.8, 0.35, 0.05) * noise_val * 0.02 * bezel * (0.3 + 0.7 * clamp(acc_lum * 0.5, 0.0, 1.0));

    final_color = final_color * bezel + noise_color;

    return vec4<f32>(final_color, 1.0);
}
