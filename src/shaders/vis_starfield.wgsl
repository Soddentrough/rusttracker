// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.xyx) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// Smooth value noise (bilinear interpolation of hash values)
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    // Smooth hermite interpolation
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Fractal Brownian Motion for smooth cloud shapes
fn fbm(p: vec2<f32>) -> f32 {
    var val = 0.0;
    var amp = 0.5;
    var pos = p;
    let rot = mat2x2<f32>(0.866, -0.5, 0.5, 0.866); // 30 deg rotation to break grid lines
    for (var i = 0; i < 4; i = i + 1) {
        val += amp * value_noise(pos);
        pos = rot * pos * 2.1 + vec2<f32>(1.7, 3.1);
        amp *= 0.5;
    }
    return val;
}

fn rotate2d(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

fn get_star_color(temp: f32) -> vec3<f32> {
    if (temp < 0.3) {
        return vec3<f32>(0.6, 0.7, 1.0); // Blue-white
    } else if (temp < 0.6) {
        return vec3<f32>(0.9, 0.9, 0.95); // White
    } else if (temp < 0.85) {
        return vec3<f32>(1.0, 0.9, 0.6); // Yellow
    } else {
        return vec3<f32>(1.0, 0.5, 0.3); // Red giant
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = max(audio.aspect_ratio, 0.1);
    let p = vec2<f32>((in.uv.x * 2.0 - 1.0) * aspect, -(in.uv.y * 2.0 - 1.0));

    // --- Audio analysis ---
    let bass = clamp(max(audio.spectrum[0].x, audio.spectrum[1].x) * 1.5, 0.0, 2.5);
    let mid = clamp((audio.spectrum[10].x + audio.spectrum[20].x + audio.spectrum[30].x) * 0.33, 0.0, 2.0);
    let treble = clamp((audio.spectrum[60].x + audio.spectrum[80].x) * 0.5, 0.0, 2.0);

    // Bass-driven hyperspace warp speed boost
    let warp_boost = smoothstep(0.25, 1.6, bass);
    let speed = audio.smooth_time * (0.14 + warp_boost * 0.40);

    var color = vec3<f32>(0.0);

    // --- Deep space background ---
    let bg_t = clamp(p.y * 0.5 + 0.5, 0.0, 1.0);
    color = mix(vec3<f32>(0.002, 0.003, 0.012), vec3<f32>(0.008, 0.005, 0.018), bg_t);

    // --- Nebula clouds (reacts to mid & treble audio energy) ---
    let neb_uv = p * 0.9 + vec2<f32>(audio.smooth_time * 0.01, audio.smooth_time * 0.015);
    let nebula_shape = fbm(neb_uv * 2.5);
    let neb_mask = smoothstep(0.28, 0.68, nebula_shape);

    let neb_col1 = vec3<f32>(0.16, 0.03, 0.24) * (1.0 + mid * 0.7);
    let neb_col2 = vec3<f32>(0.02, 0.09, 0.28) * (1.0 + treble * 0.5);
    let nebula_color = mix(neb_col1, neb_col2, smoothstep(-aspect, aspect, p.x)) * neb_mask * (0.35 + mid * 0.45);
    color += nebula_color;

    // --- 3D Flight Starfield ---
    let p_rot = rotate2d(p, audio.smooth_time * 0.04);
    
    let num_layers = 25.0;
    for (var i = 0.0; i < 1.0; i += 1.0 / num_layers) {
        // Depth goes from 1.0 (far away) to 0.0 (past the camera)
        let z = fract(i - speed); 
        
        // Scale UV by depth to create perspective
        let z_scale = z * 20.0 + 0.1;
        
        // Offset each layer so grid structures don't overlap
        let layer_offset = hash22(vec2<f32>(i * 123.4, i * 567.8)) * 100.0;
        let cell_uv = p_rot * z_scale + layer_offset;
        let id = floor(cell_uv);
        let gv = fract(cell_uv) - 0.5;
        
        // Random properties per cell
        let rnd = hash22(id + i * 133.0);
        if (rnd.x > 0.16) { continue; } // Star density probability
        
        let fade_offset = (rnd.x - 0.5) * 0.15;
        let z_fade = smoothstep(0.0, 0.08 + fade_offset, z) * (1.0 - smoothstep(0.82 + fade_offset, 1.0, z));
        
        // Position within cell
        let star_pos = (rnd - 0.5) * 0.5;
        
        // Hyperspace warp stretch: stars stretch into radial streaks on heavy bass hits
        let streak_len = 1.0 + warp_boost * 3.2 * (1.0 - z);
        let d_vec = gv - star_pos;
        let dist = length(vec2<f32>(d_vec.x, d_vec.y / streak_len));
        
        // Audio reactive size & glow per instrument channel
        let rnd_z = hash21(id + i * 77.0);
        var ch_vu = 0.0;
        
        if (rnd_z > 0.45) {
            let num_ch = max(audio.num_channels, 1u);
            var num_inst = num_ch;
            var start_idx = 0u;
            if (num_ch > 2u) {
                num_inst = num_ch - 2u;
                start_idx = 1u;
            }
            
            let inst_idx = u32(rnd.y * f32(num_inst));
            let ch_idx = start_idx + inst_idx;
            
            let v_vec = audio.channels[ch_idx / 4u];
            let comp = ch_idx % 4u;
            
            if comp == 0u { ch_vu = v_vec.x; }
            else if comp == 1u { ch_vu = v_vec.y; }
            else if comp == 2u { ch_vu = v_vec.z; }
            else { ch_vu = v_vec.w; }
            
            ch_vu = smoothstep(0.25, 0.85, ch_vu) * 2.2; 
        }

        let pulse = 1.0 + ch_vu + warp_boost * 0.5;
        let base_size = 0.012 + rnd.y * 0.016;
        let size = clamp(base_size * pulse, 0.0, 0.12);
        
        // Core and glow
        let core = 1.0 - smoothstep(size * 0.1, size, dist);
        
        let base_glow = 0.00012 / (dist * dist + 0.0004);
        let flash_glow = ((ch_vu + warp_boost * 0.8) * 0.0006) / (dist * dist + 0.0002);
        let glow = base_glow + flash_glow;
        
        let cell_mask = (1.0 - smoothstep(0.25, 0.5, abs(gv.x))) * (1.0 - smoothstep(0.25, 0.5, abs(gv.y)));
        let star_col = get_star_color(hash21(id + 30.0));
        
        let brightness = (core + glow) * cell_mask * z_fade;
        color += star_col * brightness;
    }

    // ACES tonemapping for crisp HDR star cores
    var final_col = aces_tonemap(color);
    return vec4<f32>(final_col, 1.0);
}
