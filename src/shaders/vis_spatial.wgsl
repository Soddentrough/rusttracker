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

fn rotate2d(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
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
    return vec3<f32>(proj_x, -proj_y, dist_w);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);
    
    // Perspective field of view scale (smaller = zoom in)
    let p = vec2<f32>(uv.x * aspect, uv.y) * 0.55;
    
    // True 3D Camera Setup
    let cam_dist = 1.8;
    let cam_height = 0.9;
    let ro = vec3<f32>(0.0, -cam_dist, cam_height);
    let cam_target = vec3<f32>(0.0, 0.2, 0.2); // Look slightly into the room and up
    
    let w = normalize(cam_target - ro);
    let u = normalize(cross(w, vec3<f32>(0.0, 0.0, 1.0)));
    let v_cam = cross(u, w);
    
    let rd = normalize(w + p.x * u - p.y * v_cam);
    
    // Static room orientation as requested
    let room_angle = 0.0;
    
    // Base dark background
    var color = vec3<f32>(0.015, 0.015, 0.02); 
    
    // Raytraced Perspective Floor
    var floor_xy = vec2<f32>(0.0, 0.0);
    if rd.z < 0.0 {
        let t = -ro.z / rd.z;
        let hit = ro + t * rd;
        floor_xy = rotate2d(hit.xy, -room_angle);
        
        // Rectangular tile grid instead of radar rings to match physical room
        let cell_size = 0.4;
        let grid_x = smoothstep(0.015, 0.0, abs(fract(floor_xy.x / cell_size + 0.5) - 0.5) * cell_size);
        let grid_y = smoothstep(0.015, 0.0, abs(fract(floor_xy.y / cell_size + 0.5) - 0.5) * cell_size);
        let grid_lines = max(grid_x, grid_y) * 0.4;
        
        // Perspective distance fade for floor
        let grid_mask = 1.0 - smoothstep(1.0, 4.0, t);
        
        color = color + vec3<f32>(0.04, 0.07, 0.1) * grid_lines * grid_mask;
        
        // Ambient floor glow reacting to bass
        let bass = clamp(audio.spectrum[1].x * 2.0 + audio.spectrum[2].x, 0.0, 1.0);
        let r_floor = length(floor_xy);
        color = color + vec3<f32>(0.05, 0.02, 0.08) * bass * smoothstep(3.0, 0.0, r_floor);
    }

    // Dolby Atmos 7.1.4 Rectangular Room Layout (X, Y, Z)
    // X = Left/Right, Y = Front/Back (+Y is front wall), Z = Height
    var speaker_data = array<vec3<f32>, 12>(
        vec3<f32>(-1.0, 1.2, 0.0),    // 0: L (Front Left)
        vec3<f32>(1.0, 1.2, 0.0),     // 1: R (Front Right)
        vec3<f32>(0.0, 1.2, 0.0),     // 2: C (Center)
        vec3<f32>(-0.4, 1.2, 0.0),    // 3: LFE (Subwoofer next to center)
        vec3<f32>(-1.2, 0.0, 0.0),    // 4: Ls (Left Surround, beside listener)
        vec3<f32>(1.2, 0.0, 0.0),     // 5: Rs (Right Surround, beside listener)
        vec3<f32>(-0.8, -0.8, 0.0),   // 6: Lrs (Left Rear Surround)
        vec3<f32>(0.8, -0.8, 0.0),    // 7: Rrs (Right Rear Surround)
        vec3<f32>(-0.8, 0.8, 0.7),    // 8: Ltf (Left Top Front)
        vec3<f32>(0.8, 0.8, 0.7),     // 9: Rtf (Right Top Front)
        vec3<f32>(-0.8, -0.4, 0.7),   // 10: Ltr (Left Top Rear)
        vec3<f32>(0.8, -0.4, 0.7)     // 11: Rtr (Right Top Rear)
    );
    
    // Only render the number of actual spatial speakers present in the audio file
    let render_channels = min(audio.num_spatial_channels, 12u);
    
    for (var i = 0u; i < render_channels; i = i + 1u) {
        let room_xy = vec2<f32>(speaker_data[i].x, speaker_data[i].y);
        var height = speaker_data[i].z;
        
        // Rotate to world coordinates
        let world_xy = rotate2d(room_xy, room_angle);
        
        // Get VU for this spatial channel
        var vu = 0.0;
        let vec_idx = i / 4u;
        let component_idx = i % 4u;
        let ch_vec = audio.spatial_channels[vec_idx];
        if component_idx == 0u { vu = ch_vec.x; }
            else if component_idx == 1u { vu = ch_vec.y; }
            else if component_idx == 2u { vu = ch_vec.z; }
            else { vu = ch_vec.w; }
        vu = clamp(vu, 0.0, 1.0);
        
        // Add physical "bounce" to the speaker based on its volume
        height = height + (vu * 0.08); 
        
        let p3_base = vec3<f32>(world_xy.x, world_xy.y, 0.0);
        let p3_speaker = vec3<f32>(world_xy.x, world_xy.y, height);
        
        // 3D Perspective Projection
        let proj_b = project_3d(p3_base, ro, u, v_cam, w);
        let proj_s = project_3d(p3_speaker, ro, u, v_cam, w);
        
        let proj_base_2d = proj_b.xy;
        let proj_speaker_2d = proj_s.xy;
        let depth = proj_s.z;
        
        // Color mapping
        var base_color = vec3<f32>(0.0, 0.8, 1.0); // Standard Bed Channels (Cyan)
        if i == 3u { base_color = vec3<f32>(1.0, 0.2, 0.4); } // LFE Pink
        if i >= 8u { base_color = vec3<f32>(1.0, 0.7, 0.1); } // Heights Gold
        
        // Depth cue: fade speakers that are further back from the camera
        let depth_fade = smoothstep(3.5, 1.5, depth); 
        let ch_color = base_color * (0.3 + 0.7 * depth_fade);
        
        // Dynamic floor ripple originating from the speaker's base
        if rd.z < 0.0 {
            let time_offset = audio.time * 8.0;
            let dist_room = length(floor_xy - room_xy); 
            let ripple = sin(dist_room * 25.0 - time_offset) * 0.5 + 0.5;
            let ripple_mask = smoothstep(0.6, 0.0, dist_room) * vu * 0.4;
            let grid_fade = 1.0 - smoothstep(1.0, 4.0, length(floor_xy));
            color = color + ch_color * ripple * ripple_mask * grid_fade;
        }
        
        // Perspective scaling for visual sizes
        let scale = 2.0 / max(depth, 0.1); 
        
        // Draw floor anchor for floating speakers (Drop shadow instead of stem)
        if height > 0.0 {
            // Technical anchor pad on the floor
            let dist_base = length(p - proj_base_2d);
            let base_core = smoothstep(0.03 * scale, 0.0, dist_base) * 0.4;
            // Draw a sharp outer ring for the anchor
            let base_ring = smoothstep(0.005 * scale, 0.0, abs(dist_base - 0.04 * scale)) * 0.4;
            
            color = color + ch_color * (base_core + base_ring);
        }
        
        // Draw main speaker orb
        let dist = length(p - proj_speaker_2d);
        
        // Base speaker size
        let dot_size = (0.03 + vu * 0.03) * scale; 
        
        // Solid bright core
        let core = smoothstep(dot_size, dot_size * 0.5, dist);
        
        // Tighter atmospheric glow to prevent washing out the screen
        let glow = smoothstep(dot_size * 2.5, 0.0, dist) * vu * 0.6;
        
        // Hard-edged shockwave ring closer to the speaker
        let shockwave = smoothstep(0.005 * scale, 0.0, abs(dist - (dot_size + (0.01 + vu * 0.04) * scale))) * (vu * vu);
        
        color = color + ch_color * core * 2.0;
        color = color + ch_color * glow;
        color = color + ch_color * shockwave;
    }
    
    // Vignette for cinematic feel
    let vignette = 1.0 - smoothstep(0.5, 2.0, length(uv));
    color = color * vignette;
    
    return vec4<f32>(color, 1.0);
}
