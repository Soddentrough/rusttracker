// INCLUDE: common

// ============================================================================
// Visualizer ID 27: Quantum Ferrofluid
// Analytic 3D SDF Rosensweig Ferrofluid Sculpture & Studio Optics Raymarcher
// Mode 0 (Default, toggle `c`): Levitating 3D Magnetic Core over Titanium Coil Pedestal
// Mode 1 (Toggle `c`): Macro Hexagonal Rosensweig Ferrofluid Pool in Studio Dish
// ============================================================================

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318530;
const INV_TAU: f32 = 0.15915494;

// Material IDs
const MAT_NONE: i32 = 0;
const MAT_FERRO: i32 = 1;
const MAT_TITANIUM: i32 = 2;
const MAT_COIL: i32 = 3;

var<private> g_time: f32 = 0.0;
var<private> g_pool_mode: bool = false;
var<private> g_sub_bass: f32 = 0.0;
var<private> g_low_mid: f32 = 0.0;
var<private> g_mids: f32 = 0.0;
var<private> g_highs: f32 = 0.0;
var<private> g_tilt: f32 = 0.0;
var<private> g_activity: f32 = 0.0;

var<private> g_core_center: vec3<f32> = vec3<f32>(0.0, 1.28, 0.0);
var<private> g_core_base_r: f32 = 0.48;
var<private> g_rot_c: f32 = 1.0;
var<private> g_rot_s: f32 = 0.0;
var<private> g_tilt_c: f32 = 1.0;
var<private> g_tilt_s: f32 = 0.0;
var<private> g_sat0: vec3<f32> = vec3<f32>(1.04, 1.28, 0.0);
var<private> g_sat1: vec3<f32> = vec3<f32>(-0.98, 1.22, 0.0);

fn spec_bin(idx: u32) -> f32 {
    let clamped = min(idx, 1023u);
    let v = audio.spectrum[clamped >> 2u];
    let lane = clamped & 3u;
    if (lane == 0u) { return v.x; }
    if (lane == 1u) { return v.y; }
    if (lane == 2u) { return v.z; }
    return v.w;
}

fn coil_band_raw(k: u32) -> f32 {
    // 16 log-spaced spectrum bands across ~28 Hz .. ~7 kHz (raw range 0..100)
    let bin = 56u + (k & 15u) * 58u;
    let avg = (spec_bin(bin) + spec_bin(bin + 14u)) * 0.5;
    return clamp(avg / 48.0, 0.0, 1.5);
}

fn coil_band_smooth(angle: f32) -> f32 {
    let u = fract(angle * INV_TAU) * 16.0;
    let fu = floor(u);
    let i0 = u32(fu) & 15u;
    let i1 = (i0 + 1u) & 15u;
    let f = u - fu;
    let s = f * f * (3.0 - 2.0 * f);
    return mix(coil_band_raw(i0), coil_band_raw(i1), s);
}

// Smooth periodic color wheel around pedestal: seamless transition from warm amber-gold
// (low coils) through silver-teal to electric cyan-cobalt and back without an angle=0 seam.
fn coil_palette_angle(angle: f32) -> vec3<f32> {
    let wave = 0.5 + 0.5 * cos(angle + 0.35);
    let amber = vec3<f32>(1.28, 0.56, 0.14);
    let teal = vec3<f32>(0.22, 0.82, 1.08);
    let cobalt = vec3<f32>(0.20, 0.52, 1.38);
    return mix(mix(cobalt, teal, smoothstep(0.0, 0.55, wave)), amber, smoothstep(0.45, 1.0, wave));
}

fn coil_palette(idx: u32) -> vec3<f32> {
    let angle = (f32(idx & 15u) + 0.5) * (TAU / 16.0);
    return coil_palette_angle(angle);
}

// Concave catenoid-flare Rosensweig needle profile:
// tight micro-rounded tip, slender pointed waist, wide zero-slope meniscus collar.
fn rosensweig_spike_dot(c_dot: f32, sharpness: f32, tip_eps: f32) -> f32 {
    let t2 = max(0.0, 1.0 - c_dot) * sharpness;
    let cone = sqrt(t2 + tip_eps) - sqrt(tip_eps);
    let w = max(0.0, 1.0 - cone);
    let w2 = w * w;
    return w2 * w2 / (0.34 + 0.66 * w2);
}

fn rosensweig_spike_r2(d2: f32, sharpness: f32, tip_eps: f32) -> f32 {
    let t2 = max(0.0, d2) * sharpness;
    let cone = sqrt(t2 + tip_eps) - sqrt(tip_eps);
    let w = max(0.0, 1.0 - cone);
    let w2 = w * w;
    return w2 * w / (0.38 + 0.62 * w2);
}

