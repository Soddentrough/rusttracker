// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;

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
    let mid = (audio.spectrum[10].x + audio.spectrum[20].x) * 0.5;

    // --- Sky gradient ---
    // Dark purple at bottom, transitioning to warmer hues near sun
    let sky_t = clamp(p.y * 0.8 + 0.3, 0.0, 1.0); // 0=bottom, 1=top
    let sky_bottom = vec3<f32>(0.01, 0.0, 0.04);     // Near-black purple
    let sky_mid    = vec3<f32>(0.08, 0.0, 0.15);      // Deep purple
    let sky_upper  = vec3<f32>(0.15, 0.02, 0.12);     // Warm purple near horizon
    var color = mix(sky_bottom, mix(sky_mid, sky_upper, sky_t), sky_t);

    // Sun glow halo (diffuse light around the sun, even behind terrain)
    let sun_center = vec2<f32>(0.0, 0.15);
    let sun_glow_dist = length(p - sun_center);
    let sun_halo = exp(-sun_glow_dist * 2.5) * 0.25;
    color += vec3<f32>(0.4, 0.1, 0.2) * sun_halo;

    // --- Synthwave Sun ---
    let sun_pos = vec2<f32>(0.0, 0.15);
    let sun_dist = length(p - sun_pos);
    let sun_radius = 0.35;
    // Only render sun above the horizon line and within radius
    if (sun_dist < sun_radius && p.y > -0.05) {
        // Horizontal scanline cuts for retro sun effect
        let cut = fract((p.y - sun_pos.y) * 20.0 - audio.time * 0.8);
        // More cuts in the lower half of the sun
        let cut_threshold = mix(0.3, 0.9, clamp((p.y - sun_pos.y + 0.2) * 2.5, 0.0, 1.0));
        if (cut > cut_threshold || p.y > sun_pos.y + 0.05) {
            // Sun color: hot magenta at bottom to warm yellow at top
            let sun_t = clamp((p.y - sun_pos.y + 0.2) * 2.0, 0.0, 1.0);
            let sun_col = mix(vec3<f32>(1.0, 0.05, 0.4), vec3<f32>(1.0, 0.85, 0.1), sun_t);
            // Soft edge
            let edge = smoothstep(sun_radius, sun_radius - 0.02, sun_dist);
            color = mix(color, sun_col, edge);
        }
    }

    // --- Terrain raycaster ---
    // Camera: looking forward and slightly down toward the grid
    let cam_y = 1.5; // Camera height
    let cam_fwd = audio.time * 15.0; // Forward motion speed
    let ro = vec3<f32>(0.0, cam_y, cam_fwd);
    let look_down = -0.3; // Look angle
    let rd = normalize(vec3<f32>(p.x * 0.8, p.y + look_down, 1.0));

    var t = 0.1;
    let max_t = 100.0;
    var hit = false;
    var hit_val = 0.0;
    var hit_p = vec3<f32>(0.0);
    var hit_t = 0.0;

    // 50-step raycaster
    for (var i = 0; i < 50; i = i + 1) {
        let pos = ro + rd * t;

        // Only check terrain below camera (rd.y < 0 region)
        if (pos.y < 0.0) {
            // Simple flat ground: terrain height from heatmap
            let map_x = abs(pos.x) * 2.0;
            let map_z = pos.z;

            let x_idx = clamp(u32(map_x), 0u, 255u);
            let t_idx = u32(abs(map_z)) % 120u;
            let tex_y = (audio.heatmap_row + 120u - t_idx) % 120u;
            let val = textureLoad(history_tex, vec2<i32>(i32(x_idx), i32(tex_y)), 0).x;

            // Height displacement from audio data
            let terrain_h = val * 0.08 - 0.5;

            if (pos.y < terrain_h) {
                hit = true;
                hit_val = val;
                hit_p = pos;
                hit_t = t;
                break;
            }
        }

        // Adaptive step size: large far away, small near ground
        t += max(0.3, t * 0.04);
        if (t > max_t) { break; }
    }

    // If no terrain hit, use flat ground plane intersection
    if (!hit && rd.y < -0.001) {
        let ground_t = -ro.y / rd.y;
        if (ground_t > 0.0 && ground_t < max_t) {
            hit = true;
            hit_t = ground_t;
            hit_p = ro + rd * ground_t;
            // Sample audio at the ground intersection point
            let gx = clamp(u32(abs(hit_p.x) * 2.0), 0u, 255u);
            let gz = u32(abs(hit_p.z)) % 120u;
            let gy = (audio.heatmap_row + 120u - gz) % 120u;
            hit_val = textureLoad(history_tex, vec2<i32>(i32(gx), i32(gy)), 0).x;
        }
    }

    if (hit) {
        // Distance fade: gentler falloff to show more terrain
        let z_fade = exp(-hit_t * 0.015);

        // Neon wireframe grid
        let grid_cell = 1.0; // Grid cell size
        let grid_x = smoothstep(0.08, 0.0, abs(fract(hit_p.x / grid_cell) - 0.5) * grid_cell);
        let grid_z = smoothstep(0.08, 0.0, abs(fract(hit_p.z / grid_cell) - 0.5) * grid_cell);
        let grid = max(grid_x, grid_z);

        // Grid color: shifts from magenta to cyan based on audio intensity
        let audio_intensity = clamp(hit_val * 0.08, 0.0, 1.0);
        let grid_col_base = mix(
            vec3<f32>(0.8, 0.0, 0.8),    // Magenta (quiet)
            vec3<f32>(0.0, 1.0, 1.0),     // Cyan (loud)
            audio_intensity
        );

        // Audio-reactive grid brightness boost
        let grid_brightness = 1.0 + audio_intensity * 2.0;
        let grid_col = grid_col_base * grid_brightness;

        // Dark ground between grid lines
        let ground_col = vec3<f32>(0.02, 0.0, 0.05);

        // Terrain height adds glow from below (audio mountains)
        let height_glow = clamp(hit_val * 0.02, 0.0, 1.0);
        let glow_col = mix(vec3<f32>(0.3, 0.0, 0.3), vec3<f32>(0.0, 0.8, 0.8), audio_intensity) * height_glow;

        let terrain_col = mix(ground_col + glow_col, grid_col, grid * z_fade);

        // Blend terrain with sky using distance fog
        let fog = 1.0 - z_fade;
        let fog_col = vec3<f32>(0.04, 0.0, 0.08);
        color = mix(terrain_col, fog_col, fog);
    }

    // --- Horizon glow line ---
    let horizon_y = -0.05; // Where terrain meets sky
    let horizon_dist = abs(p.y - horizon_y);
    let horizon_glow = exp(-horizon_dist * 15.0) * 0.3 * (1.0 + bass * 0.5);
    color += vec3<f32>(0.6, 0.1, 0.4) * horizon_glow;

    // ACES tonemapping
    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));

    return vec4<f32>(final_col, 1.0);
}
