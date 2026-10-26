// INCLUDE: common

// ============================================================================
// Visualizer ID 28: Acoustic Optical Glass & Water Chamber
// Navier-Stokes FLIP Water Simulation in Beveled Optical Crystal Vessel
// Refractive Water (IOR 1.333), Lead Crystal Glass (IOR 1.52),
// Ridged Cellular Caustics, Cylindrical Bessel Cymatics & Acoustic Fountain Plume
// ============================================================================

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var<storage, read> render_grid: array<u32>;

const RENDER_GRID_SIZE: i32 = 512;
const DOMAIN_HALF: f32 = 3.2;
const VESSEL_INNER_R: f32 = 2.65;
const VESSEL_OUTER_R: f32 = 2.79;
const VESSEL_HEIGHT: f32 = 1.42;
const BASIN_FLOOR_Y: f32 = 0.14;
const BASE_WATER_Y: f32 = 0.42;

// Raymarch tuning
const MAX_STEPS: i32 = 72;
const HIT_EPS: f32 = 0.0020;
const STEP_SCALE: f32 = 0.58;

var<private> g_time: f32 = 0.0;
var<private> g_sub_bass: f32 = 0.0;
var<private> g_lowmids: f32 = 0.0;
var<private> g_mids: f32 = 0.0;
var<private> g_highs: f32 = 0.0;
var<private> g_flip_energy: f32 = 0.0;

fn get_grid_height(gx: i32, gz: i32) -> f32 {
    if (gx < 0 || gx >= RENDER_GRID_SIZE || gz < 0 || gz >= RENDER_GRID_SIZE) {
        return 0.0;
    }
    let cell = u32(gz) * u32(RENDER_GRID_SIZE) + u32(gx);
    return f32(render_grid[cell]) * 0.001;
}

// Sample macro hydrodynamic energy from the FLIP simulation grid around the chamber center
// so the FLIP solver drives inertial fluid swell without injecting discrete texel/particle noise
fn sample_macro_flip_energy() -> f32 {
    let c = RENDER_GRID_SIZE / 2;
    let h0 = get_grid_height(c, c);
    let h1 = get_grid_height(c + 12, c) + get_grid_height(c - 12, c)
           + get_grid_height(c, c + 12) + get_grid_height(c, c - 12);
    let h2 = get_grid_height(c + 28, c + 28) + get_grid_height(c - 28, c + 28)
           + get_grid_height(c + 28, c - 28) + get_grid_height(c - 28, c - 28);
    return clamp((h0 * 2.0 + h1 + h2 * 0.5) * 0.35, 0.0, 0.45);
}

// Continuous C-infinity smooth 3D hydrodynamic + cymatic water surface heightfield
fn sample_water_height(p: vec2<f32>) -> f32 {
    let r_xz = length(p);
    let r_clamped = min(r_xz, VESSEL_INNER_R - 0.008);
    let p_eval = select(p, p * (r_clamped / max(r_xz, 0.0001)), r_xz > r_clamped);
    let r = length(p_eval);
    let r2 = r * r;
    let theta = atan2(p_eval.y, p_eval.x);

    // 1. Capillary Wall Meniscus clinging to inner optical crystal wall
    let wall_dist = max(0.0, VESSEL_INNER_R - r);
    let meniscus = smoothstep(0.32, 0.0, wall_dist) * 0.058;

    // 2. Central Sub-Bass Acoustic Fountain Plume & Concentric Rayleigh Crown Collars
    let flip_boost = g_flip_energy * 0.25;
    let bass_drive = g_sub_bass + flip_boost;
    let plume_pulse = 0.90 + 0.10 * sin(g_time * 6.4);
    // Smooth Gaussian-Bell fountain jet in the center
    let plume_core = exp(-r2 * 4.2) * (0.06 + bass_drive * 0.31) * plume_pulse;
    // Secondary concentric hydrodynamic collar around the fountain base
    let collar1_r = 0.68 + 0.05 * sin(g_time * 3.0);
    let collar1 = exp(-pow(r - collar1_r, 2.0) * 14.0) * (0.022 + bass_drive * 0.095);
    let trough1 = -exp(-pow(r - 0.44, 2.0) * 22.0) * (0.012 + bass_drive * 0.035);

    // 3. Radially Propagating Gravity Wave Rings + Cylindrical Wall Reflection (Bessel J0 mode)
    let wave_out = sin(r * 7.6 - g_time * 5.2) * exp(-r * 0.38) * smoothstep(0.18, 0.55, r);
    let wave_refl = sin(r * 7.6 + g_time * 5.2) * exp(-wall_dist * 0.82) * 0.48;
    let radial_waves = (wave_out + wave_refl) * (0.018 + bass_drive * 0.042 + g_lowmids * 0.028);

    // 4. Multi-Pole Azimuthal Faraday / Cymatic Standing Wave Harmonics
    let mode4 = cos(r * 8.4 - g_time * 3.2) * cos(theta * 4.0 + g_time * 0.30) * smoothstep(0.30, 1.05, r);
    let mode6 = sin(r * 11.8 + g_time * 3.8) * cos(theta * 6.0 - g_time * 0.40) * smoothstep(0.55, 1.60, r);
    let mode8 = cos(r * 15.2 - g_time * 4.5) * sin(theta * 8.0 + g_time * 0.25) * smoothstep(0.85, 2.00, r);
    let cymatic_ripples = mode4 * (0.011 + g_mids * 0.024)
                        + mode6 * (0.006 + g_highs * 0.015)
                        + mode8 * (0.003 + g_highs * 0.009);

    let wall_damp = smoothstep(0.0, 0.14, wall_dist);
    let dynamic_waves = (plume_core + collar1 + trough1 + radial_waves + cymatic_ripples) * wall_damp;

    return BASE_WATER_Y + meniscus + dynamic_waves;
}

