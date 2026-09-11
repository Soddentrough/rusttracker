// =====================================================
// Chrome Ferrofluid Visualizer — Pure Metal 7.1.4 Edition
// Raymarched liquid metal simulation naturally resting in a
// circular machined containment container with 7.1.4 spatial audio,
// studio softbox illumination, dual kickers, and crevice AO.
// =====================================================

// --- Tuning Constants ---
const DISH_RADIUS: f32 = 3.15;        // Inner radius of the circular container
const RIM_RADIUS: f32 = 3.26;         // Center radius of the retaining rim
const RIM_HEIGHT: f32 = -0.45;        // Elevation center of the container rim
const RIM_THICKNESS: f32 = 0.05;      // Minor radius of the toroidal rim
const BASE_THICKNESS: f32 = 0.09;     // Surface tension resting floor
var<private> g_step_scale: f32 = 0.38;
const MAX_MARCH_STEPS: i32 = 80;
const HIT_THRESHOLD: f32 = 0.005;
const NORMAL_EPS: f32 = 0.015;
const SPEC_POWER: f32 = 32.0;
const MAX_MARCH_DIST: f32 = 30.0;

// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

// --- 7.1.4 Dolby Atmos Spatial Speaker Layout ---
// Azimuth directions for 12 channels (Bed 7.0 + Heights 0.0.4)
const SPEAKER_DIR_2D = array<vec2<f32>, 12>(
    vec2<f32>(-0.5000, -0.8660), //  0: Left (FL, -30°)
    vec2<f32>( 0.5000, -0.8660), //  1: Right (FR, +30°)
    vec2<f32>( 0.0000, -1.0000), //  2: Center (C, 0°)
    vec2<f32>( 0.0000,  0.0000), //  3: LFE Subwoofer (Center Origin)
    vec2<f32>(-0.9397,  0.3420), //  4: Surround Left (SL, -110°)
    vec2<f32>( 0.9397,  0.3420), //  5: Surround Right (SR, +110°)
    vec2<f32>(-0.5000,  0.8660), //  6: Rear Left (RL, -150°)
    vec2<f32>( 0.5000,  0.8660), //  7: Rear Right (RR, +150°)
    vec2<f32>(-0.7071, -0.7071), //  8: Top Front Left (TFL, -45°)
    vec2<f32>( 0.7071, -0.7071), //  9: Top Front Right (TFR, +45°)
    vec2<f32>(-0.7071,  0.7071), // 10: Top Rear Left (TRL, -135°)
    vec2<f32>( 0.7071,  0.7071)  // 11: Top Rear Right (TRR, +135°)
);

// --- 7.1.4 Audio Channel Accessors ---

fn get_num_vu_channels() -> u32 {
    if (audio.num_spatial_channels > 2u) {
        return min(audio.num_spatial_channels, 12u);
    }
    if (audio.num_channels > 2u) {
        return min(audio.num_channels, 12u);
    }
    return 12u; // Full 7.1.4 spatialization active for stereo/mono
}

fn get_vu(i: u32) -> f32 {
    // 1. True multichannel spatial mix (e.g. 5.1, 7.1, 7.1.4 Dolby Atmos)
    if (audio.num_spatial_channels > 2u) {
        let n = audio.num_spatial_channels;
        if (i < n) {
            let v = audio.spatial_channels[i / 4u];
            let c = i % 4u;
            if (c == 0u) { return v.x; } else if (c == 1u) { return v.y; }
            else if (c == 2u) { return v.z; } else { return v.w; }
        }
        return 0.0;
    }

    // 2. Multitrack tracker file (e.g. XM, MOD, IT, S3M with >2 channels)
    if (audio.num_channels > 2u) {
        let n = audio.num_channels;
        let idx = min(i, n - 1u);
        let v = audio.channels[idx / 4u];
        let c = idx % 4u;
        if (c == 0u) { return v.x; } else if (c == 1u) { return v.y; }
        else if (c == 2u) { return v.z; } else { return v.w; }
    }

    // 3. Stereo / 2-channel audio: intelligent 7.1.4 spatial upmix
    let vl = audio.channels[0].x;
    let vr = audio.channels[0].y;
    let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x, 0.0, 2.0);
    switch i {
        case 0u  { return vl; }
        case 1u  { return vr; }
        case 2u  { return (vl + vr) * 0.6; }
        case 3u  { return bass; }
        case 4u  { return vl * 0.85 + audio.spectrum[4].x * 0.5; }
        case 5u  { return vr * 0.85 + audio.spectrum[5].x * 0.5; }
        case 6u  { return vl * 0.65 + audio.spectrum[8].x * 0.6; }
        case 7u  { return vr * 0.65 + audio.spectrum[9].x * 0.6; }
        case 8u  { return audio.spectrum[12].x * 0.75; }
        case 9u  { return audio.spectrum[14].x * 0.75; }
        case 10u { return audio.spectrum[18].x * 0.70; }
        default  { return audio.spectrum[22].x * 0.70; }
    }
}

