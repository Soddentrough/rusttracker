// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;


@group(0) @binding(1)
var<storage, read> waveform_history: array<f32>;

// Hash function for analog noise
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Waveforms are pre-smoothed on CPU, so we can read directly
fn get_waveform(hist_idx: u32, idx: u32) -> f32 {
    let res = max(audio.waveform_resolution, 128u);
    let clamped_idx = clamp(idx, 0u, res - 1u);
    let clamped_hist = clamp(hist_idx, 0u, max(1u, audio.waveform_history_size) - 1u);
    return waveform_history[clamped_hist * 2048u + clamped_idx];
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
    // Negate proj_y so +Z (up) maps to -Y (top of screen in our coords)
    return vec3<f32>(proj_x, -proj_y, dist_w);
}



@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let crt_uv = in.uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    let distorted_uv = crt_uv * (1.0 + r2 * 0.04);
    
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);
    
    // Scale screen coords
    let p = vec2<f32>(distorted_uv.x * aspect, distorted_uv.y) * 0.9;
    
    // Subtly rotate camera around center
    let rot_angle = sin(audio.time * 0.2) * 0.15; 
    let cam_dist = 3.2;
    let cam_height = 1.8; // Higher up to look down at the grid
    
    let ro = vec3<f32>(sin(rot_angle) * cam_dist, -cos(rot_angle) * cam_dist, cam_height);
    let cam_target = vec3<f32>(0.0, 0.0, 0.0);
    
    let w = normalize(cam_target - ro);
    let u = normalize(cross(w, vec3<f32>(0.0, 0.0, 1.0)));
    let v_cam = cross(u, w);
    
    let rd = normalize(w + p.x * u - p.y * v_cam);

    var accumulated_color = vec3<f32>(0.0);
    let amber_lo = vec3<f32>(0.6, 0.18, 0.02);
    let amber_hi = vec3<f32>(1.0, 0.55, 0.08);

    let num_lines = min(max(1u, audio.waveform_history_size), 144u);
    let res = f32(max(audio.waveform_resolution, 128u));
    
    for (var i = 0u; i < num_lines; i = i + 1u) {
        // Z layout: back to front. i=0 is back (oldest), i=num_lines-1 is front (newest).
        let y_line = mix(9.48, -1.8, f32(i) / f32(num_lines - 1u));
        let hist_idx = i;
        
        let t = (y_line - ro.y) / rd.y;
        if t <= 0.0 {
            // Lines are iterated back-to-front (decreasing Y). When rd.y > 0,
            // once t goes negative all remaining lines are also behind the camera.
            if rd.y > 0.0 { break; }
            continue;
        }
        {
            let hit_x = ro.x + rd.x * t;
            
            let edge_fade = smoothstep(9.6, 6.6, abs(hit_x));
            if edge_fade <= 0.00001 {
                continue;
            }
            
            // Map X from -9.6 to 9.6 to dynamic resolution
            let float_idx = (hit_x + 9.6) / 19.2 * (res - 1.0);
            let idx = i32(round(float_idx));
            
            // Search radius proportional to distance to avoid aliasing/dropout on distant lines
            // Scale down slightly since we have 3x more lines now
            let search_radius = clamp(i32(t * 0.4), 1, 6);
            let start_idx = max(0, idx - search_radius);
            let end_idx = min(i32(res) - 1i, idx + search_radius);
            
            var min_dist = 1000.0;
            
            var j_u = u32(start_idx);
            var x_prev = -9.6 + f32(j_u) / (res - 1.0) * 19.2;
            var mask_prev = smoothstep(9.6, 6.6, abs(x_prev));
            var v_prev = get_waveform(hist_idx, j_u);
            var p_prev = v_prev * mask_prev * 1.2;
            var p3_prev = vec3<f32>(x_prev, y_line, p_prev);
            var proj_prev = project_3d(p3_prev, ro, u, v_cam, w);
            
            for (var j = start_idx; j <= end_idx; j = j + 1) {
                j_u = u32(j + 1);
                let x_curr = -9.6 + f32(j_u) / (res - 1.0) * 19.2;
                let mask_curr = smoothstep(9.6, 6.6, abs(x_curr));
                let v_curr = get_waveform(hist_idx, j_u);
                let p_curr = v_curr * mask_curr * 1.2;
                let p3_curr = vec3<f32>(x_curr, y_line, p_curr);
                let proj_curr = project_3d(p3_curr, ro, u, v_cam, w);
                
                if proj_prev.z > 0.001 && proj_curr.z > 0.001 {
                    let d = sd_segment(p, proj_prev.xy, proj_curr.xy);
                    min_dist = min(min_dist, d);
                }
                
                proj_prev = proj_curr;
            }
            
            let r = length(crt_uv);
            let edge_blur = smoothstep(0.3, 1.2, r);
            
            // Distance-based thickness to prevent sub-pixel wireframe flickering
            let depth_thickness = t * 0.0004;
            let thickness = 0.002 + edge_blur * 0.004 + depth_thickness;
            let core = smoothstep(thickness, 0.0, min_dist);
            // High bloom for that glowing CRT look, scaling bloom spread with depth
            let bloom = 0.0004 / (min_dist * min_dist + 0.0001 + t * 0.00005) * 0.15;
            
            // Depth fade (lines further back are slightly darker)
            let depth_fade = exp(-t * 0.20);
            
            // Age fade (older lines fade out)
            let age_fade = mix(0.05, 1.0, f32(i) / f32(num_lines - 1u));
            
            // Sample waveform height at hit point for color grading
            let hit_wave_idx = u32(clamp((hit_x + 9.6) / 19.2 * (res - 1.0), 0.0, res - 1.0));
            let wave_height = clamp(abs(get_waveform(hist_idx, hit_wave_idx)) * 2.0, 0.0, 1.0);
            let line_amber = mix(amber_lo, amber_hi, wave_height);
            
            accumulated_color += line_amber * (core + bloom) * depth_fade * edge_fade * age_fade;
        }
    }
    
    let mapped = accumulated_color;
    var tonemapped = (mapped * (2.51 * mapped + 0.03)) / (mapped * (2.43 * mapped + 0.59) + 0.14);
    
    var final_color = tonemapped;
    
    // CRT scanlines
    let scanline = 0.85 + 0.15 * cos(in.clip_position.y * 3.14159);
    final_color *= scanline;
    
    // Smooth CRT bezel fade
    let r = length(crt_uv);
    let bezel = smoothstep(1.3, 0.9, r);
    
    // Analog noise
    let noise_val = hash12(in.clip_position.xy + fract(audio.smooth_time) * 100.0);
    let acc_lum = dot(accumulated_color, vec3<f32>(0.299, 0.587, 0.114));
    let noise_color = vec3<f32>(0.8, 0.35, 0.05) * noise_val * 0.02 * bezel * (0.3 + 0.7 * clamp(acc_lum * 0.5, 0.0, 1.0));
    
    final_color = final_color * bezel + noise_color;
    
    return vec4<f32>(final_color, 1.0);
}
