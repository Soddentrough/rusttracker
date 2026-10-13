// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

// --- Connected main & forked lightning bolt with horizontal X-culling ---
// Returns vec2(main_bolt_glow, branch_bolt_glow)
fn lightning_bolt_forked(uv: vec2<f32>, seed: f32, intensity: f32) -> vec2<f32> {
    // Early-out for inactive channels or pixels outside the bolt's horizontal corridor
    if (intensity < 0.01 || abs(uv.x) > 0.72) {
        return vec2<f32>(0.0);
    }

    var main_bolt = 0.0;
    var branch_bolt = 0.0;
    var x_off = (hash11_common(seed) - 0.5) * 0.22;
    let segments = 14;
    let seg_h = 1.75 / f32(segments);

    // Pick a segment index (3..7) where a forked sub-branch splits off the main trunk
    let fork_seg = 3 + i32(hash11_common(seed + 19.7) * 5.0);
    let fork_dir = select(-1.0, 1.0, hash11_common(seed + 41.3) > 0.5);
    var bx_off = 0.0;
    var branch_active = false;

    for (var i = 0; i < segments; i = i + 1) {
        let fi = f32(i);
        let y0 = 1.0 - fi * seg_h;
        let y1 = y0 - seg_h;
        let x_jitter = (hash11_common(seed + fi * 7.13) - 0.5) * 0.24;
        let x0 = x_off;
        x_off = x_off + x_jitter;
        let x1 = x_off;

        if (i == fork_seg && intensity > 0.30) {
            bx_off = x0;
            branch_active = true;
        }

        // Main trunk SDF
        let d = sd_line_segment(uv, vec2<f32>(x0, y0), vec2<f32>(x1, y1));
        main_bolt += smoothstep_r(0.015, 0.0, d) * 4.2;
        main_bolt += 0.0075 / (d + 0.004);

        // Connected forked sub-branch (grows outward from exact trunk node)
        if (branch_active && i < fork_seg + 6) {
            let b_step = f32(i - fork_seg);
            let by0 = y0;
            let by1 = y1;
            let bx0 = bx_off;
            let bx_jitter = fork_dir * (0.045 + hash11_common(seed + 91.1 + fi * 5.7) * 0.08);
            bx_off = bx_off + bx_jitter;
            let bx1 = bx_off;

            let bd = sd_line_segment(uv, vec2<f32>(bx0, by0), vec2<f32>(bx1, by1));
            let b_fade = 1.0 - b_step / 6.0;
            branch_bolt += (smoothstep_r(0.011, 0.0, bd) * 2.8 + 0.004 / (bd + 0.005)) * b_fade;
        }
    }

    return vec2<f32>(main_bolt, branch_bolt) * intensity;
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
    let cloud_uv = vec2<f32>(sample_p.x * 1.1 + audio.smooth_time * 0.035, sample_p.y * 1.8 - 0.3);
    let cloud_fbm = fbm2d_4(cloud_uv);
    let cloud_detail = value_noise2d(cloud_uv * 3.5 - vec2<f32>(audio.smooth_time * 0.06, 0.0));
    let cloud_density = clamp(cloud_fbm * 0.75 + cloud_detail * 0.25, 0.0, 1.0);
    let cloud_band = smoothstep(0.15, 0.55, sky_t) * smoothstep_r(1.0, 0.65, sky_t);

    // Dark cloud underbellies + bright silver-blue internal scattering during lightning flashes
    let cloud_shadow = smoothstep(0.35, 0.75, cloud_density) * cloud_band;
    let cloud_Scatter = smoothstep(0.42, 0.82, cloud_density) * cloud_band;
    color = mix(color, vec3<f32>(0.004, 0.006, 0.014), cloud_shadow * 0.55);
    color += vec3<f32>(0.22, 0.30, 0.52) * cloud_Scatter * (0.08 + flash * 0.85);

    // --- Lightning flash ambient illumination ---
    let flash_gradient = mix(0.12, 0.75, sky_t) * (0.6 + 0.4 * cloud_density);
    color += vec3<f32>(0.32, 0.44, 0.76) * flash * flash_gradient;

    // --- Lightning bolts (one per active channel, with connected forks) ---
    let num_ch = max(audio.num_channels, 1u);
    let strike_tick = floor(audio.smooth_time * 4.5);
    for (var i = 0u; i < num_ch; i = i + 1u) {
        let v_vec = audio.channels[i / 4u];
        let p_vec = audio.channel_peaks[i / 4u];
        let comp = i % 4u;
        let vu = v_vec[comp];
        let peak = p_vec[comp];

        let signal = max(vu * 1.25, peak);
        let intensity = smoothstep(0.32, 0.80, signal);
        if (intensity > 0.02) {
            let ch_x = -safe_aspect + (f32(i) + 0.5) * (2.0 * safe_aspect / f32(num_ch));
            let bolt_seed = strike_tick * 17.31 + f32(i) * 1.337;
            let bolt_pos = vec2<f32>(sample_p.x - ch_x, sample_p.y);
            let bolts = lightning_bolt_forked(bolt_pos, bolt_seed, intensity);

            // Tint each channel's corona subtly between electric blue, cyan, and violet
            let ch_tint = mix(
                vec3<f32>(0.68, 0.82, 1.25),
                vec3<f32>(0.82, 0.65, 1.30),
                fract(f32(i) * 0.37)
            );
            color += ch_tint * bolts.x + ch_tint * 0.78 * bolts.y;

            // Illuminate clouds immediately surrounding the strike column
            let col_dist = abs(sample_p.x - ch_x);
            let cloud_halo = exp(-col_dist * 3.5) * cloud_band * (0.4 + 0.6 * cloud_density) * intensity;
            color += ch_tint * cloud_halo * 0.28;
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
        let r_rnd = hash22_common(ripple_cell);
        let r_phase = fract(audio.smooth_time * (1.4 + r_rnd.x * 0.8) + r_rnd.y);
        let r_dist = length(ripple_local - (r_rnd - 0.5) * 0.45);
        let ring = smoothstep_r(0.035, 0.0, abs(r_dist - r_phase * 0.42)) * (1.0 - r_phase) * step(0.35, r_rnd.x);
        let ripple_glow = vec3<f32>(0.45, 0.58, 0.85) * ring * (0.35 + flash * 0.85 + clamp(treble, 0.0, 1.0) * 0.5) * smoothstep(0.0, 0.15, ground_t);
        color += ripple_glow;

        // Subtle horizon mist line
        let mist = exp(-abs(p.y - horizon_y) * 35.0) * (0.04 + flash * 0.12);
        color += vec3<f32>(0.30, 0.40, 0.65) * mist;
    }

    // --- Rain layers (6 layers for density + depth) ---
    var rain_total = vec3<f32>(0.0);
    let wind_slant = 0.055;
    let wind_sway = sin(audio.smooth_time * 0.12) * 0.8;
    let rain_brightness = 0.26 + flash * 0.45 + clamp(treble, 0.0, 1.0) * 0.15;

    for (var layer = 0; layer < 6; layer = layer + 1) {
        let f_layer = f32(layer);

        // Each layer has different scale, speed, and brightness
        let scale_x = 30.0 + f_layer * 15.0;    // columns get denser with depth
        let scale_y = 3.0 + f_layer * 1.5;       // vertical tiling
        let speed = 2.8 + f_layer * 1.0;          // natural cinematic fall speed (~1.1s screen traversal)
        let opacity = 1.0 / (1.0 + f_layer * 0.6); // closer layers are brighter

        var rain_uv = vec2<f32>(
            p.x * scale_x + f_layer * 13.7 + p.y * wind_slant * scale_x + wind_sway,
            p.y * scale_y + audio.smooth_time * speed
        );

        // Stagger columns to prevent horizontal banding
        rain_uv.y += hash11_common(floor(rain_uv.x)) * 3.14159;

        let cell_id = floor(rain_uv);
        let cell_uv = fract(rain_uv) - 0.5;

        // Random per-cell: x-offset within cell, visibility, length
        let rnd = hash22_common(cell_id + f_layer * 100.0);
        let drop_x = (rnd.x - 0.5) * 0.6;
        let visible = step(0.62, rnd.y);  // ~38% of cells have drops for clean atmospheric depth

        // Streak shape: narrow horizontal, elongated vertical
        let dx_drop = abs(cell_uv.x - drop_x);
        let streak_width = 0.02 + 0.01 * (1.0 - f_layer * 0.1);
        let streak = smoothstep_r(streak_width, 0.0, dx_drop);

        // Teardrop shape: sharp bright head at the bottom, fading trail extending upwards
        let drop_length = 0.35 + rnd.x * 0.2;
        let head = smoothstep(-drop_length, -drop_length + 0.05, cell_uv.y);
        let tail = smoothstep_r(drop_length, -drop_length, cell_uv.y);
        let vert = head * tail;

        let drop = streak * vert * visible * opacity;
        rain_total += vec3<f32>(0.52, 0.58, 0.75) * drop * rain_brightness;
    }
    color += rain_total;

    // --- Film grain for atmosphere ---
    let grain = (hash21_crt(in.clip_position.xy + fract(audio.smooth_time) * 137.0) - 0.5) * 0.025;
    color += vec3<f32>(grain);

    // ACES tonemapping
    var final_col = aces_tonemap(max(color, vec3<f32>(0.0)));
    return vec4<f32>(final_col, 1.0);
}
