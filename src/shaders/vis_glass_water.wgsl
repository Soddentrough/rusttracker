// INCLUDE: common

// ============================================================================
// Visualizer ID 28: Acoustic Optical Glass & Water Chamber
// Navier-Stokes FLIP Water Simulation in Beveled Optical Crystal Vessel
// Refractive Water (IOR 1.33), Crystal Glass (IOR 1.52), Caustics & Geyser Splashes
// ============================================================================

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var<storage, read> render_grid: array<u32>;

const RENDER_GRID_SIZE: i32 = 512;
const DOMAIN_HALF: f32 = 3.2;
const VESSEL_INNER_R: f32 = 2.65;
const VESSEL_OUTER_R: f32 = 2.95;
const VESSEL_HEIGHT: f32 = 2.10;
const BASIN_FLOOR_Y: f32 = 0.08;

// Raymarch tuning
const MAX_STEPS: i32 = 80;
const HIT_EPS: f32 = 0.0035;
const STEP_SCALE: f32 = 0.45;

fn get_grid_height(gx: i32, gz: i32) -> f32 {
    if (gx < 0 || gx >= RENDER_GRID_SIZE || gz < 0 || gz >= RENDER_GRID_SIZE) {
        return 0.0;
    }
    let cell = u32(gz) * u32(RENDER_GRID_SIZE) + u32(gx);
    return f32(render_grid[cell]) * 0.001; // Scaled by 1000 in compute shader
}

// Continuous water surface heightfield sample with resting meniscus
fn sample_water_height(p: vec2<f32>) -> f32 {
    let r_xz = length(p);
    let r_clamped = min(r_xz, VESSEL_INNER_R - 0.015);
    let p_eval = select(p, normalize(p) * r_clamped, r_xz > r_clamped);

    // Meniscus layer: resting liquid level at ~32cm with curved meniscus clinging to glass wall
    let wall_dist = max(0.0, VESSEL_INNER_R - r_xz);
    let meniscus = smoothstep(0.40, 0.0, wall_dist) * 0.055;
    let base_water = 0.32 + meniscus;

    // World [-3.2, 3.2] -> [0, 512]
    let gx_f = ((p_eval.x + DOMAIN_HALF) / (DOMAIN_HALF * 2.0)) * f32(RENDER_GRID_SIZE);
    let gz_f = ((p_eval.y + DOMAIN_HALF) / (DOMAIN_HALF * 2.0)) * f32(RENDER_GRID_SIZE);

    let ix = i32(floor(gx_f));
    let iz = i32(floor(gz_f));
    let fx = fract(gx_f);
    let fz = fract(gz_f);

    // Bilinear sample from splatted heightfield
    let h00 = get_grid_height(ix, iz);
    let h10 = get_grid_height(ix + 1, iz);
    let h01 = get_grid_height(ix, iz + 1);
    let h11 = get_grid_height(ix + 1, iz + 1);

    let h_fluid = mix(mix(h00, h10, fx), mix(h01, h11, fx), fz);

    // Water surface never dips below resting meniscus
    return max(base_water, h_fluid);
}