fn sd_capped_cylinder(p: vec3<f32>, r: f32, h: f32) -> f32 {
    let d = abs(vec2<f32>(length(p.xz), p.y)) - vec2<f32>(r, h);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0)));
}

fn sd_chamfer_cylinder(p: vec3<f32>, r: f32, h: f32, bevel: f32) -> f32 {
    let hr = r - bevel;
    let hh = h - bevel;
    let d = abs(vec2<f32>(length(p.xz), p.y)) - vec2<f32>(hr, hh);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0))) - bevel;
}

// Pedestal SDF shared by Mode 0 (compact magnetic core stage)
fn map_pedestal_core(p: vec3<f32>) -> f32 {
    let base_outer = sd_chamfer_cylinder(p - vec3<f32>(0.0, 0.115, 0.0), 1.62, 0.115, 0.025);
    let flange = sd_chamfer_cylinder(p - vec3<f32>(0.0, 0.032, 0.0), 1.72, 0.032, 0.014);
    var d_ped = min(base_outer, flange);
    // Clean recessed inner well (r = 1.26) leaving clear clearance around the ferrofluid pool (r = 1.21)
    let well_cut = sd_capped_cylinder(p - vec3<f32>(0.0, 0.26, 0.0), 1.26, 0.12);
    d_ped = smax_poly(d_ped, -well_cut, 0.012);
    return d_ped;
}

// Mode 0: Levitating 3D ferrofluid core + recessed pool + orbiting droplets
// Evaluated seamlessly across 3D space (zero discontinuous bounding-branch switches inside smin_poly)
fn map_ferro_core(p: vec3<f32>) -> f32 {
    let q_world = p - g_core_center;
    let r2 = dot(q_world, q_world);
    let r_core = sqrt(max(r2, 1e-6));

    // Orbiting miniature ferrofluid droplets
    let d_sat0 = length(p - g_sat0) - (0.064 + g_sub_bass * 0.015);
    let d_sat1 = length(p - g_sat1) - (0.052 + g_mids * 0.014);
    let d_sats = min(d_sat0, d_sat1);

    // Recessed ferrofluid pool sitting cleanly inside r <= 1.21
    let pr2 = dot(p.xz, p.xz);
    let pr = sqrt(max(pr2, 1e-6));
    let pool_edge = smoothstep_r(1.20, 0.96, pr);
    let ripple = sin(pr * 14.0 - g_time * 3.8) * (0.004 + g_low_mid * 0.008);
    let stalagmite_h = (0.14 + g_sub_bass * 0.24) / (1.0 + pr2 * 4.5 + pr2 * pr2 * 7.5);
    let pool_top = 0.202 + stalagmite_h * pool_edge + ripple * pool_edge;
    let d_pool_surf = (p.y - pool_top) * 0.62;
    let d_pool_cyl = pr - 1.21;
    let d_pool = smax_poly(max(d_pool_surf, d_pool_cyl), -(p.y - 0.12), 0.018);

    // Rotate into core-local coordinate frame
    let qx = vec3<f32>(
        q_world.x * g_rot_c - q_world.z * g_rot_s,
        q_world.y,
        q_world.x * g_rot_s + q_world.z * g_rot_c
    );
    let q_loc = vec3<f32>(
        qx.x * g_tilt_c - qx.y * g_tilt_s,
        qx.x * g_tilt_s + qx.y * g_tilt_c,
        qx.z
    );
    let dir = q_loc / r_core;
    let ad = abs(dir);

    // Three mutually disjoint, mirror-crease-free spherical Voronoi spike sets:
    // 1) 12 Primary Icosahedral Crown Needles
    let inv_l = 0.52573111;
    let phi_l = 0.85065081;
    let ic1 = ad.y * inv_l + ad.z * phi_l;
    let ic2 = ad.z * inv_l + ad.x * phi_l;
    let ic3 = ad.x * inv_l + ad.y * phi_l;
    let c12 = max(ic1, max(ic2, ic3));

    // 2) 20 Secondary Dodecahedral Needles (TRUE dual face centers of the icosahedron, 37.38 deg from every primary vertex)
    let dd0 = (ad.x + ad.y + ad.z) * 0.57735027;
    let dd1 = ad.x * 0.93417236 + ad.y * 0.35682209;
    let dd2 = ad.y * 0.93417236 + ad.z * 0.35682209;
    let dd3 = ad.z * 0.93417236 + ad.x * 0.35682209;
    let c20 = max(max(dd0, dd1), max(dd2, dd3));

    // Guard atan2 near vertical Y poles for C-infinity smoothness everywhere
    let rho_xz = length(q_world.xz);
    let raw_azi_e = coil_band_smooth(atan2(q_world.z, q_world.x));
    let azi_e = mix(g_sub_bass, raw_azi_e, smoothstep(0.05, 0.25, rho_xz));
    let breath = 0.88 + 0.12 * sin(q_world.x * 2.2 + q_world.z * 2.2 - g_time * 1.45);

    // Sharpness chosen so every spike collar vanishes cleanly before adjacent vertices:
    // c12 (31.7 deg seam) -> 10.5; c20 (20.9 deg seam, 37.4 deg from c12) -> 18.5
    let w12 = rosensweig_spike_dot(c12, 10.5, 0.0028);
    let w20 = rosensweig_spike_dot(c20, 18.5, 0.0022);

    let h12 = (0.14 + g_sub_bass * 0.24 + azi_e * 0.08) * breath;
    let h20 = 0.08 + g_low_mid * 0.16 + g_mids * 0.12 + g_highs * 0.10 + azi_e * 0.05;

    let dipole_stretch = 1.0 + 0.04 * (dir.y * dir.y);
    let target_r = g_core_base_r * dipole_stretch + w12 * h12 + w20 * h20;
    let d_core = (r_core - target_r) * 0.34;

    // Smooth catenoid magnetic neck between hovering core and pedestal pool
    let bridge_blend = 0.13 + g_sub_bass * 0.14;
    var d_fluid = smin_poly(d_core, d_pool, bridge_blend);
    d_fluid = smin_poly(d_fluid, d_sats, 0.075);
    return d_fluid;
}

