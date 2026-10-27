// INCLUDE: common

// ============================================================================
// Visualizer ID 26: Quantum FLIP Ferrofluid
// Incompressible GPU FLIP Hydrodynamic Raymarched Heightfield
// Obsidian-Black Liquid Chrome with Audio-Reactive Electromagnetic Spikes
// ============================================================================

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var<storage, read> render_grid: array<u32>;

const RENDER_GRID_SIZE: i32 = 512;
const DISH_RADIUS: f32 = 3.35;
const DOMAIN_HALF: f32 = 3.6;

// Raymarch tuning
const MAX_STEPS: i32 = 16;
const HIT_EPS: f32 = 0.0045;
const STEP_SCALE: f32 = 0.72;

fn get_grid_height(gx: i32, gz: i32) -> f32 {
    if (gx < 0 || gx >= RENDER_GRID_SIZE || gz < 0 || gz >= RENDER_GRID_SIZE) {
        return 0.0;
    }
    let cell = u32(gz) * u32(RENDER_GRID_SIZE) + u32(gx);
    return f32(render_grid[cell]) * 0.001; // Scaled by 1000 in compute shader
}

// Bilinear heightfield lookup with precomputed radius squared
fn sample_height_r2(p: vec2<f32>, r2: f32) -> f32 {
    let floor_y = 0.025 + 0.022 * r2;

    if (r2 > (DISH_RADIUS + 0.2) * (DISH_RADIUS + 0.2)) {
        return floor_y;
    }

    // Base meniscus puddle: resting liquid depth that smoothly tapers at the dish rim
    // DISH_RADIUS^2 = 11.2225, (DISH_RADIUS - 0.45)^2 = 8.41
    let dish_mask = smoothstep_r(11.2225, 8.41, r2);
    let base_puddle = floor_y + 0.055 * dish_mask;

    // World [-3.6, 3.6] -> [0, 512]
    let gx_f = (p.x + DOMAIN_HALF) * (512.0 / 7.2);
    let gz_f = (p.y + DOMAIN_HALF) * (512.0 / 7.2);

    let ix = i32(floor(gx_f));
    let iz = i32(floor(gz_f));
    let fx = fract(gx_f);
    let fz = fract(gz_f);

    // Bilinear sample
    let h00 = get_grid_height(ix, iz);
    let h10 = get_grid_height(ix + 1, iz);
    let h01 = get_grid_height(ix, iz + 1);
    let h11 = get_grid_height(ix + 1, iz + 1);

    let h_fluid = mix(mix(h00, h10, fx), mix(h01, h11, fx), fz);

    // Continuous liquid puddle floor: fluid never dips below resting meniscus
    return max(base_puddle, h_fluid);
}

// Bilinear heightfield lookup with continuous meniscus puddle
fn sample_height(p: vec2<f32>) -> f32 {
    return sample_height_r2(p, dot(p, p));
}

fn calc_normal(p: vec2<f32>, eps: f32) -> vec3<f32> {
    let e = vec2<f32>(eps, 0.0);
    let h0 = sample_height(p);
    let dx = (sample_height(p + e.xy) - h0) / eps;
    let dz = (sample_height(p + e.yx) - h0) / eps;
    return normalize(vec3<f32>(-dx, 1.0, -dz));
}

