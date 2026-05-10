struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let u = f32((in_vertex_index << 1u) & 2u);
    let v = f32(in_vertex_index & 2u);
    out.clip_position = vec4<f32>(u * 2.0 - 1.0, -(v * 2.0 - 1.0), 0.0, 1.0);
    out.uv = vec2<f32>(u, v);
    return out;
}

struct AudioUniforms {
    spectrum: array<vec4<f32>, 256>,
    fire_heat: array<vec4<f32>, 256>,
    channels: array<vec4<f32>, 8>,
    channel_peaks: array<vec4<f32>, 8>,
    spatial_channels: array<vec4<f32>, 4>,
    display_order: array<vec4<u32>, 4>,
    num_channels: u32,
    mode: u32,
    time: f32,
    duration: f32,
    smooth_time: f32,
    heatmap_row: u32,
    fft_channels: u32,
    num_spatial_channels: u32,
    ui_meters_rect: vec4<f32>,
    ui_heatmap_rect: vec4<f32>,
    ui_fire_rect: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read> waveform_history: array<vec4<f32>>;

@group(0) @binding(4)
var<storage, read> gpu_spectrum: array<vec2<f32>>;

// Hash function for analog noise
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn get_resynthesized_wave(line_idx: u32, x_norm: f32) -> f32 {
    let point_idx = clamp(u32(x_norm * 511.0), 0u, 511u);
    let base_offset = 16u * 1024u;
    let flat_idx = line_idx * 512u + point_idx;
    return gpu_spectrum[base_offset + flat_idx].x;
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
    
    let num_lines = 32u;
    let num_points = 512u;
    
    // We render the 3D grid back-to-front
    for (var i = 0u; i < num_lines; i = i + 1u) {
        // Z layout: back to front. i=0 is back (low freq), i=31 is front (high freq).
        let i_f = f32(i);
        let y_line = mix(5.64, -1.8, i_f / 31.0);
        
        let t = (y_line - ro.y) / rd.y;
        
        if t > 0.0 {
            let hit_x = ro.x + rd.x * t;
            
            let float_idx = (hit_x + 8.0) / 16.0 * f32(num_points - 1u);
            let idx = i32(round(float_idx));
            
            let start_idx = max(0, idx - 4);
            let end_idx = min(i32(num_points) - 2, idx + 4);
            
            var min_dist = 1000.0;
            
            // Hoist the first point evaluation to halve the workload
            let j_start_u = u32(start_idx);
            let x0_init = mix(-8.0, 8.0, f32(j_start_u) / f32(num_points - 1u));
            var mask0 = smoothstep(8.0, 5.0, abs(x0_init));
            var x_norm0 = f32(j_start_u) / f32(num_points - 1u);
            var v0 = get_resynthesized_wave(i, x_norm0);
            var x0 = x0_init;
            
            for (var j = start_idx; j <= end_idx; j = j + 1) {
                let j_u = u32(j);
                
                let x1 = mix(-8.0, 8.0, f32(j_u + 1u) / f32(num_points - 1u));
                let mask1 = smoothstep(8.0, 5.0, abs(x1));
                let x_norm1 = f32(j_u + 1u) / f32(num_points - 1u);
                
                let v1 = get_resynthesized_wave(i, x_norm1);
                
                // Allow true negative waveforms, just apply mask
                let p0 = v0 * mask0 * 4.0;
                let p1 = v1 * mask1 * 4.0;
                
                let p3_0 = vec3<f32>(x0, y_line, p0);
                let p3_1 = vec3<f32>(x1, y_line, p1);
                
                let proj0 = project_3d(p3_0, ro, u, v_cam, w);
                let proj1 = project_3d(p3_1, ro, u, v_cam, w);
                
                if proj0.z > 0.001 && proj1.z > 0.001 {
                    let d = sd_segment(p, proj0.xy, proj1.xy);
                    min_dist = min(min_dist, d);
                }
                
                v0 = v1;
                x0 = x1;
                mask0 = mask1;
            }
            
            let r = length(crt_uv);
            let edge_blur = smoothstep(0.3, 1.2, r);
            
            let thickness = 0.002 + edge_blur * 0.004;
            let core = smoothstep(thickness, 0.0, min_dist);
            // High bloom for that glowing CRT look
            let bloom = 0.0004 / (min_dist * min_dist + 0.0001) * 0.15;
            
            // Depth fade (lines further back are slightly darker)
            let depth_fade = exp(-t * 0.20);
            let edge_fade = smoothstep(8.0, 5.0, abs(hit_x));
            
            // Sample waveform height at hit point for color grading
            let hit_x_norm = clamp((hit_x + 8.0) / 16.0, 0.0, 1.0);
            let wave_height = clamp(abs(get_resynthesized_wave(i, hit_x_norm)) * 2.0, 0.0, 1.0);
            let line_amber = mix(amber_lo, amber_hi, wave_height);
            
            accumulated_color += line_amber * (core + bloom) * depth_fade * edge_fade;
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
