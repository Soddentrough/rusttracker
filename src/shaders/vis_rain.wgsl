// INCLUDE: common

// ============================================================================
// Visualizer ID 16: Midnight Storm (vis_rain.wgsl)
// Ground-up modern real-time atmospheric & storm renderer:
// - 3D perspective camera over a storm-tossed midnight ocean
// - Volumetric cumulonimbus cloud deck with Beer-Lambert transmittance,
//   Frostbite powder/multiple-scattering approximation, & cloud normal lighting
// - True 2D Euclidean Polyline SDF stepped-leader lightning & forked branches
//   with staggered multi-cell depth, in-cloud plasma origins, & zero card seams
// - Physical Schlick-Fresnel 3D water surface with distance-filtered swells,
//   aligned anisotropic lightning glitter columns, & raindrop splash ripples
// - Volumetric downburst virga shafts & 2D-jittered motion-blurred rain needles
// ============================================================================

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

// Fast pure-ALU hashes (avoids SFU sin() stalls on desktop & mobile GPUs)
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

// Smooth C2 quintic value noise in 2D
fn noise2d_fast(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = hash21_fast(i);
    let b = hash21_fast(i + vec2<f32>(1.0, 0.0));
    let c = hash21_fast(i + vec2<f32>(0.0, 1.0));
    let d = hash21_fast(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Continuous 1D piecewise-linear noise for fine intra-segment plasma tortuosity
fn noise1d_linear(x: f32) -> f32 {
    let i = floor(x);
    let f = fract(x);
    return mix(hash11_fast(i), hash11_fast(i + 1.0), f) - 0.5;
}

// 4-octave rotated & domain-warped cumulonimbus cloud FBM with billow turbulence
fn fbm_storm_cloud(p_in: vec2<f32>, wind_t: f32) -> f32 {
    var p = p_in;
    let rot = mat2x2<f32>(0.80, -0.60, 0.60, 0.80);

    let n0 = noise2d_fast(p + vec2<f32>(wind_t * 0.045, -wind_t * 0.018));
    let b0 = 1.0 - abs(n0 * 2.0 - 1.0);
    p = rot * p * 2.02 + vec2<f32>(17.3 + n0 * 0.42, 9.2 - b0 * 0.32);

    let n1 = noise2d_fast(p - vec2<f32>(wind_t * 0.08, wind_t * 0.035));
    let b1 = 1.0 - abs(n1 * 2.0 - 1.0);
    p = rot * p * 2.04 + vec2<f32>(31.7 - n1 * 0.28, 23.1 + b1 * 0.24);

    let n2 = noise2d_fast(p + vec2<f32>(wind_t * 0.13, 0.0));
    p = rot * p * 2.03 + vec2<f32>(11.4, 47.8);
    let n3 = noise2d_fast(p);

    let macro_shape = mix(n0, b0, 0.44) * 0.48 + mix(n1, b1, 0.36) * 0.28;
    let detail = n2 * 0.15 + n3 * 0.09;
    return clamp(macro_shape + detail, 0.0, 1.0);
}

// 2-octave macro cloud height for smooth volumetric dome normal calculation
fn fbm_storm_cloud_macro(p_in: vec2<f32>, wind_t: f32) -> f32 {
    var p = p_in;
    let rot = mat2x2<f32>(0.80, -0.60, 0.60, 0.80);
    let n0 = noise2d_fast(p + vec2<f32>(wind_t * 0.045, -wind_t * 0.018));
    let b0 = 1.0 - abs(n0 * 2.0 - 1.0);
    p = rot * p * 2.02 + vec2<f32>(17.3 + n0 * 0.42, 9.2 - b0 * 0.32);
    let n1 = noise2d_fast(p - vec2<f32>(wind_t * 0.08, wind_t * 0.035));
    let b1 = 1.0 - abs(n1 * 2.0 - 1.0);
    return mix(n0, b0, 0.44) * 0.63 + mix(n1, b1, 0.36) * 0.37;
}

// Exact 2D Euclidean squared distance to line segment AB
fn sd_segment_sq(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    let diff = pa - ba * h;
    return dot(diff, diff);
}

// Evaluate a single segment of a lightning polyline with zero-pinned endpoint micro-tortuosity
// Returns vec2(core_intensity, corona_intensity)
fn eval_bolt_segment(
    p: vec2<f32>,
    a: vec2<f32>,
    b: vec2<f32>,
    w_start: f32,
    w_end: f32,
    radius_scale: f32,
    seg_seed: f32
) -> vec2<f32> {
    let ba = b - a;
    let len_sq = max(dot(ba, ba), 1e-6);
    let pa = p - a;
    let h = clamp(dot(pa, ba) / len_sq, 0.0, 1.0);
    let base_diff = pa - ba * h;
    let raw_d2 = dot(base_diff, base_diff);

    // Beyond d = 0.095, exp(-d * 105) < 4.5e-5 (sub-LSB in 8/10-bit SDR/HDR)
    if (raw_d2 > 0.0092) {
        return vec2<f32>(0.0);
    }

    let inv_len = inverseSqrt(len_sq);
    let perp = vec2<f32>(-ba.y, ba.x) * inv_len;

    // Fine fractal plasma tortuosity pinned to 0 at both endpoints (4*h*(1-h) = 0 at h=0,1)
    let endpoint_pin = 4.0 * h * (1.0 - h);
    let micro_jag = (noise1d_linear(h * 5.0 + seg_seed) * 0.014
                   + noise1d_linear(h * 13.0 + seg_seed * 2.37) * 0.006) * endpoint_pin * radius_scale;

    let closest = a + ba * h + perp * micro_jag;
    let d = length(p - closest);
    let w = mix(w_start, w_end, h);

    let inv_r = 1.0 / max(radius_scale, 0.25);
    let d_scaled = d * inv_r;

    let core = exp(-d_scaled * d_scaled * 58000.0) * w;
    let corona = exp(-d_scaled * 105.0) * w;
    return vec2<f32>(core, corona);
}

// ============================================================================
// True 2D Euclidean Polyline SDF Lightning Bolt & Multi-Tier Forked Branches
// Returns vec4(core_plasma, corona_sheath, mie_scatter, cloud_origin_glow)
// ============================================================================
fn lightning_bolt(
    p: vec2<f32>,
    anchor_x: f32,
    y_top: f32,
    depth_scale: f32,
    seed: f32,
    intensity: f32
) -> vec4<f32> {
    if (intensity <= 0.002) {
        return vec4<f32>(0.0);
    }

    let y_bot = -0.01;
    let base_x = anchor_x + (hash11_fast(seed * 1.91) - 0.5) * 0.24;
    let slant = (hash11_fast(seed * 4.73) - 0.5) * 0.28;

    // Macro endpoints V[0], V[4], V[8] for global unclipped Mie scattering & origin glow
    let v0 = vec2<f32>(
        base_x - 0.35 * slant + ((hash11_fast(seed * 3.17) - 0.5) * 0.16) * 0.45 * depth_scale,
        y_top
    );
    let v4 = vec2<f32>(
        base_x + 0.15 * slant + ((hash11_fast(seed * 3.17 + 28.52) - 0.5) * 0.16 + (hash11_fast(seed * 9.41 + 54.8) - 0.5) * 0.06) * depth_scale,
        mix(y_top, y_bot, 0.5)
    );
    let v8 = vec2<f32>(
        base_x + 0.65 * slant + ((hash11_fast(seed * 3.17 + 57.04) - 0.5) * 0.16) * 0.45 * depth_scale,
        y_bot
    );

    // Volumetric In-Cloud Origin Plasma Root at V[0] (evaluated globally across the sky)
    let d_origin = (p - v0) * vec2<f32>(1.0, 1.35);
    let origin_r2 = dot(d_origin, d_origin);
    let origin_glow = (1.0 / (1.0 + origin_r2 * 16.0 + origin_r2 * origin_r2 * 64.0)) * intensity;

    // Macro 2-segment axis distance for global Mie scatter
    let macro_d2 = min(sd_segment_sq(p, v0, v4), sd_segment_sq(p, v4, v8));

    // If pixel is far outside the fine plasma/branch corridor, return exact global Mie + origin glow
    // (At macro_d2 > 0.14, fine core & corona exp(-d * 105) are < 1e-12 = 0.0 in f32!)
    if (macro_d2 > 0.14) {
        let mie_far = (0.0092 / (macro_d2 + 0.012 + macro_d2 * macro_d2 * 14.0)) * intensity;
        return vec4<f32>(0.0, 0.0, mie_far, origin_glow);
    }

    // Construct full 8-segment main return-stroke leader polyline vertices V[0..8]
    var v_pts: array<vec2<f32>, 9>;
    v_pts[0] = v0;
    v_pts[4] = v4;
    v_pts[8] = v8;
    for (var i = 1; i < 8; i = i + 1) {
        if (i == 4) { continue; }
        let fi = f32(i) * 0.125;
        let vy = mix(y_top, y_bot, fi);
        let macro_zig = ((hash11_fast(seed * 3.17 + f32(i) * 7.13) - 0.5) * 0.16
                       + (hash11_fast(seed * 9.41 + f32(i) * 13.7) - 0.5) * 0.06) * depth_scale;
        let env_zig = 0.45 + 0.55 * sin(fi * 3.14159);
        let vx = base_x + (fi - 0.35) * slant + macro_zig * env_zig;
        v_pts[i] = vec2<f32>(vx, vy);
    }

    var core_max = 0.0;
    var corona_max = 0.0;
    var min_trunk_d2 = macro_d2;

    // 1. Evaluate 8 connected main trunk segments
    for (var i = 0; i < 8; i = i + 1) {
        let fi0 = f32(i) * 0.125;
        let fi1 = f32(i + 1) * 0.125;
        let w0 = smoothstep(0.0, 0.09, fi0);
        let w1 = smoothstep(0.0, 0.09, fi1);

        let a = v_pts[i];
        let b = v_pts[i + 1];
        let seg_res = eval_bolt_segment(p, a, b, w0, w1, depth_scale, seed + f32(i) * 19.3);
        core_max = max(core_max, seg_res.x);
        corona_max = max(corona_max, seg_res.y);

        min_trunk_d2 = min(min_trunk_d2, sd_segment_sq(p, a, b));
    }

    // 2. Primary Forked Stepped-Leader Branch 1 (sprouting from vertex V[2])
    let b1_dir = select(-1.0, 1.0, hash11_fast(seed + 41.3) > 0.5);
    let b1_p0 = v_pts[2];
    let b1_p1 = b1_p0 + vec2<f32>(
        b1_dir * (0.085 + hash11_fast(seed + 53.1) * 0.055),
        -(0.095 + hash11_fast(seed + 61.9) * 0.045)
    ) * depth_scale;
    let b1_p2 = b1_p1 + vec2<f32>(
        b1_dir * (0.075 + hash11_fast(seed + 73.7) * 0.065),
        -(0.105 + hash11_fast(seed + 83.3) * 0.045)
    ) * depth_scale;
    let b1_p3 = b1_p2 + vec2<f32>(
        b1_dir * (0.055 + hash11_fast(seed + 89.1) * 0.055),
        -(0.090 + hash11_fast(seed + 97.3) * 0.040)
    ) * depth_scale;

    let b1_s0 = eval_bolt_segment(p, b1_p0, b1_p1, 0.78, 0.52, 0.68 * depth_scale, seed + 101.3);
    let b1_s1 = eval_bolt_segment(p, b1_p1, b1_p2, 0.52, 0.24, 0.54 * depth_scale, seed + 117.7);
    let b1_s2 = eval_bolt_segment(p, b1_p2, b1_p3, 0.24, 0.00, 0.40 * depth_scale, seed + 131.9);
    core_max = max(core_max, max(b1_s0.x, max(b1_s1.x, b1_s2.x)));
    corona_max = max(corona_max, max(b1_s0.y, max(b1_s1.y, b1_s2.y)));

    let b1_twig_end = b1_p1 + vec2<f32>(
        b1_dir * 0.045,
        -(0.085 + hash11_fast(seed + 149.3) * 0.035)
    ) * depth_scale;
    let b1_twig = eval_bolt_segment(p, b1_p1, b1_twig_end, 0.42, 0.00, 0.42 * depth_scale, seed + 157.1);
    core_max = max(core_max, b1_twig.x);
    corona_max = max(corona_max, b1_twig.y);

    // 3. Secondary Forked Stepped-Leader Branch 2 (sprouting from vertex V[4] on opposite side)
    let b2_dir = -b1_dir;
    let b2_p0 = v_pts[4];
    let b2_p1 = b2_p0 + vec2<f32>(
        b2_dir * (0.080 + hash11_fast(seed + 163.7) * 0.055),
        -(0.090 + hash11_fast(seed + 173.3) * 0.040)
    ) * depth_scale;
    let b2_p2 = b2_p1 + vec2<f32>(
        b2_dir * (0.070 + hash11_fast(seed + 181.1) * 0.050),
        -(0.095 + hash11_fast(seed + 191.9) * 0.045)
    ) * depth_scale;
    let b2_p3 = b2_p2 + vec2<f32>(
        b2_dir * (0.050 + hash11_fast(seed + 199.3) * 0.045),
        -(0.080 + hash11_fast(seed + 211.7) * 0.035)
    ) * depth_scale;

    let b2_s0 = eval_bolt_segment(p, b2_p0, b2_p1, 0.72, 0.46, 0.64 * depth_scale, seed + 223.1);
    let b2_s1 = eval_bolt_segment(p, b2_p1, b2_p2, 0.46, 0.20, 0.50 * depth_scale, seed + 239.7);
    let b2_s2 = eval_bolt_segment(p, b2_p2, b2_p3, 0.20, 0.00, 0.38 * depth_scale, seed + 251.3);
    core_max = max(core_max, max(b2_s0.x, max(b2_s1.x, b2_s2.x)));
    corona_max = max(corona_max, max(b2_s0.y, max(b2_s1.y, b2_s2.y)));

    // 4. Lower Stepped-Leader Branch 3 (sprouting from vertex V[5])
    let b3_p0 = v_pts[5];
    let b3_p1 = b3_p0 + vec2<f32>(
        b1_dir * (0.065 + hash11_fast(seed + 263.9) * 0.045),
        -(0.085 + hash11_fast(seed + 271.3) * 0.035)
    ) * depth_scale;
    let b3_p2 = b3_p1 + vec2<f32>(
        b1_dir * (0.055 + hash11_fast(seed + 281.7) * 0.040),
        -(0.075 + hash11_fast(seed + 293.1) * 0.035)
    ) * depth_scale;
    let b3_s0 = eval_bolt_segment(p, b3_p0, b3_p1, 0.62, 0.28, 0.55 * depth_scale, seed + 307.9);
    let b3_s1 = eval_bolt_segment(p, b3_p1, b3_p2, 0.28, 0.00, 0.40 * depth_scale, seed + 317.3);
    core_max = max(core_max, max(b3_s0.x, b3_s1.x));
    corona_max = max(corona_max, max(b3_s0.y, b3_s1.y));

    let mie = (0.0092 / (macro_d2 + 0.012 + macro_d2 * macro_d2 * 14.0)) * intensity;
    return vec4<f32>(core_max * 4.2 * intensity, corona_max * 1.35 * intensity, mie, origin_glow);
}

// ============================================================================
// Evaluate 3D Volumetric Storm Sky, Cumulonimbus Cloud Deck & Lightning
// ============================================================================
fn eval_storm_sky(
    rd: vec3<f32>,
    p_proj: vec2<f32>,
    wind_t: f32,
    flash: f32,
    sheet_energy: f32,
    strike_pos_x: vec3<f32>,
    strike_seeds: vec3<f32>,
    strike_intensities: vec3<f32>,
    is_reflection: bool
) -> vec3<f32> {
    let sky_elev = clamp(rd.y, 0.0, 1.0);

    // 1. Deep Brooding Midnight Atmosphere Gradient
    let horizon_col = vec3<f32>(0.005, 0.009, 0.020);
    let mid_sky_col = vec3<f32>(0.003, 0.006, 0.015);
    let zenith_col  = vec3<f32>(0.0015, 0.003, 0.009);
    var sky_col = mix(horizon_col, mix(mid_sky_col, zenith_col, sky_elev), sqrt(sky_elev));

    let moon_dir = normalize(vec3<f32>(-0.32, 0.68, 0.66));
    let moon_cos = max(0.0, dot(rd, moon_dir));
    let m2 = moon_cos * moon_cos;
    let m4 = m2 * m2;
    let m8 = m4 * m4;
    sky_col += vec3<f32>(0.10, 0.16, 0.30) * (m8 * 0.04 + m8 * m8 * m8 * 0.05);

    // 2. Perspective-Foreshortened 3D Cumulonimbus Cloud Deck
    let cloud_persp = 1.15 + (1.0 - sky_elev) * 0.55;
    let cloud_world = vec2<f32>(p_proj.x * 1.12, (1.08 - sky_elev) * 1.65 * cloud_persp);
    let cloud_uv = cloud_world + vec2<f32>(wind_t * 0.038, -wind_t * 0.016);

    let d0 = fbm_storm_cloud(cloud_uv, wind_t);
    let eps_c = vec2<f32>(0.10, 0.0);
    let d_macro_x = fbm_storm_cloud_macro(cloud_uv + eps_c.xy, wind_t) - d0;
    let d_macro_y = fbm_storm_cloud_macro(cloud_uv + eps_c.yx, wind_t) - d0;
    let cloud_normal = normalize(vec3<f32>(-d_macro_x * 2.8, -0.38, -d_macro_y * 2.8));

    let vertical_profile = smoothstep(0.03, 0.22, sky_elev);
    let cloud_density = smoothstep(0.26, 0.76, d0) * vertical_profile;
    let optical_depth = cloud_density * (1.55 + (1.0 - sky_elev) * 1.25);

    let transmittance = exp(-optical_depth * 2.45);
    let powder = 1.0 - exp(-optical_depth * 4.2);
    let beer_powder = transmittance * powder * 2.15;

    let cloud_dark_belly = vec3<f32>(0.0012, 0.0022, 0.0055);
    let cloud_ambient_edge = vec3<f32>(0.022, 0.034, 0.064);
    var cloud_col = mix(cloud_dark_belly, cloud_ambient_edge, beer_powder * (0.40 + moon_cos * 0.60));

    // 3. In-Cloud Sheet Lightning (Anvil Crawler Discharges inside cloud crevices)
    let sheet_wave = sin(cloud_uv.x * 2.1 + wind_t * 1.1) * cos(cloud_uv.y * 1.9 - wind_t * 0.8);
    let sheet_pocket = smoothstep(0.20, 0.85, sheet_wave * 0.5 + 0.5);
    let internal_scatter = exp(-optical_depth * 1.55) * (1.0 - exp(-optical_depth * 3.6));
    let sheet_glow = vec3<f32>(0.28, 0.42, 0.82) * (flash * 0.42 + sheet_energy * 0.36) * internal_scatter * sheet_pocket;
    cloud_col += sheet_glow;

    // 4. Evaluate Global Staggered 3D Lightning Strikes & Volumetric Cloud Illumination
    var bolts_hdr = vec3<f32>(0.0);
    let bolt_tints = array<vec3<f32>, 3>(
        vec3<f32>(0.58, 0.78, 1.35),
        vec3<f32>(0.76, 0.85, 1.48),
        vec3<f32>(0.78, 0.64, 1.38)
    );
    let bolt_y_tops = array<f32, 3>(0.72, 0.88, 0.66);
    let bolt_scales = array<f32, 3>(0.84, 1.08, 0.76);

    for (var s = 0; s < 3; s = s + 1) {
        let s_int = strike_intensities[s];
        if (s_int > 0.002) {
            let s_x = strike_pos_x[s];
            let s_seed = strike_seeds[s];
            let tint = bolt_tints[s];
            let y_top = bolt_y_tops[s];

            let bolt_eval = lightning_bolt(p_proj, s_x, y_top, bolt_scales[s], s_seed, s_int);
            let core_col = vec3<f32>(1.40, 1.46, 1.60) * bolt_eval.x;
            let corona_col = tint * bolt_eval.y;
            let mie_col = tint * 0.60 * bolt_eval.z;
            bolts_hdr += core_col + corona_col + mie_col;

            let delta_cloud = vec2<f32>(p_proj.x - s_x, p_proj.y - y_top * 0.85);
            let dist_sq_cloud = dot(delta_cloud, delta_cloud);
            let to_strike_dir = normalize(vec3<f32>(-delta_cloud.x, -0.36, -delta_cloud.y));
            let n_dot_l = max(0.0, dot(cloud_normal, to_strike_dir) * 0.65 + 0.35);

            let cloud_strike_atten = 1.0 / (1.0 + dist_sq_cloud * 5.0 + dist_sq_cloud * dist_sq_cloud * 16.0);
            let multi_scatter = (beer_powder * 0.90 + internal_scatter * 0.85) * n_dot_l;
            cloud_col += tint * (s_int * cloud_strike_atten * multi_scatter * 1.40 + bolt_eval.w * (0.45 + 0.55 * internal_scatter) * 0.85);
        }
    }

    sky_col = mix(sky_col, cloud_col, clamp(cloud_density * 0.94, 0.0, 0.96));

    let horizon_glow = exp(-sky_elev * 5.5);
    sky_col += vec3<f32>(0.12, 0.19, 0.38) * flash * (0.06 + 0.18 * horizon_glow);

    let upper_cloud_occ = mix(1.0, 0.28 + 0.72 * transmittance, smoothstep(0.55, 0.88, p_proj.y));
    let refl_scale = select(1.0, 0.82, is_reflection);
    sky_col += bolts_hdr * upper_cloud_occ * refl_scale;

    let virga_env = smoothstep(0.02, 0.22, p_proj.y) * smoothstep_r(0.65, 0.28, p_proj.y);
    let virga_u = p_proj.x * 1.8 + p_proj.y * 0.28 + wind_t * 0.06;
    let virga_n = noise2d_fast(vec2<f32>(virga_u, p_proj.y * 0.8));
    let virga_shaft = smoothstep(0.38, 0.78, virga_n) * virga_env;
    sky_col += vec3<f32>(0.16, 0.24, 0.44) * virga_shaft * (0.02 + flash * 0.16);

    return sky_col;
}

// ============================================================================
// 3D Storm Ocean Surface Heightfield & Analytic Normal
// ============================================================================
fn eval_ocean_normal(world_xz: vec2<f32>, dist: f32, time: f32, bass: f32, treble: f32) -> vec3<f32> {
    let swell_atten = 1.0 / (1.0 + dist * 0.075 + dist * dist * 0.010);
    let detail_atten = 1.0 / (1.0 + dist * 0.16 + dist * dist * 0.028);
    let ripple_atten = 1.0 / (1.0 + dist * 0.28 + dist * dist * 0.065);

    let d1 = vec2<f32>(0.88, 0.47);
    let d2 = vec2<f32>(-0.74, 0.67);
    let d3 = vec2<f32>(0.32, 0.95);
    let d4 = vec2<f32>(-0.94, 0.34);

    let amp_swell = (0.042 + bass * 0.052) * swell_atten;
    let p1 = dot(world_xz, d1) * 1.45 + time * 2.3;
    let p2 = dot(world_xz, d2) * 2.35 - time * 2.9 + sin(p1) * 0.35;
    let p3 = dot(world_xz, d3) * 4.60 + time * 4.1;
    let p4 = dot(world_xz, d4) * 8.20 - time * 5.4;

    var grad = d1 * (cos(p1) * 1.45 * amp_swell)
             + d2 * (cos(p2) * 2.35 * amp_swell * 0.65)
             + d3 * (cos(p3) * 4.60 * (0.012 + treble * 0.016) * detail_atten)
             + d4 * (cos(p4) * 8.20 * (0.006 + treble * 0.010) * detail_atten);

    if (ripple_atten > 0.05) {
        let rain_uv = world_xz * 2.1;
        let cell_id = floor(rain_uv);
        let cell_local = fract(rain_uv) - 0.5;
        let r_rnd = hash22_fast(cell_id);
        let r_phase = fract(time * (1.35 + r_rnd.x * 0.75) + r_rnd.y);
        let center_offset = (r_rnd - 0.5) * 0.40;
        let delta = cell_local - center_offset;
        let r_dist = length(delta) + 1e-4;
        let ring_r = r_phase * 0.42;
        let diff_r = r_dist - ring_r;

        let cell_edge_fade = smoothstep_r(0.48, 0.30, max(abs(cell_local.x), abs(cell_local.y)));
        let env = exp(-diff_r * diff_r * 420.0) * (1.0 - r_phase) * (1.0 - r_phase) * cell_edge_fade;
        let d_ring = sin(diff_r * 65.0) * env * (0.075 + treble * 0.085) * ripple_atten;
        grad += (delta / r_dist) * d_ring;
    }

    return normalize(vec3<f32>(-grad.x, 1.0, -grad.y));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let safe_aspect = max(audio.aspect_ratio, 0.2);
    let p = vec2<f32>(uv.x * safe_aspect, -uv.y);

    // --- 1. Multi-Band Audio & Channel Peak Analysis ---
    let sub_bass = max(audio.spectrum[0].x, max(audio.spectrum[1].x, audio.spectrum[2].x)) / 78.0;
    let transient_e = (audio.spectrum[1].x + audio.spectrum[2].x + audio.spectrum[3].x + audio.spectrum[4].x) / 300.0;
    let low_mid = (audio.spectrum[8].x + audio.spectrum[12].x + audio.spectrum[16].x) / 220.0;
    let high_mid = (audio.spectrum[28].x + audio.spectrum[36].x + audio.spectrum[44].x) / 210.0;
    let treble = (audio.spectrum[52].x + audio.spectrum[64].x + audio.spectrum[76].x) / 200.0;

    let num_ch = clamp(audio.num_channels, 1u, 32u);
    var ch_left = 0.0;
    var ch_center = 0.0;
    var ch_right = 0.0;
    for (var i = 0u; i < num_ch; i = i + 1u) {
        let vec_idx = i / 4u;
        let comp_idx = i % 4u;
        let sig = max(audio.channels[vec_idx][comp_idx] * 1.15, audio.channel_peaks[vec_idx][comp_idx]);
        let bucket = i % 3u;
        if (bucket == 0u) {
            ch_left = max(ch_left, sig);
        } else if (bucket == 1u) {
            ch_center = max(ch_center, sig);
        } else {
            ch_right = max(ch_right, sig);
        }
    }
    if (num_ch == 1u) {
        ch_center = ch_left;
        ch_right = ch_left * 0.85;
    } else if (num_ch == 2u) {
        let l_sig = max(audio.channels[0].x * 1.15, audio.channel_peaks[0].x);
        let r_sig = max(audio.channels[0].y * 1.15, audio.channel_peaks[0].y);
        ch_left = l_sig * 0.92;
        ch_center = max(l_sig, r_sig);
        ch_right = r_sig * 0.92;
    }

    let strike_rate = 4.0;
    let phase_c = audio.smooth_time * strike_rate;
    let phase_l = audio.smooth_time * (strike_rate * 0.86) + 1.37;
    let phase_r = audio.smooth_time * (strike_rate * 0.92) + 2.83;

    let strike_tick = floor(phase_c);
    let tick_l = floor(phase_l);
    let tick_r = floor(phase_r);

    let f_c = fract(phase_c);
    let f_l = fract(phase_l);
    let f_r = fract(phase_r);

    let env_c = clamp(exp(-f_c * 5.0) + exp(-max(f_c - 0.30, 0.0) * 9.5) * step(0.30, f_c) * 0.65, 0.16, 1.0);
    let env_l = clamp(exp(-f_l * 5.4) + exp(-max(f_l - 0.34, 0.0) * 9.5) * step(0.34, f_l) * 0.55, 0.12, 1.0);
    let env_r = clamp(exp(-f_r * 5.2) + exp(-max(f_r - 0.28, 0.0) * 9.5) * step(0.28, f_r) * 0.60, 0.12, 1.0);

    let int_left = smoothstep(0.36, 0.82, max(ch_left, low_mid * 0.95)) * env_l * 0.78;
    let int_center = smoothstep(0.30, 0.78, max(ch_center, max(sub_bass, transient_e * 1.15))) * env_c;
    let int_right = smoothstep(0.36, 0.82, max(ch_right, high_mid * 0.95)) * env_r * 0.75;

    let strike_intensities = vec3<f32>(int_left, int_center, int_right);
    let strike_seeds = vec3<f32>(
        tick_l * 17.31 + 1.37,
        strike_tick * 23.17 + 5.91,
        tick_r * 31.73 + 9.43
    );
    let strike_pos_x = vec3<f32>(
        -safe_aspect * (0.44 + (hash11_fast(strike_seeds.x) - 0.5) * 0.22),
        (hash11_fast(strike_seeds.y) - 0.5) * safe_aspect * 0.30,
        safe_aspect * (0.46 + (hash11_fast(strike_seeds.z) - 0.5) * 0.22)
    );

    let kick_base = clamp(max(sub_bass, transient_e * 1.15), 0.0, 1.0);
    let kick_trigger = kick_base * sqrt(kick_base);
    let max_strike = max(int_center, max(int_left, int_right));
    let flash = clamp(max(smoothstep(0.22, 0.86, kick_trigger), max_strike * 0.80), 0.0, 1.0);
    let sheet_energy = clamp(low_mid * 0.45 + high_mid * 0.55 + clamp(treble, 0.0, 1.0) * 0.35, 0.0, 1.0);

    // --- 2. 3D Perspective Camera Setup ---
    let y_horizon = -0.34;
    let cam_ro = vec3<f32>(sin(audio.smooth_time * 0.18) * 0.15, 1.45, 0.0);
    let ray_y = p.y - y_horizon;
    let rd = normalize(vec3<f32>(p.x * 0.62, ray_y * 0.68, 1.0));
    let wind_t = audio.smooth_time;

    var color = vec3<f32>(0.0);

    if (ray_y >= 0.0) {
        let p_sky = vec2<f32>(p.x, ray_y / (1.0 - y_horizon));
        color = eval_storm_sky(
            rd,
            p_sky,
            wind_t,
            flash,
            sheet_energy,
            strike_pos_x,
            strike_seeds,
            strike_intensities,
            false
        );
    } else {
        let t_water = cam_ro.y / (-rd.y + 0.014);
        let hit_pos = cam_ro + rd * t_water;

        let normal = eval_ocean_normal(
            hit_pos.xz,
            t_water,
            audio.smooth_time,
            clamp(sub_bass, 0.0, 1.0),
            clamp(treble, 0.0, 1.0)
        );
        let view_dir = -rd;
        let n_dot_v = max(0.0, dot(normal, view_dir));

        let nv1 = 1.0 - n_dot_v;
        let nv2 = nv1 * nv1;
        let fresnel = 0.024 + 0.976 * (nv2 * nv2 * nv1);

        var refl_rd = reflect(rd, normal);
        refl_rd.y = max(refl_rd.y, 0.005);
        refl_rd = normalize(refl_rd);

        let refl_p_sky = vec2<f32>(
            p.x + normal.x * 0.24,
            clamp((-ray_y + normal.z * 0.12) / (1.0 - y_horizon), 0.005, 0.98)
        );
        let sky_refl = eval_storm_sky(
            refl_rd,
            refl_p_sky,
            wind_t,
            flash,
            sheet_energy,
            strike_pos_x,
            strike_seeds,
            strike_intensities,
            true
        );

        let water_deep = vec3<f32>(0.001, 0.003, 0.009);
        let water_scatter = vec3<f32>(0.005, 0.015, 0.032) * (0.20 + flash * 0.70) * (1.0 - n_dot_v);
        let water_body = water_deep + water_scatter;

        color = mix(water_body, sky_refl, fresnel);

        let bolt_tints = array<vec3<f32>, 3>(
            vec3<f32>(0.58, 0.78, 1.35),
            vec3<f32>(0.76, 0.85, 1.48),
            vec3<f32>(0.78, 0.64, 1.38)
        );
        for (var s = 0; s < 3; s = s + 1) {
            let s_int = strike_intensities[s];
            if (s_int > 0.01) {
                let s_seed = strike_seeds[s];
                let base_x = strike_pos_x[s] + (hash11_fast(s_seed * 1.91) - 0.5) * 0.24;
                let slant = (hash11_fast(s_seed * 4.73) - 0.5) * 0.28;
                let impact_x = base_x + 0.65 * slant;
                let dx_glitter = (p.x - impact_x) + normal.x * 0.32;
                let glitter_col = exp(-dx_glitter * dx_glitter * 28.0) * (0.35 + 0.65 * max(0.0, normal.z));
                color += bolt_tints[s] * glitter_col * s_int * fresnel * 0.45;
            }
        }
    }

    // --- 4. Seamless Horizon Aerial Perspective Mist ---
    let horizon_mist = exp(-abs(ray_y) * 32.0);
    let mist_col = vec3<f32>(0.007, 0.013, 0.028) + vec3<f32>(0.14, 0.22, 0.42) * (flash * 0.35 + sheet_energy * 0.12);
    color = mix(color, mist_col, horizon_mist * 0.72);

    // --- 5. Multi-Depth 3D Parallax Rain Needles ---
    var rain_accum = vec3<f32>(0.0);
    let wind_gust = 0.095 + 0.025 * sin(audio.smooth_time * 0.35);
    let wind_drift = audio.smooth_time * 0.38 + sin(audio.smooth_time * 0.22) * 0.55;

    let glint_dx = p.x - strike_pos_x.y;
    let strike_glint = 1.0 / (1.0 + glint_dx * glint_dx * 2.2);
    let rain_illum = 0.18 + flash * (0.38 + 0.52 * strike_glint) + clamp(treble, 0.0, 1.0) * 0.18;

    for (var layer = 0; layer < 4; layer = layer + 1) {
        let fl = f32(layer);
        let depth_scale = 1.0 + fl * 0.75;

        let slant = wind_gust * (1.0 + fl * 0.14);
        let scale_x = 26.0 * depth_scale;
        let scale_y = 1.85 + fl * 1.10;
        let speed = 3.8 + fl * 1.25;

        let rx = (p.x + p.y * slant) * scale_x + fl * 19.7 + wind_drift * (1.15 - fl * 0.15);
        let col_id = floor(rx);
        let col_phase = hash11_fast(col_id * 1.73 + fl * 31.7);

        let ry = p.y * scale_y + audio.smooth_time * speed * (0.90 + 0.20 * col_phase) + col_phase * 6.28318;
        let row_id = floor(ry);

        let cell_rnd = hash22_fast(vec2<f32>(col_id, row_id) + vec2<f32>(fl * 43.1, fl * 71.9));
        if (cell_rnd.y > 0.58) {
            let cell_x = fract(rx) - 0.5;
            let drop_x = (cell_rnd.x - 0.5) * 0.58;
            let dx = abs(cell_x - drop_x);
            let half_w = 0.022 / (1.0 + fl * 0.22);

            if (dx < half_w) {
                let cell_y = fract(ry) - 0.5;
                let streak_x = smoothstep_r(half_w, 0.0, dx);

                let drop_len = 0.40 + 0.07 * cell_rnd.x;
                let head = smoothstep(-drop_len, -drop_len + 0.05, cell_y);
                let wake = smoothstep_r(drop_len, -drop_len, cell_y);
                let profile_y = head * wake * wake;

                let layer_weight = 1.0 / (1.0 + fl * 0.70);
                let drop_alpha = streak_x * profile_y * layer_weight;
                rain_accum += vec3<f32>(0.52, 0.65, 0.90) * drop_alpha * rain_illum;
            }
        }
    }
    color += rain_accum;

    // --- 6. Cinema Vignette, Blue-Noise Dither & ACES Filmic Tonemapping ---
    let r2_screen = dot(uv, uv);
    let vignette = 1.0 - smoothstep(0.60, 2.10, r2_screen) * 0.32;
    color *= vignette;

    let dither = (hash21_fast(in.clip_position.xy + fract(audio.smooth_time) * 137.0) - 0.5) * 0.010;
    color += vec3<f32>(dither);

    let final_col = aces_tonemap(max(color, vec3<f32>(0.0)));
    return vec4<f32>(final_col, 1.0);
}