// --- Noise & Smooth Helpers ---

fn hash(n: f32) -> f32 {
    return fract(sin(n) * 43758.5453123);
}

fn hash3_smooth(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec3<f32>(3.0) - 2.0 * f);

    let n = i.x + i.y * 57.0 + i.z * 113.0;

    let a = hash(n + 0.0);
    let b = hash(n + 1.0);
    let c = hash(n + 57.0);
    let d = hash(n + 58.0);
    let e = hash(n + 113.0);
    let f_val = hash(n + 114.0);
    let g = hash(n + 170.0);
    let h_val = hash(n + 171.0);

    return mix(
        mix(mix(a, b, u.x), mix(c, d, u.x), u.y),
        mix(mix(e, f_val, u.x), mix(g, h_val, u.x), u.y),
        u.z
    );
}

fn smax(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (a - b) / k, 0.0, 1.0);
    return mix(b, a, h) + k * h * (1.0 - h);
}

// --- SDF Scene Map (Fluid naturally contained in circular dish) ---

fn map(p: vec3<f32>, full_detail: bool) -> f32 {
    let dist_xz = length(p.xz);
    let p_xz_norm = p.xz / max(dist_xz, 0.0001);

    let num_ch = get_num_vu_channels();
    var total_displacement = 0.0;
    let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x, 0.0, 2.0);

    for (var i = 0u; i < 12u; i++) {
        if (i >= num_ch) { break; }
        let vu = clamp(get_vu(i), 0.0, 1.0);

        if (i == 3u) {
            // LFE Subwoofer: Central magnetic volcano erupting on bass transients
            let spatial_falloff = exp(-dist_xz * 3.0);
            let lfe_lobe = max(vu * 1.5, bass * 1.2) * spatial_falloff;
            total_displacement = smax(total_displacement, lfe_lobe, 0.28);
        } else if (i < 8u) {
            // 7.0 Surround Bed: Primary concentric spike ring at r = 1.40
            let dir2d = SPEAKER_DIR_2D[i];
            let alignment = max(0.0, dot(p_xz_norm, dir2d));
            let dist_to_spike = abs(dist_xz - 1.40);
            let spatial_falloff = exp(-dist_to_spike * 3.5);

            var lobe = pow(alignment, 8.0) * vu * 1.5 * spatial_falloff;
            lobe *= smoothstep(0.15, 0.55, dist_xz);
            total_displacement = smax(total_displacement, lobe, 0.28);
        } else {
            // 0.0.4 Height Channels: Outer satellite ring at r = 2.35 with slender needle spikes
            let dir2d = SPEAKER_DIR_2D[i];
            let alignment = max(0.0, dot(p_xz_norm, dir2d));
            let dist_to_spike = abs(dist_xz - 2.35);
            let spatial_falloff = exp(-dist_to_spike * 4.4);

            var lobe = pow(alignment, 10.0) * vu * 1.55 * spatial_falloff;
            lobe *= smoothstep(0.8, 1.5, dist_xz);
            total_displacement = smax(total_displacement, lobe, 0.28);
        }
    }

    // Physical surface tension resting dome inside circular container
    let resting_pool = BASE_THICKNESS * smoothstep_r(DISH_RADIUS, DISH_RADIUS * 0.35, dist_xz);

    // Subtle acoustic ripples from spectrum bass
    let ripple = sin(dist_xz * 14.0 - audio.time * 8.0) * 0.012 * bass * smoothstep_r(DISH_RADIUS, 0.0, dist_xz);

    // Organic magnetic domain micro-perturbation
    var surface_noise = 0.0;
    let d_base = p.y + 0.5 - (resting_pool + total_displacement + ripple);
    if (full_detail && abs(d_base) < 0.25) {
        let noise_p = p * 4.0 + vec3<f32>(audio.time * 0.5, 0.0, audio.time * 0.3);
        surface_noise = (hash3_smooth(noise_p) - 0.5) * 0.035;
    }

    // Rosensweig magnetic flux fluting along spike flanks
    var fluting = 0.0;
    if (full_detail && total_displacement > 0.03) {
        let angle = atan2(p.z, p.x);
        let rib = cos(dist_xz * 18.0) * cos(angle * 12.0);
        fluting = rib * 0.012 * min(1.0, total_displacement * 2.5);
    }

    // Physical convex meniscus roll-off meeting the container inner wall
    let meniscus = smoothstep_r(DISH_RADIUS, DISH_RADIUS - 0.35, dist_xz);
    let fluid_h = (resting_pool + total_displacement + ripple + surface_noise + fluting) * meniscus;

    // 1. Fluid volume: strictly bounded inside the circular dish radius
    let d_fluid_surf = p.y + 0.5 - fluid_h;
    let d_fluid_cyl = dist_xz - DISH_RADIUS;
    let d_fluid = max(d_fluid_surf, d_fluid_cyl);

    // 2. Machined circular container retaining rim (toroidal bevel)
    let d_rim_cross = vec2<f32>(dist_xz - RIM_RADIUS, p.y - RIM_HEIGHT);
    let d_rim = length(d_rim_cross) - RIM_THICKNESS;

    let d_scene = min(d_fluid, d_rim);
    return d_scene * g_step_scale;
}

