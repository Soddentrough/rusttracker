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
    let v = audio.channels[i / 4u];
    let c = i % 4u;
    if (c == 0u) { return v.x; } else if (c == 1u) { return v.y; }
    else if (c == 2u) { return v.z; } else { return v.w; }
}

struct ChannelMap {
    front_l: f32,
    front_r: f32,
    center: f32,
    lfe: f32,
    surr_l: f32,
    surr_r: f32,
    rear_l: f32,
    rear_r: f32,
};

fn get_channels() -> ChannelMap {
    var m: ChannelMap;
    let n = audio.num_channels;
    if (n <= 2u) {
        let l = clamp(get_vu(0u), 0.0, 1.0);
        let r = clamp(get_vu(min(1u, n - 1u)), 0.0, 1.0);
        m.front_l = l; m.front_r = r;
        m.center = (l + r) * 0.5;
        m.lfe = (l + r) * 0.3;
        m.surr_l = l * 0.5; m.surr_r = r * 0.5;
        m.rear_l = l * 0.3; m.rear_r = r * 0.3;
    } else if (n == 6u) {
        m.front_l = clamp(get_vu(1u), 0.0, 1.0);
        m.front_r = clamp(get_vu(4u), 0.0, 1.0);
        m.center  = clamp(get_vu(2u), 0.0, 1.0);
        m.lfe     = clamp(get_vu(3u), 0.0, 1.0);
        m.surr_l  = clamp(get_vu(0u), 0.0, 1.0);
        m.surr_r  = clamp(get_vu(5u), 0.0, 1.0);
        m.rear_l  = m.surr_l * 0.5;
        m.rear_r  = m.surr_r * 0.5;
    } else {
        m.front_l = clamp(get_vu(2u), 0.0, 1.0);
        m.front_r = clamp(get_vu(5u), 0.0, 1.0);
        m.center  = clamp(get_vu(3u), 0.0, 1.0);
        m.lfe     = clamp(get_vu(4u), 0.0, 1.0);
        m.surr_l  = clamp(get_vu(1u), 0.0, 1.0);
        m.surr_r  = clamp(get_vu(6u), 0.0, 1.0);
        m.rear_l  = clamp(get_vu(0u), 0.0, 1.0);
        m.rear_r  = clamp(get_vu(min(7u, n - 1u)), 0.0, 1.0);
    }
    return m;
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3  = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn valueNoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(i + vec2<f32>(0.0,0.0)), hash21(i + vec2<f32>(1.0,0.0)), u.x),
               mix(hash21(i + vec2<f32>(0.0,1.0)), hash21(i + vec2<f32>(1.0,1.0)), u.x), u.y);
}

// Intersects a ray with the infinite corridor planes. Returns vec4(t, nx, ny, nz)
fn intersect_corridor(ro: vec3<f32>, rd: vec3<f32>) -> vec4<f32> {
    var t = 1000.0;
    var n = vec3<f32>(0.0);
    
    // Floor (y = -1.5)
    if (rd.y < -0.001) {
        let tp = (-1.5 - ro.y) / rd.y;
        if (tp > 0.0 && tp < t) { t = tp; n = vec3<f32>(0.0, 1.0, 0.0); }
    }
    // Ceiling (y = 3.0)
    if (rd.y > 0.001) {
        let tp = (3.0 - ro.y) / rd.y;
        if (tp > 0.0 && tp < t) { t = tp; n = vec3<f32>(0.0, -1.0, 0.0); }
    }
    // Left Wall (x = -4.5)
    if (rd.x < -0.001) {
        let tp = (-4.5 - ro.x) / rd.x;
        if (tp > 0.0 && tp < t) { t = tp; n = vec3<f32>(1.0, 0.0, 0.0); }
    }
    // Right Wall (x = 4.5)
    if (rd.x > 0.001) {
        let tp = (4.5 - ro.x) / rd.x;
        if (tp > 0.0 && tp < t) { t = tp; n = vec3<f32>(-1.0, 0.0, 0.0); }
    }
    
    return vec4<f32>(t, n);
}

