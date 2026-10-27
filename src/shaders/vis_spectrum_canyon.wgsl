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
    @location(0) uv_ndc: vec4<f32>,     // x: local_uv.x, y: is_backdrop, z: screen_ndc.x, w: screen_ndc.y
    @location(1) bar_params: vec4<f32>, // x: dist_from_tip_screen, y: row_t, z: vis_amp, w: tip_frac
    @location(2) seg_optics: vec4<f32>, // x: seg_density, y: groove_half_w, z: spire_core_mask, w: row_id
    @location(3) lighting: vec4<f32>,   // x: center_heat, y: layer_brightness, z: fog_amount, w: is_outer_wing
    @location(4) base_col: vec3<f32>,
}

fn get_live_spectrum(bin_idx: u32) -> f32 {
    let safe_idx = min(bin_idx, 1023u);
    return audio.spectrum[safe_idx >> 2u][safe_idx & 3u];
}

fn get_live_peak(bin_idx: u32) -> f32 {
    let safe_idx = min(bin_idx, 1023u);
    return audio.fire_heat[safe_idx >> 2u][safe_idx & 3u];
}

// Sample audio energy in [0, 1] for a given column (col_t in [0, 1]) and depth row (0..31)
fn sample_canyon_audio(mirror_col: u32, row_idx: u32, col_t: f32) -> f32 {
    let curved_t = clamp(col_t, 0.0, 1.0);
    let center_bin = clamp(u32(curved_t * 220.0) + 2u, 0u, 255u);

    var raw_amp = 0.0;
    if row_idx == 0u {
        let b4 = center_bin * 4u;
        let s0 = get_live_spectrum(b4);
        let s1 = get_live_spectrum(b4 + 1u);
        let s2 = get_live_spectrum(b4 + 2u);
        let s3 = get_live_spectrum(b4 + 3u);
        let pk = get_live_peak(b4);
        raw_amp = max(max(s0, s1), max(s2, s3)) * 0.75 + pk * 0.25;
    } else {
        let steps_ago = row_idx * 3u;
        let h_row0 = (audio.heatmap_row + 1024u - (steps_ago & 1023u)) & 1023u;
        let a0 = textureLoad(history_tex, vec2<i32>(i32(center_bin), i32(h_row0)), 0).x;
        let live_fallback = get_live_spectrum(center_bin * 4u);
        let row_atten = 1.0 / (1.0 + f32(row_idx) * 0.055);
        raw_amp = max(a0, live_fallback * row_atten);
    }

    let scaled_amp = raw_amp * 0.45;
    let norm_db = clamp(scaled_amp / (2.8 + scaled_amp), 0.0, 1.0);

    // High-contrast bar-to-bar equalizer variation (gives tall prominent spikes and deep valleys)
    let col_f = f32(mirror_col);
    let row_f = f32(row_idx);
    let t = audio.smooth_time;
    let center_weight = 1.0 - smoothstep(0.0, 0.18, curved_t);
    // Keep depth rows phase-coherent in the center so all rows in a red column rise and fall together
    let eff_row = mix(row_f, row_f * 0.05, center_weight);
    let jag1 = sin(col_f * 1.65 + eff_row * 2.35 - t * 3.8) * 0.5 + 0.5;
    let jag2 = cos(col_f * 3.10 - eff_row * 2.80 + t * 2.6) * 0.5 + 0.5;
    let jag3 = sin(col_f * 0.62 + eff_row * 1.25 + t * 1.7) * 0.5 + 0.5;
    let col_hash = hash11_common(col_f * 19.7 + eff_row * 13.3);
    let raw_jag = clamp(jag1 * 0.42 + jag2 * 0.33 + jag3 * 0.15 + col_hash * 0.10, 0.0, 1.0);
    let sharp_jag = raw_jag * sqrt(raw_jag);

    let base_activity = 0.46 + 0.54 * norm_db;
    let wing_amp = clamp(base_activity * (0.14 + 0.86 * sharp_jag) + norm_db * 0.18, 0.05, 1.0);
    // In the center, return the un-attenuated audio envelope so vs_main_3d can drive full [0, 1] excursions
    // without double-multiplying two independent wave envelopes
    let center_amp = clamp(0.42 + 0.58 * norm_db, 0.15, 1.0);
    return mix(wing_amp, center_amp, center_weight);
}