fn calc_water_normal(p: vec2<f32>, eps: f32) -> vec3<f32> {
    let r_xz = length(p);
    let max_r = VESSEL_INNER_R - eps * 1.5;
    let p_eval = select(p, p * (max_r / max(r_xz, 0.0001)), r_xz > max_r);

    let e = vec2<f32>(eps, 0.0);
    let dx = (sample_water_height(p_eval + e.xy) - sample_water_height(p_eval - e.xy)) / (2.0 * eps);
    let dz = (sample_water_height(p_eval + e.yx) - sample_water_height(p_eval - e.yx)) / (2.0 * eps);
    return normalize(vec3<f32>(-dx, 1.0, -dz));
}

// Ray-Cylinder intersection (infinite cylinder along Y axis)
fn intersect_cylinder(ro: vec3<f32>, rd: vec3<f32>, radius: f32) -> vec2<f32> {
    let a = dot(rd.xz, rd.xz);
    let b = 2.0 * dot(ro.xz, rd.xz);
    let c = dot(ro.xz, ro.xz) - radius * radius;
    let disc = b * b - 4.0 * a * c;

    if (disc < 0.0 || a < 0.00001) {
        return vec2<f32>(-1.0, -1.0);
    }

    let sqrt_disc = sqrt(disc);
    let t0 = (-b - sqrt_disc) / (2.0 * a);
    let t1 = (-b + sqrt_disc) / (2.0 * a);
    return vec2<f32>(t0, t1);
}

// True Ridged Cellular / Voronoi Caustic Filament Network (continuous intersecting photon webs, never polka-dots)
fn calculate_caustics(pos: vec2<f32>, time: f32, bass: f32) -> f32 {
    let p = pos * 1.35;
    let c = 0.8660254;
    let s = 0.5000000;

    // Domain-warped octave 1
    let w1 = p + vec2<f32>(sin(p.y * 1.8 + time * 1.5), cos(p.x * 1.8 - time * 1.3)) * 0.32;
    // Rotated & counter-warped octave 2
    let p_rot = vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c) * 1.28;
    let w2 = p_rot + vec2<f32>(cos(p_rot.y * 2.1 - time * 1.7), sin(p_rot.x * 2.1 + time * 1.4)) * 0.26;

    // 1.0 - abs(sin(...)) produces sharp continuous 1D zero-crossing caustic filaments
    let f1 = 1.0 - abs(sin(w1.x * 3.4 + sin(w1.y * 2.8 + time * 1.1)));
    let f2 = 1.0 - abs(sin(w2.y * 3.6 + cos(w2.x * 3.0 - time * 1.2)));
    let f3 = 1.0 - abs(sin((w1.x + w2.y) * 2.5 - time * 0.9));

    let filament1 = pow(clamp(f1, 0.0, 1.0), 7.0);
    let filament2 = pow(clamp(f2, 0.0, 1.0), 7.0);
    let filament3 = pow(clamp(f3, 0.0, 1.0), 8.0) * 0.75;
    // Bright caustic cusps where filaments intersect
    let cusp_nodes = pow(clamp(f1 * f2, 0.0, 1.0), 3.5) * 2.4;

    return (filament1 + filament2 + filament3 + cusp_nodes) * (0.95 + bass * 0.85);
}

