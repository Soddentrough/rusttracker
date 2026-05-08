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
    num_channels: u32,
    mode: u32,
    time: f32,
    duration: f32,
    smooth_time: f32,
    heatmap_row: u32,
    _pad2: u32,
    _pad3: u32,
    ui_meters_rect: vec4<f32>,
    ui_heatmap_rect: vec4<f32>,
    ui_fire_rect: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

fn get_vu(i: u32) -> f32 {
    let n = max(1u, audio.num_channels);
    let idx = min(i, n - 1u);
    let v = audio.channels[idx / 4u];
    let c = idx % 4u;
    if (c == 0u) { return v.x; } else if (c == 1u) { return v.y; }
    else if (c == 2u) { return v.z; } else { return v.w; }
}

fn hash(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }

fn noise(x: vec3<f32>) -> f32 {
    let p = floor(x);
    let f = fract(x);
    let f2 = f * f * (3.0 - 2.0 * f);
    let n = p.x + p.y * 57.0 + 113.0 * p.z;
    return mix(
        mix(mix(hash(n + 0.0), hash(n + 1.0), f2.x),
            mix(hash(n + 57.0), hash(n + 58.0), f2.x), f2.y),
        mix(mix(hash(n + 113.0), hash(n + 114.0), f2.x),
            mix(hash(n + 170.0), hash(n + 171.0), f2.x), f2.y), f2.z
    );
}

fn fbm(p_in: vec3<f32>) -> f32 {
    var p = p_in;
    var f = 0.0;
    var amp = 0.5;
    for(var i=0; i<5; i++) {
        f += amp * noise(p);
        p *= 2.1;
        amp *= 0.5;
    }
    return f;
}

fn sdBox(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn sdFrame(p: vec3<f32>, w: f32, h: f32, t: f32) -> f32 {
    let b1 = sdBox(p - vec3<f32>(0.0, h, 0.0), vec3<f32>(w, t, t));
    let b2 = sdBox(p - vec3<f32>(0.0, -h, 0.0), vec3<f32>(w, t, t));
    let b3 = sdBox(p - vec3<f32>(-w, 0.0, 0.0), vec3<f32>(t, h+t, t));
    let b4 = sdBox(p - vec3<f32>(w, 0.0, 0.0), vec3<f32>(t, h+t, t));
    return min(min(b1, b2), min(b3, b4));
}

fn map_scene(p: vec3<f32>) -> vec2<f32> {
    let d_floor = p.y + 1.0;
    let d_wall = 10.0 - p.z; // Pushed far back into darkness
    let d_ceil = 5.0 - p.y;
    let d_left = p.x + 8.0;
    let d_right = 8.0 - p.x;
    
    var d_room = min(d_floor, d_wall);
    d_room = min(d_room, min(d_ceil, min(d_left, d_right)));
    
    // Taller frames (2.5 half-height = 5.0 total height)
    let d_frameL = sdFrame(p - vec3<f32>(-2.0, 1.5, 2.0), 0.6, 2.5, 0.08);
    let d_frameR = sdFrame(p - vec3<f32>(2.0, 1.5, 2.0), 0.6, 2.5, 0.08);
    let d_frames = min(d_frameL, d_frameR);
    
    if (d_frames < d_room) {
        if (d_frameL < d_frameR) { return vec2<f32>(d_frameL, 3.0); }
        return vec2<f32>(d_frameR, 4.0);
    }
    
    if (d_room == d_floor) { return vec2<f32>(d_floor, 1.0); }
    return vec2<f32>(d_room, 2.0);
}

fn calc_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.01, 0.0);
    return normalize(vec3<f32>(
        map_scene(p + e.xyy).x - map_scene(p - e.xyy).x,
        map_scene(p + e.yxy).x - map_scene(p - e.yxy).x,
        map_scene(p + e.yyx).x - map_scene(p - e.yyx).x
    ));
}

