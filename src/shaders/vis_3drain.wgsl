// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

fn hash11(p: f32) -> f32 {
    var p2 = fract(p * 0.1031);
    p2 = p2 * (p2 + 33.33);
    return fract(2.0 * p2 * p2);
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.xyx) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn project_3d(p3: vec3<f32>, ro: vec3<f32>, u: vec3<f32>, v_cam: vec3<f32>, w: vec3<f32>) -> vec3<f32> {
    let dir = p3 - ro;
    let dist_w = dot(dir, w);
    if dist_w <= 0.001 { return vec3<f32>(999.0, 999.0, dist_w); }
    let proj_x = dot(dir, u) / dist_w;
    let proj_y = dot(dir, v_cam) / dist_w;
    // Negate proj_y so +Y (up) maps to +Y in UV space
    return vec3<f32>(proj_x, proj_y, dist_w);
}

fn draw_projected_segment(p: vec2<f32>, pA: vec3<f32>, pB: vec3<f32>, thickness_base: f32, intensity: f32, is_branch: bool) -> f32 {
    if (pA.z <= 0.1 || pB.z <= 0.1) {
        return 0.0;
    }
    
    let margin = select(0.08, 0.04, is_branch);
    let min_x = min(pA.x, pB.x) - margin;
    let max_x = max(pA.x, pB.x) + margin;
    let min_y = min(pA.y, pB.y) - margin;
    let max_y = max(pA.y, pB.y) + margin;
    
    if (p.x < min_x || p.x > max_x || p.y < min_y || p.y > max_y) {
        return 0.0;
    }
    
    let d = sd_segment(p, pA.xy, pB.xy);
    let depth = max(0.4, max(pA.z, pB.z));
    let thickness = thickness_base / depth;
    let core = smoothstep(thickness * 0.5, 0.0, d);
    let glow_radius = select(60.0, 120.0, is_branch); // Branches have a tighter, less intense glow
    let glow_mult = select(0.7, 0.3, is_branch);
    let glow = exp(-d * glow_radius) * glow_mult;
    return (core + glow) * intensity;
}

