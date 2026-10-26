// INCLUDE: common

// ============================================================================
// Visualizer ID 28: Acoustic Water & Ocean Surface
// Navier-Stokes FLIP Hydrodynamic Ocean with Audio-Reactive Swells
// Refractive Water (IOR 1.333), Ridged Cellular Caustics,
// Multi-Directional Gravity Waves, Cymatic Harmonics & Acoustic Plume
// ============================================================================

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var<storage, read> render_grid: array<u32>;

const RENDER_GRID_SIZE: i32 = 512;
const BASIN_FLOOR_Y: f32 = -0.35;
const BASE_WATER_Y: f32 = 0.42;
const MAX_WATER_Y: f32 = 1.25;

// Raymarch tuning
const MAX_STEPS: i32 = 80;
const HIT_EPS: f32 = 0.0020;
const STEP_SCALE: f32 = 0.62;

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

// Sample macro hydrodynamic energy from the FLIP simulation grid
// so the FLIP solver drives inertial fluid swell without discrete particle noise
fn sample_macro_flip_energy() -> f32 {
    let c = RENDER_GRID_SIZE / 2;
    let h0 = get_grid_height(c, c);
    let h1 = get_grid_height(c + 12, c) + get_grid_height(c - 12, c)
           + get_grid_height(c, c + 12) + get_grid_height(c, c - 12);
    let h2 = get_grid_height(c + 28, c + 28) + get_grid_height(c - 28, c + 28)
           + get_grid_height(c + 28, c - 28) + get_grid_height(c - 28, c - 28);
    return clamp((h0 * 2.0 + h1 + h2 * 0.5) * 0.35, 0.0, 0.45);
}

// Continuous C-infinity smooth 3D open-ocean + acoustic cymatic water surface heightfield
fn sample_water_height(p: vec2<f32>) -> f32 {
    let r = length(p);
    let r2 = dot(p, p);
    let theta = atan2(p.y, p.x);

    let flip_boost = g_flip_energy * 0.28;
    let bass_drive = g_sub_bass + flip_boost;

    // 1. Central Sub-Bass Acoustic Fountain Plume & Concentric Rayleigh Collars
    let plume_pulse = 0.90 + 0.10 * sin(g_time * 6.4);
    let plume_core = exp(-r2 * 2.4) * (0.06 + bass_drive * 0.34) * plume_pulse;
    let collar1_r = 0.85 + 0.08 * sin(g_time * 3.0);
    let collar1 = exp(-pow(r - collar1_r, 2.0) * 8.5) * (0.025 + bass_drive * 0.105);
    let trough1 = -exp(-pow(r - 0.52, 2.0) * 14.0) * (0.015 + bass_drive * 0.042);

    // 2. Expansive Radially Propagating Gravity Wave Rings (unbounded across the viewport)
    let dist_atten = exp(-r * 0.085);
    let wave_out1 = sin(r * 3.8 - g_time * 4.6) * smoothstep(0.15, 0.65, r);
    let wave_out2 = sin(r * 6.4 - g_time * 5.8 + 1.2) * 0.55 * smoothstep(0.25, 0.95, r);
    let radial_waves = (wave_out1 + wave_out2) * dist_atten * (0.028 + bass_drive * 0.068 + g_lowmids * 0.042);

    // 3. Multi-Directional Open-Ocean Swells (fills the entire viewport edge-to-edge)
    let d1 = vec2<f32>(0.866, 0.500);
    let d2 = vec2<f32>(-0.643, 0.766);
    let d3 = vec2<f32>(0.342, -0.940);
    let d4 = vec2<f32>(-0.906, -0.423);

    let swell1 = sin(dot(p, d1) * 1.45 + g_time * 2.1);
    let swell2 = sin(dot(p, d2) * 2.15 - g_time * 2.6 + swell1 * 0.35);
    let swell3 = cos(dot(p, d3) * 3.30 + g_time * 3.2);
    let swell4 = sin(dot(p, d4) * 5.10 - g_time * 4.1);

    let ocean_swell = (swell1 * 0.040 + swell2 * 0.028) * (1.0 + bass_drive * 0.55 + g_lowmids * 0.45)
                    + (swell3 * 0.016 + swell4 * 0.009) * (1.0 + g_mids * 0.75 + g_highs * 0.65);

    // 4. Multi-Pole Azimuthal Faraday / Cymatic Standing Wave Harmonics
    let cymatic_atten = exp(-r * 0.11);
    let mode4 = cos(r * 5.2 - g_time * 3.2) * cos(theta * 4.0 + g_time * 0.30) * smoothstep(0.30, 1.15, r);
    let mode6 = sin(r * 8.4 + g_time * 3.8) * cos(theta * 6.0 - g_time * 0.40) * smoothstep(0.55, 1.70, r);
    let mode8 = cos(r * 11.8 - g_time * 4.5) * sin(theta * 8.0 + g_time * 0.25) * smoothstep(0.85, 2.20, r);
    let cymatic_ripples = (mode4 * (0.015 + g_mids * 0.032)
                        + mode6 * (0.009 + g_highs * 0.022)
                        + mode8 * (0.005 + g_highs * 0.014)) * cymatic_atten;

    let far_fade = exp(-r * 0.035);
    let dynamic_waves = plume_core + collar1 + trough1 + radial_waves + cymatic_ripples + ocean_swell * far_fade;

    return BASE_WATER_Y + dynamic_waves;
}