fn get_smoke_density(p: vec3<f32>, time: f32, audio_activity: f32) -> f32 {
    let center = vec3<f32>(0.0, -0.5, 2.0);
    let dist = length(p - center);
    
    // Smoother bounding mask to avoid visible spherical borders
    var mask = smoothstep(6.0, 0.0, dist);
    mask *= smoothstep(3.0, -1.0, p.y);
    
    if (mask < 0.01) { return 0.0; }
    
    let np = p * 1.5 - vec3<f32>(0.0, time * 0.2, time * 0.1);
    let n = fbm(np);
    
    // Higher threshold (0.45) to create distinct wispy chunks
    var dens = (n - 0.45) * (4.0 + audio_activity * 5.0);
    return max(dens, 0.0) * mask;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    
    var aspect = 1.7777;
    let dy = abs(dpdy(in.uv.y));
    let dx = abs(dpdx(in.uv.x));
    if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }
    
    let p = vec2<f32>(uv.x * aspect, -uv.y);
    
    let ro = vec3<f32>(0.0, 0.0, -4.0);
    let look_at = vec3<f32>(sin(audio.time * 0.2) * 0.1, 0.5 + cos(audio.time * 0.3) * 0.05, 2.0);
    
    let cw = normalize(look_at - ro);
    let cu = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), cw)); 
    let cv = normalize(cross(cw, cu)); 
    
    let rd = normalize(p.x * cu + p.y * cv + 1.2 * cw);
    
    let aL = clamp(get_vu(0u), 0.0, 1.0);
    let aR = clamp(get_vu(min(1u, audio.num_channels - 1u)), 0.0, 1.0);
    
    let col_red = vec3<f32>(1.0, 0.05, 0.1);
    let col_blue = vec3<f32>(0.05, 0.4, 1.0);
    
    let lightL = col_red * (2.0 + aL * 6.0);
    let lightR = col_blue * (2.0 + aR * 6.0);
    let act = (aL + aR) * 0.5;

    var t = 0.0;
    var T = 1.0;
    var color = vec3<f32>(0.0);
    var neon_glow = vec3<f32>(0.0);
    
    for(var i=0; i<100; i++) {
        let p_hit = ro + rd * t;
        let res = map_scene(p_hit);
        let d = res.x;
        
        // Accumulate smooth bloom from both frames constantly
        // This completely removes the sharp Voronoi partition line
        let d_frameL = sdFrame(p_hit - vec3<f32>(-2.0, 1.5, 2.0), 0.6, 2.5, 0.08);
        let d_frameR = sdFrame(p_hit - vec3<f32>(2.0, 1.5, 2.0), 0.6, 2.5, 0.08);
        
        neon_glow += lightL * 0.03 / (1.0 + d_frameL * d_frameL * 20.0);
        neon_glow += lightR * 0.03 / (1.0 + d_frameR * d_frameR * 20.0);
        
        if (d < 0.01) {
            var surf_col = vec3<f32>(0.0);
            
            // Solid white cores
            if (res.y == 3.0) { surf_col = vec3<f32>(3.0) + lightL * 1.5; }
            else if (res.y == 4.0) { surf_col = vec3<f32>(3.0) + lightR * 1.5; }
            else {
                let n = calc_normal(p_hit);
                
                let dirL = vec3<f32>(-2.0, 1.5, 2.0) - p_hit;
                let dirR = vec3<f32>(2.0, 1.5, 2.0) - p_hit;
                let distL = length(dirL);
                let distR = length(dirR);
                
                // Extremely dark room, quadratic light falloff
                let diffL = max(dot(n, dirL/distL), 0.0) / (distL * distL * 1.5 + 1.0);
                let diffR = max(dot(n, dirR/distR), 0.0) / (distR * distR * 1.5 + 1.0);
                
                let base_color = vec3<f32>(0.002);
                surf_col = base_color + lightL * diffL * 0.02 + lightR * diffR * 0.02;
                
                if (res.y == 1.0) {
                    let r_rd = reflect(rd, n);
                    
                    // Subtle distortion
                    let bump = (noise(vec3<f32>(p_hit.x * 6.0, 0.0, p_hit.z * 6.0)) - 0.5) * 0.02;
                    let r_rd_b = normalize(r_rd + vec3<f32>(bump, 0.0, bump));
                    
                    var ref_col = vec3<f32>(0.0);
                    var rt = 0.05;
                    var min_d_L = 100.0;
                    var min_d_R = 100.0;
                    
                    for(var j=0; j<25; j++) {
                        let rp = p_hit + r_rd_b * rt;
                        let dL = sdFrame(rp - vec3<f32>(-2.0, 1.5, 2.0), 0.6, 2.5, 0.08);
                        let dR = sdFrame(rp - vec3<f32>(2.0, 1.5, 2.0), 0.6, 2.5, 0.08);
                        
                        min_d_L = min(min_d_L, dL);
                        min_d_R = min(min_d_R, dR);
                        
                        let d_min = min(dL, dR);
                        if (d_min < 0.01) { break; }
                        
                        rt += d_min;
                        if (rt > 12.0 || rp.y > 4.0) { break; }
                    }
                    
                    // Smooth, glowing reflection bleed
                    ref_col += lightL * 0.8 / (1.0 + min_d_L * min_d_L * 15.0);
                    ref_col += lightR * 0.8 / (1.0 + min_d_R * min_d_R * 15.0);
                    if (min_d_L < 0.01) { ref_col += vec3<f32>(2.0) + lightL; }
                    if (min_d_R < 0.01) { ref_col += vec3<f32>(2.0) + lightR; }
                    
                    let f = pow(1.0 - max(dot(n, -rd), 0.0), 3.0);
                    let reflectivity = mix(0.15, 0.8, f);
                    surf_col += ref_col * reflectivity;
                }
            }
            
            color += T * surf_col;
            break;
        }
        
        let dens = get_smoke_density(p_hit, audio.time, act);
        if (dens > 0.0) {
            let d_lightL = length(p_hit - vec3<f32>(-2.0, 1.5, 2.0));
            let d_lightR = length(p_hit - vec3<f32>(2.0, 1.5, 2.0));
            
            let attenL = exp(-d_lightL * 0.7);
            let attenR = exp(-d_lightR * 0.7);
            
            // Absolutely NO ambient light. Smoke is pitch black if not illuminated.
            let smoke_light = (lightL * attenL + lightR * attenR) * 2.5;
            
            let step_len = 0.1;
            let alpha = 1.0 - exp(-dens * step_len * 4.0);
            
            color += T * alpha * smoke_light;
            T *= (1.0 - alpha);
            
            if (T < 0.01) { break; }
        }
        
        var step_size = d;
        let d_smoke_box = length(p_hit - vec3<f32>(0.0, -0.5, 2.0)) - 5.5;
        
        if (d_smoke_box < 0.0 && T > 0.01) {
            step_size = min(step_size, 0.1);
        } else {
            step_size = min(step_size, d_smoke_box + 0.05);
        }
        
        t += max(step_size, 0.02);
        if (t > 20.0) { break; }
    }
    
    color += neon_glow * T;
    
    let vr = length(uv);
    color *= smoothstep(2.5, 0.5, vr);
    
    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    // Output Linear RGB. WGPU Srgb surface will apply the sRGB gamma curve automatically.
    final_col = max(final_col, vec3<f32>(0.0));
    
    return vec4<f32>(final_col, 1.0);
}