fn eval_cymatic_floor_grid(floor_p: vec2<f32>, time: f32, sub_bass: f32, mids: f32, highs: f32) -> vec3<f32> {
    let r = length(floor_p);
    let theta = atan2(floor_p.y, floor_p.x);
    let edge_fade = smoothstep(VESSEL_OUTER_R, VESSEL_INNER_R - 0.20, r);

    // Chladni radial + azimuthal standing-wave laser nodal filaments on the obsidian transducer plate
    let nodal1 = cos(r * 7.2 - time * 1.8) * cos(theta * 4.0) + 0.55 * sin(r * 11.2 + time * 1.3) * sin(theta * 6.0);
    let chladni_line = exp(-abs(nodal1) * 16.0) * edge_fade;

    // Concentric precision machined copper/sapphire resonator rings
    let ring_grid = exp(-abs(fract(r * 1.65) - 0.5) * 32.0) * 0.42 * edge_fade;
    // 12 radial acoustic waveguide spokes
    let spoke_line = exp(-abs(sin(theta * 6.0)) * 28.0) * smoothstep(0.35, 0.85, r) * edge_fade * 0.28;

    let cymatic_col = mix(vec3<f32>(0.06, 0.72, 1.20), vec3<f32>(0.55, 0.35, 1.30), clamp(highs * 0.65, 0.0, 1.0));
    let gold_accent = vec3<f32>(1.10, 0.78, 0.32) * exp(-abs(r - 1.20) * 26.0) * (0.25 + sub_bass * 0.45);

    return cymatic_col * (chladni_line * (0.22 + mids * 0.55 + highs * 0.40) + (ring_grid + spoke_line) * (0.18 + sub_bass * 0.28)) + gold_accent;
}

// Studio environment & area-light softbox reflection for crystal glass and water surfaces
fn eval_studio_reflection(pos: vec3<f32>, ref_dir: vec3<f32>, sub_bass: f32) -> vec3<f32> {
    var refl = vec3<f32>(0.0);

    // 1. Primary Overhead Diffused Studio Softbox Panel (at y = 5.6)
    if (ref_dir.y > 0.01) {
        let t_sb = (5.6 - pos.y) / ref_dir.y;
        let hit_sb = pos.xz + ref_dir.xz * t_sb;
        let sb_x = smoothstep_r(3.2, 1.2, abs(hit_sb.x - 0.6));
        let sb_z = smoothstep_r(3.6, 1.4, abs(hit_sb.y - 0.8));
        let louver = 0.88 + 0.12 * cos(hit_sb.x * 8.0);
        refl += vec3<f32>(2.4, 2.55, 2.85) * (sb_x * sb_z * louver) * (1.0 + sub_bass * 0.25);
    }

    // 2. Left Curved Studio Rim Strip (Electric Cyan-Sapphire)
    let strip1_dir = normalize(vec3<f32>(-4.8, 2.4, 1.8) - pos);
    let align1 = max(0.0, dot(ref_dir, strip1_dir));
    let strip1 = pow(align1, 24.0) * 1.4 + pow(align1, 140.0) * 3.2;
    refl += vec3<f32>(0.22, 0.82, 1.45) * strip1;

    // 3. Right Curved Studio Kicker Strip (Warm Champagne Gold & Violet)
    let strip2_dir = normalize(vec3<f32>(4.8, 2.8, -1.6) - pos);
    let align2 = max(0.0, dot(ref_dir, strip2_dir));
    let strip2 = pow(align2, 28.0) * 1.2 + pow(align2, 160.0) * 2.8;
    refl += vec3<f32>(1.30, 0.92, 0.58) * strip2;

    // 4. Ambient Studio Dome Gradient
    let sky_grad = smoothstep(-0.2, 0.85, ref_dir.y);
    refl += mix(vec3<f32>(0.015, 0.025, 0.045), vec3<f32>(0.10, 0.18, 0.30), sky_grad);

    return refl;
}

