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

fn draw_3d_lightning_bolt(ro: vec3<f32>, u: vec3<f32>, v_cam: vec3<f32>, w: vec3<f32>, p: vec2<f32>, seed: f32, intensity: f32, origin: vec3<f32>) -> f32 {
    if (intensity < 0.01) { return 0.0; }

    var bolt = 0.0;
    var current_pos = origin;
    let segments = 12;
    let seg_h = 8.0 / f32(segments); // Bolt is 8 units tall

    // Fast bounds check in 3D: if origin is way off screen, skip
    let origin_proj = project_3d(origin + vec3<f32>(0.0, -4.0, 0.0), ro, u, v_cam, w);
    if (origin_proj.z > 0.0 && length(origin_proj.xy - p) > (2.0 / origin_proj.z) + 0.5) {
        // Approximate far glow
        let d = length(origin_proj.xy - p);
        return intensity * 0.01 / (d + 0.01);
    }

    for (var i = 0; i < segments; i = i + 1) {
        let x_jitter = (hash11(seed + f32(i) * 7.13) - 0.5) * 1.5;
        let z_jitter = (hash11(seed + f32(i) * 13.7) - 0.5) * 1.5;
        
        let next_pos = current_pos + vec3<f32>(x_jitter, -seg_h, z_jitter);

        let pA = project_3d(current_pos, ro, u, v_cam, w);
        let pB = project_3d(next_pos, ro, u, v_cam, w);
        
        if (pA.z > 0.0 || pB.z > 0.0) {
            let d = sd_segment(p, pA.xy, pB.xy);
            let depth = max(0.5, (pA.z + pB.z) * 0.5);
            let thickness = 0.01 / depth;

            bolt += smoothstep(thickness, 0.0, d) * 2.0;
            bolt += (0.003 / depth) / (d + 0.002);
        }

        current_pos = next_pos;
    }

    return bolt * intensity;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);
    let safe_aspect = select(1.0, aspect, aspect > 0.0001 || aspect < -0.0001);
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

    // Sky background
    let sky_t = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0); 
    let sky_dark = vec3<f32>(0.005, 0.008, 0.018);   
    let sky_mid  = vec3<f32>(0.015, 0.02, 0.045);     
    let sky_top  = vec3<f32>(0.025, 0.03, 0.055);     
    color = mix(sky_dark, mix(sky_mid, sky_top, sky_t), sky_t);

    // Three 3D planes for rain and lightning
    let z_layers = array<f32, 3>(16.0, 10.0, 4.0); // Far (Bass), Mid, Near (Treble)
    let layer_colors = array<vec3<f32>, 3>(
        vec3<f32>(0.4, 0.5, 1.0),  // Deep blue
        vec3<f32>(0.7, 0.4, 1.0),  // Purple
        vec3<f32>(0.4, 1.0, 0.8)   // Cyan
    );
    let layer_intensities = array<f32, 3>(
        smoothstep(0.6, 1.2, bass),
        smoothstep(0.5, 1.0, mid),
        smoothstep(0.4, 0.9, treble)
    );

    for (var i = 0; i < 3; i++) {
        let plane_z = z_layers[i];
        let layer_col = layer_colors[i];
        let intensity = layer_intensities[i];
        
        // Render 3D Lightning for this layer
        if (intensity > 0.05) {
            let bolt_seed = floor(audio.smooth_time * 5.0) + f32(i) * 13.37;
            let origin_x = (hash11(bolt_seed) - 0.5) * 15.0; // Spread horizontally
            let origin = vec3<f32>(origin_x, 8.0, plane_z); // Start high up
            
            let bolt = draw_3d_lightning_bolt(ro, u, v_cam, w, p, bolt_seed, intensity, origin);
            color += layer_col * bolt;
            
            if (intensity > 0.4) {
                let origin2 = origin + vec3<f32>((hash11(bolt_seed + 100.0) - 0.5) * 4.0, -1.0, 0.0);
                let bolt2 = draw_3d_lightning_bolt(ro, u, v_cam, w, p, bolt_seed + 50.0, intensity * 0.4, origin2);
                color += layer_col * bolt2;
            }
        }

        // Render 3D Rain plane
        // Intersect ray with Z = plane_z
        let t = (plane_z - ro.z) / rd.z;
        if (t > 0.0) {
            let pos = ro + rd * t;
            // pos.xy is the 2D coordinate on the rain plane
            
            let scale_x = 3.0;
            let scale_y = 1.0;
            let speed = 10.0;
            let wind_slant = 0.05;
            let wind_sway = sin(audio.smooth_time * 0.3) * 1.5;
            
            var rain_uv = vec2<f32>(
                pos.x * scale_x + f32(i) * 13.7 + pos.y * wind_slant * scale_x + wind_sway,
                pos.y * scale_y + audio.smooth_time * speed
            );
            
            rain_uv.y += hash11(floor(rain_uv.x)) * 3.14159;
            
            let cell_id = floor(rain_uv);
            let cell_uv = fract(rain_uv) - 0.5;
            
            let rnd = hash22(cell_id + f32(i) * 100.0);
            let drop_x = (rnd.x - 0.5) * 0.6;
            let visible = step(0.45, rnd.y); 
            
            let dx_drop = abs(cell_uv.x - drop_x);
            let streak_width = 0.02;
            let streak = smoothstep(streak_width, 0.0, dx_drop);
            
            let drop_length = 0.3 + rnd.x * 0.15;
            let head = smoothstep(-drop_length, -drop_length + 0.05, cell_uv.y);
            let tail = smoothstep(drop_length, -drop_length, cell_uv.y);
            let vert = head * tail;
            
            // Fade out based on distance and Y height
            let fog = exp(-t * 0.05);
            let drop = streak * vert * visible * fog;
            
            let rain_brightness = 0.25 + intensity * 0.4;
            color += layer_col * drop * rain_brightness;
        }
    }

    // Film grain
    let grain = (hash12(in.clip_position.xy + fract(audio.smooth_time) * 137.0) - 0.5) * 0.03;
    color += vec3<f32>(grain);

    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));

    return vec4<f32>(final_col, 1.0);
}