// 4-sample tetrahedron normal
fn calcNormal(p: vec3<f32>) -> vec3<f32> {
    let h = NORMAL_EPS;
    let k = vec2<f32>(1.0, -1.0);
    return normalize(
        k.xyy * map(p + k.xyy * h, false) + 
        k.yyx * map(p + k.yyx * h, false) + 
        k.yxy * map(p + k.yxy * h, false) + 
        k.xxx * map(p + k.xxx * h, false)
    );
}

// SDF Ambient Occlusion for deep crevices and contact shadows
fn calcAO(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var ao = 0.0;
    var s = 1.0;
    for (var i = 1; i <= 5; i++) {
        let h = 0.02 + 0.08 * f32(i);
        let d = map(p + n * h, false) / g_step_scale;
        ao += (h - d) * s;
        s *= 0.65;
    }
    return clamp(1.0 - 4.5 * ao, 0.0, 1.0);
}

fn ray_box_intersect(ro: vec3<f32>, rd: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> vec2<f32> {
    let inv_d = 1.0 / (rd + select(vec3<f32>(1e-6), vec3<f32>(-1e-6), rd < vec3<f32>(0.0)));
    let t0 = (bmin - ro) * inv_d;
    let t1 = (bmax - ro) * inv_d;
    let tmin = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let tmax = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    return vec2<f32>(tmin, tmax);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let aspect = max(audio.aspect_ratio, 0.1);
    let p_screen = vec2<f32>(uv.x * aspect, -uv.y);

    // Camera framed looking onto the circular containment stage
    let ro = vec3<f32>(0.0, 2.40, 4.50);
    let cam_target = vec3<f32>(0.0, 0.00, 0.0);

    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = normalize(cross(uu, ww));

    let fov = 1.10;
    let rd = normalize(p_screen.x * uu + p_screen.y * vv + fov * ww);

    var col = vec3<f32>(0.0);
    var t = 0.0;
    var hit = false;
    var final_p = vec3<f32>(0.0);

    let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x, 0.0, 2.0);

    // Circular container bounding box early-out (AABB strictly enclosing the circular dish)
    let bound_r = RIM_RADIUS + RIM_THICKNESS + 0.15;
    let bound_min = vec3<f32>(-bound_r, -0.55, -bound_r);
    let bound_max = vec3<f32>( bound_r,  1.80,  bound_r);
    let t_bounds = ray_box_intersect(ro, rd, bound_min, bound_max);

    let ray_hits_bounds = t_bounds.y >= max(t_bounds.x, 0.0) && t_bounds.x < MAX_MARCH_DIST;

    if (ray_hits_bounds) {
        t = max(t_bounds.x, 0.0);
        let t_end = min(t_bounds.y, MAX_MARCH_DIST);

        let num_ch = get_num_vu_channels();
        var max_vu = 0.0;
        for (var i = 0u; i < num_ch; i++) {
            max_vu = max(max_vu, get_vu(i));
        }
        g_step_scale = min(0.38, 1.0 / sqrt(1.0 + 25.0 * max_vu * max_vu));

        for (var i = 0; i < MAX_MARCH_STEPS; i++) {
            let p_current = ro + rd * t;

            if (rd.y > 0.0 && p_current.y > 1.75) { break; }

            let d = map(p_current, true);

            if (d < HIT_THRESHOLD) {
                hit = true;
                final_p = p_current;
                break;
            }

            t += d;
            if (t > t_end) { break; }
        }
    }

    if (hit) {
        let n = calcNormal(final_p);
        let v = -rd;
        let ref_dir = reflect(rd, n);
        let NdotV = max(0.0, dot(n, v));
        let dist_hit_xz = length(final_p.xz);

        // Check if hit point is on the circular container rim or liquid metal
        let is_container_rim = dist_hit_xz > (DISH_RADIUS - 0.05) && final_p.y < -0.38;

        // Overhead Studio Softbox Strip
        let t_sb = (5.2 - final_p.y) / max(0.01, ref_dir.y);
        var spec_sb = 0.0;
        if (t_sb > 0.0 && ref_dir.y > 0.0) {
            let hit_sb = final_p + ref_dir * t_sb;
            let edge_x = smoothstep(2.5, 0.5, abs(hit_sb.x));
            let edge_z = smoothstep(5.0, 1.4, abs(hit_sb.z));
            spec_sb = edge_x * edge_z;
        }
        let sb_radiance = vec3<f32>(3.8, 4.0, 4.3) * (1.0 + bass * 0.35);
        var refl = sb_radiance * spec_sb;

        // Neutral Kicker 1 (Left flank)
        let light_pos1 = vec3<f32>(-4.5, 3.2, -1.0);
        let L1 = normalize(light_pos1 - final_p);
        let spec1 = pow(max(0.0, dot(ref_dir, L1)), SPEC_POWER);
        refl += vec3<f32>(2.4, 2.6, 2.9) * spec1 * 1.6;

        // Neutral Kicker 2 (Right flank)
        let light_pos2 = vec3<f32>(4.5, 2.6, -0.6);
        let L2 = normalize(light_pos2 - final_p);
        let spec2 = pow(max(0.0, dot(ref_dir, L2)), SPEC_POWER * 1.25);
        refl += vec3<f32>(2.5, 2.3, 2.0) * spec2 * 1.4;

        // Dark studio environment dome reflection
        let env = mix(vec3<f32>(0.015, 0.018, 0.022), vec3<f32>(0.07, 0.08, 0.10), smoothstep(-0.2, 0.8, ref_dir.y));
        refl += env;

        let ao = calcAO(final_p, n);

        if (is_container_rim) {
            // =================================================================
            // Machined Titanium Container Retaining Rim (Circular Dish Edge)
            // =================================================================
            let rim_angle = atan2(final_p.z, final_p.x);
            let rim_lathe = sin(rim_angle * 180.0) * 0.003;
            let rim_base = vec3<f32>(0.045, 0.048, 0.052) + rim_lathe;
            let rim_fresnel = 0.50 + 0.50 * pow(1.0 - NdotV, 4.0);
            col = (rim_base + refl * rim_fresnel * 0.75) * ao;
        } else {
            // =================================================================
            // Pure Liquid Metal Ferrofluid (Liquid Chrome / Mercury)
            // =================================================================
            let fresnel = 0.72 + 0.28 * pow(1.0 - NdotV, 5.0);
            let metal_base = vec3<f32>(0.026, 0.028, 0.032);
            col = (metal_base + refl * fresnel) * ao;
        }
    } else {
        // =====================================================================
        // Analytical Stage Plate & Dark Studio Cyclorama (Beyond the Dish)
        // =====================================================================
        if (rd.y < -0.001) {
            let t_floor = (-0.5 - ro.y) / rd.y;
            if (t_floor > 0.0 && t_floor < MAX_MARCH_DIST) {
                let p_floor = ro + rd * t_floor;
                let r_floor = length(p_floor.xz);

                if (r_floor < 5.8) {
                    // Brushed Titanium Containment Stage Plate
                    let brush = sin(p_floor.x * 120.0 + p_floor.z * 120.0) * 0.003;
                    let groove = sin(r_floor * 50.0) * 0.004;
                    var stage_albedo = vec3<f32>(0.038, 0.040, 0.044) + brush + groove;

                    // Precision etched concentric circular magnetic induction rings
                    let dish_outer_shadow = smoothstep(RIM_RADIUS + RIM_THICKNESS, RIM_RADIUS + RIM_THICKNESS + 0.40, r_floor);
                    let coil_ring1 = exp(-abs(r_floor - 3.80) * 35.0) * 0.035;
                    let coil_ring2 = exp(-abs(r_floor - 4.50) * 35.0) * 0.030;
                    let outer_lip   = exp(-abs(r_floor - 5.50) * 25.0) * 0.055;
                    stage_albedo += vec3<f32>(coil_ring1 + coil_ring2 + outer_lip);

                    // Contact shadow directly beneath the container dish
                    stage_albedo *= mix(0.35, 1.0, dish_outer_shadow);

                    // Softbox specular reflection on brushed stage plate
                    let n_floor = vec3<f32>(0.0, 1.0, 0.0);
                    let ref_floor = reflect(rd, n_floor);
                    let t_sb = (5.2 - (-0.5)) / max(0.01, ref_floor.y);
                    var floor_spec = 0.0;
                    if (t_sb > 0.0 && ref_floor.y > 0.0) {
                        let hit_sb = p_floor + ref_floor * t_sb;
                        floor_spec = smoothstep(2.6, 0.4, abs(hit_sb.x)) * smoothstep(5.0, 1.2, abs(hit_sb.z)) * 0.22;
                    }
                    stage_albedo += vec3<f32>(0.08, 0.085, 0.09) * floor_spec;

                    col = stage_albedo;
                    t = t_floor;
                } else {
                    // Dark studio floor beyond the stage plate
                    col = mix(vec3<f32>(0.024, 0.026, 0.030), vec3<f32>(0.010, 0.012, 0.015), smoothstep(5.8, 14.0, r_floor));
                    t = t_floor;
                }
            } else {
                col = mix(vec3<f32>(0.010, 0.012, 0.016), vec3<f32>(0.030, 0.034, 0.042), rd.y * 0.5 + 0.5);
            }
        } else {
            // Upper hemisphere: dark studio cyclorama
            col = mix(vec3<f32>(0.010, 0.012, 0.016), vec3<f32>(0.030, 0.034, 0.042), rd.y * 0.5 + 0.5);
        }
    }

    // Atmospheric studio depth falloff
    let bg = mix(vec3<f32>(0.010, 0.012, 0.016), vec3<f32>(0.030, 0.034, 0.042), rd.y * 0.5 + 0.5);
    col = mix(col, bg, smoothstep(12.0, MAX_MARCH_DIST, t));

    // Studio vignette
    let vignette = 1.0 - smoothstep(0.65, 1.55, length(uv));
    col *= vignette;

    // Narkowicz ACES tonemap
    col = aces_tonemap(col);

    return vec4<f32>(col, 1.0);
}
