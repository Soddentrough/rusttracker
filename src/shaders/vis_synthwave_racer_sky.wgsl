// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;

fn hash1(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }
fn hash2(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = audio.aspect_ratio;
    // Y positive UP, centered coordinates
    let p = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, -1.0);

    let bass_pulse = clamp(audio.spectrum[2].x * 1.4, 0.0, 1.0);
    let treble_pulse = clamp(audio.spectrum[80].x * 1.6, 0.0, 1.0);

    let horizon_y = 0.02; // Horizon line matching 3D camera line of sight
    let sway = sin(audio.smooth_time * 0.45) * 0.25;

    var final_color = vec3<f32>(0.0);

    if (p.y >= horizon_y) {
        // =========================================================================
        // 1. 4-STOP SYNTHWAVE SUNSET ATMOSPHERIC GRADIENT (Deep Background)
        // =========================================================================
        let sky_norm = clamp((p.y - horizon_y) / (0.50 - horizon_y), 0.0, 1.0);
        let col_deep_space = vec3<f32>(0.03, 0.005, 0.08);  // Top: Midnight Indigo
        let col_purple     = vec3<f32>(0.28, 0.02, 0.35);   // Upper-Mid: Synthwave Purple
        let col_crimson    = vec3<f32>(0.85, 0.05, 0.32);   // Lower-Mid: Neon Crimson
        let col_sunset     = vec3<f32>(1.00, 0.65, 0.15);   // Horizon: Warm Golden Peach

        var sky_col = mix(col_sunset, col_crimson, smoothstep(0.0, 0.25, sky_norm));
        sky_col = mix(sky_col, col_purple, smoothstep(0.25, 0.60, sky_norm));
        sky_col = mix(sky_col, col_deep_space, smoothstep(0.60, 1.0, sky_norm));

        // Fine Pinpoint Twinkling Starfield in Upper Atmosphere
        if (p.y > (horizon_y + 0.10)) {
            let star_scale = 180.0;
            let star_cell = floor(p * star_scale);
            let star_local = fract(p * star_scale) - vec2<f32>(0.5);
            let star_rnd = hash2(star_cell);
            if (star_rnd > 0.94) {
                let star_offset = (vec2<f32>(hash1(star_rnd * 13.1), hash1(star_rnd * 37.7)) - 0.5) * 0.6;
                let d = length(star_local - star_offset);
                let star_point = smoothstep(0.18, 0.0, d);
                let star_twinkle = sin(audio.smooth_time * 3.5 + star_rnd * 40.0) * 0.4 + 0.6;
                sky_col += vec3<f32>(0.92, 0.96, 1.0) * star_point * star_twinkle * (0.6 + treble_pulse * 0.4);
            }
        }

        // =========================================================================
        // 2. GIANT GLOWING SEGMENTED SYNTHWAVE SUN (Static Horizon Center)
        // =========================================================================
        let sun_center = vec2<f32>(0.0, horizon_y + 0.15);
        let sun_dist = length(p - sun_center);
        let sun_radius = 0.32;

        if (sun_dist < sun_radius) {
            let rel_y = (p.y - (sun_center.y - sun_radius)) / (sun_radius * 2.0);
            
            let blind_freq = 24.0;
            let blind_phase = fract((p.y - horizon_y) * blind_freq);
            let blind_gap = mix(0.55, 0.05, smoothstep(0.05, 0.75, rel_y));
            let is_slit = (rel_y < 0.70) && (blind_phase < blind_gap);

            if (!is_slit) {
                let sun_c = mix(vec3<f32>(1.0, 0.88, 0.20), vec3<f32>(1.0, 0.05, 0.45), clamp(rel_y * 1.3, 0.0, 1.0));
                let sun_edge = smoothstep(sun_radius, sun_radius - 0.015, sun_dist);
                sky_col = mix(sky_col, sun_c * (2.2 + bass_pulse * 0.6), sun_edge);
            }
        }

        // Volumetric Sun Corona / Glow Halo
        let corona_f = 1.0 / (1.0 + pow(sun_dist / sun_radius, 2.5));
        sky_col += vec3<f32>(1.0, 0.15, 0.40) * corona_f * (0.8 + bass_pulse * 0.6);

        // =========================================================================
        // 3. DISTANT JAGGED MOUNTAIN PEAKS (Far Range)
        // =========================================================================
        let mtn_far_h = horizon_y + 0.080 + sin(p.x * 2.5) * 0.045 + sin(p.x * 6.8 + 0.5) * 0.022 + sin(p.x * 14.0) * 0.008;
        if (p.y < mtn_far_h) {
            let mtn_body = vec3<f32>(0.04, 0.010, 0.07);
            let rim_dist = abs(p.y - mtn_far_h);
            let rim_glow = smoothstep(0.012, 0.0, rim_dist) * vec3<f32>(0.98, 0.05, 0.55) * 1.8;
            sky_col = mtn_body + rim_glow;
        }

        // =========================================================================
        // 4. FOREGROUND ROLLING FOOTHILLS & RIDGES (Near Range)
        // =========================================================================
        let mtn_near_h = horizon_y + 0.048 + sin(p.x * 4.0 + 1.8) * 0.030 + sin(p.x * 9.5) * 0.014;
        if (p.y < mtn_near_h) {
            let mtn_body = vec3<f32>(0.02, 0.005, 0.04);
            let rim_dist = abs(p.y - mtn_near_h);
            let rim_glow = smoothstep(0.009, 0.0, rim_dist) * vec3<f32>(0.0, 0.92, 0.98) * 1.6;
            sky_col = mtn_body + rim_glow;
        }

        final_color = sky_col;
    } else {
        // Below horizon fallback ground tint
        let ground_f = (horizon_y - p.y) / (0.50 + horizon_y);
        final_color = mix(vec3<f32>(0.18, 0.02, 0.16), vec3<f32>(0.025, 0.008, 0.045), clamp(ground_f, 0.0, 1.0));
    }

    let tonemapped = aces_tonemap(final_color);
    return vec4<f32>(tonemapped, 1.0);
}
