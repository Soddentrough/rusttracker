// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;

fn hash1(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }
fn hash2(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = max(audio.aspect_ratio, 0.2);
    // Y positive UP, centered coordinates
    let p = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, -1.0);

    let bass_pulse = clamp(audio.spectrum[2].x * 1.5, 0.0, 1.2);
    let mid_pulse = clamp(audio.spectrum[25].x * 1.6, 0.0, 1.2);
    let treble_pulse = clamp(audio.spectrum[80].x * 1.8, 0.0, 1.2);

    let horizon_y = 0.02; // Horizon line matching 3D camera line of sight

    // Below horizon early out (covered by 3D highway, ocean, and terrain mesh pass)
    if (p.y < horizon_y) {
        return vec4<f32>(0.015, 0.005, 0.025, 1.0);
    }

    // =========================================================================
    // 1. 4-STOP SYNTHWAVE SUNSET ATMOSPHERIC GRADIENT (Deep Background)
    // =========================================================================
    let sky_norm = clamp((p.y - horizon_y) / 0.48, 0.0, 1.0);
    let col_deep_space = vec3<f32>(0.025, 0.005, 0.07);  // Top: Midnight Indigo
    let col_purple     = vec3<f32>(0.26, 0.02, 0.34);   // Upper-Mid: Synthwave Purple
    let col_crimson    = vec3<f32>(0.88, 0.06, 0.30);   // Lower-Mid: Neon Crimson
    let col_sunset     = vec3<f32>(1.00, 0.65, 0.15);   // Horizon: Warm Golden Peach

    var sky_col = mix(col_sunset, col_crimson, smoothstep(0.0, 0.22, sky_norm));
    sky_col = mix(sky_col, col_purple, smoothstep(0.22, 0.58, sky_norm));
    sky_col = mix(sky_col, col_deep_space, smoothstep(0.58, 1.0, sky_norm));

    // Dynamic Starfield with Twinkle & Channel Audio Reactivity
    if (p.y > (horizon_y + 0.09)) {
        let star_scale = 160.0;
        let star_cell = floor(p * star_scale);
        let star_local = fract(p * star_scale) - vec2<f32>(0.5);
        let star_rnd = hash2(star_cell);
        if (star_rnd > 0.935) {
            let star_offset = (vec2<f32>(hash1(star_rnd * 13.1), hash1(star_rnd * 37.7)) - 0.5) * 0.6;
            let d = length(star_local - star_offset);
            let star_point = smoothstep(0.18, 0.0, d);
            let star_twinkle = sin(audio.smooth_time * 3.5 + star_rnd * 40.0) * 0.4 + 0.6;
            sky_col += vec3<f32>(0.92, 0.96, 1.0) * star_point * star_twinkle * (0.6 + treble_pulse * 0.6);
        }
    }

    // =========================================================================
    // 2. GIANT GLOWING SEGMENTED SYNTHWAVE SUN (Aspect-Ratio Scaled)
    // =========================================================================
    let sun_radius = clamp(0.24 + 0.06 * min(aspect, 1.6), 0.20, 0.34);
    let sun_center = vec2<f32>(0.0, horizon_y + sun_radius * 0.46);
    let sun_dist = length(p - sun_center);

    if (sun_dist < sun_radius) {
        let rel_y = (p.y - (sun_center.y - sun_radius)) / (sun_radius * 2.0);
        
        let blind_freq = 24.0;
        let blind_phase = fract((p.y - horizon_y) * blind_freq);
        let blind_gap = mix(0.55, 0.04, smoothstep(0.05, 0.75, rel_y));
        let is_slit = (rel_y < 0.70) && (blind_phase < blind_gap);

        if (!is_slit) {
            let sun_c = mix(vec3<f32>(1.0, 0.90, 0.20), vec3<f32>(1.0, 0.04, 0.48), clamp(rel_y * 1.25, 0.0, 1.0));
            let sun_edge = smoothstep(sun_radius, sun_radius - 0.015, sun_dist);
            sky_col = mix(sky_col, sun_c * (2.4 + bass_pulse * 0.7), sun_edge);
        }
    }

    // Volumetric Sun Corona / Glow Halo
    let corona_f = 1.0 / (1.0 + pow(sun_dist / sun_radius, 2.4));
    sky_col += vec3<f32>(1.0, 0.14, 0.42) * corona_f * (0.85 + bass_pulse * 0.65);

    // =========================================================================
    // 3. DISTANT JAGGED MOUNTAIN PEAKS (Far Range + Holographic Grid)
    // =========================================================================
    let mtn_far_h = horizon_y + 0.082 + sin(p.x * 2.5) * 0.045 + sin(p.x * 6.8 + 0.5) * 0.022 + sin(p.x * 14.0) * 0.008;
    if (p.y < mtn_far_h) {
        let mtn_body = vec3<f32>(0.038, 0.008, 0.065);
        let rim_dist = abs(p.y - mtn_far_h);
        let rim_glow = smoothstep(0.012, 0.0, rim_dist) * vec3<f32>(0.99, 0.05, 0.58) * (1.8 + mid_pulse * 0.8);
        
        // Holographic neon altitude contour lines across far mountains
        let contour = smoothstep(0.88, 0.98, sin((p.y - horizon_y) * 120.0));
        let grid_glow = vec3<f32>(0.98, 0.02, 0.50) * contour * 0.35 * (0.8 + bass_pulse * 0.6);
        
        sky_col = mtn_body + rim_glow + grid_glow;
    }

    // =========================================================================
    // 4. FOREGROUND ROLLING FOOTHILLS & RIDGES (Near Range + Cyan Grid)
    // =========================================================================
    let mtn_near_h = horizon_y + 0.050 + sin(p.x * 4.0 + 1.8) * 0.030 + sin(p.x * 9.5) * 0.014;
    if (p.y < mtn_near_h) {
        let mtn_body = vec3<f32>(0.020, 0.004, 0.038);
        let rim_dist = abs(p.y - mtn_near_h);
        let rim_glow = smoothstep(0.009, 0.0, rim_dist) * vec3<f32>(0.0, 0.94, 1.0) * (1.6 + treble_pulse * 0.8);
        
        let contour = smoothstep(0.86, 0.98, sin((p.y - horizon_y) * 150.0));
        let grid_glow = vec3<f32>(0.0, 0.90, 1.0) * contour * 0.30 * (0.8 + mid_pulse * 0.6);

        sky_col = mtn_body + rim_glow + grid_glow;
    }

    let tonemapped = aces_tonemap(sky_col);
    return vec4<f32>(tonemapped, 1.0);
}