fn calc_water_normal(p: vec2<f32>, eps: f32) -> vec3<f32> {
    let r_xz = length(p);
    let max_r = VESSEL_INNER_R - eps * 1.5;
    let p_eval = select(p, normalize(p) * max_r, r_xz > max_r);

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

// Prismatic chromatic dispersion palette for beveled crystal edges
fn prism_palette(t: f32) -> vec3<f32> {
    let a = vec3<f32>(0.5, 0.5, 0.5);
    let b = vec3<f32>(0.5, 0.5, 0.5);
    let c = vec3<f32>(1.0, 1.0, 1.0);
    let d = vec3<f32>(0.0, 0.333, 0.667);
    return a + b * cos(6.2831853 * (c * t + d));
}

// Dynamic underwater caustic texture based on wave curvature
fn calculate_caustics(pos: vec2<f32>, time: f32, bass: f32) -> f32 {
    let p = pos * 4.8;
    let w1 = sin(p.x * 2.4 + p.y * 1.9 + time * 2.8);
    let w2 = cos(p.x * 3.3 - p.y * 2.6 - time * 2.2);
    let w3 = sin((p.x + p.y) * 4.4 + time * 3.4);
    let caustic_pattern = pow(max(0.0, (w1 + w2 + w3 + 1.5) / 4.5), 6.0) * 4.8;
    return caustic_pattern * (1.0 + bass * 1.8);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let aspect = audio.aspect_ratio;
    let screen_p = (uv * 2.0 - 1.0) * vec2<f32>(aspect, -1.0);

    let time = audio.time;
    let sub_bass = clamp(audio.spectrum[0].x * 1.8 + audio.channels[0].x * 0.9, 0.0, 2.5);

    // Smooth studio camera setup: elevated ~25 degrees looking down into the optical vessel
    let cam_orbit = time * 0.07;
    let cam_dist = 6.8;
    let cam_h = 3.6 + sin(time * 0.12) * 0.12;
    let ro = vec3<f32>(cos(cam_orbit) * cam_dist, cam_h, sin(cam_orbit) * cam_dist);
    let cam_target = vec3<f32>(0.0, 0.75, 0.0);

    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = cross(uu, ww);
    let rd = normalize(screen_p.x * uu + screen_p.y * vv + 1.85 * ww);

    // Dark minimalist studio backdrop with subtle radial falloff
    let bg_dist = length(screen_p);
    let bg_gradient = clamp(0.045 - 0.022 * bg_dist, 0.008, 0.065);
    var col = vec3<f32>(bg_gradient * 0.85, bg_gradient * 0.95, bg_gradient * 1.25);

    // Soft studio pedestal floor reflection beneath vessel
    let floor_glow = exp(-length(screen_p - vec2<f32>(0.0, -0.42)) * 1.8) * (0.06 + sub_bass * 0.10);
    col += vec3<f32>(0.04, 0.12, 0.24) * floor_glow;

    // Lights
    let softbox_pos = vec3<f32>(2.5, 6.5, 3.2);
    let kicker_pos = vec3<f32>(-4.5, 4.2, -3.8);

    // --- 1. Intersect Outer Optical Crystal Glass Cylinder & Top Rim ---
    let outer_cyl = intersect_cylinder(ro, rd, VESSEL_OUTER_R);
    var hit_outer_glass = false;
    var t_glass_outer = 1e5;
    var n_glass_outer = vec3<f32>(0.0);
    var is_glass_rim = false;

    // Check top rim: plane y = VESSEL_HEIGHT between [VESSEL_INNER_R, VESSEL_OUTER_R]
    if (rd.y < -0.0001 && ro.y > VESSEL_HEIGHT) {
        let t_top = (VESSEL_HEIGHT - ro.y) / rd.y;
        if (t_top > 0.0) {
            let p_top = ro + rd * t_top;
            let r_top = length(p_top.xz);
            if (r_top >= VESSEL_INNER_R && r_top <= VESSEL_OUTER_R) {
                hit_outer_glass = true;
                t_glass_outer = t_top;
                // Beveled facet normal on rim edges
                let wall_frac = (r_top - VESSEL_INNER_R) / (VESSEL_OUTER_R - VESSEL_INNER_R);
                if (wall_frac > 0.88) {
                    n_glass_outer = normalize(vec3<f32>(p_top.x / r_top, 1.0, p_top.z / r_top));
                } else if (wall_frac < 0.12) {
                    n_glass_outer = normalize(vec3<f32>(-p_top.x / r_top, 1.0, -p_top.z / r_top));
                } else {
                    n_glass_outer = vec3<f32>(0.0, 1.0, 0.0);
                }
                is_glass_rim = true;
            }
        }
    }

    // Check outer cylindrical wall
    if (outer_cyl.x > 0.0) {
        let p_cyl = ro + rd * outer_cyl.x;
        if (p_cyl.y >= 0.0 && p_cyl.y <= VESSEL_HEIGHT && outer_cyl.x < t_glass_outer) {
            hit_outer_glass = true;
            t_glass_outer = outer_cyl.x;
            n_glass_outer = normalize(vec3<f32>(p_cyl.x, 0.0, p_cyl.z));
            is_glass_rim = false;
        }
    }

    // Outer glass specular and prismatic highlights
    var outer_glass_spec = vec3<f32>(0.0);
    var outer_fresnel = 0.0;
    if (hit_outer_glass) {
        let p_enter = ro + rd * t_glass_outer;
        let l_soft = normalize(softbox_pos - p_enter);
        let h_soft = normalize(l_soft - rd);
        let NdotV = max(0.0, dot(n_glass_outer, -rd));
        outer_fresnel = 0.04 + 0.96 * pow(1.0 - NdotV, 5.0);

        let spec_tight = pow(max(0.0, dot(n_glass_outer, h_soft)), 220.0) * 4.8;
        let spec_broad = pow(max(0.0, dot(n_glass_outer, h_soft)), 28.0) * 0.55;
        outer_glass_spec = vec3<f32>(1.05, 1.12, 1.25) * (spec_tight + spec_broad);

        // Prismatic rainbow flares along crystal rim
        if (is_glass_rim || p_enter.y > VESSEL_HEIGHT - 0.08) {
            let rim_phase = fract(dot(n_glass_outer, vec3<f32>(2.2, 1.5, 2.2)) * 0.8 + time * 0.06);
            let rim_glint = pow(max(0.0, dot(n_glass_outer, h_soft)), 48.0) * 4.2;
            outer_glass_spec += prism_palette(rim_phase) * rim_glint * (0.9 + sub_bass * 0.6);
        }
    }

    // --- 2. Raymarch Inside Inner Water Chamber ---
    let inner_cyl = intersect_cylinder(ro, rd, VESSEL_INNER_R);
    var water_hit = false;
    var is_submerged = false;
    var hit_p = vec3<f32>(0.0);
    var t_water = 0.0;
    var t_start = 0.0;

    if (inner_cyl.y > 0.0) {
        // Determine segment of ray traversing the inner chamber volume [t_start, t_end]
        t_start = max(0.0, inner_cyl.x);
        var t_end = inner_cyl.y;

        // Clip to vertical height bounds [BASIN_FLOOR_Y, VESSEL_HEIGHT]
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
                // Ray entered through air above liquid: raymarch to find the top water surface
                var t = t_start;
                for (var i = 0; i < MAX_STEPS; i++) {
                    let p = ro + rd * t;
                    let h = sample_water_height(p.xz);
                    let dist = p.y - h;

                    if (dist < HIT_EPS) {
                        water_hit = true;
                        hit_p = p;
                        t_water = t;
                        break;
                    }

                    if (p.y <= BASIN_FLOOR_Y + 0.005) {
                        break;
                    }

                    t += max(dist * STEP_SCALE, 0.012);
                    if (t >= t_end) { break; }
                }
            } else {
                // Ray entered through the glass wall BELOW the liquid level
                var t_interior = t_end;
                if (rd.y < -0.0001) {
                    let t_floor = (BASIN_FLOOR_Y - ro.y) / rd.y;
                    t_interior = min(t_end, t_floor);
                }
                hit_p = ro + rd * t_interior;
                t_water = t_interior;
                is_submerged = true;
            }
        }
    }

    if (water_hit) {
        let normal = calc_water_normal(hit_p.xz, 0.026);
        let view_dir = -rd;
        let NdotV = max(0.0, dot(normal, view_dir));

        // Dielectric Fresnel reflection (F0 = 0.02 for liquid water)
        let fresnel_water = 0.02 + 0.98 * pow(1.0 - NdotV, 5.0);

        // Primary Overhead Softbox Specular Highlight on Water
        let l1_dir = normalize(softbox_pos - hit_p);
        let h1 = normalize(l1_dir + view_dir);
        let NdotL1 = max(0.0, dot(normal, l1_dir));
        let spec1_tight = pow(max(0.0, dot(normal, h1)), 256.0) * 5.5;
        let spec1_soft = pow(max(0.0, dot(normal, h1)), 32.0) * 0.85;
        let water_specular1 = vec3<f32>(1.1, 1.15, 1.25) * (spec1_tight + spec1_soft) * NdotL1;

        // Secondary Kicker Specular (Cool Cyan Rim Light)
        let l2_dir = normalize(kicker_pos - hit_p);
        let h2 = normalize(l2_dir + view_dir);
        let NdotL2 = max(0.0, dot(normal, l2_dir));
        let spec2 = pow(max(0.0, dot(normal, h2)), 128.0) * 2.8;
        let water_specular2 = vec3<f32>(0.35, 0.85, 1.35) * spec2 * NdotL2;

        // Environment reflection (Studio ceiling & glass rim)
        let ref_dir = reflect(-view_dir, normal);
        let env_reflection = max(0.0, ref_dir.y) * vec3<f32>(0.12, 0.20, 0.32) * 1.6;

        // Refraction & Beer-Lambert Volumetric Depth Absorption
        let water_depth = max(0.05, hit_p.y - BASIN_FLOOR_Y);
        let alpha = vec3<f32>(0.36, 0.07, 0.012);
        let transmission = exp(-alpha * (water_depth * 4.6));

        // Transducer Floor & Caustic Light Projection
        let caustics = calculate_caustics(hit_p.xz, time, sub_bass);
        let transducer_plate = vec3<f32>(0.02, 0.03, 0.05) + vec3<f32>(0.28, 0.72, 1.05) * (caustics * 0.20);

        // Acoustic Center Transducer Luminescence
        let center_dist = length(hit_p.xz);
        let ring_pulse = exp(-pow(center_dist - 0.75, 2.0) * 6.0) * sub_bass * 0.75;
        let bass_core = exp(-center_dist * 2.2) * sub_bass * 0.85;
        let transducer_emission = (ring_pulse + bass_core) * vec3<f32>(0.18, 0.75, 1.15);

        let water_body = (transducer_plate + transducer_emission) * transmission;

        // Airborne Geyser Spray & Sparkling Droplets
        var droplet_sparkle = vec3<f32>(0.0);
        let geyser_height = max(0.0, hit_p.y - 0.38);
        if (geyser_height > 0.05) {
            let sparkle_factor = pow(max(0.0, dot(normal, h1)), 64.0) * smoothstep(0.05, 0.45, geyser_height);
            droplet_sparkle = vec3<f32>(1.25, 1.45, 1.70) * sparkle_factor * (1.3 + sub_bass * 0.9);
        }

        var water_col = water_body;
        water_col += (water_specular1 + water_specular2 + env_reflection) * fresnel_water;
        water_col += droplet_sparkle;

        let glass_transmission = vec3<f32>(0.94, 0.97, 1.0);
        col = water_col * glass_transmission;
    } else if (is_submerged) {
        // View into submerged water through glass
        let water_path = max(0.1, t_water - t_start);
        let alpha = vec3<f32>(0.36, 0.07, 0.012);
        let transmission = exp(-alpha * (water_path * 2.8));

        let caustics = calculate_caustics(hit_p.xz, time, sub_bass);
        let floor_plate = vec3<f32>(0.02, 0.03, 0.05) + vec3<f32>(0.25, 0.65, 0.95) * (caustics * 0.18);

        let center_dist = length(hit_p.xz);
        let ring_pulse = exp(-pow(center_dist - 0.75, 2.0) * 6.0) * sub_bass * 0.75;
        let bass_core = exp(-center_dist * 2.2) * sub_bass * 0.85;
        let transducer_emission = (ring_pulse + bass_core) * vec3<f32>(0.18, 0.75, 1.15);

        let water_submerged = (floor_plate + transducer_emission) * transmission;
        col = water_submerged * vec3<f32>(0.92, 0.96, 1.0);
    } else {
        // Ray did not hit water inside: check studio floor at y = 0
        if (rd.y < -0.0001) {
            let t_ground = -ro.y / rd.y;
            if (t_ground > 0.0) {
                let p_ground = ro + rd * t_ground;
                let r_ground = length(p_ground.xz);
                let shadow = smoothstep(VESSEL_OUTER_R * 0.8, VESSEL_OUTER_R + 1.2, r_ground);
                let caustics_floor = calculate_caustics(p_ground.xz * 0.7, time, sub_bass);
                let floor_glow_caustic = vec3<f32>(0.04, 0.16, 0.28) * (caustics_floor * 0.12 * (1.0 - smoothstep(VESSEL_OUTER_R, VESSEL_OUTER_R + 1.8, r_ground)));
                col = vec3<f32>(0.012, 0.018, 0.028) * (0.25 + 0.75 * shadow) + floor_glow_caustic;
            }
        }
    }

    // Apply Outer Glass Specular, Fresnel, and Rim Prism Highlights
    if (hit_outer_glass) {
        col = mix(col, vec3<f32>(0.9, 0.95, 1.05), outer_fresnel * 0.35);
        col += outer_glass_spec * outer_fresnel;
    }

    // ACES Tonemapping for photorealistic HDR highlight rolloff
    col = aces_tonemap(col);

    // Subtle edge vignette
    let d_center = length(uv - vec2<f32>(0.5));
    col *= clamp(1.22 - d_center * 0.75, 0.0, 1.0);

    return vec4<f32>(col, 1.0);
}