fn draw_3d_lightning_bolt(ro: vec3<f32>, u: vec3<f32>, v_cam: vec3<f32>, w: vec3<f32>, p: vec2<f32>, seed: f32, intensity: f32, origin: vec3<f32>, plane_z: f32) -> f32 {
    if (intensity < 0.01) { return 0.0; }

    let segments = 15;
    let frustum_height = plane_z - ro.z;
    let bolt_height = frustum_height * 2.4; // Ensure it reaches ground
    let seg_h = bolt_height / f32(segments);
    let j_scale = plane_z * 0.15; // Scale jitter with distance

    // Bounding box check in screen space (Fast Pruning)
    let max_dev_3d = 6.5 * j_scale;
    let p_top = project_3d(origin, ro, u, v_cam, w);
    let p_bot = project_3d(origin - vec3<f32>(0.0, bolt_height, 0.0), ro, u, v_cam, w);
    
    let depth_min = max(0.1, plane_z - max_dev_3d - ro.z);
    let margin = max_dev_3d / depth_min + 0.15;
    
    let d_line = sd_segment(p, p_top.xy, p_bot.xy);
    if (d_line > margin) { return 0.0; }

    var bolt = 0.0;
    var current_pos = origin;
    var p_curr_proj = p_top;

    // Simulate dart leaders and return stroke pulsing via high-frequency flicker
    let flicker = 0.6 + 0.4 * sin(audio.smooth_time * 60.0 + seed);
    let main_intensity = intensity * flicker;
    let branch_intensity = intensity * 0.4; // Stepped leaders are much dimmer

    for (var i = 0; i < segments; i = i + 1) {
        let f_i = f32(i);
        let x_jitter = (hash11(seed + f_i * 7.13) - 0.5) * j_scale * 2.5;
        let z_jitter = (hash11(seed + f_i * 13.7) - 0.5) * j_scale * 2.5;
        let next_pos = current_pos + vec3<f32>(x_jitter, -seg_h, z_jitter);
        let p_next_proj = project_3d(next_pos, ro, u, v_cam, w);

        // 1. Main Return Stroke Channel
        let thickness = 0.012 + main_intensity * 0.015;
        bolt += draw_projected_segment(p, p_curr_proj, p_next_proj, thickness, main_intensity, false);

        // 2. Stepped Leaders (Primary Branches)
        if (hash11(seed + f_i * 3.1) > 0.25) {
            let bx = (hash11(seed + f_i * 8.2) - 0.5) * j_scale * 4.0;
            let bz = (hash11(seed + f_i * 9.3) - 0.5) * j_scale * 4.0;
            let branch_pos = current_pos + vec3<f32>(bx, -seg_h * 1.5, bz);
            let p_branch_proj = project_3d(branch_pos, ro, u, v_cam, w);
            
            bolt += draw_projected_segment(p, p_curr_proj, p_branch_proj, 0.005, branch_intensity, true);
            
            // 3. Sub-branches (Secondary splits from the stepped leaders)
            if (hash11(seed + f_i * 2.4) > 0.35) {
                let bx2 = (hash11(seed + f_i * 1.2) - 0.5) * j_scale * 3.0;
                let bz2 = (hash11(seed + f_i * 4.3) - 0.5) * j_scale * 3.0;
                let branch_pos2 = branch_pos + vec3<f32>(bx2, -seg_h * 1.2, bz2);
                let p_branch2_proj = project_3d(branch_pos2, ro, u, v_cam, w);
                
                bolt += draw_projected_segment(p, p_branch_proj, p_branch2_proj, 0.003, branch_intensity * 0.6, true);
            }
        }
        
        // 4. Major splits (connecting leaders that travel alongside the main channel)
        if (i == 3 || i == 8) {
            var split_pos = current_pos;
            var p_split_proj = p_curr_proj;
            for (var j = 0; j < 4; j = j + 1) {
                let f_j = f32(j);
                let sx = (hash11(seed + f_i * 5.0 + f_j) - 0.5) * j_scale * 3.0;
                let sz = (hash11(seed + f_i * 6.0 + f_j) - 0.5) * j_scale * 3.0;
                let next_split = split_pos + vec3<f32>(sx, -seg_h * 1.1, sz);
                let p_next_split_proj = project_3d(next_split, ro, u, v_cam, w);
                
                bolt += draw_projected_segment(p, p_split_proj, p_next_split_proj, 0.007, branch_intensity * 1.5, true);
                split_pos = next_split;
                p_split_proj = p_next_split_proj;
            }
        }

        current_pos = next_pos;
        p_curr_proj = p_next_proj;
    }

    return bolt;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);
    let safe_aspect = select(1.0, aspect, abs(aspect) > 0.0001);
    let p = vec2<f32>(uv.x * safe_aspect, -uv.y); // p.y is +1 at top, -1 at bottom

    let bass = max(audio.spectrum[0].x, audio.spectrum[1].x);
    let mid = (audio.spectrum[20].x + audio.spectrum[30].x) * 0.5;
    let treble = (audio.spectrum[60].x + audio.spectrum[80].x) * 0.5;

    // Camera setup
    let rot_angle = sin(audio.smooth_time * 0.2) * 0.15; 
    let cam_dist = 3.0;
    let ro = vec3<f32>(sin(rot_angle) * cam_dist, 1.0, -cos(rot_angle) * cam_dist);
    let cam_target = vec3<f32>(0.0, 1.0, 10.0);
    
    let w = normalize(cam_target - ro);
    let u = normalize(cross(w, vec3<f32>(0.0, 1.0, 0.0)));
    let v_cam = cross(u, w); // v_cam points UP
    let rd = normalize(w + p.x * u + p.y * v_cam);

    var color = vec3<f32>(0.0);
    var total_flash = 0.0;

    // Sky background
    let sky_t = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0); 
    let sky_dark = vec3<f32>(0.005, 0.008, 0.018);   
    let sky_mid  = vec3<f32>(0.015, 0.02, 0.045);     
    let sky_top  = vec3<f32>(0.025, 0.03, 0.055);     
    color = mix(sky_dark, mix(sky_mid, sky_top, sky_t), sky_t);

    // Three 3D planes for rain and lightning
    let z_layers = array<f32, 3>(18.0, 11.0, 6.5); // Far (Bass), Mid, Near (Treble)
    let layer_colors = array<vec3<f32>, 3>(
        vec3<f32>(1.0, 1.0, 1.0),  // White (Far)
        vec3<f32>(1.0, 1.0, 1.0),  // White (Mid)
        vec3<f32>(1.0, 1.0, 1.0)   // White (Near)
    );
    let layer_intensities = array<f32, 3>(
        smoothstep(0.6, 1.2, bass),
        smoothstep(0.5, 1.0, mid),
        smoothstep(0.4, 0.9, treble)
    );
    let layer_speeds = array<f32, 3>(8.0, 11.0, 15.0);
    let layer_scales = array<f32, 3>(6.0, 3.5, 2.0);
    let layer_base_brightness = array<f32, 3>(0.05, 0.25, 0.6);

    for (var i = 0; i < 3; i = i + 1) {
        let plane_z = z_layers[i];
        let layer_col = layer_colors[i];
        let intensity = layer_intensities[i];
        
        // Render 3D Lightning for this layer
        // Lock lightning to discrete time windows so the geometry is stable during an audio transient.
        let interval = 0.35; // 350ms strike window
        let time_window = floor(audio.smooth_time / interval);
        let strike_chance = hash11(time_window + f32(i) * 17.3);
        
        // Far layer (i=0) strikes more often. Near layer (i=2) needs higher chance.
        let chance_threshold = 0.25 + f32(i) * 0.2; 
        
        if (strike_chance > chance_threshold && intensity > 0.05) {
            let bolt_seed = time_window + f32(i) * 13.37;
            let frustum_height = plane_z - ro.z;
            let frustum_width = safe_aspect * frustum_height * 2.0;
            let origin_x = (hash11(bolt_seed) - 0.5) * frustum_width * 1.5; // Spread horizontally
            let origin_y = ro.y + frustum_height * 1.2;
            let origin = vec3<f32>(origin_x, origin_y, plane_z); // Start above top of screen
            
            let bolt = draw_3d_lightning_bolt(ro, u, v_cam, w, p, bolt_seed, intensity, origin, plane_z);
            color += vec3<f32>(1.0) * bolt * (0.65 + intensity * 0.4);
            total_flash += bolt * 0.45;
            
            // Secondary parallel strike (rare)
            if (intensity > 0.4 && hash11(bolt_seed + 7.7) > 0.6) {
                let origin2_x = origin_x + (hash11(bolt_seed + 100.0) - 0.5) * frustum_width * 0.3;
                let origin2 = vec3<f32>(origin2_x, origin_y, plane_z);
                let bolt2 = draw_3d_lightning_bolt(ro, u, v_cam, w, p, bolt_seed + 50.0, intensity * 0.38, origin2, plane_z);
                color += vec3<f32>(1.0) * bolt2 * 0.28;
                total_flash += bolt2 * 0.18;
            }
        }

        // Render 3D Rain plane
        // Intersect ray with Z = plane_z
        let t = (plane_z - ro.z) / rd.z;
        if (t > 0.0) {
            let pos = ro + rd * t;
            // pos.xy is the 2D coordinate on the rain plane
            
            let scale_x = layer_scales[i];
            let scale_y = 1.0 + f32(i) * 0.18;
            let speed = layer_speeds[i];
            let wind_slant = 0.08 + f32(i) * 0.03;
            
            let rain_x = pos.x * scale_x + f32(i) * 10.2 + pos.y * wind_slant * scale_x;
            let col_id = floor(rain_x);
            let col_speed = 1.0 + (hash11(col_id) - 0.5) * 0.3;
            
            var rain_uv = vec2<f32>(
                rain_x,
                pos.y * scale_y + audio.smooth_time * speed * col_speed
            );
            
            rain_uv.y += hash11(col_id) * 3.14159;
            
            let cell_id = floor(rain_uv);
            let cell_uv = fract(rain_uv) - 0.5;
            
            let rnd = hash22(cell_id + f32(i) * 100.0);
            let drop_sway = sin(audio.smooth_time * (2.0 + rnd.y * 2.0) + rnd.x * 6.28) * 0.15;
            let drop_x = (rnd.x - 0.5) * 0.6 + drop_sway;
            let visible = step(0.45, rnd.y);
            
            let dx_drop = abs(cell_uv.x - drop_x);
            let streak_width = 0.02 + f32(i) * 0.003;
            let streak = smoothstep(streak_width, 0.0, dx_drop);
            
            let drop_line = cell_uv.y + cell_uv.x * 0.35;
            let drop_length = 0.25 + rnd.x * 0.24 + f32(i) * 0.08;
            let head = smoothstep(-0.05, 0.0, drop_line);
            let tail = smoothstep(drop_length, -0.08, drop_line);
            let vert = head * tail;
            
            let drop = streak * vert * visible;
            let rain_brightness = layer_base_brightness[i] + intensity * 0.55;
            color += layer_col * drop * rain_brightness;
            color += vec3<f32>(1.0) * drop * intensity * 0.08;
        }
    }

    let flash = clamp(total_flash, 0.0, 1.0);
    color += vec3<f32>(0.85, 0.9, 1.0) * flash * 0.28;

    // Film grain
    let grain = (hash12(in.clip_position.xy + fract(audio.smooth_time) * 137.0) - 0.5) * 0.03;
    color += vec3<f32>(grain);

    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));

    return vec4<f32>(final_col, 1.0);
}
