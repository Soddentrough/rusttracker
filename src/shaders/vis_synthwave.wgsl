// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;

fn hash1(n: f32) -> f32 { return fract(sin(n) * 43758.5453); }

fn hash2d(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise2d(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2d(i);
    let b = hash2d(i + vec2<f32>(1.0, 0.0));
    let c = hash2d(i + vec2<f32>(0.0, 1.0));
    let d = hash2d(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Ridge noise: produces sharp peaks instead of smooth blobs
fn ridge(p: vec2<f32>) -> f32 {
    return 1.0 - abs(noise2d(p) * 2.0 - 1.0);
}

// Procedural terrain: flat road in center, jagged mountains on sides
fn terrain_h(wx: f32, wz: f32) -> f32 {
    let road_half = 2.0;
    let dx = max(abs(wx) - road_half, 0.0);
    let slope = dx * 1.2;
    let q = vec2<f32>(wx, wz);
    // Ridge noise for sharp peaks + detail octaves
    let r1 = ridge(q * 0.04) * 1.0;
    let r2 = ridge(q * 0.1) * 0.45;
    let n3 = noise2d(q * 0.25) * 0.2;
    let n4 = noise2d(q * 0.6) * 0.08;
    let h = r1 + r2 + n3 + n4;
    return slope * h * 0.5 - 0.5;
}

// Surface normal via central differences
fn terrain_normal(wx: f32, wz: f32) -> vec3<f32> {
    let e = 0.15;
    let hc = terrain_h(wx, wz);
    let hx = terrain_h(wx + e, wz);
    let hz = terrain_h(wx, wz + e);
    return normalize(vec3<f32>(hc - hx, e, hc - hz));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let ddx = dpdx(in.uv.x);
    let ddy = dpdy(in.uv.y);
    let aspect = ddy / max(ddx, 0.00001);
    let safe_aspect = select(1.0, aspect, aspect > 0.0001 || aspect < -0.0001);
    let p = vec2<f32>(uv.x * safe_aspect, -uv.y);

    // --- Audio (lighting only) ---
    let bass = max(audio.spectrum[0].x, audio.spectrum[1].x);
    let mid = (audio.spectrum[10].x + audio.spectrum[20].x) * 0.5;

    // --- Sky gradient ---
    let sky_t = clamp(p.y * 0.8 + 0.3, 0.0, 1.0);
    var color = mix(vec3<f32>(0.01, 0.0, 0.04),
                    mix(vec3<f32>(0.08, 0.0, 0.15), vec3<f32>(0.15, 0.02, 0.12), sky_t), sky_t);

    // Sun glow halo
    let sun_pos = vec2<f32>(0.0, 0.35);
    let sun_glow_dist = length(p - sun_pos);
    color += vec3<f32>(0.4, 0.1, 0.2) * exp(-sun_glow_dist * 2.5) * 0.25;

    // --- Stars ---
    let star_uv = p * 80.0;
    let star_id = floor(star_uv);
    let star_rnd = hash1(star_id.x * 127.1 + star_id.y * 311.7);
    let star_b = step(0.97, star_rnd) * smoothstep(0.04, 0.0, length(fract(star_uv) - 0.5)) * clamp(p.y - 0.1, 0.0, 1.0);
    color += vec3<f32>(star_b);

    // --- Synthwave Sun (fixed) ---
    let sun_dist = length(p - sun_pos);
    let sun_radius = 0.35;
    if (sun_dist < sun_radius && p.y > -0.05) {
        let cut = fract((p.y - sun_pos.y) * 20.0 - audio.time * 0.8);
        let cut_threshold = mix(0.3, 0.9, clamp((p.y - sun_pos.y + 0.2) * 2.5, 0.0, 1.0));
        if (cut > cut_threshold || p.y > sun_pos.y + 0.05) {
            let sun_t = clamp((p.y - sun_pos.y + 0.2) * 2.0, 0.0, 1.0);
            let sun_col = mix(vec3<f32>(1.0, 0.05, 0.4), vec3<f32>(1.0, 0.85, 0.1), sun_t);
            color = mix(color, sun_col, smoothstep(sun_radius, sun_radius - 0.02, sun_dist));
        }
    }

    // --- Clouds ---
    if (p.y > 0.0 && p.y < 0.8) {
        let cu = vec2<f32>(p.x * 2.0 + audio.time * 0.015, (p.y - 0.05) * 4.0);
        let cloud = noise2d(cu * 2.5) * 0.6 + noise2d(cu * 5.0 + vec2<f32>(3.7, 1.2)) * 0.4;
        let alt_mask = smoothstep(0.0, 0.15, p.y) * smoothstep(0.7, 0.2, p.y);
        let cloud_alpha = smoothstep(0.4, 0.6, cloud) * alt_mask * 0.55;
        color = mix(color, vec3<f32>(0.08, 0.02, 0.12), cloud_alpha);
        color += vec3<f32>(0.2, 0.05, 0.1) * smoothstep(0.55, 0.4, cloud) * alt_mask * exp(-sun_glow_dist * 1.5) * 0.5;
    }

    // --- 3D Terrain raymarcher ---
    let cam_y = 1.5;
    let cam_fwd = audio.time * 15.0;
    let ro = vec3<f32>(0.0, cam_y, cam_fwd);
    let rd = normalize(vec3<f32>(p.x * 1.5, p.y - 0.3, 1.0));
    let sun_dir = normalize(vec3<f32>(0.0, 0.6, -1.0)); // Sun light direction

    var t_ray = 0.1;
    var prev_t = 0.0;
    var hit = false;
    var hit_val = 0.0;
    var hit_p = vec3<f32>(0.0);
    var hit_t = 0.0;
    var hit_road = false;
    let road_half = 2.0;

    // Coarse raymarcher: find interval where ray crosses terrain
    for (var i = 0; i < 96; i = i + 1) {
        let pos = ro + rd * t_ray;
        let th = terrain_h(pos.x, pos.z);

        if (pos.y < th) {
            // --- Binary search refinement for precise surface ---
            var t_lo = prev_t;
            var t_hi = t_ray;
            for (var j = 0; j < 6; j = j + 1) {
                let t_mid = (t_lo + t_hi) * 0.5;
                let pm = ro + rd * t_mid;
                if (pm.y < terrain_h(pm.x, pm.z)) {
                    t_hi = t_mid;
                } else {
                    t_lo = t_mid;
                }
            }
            hit = true;
            hit_t = t_hi;
            hit_p = ro + rd * t_hi;
            hit_road = abs(hit_p.x) < road_half + 0.5;
            let x_idx = clamp(u32(abs(hit_p.x) * 2.0), 0u, 255u);
            let t_idx = u32(abs(hit_p.z)) % 120u;
            let tex_y = (audio.heatmap_row + 120u - t_idx) % 120u;
            hit_val = textureLoad(history_tex, vec2<i32>(i32(x_idx), i32(tex_y)), 0).x;
            break;
        }

        prev_t = t_ray;
        let margin = pos.y - th;
        // Very conservative stepping to eliminate horizon gaps
        t_ray += max(0.08, min(margin * 0.25, t_ray * 0.02));
        if (t_ray > 80.0) { break; }
    }

    // Flat ground fallback
    if (!hit && rd.y < -0.001) {
        let ground_t = (ro.y + 0.5) / (-rd.y);
        if (ground_t > 0.0 && ground_t < 80.0) {
            let gp = ro + rd * ground_t;
            hit = true;
            hit_t = ground_t;
            hit_p = gp;
            hit_road = abs(gp.x) < road_half + 0.5;
            let gx = clamp(u32(abs(gp.x) * 2.0), 0u, 255u);
            let gz = u32(abs(gp.z)) % 120u;
            let gy = (audio.heatmap_row + 120u - gz) % 120u;
            hit_val = textureLoad(history_tex, vec2<i32>(i32(gx), i32(gy)), 0).x;
        }
    }

    if (hit) {
        let z_fade = exp(-hit_t * 0.015);
        let fog = 1.0 - z_fade;
        let fog_col = vec3<f32>(0.04, 0.0, 0.08);

        if (hit_road) {
            // --- Neon wireframe grid (road) ---
            let gx = smoothstep(0.08, 0.0, abs(fract(hit_p.x) - 0.5));
            let gz = smoothstep(0.08, 0.0, abs(fract(hit_p.z) - 0.5));
            let grid = max(gx, gz);
            let audio_i = clamp(hit_val * 0.08, 0.0, 1.0);
            let grid_col = mix(vec3<f32>(0.8, 0.0, 0.8), vec3<f32>(0.0, 1.0, 1.0), audio_i)
                         * (1.0 + bass * 1.5 + mid * 0.5);
            let terrain_col = mix(vec3<f32>(0.02, 0.0, 0.05), grid_col, grid * z_fade);
            color = mix(terrain_col, fog_col, fog);
        } else {
            // --- Mountain surface with normal-based shading ---
            let N = terrain_normal(hit_p.x, hit_p.z);

            // Diffuse sun lighting
            let sun_diffuse = max(dot(N, sun_dir), 0.0);
            // Rim/backlight from sun (highlights mountain edges)
            let view_dir = normalize(ro - hit_p);
            let rim = pow(1.0 - max(dot(N, view_dir), 0.0), 3.0);

            // Height-based color: darker at base, lighter at peaks
            let elev = clamp((hit_p.y + 0.5) * 0.3, 0.0, 1.0);
            let base_col = mix(vec3<f32>(0.03, 0.0, 0.06), vec3<f32>(0.06, 0.01, 0.10), elev);

            // Sun-lit faces get warm magenta tint
            let lit_col = base_col + vec3<f32>(0.25, 0.03, 0.15) * sun_diffuse;

            // Rim light: neon magenta edge highlight
            let rim_col = vec3<f32>(0.6, 0.05, 0.4) * rim * 0.5;

            // Wireframe grid on mountain surface
            let mgx = smoothstep(0.06, 0.0, abs(fract(hit_p.x * 0.5) - 0.5));
            let mgz = smoothstep(0.06, 0.0, abs(fract(hit_p.z * 0.5) - 0.5));
            let mtn_grid = max(mgx, mgz);
            let wire_col = vec3<f32>(0.4, 0.0, 0.4) * (1.0 + bass * 0.8) * z_fade;

            let mtn_col = lit_col + rim_col + wire_col * mtn_grid * 0.5;
            color = mix(mtn_col, fog_col, fog);
        }
    }

    // --- Horizon glow line ---
    let horizon_dist = abs(p.y + 0.05);
    color += vec3<f32>(0.6, 0.1, 0.4) * exp(-horizon_dist * 15.0) * (0.3 + bass * 0.4);

    // ACES tonemapping
    var fc = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    fc = max(fc, vec3<f32>(0.0));
    return vec4<f32>(fc, 1.0);
}
