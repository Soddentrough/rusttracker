// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

// Fast ALU hashes (avoids SFU sin() stalls on mobile GPUs)
fn hash11_fast(p: f32) -> f32 {
    var p1 = fract(p * 0.1031);
    p1 *= p1 + 33.33;
    p1 *= p1 + p1;
    return fract(p1);
}

fn hash21_fast(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22_fast(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + vec3<f32>(33.33));
    return fract((p3.xx + p3.yz) * p3.zy);
}

fn noise2d_fast(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21_fast(i);
    let b = hash21_fast(i + vec2<f32>(1.0, 0.0));
    let c = hash21_fast(i + vec2<f32>(0.0, 1.0));
    let d = hash21_fast(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm_cloud_fast(p_in: vec2<f32>) -> f32 {
    let n0 = noise2d_fast(p_in);
    let n1 = noise2d_fast(p_in * 2.04 + vec2<f32>(1.7, 9.2));
    return n0 * 0.62 + n1 * 0.38;
}

fn sd_segment_sq(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    let diff = pa - ba * h;
    return dot(diff, diff);
}

// Continuous 1D piecewise-linear noise for O(1) loop-free jagged lightning paths
fn noise1d_linear(x: f32) -> f32 {
    let i = floor(x);
    let f = fract(x);
    return mix(hash11_fast(i), hash11_fast(i + 1.0), f) - 0.5;
}

// --- Connected main & forked lightning bolt with O(1) loop-free evaluation ---
// Returns vec2(main_bolt_glow, branch_bolt_glow)
fn lightning_bolt_forked(uv: vec2<f32>, seed: f32, intensity: f32) -> vec2<f32> {
    // Early-out for inactive channels or pixels outside the strike corridor or vertical range
    let corridor_limit = 0.42;
    let dist_x = abs(uv.x);
    if (intensity < 0.015 || dist_x > corridor_limit || uv.y > 1.05 || uv.y < -0.78) {
        return vec2<f32>(0.0);
    }
    let edge_fade = smoothstep_r(corridor_limit, corridor_limit * 0.40, dist_x);

    let t_y = clamp((1.0 - uv.y) * (1.0 / 1.75), 0.0, 1.0);
    let base_x = (hash11_fast(seed) - 0.5) * 0.18;

    // 2-octave piecewise-linear jagged trunk path x(y) (continuous, 0 segment loops!)
    let trunk_x = base_x
                + noise1d_linear(t_y * 3.5 + seed * 3.17) * 0.28
                + noise1d_linear(t_y * 11.0 + seed * 7.13) * 0.11;

    let dx_main = abs(uv.x - trunk_x);
    // Account for local slope (~1.15) to match true perpendicular Euclidean distance
    let d_main = dx_main * 0.86;
    let inv_d = 1.0 / (d_main + 0.0042);
    var main_bolt = 0.068 * inv_d;
    if (d_main < 0.015) {
        main_bolt += smoothstep_r(0.015, 0.0, d_main) * 4.4;
    }

    var branch_bolt = 0.0;
    let fork_t = 0.22 + hash11_fast(seed + 19.7) * 0.28;
    let branch_len = 0.42;
    let dt_fork = t_y - fork_t;
    if (intensity > 0.30 && dt_fork > 0.0 && dt_fork < branch_len) {
        let fork_dir = select(-1.0, 1.0, hash11_fast(seed + 41.3) > 0.5);
        let fork_root_x = base_x
                        + noise1d_linear(fork_t * 3.5 + seed * 3.17) * 0.28
                        + noise1d_linear(fork_t * 11.0 + seed * 7.13) * 0.11;
        let branch_x = fork_root_x
                     + fork_dir * dt_fork * 0.62
                     + noise1d_linear(dt_fork * 12.0 + seed * 13.7) * 0.09;
        let d_branch = abs(uv.x - branch_x) * 0.80;
        let b_fade = 1.0 - dt_fork * (1.0 / branch_len);
        var b_contrib = 0.032 / (d_branch + 0.005);
        if (d_branch < 0.011) {
            b_contrib += smoothstep_r(0.011, 0.0, d_branch) * 3.0;
        }
        branch_bolt = b_contrib * b_fade;
    }

    return vec2<f32>(main_bolt, branch_bolt) * (intensity * edge_fade);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let safe_aspect = max(audio.aspect_ratio, 0.2);
    let p = vec2<f32>(uv.x * safe_aspect, -uv.y);

    // Ground horizon for wet midnight asphalt / puddle reflections at the bottom
    let horizon_y = -0.68;
    let is_ground = p.y < horizon_y;

    // Mirror vertical coordinate across horizon_y when evaluating reflected sky on the ground
    var sample_p = p;
    if (is_ground) {
        let ground_depth = max(horizon_y - p.y, 0.001);
        // Subtle water ripple distortion on reflected coordinates
        let ripple_x = sin(p.x * 28.0 + audio.smooth_time * 6.0) * cos(ground_depth * 65.0 - audio.smooth_time * 8.0);
        sample_p = vec2<f32>(p.x + ripple_x * 0.018 * (1.0 - ground_depth * 1.5), 2.0 * horizon_y - p.y);
    }

    // --- Audio analysis (properly normalized from 0..100 spectrum scale) ---
    let bass = max(audio.spectrum[0].x, max(audio.spectrum[1].x, audio.spectrum[2].x)) / 78.0;
    let transient_energy = (audio.spectrum[1].x + audio.spectrum[2].x + audio.spectrum[3].x + audio.spectrum[4].x) / 300.0;
    let treble = (audio.spectrum[50].x + audio.spectrum[60].x + audio.spectrum[70].x) / 210.0;

    // Dramatic lightning flash triggers on bass kicks or strong transients (dark between beats)
    let kick_trigger = pow(clamp(max(bass, transient_energy * 1.15), 0.0, 1.0), 2.0);
    let flash_raw = smoothstep(0.22, 0.85, kick_trigger) + smoothstep(0.35, 0.90, clamp(treble, 0.0, 1.0)) * 0.30;
    let flash = clamp(flash_raw, 0.0, 1.0);

    // --- Dark stormy sky gradient ---
    let sky_t = clamp((sample_p.y - horizon_y) / (1.0 - horizon_y), 0.0, 1.0); // 0=horizon, 1=zenith
    let sky_dark = vec3<f32>(0.006, 0.010, 0.022);   // deep midnight horizon
    let sky_mid  = vec3<f32>(0.016, 0.024, 0.052);   // dark storm blue-grey mid
    let sky_top  = vec3<f32>(0.028, 0.036, 0.068);   // cloud-backlit zenith
    var color = mix(sky_dark, mix(sky_mid, sky_top, sky_t), sky_t);

    // --- Volumetric FBM Storm Clouds ---
    let cloud_band = smoothstep(0.15, 0.55, sky_t) * smoothstep_r(1.0, 0.65, sky_t);
    var cloud_density = 0.5;
    if (cloud_band > 0.001) {
        let cloud_uv = vec2<f32>(sample_p.x * 1.1 + audio.smooth_time * 0.035, sample_p.y * 1.8 - 0.3);
        cloud_density = clamp(fbm_cloud_fast(cloud_uv), 0.0, 1.0);

        // Dark cloud underbellies + bright silver-blue internal scattering during lightning flashes
        let cloud_shadow = smoothstep(0.32, 0.72, cloud_density) * cloud_band;
        let cloud_Scatter = smoothstep(0.40, 0.80, cloud_density) * cloud_band;
        color = mix(color, vec3<f32>(0.004, 0.006, 0.014), cloud_shadow * 0.55);
        color += vec3<f32>(0.22, 0.30, 0.52) * cloud_Scatter * (0.08 + flash * 0.85);
    }

    // --- Lightning flash ambient illumination ---
    let flash_gradient = mix(0.12, 0.75, sky_t) * (0.6 + 0.4 * cloud_density);
    color += vec3<f32>(0.32, 0.44, 0.76) * flash * flash_gradient;

    // --- Lightning bolts (only evaluate the 2 nearest channel columns to sample_p.x) ---
    let num_ch = max(audio.num_channels, 1u);
    let strike_tick = floor(audio.smooth_time * 4.5);
    let ch_step = (2.0 * safe_aspect) / f32(num_ch);
    let ch_f = (sample_p.x + safe_aspect) / ch_step - 0.5;
    let i_min = u32(clamp(floor(ch_f), 0.0, f32(num_ch - 1u)));
    let i_max = u32(clamp(floor(ch_f) + 1.0, 0.0, f32(num_ch - 1u)));
    for (var i = i_min; i <= i_max; i = i + 1u) {
        let v_vec = audio.channels[i / 4u];
        let p_vec = audio.channel_peaks[i / 4u];
        let comp = i % 4u;
        let vu = v_vec[comp];
        let peak = p_vec[comp];

        let signal = max(vu * 1.25, peak);
        let intensity = smoothstep(0.32, 0.80, signal);
        if (intensity > 0.02) {
            let ch_x = -safe_aspect + (f32(i) + 0.5) * ch_step;
            let col_dist = abs(sample_p.x - ch_x);
            if (col_dist < 0.55) {
                let bolt_seed = strike_tick * 17.31 + f32(i) * 1.337;
                let bolt_pos = vec2<f32>(sample_p.x - ch_x, sample_p.y);
                let bolts = lightning_bolt_forked(bolt_pos, bolt_seed, intensity);

                let ch_tint = mix(
                    vec3<f32>(0.68, 0.82, 1.25),
                    vec3<f32>(0.82, 0.65, 1.30),
                    fract(f32(i) * 0.37)
                );
                color += ch_tint * bolts.x + ch_tint * 0.78 * bolts.y;

                let cloud_halo = (1.0 / (1.0 + col_dist * col_dist * 18.0)) * cloud_band * (0.4 + 0.6 * cloud_density) * intensity;
                color += ch_tint * cloud_halo * 0.28;
            }
        }
    }

    // --- Wet ground puddle reflection & expanding raindrop splash rings ---
    if (is_ground) {
        let ground_t = clamp((horizon_y - p.y) / (1.0 + horizon_y), 0.0, 1.0);
        // Darken reflection with Fresnel falloff and blend with wet asphalt base
        let fresnel = 0.22 + 0.55 * pow(1.0 - ground_t, 2.2);
        let asphalt_base = vec3<f32>(0.004, 0.006, 0.012);
        color = mix(asphalt_base, color, fresnel);

        // Perspective-projected expanding raindrop ripples on the wet ground
        let persp_z = 0.35 / max(ground_t + 0.04, 0.04);
        let persp_uv = vec2<f32>(p.x * persp_z * 2.4, persp_z * 2.4);
        let ripple_cell = floor(persp_uv);
        let ripple_local = fract(persp_uv) - 0.5;
        let r_rnd = hash22_fast(ripple_cell);
        if (r_rnd.x >= 0.35) {
            let r_phase = fract(audio.smooth_time * (1.4 + r_rnd.x * 0.8) + r_rnd.y);
            let r_dist = length(ripple_local - (r_rnd - 0.5) * 0.45);
            let ring = smoothstep_r(0.035, 0.0, abs(r_dist - r_phase * 0.42)) * (1.0 - r_phase);
            let ripple_glow = vec3<f32>(0.45, 0.58, 0.85) * ring * (0.35 + flash * 0.85 + clamp(treble, 0.0, 1.0) * 0.5) * smoothstep(0.0, 0.15, ground_t);
            color += ripple_glow;
        }

        // Subtle horizon mist line
        let mist = exp(-abs(p.y - horizon_y) * 35.0) * (0.04 + flash * 0.12);
        color += vec3<f32>(0.30, 0.40, 0.65) * mist;
    }

    // --- Rain layers (4 layers for density + depth) ---
    var rain_total = vec3<f32>(0.0);
    let wind_slant = 0.055;
    let slanted_x = p.x + p.y * wind_slant;
    let wind_sway = sin(audio.smooth_time * 0.12) * 0.8;
    let rain_brightness = (0.28 + flash * 0.48 + clamp(treble, 0.0, 1.0) * 0.16) * 1.25;

    for (var layer = 0; layer < 4; layer = layer + 1) {
        let f_layer = f32(layer);

        // Each layer has different scale, speed, and brightness
        let scale_x = 30.0 + f_layer * 20.0;    // columns get denser with depth
        let rain_x = slanted_x * scale_x + f_layer * 13.7 + wind_sway;
        let cell_uv_x = fract(rain_x) - 0.5;
        // drop_x is in [-0.30, 0.30] and streak_width <= 0.03; reject outside [-0.33, 0.33] before hashing
        if (abs(cell_uv_x) >= 0.33) { continue; }

        let col_id = floor(rain_x);
        let col_hash = hash11_fast(col_id + f_layer * 37.0);
        let drop_x = (col_hash - 0.5) * 0.6;
        let dx_drop = abs(cell_uv_x - drop_x);
        let streak_width = 0.02 + 0.01 * (1.0 - f_layer * 0.1);
        if (dx_drop >= streak_width) { continue; }

        let scale_y = 3.0 + f_layer * 2.0;       // vertical tiling
        let speed = 2.8 + f_layer * 1.3;         // natural cinematic fall speed (~1.1s screen traversal)

        let rain_y = p.y * scale_y + audio.smooth_time * speed + col_hash * 3.14159;
        let cell_id_y = floor(rain_y);
        let rnd_y = hash11_fast(col_id * 19.3 + cell_id_y * 7.1 + f_layer * 100.0);
        if (rnd_y < 0.58) { continue; }

        let cell_uv_y = fract(rain_y) - 0.5;
        let opacity = 1.0 / (1.0 + f_layer * 0.6);
        let streak = smoothstep_r(streak_width, 0.0, dx_drop);

        // Teardrop shape: sharp bright head at the bottom, fading trail extending upwards
        let drop_length = 0.35 + col_hash * 0.2;
        let head = smoothstep(-drop_length, -drop_length + 0.05, cell_uv_y);
        let tail = smoothstep_r(drop_length, -drop_length, cell_uv_y);
        let vert = head * tail;

        let drop = streak * vert * opacity;
        rain_total += vec3<f32>(0.52, 0.58, 0.75) * drop * rain_brightness;
    }
    color += rain_total;

    // --- Film grain for atmosphere ---
    let grain = (hash21_fast(in.clip_position.xy + fract(audio.smooth_time) * 137.0) - 0.5) * 0.025;
    color += vec3<f32>(grain);

    // ACES tonemapping
    var final_col = aces_tonemap(max(color, vec3<f32>(0.0)));
    return vec4<f32>(final_col, 1.0);
}