fn calc_water_normal(p: vec2<f32>, eps: f32) -> vec3<f32> {
    let e = vec2<f32>(eps, 0.0);
    let dx = (sample_water_height(p + e.xy) - sample_water_height(p - e.xy)) / (2.0 * eps);
    let dz = (sample_water_height(p + e.yx) - sample_water_height(p - e.yx)) / (2.0 * eps);
    return normalize(vec3<f32>(-dx, 1.0, -dz));
}

// True Ridged Cellular / Voronoi Caustic Filament Network (continuous intersecting photon webs)
fn calculate_caustics(pos: vec2<f32>, time: f32, bass: f32) -> f32 {
    let p = pos * 1.05;
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
    let dist_fade = exp(-r * 0.09);

    // Chladni radial + azimuthal standing-wave laser nodal filaments across the seabed
    let nodal1 = cos(r * 4.8 - time * 1.8) * cos(theta * 4.0) + 0.55 * sin(r * 7.6 + time * 1.3) * sin(theta * 6.0);
    let chladni_line = exp(-abs(nodal1) * 16.0) * dist_fade;

    // Concentric harmonic resonator rings
    let ring_grid = exp(-abs(fract(r * 0.95) - 0.5) * 32.0) * 0.38 * dist_fade;
    // 12 radial acoustic waveguide spokes
    let spoke_line = exp(-abs(sin(theta * 6.0)) * 28.0) * smoothstep(0.35, 1.10, r) * dist_fade * 0.24;

    let cymatic_col = mix(vec3<f32>(0.06, 0.72, 1.20), vec3<f32>(0.55, 0.35, 1.30), clamp(highs * 0.65, 0.0, 1.0));
    let gold_accent = vec3<f32>(1.10, 0.78, 0.32) * exp(-abs(r - 1.45) * 20.0) * (0.25 + sub_bass * 0.45);

    return cymatic_col * (chladni_line * (0.22 + mids * 0.55 + highs * 0.40) + (ring_grid + spoke_line) * (0.18 + sub_bass * 0.28)) + gold_accent;
}

