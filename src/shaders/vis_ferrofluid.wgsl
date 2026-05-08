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

struct MapData {
    d: f32,
    mat_id: i32,
    glow: vec3<f32>,
}

fn smax(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (a - b) / k, 0.0, 1.0);
    return mix(b, a, h) + k * h * (1.0 - h);
}

fn map(p: vec3<f32>) -> MapData {
    let puddle_radius = 4.0;
    let dist_xz = length(p.xz);
    
    // Base puddle thickness
    var fluid_h = 0.1 * smoothstep(puddle_radius, puddle_radius * 0.5, dist_xz);
    var glow = vec3<f32>(0.0);

    var speaker_dirs = array<vec3<f32>, 12>(
        vec3<f32>(-0.5, 0.0, -0.866), // L
        vec3<f32>(0.5, 0.0, -0.866),  // R
        vec3<f32>(0.0, 0.0, -1.0),    // C
        vec3<f32>(0.0, 0.0, 0.0),     // LFE (Center blob)
        vec3<f32>(-0.94, 0.0, 0.34),  // Ls
        vec3<f32>(0.94, 0.0, 0.34),   // Rs
        vec3<f32>(-0.5, 0.0, 0.866),  // Lrs
        vec3<f32>(0.5, 0.0, 0.866),   // Rrs
        vec3<f32>(-0.7, 0.0, -0.7),   // Ltf
        vec3<f32>(0.7, 0.0, -0.7),    // Rtf
        vec3<f32>(-0.7, 0.0, 0.7),    // Ltr
        vec3<f32>(0.7, 0.0, 0.7)      // Rtr
    );
    
    let num_ch = min(audio.num_channels, 12u);
    var total_displacement = 0.0;
    
    // Normalized xz for angle alignment
    let p_xz_norm = normalize(p.xz + vec2<f32>(0.001)); // avoid div zero
    
    for (var i = 0u; i < num_ch; i++) {
        let vec_idx = i / 4u;
        let comp_idx = i % 4u;
        var vu = 0.0;
        if comp_idx == 0u { vu = audio.channels[vec_idx].x; }
        else if comp_idx == 1u { vu = audio.channels[vec_idx].y; }
        else if comp_idx == 2u { vu = audio.channels[vec_idx].z; }
        else { vu = audio.channels[vec_idx].w; }
        vu = clamp(vu, 0.0, 1.0);
        
        var alignment = 1.0;
        var spike_pos_r = 1.5;
        
        if i == 3u { // LFE channel in center
            alignment = 1.0;
            spike_pos_r = 0.0;
        } else {
            let dir2d = normalize(speaker_dirs[i].xz);
            alignment = max(0.0, dot(p_xz_norm, dir2d));
            spike_pos_r = 1.5;
        }
        
        let dist_to_spike = abs(dist_xz - spike_pos_r);
        let spatial_falloff = exp(-dist_to_spike * 3.0);
        
        let lobe = pow(alignment, 12.0) * vu * 1.5 * spatial_falloff;
        total_displacement = smax(total_displacement, lobe, 0.3);
        
        // Inner glow for spikes
        if lobe > 0.1 {
            var ch_color = vec3<f32>(0.2, 0.6, 1.0);
            if i < 3u { ch_color = vec3<f32>(1.0, 0.4, 0.1); }
            if i == 3u { ch_color = vec3<f32>(1.0, 0.1, 0.2); }
            glow += ch_color * pow(lobe, 2.0) * 2.0;
        }
    }
    
    // Add subtle ripples from spectrum bass
    let bass = audio.spectrum[0].x + audio.spectrum[1].x;
    let ripple = sin(dist_xz * 12.0 - audio.time * 8.0) * 0.015 * bass * smoothstep(puddle_radius, 0.0, dist_xz);
    
    fluid_h += total_displacement + ripple;
    
    let d = p.y + 0.5 - fluid_h;
    
    return MapData(d * 0.6, 1, glow);
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
    
    // Fix aspect ratio and INVERT Y axis so we aren't upside down!
    let aspect = dpdy(in.uv.y) / dpdx(in.uv.x);
    let p = vec2<f32>(uv.x * abs(aspect), -uv.y);
    
    // Camera looking slightly down at the puddle
    let ro = vec3<f32>(0.0, 2.5, 4.0);
    let cam_target = vec3<f32>(0.0, -0.5, 0.0);
    
    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = normalize(cross(uu, ww));
    
    let fov = 1.0;
    let rd = normalize(p.x * uu + p.y * vv + fov * ww);
    
    var col = vec3<f32>(0.0);
    var glow = vec3<f32>(0.0);
    
    var t = 0.0;
    let max_t = 30.0;
    var hit = false;
    var final_p = vec3<f32>(0.0);
    
    for (var i = 0; i < 100; i++) {
        let p_current = ro + rd * t;
        let map_data = map(p_current);
        let d = map_data.d;
        
        glow += map_data.glow * 0.05 / (1.0 + abs(d) * 20.0);
        
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
        
        // Fluid transition based on height
        let is_fluid = smoothstep(-0.495, -0.47, final_p.y);
        
        // Floor Material (Pure White Studio)
        // No diffuse shading to keep it looking like a pure white void/backdrop
        let floor_ao = smoothstep(1.0, 2.5, length(final_p.xz)); // subtle shadow under puddle
        let final_floor = vec3<f32>(5.0) * (0.8 + 0.2 * floor_ao); // Boosted to blow out to pure white after tonemapping
        
        // Fluid Material (Pure Black Liquid with Highlights)
        let ref_dir = reflect(rd, n);
        let fresnel = pow(1.0 - max(0.0, dot(n, -rd)), 5.0);
        
        // Sharp specular highlights mimicking studio softboxes
        let light1 = pow(max(0.0, dot(ref_dir, normalize(vec3<f32>(1.0, 1.5, 1.0)))), 64.0);
        let light2 = pow(max(0.0, dot(ref_dir, normalize(vec3<f32>(-1.0, 1.0, -1.0)))), 32.0);
        let rim_light = smoothstep(0.7, 1.0, max(0.0, ref_dir.y));
        
        var fluid_ref = vec3<f32>(0.0); // Pure black base reflection
        fluid_ref += vec3<f32>(5.0) * light1 * 2.0; // Intense white highlight
        fluid_ref += vec3<f32>(3.0) * light2;       // Intense white highlight
        fluid_ref += vec3<f32>(1.0) * rim_light;    // Rim light from the white environment
        
        let fluid_col = mix(vec3<f32>(0.0), fluid_ref, 0.2 + 0.8 * fresnel);
        
        col = mix(final_floor, fluid_col, is_fluid);
    } else {
        // Background color (Pure white void)
        col = vec3<f32>(5.0);
    }
    
    // Fade out to pure white environment to hide the sharp horizon
    col = mix(col, vec3<f32>(5.0), smoothstep(15.0, 30.0, t));
    
    col += glow;
    
    // Vignette
    let vignette = 1.0 - smoothstep(0.5, 1.5, length(uv));
    col *= vignette;
    
    // Tone mapping
    col = (col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14);
    
    return vec4<f32>(pow(col, vec3<f32>(1.0 / 2.2)), 1.0);
}
