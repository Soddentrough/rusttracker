// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(4) var<storage, read> gpu_spectrum: array<vec2<f32>>;
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
    @location(4) band_norm: f32,
    @location(5) screen_dist: f32,
}

fn get_resynthesized_wave_pair(line_idx: u32, point_idx: u32) -> vec2<f32> {
    let base_offset = 16u * 1024u;
    let flat_idx = min(line_idx, 31u) * 512u + min(point_idx, 511u);
    return gpu_spectrum[base_offset + flat_idx];
}

fn sample_joined_wave(line_idx: u32, u_coord: f32) -> f32 {
    let pt_f = clamp(u_coord * 511.0, 0.0, 511.0);
    let i0 = u32(floor(pt_f));
    let i1 = min(i0 + 1u, 511u);
    let frac = fract(pt_f);
    let pair = mix(
        get_resynthesized_wave_pair(line_idx, i0),
        get_resynthesized_wave_pair(line_idx, i1),
        frac
    );
    // Single joined horizontal line across the full display width:
    // Left channel (pair.x) on the left half, Right channel (pair.y) on the right half,
    // joined cleanly across the center without opposite-slope kinks or a wide flat trough.
    let env_left  = smoothstep(0.0, 0.06, u_coord) * (1.0 - smoothstep(0.42, 0.495, u_coord));
    let env_right = smoothstep(0.505, 0.58, u_coord) * smoothstep(0.0, 0.06, 1.0 - u_coord);
    return pair.x * env_left + pair.y * env_right;
}