// Mode 1: Macro Studio Dish with O(1) Hexagonal Rosensweig Spike Lattice
fn map_pedestal_pool(p: vec3<f32>) -> f32 {
    let base_outer = sd_chamfer_cylinder(p - vec3<f32>(0.0, 0.115, 0.0), 2.68, 0.115, 0.025);
    let flange = sd_chamfer_cylinder(p - vec3<f32>(0.0, 0.032, 0.0), 2.78, 0.032, 0.014);
    var d_ped = min(base_outer, flange);
    let well_cut = sd_capped_cylinder(p - vec3<f32>(0.0, 0.25, 0.0), 2.42, 0.12);
    d_ped = smax_poly(d_ped, -well_cut, 0.012);
    return d_ped;
}

fn eval_pool_height_at(p_xz: vec2<f32>) -> f32 {
    let r2_w = dot(p_xz, p_xz);
    let r_w = sqrt(max(r2_w, 1e-6));
    let rim_fade = smoothstep_r(2.38, 1.92, r_w);

    let dome = (0.040 + g_sub_bass * 0.075) / (1.0 + r2_w * 0.55) * rim_fade;
    let radial_wave = sin(r_w * 5.2 - g_time * 3.6) * (0.008 + g_low_mid * 0.022) * rim_fade / (1.0 + r2_w * 0.42);

    // Hexagonal lattice lookup in scaled coordinates
    let lat_scale = 2.45;
    let lp = p_xz * lat_scale;
    let s = vec2<f32>(1.0, 1.73205081);
    let h = s * 0.5;
    let cell_a = lp - s * floor(lp / s + 0.5);
    let lp_b = lp - h;
    let cell_b = lp_b - s * floor(lp_b / s + 0.5);

    let da2 = dot(cell_a, cell_a);
    let db2 = dot(cell_b, cell_b);
    let use_a = da2 < db2;
    let rel = select(cell_b, cell_a, use_a);
    let d2 = select(db2, da2, use_a);

    // Exact Voronoi cell site ensures every spike cone is axially symmetric
    let site_w = (lp - rel) / lat_scale;
    let site_r2 = dot(site_w, site_w);
    let site_r = sqrt(max(site_r2, 1e-6));
    // Blend azimuth smoothly to azimuth-invariant bass at center site (0,0) -> zero atan2(0,0) jitter!
    let site_band = mix(g_sub_bass, coil_band_smooth(atan2(site_w.y, site_w.x)), smoothstep(0.15, 0.65, site_r));

    let spike_prof = rosensweig_spike_r2(d2, 5.0, 0.013);

    let center_env = 1.0 / (1.0 + site_r2 * 0.58);
    let ring_wave = 0.65 + 0.35 * cos(site_r * 3.4 - g_time * 2.40);
    let audio_drive = 0.34 + g_sub_bass * 0.38 * center_env + g_low_mid * 0.20 + g_mids * 0.14 * ring_wave + site_band * 0.12 + g_highs * 0.07 * ring_wave;
    let dish_taper = smoothstep_r(2.28, 1.55, site_r);
    let spike_h = spike_prof * (0.38 * audio_drive * (0.45 + 0.55 * center_env) * dish_taper);

    return 0.192 + dome + radial_wave + spike_h;
}

