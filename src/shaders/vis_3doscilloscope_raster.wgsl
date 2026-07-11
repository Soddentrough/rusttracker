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
    @location(1) world_pos: vec3<f32>,
    @location(2) hit_val: f32,
    @location(3) ndc_pos: vec3<f32>,
    @location(4) local_z: f32,
    @location(5) cam_dist: f32,
}

// Hash function for analog noise
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;

    let res = max(audio.waveform_resolution, 128u);
    let history_size = max(audio.waveform_history_size, 1u);

    // tex_coords.x goes from 0.0 to 1.0 (sample index)
    // tex_coords.y goes from 0.0 to 1.0 (history row: 0 is oldest, 1 is newest)
    let u_coord = in.tex_coords.x;
    let v_coord = in.tex_coords.y;

    // Physical Z-scroll: slide the entire grid smoothly in Z by step_fraction,
    // while reading waveform data from integer-snapped history slots.
    // This avoids interpolating between two different waveform shapes (which
    // causes visible morphing/wobble) and instead keeps each row's shape
    // perfectly stable as it scrolls away from the camera.
    let row_spacing = 10.8 / f32(max(history_size - 1u, 1u)); // Z range / rows
    let z_shift = audio.step_fraction * row_spacing;

    // Integer history index — no fractional interpolation
    let hist_idx = u32(clamp(v_coord * f32(history_size - 1u), 0.0, f32(history_size - 1u)));

    let sample_idx = u32(u_coord * f32(res - 1u));

    // Read waveform amplitude from a single, stable history slot
    let wave_val = waveform_history[hist_idx * 2048u + sample_idx];

    // Scale coordinates into 3D world coordinates
    // X goes from -4.0 to 4.0
    // Y (height/UP) is wave_val * 0.8
    // Z (depth/FORWARD) goes from 9.0 (oldest/back) to -1.8 (newest/front),
    //   offset by z_shift for smooth sub-frame physical scrolling
    let x = (u_coord - 0.5) * 8.0;
    let y = wave_val * 0.8;
    let z = mix(9.0, -1.8, v_coord) - z_shift;

    let p3 = vec3<f32>(x, y, z);

    // Camera rotation matches the original 3D CRT Oscilloscope
    let rot_angle = sin(audio.time * 0.2) * 0.15;
    let cam_dist_val = 3.2;
    let cam_height = 1.7; // Adjusted camera height to match others
    let ro = vec3<f32>(sin(rot_angle) * cam_dist_val, cam_height, -cos(rot_angle) * cam_dist_val);
    let cam_target = vec3<f32>(0.0, 0.0, 0.8); // Target adjusted to match others (Depth Z = 0.8, Height Y = 0.0)

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
    // Custom projection matrix to match the 116 degree horizontal / 84 degree vertical FOV of the raymarched versions
    let p_00 = 1.0 / (0.9 * audio.aspect_ratio);
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

    out.world_pos = p3;
    out.clip_position = final_clip_pos;
    out.ndc_pos = vec3<f32>(distorted_ndc, clip_pos.z / max(clip_pos.w, 0.0001));
    out.uv = in.tex_coords;
    out.local_z = y;
    out.hit_val = wave_val;
    out.cam_dist = length(p3 - ro);

    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let crt_uv = in.ndc_pos.xy;
    let r = length(crt_uv);
    let bezel = 1.0 - smoothstep(0.9, 1.3, r);

    // Depth of field (DOF) and edge-based electron beam defocusing
    let dist = in.cam_dist;
    let focus_dist = 3.6;
    let coc = abs(dist - focus_dist) * 0.35; // Circle of Confusion radius in pixels
    let edge_defocus = r * r * 0.75;          // Beam defocuses towards screen edges
    let total_coc = coc + edge_defocus;

    // UV derivative-based wireframe grid shader
    // Width has 128 lines, depth has 72 lines
    let grid_res = vec2<f32>(128.0, 72.0);
    
    // Use the continuous world-space Z for the vertical grid coordinate to eliminate temporal jitter
    let uv_g = vec2<f32>(in.uv.x, (in.world_pos.z + 1.8) / 10.8);
    
    // Amplitude-reactive bloom width and brightness
    let wave_height = clamp(abs(in.hit_val) * 2.0, 0.0, 1.0);
    let bloom_boost = 1.0 + wave_height * 0.8;

    // Red/blue chromatic aberration coordinate offsets
    let offset_r = vec2<f32>(0.0005, 0.0003);
    let offset_b = vec2<f32>(-0.0005, -0.0003);

    let fwidth_uv = fwidth(uv_g * grid_res) + 0.0001;

    // Red Channel Grid
    let uv_r = uv_g + offset_r;
    let grid_r = abs(fract(uv_r * grid_res - 0.5) - 0.5) / fwidth_uv;
    let line_r = min(grid_r.x, grid_r.y) / (1.0 + total_coc);
    let wire_r = exp(-line_r * 1.8) / (1.0 + total_coc * 0.5) +
                 (exp(-line_r * 0.15) * 0.45 / (1.0 + total_coc * 0.2) + exp(-line_r * 0.04) * 0.2 / (1.0 + total_coc * 0.1)) * bloom_boost;

    // Green/Amber Channel Grid (center)
    let grid_g = abs(fract(uv_g * grid_res - 0.5) - 0.5) / fwidth_uv;
    let line_g = min(grid_g.x, grid_g.y) / (1.0 + total_coc);
    let wire_g = exp(-line_g * 1.8) / (1.0 + total_coc * 0.5) +
                 (exp(-line_g * 0.15) * 0.45 / (1.0 + total_coc * 0.2) + exp(-line_g * 0.04) * 0.2 / (1.0 + total_coc * 0.1)) * bloom_boost;

    // Blue Channel Grid
    let uv_b = uv_g + offset_b;
    let grid_b = abs(fract(uv_b * grid_res - 0.5) - 0.5) / fwidth_uv;
    let line_b = min(grid_b.x, grid_b.y) / (1.0 + total_coc);
    let wire_b = exp(-line_b * 1.8) / (1.0 + total_coc * 0.5) +
                 (exp(-line_b * 0.15) * 0.45 / (1.0 + total_coc * 0.2) + exp(-line_b * 0.04) * 0.2 / (1.0 + total_coc * 0.1)) * bloom_boost;

    // Combine channels to get a shifted sub-pixel wireframe vector
    let base_wire = vec3<f32>(wire_r, wire_g, wire_b);

    // Warm amber phosphor color palette
    let amber_lo = vec3<f32>(0.6, 0.18, 0.02);
    let amber_hi = vec3<f32>(1.0, 0.55, 0.08);
    let line_amber = mix(amber_lo, amber_hi, wave_height);

    // Age fade (oldest frames fade out)
    let age_fade = mix(0.08, 1.0, in.uv.y);

    // Edge fade (fade out grid at left/right boundaries)
    let edge_fade = 1.0 - smoothstep(0.35, 0.5, abs(in.uv.x - 0.5));

    // Smooth vignette boundary fade to transition seamlessly into the black background
    let depth_fade = smoothstep(0.0, 0.20, in.uv.y);
    let front_fade = 1.0 - smoothstep(0.80, 1.0, in.uv.y);
    let boundary_fade = edge_fade * depth_fade * front_fade;

    // Base color with glowing wireframe (and chromatic aberration)
    var color = line_amber * base_wire * age_fade * boundary_fade * 1.6;

    // Ambient CRT screen glow (phosphor background glow)
    let ambient_glow = vec3<f32>(0.02, 0.01, 0.003) * bezel * boundary_fade;
    color += ambient_glow;

    // Scanlines (wider 3-pixel period scanlines, reactive to brightness)
    let brightness = dot(color, vec3<f32>(0.299, 0.587, 0.114));
    let scanline_strength = mix(0.24, 0.03, clamp(brightness * 2.0, 0.0, 1.0));
    let scanline = 1.0 - scanline_strength + scanline_strength * cos(in.clip_position.y * (3.14159 / 1.5));
    color *= scanline;

    // Aperture grille sub-pixel phosphor mask
    let mask_x = i32(in.clip_position.x) % 3;
    var phosphor_mask = vec3<f32>(0.75, 0.75, 0.75);
    if (mask_x == 0) {
        phosphor_mask = vec3<f32>(1.2, 0.8, 0.8);
    } else if (mask_x == 1) {
        phosphor_mask = vec3<f32>(0.8, 1.2, 0.8);
    } else {
        phosphor_mask = vec3<f32>(0.8, 0.8, 1.2);
    }
    color *= phosphor_mask;

    // Realistic CRT glass reflection overlay (added after scanlines/phosphor mask)
    let norm = normalize(vec3<f32>(crt_uv * 0.18, 1.0));
    let light_source = normalize(vec3<f32>(-0.4, 0.7, -0.6));
    let reflection_spec = pow(max(dot(norm, light_source), 0.0), 32.0) * 0.08;
    let reflection_ambient = (1.0 - r * 0.4) * 0.015;
    let glass_reflection = vec3<f32>(0.85, 0.92, 1.0) * (reflection_spec + reflection_ambient) * bezel;
    color += glass_reflection;

    // Bezel fade
    color *= bezel;

    // Analog noise (vignetted by bezel and boundary_fade)
    let noise_val = hash12(in.clip_position.xy + fract(f32(audio.frame_count) * 0.073) * 100.0);
    let noise_color = vec3<f32>(0.8, 0.35, 0.05) * noise_val * 0.025 * bezel * boundary_fade;
    color += noise_color;

    // Tonemapping
    let final_color = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);

    return vec4<f32>(final_color, 1.0);
}