@vertex
fn vs_main_3d(in: VertexInput, @builtin(instance_index) instance_idx: u32) -> VertexOutput3D {
    var out: VertexOutput3D;

    // 32 continuous full-width lines: 0 = 20Hz sub-bass at back .. 31 = 10kHz treble at front
    let line_idx = min(instance_idx, 31u);
    let band_norm = f32(line_idx) / 31.0;

    let u_coord = in.tex_coords.x;
    let v_coord = in.tex_coords.y; // 0.0 = top edge of line ribbon, 1.0 = bottom edge of line ribbon

    let du = 1.0 / 511.0;
    let wave_val  = sample_joined_wave(line_idx, u_coord);
    let wave_prev = sample_joined_wave(line_idx, max(u_coord - du, 0.0));
    let wave_next = sample_joined_wave(line_idx, min(u_coord + du, 1.0));

    let aspect = max(audio.aspect_ratio, 0.1);

    // Rational perspective-compensated Z parameter on a true flat 3D plane:
    // Interpolating z, y_stage, and x_span with the exact same z_t keeps the 3D stage 100% planar
    // with straight perspective side borders and even vertical spacing from back to front.
    let z_t = (band_norm * 2.2) / (1.0 + band_norm * 1.2);
    let z = mix(7.2, -2.30, z_t);
    let y_stage = mix(1.05, -0.34, z_t);

    // Scale world-space wave height with depth so active bands reach strong physical height
    let amp_scale = mix(1.42, 0.36, z_t);

    // Scale world X span linearly with z_t and aspect ratio so every row covers the full display width symmetrically
    let x_span = mix(8.85, 2.65, z_t) * aspect;
    let x_cur  = mix(-x_span, x_span, u_coord);
    let dx_step = (2.0 * x_span) * du;

    let y_crest = y_stage + wave_val * amp_scale;
    let p_base  = vec3<f32>(x_cur, y_stage, z);
    let p_cur   = vec3<f32>(x_cur, y_crest, z);

    // Shallow camera angle matching 3D Oscilloscope, centered horizontally so full-width lines stay level
    let cam_dist_val = 4.6;
    let cam_height = 2.2;
    let ro = vec3<f32>(0.0, cam_height, -cam_dist_val);
    let cam_target = vec3<f32>(0.0, 0.0, 1.2);

    let f = normalize(cam_target - ro);
    let s = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), f));
    let u = cross(f, s);

    let view_matrix = mat4x4<f32>(
        vec4<f32>(s.x, u.x, -f.x, 0.0),
        vec4<f32>(s.y, u.y, -f.y, 0.0),
        vec4<f32>(s.z, u.z, -f.z, 0.0),
        vec4<f32>(-dot(s, ro), -dot(u, ro), dot(f, ro), 1.0)
    );

    let p_00 = 1.0 / (0.9 * aspect);
    let p_11 = 1.11111111;
    let p_22 = -1.0001;
    let p_32 = -0.10001;

    let cam_keep = camera.view_matrix[0][0] * 0.0;

    // Evaluate perspective depth w_cur from the band baseline p_base so vertical wave excursions
    // never skew ndc_cur.x horizontally near the left and right edges of the frustum
    let v_base = view_matrix * vec4<f32>(p_base, 1.0);
    let v_cur  = view_matrix * vec4<f32>(p_cur,  1.0);
    let w_cur  = max(-v_base.z, 0.001);
    let ndc_cur = vec2<f32>(v_base.x * p_00 + cam_keep, v_cur.y * p_11) / w_cur;

    // Screen-space tangent & upward normal at constant depth w_cur
    let dx_screen = max((2.0 * dx_step * s.x * p_00 / w_cur) * aspect, 0.001);
    let dy_screen = ((wave_next - wave_prev) * amp_scale * u.y * p_11) / w_cur;
    let tang = normalize(vec2<f32>(max(dx_screen, abs(dy_screen) * 0.65), dy_screen));
    let norm_unit = vec2<f32>(-tang.y, tang.x);
    let norm_ndc = vec2<f32>(norm_unit.x / aspect, norm_unit.y);

    // Multi-pixel NDC ribbon width: thick enough at the back to prevent scanline shimmering,
    // and clean at the front so high-frequency peaks stay crisp
    let beam_half_ndc = mix(0.0115, 0.0095, band_norm);

    // signed_d: -1.0 at top edge (v = 0.0), 0.0 at exact line center (v = 0.5), +1.0 at bottom edge (v = 1.0)
    let signed_d = v_coord * 2.0 - 1.0;
    let ndc_pos_2d = ndc_cur - norm_ndc * (signed_d * beam_half_ndc);
    let clip_pos = vec4<f32>(ndc_pos_2d * w_cur, v_base.z * p_22 + p_32, w_cur);

    // Keep straight unbent projection (no barrel distortion bowing the horizontal lines)
    let ndc = clip_pos.xy / max(clip_pos.w, 0.0001);

    out.world_pos = p_cur;
    out.clip_position = clip_pos;
    out.ndc_pos = vec3<f32>(ndc, u_coord);
    out.uv = vec2<f32>(u_coord, v_coord);
    out.band_norm = band_norm;
    out.hit_val = wave_val;
    out.screen_dist = signed_d;

    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let crt_uv = in.ndc_pos.xy;

    if abs(crt_uv.x) > 1.05 || abs(crt_uv.y) > 1.05 {
        discard;
    }

    let edge_box = max(abs(crt_uv.x), abs(crt_uv.y));
    let bezel = smoothstep_r(1.04, 0.94, edge_box);
    let u_coord = in.uv.x;
    let band_norm = in.band_norm;
    let d = clamp(abs(in.screen_dist), 0.0, 1.0);
    let lr_mix = smoothstep(0.35, 0.65, u_coord);

    // Distinct frequency-depth & stereo phosphor palette:
    // Back (sub-bass 20Hz): deep ruby-copper ember
    // Middle (mids): rich warm amber-orange
    // Front (treble 10kHz): bright golden-amber with subtle Left-to-Right stereo tint across the joined line
    let wave_height = clamp(abs(in.hit_val) * 1.35, 0.0, 1.0);
    let bass_col = mix(vec3<f32>(0.86, 0.18, 0.03), vec3<f32>(0.76, 0.26, 0.05), lr_mix);
    let mid_col  = mix(vec3<f32>(0.98, 0.42, 0.06), vec3<f32>(0.92, 0.54, 0.10), lr_mix);
    let high_col = mix(vec3<f32>(1.00, 0.70, 0.20), vec3<f32>(0.96, 0.82, 0.32), lr_mix);

    let band_col = mix(
        mix(bass_col, mid_col, smoothstep(0.0, 0.55, band_norm)),
        high_col,
        smoothstep(0.45, 1.0, band_norm)
    );
    let line_col = mix(band_col, high_col, wave_height * 0.40);

    // Calm reference glow on flat baselines (~0.36) surging to full incandescent brightness on active peaks
    let activity_gain = mix(0.36, 1.05, smoothstep(0.0, 0.50, wave_height));
    let depth_fade = mix(0.68, 0.98, pow(band_norm, 0.55));
    let end_fade = smoothstep(0.0, 0.02, u_coord) * smoothstep(0.0, 0.02, 1.0 - u_coord);
    let boundary_fade = depth_fade * end_fade * activity_gain;

    // Multi-pixel anti-aliased CRT beam core (broader at the back to prevent shimmering, crisp at the front)
    let core_sharpness = mix(9.5, 16.0, band_norm);
    let edge_zero = (1.0 - d) * (1.0 - d);
    let core = exp(-d * d * core_sharpness) * edge_zero;
    let halo = exp(-d * 4.2) * edge_zero * (0.10 + wave_height * 0.16);

    let core_hot = vec3<f32>(1.00, 0.92, 0.65) * (core * core * (0.12 + wave_height * 0.36));
    var color = (line_col * (core * 0.74 + halo) + core_hot) * boundary_fade * bezel;
    color = color / (vec3<f32>(1.0) + color * 0.22);

    return vec4<f32>(color, 1.0);
}