fn map_ferro_pool(p: vec3<f32>) -> f32 {
    let pr = length(p.xz);
    let surf_h = eval_pool_height_at(p.xz);
    let d_surf = (p.y - surf_h) * 0.25;
    let d_cyl = pr - 2.38;
    return smax_poly(max(d_surf, d_cyl), -(p.y - 0.12), 0.015);
}

fn map_dist(p: vec3<f32>) -> f32 {
    if (g_pool_mode) {
        return min(map_ferro_pool(p), map_pedestal_pool(p));
    }
    return min(map_ferro_core(p), map_pedestal_core(p));
}

// Return material ID + coil index at hit point
fn eval_surface_material(p: vec3<f32>) -> vec2<f32> {
    var d_ferro: f32;
    var d_ped: f32;
    var r_slit_min: f32;
    var r_slit_max: f32;
    if (g_pool_mode) {
        d_ferro = map_ferro_pool(p);
        d_ped = map_pedestal_pool(p);
        r_slit_min = 2.48;
        r_slit_max = 2.58;
    } else {
        d_ferro = map_ferro_core(p);
        d_ped = map_pedestal_core(p);
        r_slit_min = 1.38;
        r_slit_max = 1.48;
    }

    if (d_ferro < d_ped) {
        return vec2<f32>(f32(MAT_FERRO), 0.0);
    }

    let r = length(p.xz);
    if (p.y > 0.215 && r > r_slit_min && r < r_slit_max) {
        let ang = atan2(p.z, p.x);
        let u = fract(ang * INV_TAU) * 16.0;
        let idx = floor(u);
        let cell_pos = abs((u - idx) - 0.5);
        if (cell_pos < 0.095) {
            return vec2<f32>(f32(MAT_COIL), idx);
        }
    }
    return vec2<f32>(f32(MAT_TITANIUM), 0.0);
}

// 4-tap Tetrahedron SDF Normal
fn calc_sdf_normal(p: vec3<f32>, eps: f32) -> vec3<f32> {
    let k0 = vec3<f32>(1.0, -1.0, -1.0);
    let k1 = vec3<f32>(-1.0, -1.0, 1.0);
    let k2 = vec3<f32>(-1.0, 1.0, -1.0);
    let k3 = vec3<f32>(1.0, 1.0, 1.0);
    return normalize(
        k0 * map_dist(p + k0 * eps) +
        k1 * map_dist(p + k1 * eps) +
        k2 * map_dist(p + k2 * eps) +
        k3 * map_dist(p + k3 * eps)
    );
}

// Fast 3-tap SDF valley ambient occlusion
fn calc_sdf_ao(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var occ = 0.0;
    var w = 1.0;
    for (var i = 0; i < 3; i++) {
        let h = 0.025 + 0.065 * f32(i);
        let d = map_dist(p + n * h);
        occ += (h - d) * w;
        w *= 0.48;
    }
    return clamp(1.0 - 2.35 * occ, 0.04, 1.0);
}

// 5-tap directional soft self-shadow toward overhead studio key
fn calc_soft_shadow(ro: vec3<f32>, rd: vec3<f32>) -> f32 {
    var res = 1.0;
    var t = 0.05;
    for (var i = 0; i < 5; i++) {
        let h = map_dist(ro + rd * t);
        if (h < 0.002) {
            return 0.06;
        }
        res = min(res, 6.5 * h / t);
        t += clamp(h, 0.035, 0.22);
        if (t > 2.4) {
            break;
        }
    }
    return clamp(res, 0.06, 1.0);
}

