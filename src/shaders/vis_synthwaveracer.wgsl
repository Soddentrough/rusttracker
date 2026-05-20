// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

fn hash1(n: f32) -> f32 {
    return fract(sin(n) * 43758.5453);
}

// Integer-based hash for procedural deterministic heights
fn ihash2(ix: i32, iy: i32) -> f32 {
    var n = u32(ix) * 1597334677u ^ u32(iy) * 3812015801u;
    n = n ^ (n >> 16u);
    n = n * 2654435769u;
    n = n ^ (n >> 16u);
    return f32(n & 0x00FFFFFFu) / f32(0x01000000);
}

// Road curve function: determines the centerline X coordinate at a given Z distance
// Uses fixed gentle sinusoidal curves based purely on Z position — no audio dependency
fn road_x(z: f32) -> f32 {
    return 12.0 * sin(z * 0.004) + 5.0 * cos(z * 0.009);
}

// Analytical ray-box intersection (Slab method)
fn intersect_box(ro: vec3<f32>, rd: vec3<f32>, box_min: vec3<f32>, box_max: vec3<f32>) -> f32 {
    var safe_rd = rd;
    if (abs(safe_rd.x) < 1e-6) {
        if (safe_rd.x >= 0.0) { safe_rd.x = 1e-6; } else { safe_rd.x = -1e-6; }
    }
    if (abs(safe_rd.y) < 1e-6) {
        if (safe_rd.y >= 0.0) { safe_rd.y = 1e-6; } else { safe_rd.y = -1e-6; }
    }
    if (abs(safe_rd.z) < 1e-6) {
        if (safe_rd.z >= 0.0) { safe_rd.z = 1e-6; } else { safe_rd.z = -1e-6; }
    }
    let inv_d = 1.0 / safe_rd;
    let t0 = (box_min - ro) * inv_d;
    let t1 = (box_max - ro) * inv_d;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    let t_near = max(max(tmin.x, tmin.y), tmin.z);
    let t_far = min(min(tmax.x, tmax.y), tmax.z);
    if (t_near > t_far || t_far < 0.0) {
        return -1.0;
    }
    return t_near;
}

// Analytical normal for axis-aligned box
fn get_box_normal(p: vec3<f32>, b_min: vec3<f32>, b_max: vec3<f32>) -> vec3<f32> {
    let epsilon = 0.01;
    if (abs(p.x - b_min.x) < epsilon) { return vec3<f32>(-1.0, 0.0, 0.0); }
    if (abs(p.x - b_max.x) < epsilon) { return vec3<f32>(1.0, 0.0, 0.0); }
    if (abs(p.y - b_min.y) < epsilon) { return vec3<f32>(0.0, -1.0, 0.0); }
    if (abs(p.y - b_max.y) < epsilon) { return vec3<f32>(0.0, 1.0, 0.0); }
    if (abs(p.z - b_min.z) < epsilon) { return vec3<f32>(0.0, 0.0, -1.0); }
    if (abs(p.z - b_max.z) < epsilon) { return vec3<f32>(0.0, 0.0, 1.0); }
    return vec3<f32>(0.0, 1.0, 0.0);
}
// Simple box SDF helper
fn sdBox(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}


