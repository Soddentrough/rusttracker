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
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
    ui_meters_rect: vec4<f32>,
    ui_heatmap_rect: vec4<f32>,
    ui_fire_rect: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

struct MapData {
    d: f32,
    mat_id: i32,
    glow: vec3<f32>,
}

fn get_vu(i: u32) -> f32 {
    let vec_idx = i / 4u;
    let comp_idx = i % 4u;
    var vu = 0.0;
    if comp_idx == 0u { vu = audio.channels[vec_idx].x; }
    else if comp_idx == 1u { vu = audio.channels[vec_idx].y; }
    else if comp_idx == 2u { vu = audio.channels[vec_idx].z; }
    else { vu = audio.channels[vec_idx].w; }
    return clamp(vu, 0.0, 1.0);
}

// Fixed camera Z
const CAM_Z: f32 = -4.5;

fn map(p: vec3<f32>) -> MapData {
    var total_glow = vec3<f32>(0.0);
    
    // 1. Frequency Pillars (Receding down the corridor)
    let spacing = 4.0;
    let local_z = (p.z % spacing + spacing) % spacing - spacing/2.0;
    let freq_global_z = p.z - local_z;
    
    // Only draw freq pillars if they are far away enough to not overlap with camera
    var d_freq = 999.0;
    if freq_global_z > -4.0 {
        let dist_to_cam = max(0.0, freq_global_z - CAM_Z);
        let freq_idx = min(u32(dist_to_cam / spacing), 63u);
        
        let vec_f_idx = freq_idx / 4u;
        let comp_f_idx = freq_idx % 4u;
        var freq_val = 0.0;
        if comp_f_idx == 0u { freq_val = audio.spectrum[vec_f_idx].x; }
        else if comp_f_idx == 1u { freq_val = audio.spectrum[vec_f_idx].y; }
        else if comp_f_idx == 2u { freq_val = audio.spectrum[vec_f_idx].z; }
        else { freq_val = audio.spectrum[vec_f_idx].w; }
        
        let pillar_r = 0.08;
        let d_freq_l = length(vec2<f32>(p.x + 4.5, local_z)) - pillar_r;
        let d_freq_r = length(vec2<f32>(p.x - 4.5, local_z)) - pillar_r;
        d_freq = min(d_freq_l, d_freq_r);
        
        let freq_glow_col = vec3<f32>(1.0, 0.02, 0.0) * freq_val * 4.0;
        total_glow += freq_glow_col * exp(-abs(d_freq) * 10.0);
    }
    
    var final_d = d_freq;
    
    // 2. Spatial Channel Pillars
    var speaker_pos = array<vec3<f32>, 12>(
        vec3<f32>(-4.5, 0.0, 6.0),  // L  (Far Front)
        vec3<f32>(4.5, 0.0, 6.0),   // R  (Far Front)
        vec3<f32>(0.0, 3.0, 6.0),   // C  (Ceiling Front)
        vec3<f32>(0.0, -3.0, 2.0),  // LFE (Floor Mid)
        vec3<f32>(-4.5, 0.0, 2.0),  // Ls  (Side Mid)
        vec3<f32>(4.5, 0.0, 2.0),   // Rs  (Side Mid)
        vec3<f32>(-4.5, 0.0, -2.0), // Lrs (Rear -> Foreground corners)
        vec3<f32>(4.5, 0.0, -2.0),  // Rrs (Rear -> Foreground corners)
        vec3<f32>(-3.0, 3.0, 6.0),  // Ltf (Ceiling Front)
        vec3<f32>(3.0, 3.0, 6.0),   // Rtf 
        vec3<f32>(-3.0, 3.0, -2.0), // Ltr (Ceiling Rear -> Foreground)
        vec3<f32>(3.0, 3.0, -2.0)   // Rtr 
    );
    
    let num_ch = min(audio.num_channels, 12u);
    for (var i = 0u; i < num_ch; i++) {
        let pos = speaker_pos[i];
        var d = 999.0;
        
        if i == 0u || i == 1u || i == 4u || i == 5u || i == 6u || i == 7u {
            // Wall Pillars
            d = length(p.xz - pos.xz) - 0.15;
        } else if i == 2u {
            // Center (Ceiling horizontal bar)
            let strip_d = length(vec2<f32>(p.y - 3.0, p.z - pos.z)) - 0.1;
            d = max(strip_d, abs(p.x) - 2.0);
        } else if i == 3u {
            // LFE (Floor center square/strip)
            let strip_d = length(vec2<f32>(p.y + 3.0, p.z - pos.z)) - 0.2;
            d = max(strip_d, abs(p.x) - 1.5);
        } else {
            // Heights (Ceiling small bars)
            let strip_d = length(vec2<f32>(p.y - 3.0, p.z - pos.z)) - 0.1;
            d = max(strip_d, abs(p.x - pos.x) - 0.5);
        }
        
        let vu = get_vu(i);
        // Intense red with slight orange/white core at high peaks
        let ch_glow_col = vec3<f32>(1.0, 0.1, 0.05) * vu * 6.0;
        total_glow += ch_glow_col * exp(-abs(d) * 12.0);
        
        final_d = min(final_d, d);
    }
    
    // 3. Black Glossy Floor
    let floor_d = p.y + 3.0;
    final_d = min(final_d, floor_d);
    
    return MapData(final_d, 0, total_glow);
}

