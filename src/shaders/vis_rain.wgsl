// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

// --- Utility hashes ---
fn hash11(p: f32) -> f32 {
    var p2 = fract(p * 0.1031);
    p2 = p2 * (p2 + 33.33);
    return fract(2.0 * p2 * p2);
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.xyx) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// --- Lightning bolt branching ---
fn lightning_bolt(uv: vec2<f32>, seed: f32, intensity: f32) -> f32 {
    if (intensity < 0.01) { return 0.0; }

    var bolt = 0.0;
    // Main bolt
    var x_off = 0.0;
    let segments = 12;
    let seg_h = 2.0 / f32(segments);

    for (var i = 0; i < segments; i = i + 1) {
        let y0 = 1.0 - f32(i) * seg_h;
        let y1 = y0 - seg_h;
        let x_jitter = (hash11(seed + f32(i) * 7.13) - 0.5) * 0.3;
        let x0 = x_off;
        x_off = x_off + x_jitter;
        let x1 = x_off;

        // SDF to line segment
        let a = vec2<f32>(x0, y0);
        let b = vec2<f32>(x1, y1);
        let pa = uv - a;
        let ba = b - a;
        let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
        let d = length(pa - ba * h);

        // Thin bright core + wider glow
        bolt += smoothstep(0.012, 0.0, d) * 2.0;
        bolt += 0.003 / (d + 0.005);
    }

    return bolt * intensity;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);
    let safe_aspect = select(1.0, aspect, aspect > 0.0001 || aspect < -0.0001);
    let p = vec2<f32>(uv.x * safe_aspect, -uv.y);

    // --- Audio analysis ---
    let bass = max(audio.spectrum[0].x, audio.spectrum[1].x);
    let sub_bass = audio.spectrum[0].x;
    let treble = (audio.spectrum[50].x + audio.spectrum[60].x + audio.spectrum[70].x) * 0.33;

    // Lightning triggers only on sharp transients — very high threshold
    let flash_raw = smoothstep(0.7, 1.5, bass) + smoothstep(0.6, 1.2, treble) * 0.3;
    let flash = clamp(flash_raw, 0.0, 1.0);

    // --- Dark stormy sky gradient ---
    // p.y goes from -1 (bottom) to +1 (top)
    let sky_t = clamp(p.y * 0.5 + 0.5, 0.0, 1.0); // 0=bottom, 1=top
    let sky_dark = vec3<f32>(0.005, 0.008, 0.018);   // near-black at bottom
    let sky_mid  = vec3<f32>(0.015, 0.02, 0.045);     // dark blue-grey mid
    let sky_top  = vec3<f32>(0.025, 0.03, 0.055);     // slightly lighter at top (cloud backlit)
    var color = mix(sky_dark, mix(sky_mid, sky_top, sky_t), sky_t);

    // Subtle cloud-like variation
    let cloud_noise = hash12(p * 3.0 + vec2<f32>(audio.time * 0.02, 0.0));
    let cloud_band = smoothstep(0.3, 0.7, sky_t) * smoothstep(1.0, 0.7, sky_t);
    color += vec3<f32>(0.01, 0.012, 0.02) * cloud_noise * cloud_band;

    // --- Lightning flash illumination ---
    // Brief flash lights up the sky momentarily — kept subtle so midnight feel persists
    let flash_gradient = mix(0.05, 0.4, sky_t); // brighter at top but still restrained
    let lightning_ambient = vec3<f32>(0.12, 0.15, 0.25) * flash * flash_gradient;
    color += lightning_ambient;

    // --- Lightning bolts (one per channel) ---
    let num_ch = max(audio.num_channels, 1u);
    for (var i = 0u; i < num_ch; i = i + 1u) {
        let v_vec = audio.channels[i / 4u];
        let comp = i % 4u;
        var vu = 0.0;
        if comp == 0u { vu = v_vec.x; }
        else if comp == 1u { vu = v_vec.y; }
        else if comp == 2u { vu = v_vec.z; }
        else { vu = v_vec.w; }
        
        let intensity = smoothstep(0.6, 1.1, vu);
        if (intensity > 0.05) {
            let ch_x = -safe_aspect + (f32(i) + 0.5) * (2.0 * safe_aspect / f32(num_ch));
            // Stable seed during strikes
            let bolt_seed = floor(audio.time * 5.0) + f32(i) * 1.337;
            let bolt_pos = vec2<f32>(p.x - ch_x + (hash11(bolt_seed) - 0.5) * 0.5, p.y);
            let bolt = lightning_bolt(bolt_pos, bolt_seed, intensity);
            
            color += vec3<f32>(0.6, 0.7, 1.0) * bolt;
            
            // Sub-branch
            if (intensity > 0.4) {
                let bolt_pos2 = vec2<f32>(p.x - ch_x + (hash11(bolt_seed + 100.0) - 0.5) * 1.0, p.y);
                let bolt2 = lightning_bolt(bolt_pos2, bolt_seed + 50.0, intensity * 0.4);
                color += vec3<f32>(0.4, 0.5, 0.8) * bolt2;
            }
        }
    }

    // --- Rain layers (6 layers for density + depth) ---
    var rain_total = vec3<f32>(0.0);
    for (var layer = 0; layer < 6; layer = layer + 1) {
        let f_layer = f32(layer);

        // Each layer has different scale, speed, and brightness
        let scale_x = 30.0 + f_layer * 15.0;    // columns get denser with depth
        let scale_y = 3.0 + f_layer * 1.5;       // vertical tiling
        let speed = 8.0 + f_layer * 3.0;          // faster = closer
        let opacity = 1.0 / (1.0 + f_layer * 0.6); // closer layers are brighter

        // Rain falls straight down with slight wind offset
        let wind = sin(audio.time * 0.3) * 0.15;
        var rain_uv = vec2<f32>(
            p.x * scale_x + f_layer * 13.7 + p.y * wind * scale_x,
            p.y * scale_y + audio.time * speed
        );

        // Stagger columns to prevent horizontal banding
        rain_uv.y += hash11(floor(rain_uv.x)) * 3.14159;

        let cell_id = floor(rain_uv);
        let cell_uv = fract(rain_uv) - 0.5;

        // Random per-cell: x-offset within cell, visibility, length
        let rnd = hash22(cell_id + f_layer * 100.0);
        let drop_x = (rnd.x - 0.5) * 0.6;
        let visible = step(0.45, rnd.y);  // ~55% of cells have drops

        // Streak shape: narrow horizontal, elongated vertical
        let dx_drop = abs(cell_uv.x - drop_x);
        let streak_width = 0.02 + 0.01 * (1.0 - f_layer * 0.1);
        let streak = smoothstep(streak_width, 0.0, dx_drop);

        // Vertical falloff — elongated teardrop
        let drop_length = 0.3 + rnd.x * 0.15;
        let vert = smoothstep(drop_length, 0.0, abs(cell_uv.y)) * smoothstep(-0.5, -0.2, cell_uv.y);

        let drop = streak * vert * visible * opacity;

        // Rain color: cool blue-white, slightly brighter with lightning
        let rain_brightness = 0.25 + flash * 0.4;
        rain_total += vec3<f32>(0.5, 0.55, 0.7) * drop * rain_brightness;
    }
    color += rain_total;

    // --- Film grain for atmosphere ---
    let grain = (hash12(in.clip_position.xy + fract(audio.smooth_time) * 137.0) - 0.5) * 0.03;
    color += vec3<f32>(grain);

    // ACES tonemapping
    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));

    return vec4<f32>(final_col, 1.0);
}
