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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);
    let p = vec2<f32>(uv.x * aspect, uv.y);
    let r = length(p);
    
    var color = vec3<f32>(0.02, 0.02, 0.03); // base dark background
    
    // Radar rings
    let ring1 = smoothstep(0.01, 0.0, abs(r - 0.4));
    let ring2 = smoothstep(0.01, 0.0, abs(r - 0.8));
    color = color + vec3<f32>(0.05, 0.1, 0.1) * (ring1 + ring2);
    
    // Standard Dolby Angles for up to 12 channels
    var speaker_angles = array<vec2<f32>, 12>(
        vec2<f32>(-30.0, 0.8),   // L
        vec2<f32>(30.0, 0.8),    // R
        vec2<f32>(0.0, 0.8),     // C
        vec2<f32>(0.0, 0.0),     // LFE
        vec2<f32>(-110.0, 0.8),  // Ls
        vec2<f32>(110.0, 0.8),   // Rs
        vec2<f32>(-150.0, 0.8),  // Lrs
        vec2<f32>(150.0, 0.8),   // Rrs
        vec2<f32>(-45.0, 0.4),   // Ltf
        vec2<f32>(45.0, 0.4),    // Rtf
        vec2<f32>(-135.0, 0.4),  // Ltr
        vec2<f32>(135.0, 0.4)    // Rtr
    );
    
    let num_ch = min(audio.num_channels, 12u);
    
    for (var i = 0u; i < num_ch; i = i + 1u) {
        var angle_deg = 0.0;
        var radius = 0.8;
        if audio.num_channels == 2u || audio.num_channels == 6u || audio.num_channels == 8u || audio.num_channels == 12u {
            angle_deg = speaker_angles[i].x;
            radius = speaker_angles[i].y;
        } else {
            angle_deg = (f32(i) / f32(num_ch)) * 360.0;
        }
        
        let angle_rad = radians(angle_deg);
        
        let pos_x = sin(angle_rad) * radius;
        let pos_y = -cos(angle_rad) * radius; // negative Y is UP (Front)
        let pos = vec2<f32>(pos_x, pos_y);
        
        let dist = length(p - pos);
        
        // Get VU
        let vec_idx = i / 4u;
        let component_idx = i % 4u;
        var vu = 0.0;
        let ch_vec = audio.channels[vec_idx];
        if component_idx == 0u { vu = ch_vec.x; }
        else if component_idx == 1u { vu = ch_vec.y; }
        else if component_idx == 2u { vu = ch_vec.z; }
        else { vu = ch_vec.w; }
        
        vu = clamp(vu, 0.0, 1.0);
        
        // Draw speaker dot
        let dot_size = 0.02 + vu * 0.05;
        let dot_intensity = smoothstep(dot_size, 0.0, dist);
        
        // Draw ripple
        let time_offset = audio.time * 10.0;
        let ripple = sin(dist * 30.0 - time_offset) * 0.5 + 0.5;
        let ripple_mask = smoothstep(0.4, 0.0, dist) * vu * 0.5;
        
        var ch_color = vec3<f32>(0.2, 0.8, 1.0); // Surround blue
        if i < 3u { ch_color = vec3<f32>(1.0, 0.6, 0.1); } // Fronts orange
        if i == 3u { ch_color = vec3<f32>(1.0, 0.1, 0.3); } // LFE Red
        
        color = color + ch_color * dot_intensity * 2.0;
        color = color + ch_color * ripple * ripple_mask;
    }
    
    return vec4<f32>(color, 1.0);
}