fn calcNormal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.005, 0.0);
    return normalize(vec3<f32>(
        map(p + e.xyy).d - map(p - e.xyy).d,
        map(p + e.yxy).d - map(p - e.yxy).d,
        map(p + e.yyx).d - map(p - e.yyx).d
    ));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    
    // Aspect ratio and invert Y (so camera isn't upside down like ferrofluid was!)
    let aspect = dpdy(in.uv.y) / dpdx(in.uv.x);
    let p = vec2<f32>(uv.x * abs(aspect), -uv.y);
    
    let ro = vec3<f32>(0.0, 0.0, CAM_Z);
    let cam_target = ro + vec3<f32>(0.0, 0.0, 1.0);
    
    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = normalize(cross(uu, ww));
    
    // Wider FOV so user can see side pillars!
    let fov = 0.8; 
    let rd = normalize(p.x * uu + p.y * vv + fov * ww);
    
    var col = vec3<f32>(0.0);
    var glow = vec3<f32>(0.0);
    
    var t = 0.0;
    let max_t = 80.0;
    var hit = false;
    var final_p = vec3<f32>(0.0);
    
    // Primary Raymarch
    for (var i = 0; i < 90; i++) {
        let p_current = ro + rd * t;
        let map_data = map(p_current);
        let d = map_data.d;
        
        glow += map_data.glow * 0.02 / (1.0 + abs(d) * 15.0);
        
        if d < 0.005 {
            hit = true;
            final_p = p_current;
            break;
        }
        
        t += d;
        if t > max_t {
            break;
        }
    }
    
    if hit {
        let n = calcNormal(final_p);
        let base_col = vec3<f32>(0.0); // Pure black material
        
        if final_p.y < -2.99 {
            // Floor Mirror Reflection
            let ref_dir = reflect(rd, n);
            var ref_t = 0.1;
            var ref_glow = vec3<f32>(0.0);
            var ref_hit = false;
            
            // Secondary Raymarch
            for (var j = 0; j < 50; j++) {
                let ref_p = final_p + ref_dir * ref_t;
                let ref_data = map(ref_p);
                let d = ref_data.d;
                
                ref_glow += ref_data.glow * 0.015 / (1.0 + abs(d) * 15.0);
                
                if d < 0.01 {
                    ref_hit = true;
                    break;
                }
                ref_t += d;
                if ref_t > 40.0 { break; }
            }
            
            // Floor base color + reflection
            let fresnel = pow(1.0 - max(0.0, dot(n, -rd)), 3.0);
            col = ref_glow * mix(0.1, 0.4, fresnel);
        } else {
            // Physical pillars
            col = base_col;
        }
    }
    
    // Add primary volumetric glow
    col += glow;
    
    // Distance fog (pure black abyss)
    col = mix(col, vec3<f32>(0.0), smoothstep(40.0, max_t, t));
    
    // Tone mapping
    col = (col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14);
    
    return vec4<f32>(pow(col, vec3<f32>(1.0 / 2.2)), 1.0);
}
