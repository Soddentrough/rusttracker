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
    // Place Left channel on the left side and Right channel on the right side of the same joined line
    let lr_mix = smoothstep(0.28, 0.72, u_coord);
    let w = mix(pair.x, pair.y, lr_mix);
    // Smooth window taper only at the far left and far right bezel edges
    let taper = smoothstep(0.0, 0.09, u_coord) * smoothstep(0.0, 0.09, 1.0 - u_coord);
    return w * taper;
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

    // Terraced 3D stage: deep sub-bass at the back (band_norm = 0.0), high treble at the front (band_norm = 1.0)
    let y_stage = mix(4.85, -0.35, pow(band_norm, 0.84));
    let amp_scale = mix(1.45, 0.58, pow(band_norm, 0.75));
    let z = mix(9.4, -1.35, band_norm);

    // Single joined mesh stretching continuously across the visible field from left to right
    let x_span = mix(17.20, 4.80, band_norm);
    let x_cur  = mix(-x_span, x_span, u_coord);
    let dx_step = (2.0 * x_span) * du;

    let y_crest = y_stage + wave_val * amp_scale;
    let p_base  = vec3<f32>(x_cur, y_stage, z);
    let p_cur   = vec3<f32>(x_cur, y_crest, z);

    // Smoothly rotate camera around center using seek-monotonic smooth_time
    let rot_angle = sin(audio.smooth_time * 0.2) * 0.14;
    let cam_dist_val = 4.00;
    let cam_height = 2.20;
    let ro = vec3<f32>(sin(rot_angle) * cam_dist_val, cam_height, -cos(rot_angle) * cam_dist_val);
    let cam_target = vec3<f32>(0.0, 0.20, 2.0);

    let f = normalize(cam_target - ro);
    let s = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), f));
    let u = cross(f, s);

    let view_matrix = mat4x4<f32>(
        vec4<f32>(s.x, u.x, -f.x, 0.0),
        vec4<f32>(s.y, u.y, -f.y, 0.0),
        vec4<f32>(s.z, u.z, -f.z, 0.0),
        vec4<f32>(-dot(s, ro), -dot(u, ro), dot(f, ro), 1.0)
    );

    let aspect = max(audio.aspect_ratio, 0.1);
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
    let tang = normalize(vec2<f32>(max(dx_screen, abs(dy_screen) * 0.55), dy_screen));
    let norm_unit = vec2<f32>(-tang.y, tang.x);
    let norm_ndc = vec2<f32>(norm_unit.x / aspect, norm_unit.y);

    // Crisp CRT line ribbon half-width in NDC
    let r_crest = length(ndc_cur);
    let depth_scale = mix(0.76, 1.15, band_norm);
    let beam_half_ndc = 0.0092 * depth_scale * (1.0 + smoothstep(0.3, 1.15, r_crest) * 0.38);

    // signed_d: -1.0 at top edge (v = 0.0), 0.0 at exact line center (v = 0.5), +1.0 at bottom edge (v = 1.0)
    let signed_d = v_coord * 2.0 - 1.0;
    let ndc_pos_2d = ndc_cur - norm_ndc * (signed_d * beam_half_ndc);
    let clip_pos = vec4<f32>(ndc_pos_2d * w_cur, v_base.z * p_22 + p_32, w_cur);

    // Apply CRT barrel distortion in clip space
    let ndc = clip_pos.xy / max(clip_pos.w, 0.0001);
    let r2 = dot(ndc, ndc);
    let distorted_ndc = ndc * (1.0 + r2 * 0.038);
    let final_clip_pos = vec4<f32>(distorted_ndc * clip_pos.w, clip_pos.z, clip_pos.w);

    out.world_pos = p_cur;
    out.clip_position = final_clip_pos;
    out.ndc_pos = vec3<f32>(distorted_ndc, u_coord);
    out.uv = vec2<f32>(u_coord, v_coord);
    out.band_norm = band_norm;
    out.hit_val = wave_val;
    out.screen_dist = signed_d;

    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let crt_uv = in.ndc_pos.xy;
    let r = length(crt_uv);

    if abs(crt_uv.x) > 1.05 || abs(crt_uv.y) > 1.05 {
        discard;
    }

    let bezel = smoothstep_r(1.28, 0.88, r);
    let u_coord = in.uv.x;
    let band_norm = in.band_norm;
    let d = clamp(abs(in.screen_dist), 0.0, 1.0);
    let lr_mix = smoothstep(0.28, 0.72, u_coord);

    // Distinct frequency-depth & stereo phosphor palette (unclipped in Rgba8Unorm so hue is preserved):
    // Back (sub-bass 20Hz): deep ruby-copper ember
    // Middle (mids): rich warm amber-orange
    // Front (treble 10kHz): bright golden-amber with smooth Left-to-Right stereo tint across the joined line
    let wave_height = clamp(abs(in.hit_val) * 1.6, 0.0, 1.0);
    let bass_col = mix(vec3<f32>(0.82, 0.14, 0.02), vec3<f32>(0.72, 0.22, 0.04), lr_mix);
    let mid_col  = mix(vec3<f32>(0.98, 0.38, 0.05), vec3<f32>(0.92, 0.50, 0.09), lr_mix);
    let high_col = mix(vec3<f32>(1.00, 0.68, 0.20), vec3<f32>(0.96, 0.80, 0.32), lr_mix);

    let band_col = mix(
        mix(bass_col, mid_col, smoothstep(0.0, 0.55, band_norm)),
        high_col,
        smoothstep(0.45, 1.0, band_norm)
    );
    let line_col = mix(band_col, high_col, wave_height * 0.38);

    // Distance & outer bezel edge attenuation (continuous across the center, fades only at far left/right edges)
    let depth_fade = mix(0.42, 0.96, pow(band_norm, 0.65));
    let end_fade = smoothstep(0.0, 0.05, u_coord) * smoothstep(0.0, 0.05, 1.0 - u_coord);
    let boundary_fade = depth_fade * end_fade;

    // Anti-aliased CRT beam core (exp(-d*d*28)) + soft phosphor halo reaching strictly 0.0 at d = 1.0
    let edge_blur = 1.0 + smoothstep(0.3, 1.15, r) * 0.55;
    let edge_zero = (1.0 - d) * (1.0 - d);
    let core = exp(-d * d * (28.0 / (edge_blur * edge_blur))) * edge_zero;
    let halo = exp(-d * 3.2) * edge_zero * (0.20 + wave_height * 0.24);

    let core_hot = vec3<f32>(1.00, 0.92, 0.68) * (core * core * (0.22 + wave_height * 0.38));
    var color = (line_col * (core * 0.82 + halo) + core_hot) * boundary_fade;

    let scanline = 0.88 + 0.12 * cos(in.clip_position.y * 3.14159);
    color *= scanline * bezel;

    return vec4<f32>(color, 1.0);
}
