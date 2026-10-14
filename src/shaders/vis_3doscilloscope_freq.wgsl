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

fn get_resynthesized_wave_pt(line_idx: u32, point_idx: u32) -> f32 {
    let base_offset = 16u * 1024u;
    let flat_idx = min(line_idx, 31u) * 512u + min(point_idx, 511u);
    return gpu_spectrum[base_offset + flat_idx].x;
}

// Smoothly sample and filter the resynthesized wave for a given band and point index
fn sample_band_wave(line_idx: u32, idx: i32) -> f32 {
    let i0 = u32(clamp(idx - 1, 0, 511));
    let i1 = u32(clamp(idx,     0, 511));
    let i2 = u32(clamp(idx + 1, 0, 511));
    let w = get_resynthesized_wave_pt(line_idx, i0) * 0.22
          + get_resynthesized_wave_pt(line_idx, i1) * 0.56
          + get_resynthesized_wave_pt(line_idx, i2) * 0.22;
    let u = f32(i1) / 511.0;
    let edge_taper = smoothstep_r(0.50, 0.3125, abs(u - 0.5));
    return w * edge_taper;
}

@vertex
fn vs_main_3d(in: VertexInput, @builtin(instance_index) instance_idx: u32) -> VertexOutput3D {
    var out: VertexOutput3D;

    // 32 frequency bands: instance 0 = 20Hz sub-bass at back, instance 31 = 10kHz treble at front
    let line_idx = min(instance_idx, 31u);
    let band_norm = f32(line_idx) / 31.0;

    // tex_coords.x: 0.0..1.0 across 512 resynthesized wave points (0..511)
    // tex_coords.y: 0.0, 0.5, 1.0 across the 3-row 3D topographical ribbon strip:
    //   Row 0 (v = 0.0): top edge of the CRT electron beam (+norm_screen * beam_half_ndc, screen_dist = -1.0)
    //   Row 1 (v = 0.5): exact 3D wave crest (ndc_cur, screen_dist = 0.0)
    //   Row 2 (v = 1.0): bottom of the 3D topographical occlusion curtain (ndc_floor, screen_dist = +curtain_radii)
    let u_coord = in.tex_coords.x;
    let v_coord = in.tex_coords.y;

    let pt_idx = i32(round(clamp(u_coord * 511.0, 0.0, 511.0)));
    let wave_val = sample_band_wave(line_idx, pt_idx);
    let wave_prev = sample_band_wave(line_idx, max(pt_idx - 1, 0));
    let wave_next = sample_band_wave(line_idx, min(pt_idx + 1, 511));

    // Terraced 3D amphitheater stage spanning the CRT frame vertically from bottom-front to top-back
    let y_stage = mix(4.85, -0.05, pow(band_norm, 0.82));
    let amp_scale = mix(1.65, 0.90, band_norm);
    let z = mix(9.6, -1.65, band_norm);

    // Taper width slightly in world space so rear bands frame cleanly inside the CRT bezel
    let width_x = mix(21.0, 15.0, band_norm);
    let x_cur = (u_coord - 0.5) * width_x;
    let y_crest = y_stage + wave_val * amp_scale;

    let dx_step = width_x / 511.0;
    let p_prev = vec3<f32>(x_cur - dx_step, y_stage + wave_prev * amp_scale, z);
    let p_cur  = vec3<f32>(x_cur,           y_crest,                         z);
    let p_next = vec3<f32>(x_cur + dx_step, y_stage + wave_next * amp_scale, z);

    // Smoothly rotate camera around center using seek-monotonic smooth_time
    let rot_angle = sin(audio.smooth_time * 0.2) * 0.14;
    let cam_dist_val = 4.00;
    let cam_height = 2.20;
    let ro = vec3<f32>(sin(rot_angle) * cam_dist_val, cam_height, -cos(rot_angle) * cam_dist_val);
    let cam_target = vec3<f32>(0.0, 0.20, 2.0);

    let f = normalize(cam_target - ro);
    let s = normalize(cross(f, vec3<f32>(0.0, 1.0, 0.0)));
    let u = cross(s, f);

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

    // Project previous, current, and next crest points to compute exact 2D screen-space normal
    let v_prev = view_matrix * vec4<f32>(p_prev, 1.0);
    let v_cur  = view_matrix * vec4<f32>(p_cur,  1.0);
    let v_next = view_matrix * vec4<f32>(p_next, 1.0);

    let w_prev = max(-v_prev.z, 0.001);
    let w_cur  = max(-v_cur.z,  0.001);
    let w_next = max(-v_next.z, 0.001);

    let ndc_prev = vec2<f32>(v_prev.x * p_00, v_prev.y * p_11) / w_prev;
    let ndc_cur  = vec2<f32>(v_cur.x  * p_00 + cam_keep, v_cur.y  * p_11) / w_cur;
    let ndc_next = vec2<f32>(v_next.x * p_00, v_next.y * p_11) / w_next;

    // Screen-space tangent & upward unit normal (aspect-corrected so beam width is isotropic at any slope)
    let delta_screen = (ndc_next - ndc_prev) * vec2<f32>(aspect, 1.0);
    let tang = delta_screen / max(length(delta_screen), 0.00001);
    var norm_unit = vec2<f32>(-tang.y, tang.x);
    if norm_unit.y < 0.0 {
        norm_unit = -norm_unit;
    }
    let norm_ndc = vec2<f32>(norm_unit.x / aspect, norm_unit.y);

    // Electron-beam half-width in screen NDC units (scales subtly with CRT corner defocus)
    let r_crest = length(ndc_cur);
    let beam_half_ndc = 0.0048 * (1.0 + smoothstep(0.25, 1.15, r_crest) * 0.45);

    var world_p = p_cur;
    var clip_pos: vec4<f32>;
    var screen_dist = 0.0;

    if v_coord < 0.25 {
        // Row 0 (v = 0.0): top edge of the glowing CRT electron beam (1 beam radius above crest)
        let ndc_top = ndc_cur + norm_ndc * beam_half_ndc;
        clip_pos = vec4<f32>(ndc_top * w_cur, v_cur.z * p_22 + p_32, w_cur);
        screen_dist = -1.0;
    } else if v_coord < 0.75 {
        // Row 1 (v = 0.5): exact 3D wave crest (center of the electron beam)
        clip_pos = vec4<f32>(ndc_cur * w_cur, v_cur.z * p_22 + p_32, w_cur);
        screen_dist = 0.0;
    } else {
        // Row 2 (v = 1.0): bottom of the 3D topographical occlusion curtain
        let y_floor = y_stage - 1.10;
        world_p = vec3<f32>(x_cur, min(y_crest - 0.06, y_floor), z);
        let v_floor = view_matrix * vec4<f32>(world_p, 1.0);
        let w_floor = max(-v_floor.z, 0.001);
        let ndc_floor = vec2<f32>(v_floor.x * p_00, v_floor.y * p_11) / w_floor;
        clip_pos = vec4<f32>(ndc_floor * w_floor, v_floor.z * p_22 + p_32, w_floor);

        // Measure perpendicular screen distance in beam-radii so the lower beam core & glow match the upper half C1-smoothly
        let floor_delta = (ndc_cur - ndc_floor) * vec2<f32>(aspect, 1.0);
        let perp_dist = max(dot(floor_delta, norm_unit), max(ndc_cur.y - ndc_floor.y, 0.025));
        screen_dist = perp_dist / beam_half_ndc;
    }

    // Apply CRT barrel distortion in clip space
    let ndc = clip_pos.xy / max(clip_pos.w, 0.0001);
    let r2 = dot(ndc, ndc);
    let distorted_ndc = ndc * (1.0 + r2 * 0.038);
    let final_clip_pos = vec4<f32>(distorted_ndc * clip_pos.w, clip_pos.z, clip_pos.w);

    out.world_pos = world_p;
    out.clip_position = final_clip_pos;
    out.ndc_pos = vec3<f32>(distorted_ndc, clip_pos.z / max(clip_pos.w, 0.0001));
    out.uv = vec2<f32>(u_coord, v_coord);
    out.band_norm = band_norm;
    out.hit_val = wave_val;
    out.screen_dist = screen_dist;

    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let crt_uv = in.ndc_pos.xy;
    let r = length(crt_uv);

    let u_coord = in.uv.x;
    let band_norm = in.band_norm;
    let s_dist = in.screen_dist;

    // Compute screen-space derivatives in uniform control flow BEFORE any discard
    let rib_freq = 96.0;
    let fw_x = max(fwidth(u_coord * rib_freq), 0.001);
    let rib_grid = abs(fract(u_coord * rib_freq - 0.5) - 0.5) / fw_x;

    if abs(crt_uv.x) > 1.05 || abs(crt_uv.y) > 1.05 {
        discard;
    }
    // Discard the sub-pixel outer fringe on the top lip so it never overwrites rear traces with dark pixels
    if s_dist < -0.78 {
        discard;
    }

    let bezel = smoothstep_r(1.28, 0.88, r);

    // Warm CRT amber-to-cream phosphor palette across frequency bands and wave peaks
    let wave_height = clamp(abs(in.hit_val) * 2.1, 0.0, 1.0);
    let amber_bass = vec3<f32>(0.76, 0.20, 0.02);
    let amber_mid  = vec3<f32>(1.08, 0.55, 0.07);
    let amber_high = vec3<f32>(1.42, 0.92, 0.36);
    let base_band_col = mix(amber_bass, amber_mid, band_norm * 0.78);
    let line_amber = mix(base_band_col, amber_high, wave_height * 0.75 + band_norm * 0.25);

    // Smooth depth & lateral boundary attenuation
    let depth_fade = mix(0.38, 1.15, pow(band_norm, 0.75));
    let edge_fade = smoothstep_r(0.50, 0.34, abs(u_coord - 0.5));
    let boundary_fade = depth_fade * edge_fade;

    let d = abs(s_dist);
    let edge_blur = 1.0 + smoothstep(0.3, 1.15, r) * 0.65;

    // Symmetric incandescent CRT electron-beam core centered at s_dist == 0.0 (Row 1 crest)
    let core = exp(-d * d * (3.2 / (edge_blur * edge_blur)));

    // Asymmetric phosphor persistence & 3D topographical curtain below the crest (s_dist >= 0.0)
    let glow_amp = 0.42 + wave_height * 0.42;
    let under_glow = exp(-max(s_dist, 0.0) * 0.32) * glow_amp;

    // Fine vertical harmonic comb ribs on the 3D topographical curtain below the crest
    let curtain_mask = smoothstep(0.4, 1.8, s_dist) * exp(-max(s_dist, 0.0) * 0.14);
    let rib_line = exp(-rib_grid * 1.6) * curtain_mask * (0.05 + wave_height * 0.12);
    let ridge_body = (0.005 + 0.022 * wave_height * wave_height) * exp(-max(s_dist, 0.0) * 0.12);

    let intensity = core * 1.55 + under_glow + rib_line + ridge_body;
    let core_hot = vec3<f32>(1.65, 1.25, 0.70) * (core * (0.30 + wave_height * 0.48));
    var color = (line_amber * intensity + core_hot) * boundary_fade;

    // Subtle CRT glass reflection & scanlines
    let norm = normalize(vec3<f32>(crt_uv * 0.18, 1.0));
    let light_source = normalize(vec3<f32>(-0.4, 0.7, -0.6));
    let reflection_spec = pow(max(dot(norm, light_source), 0.0), 32.0) * 0.04;
    let glass_reflection = vec3<f32>(0.85, 0.92, 1.0) * reflection_spec * bezel;
    color += glass_reflection;

    let scanline = 0.86 + 0.14 * cos(in.clip_position.y * 3.14159);
    color *= scanline * bezel;

    let noise_val = hash21_crt(in.clip_position.xy + fract(audio.smooth_time) * 100.0);
    let lum = dot(color, vec3<f32>(0.299, 0.587, 0.114));
    let noise_color = vec3<f32>(0.8, 0.35, 0.05) * noise_val * 0.012 * bezel * (0.25 + 0.75 * clamp(lum, 0.0, 1.0));
    color += noise_color;

    let final_color = aces_tonemap(color);
    return vec4<f32>(final_color, 1.0);
}