// Prismatic rainbow palette mapped from hue_t in [0, 1]:
// - 0.00..0.07: Crimson Red
// - 0.07..0.16: Fiery Orange
// - 0.16..0.34: Brilliant Lemon / Golden Yellow
// - 0.34..0.56: Chartreuse -> Vibrant Neon Emerald Green
// - 0.56..0.72: Aqua / Electric Cyan
// - 0.72..0.86: Cobalt / Royal Blue
// - 0.86..1.00: Deep Indigo -> Neon Violet / Purple
fn canyon_rainbow_palette(hue_t: f32) -> vec3<f32> {
    let t = clamp(hue_t, 0.0, 1.0);

    let c_red     = vec3<f32>(1.06, 0.05, 0.03);
    let c_orange  = vec3<f32>(1.08, 0.36, 0.02);
    let c_yellow  = vec3<f32>(1.02, 0.98, 0.04);
    let c_lime    = vec3<f32>(0.40, 0.98, 0.08);
    let c_green   = vec3<f32>(0.03, 0.92, 0.22);
    let c_cyan    = vec3<f32>(0.03, 0.86, 0.96);
    let c_blue    = vec3<f32>(0.15, 0.32, 1.02);
    let c_violet  = vec3<f32>(0.78, 0.10, 1.00);

    if t < 0.07 {
        return mix(c_red, c_orange, t / 0.07);
    } else if t < 0.16 {
        return mix(c_orange, c_yellow, (t - 0.07) / 0.09);
    } else if t < 0.33 {
        return mix(c_yellow, c_lime, (t - 0.16) / 0.17);
    } else if t < 0.55 {
        return mix(c_lime, c_green, (t - 0.33) / 0.22);
    } else if t < 0.72 {
        return mix(c_green, c_cyan, (t - 0.55) / 0.17);
    } else if t < 0.86 {
        return mix(c_cyan, c_blue, (t - 0.72) / 0.14);
    } else {
        return mix(c_blue, c_violet, (t - 0.86) / 0.14);
    }
}

