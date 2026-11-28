// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;
@group(1) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
}

fn hash1(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }
fn hash2(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Reconstruct 3D world ray per pixel from active CameraUniforms so sun, stars & 360°
    // mountains remain world-locked when cycling chase, front, low-wheel, hood & orbit cameras.
    let ndc_x = in.uv.x * 2.0 - 1.0;
    let ndc_y = 1.0 - in.uv.y * 2.0;
    let rd_view = normalize(vec3<f32>(
        ndc_x / max(camera.proj_matrix[0][0], 0.05),
        ndc_y / max(camera.proj_matrix[1][1], 0.05),
        -1.0,
    ));
    let rd = normalize(vec3<f32>(
        dot(camera.view_matrix[0].xyz, rd_view),
        dot(camera.view_matrix[1].xyz, rd_view),
        dot(camera.view_matrix[2].xyz, rd_view),
    ));

    let bass_pulse = clamp((audio.spectrum[1].x + audio.spectrum[2].x + audio.spectrum[3].x) / 180.0, 0.0, 1.2);
    let mid_pulse = clamp((audio.spectrum[20].x + audio.spectrum[28].x) / 130.0, 0.0, 1.2);
    let treble_pulse = clamp((audio.spectrum[70].x + audio.spectrum[85].x) / 120.0, 0.0, 1.2);

    let elev = rd.y;
    let az = atan2(rd.x, rd.z); // [-pi, pi], 0 = ahead (+Z)
    let sun_dir = normalize(vec3<f32>(0.0, 0.135, 1.0));
    let forward_align = clamp(dot(normalize(vec3<f32>(rd.x, 0.0, rd.z)), vec3<f32>(0.0, 0.0, 1.0)) * 0.5 + 0.5, 0.0, 1.0);

    // Below-horizon fallback smoothly matching distant dusk ocean edge (no hard seam)
    if (elev < -0.025) {
        let sub_col = mix(vec3<f32>(0.18, 0.025, 0.22), vec3<f32>(0.015, 0.005, 0.025), smoothstep(-0.025, -0.18, elev));
        return vec4<f32>(aces_tonemap(sub_col), 1.0);
    }

    // =========================================================================
    // 1. 360° DIRECTION-AWARE SYNTHWAVE SUNSET GRADIENT
    // =========================================================================
    let sky_norm = clamp(max(elev, 0.0) / 0.48, 0.0, 1.0);
    let col_deep_space = vec3<f32>(0.025, 0.005, 0.07);
    let col_purple     = vec3<f32>(0.24, 0.02, 0.34);
    let col_crimson    = mix(vec3<f32>(0.42, 0.04, 0.32), vec3<f32>(0.90, 0.08, 0.32), forward_align);
    let col_sunset     = mix(vec3<f32>(0.55, 0.12, 0.36), vec3<f32>(1.00, 0.65, 0.16), pow(forward_align, 1.4));

    var sky_col = mix(col_sunset, col_crimson, smoothstep(0.0, 0.22, sky_norm));
    sky_col = mix(sky_col, col_purple, smoothstep(0.22, 0.58, sky_norm));
    sky_col = mix(sky_col, col_deep_space, smoothstep(0.58, 1.0, sky_norm));

    // Seamless Spherical Starfield
    if (elev > 0.05) {
        let az_u = az / 6.28318530718 + 0.5;
        let star_coord = vec2<f32>(az_u * 384.0, elev * 150.0);
        let star_cell = floor(star_coord);
        let star_local = fract(star_coord) - vec2<f32>(0.5);
        let star_rnd = hash2(star_cell);
        if (star_rnd > 0.935) {
            let star_offset = (vec2<f32>(hash1(star_rnd * 13.1), hash1(star_rnd * 37.7)) - 0.5) * 0.6;
            let d = length(star_local - star_offset);
            let zenith_fade = smoothstep_r(0.88, 0.55, abs(elev));
            let elev_fade = smoothstep(0.05, 0.16, elev);
            let star_point = smoothstep_r(0.18, 0.0, d) * zenith_fade * elev_fade;
            let star_twinkle = sin(audio.smooth_time * 3.5 + star_rnd * 40.0) * 0.4 + 0.6;
            sky_col += vec3<f32>(0.92, 0.96, 1.0) * star_point * star_twinkle * (0.6 + treble_pulse * 0.7);
        }
    }

    // =========================================================================
    // 2. GIANT GLOWING SEGMENTED 3D SYNTHWAVE SUN (Anchored Forward +Z)
    // =========================================================================
    let sun_radius = 0.235;
    let sun_dist = length(rd - sun_dir);
    let in_front_hemi = dot(rd, sun_dir) > 0.0;

    if (in_front_hemi && sun_dist < sun_radius && elev > -0.01) {
        let rel_y = (elev - (sun_dir.y - sun_radius)) / (sun_radius * 2.0);
        let blind_freq = 24.0;
        let blind_phase = fract(elev * blind_freq - audio.smooth_time * (0.45 + bass_pulse * 0.35));
        let blind_gap = mix(0.55, 0.04, smoothstep(0.05, 0.75, rel_y));
        let is_slit = (rel_y < 0.70) && (blind_phase < blind_gap);

        if (!is_slit) {
            let sun_c = mix(vec3<f32>(1.0, 0.90, 0.20), vec3<f32>(1.0, 0.04, 0.48), clamp(rel_y * 1.25, 0.0, 1.0));
            let sun_edge = smoothstep_r(sun_radius, sun_radius - 0.014, sun_dist);
            sky_col = mix(sky_col, sun_c * (2.2 + bass_pulse * 0.85), sun_edge);
        }
    }

    // Volumetric Sun Corona / Glow Halo (3D Angular Falloff)
    let ang_sep = length(rd - sun_dir);
    let corona_f = 1.0 / (1.0 + pow(ang_sep / sun_radius, 2.5));
    sky_col += vec3<f32>(1.0, 0.14, 0.42) * corona_f * (0.78 + bass_pulse * 0.68) * smoothstep(-0.025, 0.06, elev);

    // =========================================================================
    // 3. SEAMLESS 360° FAR & NEAR MOUNTAIN RANGES (Integer Azimuth Harmonics)
    // =========================================================================
    let pass_dip = 1.0 - 0.30 * exp(-az * az * 7.0);
    let mtn_far_h = (0.072 + sin(az * 3.0) * 0.036 + sin(az * 8.0 + 0.5) * 0.018 + sin(az * 17.0) * 0.007) * pass_dip;
    if (elev < mtn_far_h && elev > -0.025) {
        let mtn_body = vec3<f32>(0.038, 0.008, 0.065);
        let rim_dist = abs(elev - mtn_far_h);
        let rim_glow = smoothstep_r(0.010, 0.0, rim_dist) * vec3<f32>(0.99, 0.05, 0.58) * (1.8 + mid_pulse * 0.8);
        let contour = smoothstep(0.88, 0.98, sin(elev * 130.0));
        let grid_glow = vec3<f32>(0.98, 0.02, 0.50) * contour * 0.35 * (0.8 + bass_pulse * 0.6);
        sky_col = mtn_body + rim_glow + grid_glow;
    }

    let mtn_near_h = (0.044 + sin(az * 5.0 + 1.8) * 0.022 + sin(az * 11.0) * 0.011) * pass_dip;
    if (elev < mtn_near_h && elev > -0.025) {
        let mtn_body = vec3<f32>(0.020, 0.004, 0.038);
        let rim_dist = abs(elev - mtn_near_h);
        let rim_glow = smoothstep_r(0.008, 0.0, rim_dist) * vec3<f32>(0.0, 0.94, 1.0) * (1.6 + treble_pulse * 0.8);
        let contour = smoothstep(0.86, 0.98, sin(elev * 165.0));
        let grid_glow = vec3<f32>(0.0, 0.90, 1.0) * contour * 0.30 * (0.8 + mid_pulse * 0.6);
        sky_col = mtn_body + rim_glow + grid_glow;
    }

    let tonemapped = aces_tonemap(sky_col);
    return vec4<f32>(tonemapped, 1.0);
}