// High-contrast studio HDRI reflection dome for mirror-glossy ferrofluid
fn eval_studio_dome(pos: vec3<f32>, ref_dir: vec3<f32>) -> vec3<f32> {
    let elev = smoothstep(-0.20, 0.85, ref_dir.y);
    var col = mix(vec3<f32>(0.010, 0.014, 0.022), vec3<f32>(0.003, 0.005, 0.010), elev);

    // Dual overhead rectangular studio softboxes at y = 5.4 separated by dark center louver
    if (ref_dir.y > 0.04) {
        let t_sb = (5.4 - max(pos.y, 0.0)) / ref_dir.y;
        let hit_sb = pos.xz + ref_dir.xz * min(t_sb, 26.0);
        let sb_abs_x = abs(hit_sb.x);
        let strip_x = smoothstep(0.32, 0.58, sb_abs_x) * smoothstep_r(2.70, 2.15, sb_abs_x);
        let strip_z = smoothstep_r(3.40, 2.50, abs(hit_sb.y));
        let louver = 0.90 + 0.10 * cos(hit_sb.y * 4.5);
        let softbox_power = (2.25 + g_sub_bass * 0.45) * strip_x * strip_z * louver;
        col += vec3<f32>(1.04, 1.08, 1.16) * softbox_power;
    }

    // Surgical left electric cyan-cobalt vertical rim strip
    let l_cyan = normalize(vec3<f32>(-0.84, 0.28, -0.46));
    let d_cyan = max(0.0, dot(ref_dir, l_cyan));
    let c2 = d_cyan * d_cyan;
    let c4 = c2 * c2;
    let c16 = c4 * c4 * c4 * c4;
    let c64 = c16 * c16 * c16 * c16;
    let rim_cyan = (c16 * c4) * 0.35 + c64 * 2.20;
    col += vec3<f32>(0.16, 0.78, 1.65) * rim_cyan * (0.80 + g_mids * 0.45);

    // Surgical right warm amber-gold kicker rim strip
    let l_amber = normalize(vec3<f32>(0.82, 0.34, 0.46));
    let d_amber = max(0.0, dot(ref_dir, l_amber));
    let a2 = d_amber * d_amber;
    let a4 = a2 * a2;
    let a16 = a4 * a4 * a4 * a4;
    let a64 = a16 * a16 * a16 * a16;
    let rim_amber = (a16 * a4) * 0.32 + a64 * 2.05;
    col += vec3<f32>(1.55, 0.80, 0.28) * rim_amber * (0.80 + g_sub_bass * 0.40);

    // Subtle coil bounce onto lower spike undersides (smoothly periodic in angle)
    if (ref_dir.y < 0.14) {
        let bounce_mask = smoothstep_r(0.14, -0.35, ref_dir.y) * smoothstep_r(2.4, 0.5, length(pos.xz));
        let ang = atan2(ref_dir.z, ref_dir.x);
        let coil_e = 0.08 + coil_band_smooth(ang) * 0.65;
        col += coil_palette_angle(ang) * (bounce_mask * coil_e * 0.15);
    }

    return col;
}

// Diffused/satin reflection lobe for honed studio slate bench
fn eval_satin_bench_dome(ref_dir: vec3<f32>) -> vec3<f32> {
    let elev = smoothstep(-0.15, 0.80, ref_dir.y);
    var col = mix(vec3<f32>(0.012, 0.016, 0.026), vec3<f32>(0.004, 0.006, 0.012), elev);

    let ov_h = max(0.0, ref_dir.y);
    let ov2 = ov_h * ov_h;
    let ov4 = ov2 * ov2;
    col += vec3<f32>(0.09, 0.10, 0.12) * (ov4 * ov2);

    let l_cyan = normalize(vec3<f32>(-0.84, 0.28, -0.46));
    let dc = max(0.0, dot(ref_dir, l_cyan));
    let l_amber = normalize(vec3<f32>(0.82, 0.34, 0.46));
    let da = max(0.0, dot(ref_dir, l_amber));
    col += vec3<f32>(0.06, 0.24, 0.45) * (dc * dc * dc * dc) * (0.65 + g_mids * 0.35);
    col += vec3<f32>(0.36, 0.20, 0.07) * (da * da * da * da) * (0.65 + g_sub_bass * 0.35);
    return col;
}

fn eval_studio_backdrop(rd: vec3<f32>) -> vec3<f32> {
    let sky_t = smoothstep(-0.08, 0.72, rd.y);
    var col = mix(vec3<f32>(0.014, 0.018, 0.028), vec3<f32>(0.004, 0.006, 0.011), sky_t);
    let hz = 1.0 / (1.0 + abs(rd.y - 0.04) * 12.0 + rd.y * rd.y * 28.0);
    let side = clamp(rd.x * 0.5 + 0.5, 0.0, 1.0);
    let halo_tint = mix(vec3<f32>(0.05, 0.14, 0.24), vec3<f32>(0.15, 0.08, 0.04), side);
    col += halo_tint * (hz * (0.14 + g_activity * 0.09));
    return col;
}

fn intersect_sphere(ro: vec3<f32>, rd: vec3<f32>, center: vec3<f32>, radius: f32) -> vec2<f32> {
    let oc = ro - center;
    let b = dot(oc, rd);
    let c = dot(oc, oc) - radius * radius;
    let disc = b * b - c;
    if (disc < 0.0) {
        return vec2<f32>(1e9, -1e9);
    }
    let s = sqrt(disc);
    return vec2<f32>(-b - s, -b + s);
}