@vertex
fn vs_main_3d(in: VertexInput, @builtin(instance_index) inst_idx: u32) -> VertexOutput3D {
    var out: VertexOutput3D;

    // Instance 8192 is the far-plane atmospheric chasm backdrop quad
    if inst_idx >= 8192u {
        let ndc = vec2<f32>(-in.position.x, in.position.y) * 2.0;
        out.clip_position = vec4<f32>(ndc.x, ndc.y, 0.9999, 1.0);
        out.uv_ndc = vec4<f32>(in.tex_coords.x, 1.0, ndc.x, ndc.y);
        out.bar_params = vec4<f32>(0.0);
        out.seg_optics = vec4<f32>(0.0);
        out.lighting = vec4<f32>(0.0);
        out.base_col = vec3<f32>(0.0);
        return out;
    }

    // 8192 3D Bar Instances:
    // 2 vertical halves (bottom floor = 0..4095, top ceiling = 4096..8191)
    // 32 depth rows (0..31) stretching deep into the background
    // 128 horizontal columns per row (64 left + 64 right, bilaterally mirrored)
    let is_top = inst_idx >= 4096u;
    let half_idx = select(inst_idx, inst_idx - 4096u, is_top);
    let row_idx = half_idx / 128u;
    let col_idx = half_idx % 128u;

    let is_right = col_idx >= 64u;
    let mirror_col = select(63u - col_idx, col_idx - 64u, is_right);
    let side_sign = select(-1.0, 1.0, is_right);
    let y_sign = select(-1.0, 1.0, is_top);

    let row_f = f32(row_idx);
    let row_t = row_f / 31.0; // 0.0 at front edge, 1.0 far in the background

    // Stagger rows so deeper columns interleave along diagonal perspective lines
    let row_stagger = fract(row_f * 0.38) * 0.65;
    let col_t = (f32(mirror_col) + 0.35 + row_stagger) / 64.0;

    // Camera is at (0, 0, -6.5) with fov_y = 48 deg (tan(24 deg) = 0.445228685)
    let cam_dist0 = 6.5;
    let tan_half_fov = 0.445228685;
    let aspect = max(audio.aspect_ratio, 0.5);

    let world_z = row_f * 1.50;
    let dist_from_cam = cam_dist0 + world_z;
    let frustum_h = dist_from_cam * tan_half_fov;
    let frustum_w = frustum_h * aspect;

    // Perspective X convergence (fast polynomial replacing pow(x, 1.02..1.52))
    let c0 = clamp(col_t, 0.0, 1.0);
    let c0_15 = c0 * sqrt(c0);
    let screen_x_abs = mix(c0, c0_15, row_t) * 1.015;
    let next_col_t = clamp((f32(mirror_col) + 1.35 + row_stagger) / 64.0, 0.0, 1.05);
    let c1_15 = next_col_t * sqrt(next_col_t);
    let next_screen_x_abs = mix(next_col_t, c1_15, row_t) * 1.015;
    let local_col_step_w = max(next_screen_x_abs - screen_x_abs, 0.004) * frustum_w;

    let world_x = side_sign * screen_x_abs * frustum_w;

    // Fast fractional powers of row_t (eliminates 3 SFU pow() calls per vertex)
    let sqrt_row_t = sqrt(row_t);
    let row_t_072 = mix(sqrt_row_t, row_t, 0.44);
    let row_t_055 = mix(sqrt_row_t, row_t, 0.10);

    // Color progression (hue_t):
    // - At row_t = 0 (front border): Red (center) -> Orange -> Yellow -> Neon Green (edges).
    // - As row_t increases into the background, hue_t shifts outward toward Cyan -> Blue -> Violet!
    let center_lock = smoothstep(0.04, 0.14, screen_x_abs);
    let hue_t = clamp(
        screen_x_abs * 0.44 + center_lock * (row_t_072 * 0.62 + screen_x_abs * row_t * 0.24),
        0.0,
        1.0
    );

    // Sample audio energy for this (column, depth row)
    let amp = sample_canyon_audio(mirror_col, row_idx, col_t);

    // Gaussian lobes in screen_x_abs using pure-ALU rational Gaussian (eliminates 4 SFU exp() calls per vertex)
    let dx_c2 = (screen_x_abs / 0.088) * (screen_x_abs / 0.088);
    let center_spire = 1.0 / (1.0 + dx_c2 + 0.48 * dx_c2 * dx_c2);

    // Primary yellow/orange inner flank lobe (around x = 0.20)
    let dx_y2 = ((screen_x_abs - 0.20) / 0.115) * ((screen_x_abs - 0.20) / 0.115);
    let yellow_lobe = 1.0 / (1.0 + dx_y2 + 0.48 * dx_y2 * dx_y2);

    // Secondary neon-green flank lobe (around x = 0.40)
    let dx_g2 = ((screen_x_abs - 0.40) / 0.155) * ((screen_x_abs - 0.40) / 0.155);
    let green_lobe = 1.0 / (1.0 + dx_g2 + 0.48 * dx_g2 * dx_g2);

    // Outer cyan/blue/violet wing lobe (around x = 0.72)
    let dx_o2 = ((screen_x_abs - 0.72) / 0.30) * ((screen_x_abs - 0.72) / 0.30);
    let outer_lobe = 1.0 / (1.0 + dx_o2 + 0.48 * dx_o2 * dx_o2);

    // High-contrast per-column spike envelope so adjacent columns have tall dramatic peaks and deep dark valleys
    let col_f = f32(mirror_col);
    let t = audio.smooth_time;
    let spike_wave1 = sin(col_f * 1.15 + row_f * 0.85 - t * 2.8) * 0.5 + 0.5;
    let spike_wave2 = cos(col_f * 2.45 - row_f * 1.35 + t * 1.9 + 0.8) * 0.5 + 0.5;
    let raw_spike = clamp(spike_wave1 * 0.58 + spike_wave2 * 0.42, 0.0, 1.0);
    let col_spike = raw_spike * sqrt(raw_spike);
    let jagged_amp = clamp(amp * (0.30 + 0.95 * col_spike), 0.04, 1.0);

    // Wide-range dynamic oscillator for the central red/orange spire:
    // Combine traveling column waves with a macro center pulse, mapped through a full-swing S-curve
    // so every red column regularly plunges down to 0.0 (deep valley) and surges up to 1.0 (horizon peak).
    let center_x_phase = screen_x_abs * 44.0 + col_f * 0.65;
    let center_wave_a = sin(center_x_phase + row_t * 0.55 - t * 4.2);
    let center_wave_b = cos(center_x_phase * 1.65 - row_t * 0.40 + t * 2.9);
    let center_macro  = sin(t * 2.1 - screen_x_abs * 12.0);
    let raw_center_wave = center_wave_a * 0.48 + center_wave_b * 0.32 + center_macro * 0.20;
    let center_osc = smoothstep(-0.62, 0.62, raw_center_wave);
    let center_drive = clamp(center_osc * amp, 0.0, 1.0);

    // Front-to-back multi-tier canyon architecture across 32 depth rows:
    // - Foreground rows (row_t ~ 0.0..0.22): Green at top/bottom corners (reach ~ 0.15..0.34),
    //   Yellow/Green equalizer spikes at x ~ 0.18..0.42 jutting deep into the chasm.
    // - Midground rows (row_t ~ 0.25..0.58): Cyan & Royal Blue waves stepping deeper below the green wings
    //   (reaching 0.42..0.64 on the wings and 0.65..0.82 near the inner flanks).
    // - Deep background rows (row_t ~ 0.60..1.0): Indigo & Violet dotted perspective trails stretching
    //   far into the background chasm (reaching 0.56..0.76 on the wings and 0.80..0.97 toward the center!)
    var reach_norm = 0.12;
    if row_idx == 0u {
        // Frontmost row: wide-swinging red/yellow center, medium-short jagged green comb on the outer edges
        let front_wing = (0.06 + 0.16 * jagged_amp) * (1.0 - center_spire * 0.65);
        let front_inner = center_spire * (0.14 + 0.78 * center_drive) + yellow_lobe * (0.20 + 0.38 * jagged_amp) + green_lobe * (0.10 + 0.22 * jagged_amp);
        reach_norm = clamp(front_wing + front_inner, 0.06, 0.94);
    } else {
        // Foreground envelope (row_t near 0.03):
        let front_envelope =
            0.14 +
            center_spire * 0.28 +
            yellow_lobe  * 0.38 +
            green_lobe   * 0.24 +
            outer_lobe   * 0.12;

        // Deep background envelope (row_t -> 1.0): stretches MUCH further into the chasm across the entire width!
        let deep_envelope =
            0.54 +
            center_spire * 0.24 +
            yellow_lobe  * 0.30 +
            green_lobe   * 0.24 +
            outer_lobe   * 0.16;

        let prog = row_t_055;
        let base_reach = mix(front_envelope, deep_envelope, prog);

        // Dramatic foreground/midground equalizer spikes: tall yellow & green bars jutting out in front of
        // the darker cyan/blue/violet layers
        let fg_weight = 1.0 - smoothstep(0.02, 0.50, row_t);
        let bg_mod = mix(0.76, 1.06, jagged_amp);
        let fg_spike = fg_weight * (jagged_amp - 0.32) * (yellow_lobe * 0.56 + green_lobe * 0.44 + outer_lobe * 0.28);
        let wing_reach = base_reach * bg_mod + fg_spike;

        // Central red/orange twin spire swings across a wide vertical range from retracted valleys (0.14..0.26)
        // all the way up to horizon-kissing peaks (0.90..0.99) without being floored by deep_envelope!
        let spire_floor = 0.14 + 0.12 * smoothstep(0.0, 0.80, row_t);
        let spire_peak  = 0.90 + 0.09 * smoothstep(0.0, 0.35, row_t);
        let spire_reach = mix(spire_floor, spire_peak, center_drive);
        reach_norm = mix(wing_reach, spire_reach, center_spire);
    }

    // Organic, staggered base recession (no flat straight black line across the top/bottom!):
    // Each row's base steps down smoothly with depth AND undulates with per-column wave variation,
    // creating natural jagged dark pockets between the green foreground, cyan/blue midground, and violet deep rows.
    let flank_mask = 0.32 + 0.68 * smoothstep(0.04, 0.24, screen_x_abs);
    let col_base_wave = (sin(col_f * 0.95 + row_f * 1.7 - t * 1.6) * 0.5 + 0.5) * 0.12 + (1.0 - jagged_amp) * 0.10;
    let row_step_in = row_t_072 * (0.28 + 0.30 * smoothstep(0.22, 1.0, screen_x_abs));
    let y_base_norm = clamp(1.03 - (row_step_in + col_base_wave * smoothstep(0.0, 0.25, row_t)) * flank_mask, 0.26, 1.03);

    let min_horizon_gap = mix(0.014, 0.14, smoothstep(0.05, 0.42, screen_x_abs));
    let y_tip_norm = clamp(1.02 - reach_norm, min_horizon_gap, y_base_norm - 0.025);
    let bar_len_norm = y_base_norm - y_tip_norm;

    // Convert normalized screen vertical interval [y_tip_norm, y_base_norm] into 3D world Y at depth world_z
    let y_base_world = y_base_norm * frustum_h;
    let y_tip_world  = y_tip_norm * frustum_h;
    let bar_h_world  = max(y_base_world - y_tip_world, 0.01);
    let y_center_world = 0.5 * (y_base_world + y_tip_world);

    let final_y_center = y_sign * y_center_world;

    // Bar width: narrower with clear dark spacing between adjacent columns so deeper layers peek through!
    let fill_ratio = mix(0.54, 0.36, row_t);
    let bar_w_world = local_col_step_w * fill_ratio;
    let bar_d_world = 0.035;

    let local_scaled = vec3<f32>(
        in.position.x * bar_w_world,
        in.position.y * bar_h_world,
        in.position.z * bar_d_world
    );
    let world_pos = local_scaled + vec3<f32>(world_x, final_y_center, world_z);

    let clip_pos = camera.proj_matrix * camera.view_matrix * vec4<f32>(world_pos, 1.0);
    out.clip_position = clip_pos;
    let screen_ndc = clip_pos.xy / max(clip_pos.w, 0.0001);
    out.uv_ndc = vec4<f32>(in.tex_coords.x, 0.0, screen_ndc.x, screen_ndc.y);

    // tip_frac: 0.0 at inner tip, 1.0 at outer base
    let tip_frac = select(0.5 - in.position.y, in.position.y + 0.5, is_top);
    let dist_from_tip_screen = tip_frac * bar_len_norm;

    let vis_amp = mix(jagged_amp, center_drive, center_spire);
    out.bar_params = vec4<f32>(dist_from_tip_screen, row_t, vis_amp, tip_frac);

    // Precompute per-bar optical parameters in the vertex stage using rational Gaussians
    let base_col = canyon_rainbow_palette(hue_t);
    let is_outer_wing = smoothstep(0.08, 0.22, screen_x_abs);
    let dot_wing_blend = clamp(smoothstep(0.28, 0.62, hue_t) * 0.55 + smoothstep(0.12, 0.55, row_t) * 0.65, 0.0, 1.0);
    let seg_density = mix(22.0, 76.0, dot_wing_blend);
    let groove_half_w = mix(0.10, 0.33, dot_wing_blend);
    let sc_x2 = (screen_x_abs / 0.075) * (screen_x_abs / 0.075);
    let spire_core_mask = 1.0 / (1.0 + sc_x2 + 0.48 * sc_x2 * sc_x2);

    let dx_core = (screen_x_abs - 0.046) / 0.028;
    let dxc2 = dx_core * dx_core;
    let center_heat = (1.0 / (1.0 + dxc2 + 0.48 * dxc2 * dxc2)) * smoothstep(0.02, 0.35, row_t) * (1.0 - smoothstep(0.45, 0.92, row_t));
    let center_cleft = 0.38 + 0.62 * smoothstep(0.004, 0.024, screen_x_abs);
    let w_arg = row_t * row_t * 2.35 + row_t * 0.55;
    let c_arg = row_t * row_t * 2.10 + row_t * 0.60;
    let wing_depth_darken = 1.0 / (1.0 + w_arg + 0.48 * w_arg * w_arg);
    let core_depth_darken = 1.0 / (1.0 + c_arg + 0.48 * c_arg * c_arg);
    let depth_atten = mix(wing_depth_darken, core_depth_darken, spire_core_mask);
    let layer_brightness = (0.018 + 1.50 * depth_atten) * (0.80 + 0.40 * vis_amp) * center_cleft;
    let fog_amount = smoothstep(0.52, 1.0, row_t) * (1.0 - spire_core_mask * 0.40) * 0.84;

    out.seg_optics = vec4<f32>(seg_density, groove_half_w, spire_core_mask, row_f);
    out.lighting = vec4<f32>(center_heat, layer_brightness, fog_amount, is_outer_wing);
    out.base_col = base_col;
    return out;
}