// Vertical studio strip-light & silhouette reflections specifically for cylindrical crystal walls
fn eval_cylinder_wall_glass(p_wall: vec3<f32>, n_wall: vec3<f32>, rd: vec3<f32>, cam_right: vec3<f32>, sub_bass: f32, is_front: bool) -> vec3<f32> {
    let view_xz = normalize(-rd.xz);
    let norm_xz = normalize(n_wall.xz);
    let NdotV_xz = abs(dot(norm_xz, view_xz));

    // Continuous vertical Fresnel silhouette along the left and right edges of the glass cylinder
    let cyl_fresnel = pow(1.0 - NdotV_xz, 4.2);
    let side_coord = dot(norm_xz, normalize(cam_right.xz));

    // Sharp vertical studio softbox strip reflections along the curved cylinder flanks
    let strip_sharpness = select(36.0, 85.0, is_front);
    let left_strip = exp(-pow(side_coord + 0.78, 2.0) * strip_sharpness);
    let right_strip = exp(-pow(side_coord - 0.82, 2.0) * strip_sharpness);

    let vertical_fade = smoothstep(BASIN_FLOOR_Y, BASIN_FLOOR_Y + 0.08, p_wall.y)
                      * smoothstep(VESSEL_HEIGHT + 0.02, VESSEL_HEIGHT - 0.05, p_wall.y);

    var wall_light = vec3<f32>(0.70, 0.92, 1.25) * (left_strip * 1.15)
                   + vec3<f32>(1.15, 0.98, 0.82) * (right_strip * 1.0);
    wall_light *= vertical_fade;

    // Crisp rim edge line along the left/right cylinder silhouette contour
    let contour_edge = pow(1.0 - NdotV_xz, 6.5) * vec3<f32>(0.65, 0.90, 1.25) * (1.35 + sub_bass * 0.35);

    return wall_light + contour_edge + vec3<f32>(0.10, 0.20, 0.34) * cyl_fresnel * 0.38;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let aspect = max(audio.aspect_ratio, 0.25);
    let screen_p = (uv * 2.0 - 1.0) * vec2<f32>(aspect, -1.0);

    let time = select(audio.time, audio.smooth_time, audio.smooth_time > 0.0);
    g_time = time;

    // Support both live [0..100] spectrum and synthetic [0..2] benchmark inputs seamlessly
    let raw_s0 = audio.spectrum[0].x;
    let raw_lm = audio.spectrum[3].x + audio.spectrum[5].x;
    let raw_m = audio.spectrum[8].x + audio.spectrum[16].x;
    let raw_h = audio.spectrum[32].x + audio.spectrum[48].x;

    let s0_norm = select(raw_s0 / 55.0, raw_s0, raw_s0 > 0.0 && raw_s0 <= 3.0);
    let lm_norm = select(raw_lm / 110.0, raw_lm * 0.5, raw_lm > 0.0 && raw_lm <= 4.0);
    let m_norm = select(raw_m / 110.0, raw_m * 0.5, raw_m > 0.0 && raw_m <= 4.0);
    let h_norm = select(raw_h / 110.0, raw_h * 0.5, raw_h > 0.0 && raw_h <= 4.0);

    let sub_bass = clamp(s0_norm + audio.channels[0].x * 0.85, 0.0, 2.2);
    let lowmids = clamp(lm_norm + (audio.channels[0].x + audio.channels[0].y) * 0.35, 0.0, 1.6);
    let mids = clamp(m_norm + audio.channels[0].y * 0.45 + sub_bass * 0.25, 0.0, 1.6);
    let highs = clamp(h_norm + audio.channels[0].x * 0.35 + mids * 0.25, 0.0, 1.4);

    g_sub_bass = sub_bass;
    g_lowmids = lowmids;
    g_mids = mids;
    g_highs = highs;
    g_flip_energy = sample_macro_flip_energy();

    // Responsive studio camera framing: closer & fuller in frame, adapting cleanly to portrait viewports
    let portrait_scale = max(1.0, 1.35 / aspect);
    let cam_orbit = time * 0.065;
    let cam_dist = 5.35 * portrait_scale;
    let cam_h = (2.95 + sin(time * 0.14) * 0.10) * portrait_scale;
    let ro = vec3<f32>(cos(cam_orbit) * cam_dist, cam_h, sin(cam_orbit) * cam_dist);
    let cam_target = vec3<f32>(0.0, 0.44, 0.0);

    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = cross(uu, ww);
    let rd = normalize(screen_p.x * uu + screen_p.y * vv + 1.85 * ww);

    // Seamless dark studio backdrop with soft horizon glow (zero horizon step artifact)
    let bg_dist = length(screen_p);
    let horizon_glow = exp(-abs(rd.y + 0.08) * 6.5) * 0.025;
    let bg_gradient = clamp(0.036 - 0.016 * bg_dist, 0.008, 0.055) + horizon_glow;
    let bg_col = vec3<f32>(bg_gradient * 0.72, bg_gradient * 0.92, bg_gradient * 1.32);
    var col = bg_col;

    // Primary studio light positions
    let softbox_pos = vec3<f32>(2.4, 6.2, 3.0);
    let kicker_pos = vec3<f32>(-4.5, 4.0, -3.5);

    // --- 1. Intersect Outer Optical Crystal Vessel, Top Beveled Rim & Transducer Base ---
    let outer_cyl = intersect_cylinder(ro, rd, VESSEL_OUTER_R);
    var hit_outer_glass = false;
    var hit_pedestal = false;
    var t_glass_outer = 1e5;
    var n_glass_outer = vec3<f32>(0.0);
    var is_glass_rim = false;

    // Check top beveled crystal rim: plane y = VESSEL_HEIGHT between [VESSEL_INNER_R, VESSEL_OUTER_R]
    if (rd.y < -0.0001 && ro.y > VESSEL_HEIGHT) {
        let t_top = (VESSEL_HEIGHT - ro.y) / rd.y;
        if (t_top > 0.0) {
            let p_top = ro + rd * t_top;
            let r_top = length(p_top.xz);
            if (r_top >= VESSEL_INNER_R && r_top <= VESSEL_OUTER_R) {
                hit_outer_glass = true;
                t_glass_outer = t_top;
                let wall_frac = (r_top - VESSEL_INNER_R) / (VESSEL_OUTER_R - VESSEL_INNER_R);
                let radial_n = p_top.xz / max(r_top, 0.001);
                let bevel_tilt = (wall_frac - 0.5) * 0.85;
                n_glass_outer = normalize(vec3<f32>(radial_n.x * bevel_tilt, 1.0, radial_n.y * bevel_tilt));
                is_glass_rim = true;
            }
        }
    }

    // Check front outer cylindrical wall & machined transducer base pedestal
    if (outer_cyl.x > 0.0) {
        let p_cyl = ro + rd * outer_cyl.x;
        if (p_cyl.y >= 0.0 && p_cyl.y <= VESSEL_HEIGHT && outer_cyl.x < t_glass_outer) {
            t_glass_outer = outer_cyl.x;
            n_glass_outer = normalize(vec3<f32>(p_cyl.x, 0.0, p_cyl.z));
            if (p_cyl.y < BASIN_FLOOR_Y) {
                hit_pedestal = true;
            } else {
                hit_outer_glass = true;
                is_glass_rim = false;
            }
        }
    }

    // --- 2. Raymarch Inside Inner Acoustic Water Chamber ---
    let inner_cyl = intersect_cylinder(ro, rd, VESSEL_INNER_R);
    var water_hit = false;
    var is_submerged = false;
    var hit_p = vec3<f32>(0.0);
    var t_water = 0.0;
    var t_start = 0.0;

    if (!hit_pedestal && inner_cyl.y > 0.0) {
        t_start = max(0.0, inner_cyl.x);
        var t_end = inner_cyl.y;

        if (rd.y < -0.0001) {
            let t_top_entry = (VESSEL_HEIGHT - ro.y) / rd.y;
            t_start = max(t_start, t_top_entry);
            let t_floor_exit = (BASIN_FLOOR_Y - ro.y) / rd.y;
            t_end = min(t_end, t_floor_exit);
        } else if (rd.y > 0.0001) {
            let t_floor_entry = (BASIN_FLOOR_Y - ro.y) / rd.y;
            t_start = max(t_start, t_floor_entry);
            let t_top_exit = (VESSEL_HEIGHT - ro.y) / rd.y;
            t_end = min(t_end, t_top_exit);
        }

        if (t_start < t_end) {
            let p_start = ro + rd * t_start;
            let h_start = sample_water_height(p_start.xz);

            if (p_start.y > h_start) {
                // Ray entered through air above liquid: march heightfield with secant root-refinement
                var t = t_start;
                var t_prev = t_start;
                var dist_prev = p_start.y - h_start;

                for (var i = 0; i < MAX_STEPS; i++) {
                    let p = ro + rd * t;
                    let h = sample_water_height(p.xz);
                    let dist = p.y - h;

                    if (dist < HIT_EPS) {
                        let denom = max(dist_prev - dist, 0.0001);
                        let frac_t = clamp(dist_prev / denom, 0.0, 1.0);
                        var t_refined = mix(t_prev, t, frac_t);
                        let p_mid = ro + rd * t_refined;
                        let d_mid = p_mid.y - sample_water_height(p_mid.xz);
                        t_refined += clamp(d_mid / max(-rd.y + 0.35, 0.25), -0.015, 0.015);

                        water_hit = true;
                        t_water = t_refined;
                        hit_p = ro + rd * t_refined;
                        break;
                    }

                    if (p.y <= BASIN_FLOOR_Y + 0.002) {
                        break;
                    }

                    t_prev = t;
                    dist_prev = dist;
                    t += max(dist * STEP_SCALE, 0.012);
                    if (t >= t_end) { break; }
                }
            } else {
                // Ray entered through the curved crystal cylinder wall BELOW the water meniscus
                var t_interior = t_end;
                if (rd.y < -0.0001) {
                    let t_floor = (BASIN_FLOOR_Y - ro.y) / rd.y;
                    t_interior = min(t_end, t_floor);
                }
                t_water = t_interior;
                hit_p = ro + rd * t_interior;
                is_submerged = true;
            }
        }
    }

    if (hit_pedestal) {
        // --- Machined Dark-Titanium & Copper Acoustic Transducer Pedestal Base ---
        let p_ped = ro + rd * t_glass_outer;
        let view_dir = -rd;
        let NdotV = max(0.0, dot(n_glass_outer, view_dir));
        let l1 = normalize(softbox_pos - p_ped);
        let h1 = normalize(l1 + view_dir);
        let spec_ped = pow(max(0.0, dot(n_glass_outer, h1)), 48.0) * 0.85;

        let azimuth = atan2(p_ped.z, p_ped.x);
        let seg_f = fract((azimuth + 3.14159265) / 6.2831853 * 24.0);
        let seg_mask = smoothstep(0.12, 0.22, seg_f) * (1.0 - smoothstep(0.78, 0.88, seg_f));
        let band_y = smoothstep(0.035, 0.055, p_ped.y) * (1.0 - smoothstep(0.085, 0.105, p_ped.y));
        let led_glow = vec3<f32>(0.08, 0.78, 1.35) * seg_mask * band_y * (0.35 + sub_bass * 1.1);

        let copper_ring = smoothstep(0.115, 0.135, p_ped.y) * vec3<f32>(0.55, 0.35, 0.18) * (0.4 + spec_ped);
        let base_metal = vec3<f32>(0.025, 0.032, 0.045) * (0.35 + 0.65 * NdotV) + vec3<f32>(0.35, 0.45, 0.60) * spec_ped * 0.45;
        col = base_metal + copper_ring + led_glow;
    } else if (water_hit) {
        // --- 3. Shade Continuous 3D Water Surface, Reflections, Refraction & Caustics ---
        let eps = 0.024;
        let normal = calc_water_normal(hit_p.xz, eps);
        let view_dir = -rd;
        let NdotV = max(0.0, dot(normal, view_dir));

        // Dielectric Fresnel reflection for water (F0 = 0.028)
        let fresnel_water = 0.028 + 0.972 * pow(1.0 - NdotV, 4.5);

        // Studio Softbox Area-Light + Dual Rim Strip Reflections on 3D Wave Slopes
        let ref_dir = reflect(rd, normal);
        let env_reflection = eval_studio_reflection(hit_p, ref_dir, sub_bass);

        // Pinpoint specular glints on steep cymatic wave crests
        let l1_dir = normalize(softbox_pos - hit_p);
        let h1 = normalize(l1_dir + view_dir);
        let NdotL1 = max(0.0, dot(normal, l1_dir));
        let spec1_tight = pow(max(0.0, dot(normal, h1)), 240.0) * 4.8;
        let spec1_soft = pow(max(0.0, dot(normal, h1)), 28.0) * 0.75;
        let water_specular1 = vec3<f32>(1.15, 1.22, 1.35) * (spec1_tight + spec1_soft) * NdotL1;

        let l2_dir = normalize(kicker_pos - hit_p);
        let h2 = normalize(l2_dir + view_dir);
        let NdotL2 = max(0.0, dot(normal, l2_dir));
        let spec2 = pow(max(0.0, dot(normal, h2)), 96.0) * 2.4;
        let water_specular2 = vec3<f32>(0.25, 0.88, 1.45) * spec2 * NdotL2;

        // Snell's Law Refraction (IOR 1.0 -> 1.333) & Beer-Lambert Optical Absorption
        let refr_dir = refract(rd, normal, 1.0 / 1.333);
        let water_depth = max(0.06, hit_p.y - BASIN_FLOOR_Y);
        let t_refr_floor = water_depth / max(0.12, -refr_dir.y);
        let floor_p = hit_p.xz + refr_dir.xz * min(t_refr_floor, 1.8);

        // Rich emerald-sapphire Beer-Lambert depth absorption + subsurface acoustic plume scatter
        let absorption_coeff = vec3<f32>(0.42, 0.075, 0.014);
        let transmission = exp(-absorption_coeff * (t_refr_floor * 2.8));

        // Wave-slope Jacobian focusing: steeper wave crests concentrate brighter caustic filaments
        let wave_slope = length(normal.xz);
        let jacobian_focus = clamp(1.0 + wave_slope * 4.0, 1.0, 3.0);
        let caustics = calculate_caustics(floor_p, time, sub_bass) * jacobian_focus;
        let cymatic_grid = eval_cymatic_floor_grid(floor_p, time, sub_bass, mids, highs);

        // Subtle chromatic dispersion on refracted caustics
        let caustics_r = calculate_caustics(floor_p + normal.xz * 0.025, time, sub_bass);
        let caustics_b = calculate_caustics(floor_p - normal.xz * 0.025, time, sub_bass);
        let caustic_rgb = vec3<f32>(
            caustics_r * 0.18,
            caustics * 0.68,
            (caustics * 0.5 + caustics_b * 0.5) * 1.15
        ) * jacobian_focus;

        let transducer_plate = vec3<f32>(0.012, 0.022, 0.040) + caustic_rgb * 0.24 + cymatic_grid;

        // Acoustic Center Transducer Core & Resonator Ring Luminescence
        let center_dist = length(floor_p);
        let ring_pulse = exp(-pow(center_dist - 0.68, 2.0) * 9.0) * (0.30 + sub_bass * 0.65);
        let bass_core = exp(-center_dist * center_dist * 2.8) * (0.25 + sub_bass * 0.60);
        let transducer_emission = (ring_pulse * vec3<f32>(0.12, 0.82, 1.35) + bass_core * vec3<f32>(0.18, 0.68, 1.25));

        // Volumetric subsurface scattering inside the elevated central acoustic fountain plume
        let plume_elev = clamp((hit_p.y - BASE_WATER_Y) * 3.2, 0.0, 1.0);
        let sss_plume = vec3<f32>(0.08, 0.52, 0.92) * plume_elev * (0.35 + sub_bass * 0.65) * (1.0 - NdotV * 0.5);

        let water_body = (transducer_plate + transducer_emission) * transmission + sss_plume;

        var water_col = mix(water_body, env_reflection, fresnel_water * 0.88 + 0.06);
        water_col += (water_specular1 + water_specular2) * (0.35 + 0.65 * fresnel_water);

        col = water_col;
    } else if (is_submerged) {
        // --- 4. View Through Curved Front Crystal Wall Below the Water Meniscus ---
        let p_enter = ro + rd * t_start;
        let wall_n = normalize(vec2<f32>(p_enter.x, p_enter.z));
        let lens_pos = hit_p.xz - wall_n * 0.12;

        let water_path = max(0.12, t_water - t_start);
        let absorption_coeff = vec3<f32>(0.42, 0.075, 0.014);
        let transmission = exp(-absorption_coeff * (water_path * 1.85));

        let caustics = calculate_caustics(lens_pos, time, sub_bass);
        let cymatic_grid = eval_cymatic_floor_grid(lens_pos, time, sub_bass, mids, highs);
        let floor_plate = vec3<f32>(0.012, 0.022, 0.040) + vec3<f32>(0.18, 0.68, 1.12) * (caustics * 0.22) + cymatic_grid;

        let center_dist = length(lens_pos);
        let ring_pulse = exp(-pow(center_dist - 0.68, 2.0) * 9.0) * (0.30 + sub_bass * 0.65);
        let bass_core = exp(-center_dist * center_dist * 2.8) * (0.25 + sub_bass * 0.60);
        let transducer_emission = (ring_pulse + bass_core) * vec3<f32>(0.15, 0.72, 1.25);

        // Glowing capillary meniscus line where water clings to the inner front crystal wall
        let h_wall = sample_water_height(wall_n * (VESSEL_INNER_R - 0.02));
        let meniscus_dist = abs(p_enter.y - h_wall);
        let meniscus_rim = exp(-meniscus_dist * meniscus_dist * 1800.0) * vec3<f32>(0.45, 0.95, 1.45) * 1.45;

        col = (floor_plate + transducer_emission) * transmission + meniscus_rim;
    } else {
        // --- 5. Solid Internal Crystal Base Ring, Studio Table & Back Crystal Wall ---
        var hit_internal_base = false;
        if (rd.y < -0.001) {
            let t_base = (BASIN_FLOOR_Y - ro.y) / rd.y;
            if (t_base > 0.0) {
                let p_base = ro + rd * t_base;
                let r_base = length(p_base.xz);
                if (r_base <= VESSEL_OUTER_R) {
                    hit_internal_base = true;
                    let caustics = calculate_caustics(p_base.xz, time, sub_bass);
                    let cymatic_grid = eval_cymatic_floor_grid(p_base.xz, time, sub_bass, mids, highs);
                    let ring_bevel = smoothstep(VESSEL_INNER_R - 0.08, VESSEL_OUTER_R, r_base);
                    let crystal_seat = vec3<f32>(0.08, 0.22, 0.38) * ring_bevel;
                    col = vec3<f32>(0.012, 0.022, 0.040) + vec3<f32>(0.16, 0.62, 1.05) * (caustics * 0.18) + cymatic_grid + crystal_seat;
                }
            }

            if (!hit_internal_base) {
                let t_ground = -ro.y / rd.y;
                if (t_ground > 0.0 && t_ground < 45.0) {
                    let p_ground = ro + rd * t_ground;
                    let r_ground = length(p_ground.xz);

                    let dist_fade = exp(-t_ground * 0.095) * smoothstep(42.0, 8.0, t_ground);
                    let contact_shadow = smoothstep(VESSEL_OUTER_R * 0.94, VESSEL_OUTER_R + 0.55, r_ground);
                    let halo_band = smoothstep(VESSEL_OUTER_R - 0.05, VESSEL_OUTER_R + 0.30, r_ground)
                                  * exp(-max(0.0, r_ground - VESSEL_OUTER_R) * 1.15);
                    let caustics_floor = calculate_caustics(p_ground.xz * 0.85, time, sub_bass);
                    let floor_caustic_col = vec3<f32>(0.06, 0.38, 0.72) * (caustics_floor * 0.42 + 0.18) * halo_band * (0.8 + sub_bass * 0.5);

                    let table_base = vec3<f32>(0.014, 0.020, 0.032) * (0.20 + 0.80 * contact_shadow) + floor_caustic_col;
                    col = mix(bg_col, table_base, dist_fade);
                }
            }
        }

        // Back wall of the cylindrical optical crystal vessel (visible above water line)
        if (outer_cyl.y > 0.0) {
            let p_back = ro + rd * outer_cyl.y;
            if (p_back.y >= BASIN_FLOOR_Y && p_back.y <= VESSEL_HEIGHT) {
                let n_back = normalize(vec3<f32>(p_back.x, 0.0, p_back.z));
                let wall_glass_b = eval_cylinder_wall_glass(p_back, n_back, rd, uu, sub_bass, false);
                let rim_top_b = exp(-abs(p_back.y - VESSEL_HEIGHT) * 42.0) * vec3<f32>(0.65, 0.88, 1.20) * 0.75;
                col += wall_glass_b * 0.36 + rim_top_b;
            }
        }
    }

    // --- 6. Front Optical Crystal Cylinder Wall, Vertical Studio Strips & Bevel Highlights ---
    if (hit_outer_glass) {
        let p_enter = ro + rd * t_glass_outer;
        let view_dir = -rd;
        let NdotV = max(0.0, dot(n_glass_outer, view_dir));
        let outer_fresnel = 0.035 + 0.965 * pow(1.0 - NdotV, 5.0);

        let l_soft = normalize(softbox_pos - p_enter);
        let h_soft = normalize(l_soft + view_dir);
        let spec_tight = pow(max(0.0, dot(n_glass_outer, h_soft)), 220.0) * 4.2;
        let spec_broad = pow(max(0.0, dot(n_glass_outer, h_soft)), 28.0) * 0.35;

        if (is_glass_rim) {
            let ref_rim = reflect(rd, n_glass_outer);
            let rim_env = eval_studio_reflection(p_enter, ref_rim, sub_bass);
            // Crisp optical crystal top rim with subtle diamond dispersion only on specular glints
            let rim_phase = fract(dot(n_glass_outer, vec3<f32>(2.0, 1.2, 2.0)) * 0.65 + time * 0.05);
            let diamond_disp = mix(vec3<f32>(0.90, 0.96, 1.08), iridescence_cos(rim_phase), 0.22);
            let rim_col = (vec3<f32>(0.72, 0.88, 1.12) * 0.50 + rim_env * 0.72 + vec3<f32>(spec_tight + spec_broad)) * diamond_disp;
            col = mix(col, rim_col, 0.65);
        } else {
            // Ultra-clear optical crystal front wall with sharp vertical flank reflections & silhouette contour
            let wall_glass = eval_cylinder_wall_glass(p_enter, n_glass_outer, rd, uu, sub_bass, true);
            let wall_spec = vec3<f32>(1.05, 1.15, 1.30) * (spec_tight + spec_broad);

            // Glowing capillary meniscus highlight where water surface meets front glass wall
            let wall_dir = normalize(p_enter.xz);
            let h_front_meniscus = sample_water_height(wall_dir * (VESSEL_INNER_R - 0.02));
            let m_dist = abs(p_enter.y - h_front_meniscus);
            let front_meniscus_glow = exp(-m_dist * m_dist * 2600.0) * vec3<f32>(0.50, 0.95, 1.45) * 0.75;

            // Bottom crystal base bevel highlight
            let bottom_chamfer = exp(-abs(p_enter.y - BASIN_FLOOR_Y) * 48.0) * vec3<f32>(0.45, 0.78, 1.15) * 0.55;

            col = col * (1.0 - outer_fresnel * 0.12) + wall_glass * 0.38 + wall_spec * outer_fresnel + front_meniscus_glow + bottom_chamfer;
        }
    }

    // ACES Tonemapping for photorealistic HDR highlight rolloff
    col = aces_tonemap(col);

    // Subtle studio edge vignette
    let d_center = length(uv - vec2<f32>(0.5));
    col *= clamp(1.22 - d_center * 0.72, 0.0, 1.0);

    return vec4<f32>(col, 1.0);
}
