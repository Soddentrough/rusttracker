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

fn hash2(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    let mat_id = in.tex_coords.x;
    var pos = in.position;

    // Instanced 3D Falling Raindrops in Frustum Volume (mat = 3.0)
    let rain_speed = 38.0;
    let rain_height = 30.0;
    let anchor_y = in.tex_coords.y;
    let offset_y = pos.y - anchor_y;
    let drop_seed = hash2(vec2<f32>(pos.x, pos.z));
    let drop_y = fract((anchor_y - audio.smooth_time * rain_speed + drop_seed * 25.0) / rain_height) * rain_height;
    pos.y = drop_y + offset_y;
    
    // Wind drift slant
    let wind_drift = sin(audio.smooth_time * 0.4) * 1.5;
    pos.x += (pos.y - 15.0) * 0.08 + wind_drift * 0.5;

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
    let treble_pulse = clamp(audio.spectrum[80].x * 1.8, 0.0, 1.0);

    // Lightning Flash Global Illumination
    let is_flash = step(0.68, bass_pulse);
    let flash_power = max(0.0, (bass_pulse - 0.68) / 0.32);
    let lightning_light = vec3<f32>(0.85, 0.92, 1.0) * flash_power * 3.5;

    // Translucent cool blue-white falling rain streak
    let base_rain = vec3<f32>(0.70, 0.85, 1.0);
    let rain_col = base_rain * (0.85 + lightning_light * 0.6) * (1.1 + treble_pulse * 0.7);

    // Distance depth fade
    let depth_fade = smoothstep(120.0, 10.0, in.world_pos.z);
    let final_color = rain_col * depth_fade;

    let tonemapped = aces_tonemap(final_color);
    return vec4<f32>(tonemapped, 1.0);
}