// Render the dark smoky central chasm & perspective speed streaks on the far-plane backdrop
fn render_chasm_backdrop(ndc: vec2<f32>) -> vec3<f32> {
    let ax = abs(ndc.x);
    let ay = abs(ndc.y);

    // Subtle dark smoky slate chasm in the center (2-octave noise instead of 8-octave fbm2d_4)
    let smoke_p1 = vec2<f32>(ndc.x * 2.2, ndc.y * 3.0) + vec2<f32>(0.0, audio.smooth_time * 0.03);
    let smoke_p2 = vec2<f32>(ndc.x * 4.0 - audio.smooth_time * 0.02, ndc.y * 4.8);
    let cloud = value_noise2d(smoke_p1) * 0.65 + value_noise2d(smoke_p2) * 0.35;

    let ay_b = ay * 4.0;
    let center_band = 1.0 / (1.0 + ay_b * ay_b);
    var bg = mix(
        vec3<f32>(0.0012, 0.0016, 0.0028),
        vec3<f32>(0.0085, 0.0115, 0.0185),
        smoothstep(0.22, 0.85, cloud) * (0.30 + 0.70 * center_band)
    );

    // Faint atmospheric crimson haze in the central chasm (pulsing with bass)
    let sgx = ax / 0.032;
    let sgy = ay / 0.16;
    let spire_glow_x = 1.0 / (1.0 + sgx * sgx);
    let spire_glow_y = 1.0 / (1.0 + sgy * sgy);
    let bass_bin = get_live_spectrum(2u) / 100.0;
    let core_bloom = spire_glow_x * spire_glow_y * (0.0015 + 0.0035 * clamp(bass_bin, 0.0, 1.0));
    bg += vec3<f32>(0.95, 0.12, 0.02) * core_bloom;

    return bg;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let screen_ndc = in.uv_ndc.zw;
    if in.uv_ndc.y > 0.5 {
        let bg_void = render_chasm_backdrop(screen_ndc);
        return vec4<f32>(aces_tonemap(bg_void), 1.0);
    }

    // Fast analytical chasm color for bar groove recesses & far depth fog
    let ay = abs(screen_ndc.y);
    let ay4 = ay * 4.0;
    let center_band = 1.0 / (1.0 + ay4 * ay4);
    let bg_void = mix(vec3<f32>(0.0016, 0.0022, 0.0038), vec3<f32>(0.0052, 0.0070, 0.0115), center_band);

    let dist_from_tip = in.bar_params.x;
    let row_t = in.bar_params.y;
    let amp = in.bar_params.z;
    let tip_frac = in.bar_params.w;

    let seg_density = in.seg_optics.x;
    let groove_half_w = in.seg_optics.y;
    let spire_core_mask = in.seg_optics.z;
    let row_id = in.seg_optics.w;

    let center_heat = in.lighting.x;
    let layer_brightness = in.lighting.y;
    let fog_amount = in.lighting.z;
    let is_outer_wing = in.lighting.w;

    var base_col = in.base_col;

    let dot_wing_blend = (seg_density - 22.0) * (1.0 / 54.0);
    let seg_coord = dist_from_tip * seg_density;
    let seg_frac = fract(seg_coord);

    let raw_seg_mask = smoothstep(groove_half_w, groove_half_w + 0.06, seg_frac) *
                       smoothstep_r(1.0 - groove_half_w, 1.0 - groove_half_w - 0.06, seg_frac);

    // Even foreground green/yellow bars have crisp LED segment breaks near their upper base (tip_frac > 0.55)
    let base_dot_boost = smoothstep(0.45, 0.85, tip_frac) * is_outer_wing * 0.75;
    let seg_contrast = clamp(mix(0.18, 0.98, dot_wing_blend) + base_dot_boost, 0.0, 0.98);
    let seg_mask = mix(1.0, raw_seg_mask, seg_contrast);

    // Horizontal edge bevel & dark border around each 3D bar
    let edge_x = abs(in.uv_ndc.x - 0.5) * 2.0;
    let raw_bevel_x = smoothstep_r(1.0, 0.56, edge_x);
    let bar_bevel_x = mix(raw_bevel_x, 0.45 + 0.55 * raw_bevel_x, spire_core_mask * smoothstep(0.08, 0.40, row_t));

    // Incandescent peach/orange-gold core highlight on the flanking pillars of the central Red spire
    let tip_glow = 1.0 / (1.0 + dist_from_tip * 9.0 + dist_from_tip * dist_from_tip * 36.0);
    let hot_core = center_heat * (0.45 + 0.55 * tip_glow) * (0.55 + 0.45 * amp);
    base_col = mix(base_col, vec3<f32>(1.05, 0.76, 0.38), clamp(hot_core * 0.58, 0.0, 0.62));

    // Upper-base shadow gradient on deeper rows (row_id > 0):
    var tier_cutout_shadow = 1.0;
    if row_id > 0.5 {
        let shadow_fade = smoothstep_r(0.84, 0.22, tip_frac);
        tier_cutout_shadow = mix(shadow_fade, 1.0, spire_core_mask * 0.28);
    }

    let raw_tip_rim = smoothstep(0.004, 0.014, dist_from_tip);
    let tip_rim_shadow = mix(raw_tip_rim, 0.42 + 0.58 * raw_tip_rim, spire_core_mask * smoothstep(0.10, 0.45, row_t));

    let active_led = seg_mask * bar_bevel_x * tip_rim_shadow * tier_cutout_shadow;
    let dark_groove_col = bg_void * 0.55;
    let lit_led_col = base_col * layer_brightness * (1.16 + 0.30 * tip_glow + 0.40 * hot_core);

    var color = mix(dark_groove_col, lit_led_col, active_led);

    // Seamless blend into the dark chasm backdrop as row_t -> 1.0 so deep bars fade smoothly into darkness
    color = mix(color, bg_void, fog_amount);

    color = aces_tonemap(color);
    return vec4<f32>(color, 1.0);
}