// Fast Ray-AABB intersection for early background rejection
fn intersect_aabb(ro: vec3<f32>, rd: vec3<f32>, box_min: vec3<f32>, box_max: vec3<f32>) -> vec2<f32> {
    let inv_d = 1.0 / rd;
    let t0 = (box_min - ro) * inv_d;
    let t1 = (box_max - ro) * inv_d;

    let t_min = min(t0, t1);
    let t_max = max(t0, t1);

    let t_near = max(max(t_min.x, t_min.y), t_min.z);
    let t_far = min(min(t_max.x, t_max.y), t_max.z);

    return vec2<f32>(t_near, t_far);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let aspect = audio.aspect_ratio;
    let screen_p = (uv * 2.0 - 1.0) * vec2<f32>(aspect, -1.0);

    // Subtle audio-driven camera orbit using continuous smooth_time
    let time = select(audio.time, audio.smooth_time, audio.smooth_time > 0.0);
    let cam_orbit = time * 0.12;
    let cam_dist = 4.2;
    let cam_h = 2.45 + sin(time * 0.25) * 0.2;
    let ro = vec3<f32>(cos(cam_orbit) * cam_dist, cam_h, sin(cam_orbit) * cam_dist);
    let cam_target = vec3<f32>(0.0, 0.45, 0.0);

    // Camera matrix
    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = cross(uu, ww);
    let rd = normalize(screen_p.x * uu + screen_p.y * vv + 1.85 * ww);

    // Studio background gradient
    let bg_grad = clamp(0.04 - 0.025 * length(screen_p), 0.005, 0.06);
    var col = vec3<f32>(bg_grad * 0.8, bg_grad * 0.85, bg_grad * 1.1);

    // Under-dish glow reflection on floor (normalized from [0, 100] spectrum scale)
    let sub_bass = clamp((audio.spectrum[0].x + audio.spectrum[0].y) / 110.0, 0.0, 1.8);
    let floor_glow = exp(-length(screen_p - vec2<f32>(0.0, -0.4)) * 1.8) * (0.04 + sub_bass * 0.08);
    col += vec3<f32>(0.1, 0.06, 0.18) * floor_glow;

    // Exact Ray-Cylinder + Vertical Slab intersection for circular dish (eliminates corner AABB waste)
    let max_r2 = (DISH_RADIUS + 0.12) * (DISH_RADIUS + 0.12);
    let a_cyl = dot(rd.xz, rd.xz);
    let b_cyl = dot(ro.xz, rd.xz);
    let c_cyl = dot(ro.xz, ro.xz) - max_r2;
    let disc = b_cyl * b_cyl - a_cyl * c_cyl;
    if (disc <= 0.0) {
        return vec4<f32>(aces_tonemap(col), 1.0);
    }
    let sqrt_disc = sqrt(disc);
    let inv_a_cyl = 1.0 / max(a_cyl, 1e-6);
    let t_cyl0 = (-b_cyl - sqrt_disc) * inv_a_cyl;
    let t_cyl1 = (-b_cyl + sqrt_disc) * inv_a_cyl;

    let inv_rdy = 1.0 / rd.y;
    let ty0 = (0.0 - ro.y) * inv_rdy;
    let ty1 = (1.32 - ro.y) * inv_rdy;
    let t_near = max(max(0.0, t_cyl0), min(ty0, ty1));
    let t_far  = min(t_cyl1, max(ty0, ty1));

    if (t_near >= t_far || t_far <= 0.0) {
        // Ray missed circular fluid dish completely — return background immediately
        return vec4<f32>(aces_tonemap(col), 1.0);
    }

    var t_start = t_near;
    // Outside central spike zone (r2 > 7.5), fluid is only the resting meniscus puddle (y <= 0.30)
    let p_entry = ro + rd * t_start;
    if (rd.y < -0.01 && dot(p_entry.xz, p_entry.xz) > 7.5 && dot(p_entry.xz, rd.xz) >= -0.2) {
        t_start = max(t_start, (0.32 - ro.y) * inv_rdy);
    }

    let t_end = t_far;
    var t = t_start;
    var t_prev = t_start;
    var dist_prev = 1.0;
    var hit = false;
    var hit_p = vec3<f32>(0.0);
    let inv_slant = 1.0 / (max(-rd.y, 0.02) + 0.92);

    // March heightfield inside circular cylinder
    for (var i = 0; i < MAX_STEPS; i++) {
        let p = ro + rd * t;
        let r2 = dot(p.xz, p.xz);
        let h = sample_height_r2(p.xz, r2);
        let dist = p.y - h;

        if (dist < HIT_EPS) {
            let denom = max(dist_prev - dist, 0.0001);
            let frac_t = clamp(dist_prev / denom, 0.0, 1.0);
            t = mix(t_prev, t, frac_t);
            hit = true;
            hit_p = ro + rd * t;
            break;
        }

        // Sky early-out: ray is pointing up and is already above max possible fluid height
        if (rd.y > 0.0 && p.y > 1.30) {
            break;
        }

        t_prev = t;
        dist_prev = dist;
        t += max(dist * inv_slant, 0.032);
        if (t >= t_end) { break; }
    }

    if (hit) {
        let normal = calc_normal(hit_p.xz, 0.025);
        let view_dir = -rd;
        let NdotV = max(0.0, dot(normal, view_dir));
        let r2_hit = dot(hit_p.xz, hit_p.xz);
        let r_xz = sqrt(r2_hit);
        let floor_y = 0.025 + 0.022 * r2_hit;
        let dish_mask = smoothstep(DISH_RADIUS, DISH_RADIUS - 0.45, r_xz);
        let base_puddle = floor_y + 0.055 * dish_mask;
        let spike_h = max(0.0, hit_p.y - base_puddle);

        // --- Obsidian Chrome Material Lighting ---
        // Obsidian base color: pitch black with faint graphite undertone
        let albedo = vec3<f32>(0.012, 0.014, 0.018);

        // Fresnel reflection (Schlick approximation via fast squaring)
        let F0 = 0.62; // High metallic chrome reflectivity
        let om_nv = 1.0 - NdotV;
        let om_nv2 = om_nv * om_nv;
        let om_nv3 = om_nv2 * om_nv;
        let fresnel = F0 + (1.0 - F0) * (om_nv2 * om_nv3);

        // Primary Overhead Softbox Area Light (Warm Studio Amber)
        let light1_pos = vec3<f32>(2.2, 5.2, 2.5);
        let l1_dir = normalize(light1_pos - hit_p);
        let h1 = normalize(l1_dir + view_dir);
        let NdotL1 = max(0.0, dot(normal, l1_dir));
        let ndh1_2 = max(0.0, dot(normal, h1)) * max(0.0, dot(normal, h1));
        let ndh1_4 = ndh1_2 * ndh1_2;
        let ndh1_8 = ndh1_4 * ndh1_4;
        let ndh1_16 = ndh1_8 * ndh1_8;
        let ndh1_28 = ndh1_16 * ndh1_8 * ndh1_4;
        let ndh1_64 = ndh1_16 * ndh1_16 * ndh1_16 * ndh1_16;
        let ndh1_220 = ndh1_64 * ndh1_64 * ndh1_64 * ndh1_28;
        let spec1_tight = ndh1_220 * 5.2;
        let spec1_soft = ndh1_28 * 0.85;
        let light1_col = vec3<f32>(1.12, 0.98, 0.82) * (spec1_tight + spec1_soft) * NdotL1;

        // Secondary Kicker Light (Cool Blue-Cyan Rim Light)
        let light2_pos = vec3<f32>(-2.8, 4.0, -2.8);
        let l2_dir = normalize(light2_pos - hit_p);
        let h2 = normalize(l2_dir + view_dir);
        let NdotL2 = max(0.0, dot(normal, l2_dir));
        let ndh2_2 = max(0.0, dot(normal, h2)) * max(0.0, dot(normal, h2));
        let ndh2_4 = ndh2_2 * ndh2_2;
        let ndh2_8 = ndh2_4 * ndh2_4;
        let ndh2_16 = ndh2_8 * ndh2_8;
        let ndh2_32 = ndh2_16 * ndh2_16;
        let ndh2_64 = ndh2_32 * ndh2_32;
        let spec2 = (ndh2_64 * ndh2_32 * ndh2_16) * 2.6;
        let light2_col = vec3<f32>(0.45, 0.85, 1.25) * spec2 * NdotL2;

        // Electromagnetic Under-Basin Coil Radiance
        // Concentric neon filament coils visible in the glass dish rim and around erupting spike roots
        let cf_1 = max(0.0, sin(r_xz * 28.0 - time * 1.5));
        let cf_2 = cf_1 * cf_1;
        let coil_freq = cf_2 * cf_2 * cf_2;
        let coil_col = mix(vec3<f32>(1.0, 0.40, 0.06), vec3<f32>(0.10, 0.75, 1.0), sin(r_xz * 6.0) * 0.5 + 0.5);
        
        // Dish glass rim glow: visible where fluid thins out towards container edge
        let dish_glass_glow = smoothstep(DISH_RADIUS - 0.45, DISH_RADIUS, r_xz) * coil_freq * (0.8 + sub_bass * 0.8);
        // Spike root flux: electromagnetic energy exciting fluid at the base of erupting spikes
        let spike_root_flux = smoothstep(0.01, 0.12, spike_h) * smoothstep(0.45, 0.10, spike_h) * (0.35 + sub_bass * 0.5);
        let underglow = coil_col * (dish_glass_glow + spike_root_flux) * (1.0 - NdotV * 0.4);

        // Iridescent Thin-Film Sheen on Sharp Magnetic Spike Tips
        let spike_crest = smoothstep(0.35, 1.1, spike_h);
        let irid_t = fract(dot(normal, vec3<f32>(1.0, 2.0, 1.0)) * 0.65 + time * 0.1);
        let irid_col = iridescence_cos(irid_t) * spike_crest * (0.35 + sub_bass * 0.45);

        // Environment reflection
        let ref_dir = reflect(-view_dir, normal);
        let env_sky = max(0.0, ref_dir.y) * vec3<f32>(0.16, 0.20, 0.28);
        let env_floor = max(0.0, -ref_dir.y) * vec3<f32>(0.04, 0.02, 0.06);

        // Compose fluid color
        var fluid_col = albedo + underglow;
        fluid_col += (light1_col + light2_col) * fresnel;
        fluid_col += (env_sky + env_floor) * fresnel;
        fluid_col += irid_col * om_nv3;

        // Dish rim & 16-pole azimuthal electromagnetic coil segments
        if (r_xz > DISH_RADIUS - 0.14) {
            let rim_factor = smoothstep(DISH_RADIUS - 0.08, DISH_RADIUS, r_xz);
            let rim_metal = vec3<f32>(0.18, 0.19, 0.22) * (0.35 + 0.65 * NdotL1);
            fluid_col = mix(fluid_col, rim_metal, rim_factor);

            let azimuth = atan2(hit_p.z, hit_p.x);
            let angle_norm = fract((azimuth + 3.14159265) / 6.2831853);
            let pole_f = angle_norm * 16.0;
            let pole_idx = u32(pole_f) % 16u;
            let pole_fract = fract(pole_f);
            // Gap between the 16 discrete copper/neon stator poles around the rim
            let seg_mask = smoothstep(0.08, 0.16, pole_fract) * (1.0 - smoothstep(0.84, 0.92, pole_fract));
            let radial_led_band = smoothstep(DISH_RADIUS - 0.14, DISH_RADIUS - 0.09, r_xz) *
                                  (1.0 - smoothstep(DISH_RADIUS - 0.04, DISH_RADIUS - 0.01, r_xz));
            let pole_spec = clamp(audio.spectrum[pole_idx / 4u][pole_idx % 4u] / 55.0, 0.0, 1.5);
            let pole_hue_col = mix(vec3<f32>(1.0, 0.35, 0.05), vec3<f32>(0.08, 0.85, 1.0), f32(pole_idx) / 15.0);
            fluid_col += pole_hue_col * seg_mask * radial_led_band * (0.15 + pole_spec * 1.6);
        }

        // Atmospheric depth fog
        let fog_factor = clamp(t * 0.04, 0.0, 0.65);
        col = mix(fluid_col, col, fog_factor);
    }

    // ACES tonemapping for consistent HDR highlight response
    col = aces_tonemap(col);

    // Vignette
    let d_center = length(uv - vec2<f32>(0.5));
    col *= clamp(1.2 - d_center * 0.85, 0.0, 1.0);

    return vec4<f32>(col, 1.0);
}