// Calculate lighting and material at a hit point
fn calc_surface(p: vec3<f32>, n: vec3<f32>, ch: ChannelMap) -> vec3<f32> {
    let neon_red = vec3<f32>(1.0, 0.02, 0.01);
    let neon_core = vec3<f32>(1.0, 0.6, 0.4);
    
    var col = vec3<f32>(0.0);
    
    // --- WALLS ---
    if (abs(n.x) > 0.5) {
        let is_left = (n.x > 0.0);
        let uv = vec2<f32>(p.z, p.y);
        
        // Wall structure: repeats every 4 units. Pillars are 1.0 wide. Recesses are 3.0 wide.
        let local_z = fract(p.z / 4.0);
        let in_recess = (local_z > 0.1 && local_z < 0.9);
        
        // Dark metallic/concrete base
        let noise = valueNoise(uv * 10.0) * 0.05;
        var base_col = vec3<f32>(0.01 + noise);
        
        if (!in_recess) {
            // Pillar: darker, catches some edge light
            base_col *= 0.3;
        } else {
            // Recess: evaluate distance to the central neon tube
            let tube_local = abs(local_z - 0.5) * 4.0; // 0 at center, 1.6 at edges
            let tube_dist = length(vec2<f32>(tube_local, max(0.0, abs(p.y - 0.5) - 0.8)));
            
            // Tube intensity driven by front channels
            let intensity_z_blend = smoothstep(2.0, 15.0, p.z);
            var intensity = 0.0;
            if (is_left) {
                intensity = mix(ch.front_l, ch.surr_l, intensity_z_blend);
            } else {
                intensity = mix(ch.front_r, ch.surr_r, intensity_z_blend);
            }
            
            // Glow math
            let core = exp(-tube_dist * 40.0) * intensity * 2.0;
            let bloom = exp(-tube_dist * 3.0) * intensity * 1.5;
            
            col += neon_core * core + neon_red * bloom;
        }
        
        // Add procedural bump roughness
        let rough = valueNoise(uv * 20.0) * 0.2;
        col += base_col * (1.0 - rough);
    }
    
    // --- CEILING ---
    else if (n.y < -0.5) {
        let uv = vec2<f32>(p.x, p.z);
        let local_z = fract(p.z / 4.0);
        let is_beam = (local_z < 0.2); // Beams are 0.8 wide
        
        let noise = valueNoise(uv * 15.0) * 0.03;
        var base_col = vec3<f32>(0.005 + noise);
        
        if (is_beam) {
            base_col *= 0.2; // Beams are dark
        } else {
            // Recessed ceiling panel glow (driven by center)
            let panel_dist = max(abs(p.x) - 1.5, abs(local_z - 0.6) * 4.0 - 0.8);
            let glow = exp(-max(panel_dist, 0.0) * 5.0) * ch.center;
            col += neon_red * glow * 1.0;
        }
        
        col += base_col;
    }
    
    // --- FLOOR ---
    else if (n.y > 0.5) {
        // Dark, glossy floor
        let noise = valueNoise(p.xz * 15.0) * 0.02;
        col += vec3<f32>(0.002 + noise);
        
        // Floor Lasers (driven by surround/rear)
        let x_bias = smoothstep(-0.5, 0.5, p.x / 4.5);
        let surr_i = mix(ch.surr_l, ch.surr_r, x_bias);
        let rear_i = mix(ch.rear_l, ch.rear_r, x_bias);
        
        // Grid pattern
        let grid_z = abs(fract(p.z / 2.0 + 0.5) - 0.5) * 2.0;
        let grid_x = abs(fract(p.x / 2.0 + 0.5) - 0.5) * 2.0;
        
        let lz = exp(-grid_z * 40.0) * surr_i;
        let lx = exp(-grid_x * 40.0) * max(rear_i, surr_i * 0.3);
        
        col += neon_red * max(lz, lx) * 1.5;
        col += neon_core * (lz * lx) * 1.0;
    }
    
    return col;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let aspect = dpdx(in.uv.x) / dpdy(in.uv.y);
    let p = vec2<f32>(uv.x * aspect, uv.y);
    let ch = get_channels();

    // Camera
    let ro = vec3<f32>(0.0, 0.3, 0.0);
    let rd = normalize(vec3<f32>(p.x, p.y, 0.85));

    var col = vec3<f32>(0.0);

    // Primary Intersection
    let hit = intersect_corridor(ro, rd);
    let t = hit.x;
    let n = hit.yzw;

    if (t < 900.0) {
        let p_hit = ro + rd * t;
        
        // Calculate primary surface lighting
        col = calc_surface(p_hit, n, ch);
        
        // Floor Reflection
        if (n.y > 0.5) {
            let ref_rd = reflect(rd, vec3<f32>(0.0, 1.0, 0.0));
            // Tiny offset to prevent self-intersection
            let ref_hit = intersect_corridor(p_hit + ref_rd * 0.01, ref_rd);
            if (ref_hit.x < 900.0) {
                let p_ref = p_hit + ref_rd * ref_hit.x;
                let ref_col = calc_surface(p_ref, ref_hit.yzw, ch);
                
                // Fresnel reflection factor
                let fresnel = pow(1.0 - max(0.0, dot(vec3<f32>(0.0, 1.0, 0.0), -rd)), 2.0);
                let reflectivity = mix(0.1, 0.6, fresnel);
                
                col = mix(col, ref_col, reflectivity);
            }
        }
    }

    // Depth Fog
    let fog_col = vec3<f32>(0.02, 0.001, 0.0008);
    let fog = 1.0 - exp(-t * 0.025);
    col = mix(col, fog_col, fog);

    // LFE Ambient Pulse
    col += vec3<f32>(1.0, 0.02, 0.0) * (ch.lfe * ch.lfe) * 0.02;

    // Vignette
    let vr = length(uv);
    col *= smoothstep(1.9, 0.5, vr);

    // ACES Tonemapping
    col = (col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14);

    // Gamma
    col = pow(col, vec3<f32>(1.0 / 2.2));

    return vec4<f32>(col, 1.0);
}