fn shade_ferrofluid(p: vec3<f32>, rd: vec3<f32>, n: vec3<f32>, ao: f32) -> vec3<f32> {
    let v = -rd;
    let ndotv = max(0.0, dot(n, v));

    // Hydrocarbon oil-carrier dielectric Fresnel (F0 = 0.068 via pure squaring)
    let nv = 1.0 - ndotv;
    let nv2 = nv * nv;
    let fresnel = 0.068 + 0.932 * (nv2 * nv2 * nv);

    let key_dir = normalize(vec3<f32>(0.25, 0.92, 0.28));
    let shadow = calc_soft_shadow(p + n * 0.006, key_dir);

    let ref_dir = reflect(rd, n);
    let env = eval_studio_dome(p, ref_dir);

    // Smooth dual-lobe key highlight + rim highlights (anti-aliased at fine needle tips)
    let h_key = normalize(key_dir + v);
    let nh_k = max(0.0, dot(n, h_key));
    let hk2 = nh_k * nh_k;
    let hk4 = hk2 * hk2;
    let hk16 = hk4 * hk4 * hk4 * hk4;
    let hk64 = hk16 * hk16 * hk16 * hk16;
    let spec_key = (hk64 * hk16) * 2.2 + hk16 * 0.22;

    let l_cyan = normalize(vec3<f32>(-0.84, 0.28, -0.46));
    let h_cyan = normalize(l_cyan + v);
    let hc = max(0.0, dot(n, h_cyan));
    let hc2 = hc * hc;
    let hc4 = hc2 * hc2;
    let hc16 = hc4 * hc4 * hc4 * hc4;
    let hc64 = hc16 * hc16 * hc16 * hc16;
    let spec_cyan = (hc64 * hc64) * (1.45 + g_mids * 0.60);

    let l_amber = normalize(vec3<f32>(0.82, 0.34, 0.46));
    let h_amber = normalize(l_amber + v);
    let ha = max(0.0, dot(n, h_amber));
    let ha2 = ha * ha;
    let ha4 = ha2 * ha2;
    let ha16 = ha4 * ha4 * ha4 * ha4;
    let ha64 = ha16 * ha16 * ha16 * ha16;
    let spec_amber = (ha64 * ha64) * (1.35 + g_sub_bass * 0.60);

    let irid_strength = (nv2 * nv2) * (0.040 + g_highs * 0.055) * ao;
    let irid = iridescence_cos(ndotv * 2.4 + g_time * 0.15) * irid_strength;

    let base_albedo = vec3<f32>(0.0022, 0.0028, 0.0042) * ao;
    let refl_term = (env + irid) * fresnel * ao * (0.28 + 0.72 * shadow);
    let spec_term = (
        vec3<f32>(1.18, 1.22, 1.30) * spec_key * shadow +
        vec3<f32>(0.18, 0.85, 1.65) * spec_cyan +
        vec3<f32>(1.60, 0.84, 0.28) * spec_amber
    ) * ao * (0.22 + 0.78 * fresnel);

    return base_albedo + refl_term + spec_term;
}

fn shade_titanium_pedestal(p: vec3<f32>, rd: vec3<f32>, n: vec3<f32>, ao: f32) -> vec3<f32> {
    let v = -rd;
    let ndotv = max(0.0, dot(n, v));
    let nv = 1.0 - ndotv;
    let nv2 = nv * nv;
    let fresnel = 0.07 + 0.38 * (nv2 * nv2);

    let r = length(p.xz);
    let brush = 0.92 + 0.08 * sin(r * 90.0);

    let key_dir = normalize(vec3<f32>(0.25, 0.92, 0.28));
    let shadow = calc_soft_shadow(p + n * 0.006, key_dir);
    let ndotl = max(0.0, dot(n, key_dir));

    let ref_dir = reflect(rd, n);
    let env = eval_satin_bench_dome(ref_dir);

    let h_key = normalize(key_dir + v);
    let nh = max(0.0, dot(n, h_key));
    let nh2 = nh * nh;
    let nh4 = nh2 * nh2;
    let nh16 = nh4 * nh4 * nh4 * nh4;

    let diffuse = vec3<f32>(0.030, 0.034, 0.042) * brush * (0.18 + 0.82 * ndotl * shadow) * ao;
    let refl = env * (fresnel * ao * (0.38 + 0.62 * shadow));
    let spec = vec3<f32>(0.38, 0.42, 0.48) * (nh16 * ndotl * shadow * ao);

    // Smooth continuous LED bleed flanking micro-slits (zero sector seam)
    let ang = atan2(p.z, p.x);
    let slit_r = select(1.43, 2.53, g_pool_mode);
    let dr = abs(r - slit_r);
    let coil_prox = exp(-dr * dr * 85.0) * smoothstep(0.16, 0.23, p.y);
    let coil_e = 0.08 + coil_band_smooth(ang) * 0.85;
    let local_coil = coil_palette_angle(ang) * (coil_prox * coil_e * 0.09);

    return diffuse + refl + spec + local_coil;
}

