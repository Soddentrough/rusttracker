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

// Hash function for analog noise
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn get_fft_amplitude(line_idx: u32, num_lines: u32) -> f32 {
    // Ensure every single line gets a strictly unique frequency bin
    // Maps line 0 to bin 1 (bass), up to line 31 to bin ~132 (treble)
    let non_linear = u32(pow(f32(line_idx) / f32(num_lines - 1u), 2.0) * 100.0);
    let bin_idx = line_idx + non_linear + 1u;
    
    let vec_idx = bin_idx / 4u;
    let comp_idx = bin_idx % 4u;
    
    let spec = audio.spectrum[vec_idx];
    var amp = 0.0;
    if comp_idx == 0u { amp = spec.x; }
    else if comp_idx == 1u { amp = spec.y; }
    else if comp_idx == 2u { amp = spec.z; }
    else { amp = spec.w; }
    
    // Smooth the amplitude over time a bit to avoid jitter
    return min(amp * 1.5, 2.0);
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

    var accumulated_intensity = 0.0;

    let num_lines = 32u;
    let num_points = 512u; // Increased resolution for wider lines
    
    for (var i = 0u; i < num_lines; i = i + 1u) {
        // Z layout: back to front.
        // Maintain same 0.24 spacing: 31 * 0.24 = 7.44 span. Front remains -1.8, back becomes 5.64.
        let y_line = mix(5.64, -1.8, f32(i) / f32(num_lines - 1u));
        
        // 1. Get the real FFT amplitude for this specific frequency band
        let fft_amp = get_fft_amplitude(i, num_lines);
        
        // 2. Calculate the visual frequency of this band (from 1 cycle to ~32 cycles)
        let cycles = pow(1.25, f32(i) * (16.0 / f32(num_lines))); 
        let phase = audio.time * cycles * 2.0;
        
        let t = (y_line - ro.y) / rd.y;
        if t > 0.0 {
            let hit_x = ro.x + rd.x * t;
            
            // Map X from -8.0 to 8.0
            let float_idx = (hit_x + 8.0) / 16.0 * f32(num_points - 1u);
            let idx = i32(round(float_idx));
            
            let start_idx = max(0, idx - 12);
            let end_idx = min(i32(num_points) - 2, idx + 12);
            
            var min_dist = 1000.0;
            
            for (var j = start_idx; j <= end_idx; j = j + 1) {
                let j_u = u32(j);
                
                let x0 = mix(-8.0, 8.0, f32(j_u) / f32(num_points - 1u));
                let x1 = mix(-8.0, 8.0, f32(j_u + 1u) / f32(num_points - 1u));
                
                // Falloff mask so lines flatten out perfectly at the left/right edges
                let mask0 = smoothstep(8.0, 5.0, abs(x0));
                let mask1 = smoothstep(8.0, 5.0, abs(x1));
                
                let x_norm0 = f32(j_u) / f32(num_points - 1u);
                let x_norm1 = f32(j_u + 1u) / f32(num_points - 1u);
                
                // 3. Synthesize the bandpassed waveform!
                let v0 = sin(x_norm0 * cycles * 6.28318 - phase) * fft_amp;
                let v1 = sin(x_norm1 * cycles * 6.28318 - phase) * fft_amp;
                
                // Only go UP (positive Z) from the baseline. 
                // We use abs() so both positive and negative waveform phases create peaks
                let p0 = abs(v0) * mask0 * 0.8;
                let p1 = abs(v1) * mask1 * 0.8;
                
                let p3_0 = vec3<f32>(x0, y_line, p0);
                let p3_1 = vec3<f32>(x1, y_line, p1);
                
                let proj0 = project_3d(p3_0, ro, u, v_cam, w);
                let proj1 = project_3d(p3_1, ro, u, v_cam, w);
                
                if proj0.z > 0.001 && proj1.z > 0.001 {
                    let d = sd_segment(p, proj0.xy, proj1.xy);
                    min_dist = min(min_dist, d);
                }
            }
            
            let r = length(crt_uv);
            let edge_blur = smoothstep(0.3, 1.2, r);
            
            let thickness = 0.002 + edge_blur * 0.004;
            let core = smoothstep(thickness, 0.0, min_dist);
            // High bloom for that glowing CRT look
            let bloom = 0.0004 / (min_dist * min_dist + 0.0001) * 0.15;
            
            // Depth fade (lines further back are slightly darker)
            let depth_fade = exp(-t * 0.35);
            let edge_fade = smoothstep(8.0, 5.0, abs(hit_x));
            
            // Age fade (older lines fade out)
            let age_fade = mix(0.05, 1.0, f32(i) / f32(num_lines - 1u));
            
            accumulated_intensity += (core + bloom) * depth_fade * edge_fade * age_fade;
        }
    }
    
    let amber = vec3<f32>(1.0, 0.45, 0.05);
    let mapped = accumulated_intensity * amber;
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
    let noise_color = amber * noise_val * 0.02 * bezel * (0.3 + 0.7 * clamp(accumulated_intensity * 0.5, 0.0, 1.0));
    
    final_color = final_color * bezel + noise_color;
    
    return vec4<f32>(final_color, 1.0);
}