// Distance estimator (SDF) for the retro sports car — Countach/Testarossa inspired
fn sd_car(p: vec3<f32>, car_pos: vec3<f32>) -> f32 {
    let p_rel = p - car_pos;
    
    // Rotate car to align with road curve tangent
    let z_car = car_pos.z;
    let tangent = normalize(vec3<f32>(road_x(z_car + 1.0) - road_x(z_car - 1.0), 0.0, 2.0));
    let angle = atan2(tangent.x, tangent.z);
    
    let c = cos(angle);
    let s = sin(angle);
    let px = p_rel.x * c + p_rel.z * s;
    let py = p_rel.y;
    let pz = -p_rel.x * s + p_rel.z * c;
    let p_car = vec3<f32>(px, py, pz);
    
    // 1. Main body — wide, low wedge
    var d = sdBox(p_car - vec3<f32>(0.0, -0.02, 0.0), vec3<f32>(1.15, 0.16, 2.0));
    
    // 2. Front nose — tapered wedge sloping downward
    let p_nose = p_car - vec3<f32>(0.0, -0.10, 1.4);
    // Tilt forward ~20 degrees
    let p_nose_rot = vec3<f32>(p_nose.x, p_nose.y * 0.94 - p_nose.z * 0.34, p_nose.y * 0.34 + p_nose.z * 0.94);
    let d_nose = sdBox(p_nose_rot, vec3<f32>(1.10, 0.10, 0.7));
    d = min(d, d_nose);
    
    // 3. Rear deck — slight upward slope for fastback profile
    let p_rear = p_car - vec3<f32>(0.0, -0.02, -1.4);
    let p_rear_rot = vec3<f32>(p_rear.x, p_rear.y * 0.97 + p_rear.z * 0.24, -p_rear.y * 0.24 + p_rear.z * 0.97);
    let d_rear = sdBox(p_rear_rot, vec3<f32>(1.14, 0.14, 0.65));
    d = min(d, d_rear);
    
    // 4. Fender flares — wider haunches over rear wheels
    let d_fender_l = sdBox(p_car - vec3<f32>(-1.05, -0.06, -0.9), vec3<f32>(0.22, 0.12, 0.55));
    let d_fender_r = sdBox(p_car - vec3<f32>(1.05, -0.06, -0.9), vec3<f32>(0.22, 0.12, 0.55));
    d = min(d, min(d_fender_l, d_fender_r));
    
    // 5. Cabin — raked windshield flowing into fastback
    // Windshield (steeply raked ~30 degrees)
    let p_ws = p_car - vec3<f32>(0.0, 0.20, 0.35);
    let p_ws_rot = vec3<f32>(p_ws.x, p_ws.y * 0.87 - p_ws.z * 0.50, p_ws.y * 0.50 + p_ws.z * 0.87);
    let d_ws = sdBox(p_ws_rot, vec3<f32>(0.78, 0.10, 0.50));
    
    // Roof center section
    let d_roof = sdBox(p_car - vec3<f32>(0.0, 0.28, -0.10), vec3<f32>(0.74, 0.08, 0.50));
    
    // Rear window (angled back — Countach louver style)
    let p_rw = p_car - vec3<f32>(0.0, 0.18, -0.55);
    let p_rw_rot = vec3<f32>(p_rw.x, p_rw.y * 0.87 + p_rw.z * 0.50, -p_rw.y * 0.50 + p_rw.z * 0.87);
    let d_rw = sdBox(p_rw_rot, vec3<f32>(0.76, 0.10, 0.45));
    
    let d_cabin = min(d_ws, min(d_roof, d_rw));
    d = min(d, d_cabin);
    
    // 6. Side intake scoops (Testarossa style strakes)
    let d_intake_l = sdBox(p_car - vec3<f32>(-1.17, 0.02, -0.3), vec3<f32>(0.06, 0.08, 0.45));
    let d_intake_r = sdBox(p_car - vec3<f32>(1.17, 0.02, -0.3), vec3<f32>(0.06, 0.08, 0.45));
    d = min(d, min(d_intake_l, d_intake_r));
    
    // 7. Wheels — cylindrical proportions
    let d_tire_fl = sdBox(p_car - vec3<f32>(-1.05, -0.18, 1.15), vec3<f32>(0.14, 0.18, 0.30));
    let d_tire_fr = sdBox(p_car - vec3<f32>(1.05, -0.18, 1.15), vec3<f32>(0.14, 0.18, 0.30));
    let d_tire_rl = sdBox(p_car - vec3<f32>(-1.12, -0.18, -1.05), vec3<f32>(0.18, 0.20, 0.35));
    let d_tire_rr = sdBox(p_car - vec3<f32>(1.12, -0.18, -1.05), vec3<f32>(0.18, 0.20, 0.35));
    d = min(d, min(d_tire_fl, min(d_tire_fr, min(d_tire_rl, d_tire_rr))));
    
    // 8. Side mirrors
    let d_mirror_l = sdBox(p_car - vec3<f32>(-1.0, 0.18, 0.50), vec3<f32>(0.12, 0.04, 0.06));
    let d_mirror_r = sdBox(p_car - vec3<f32>(1.0, 0.18, 0.50), vec3<f32>(0.12, 0.04, 0.06));
    d = min(d, min(d_mirror_l, d_mirror_r));
    
    // 9. Rear spoiler — wide wing on twin pylons
    let d_sp_wing = sdBox(p_car - vec3<f32>(0.0, 0.42, -1.82), vec3<f32>(1.20, 0.015, 0.12));
    let d_sp_sup_l = sdBox(p_car - vec3<f32>(-0.85, 0.28, -1.80), vec3<f32>(0.03, 0.15, 0.04));
    let d_sp_sup_r = sdBox(p_car - vec3<f32>(0.85, 0.28, -1.80), vec3<f32>(0.03, 0.15, 0.04));
    d = min(d, min(d_sp_wing, min(d_sp_sup_l, d_sp_sup_r)));
    
    return d;
}

