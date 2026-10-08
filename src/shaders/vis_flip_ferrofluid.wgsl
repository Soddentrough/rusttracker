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
const MAX_STEPS: i32 = 96;
const HIT_EPS: f32 = 0.0035;
const STEP_SCALE: f32 = 0.42;

fn get_grid_height(gx: i32, gz: i32) -> f32 {
    if (gx < 0 || gx >= RENDER_GRID_SIZE || gz < 0 || gz >= RENDER_GRID_SIZE) {
        return 0.0;
    }
    let cell = u32(gz) * u32(RENDER_GRID_SIZE) + u32(gx);
    return f32(render_grid[cell]) * 0.001; // Scaled by 1000 in compute shader
}

// Bilinear heightfield lookup with continuous meniscus puddle
fn sample_height(p: vec2<f32>) -> f32 {
    let r_xz = length(p);
    let floor_y = 0.025 + 0.022 * (r_xz * r_xz);

    if (r_xz > DISH_RADIUS + 0.2) {
        return floor_y;
    }

    // Base meniscus puddle: resting liquid depth that smoothly tapers at the dish rim
    let dish_mask = smoothstep(DISH_RADIUS, DISH_RADIUS - 0.45, r_xz);
    let base_puddle = floor_y + 0.055 * dish_mask;

    // World [-3.6, 3.6] -> [0, 512]
    let gx_f = ((p.x + DOMAIN_HALF) / (DOMAIN_HALF * 2.0)) * f32(RENDER_GRID_SIZE);
    let gz_f = ((p.y + DOMAIN_HALF) / (DOMAIN_HALF * 2.0)) * f32(RENDER_GRID_SIZE);

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

fn calc_normal(p: vec2<f32>, eps: f32) -> vec3<f32> {
    let e = vec2<f32>(eps, 0.0);
    let dx = (sample_height(p + e.xy) - sample_height(p - e.xy)) / (2.0 * eps);
    let dz = (sample_height(p + e.yx) - sample_height(p - e.yx)) / (2.0 * eps);
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

// Palette for subtle thin-film interference on crests
fn iridescence_palette(t: f32) -> vec3<f32> {
    let a = vec3<f32>(0.5, 0.5, 0.5);
    let b = vec3<f32>(0.5, 0.5, 0.5);
    let c = vec3<f32>(1.0, 1.0, 1.0);
    let d = vec3<f32>(0.0, 0.333, 0.667);
    return a + b * cos(6.2831853 * (c * t + d));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let aspect = audio.aspect_ratio;
    let screen_p = (uv * 2.0 - 1.0) * vec2<f32>(aspect, -1.0);

    // Subtle audio-driven camera orbit
    let time = audio.time;
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

    // Under-dish glow reflection on floor
    let sub_bass = clamp(audio.spectrum[0].x * 1.6, 0.0, 2.0);
    let floor_glow = exp(-length(screen_p - vec2<f32>(0.0, -0.4)) * 1.8) * (0.04 + sub_bass * 0.08);
    col += vec3<f32>(0.1, 0.06, 0.18) * floor_glow;

    // Ray-AABB intersection test for early-out
    let box_min = vec3<f32>(-3.55, 0.0, -3.55);
    let box_max = vec3<f32>(3.55, 2.5, 3.55);
    let box_hit = intersect_aabb(ro, rd, box_min, box_max);

    if (box_hit.x > box_hit.y || box_hit.y < 0.0) {
        // Ray missed fluid domain completely — return background immediately
        return vec4<f32>(aces_tonemap(col), 1.0);
    }

    let t_start = max(0.0, box_hit.x);
    let t_end = box_hit.y;
    var t = t_start;
    var hit = false;
    var hit_p = vec3<f32>(0.0);

    // March heightfield
    for (var i = 0; i < MAX_STEPS; i++) {
        let p = ro + rd * t;
        let r = length(p.xz);

        // Outside dish bounds
        if (r > DISH_RADIUS + 0.15) {
            t += 0.06;
            if (t >= t_end) { break; }
            continue;
        }

        let h = sample_height(p.xz);
        let dist = p.y - h;

        if (dist < HIT_EPS) {
            hit = true;
            hit_p = p;
            break;
        }

        // Sky early-out: ray is pointing up and is already above max possible fluid height
        if (rd.y > 0.0 && p.y > 2.4) {
            break;
        }

        t += max(dist * STEP_SCALE, 0.012);
        if (t >= t_end) { break; }
    }

    if (hit) {
        let normal = calc_normal(hit_p.xz, 0.025);
        let view_dir = -rd;
        let NdotV = max(0.0, dot(normal, view_dir));
        let r_xz = length(hit_p.xz);
        let floor_y = 0.025 + 0.022 * (r_xz * r_xz);
        let dish_mask = smoothstep(DISH_RADIUS, DISH_RADIUS - 0.45, r_xz);
        let base_puddle = floor_y + 0.055 * dish_mask;
        let spike_h = max(0.0, hit_p.y - base_puddle);

        // --- Obsidian Chrome Material Lighting ---
        // Obsidian base color: pitch black with faint graphite undertone
        let albedo = vec3<f32>(0.012, 0.014, 0.018);

        // Fresnel reflection (Schlick approximation)
        let F0 = 0.62; // High metallic chrome reflectivity
        let fresnel = F0 + (1.0 - F0) * pow(1.0 - NdotV, 5.0);

        // Primary Overhead Softbox Area Light (Warm Studio Amber)
        let light1_pos = vec3<f32>(2.2, 5.2, 2.5);
        let l1_dir = normalize(light1_pos - hit_p);
        let h1 = normalize(l1_dir + view_dir);
        let NdotL1 = max(0.0, dot(normal, l1_dir));
        let spec1_tight = pow(max(0.0, dot(normal, h1)), 220.0) * 5.2;
        let spec1_soft = pow(max(0.0, dot(normal, h1)), 28.0) * 0.85;
        let light1_col = vec3<f32>(1.12, 0.98, 0.82) * (spec1_tight + spec1_soft) * NdotL1;

        // Secondary Kicker Light (Cool Blue-Cyan Rim Light)
        let light2_pos = vec3<f32>(-2.8, 4.0, -2.8);
        let l2_dir = normalize(light2_pos - hit_p);
        let h2 = normalize(l2_dir + view_dir);
        let NdotL2 = max(0.0, dot(normal, l2_dir));
        let spec2 = pow(max(0.0, dot(normal, h2)), 110.0) * 2.6;
        let light2_col = vec3<f32>(0.45, 0.85, 1.25) * spec2 * NdotL2;

        // Electromagnetic Under-Basin Coil Radiance
        // Concentric neon filament coils visible in the glass dish rim and around erupting spike roots
        let coil_freq = pow(max(0.0, sin(r_xz * 28.0 - time * 1.5)), 6.0);
        let coil_col = mix(vec3<f32>(1.0, 0.40, 0.06), vec3<f32>(0.10, 0.75, 1.0), sin(r_xz * 6.0) * 0.5 + 0.5);
        
        // Dish glass rim glow: visible where fluid thins out towards container edge
        let dish_glass_glow = smoothstep(DISH_RADIUS - 0.45, DISH_RADIUS, r_xz) * coil_freq * (0.8 + sub_bass * 0.8);
        // Spike root flux: electromagnetic energy exciting fluid at the base of erupting spikes
        let spike_root_flux = smoothstep(0.01, 0.12, spike_h) * smoothstep(0.45, 0.10, spike_h) * (0.35 + sub_bass * 0.5);
        let underglow = coil_col * (dish_glass_glow + spike_root_flux) * (1.0 - NdotV * 0.4);

        // Iridescent Thin-Film Sheen on Sharp Magnetic Spike Tips
        let spike_crest = smoothstep(0.35, 1.1, spike_h);
        let irid_t = fract(dot(normal, vec3<f32>(1.0, 2.0, 1.0)) * 0.65 + time * 0.1);
        let irid_col = iridescence_palette(irid_t) * spike_crest * (0.35 + sub_bass * 0.45);

        // Environment reflection
        let ref_dir = reflect(-view_dir, normal);
        let env_sky = max(0.0, ref_dir.y) * vec3<f32>(0.16, 0.20, 0.28);
        let env_floor = max(0.0, -ref_dir.y) * vec3<f32>(0.04, 0.02, 0.06);

        // Compose fluid color
        var fluid_col = albedo + underglow;
        fluid_col += (light1_col + light2_col) * fresnel;
        fluid_col += (env_sky + env_floor) * fresnel;
        fluid_col += irid_col * pow(1.0 - NdotV, 3.0);

        // Dish rim & lip shading
        if (r_xz > DISH_RADIUS - 0.08) {
            let rim_factor = smoothstep(DISH_RADIUS - 0.08, DISH_RADIUS, r_xz);
            let rim_metal = vec3<f32>(0.18, 0.19, 0.22) * (0.35 + 0.65 * NdotL1);
            fluid_col = mix(fluid_col, rim_metal, rim_factor);
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