fn shade_studio_bench(p: vec3<f32>, rd: vec3<f32>) -> vec3<f32> {
    let n = vec3<f32>(0.0, 1.0, 0.0);
    let v = -rd;
    let ndotv = max(0.0, dot(n, v));
    let nv = 1.0 - ndotv;
    let nv2 = nv * nv;
    let fresnel = 0.032 + 0.34 * (nv2 * nv2 * nv);

    let r2 = dot(p.xz, p.xz);
    let r = sqrt(max(r2, 1e-6));

    let lathe = 0.96 + 0.04 * sin(r * 36.0) * smoothstep_r(8.0, 2.0, r);
    let ped_r = select(1.74, 2.80, g_pool_mode);

    let ped_ao = smoothstep(ped_r - 0.12, ped_r + 0.95, r);
    let core_shadow_offset = p.xz - vec2<f32>(-0.22, -0.24);
    let core_shadow = smoothstep(0.65, 2.35, length(core_shadow_offset));
    let contact = mix(ped_ao, ped_ao * (0.48 + 0.52 * core_shadow), select(1.0, 0.25, g_pool_mode));

    let ref_dir = reflect(rd, n);
    var env = eval_satin_bench_dome(ref_dir);

    let b_center = select(vec3<f32>(0.0, 1.12, 0.0), vec3<f32>(0.0, 0.34, 0.0), g_pool_mode);
    let b_rad = select(1.48, 2.40, g_pool_mode);
    let refl_occ = intersect_sphere(p, ref_dir, b_center, b_rad);
    if (refl_occ.y > refl_occ.x && refl_occ.y > 0.0) {
        let occ_t = clamp((refl_occ.y - refl_occ.x) / (b_rad * 1.4), 0.0, 1.0);
        env *= mix(1.0, 0.12, occ_t * smoothstep_r(6.0, 2.0, r));
    }

    // Smooth continuous ambient LED glow tightly hugging outer pedestal collar
    let dr_ped = max(0.0, r - ped_r);
    let ring_glow = exp(-dr_ped * dr_ped * 4.5);
    let ang = atan2(p.z, p.x);
    let coil_e = 0.06 + coil_band_smooth(ang) * 0.75;
    let collar_glow = coil_palette_angle(ang) * (ring_glow * coil_e * 0.048);

    let base = vec3<f32>(0.011, 0.014, 0.020) * lathe * (0.28 + 0.72 * contact);
    let refl = (env * fresnel * (0.24 + 0.76 * contact) + collar_glow) * smoothstep_r(14.0, 4.0, r);
    return base + refl;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let aspect = max(audio.aspect_ratio, 0.25);
    let screen_p = (uv * 2.0 - 1.0) * vec2<f32>(aspect, -1.0);

    let time = select(audio.time, audio.smooth_time, audio.smooth_time > 0.0);
    g_time = time;
    g_pool_mode = audio._pad3 > 0.5;

    // Extract true log-spaced 20 Hz..10 kHz frequency bands (live input is in [0..100])
    let raw_sub = (spec_bin(96u) + spec_bin(136u) + spec_bin(176u) + spec_bin(224u)) * 0.25;
    let raw_lm = (spec_bin(280u) + spec_bin(340u) + spec_bin(400u) + spec_bin(460u)) * 0.25;
    let raw_mid = (spec_bin(512u) + spec_bin(584u) + spec_bin(660u) + spec_bin(736u)) * 0.25;
    let raw_hi = (spec_bin(792u) + spec_bin(856u) + spec_bin(920u) + spec_bin(984u)) * 0.25;

    let ch_l = clamp(audio.channels[0].x, 0.0, 1.5);
    let ch_r = clamp(audio.channels[0].y, 0.0, 1.5);

    g_sub_bass = clamp(raw_sub / 55.0 + ch_l * 0.40, 0.0, 1.5);
    g_low_mid = clamp(raw_lm / 50.0 + (ch_l + ch_r) * 0.22, 0.0, 1.4);
    g_mids = clamp(raw_mid / 45.0 + ch_r * 0.24, 0.0, 1.4);
    g_highs = clamp(raw_hi / 42.0, 0.0, 1.3);
    g_tilt = clamp(ch_l - ch_r, -1.0, 1.0);
    g_activity = clamp(g_sub_bass * 0.45 + g_low_mid * 0.30 + g_mids * 0.25, 0.0, 1.5);

    g_core_center = vec3<f32>(
        g_tilt * 0.07,
        1.26 + sin(time * 0.85) * 0.032 + g_sub_bass * 0.04,
        0.0
    );
    g_core_base_r = 0.48 + g_sub_bass * 0.04;

    let rot_ang = time * 0.28 + g_tilt * 0.24;
    g_rot_c = cos(rot_ang);
    g_rot_s = sin(rot_ang);
    let tilt_ang = sin(time * 0.37) * 0.20 + g_tilt * 0.16;
    g_tilt_c = cos(tilt_ang);
    g_tilt_s = sin(tilt_ang);

    let sat_a0 = time * 0.72;
    let sat_r0 = 1.02 + g_sub_bass * 0.09;
    g_sat0 = g_core_center + vec3<f32>(cos(sat_a0) * sat_r0, sin(time * 1.25) * 0.14, sin(sat_a0) * sat_r0);
    let sat_a1 = -time * 0.54 + 2.4;
    let sat_r1 = 0.96 + g_mids * 0.10;
    g_sat1 = g_core_center + vec3<f32>(cos(sat_a1) * sat_r1, cos(time * 1.05) * 0.16, sin(sat_a1) * sat_r1);

    // Aspect-adaptive hero camera framing (portrait, 16:9, 3.5:1 ultrawide HUD-on)
    let portrait_scale = max(1.0, 1.18 / aspect);
    let cam_ang = select(time * 0.12, time * 0.10, g_pool_mode);
    let cam_dist = select(4.55, 4.45, g_pool_mode) * portrait_scale;
    let cam_h = select(1.88 + sin(time * 0.20) * 0.10, 1.98 + sin(time * 0.18) * 0.10, g_pool_mode) * portrait_scale;
    let cam_target = select(vec3<f32>(0.0, 1.02, 0.0), vec3<f32>(0.0, 0.34, 0.0), g_pool_mode);

    let ro = vec3<f32>(cos(cam_ang) * cam_dist, cam_h, sin(cam_ang) * cam_dist);
    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = cross(uu, ww);
    let focal = select(1.88, 1.80, g_pool_mode);
    let rd = normalize(screen_p.x * uu + screen_p.y * vv + focal * ww);

    var col = eval_studio_backdrop(rd);
    var t_floor = 1e9;
    var has_floor = false;
    if (rd.y < -0.0005 && ro.y > 0.0) {
        let tf = -ro.y / rd.y;
        if (tf > 0.0 && tf < 32.0) {
            has_floor = true;
            t_floor = tf;
        }
    }

    let bound_center = select(vec3<f32>(0.0, 1.10, 0.0), vec3<f32>(0.0, 0.34, 0.0), g_pool_mode);
    let bound_rad = select(2.15, 2.86, g_pool_mode);
    let sp = intersect_sphere(ro, rd, bound_center, bound_rad);

    var hit = false;
    var t_hit = 0.0;
    var t_prev = 0.0;

    if (sp.y > max(sp.x, 0.0)) {
        var t = max(sp.x, 0.0);
        let t_end = min(sp.y, t_floor);
        let max_steps = select(52, 64, g_pool_mode);

        for (var i = 0; i < max_steps; i++) {
            if (t >= t_end) {
                break;
            }
            let p = ro + rd * t;
            let d = map_dist(p);
            if (d < 0.0020) {
                var t_lo = select(t, t_prev, i > 0);
                var t_hi = t;
                for (var b = 0; b < 3; b++) {
                    let t_mid = (t_lo + t_hi) * 0.5;
                    let d_mid = map_dist(ro + rd * t_mid);
                    if (d_mid < 0.0005) {
                        t_hi = t_mid;
                    } else {
                        t_lo = t_mid;
                    }
                }
                t_hit = t_hi;
                hit = true;
                break;
            }
            t_prev = t;
            t += max(d, 0.0040);
        }
    }

    if (hit) {
        let p_hit = ro + rd * t_hit;
        let mat_info = eval_surface_material(p_hit);
        let mat_id = i32(mat_info.x);
        let eps = clamp(0.0020 + t_hit * 0.00035, 0.0020, 0.005);
        let n = calc_sdf_normal(p_hit, eps);
        let ao = calc_sdf_ao(p_hit, n);

        if (mat_id == MAT_FERRO) {
            col = shade_ferrofluid(p_hit, rd, n, ao);
        } else if (mat_id == MAT_COIL) {
            let idx = u32(mat_info.y);
            let band_e = 0.14 + coil_band_raw(idx) * 0.86;
            col = coil_palette(idx) * (0.40 + 1.55 * band_e);
        } else {
            col = shade_titanium_pedestal(p_hit, rd, n, ao);
        }
    } else if (has_floor) {
        let p_floor = ro + rd * t_floor;
        let bench_col = shade_studio_bench(p_floor, rd);
        let fog = smoothstep(9.0, 26.0, t_floor);
        col = mix(bench_col, eval_studio_backdrop(rd), fog);
    }

    col = aces_tonemap(col);

    // Gentle optical vignette
    let d_center = length(uv - vec2<f32>(0.5));
    col *= clamp(1.16 - d_center * 0.36, 0.68, 1.0);

    return vec4<f32>(col, 1.0);
}