// Distance estimator normal for the car
fn get_car_normal(p: vec3<f32>, car_pos: vec3<f32>) -> vec3<f32> {
    let eps = 0.002;
    let d = sd_car(p, car_pos);
    let n = vec3<f32>(
        sd_car(p + vec3<f32>(eps, 0.0, 0.0), car_pos) - d,
        sd_car(p + vec3<f32>(0.0, eps, 0.0), car_pos) - d,
        sd_car(p + vec3<f32>(0.0, 0.0, eps), car_pos) - d
    );
    return normalize(n);
}

// Sky sampling function containing sun, stars, and sunset colors
fn sample_sky(rd: vec3<f32>) -> vec3<f32> {
    let horizon_y = 0.02;
    let sky_t = clamp((rd.y - horizon_y) * 1.5 + 0.3, 0.0, 1.0);
    
    // Sleek vaporwave color palette
    var sky_color = mix(
        vec3<f32>(0.03, 0.0, 0.08), // Dark purple/blue at zenith
        mix(vec3<f32>(0.7, 0.0, 0.45), vec3<f32>(0.95, 0.35, 0.02), sky_t), // Magenta to sunset gold
        sky_t
    );
    
    // Giant striped setting sun
    let sun_dir = normalize(vec3<f32>(0.0, 0.02, 1.0));
    let sun_dot = dot(rd, sun_dir);
    if (sun_dot > 0.0) {
        let sun_dist = acos(sun_dot);
        let sun_radius = 0.16;
        
        let bass_pulse = clamp(audio.spectrum[2].x * 1.2, 0.0, 1.0);
        let mid_energy = clamp(audio.spectrum[20].x * 0.8, 0.0, 1.0);
        
        // Sun halo glow
        sky_color += vec3<f32>(0.95, 0.05, 0.35) * exp(-sun_dist * 12.0) * (0.3 + mid_energy * 0.7);
        
        // Sun disk
        if (sun_dist < sun_radius && rd.y > horizon_y - 0.01) {
            let sun_y = rd.y - horizon_y;
            // Synthwave horizontal slice effect (cuts across the entire height of the sun disk)
            let cut = fract(sun_y * 15.0 - audio.smooth_time * 0.95);
            let cut_threshold = mix(0.42, 0.03, clamp(sun_y / sun_radius, 0.0, 1.0));
            if (cut > cut_threshold) {
                let sun_t = clamp(sun_y / sun_radius, 0.0, 1.0);
                let sun_col = mix(vec3<f32>(0.98, 0.08, 0.38), vec3<f32>(0.98, 0.88, 0.05), sun_t);
                sky_color = mix(sky_color, sun_col, smoothstep(sun_radius, sun_radius - 0.01, sun_dist));
            }
        }
    }
    
    // Twinkling stars in the sky (projected onto a sky cylinder down to the horizon)
    if (rd.y > 0.03) {
        let sky_uv = vec2<f32>(atan2(rd.x, rd.z), rd.y / (length(rd.xz) + 0.01));
        let star_uv = sky_uv * 48.0;
        let star_id = floor(star_uv);
        let star_rnd = hash1(star_id.x * 41.7 + star_id.y * 97.3);
        if (star_rnd > 0.94) {
            let star_center = star_id + 0.5 + 0.3 * vec2<f32>(hash1(star_rnd * 13.3), hash1(star_rnd * 37.7)) - 0.15;
            let dist = length(star_uv - star_center);
            let twinkle = 0.4 + 0.6 * sin(audio.smooth_time * 3.0 + star_rnd * 6.28);
            let intensity = smoothstep(0.18, 0.0, dist) * twinkle * smoothstep(0.03, 0.12, rd.y);
            let treble_pulse = clamp(audio.spectrum[75].x * 1.5, 0.0, 1.0);
            sky_color += vec3<f32>(1.0, 0.95, 1.0) * intensity * (0.6 + treble_pulse * 1.0);
        }
    }
    
    return sky_color;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // 1. Setup camera and perspective projection
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);
    let safe_aspect = select(1.0, aspect, aspect > 0.0001 || aspect < -0.0001);
    let p = vec2<f32>((in.uv.x * 2.0 - 1.0) * safe_aspect, -(in.uv.y * 2.0 - 1.0));
    
    let speed = 72.0;
    let cam_z = audio.smooth_time * speed;
    let cam_x = road_x(cam_z);
    let cam_y = 0.95;
    let ro = vec3<f32>(cam_x, cam_y, cam_z);
    
    let look_ahead = 15.0;
    let target_z = cam_z + look_ahead;
    let target_x = road_x(target_z);
    let target_y = 0.8;
    let lookAt = vec3<f32>(target_x, target_y, target_z);
    
    let F = normalize(lookAt - ro);
    let R = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), F));
    let U = cross(F, R);
    let rd = normalize(F + p.x * R * 1.1 + p.y * U);
    
    // 2. Traversal and intersection setup
    var t_closest = 1e9;
    var hit_material = 0; // 0 = none, 1 = left inner building, 2 = right inner building, 3 = left outer building, 4 = right outer building, 5 = left guardrail, 6 = right guardrail, 7 = sports car
    var hit_block_id = 0;
    var hit_box_min = vec3<f32>(0.0);
    var hit_box_max = vec3<f32>(0.0);
    
    let L = 38.0; // Segment block length
    let start_block = i32(floor(cam_z / L)) - 1;
    let end_block = start_block + 11;
    
    // Traversal loop over visible building blocks
    for (var i = start_block; i <= end_block; i++) {
        let z_center = f32(i) * L;
        let rx = road_x(z_center);
        
        // Left inner buildings
        {
            let h = 8.0 + ihash2(i, 0) * 18.0;
            let w = 7.0;
            let d = 15.0;
            let b_min = vec3<f32>(rx - 15.5 - w/2.0, -1.0, z_center - d/2.0);
            let b_max = vec3<f32>(rx - 15.5 + w/2.0, h, z_center + d/2.0);
            let t = intersect_box(ro, rd, b_min, b_max);
            if (t > 0.0 && t < t_closest) {
                t_closest = t;
                hit_material = 1;
                hit_block_id = i;
                hit_box_min = b_min;
                hit_box_max = b_max;
            }
        }
        // Right inner buildings
        {
            let h = 8.0 + ihash2(i, 1) * 18.0;
            let w = 7.0;
            let d = 15.0;
            let b_min = vec3<f32>(rx + 15.5 - w/2.0, -1.0, z_center - d/2.0);
            let b_max = vec3<f32>(rx + 15.5 + w/2.0, h, z_center + d/2.0);
            let t = intersect_box(ro, rd, b_min, b_max);
            if (t > 0.0 && t < t_closest) {
                t_closest = t;
                hit_material = 2;
                hit_block_id = i;
                hit_box_min = b_min;
                hit_box_max = b_max;
            }
        }
        // Left outer buildings (massive skyscrapers)
        {
            let h = 28.0 + ihash2(i, 2) * 52.0;
            let w = 16.0;
            let d = 26.0;
            let b_min = vec3<f32>(rx - 32.5 - w/2.0, -1.0, z_center - d/2.0);
            let b_max = vec3<f32>(rx - 32.5 + w/2.0, h, z_center + d/2.0);
            let t = intersect_box(ro, rd, b_min, b_max);
            if (t > 0.0 && t < t_closest) {
                t_closest = t;
                hit_material = 3;
                hit_block_id = i;
                hit_box_min = b_min;
                hit_box_max = b_max;
            }
        }
        // Right outer buildings (massive skyscrapers)
        {
            let h = 28.0 + ihash2(i, 3) * 52.0;
            let w = 16.0;
            let d = 26.0;
            let b_min = vec3<f32>(rx + 32.5 - w/2.0, -1.0, z_center - d/2.0);
            let b_max = vec3<f32>(rx + 32.5 + w/2.0, h, z_center + d/2.0);
            let t = intersect_box(ro, rd, b_min, b_max);
            if (t > 0.0 && t < t_closest) {
                t_closest = t;
                hit_material = 4;
                hit_block_id = i;
                hit_box_min = b_min;
                hit_box_max = b_max;
            }
        }
        // Left guardrail (low height 0.75, doesn't block buildings)
        {
            let b_min = vec3<f32>(rx - 8.65, 0.0, z_center - L/2.0);
            let b_max = vec3<f32>(rx - 8.35, 0.75, z_center + L/2.0);
            let t = intersect_box(ro, rd, b_min, b_max);
            if (t > 0.0 && t < t_closest) {
                t_closest = t;
                hit_material = 5;
                hit_block_id = i;
                hit_box_min = b_min;
                hit_box_max = b_max;
            }
        }
        // Right guardrail (low height 0.75, doesn't block buildings)
        {
            let b_min = vec3<f32>(rx + 8.35, 0.0, z_center - L/2.0);
            let b_max = vec3<f32>(rx + 8.65, 0.75, z_center + L/2.0);
            let t = intersect_box(ro, rd, b_min, b_max);
            if (t > 0.0 && t < t_closest) {
                t_closest = t;
                hit_material = 6;
                hit_block_id = i;
                hit_box_min = b_min;
                hit_box_max = b_max;
            }
        }
        // Streetlight lampposts (narrow poles every second block)
        if (i % 2 == 0) {
            // Left lamppost
            let l_min = vec3<f32>(rx - 8.6, 0.0, z_center - 0.2);
            let l_max = vec3<f32>(rx - 8.4, 6.5, z_center + 0.2);
            let t_left = intersect_box(ro, rd, l_min, l_max);
            if (t_left > 0.0 && t_left < t_closest) {
                t_closest = t_left;
                hit_material = 10;
                hit_block_id = i;
                hit_box_min = l_min;
                hit_box_max = l_max;
            }
            
            // Right lamppost
            let r_min = vec3<f32>(rx + 8.4, 0.0, z_center - 0.2);
            let r_max = vec3<f32>(rx + 8.6, 6.5, z_center + 0.2);
            let t_right = intersect_box(ro, rd, r_min, r_max);
            if (t_right > 0.0 && t_right < t_closest) {
                t_closest = t_right;
                hit_material = 15;
                hit_block_id = i;
                hit_box_min = r_min;
                hit_box_max = r_max;
            }
        }
    }
    
    // 3. Sports car intersection (within its bounding box - brought closer)
    let car_lead = 6.8;
    let z_car = cam_z + car_lead;
    // Car follows road center smoothly — no audio-reactive swerving
    let car_pos = vec3<f32>(road_x(z_car), 0.35, z_car);
    let car_box_min = car_pos - vec3<f32>(1.8, 0.6, 2.4);
    let car_box_max = car_pos + vec3<f32>(1.8, 0.8, 2.4);
    let t_car_box = intersect_box(ro, rd, car_box_min, car_box_max);
    
    var hit_car_t = -1.0;
    if (t_car_box > 0.0 && t_car_box < t_closest) {
        let t_entry = max(0.0, t_car_box);
        let t_exit = t_entry + 4.8;
        var t_march = t_entry;
        for (var step = 0; step < 10; step++) {
            let p_march = ro + t_march * rd;
            let d_car = sd_car(p_march, car_pos);
            if (d_car < 0.005) {
                hit_car_t = t_march;
                break;
            }
            t_march += d_car;
            if (t_march > t_closest || t_march > t_exit) {
                break;
            }
        }
        if (hit_car_t > 0.0 && hit_car_t < t_closest) {
            t_closest = hit_car_t;
            hit_material = 7;
        }
    }
    
    // 4. Ground plane intersection
    let t_ground = -ro.y / rd.y;
    var hit_ground = false;
    if (t_ground > 0.0 && t_ground < t_closest) {
        t_closest = t_ground;
        hit_material = 8;
        hit_ground = true;
    }
    
    // 5. Shader coloring and materials (audio reactivity is limited to building window lights only)
    
    var final_color = vec3<f32>(0.0);
    
    if (t_closest > 1e8) {
        // Render sky background
        final_color = sample_sky(rd);
    } else {
        let hit_pos = ro + t_closest * rd;
        var material_color = vec3<f32>(0.0);
        
        switch (hit_material) {
            // Skyscraper lighting and patterns
            case 1, 2, 3, 4: {
                let rx = road_x(f32(hit_block_id) * L);
                let norm = get_box_normal(hit_pos, hit_box_min, hit_box_max);
                // Sky reflecting on glass facades
                let reflection = sample_sky(reflect(rd, norm)) * 0.15;
                
                // Base skyscraper gradient
                let height_ratio = hit_pos.y / hit_box_max.y;
                let building_base = vec3<f32>(0.02, 0.005, 0.06);
                let building_top = vec3<f32>(0.1, 0.02, 0.2);
                var material_color_temp = mix(building_base, building_top, height_ratio) + reflection;
                
                // Lateral neon accent highlights on building walls
                var lateral_light = vec3<f32>(0.0);
                if (norm.x > 0.0) {
                    lateral_light = vec3<f32>(0.0, 0.65, 0.85) * 0.6; // Cyan on right-facing facades
                } else if (norm.x < 0.0) {
                    lateral_light = vec3<f32>(0.85, 0.0, 0.55) * 0.6;  // Magenta on left-facing facades
                }
                material_color_temp += lateral_light * 0.4;
                
                // Procedural window grids on the vertical sides
                if (abs(norm.y) < 0.5) {
                    let u_coord = select(hit_pos.z, hit_pos.x, abs(norm.z) > 0.5);
                    let window_uv = vec2<f32>(u_coord * 0.7, hit_pos.y * 0.45);
                    let cell = floor(window_uv);
                    let gv = fract(window_uv);
                    
                    let win_hash = ihash2(i32(cell.x) + hit_block_id * 17, i32(cell.y));
                    
                    if (gv.x > 0.22 && gv.x < 0.78 && gv.y > 0.22 && gv.y < 0.78) {
                        if (win_hash > 0.48) {
                            var win_color = vec3<f32>(0.98, 0.82, 0.08); // Golden-yellow
                            if (win_hash > 0.80) {
                                win_color = vec3<f32>(0.98, 0.08, 0.48); // Neon pink/magenta
                            } else if (win_hash > 0.62) {
                                win_color = vec3<f32>(0.0, 0.88, 0.98); // Neon cyan
                            }
                            let reactivity = mix(audio.spectrum[10].x, audio.spectrum[45].x, step(0.7, win_hash));
                            win_color *= (1.0 + reactivity * 2.8);
                            material_color_temp = mix(material_color_temp, win_color, 0.90);
                        }
                    }
                    
                    // Horizontal structure lines
                    if (hit_material >= 3 && abs(gv.y - 0.5) < 0.06) {
                        let stripe_pulse = mix(vec3<f32>(0.98, 0.08, 0.48), vec3<f32>(0.0, 0.88, 0.98), step(0.5, win_hash));
                        material_color_temp += stripe_pulse * 0.55;
                    }
                }
                
                material_color = material_color_temp;
            }
            // 3D Guardrails and posts
            case 5, 6, 9, 12, 13, 14: {
                let norm = get_box_normal(hit_pos, hit_box_min, hit_box_max);
                material_color = vec3<f32>(0.05, 0.05, 0.08); // Dark metal base
                
                if (hit_material == 5 || hit_material == 12) {
                    // Top rail: Glowing pink/red stripe
                    material_color = vec3<f32>(0.98, 0.08, 0.48) * 2.0;
                } else if (hit_material == 6 || hit_material == 13) {
                    // Bottom rail: Glowing cyan stripe
                    material_color = vec3<f32>(0.0, 0.85, 0.98) * 1.8;
                } else {
                    // Posts
                    material_color = vec3<f32>(0.12, 0.12, 0.16);
                }
            }
            // Lamppost poles
            case 10, 15: {
                material_color = vec3<f32>(0.15, 0.15, 0.2);
            }
            // Lamppost heads (Streetlights)
            case 11, 16: {
                material_color = vec3<f32>(0.98, 0.88, 0.18) * 3.0;
            }
            // Sports car wedge styling & taillights
            case 7: {
                let norm = get_car_normal(hit_pos, car_pos);
                let R_refl = reflect(rd, norm);
                let refl_sky = sample_sky(R_refl);
                
                // Deep metallic purple paint with environment reflection
                let base_paint = vec3<f32>(0.06, 0.01, 0.14);
                let fresnel = pow(1.0 - max(0.0, dot(-rd, norm)), 3.0);
                material_color = base_paint + refl_sky * (0.25 + fresnel * 0.5);
                
                let p_rel = hit_pos - car_pos;
                let tangent = normalize(vec3<f32>(road_x(car_pos.z + 1.0) - road_x(car_pos.z - 1.0), 0.0, 2.0));
                let angle = atan2(tangent.x, tangent.z);
                let c = cos(angle);
                let s = sin(angle);
                let p_car = vec3<f32>(p_rel.x * c + p_rel.z * s, p_rel.y, -p_rel.x * s + p_rel.z * c);
                
                // 1. Windows/Windshield (dark tinted reflective glass)
                if (p_car.y > 0.14 && p_car.y < 0.36 && abs(p_car.x) < 0.80 && p_car.z > -0.85 && p_car.z < 0.7) {
                    material_color = vec3<f32>(0.01, 0.01, 0.02) + refl_sky * 0.65;
                }
                
                // 2. Side intake scoops (Testarossa strakes — subtle horizontal lines)
                if (abs(p_car.x) > 1.10 && p_car.y > -0.08 && p_car.y < 0.12 && p_car.z > -0.75 && p_car.z < 0.15) {
                    let strake_line = step(0.5, fract(p_car.z * 8.0));
                    material_color = mix(vec3<f32>(0.02, 0.02, 0.02), vec3<f32>(0.04, 0.04, 0.06), strake_line);
                }
                
                // 3. Wheels/Tires & Rims (updated positions)
                let fl_dist = length(p_car - vec3<f32>(-1.05, -0.18, 1.15));
                let fr_dist = length(p_car - vec3<f32>(1.05, -0.18, 1.15));
                let rl_dist = length(p_car - vec3<f32>(-1.12, -0.18, -1.05));
                let rr_dist = length(p_car - vec3<f32>(1.12, -0.18, -1.05));
                if (min(fl_dist, min(fr_dist, min(rl_dist, rr_dist))) < 0.38) {
                    material_color = vec3<f32>(0.01, 0.01, 0.01); // Matte black tires
                    // Neon cyan rims on the outer face
                    if (abs(p_car.x) > 1.10) {
                        material_color = vec3<f32>(0.0, 0.75, 0.88) * 1.2;
                    }
                }
                
                // 4. Taillights — full-width light bar with twin glowing sections
                if (p_car.z < -1.75) {
                    let is_left_light = p_car.x < -0.25 && p_car.x > -1.12 && abs(p_car.y - 0.04) < 0.05;
                    let is_right_light = p_car.x > 0.25 && p_car.x < 1.12 && abs(p_car.y - 0.04) < 0.05;
                    if (is_left_light || is_right_light) {
                        material_color = vec3<f32>(0.99, 0.08, 0.02) * 5.0;
                    } else if (abs(p_car.x) <= 1.2 && p_car.y > -0.12 && p_car.y < 0.20) {
                        material_color = vec3<f32>(0.01, 0.01, 0.01); // Dark bumper
                    }
                }
                
                // 5. Headlights — pop-up style glow at the front
                if (p_car.z > 1.85 && p_car.y > -0.08 && p_car.y < 0.10) {
                    if ((p_car.x > 0.4 && p_car.x < 0.95) || (p_car.x < -0.4 && p_car.x > -0.95)) {
                        material_color = vec3<f32>(0.98, 0.95, 0.80) * 3.0;
                    }
                }
            }
            // Asphalt road, dividers, foliage, and streetlight illumination cones
            case 8: {
                let road_half = 8.5;
                let dist_road = abs(hit_pos.x - road_x(hit_pos.z));
                
                if (dist_road < road_half) {
                    let asphalt = vec3<f32>(0.005, 0.004, 0.015);
                    
                    // Transverse cyan grid lines
                    let trans_grid = smoothstep(0.06, 0.0, abs(fract(hit_pos.z * 0.08 - audio.smooth_time * 2.6) - 0.5));
                    
                    // Longitudinal road lines (magenta centerline & parallel dashed cyan lanes)
                    let center_line = smoothstep(0.08, 0.0, dist_road);
                    
                    let lane1 = smoothstep(0.05, 0.0, abs(dist_road - 2.8));
                    let lane2 = smoothstep(0.05, 0.0, abs(dist_road - 5.6));
                    let dash = step(0.35, fract(hit_pos.z * 0.1 - audio.smooth_time * 2.6));
                    let lane_lines = max(lane1, lane2) * dash;
                    
                    let cyan_c = vec3<f32>(0.0, 0.85, 0.98) * 1.0;
                    let magenta_c = vec3<f32>(0.95, 0.02, 0.5) * 1.2;
                    
                    material_color = mix(asphalt, cyan_c, max(trans_grid * 0.45, lane_lines));
                    material_color = mix(material_color, magenta_c, center_line);
                    
                    // Streetlight illumination pools projected on the asphalt surface
                    let lamp_z_nearest = round(hit_pos.z / 24.0) * 24.0;
                    let dist_to_lamp_left = length(hit_pos.xz - vec2<f32>(road_x(lamp_z_nearest) - 8.5, lamp_z_nearest));
                    let dist_to_lamp_right = length(hit_pos.xz - vec2<f32>(road_x(lamp_z_nearest) + 8.5, lamp_z_nearest));
                    let lamp_glow = smoothstep(6.5, 0.0, min(dist_to_lamp_left, dist_to_lamp_right)) * 
                                    vec3<f32>(0.98, 0.82, 0.25) * 0.7;
                    material_color += lamp_glow;
                    
                    // Glowing pink/magenta underglow under the car
                    let dist_to_car = length(hit_pos.xz - car_pos.xz);
                    let underglow = smoothstep(4.5, 0.0, dist_to_car) * vec3<f32>(0.98, 0.02, 0.52) * 3.5;
                    material_color += underglow;
                } else {
                    // Deep neon purple landscape with grid lines
                    let ground_grid = smoothstep(0.08, 0.0, abs(fract(hit_pos.x * 0.12) - 0.5)) +
                                      smoothstep(0.08, 0.0, abs(fract(hit_pos.z * 0.12 - audio.smooth_time * 2.6) - 0.5));
                    let ground_base = vec3<f32>(0.015, 0.0, 0.045);
                    let ground_line = vec3<f32>(0.7, 0.0, 0.45) * 0.6;
                    material_color = mix(ground_base, ground_line, clamp(ground_grid, 0.0, 1.0));
                    
                    // Neon green foliage bands along the road shoulders
                    if (dist_road > 8.8 && dist_road < 11.5) {
                        let wave_f = sin(hit_pos.z * 0.2) * 0.4 + cos(hit_pos.z * 0.08) * 0.3;
                        let foliage_mask = step(0.0, wave_f + 0.3 * sin(audio.smooth_time + hit_pos.x * 2.0));
                        let foliage_c = vec3<f32>(0.0, 0.95, 0.65) * 1.0 * foliage_mask;
                        material_color = mix(material_color, foliage_c, 0.85);
                    }
                }
            }
            default: {}
        }
        
        // 6. Horizon atmospheric fog to blend the 3D scene cleanly into the sunset
        let fog_factor = smoothstep(120.0, 480.0, t_closest);
        let sky_background = sample_sky(rd);
        final_color = mix(material_color, sky_background, fog_factor);
    }
    
    // ACES Cinematic Tone Mapping for premium visual color grading
    let tonemapped = (final_color * (2.51 * final_color + 0.03)) / (final_color * (2.43 * final_color + 0.59) + 0.14);
    let exposure_adjusted = max(tonemapped, vec3<f32>(0.0));
    
    return vec4<f32>(exposure_adjusted, 1.0);
}
