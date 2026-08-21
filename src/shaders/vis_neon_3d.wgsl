// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(2) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
};

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) mat: f32,
    @location(4) local_pos: vec3<f32>,
};

fn channel_color(idx: u32) -> vec3<f32> {
    switch idx {
        case 0u { return vec3<f32>(1.0, 0.08, 0.18); } // Ch 0 (Left): Vivid Red
        case 1u { return vec3<f32>(0.08, 0.45, 1.0);  } // Ch 1 (Right): Vivid Blue
        case 2u { return vec3<f32>(1.0, 0.88, 0.65);  } // Ch 2 (Center): Warm Gold-White
        case 3u { return vec3<f32>(0.95, 0.05, 0.40); } // Ch 3 (LFE / Bass): Deep Magenta
        case 4u { return vec3<f32>(0.0, 0.95, 0.70);  } // Ch 4 (Surround L): Electric Teal
        case 5u { return vec3<f32>(0.72, 0.15, 1.0);  } // Ch 5 (Surround R): Vivid Violet
        case 6u { return vec3<f32>(1.0, 0.55, 0.08);  } // Ch 6 (Rear L): Neon Orange
        default { return vec3<f32>(1.0, 0.85, 0.10);  } // Ch 7 (Rear R): Bright Gold
    }
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    let mat_id = in.tex_coords.x;
    var pos = in.position;

    // Audio-reactive frame expansion & vibration for channels 0..7 (mat 1.0..8.0)
    if (mat_id >= 0.8 && mat_id <= 8.2) {
        let ch_idx = min(7u, u32(round(mat_id)) - 1u);
        let ch_vol = clamp(audio.channels[ch_idx / 4u][ch_idx % 4u], 0.0, 1.2);
        
        let scale_x = 1.0 + ch_vol * 0.22;
        let scale_y = 1.0 + ch_vol * 0.26;
        let pulse_vib = sin(audio.smooth_time * 30.0 + f32(ch_idx) * 2.5) * ch_vol * 0.03;

        pos.x = pos.x * scale_x;
        pos.y = pos.y * (scale_y + pulse_vib);
    }

    out.world_pos = pos;
    out.normal = in.normal;
    out.uv = in.tex_coords;
    out.mat = mat_id;
    out.local_pos = in.position;

    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let bass_pulse = clamp(audio.spectrum[2].x * 1.6, 0.0, 1.0);
    let mat_id = in.mat;

    let n = normalize(in.normal);
    let v = normalize(vec3<f32>(0.0, 1.6, -1.0) - in.world_pos);

    var color = vec3<f32>(0.0);

    if (mat_id < 0.5) {
        // =========================================================================
        // 0.0: DARK POLISHED REFLECTIVE FLOOR WITH NEON SPECULAR MIRRORING
        // =========================================================================
        let floor_dark = vec3<f32>(0.008, 0.009, 0.012);

        // Floor reflective tile grid lines
        let grid_x = smoothstep(0.04, 0.0, abs(fract(in.world_pos.x * 1.5) - 0.5));
        let grid_z = smoothstep(0.04, 0.0, abs(fract(in.world_pos.z * 1.5) - 0.5));
        let grid_tile = (grid_x + grid_z) * 0.20;

        // Dynamic reflected light from all 8 neon portal frames
        var neon_reflection = vec3<f32>(0.0);
        let z_positions = array<f32, 8>(3.0, 6.5, 10.0, 13.5, 17.0, 20.5, 24.0, 27.5);
        for (var i = 0u; i < 8u; i = i + 1u) {
            let frame_z = z_positions[i];
            let dist_z = abs(in.world_pos.z - frame_z);
            let dist_x = max(0.0, abs(in.world_pos.x) - 2.5);
            let refl_dist = length(vec2<f32>(dist_x, dist_z));
            
            let ch_vol = clamp(audio.channels[i / 4u][i % 4u], 0.0, 1.2);
            let col = channel_color(i);
            let falloff = 1.0 / (1.0 + refl_dist * refl_dist * 0.8);
            neon_reflection += col * falloff * (0.8 + ch_vol * 2.2);
        }

        // Fresnel reflection
        let fresnel = pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.4 + 0.6;
        color = floor_dark + vec3<f32>(grid_tile * 0.02) + neon_reflection * fresnel * 0.35;

    } else {
        // =========================================================================
        // 1.0 .. 8.0: GLOWING 3D NEON AUDIO-CHANNEL PORTAL FRAMES
        // =========================================================================
        let ch_idx = min(7u, u32(round(mat_id)) - 1u);
        let ch_vol = clamp(audio.channels[ch_idx / 4u][ch_idx % 4u], 0.0, 1.2);
        let col = channel_color(ch_idx);

        // Core tube brightness with audio volume expansion
        let tube_glow = (3.2 + ch_vol * 5.0 + bass_pulse * 1.5);
        let emissive_tube = col * tube_glow;

        // Tube specular sheen along cylinder normal
        let spec = pow(max(dot(n, v), 0.0), 4.0) * 0.5;
        color = emissive_tube + vec3<f32>(spec);
    }

    // Atmospheric depth fog
    let fog_f = smoothstep(12.0, 45.0, in.world_pos.z);
    let fog_c = vec3<f32>(0.003, 0.003, 0.006);
    color = mix(color, fog_c, fog_f);

    let tonemapped = aces_tonemap(color);
    return vec4<f32>(tonemapped, 1.0);
}