// Atmospheric sky & area-light reflection for the open ocean surface
fn eval_studio_reflection(pos: vec3<f32>, ref_dir: vec3<f32>, sub_bass: f32) -> vec3<f32> {
    var refl = vec3<f32>(0.0);

    // 1. Primary Overhead Diffused Sky Panel (at y = 6.5)
    if (ref_dir.y > 0.01) {
        let t_sb = (6.5 - pos.y) / ref_dir.y;
        let hit_sb = pos.xz + ref_dir.xz * t_sb;
        let sb_x = smoothstep_r(5.8, 1.8, abs(hit_sb.x - 0.6));
        let sb_z = smoothstep_r(6.2, 2.0, abs(hit_sb.y - 0.8));
        let louver = 0.90 + 0.10 * cos(hit_sb.x * 5.0);
        refl += vec3<f32>(2.1, 2.35, 2.75) * (sb_x * sb_z * louver) * (1.0 + sub_bass * 0.25);
    }

    // 2. Left Horizon Rim Glow (Electric Cyan-Sapphire)
    let strip1_dir = normalize(vec3<f32>(-6.5, 2.6, 2.2) - pos);
    let align1 = max(0.0, dot(ref_dir, strip1_dir));
    let strip1 = pow(align1, 18.0) * 1.25 + pow(align1, 120.0) * 2.8;
    refl += vec3<f32>(0.22, 0.82, 1.45) * strip1;

    // 3. Right Horizon Kicker Glow (Warm Champagne Gold & Violet)
    let strip2_dir = normalize(vec3<f32>(6.5, 3.0, -2.0) - pos);
    let align2 = max(0.0, dot(ref_dir, strip2_dir));
    let strip2 = pow(align2, 22.0) * 1.1 + pow(align2, 140.0) * 2.4;
    refl += vec3<f32>(1.30, 0.92, 0.58) * strip2;

    // 4. Ambient Ocean Sky Dome Gradient
    let sky_grad = smoothstep(-0.15, 0.85, ref_dir.y);
    let horizon_band = exp(-abs(ref_dir.y) * 9.0) * (0.14 + sub_bass * 0.08);
    refl += mix(vec3<f32>(0.015, 0.032, 0.065), vec3<f32>(0.09, 0.19, 0.34), sky_grad)
          + vec3<f32>(0.10, 0.45, 0.85) * horizon_band;

    return refl;
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

    // Panoramic camera framing looking out across the expansive water surface
    let portrait_scale = max(1.0, 1.25 / aspect);
    let cam_orbit = time * 0.055;
    let cam_dist = 4.25 * portrait_scale;
    let cam_h = (2.15 + sin(time * 0.14) * 0.08) * portrait_scale;
    let ro = vec3<f32>(cos(cam_orbit) * cam_dist, cam_h, sin(cam_orbit) * cam_dist);
    let cam_target = vec3<f32>(0.0, 0.36, 0.0);

    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = cross(uu, ww);
    let rd = normalize(screen_p.x * uu + screen_p.y * vv + 1.65 * ww);

    // Atmospheric night-ocean sky & horizon glow
    let sky_elev = max(0.0, rd.y);
    let horizon_glow = exp(-abs(rd.y + 0.02) * 8.5) * (0.08 + sub_bass * 0.04);
    let bg_col = mix(
        vec3<f32>(0.016, 0.038, 0.078),
        vec3<f32>(0.005, 0.012, 0.026),
        smoothstep(0.0, 0.55, sky_elev)
    ) + vec3<f32>(0.08, 0.42, 0.82) * horizon_glow;
    var col = bg_col;

    // Primary light positions
    let softbox_pos = vec3<f32>(2.8, 7.2, 3.6);
    let kicker_pos = vec3<f32>(-5.5, 4.5, -4.2);

    // --- Raymarch Unbounded 3D Water / Ocean Surface ---
    var water_hit = false;
    var hit_p = vec3<f32>(0.0);
    var t_water = 0.0;

    if (rd.y < -0.0005) {
        let t_start = max(0.0, (MAX_WATER_Y - ro.y) / rd.y);
        let t_max = min(60.0, (BASIN_FLOOR_Y - ro.y) / rd.y);

        if (t_start < t_max) {
            var t = t_start;
            var t_prev = t_start;
            let p_start = ro + rd * t_start;
            var dist_prev = p_start.y - sample_water_height(p_start.xz);

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

                t_prev = t;
                dist_prev = dist;
                // Adaptive step size that scales smoothly with distance to reach the horizon
                t += max(dist * STEP_SCALE, 0.014 + t * 0.008);
                if (t >= t_max) {
                    break;
                }
            }

            // Analytic distant plane fallback so water extends seamlessly to the horizon
            if (!water_hit && ro.y > BASE_WATER_Y) {
                let t_plane = (BASE_WATER_Y - ro.y) / rd.y;
                if (t_plane > 0.0 && t_plane < 85.0) {
                    water_hit = true;
                    t_water = t_plane;
                    hit_p = ro + rd * t_plane;
                }
            }
        }
    }

    if (water_hit) {
        // --- Shade Continuous 3D Ocean Surface, Reflections, Refraction & Caustics ---
        // Scale normal derivative epsilon slightly with distance to prevent horizon aliasing
        let eps = clamp(0.022 + t_water * 0.0018, 0.022, 0.085);
        let normal = calc_water_normal(hit_p.xz, eps);
        let view_dir = -rd;
        let NdotV = max(0.0, dot(normal, view_dir));

        // Dielectric Fresnel reflection for water (F0 = 0.028)
        let fresnel_water = 0.028 + 0.972 * pow(1.0 - NdotV, 4.5);

        // Sky & Dual Rim Reflections on 3D Wave Slopes
        let ref_dir = reflect(rd, normal);
        let env_reflection = eval_studio_reflection(hit_p, ref_dir, sub_bass);

        // Pinpoint specular glints on steep wave crests
        let l1_dir = normalize(softbox_pos - hit_p);
        let h1 = normalize(l1_dir + view_dir);
        let NdotL1 = max(0.0, dot(normal, l1_dir));
        let spec1_tight = pow(max(0.0, dot(normal, h1)), 220.0) * 4.5;
        let spec1_soft = pow(max(0.0, dot(normal, h1)), 26.0) * 0.72;
        let water_specular1 = vec3<f32>(1.15, 1.22, 1.35) * (spec1_tight + spec1_soft) * NdotL1;

        let l2_dir = normalize(kicker_pos - hit_p);
        let h2 = normalize(l2_dir + view_dir);
        let NdotL2 = max(0.0, dot(normal, l2_dir));
        let spec2 = pow(max(0.0, dot(normal, h2)), 92.0) * 2.2;
        let water_specular2 = vec3<f32>(0.25, 0.88, 1.45) * spec2 * NdotL2;

        // Snell's Law Refraction (IOR 1.0 -> 1.333) & Beer-Lambert Optical Absorption
        let refr_dir = refract(rd, normal, 1.0 / 1.333);
        let water_depth = max(0.12, hit_p.y - BASIN_FLOOR_Y);
        let t_refr_floor = water_depth / max(0.15, -refr_dir.y);
        let floor_p = hit_p.xz + refr_dir.xz * min(t_refr_floor, 2.2);

        // Rich emerald-sapphire Beer-Lambert depth absorption
        let absorption_coeff = vec3<f32>(0.38, 0.068, 0.012);
        let transmission = exp(-absorption_coeff * (t_refr_floor * 2.2));

        // Wave-slope Jacobian focusing: steeper wave crests concentrate brighter caustic filaments
        let wave_slope = length(normal.xz);
        let jacobian_focus = clamp(1.0 + wave_slope * 3.8, 1.0, 2.8);
        let caustic_dist_fade = exp(-length(hit_p.xz) * 0.045);
        let caustics = calculate_caustics(floor_p, time, sub_bass) * jacobian_focus * caustic_dist_fade;
        let cymatic_grid = eval_cymatic_floor_grid(floor_p, time, sub_bass, mids, highs);

        // Subtle chromatic dispersion on refracted caustics
        let caustics_r = calculate_caustics(floor_p + normal.xz * 0.025, time, sub_bass);
        let caustics_b = calculate_caustics(floor_p - normal.xz * 0.025, time, sub_bass);
        let caustic_rgb = vec3<f32>(
            caustics_r * 0.18,
            caustics * 0.68,
            (caustics * 0.5 + caustics_b * 0.5) * 1.15
        ) * jacobian_focus * caustic_dist_fade;

        let seabed_plate = vec3<f32>(0.010, 0.022, 0.042) + caustic_rgb * 0.28 + cymatic_grid;

        // Acoustic Center Core & Expanding Resonator Ring Luminescence
        let center_dist = length(floor_p);
        let ring_pulse = exp(-pow(center_dist - 0.85, 2.0) * 6.0) * (0.30 + sub_bass * 0.65);
        let bass_core = exp(-center_dist * center_dist * 1.8) * (0.25 + sub_bass * 0.60);
        let transducer_emission = ring_pulse * vec3<f32>(0.12, 0.82, 1.35) + bass_core * vec3<f32>(0.18, 0.68, 1.25);

        // Volumetric subsurface scattering inside elevated wave crests & central acoustic plume
        let crest_elev = clamp((hit_p.y - BASE_WATER_Y) * 3.0, 0.0, 1.0);
        let sss_plume = vec3<f32>(0.07, 0.54, 0.95) * crest_elev * (0.35 + sub_bass * 0.65) * (1.0 - NdotV * 0.5);

        let water_body = (seabed_plate + transducer_emission) * transmission + sss_plume;

        var water_col = mix(water_body, env_reflection, fresnel_water * 0.86 + 0.06);
        water_col += (water_specular1 + water_specular2) * (0.35 + 0.65 * fresnel_water);

        // Smooth atmospheric horizon fog blend at far distance
        let horizon_fog = smoothstep(14.0, 52.0, t_water);
        col = mix(water_col, bg_col, horizon_fog);
    }

    // ACES Tonemapping for photorealistic HDR highlight rolloff
    col = aces_tonemap(col);

    // Subtle edge vignette
    let d_center = length(uv - vec2<f32>(0.5));
    col *= clamp(1.25 - d_center * 0.55, 0.0, 1.0);

    return vec4<f32>(col, 1.0);
}
