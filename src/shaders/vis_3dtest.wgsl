// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

struct MVP {
    model: mat4x4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
};

@group(1) @binding(0) var<uniform> mvp: MVP;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main_3d(model: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    let world_pos = mvp.model * vec4<f32>(model.position, 1.0);
    out.world_pos = world_pos.xyz;
    out.clip_position = mvp.proj * mvp.view * world_pos;
    
    // Normal matrix should technically be inverse-transpose, but assuming uniform scaling:
    out.normal = (mvp.model * vec4<f32>(model.normal, 0.0)).xyz;
    out.uv = model.uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let N = normalize(in.normal);
    let V = normalize(-in.world_pos); // simple view vector assuming camera at origin
    
    // Audio reactivity (using .x since AudioUniforms packs them as vec4)
    let bass = audio.channels[0].x * 2.0;
    let treble = audio.channels[2].x * 2.0; // Index 2 is channels[8] logically? No, array<vec4,8> = 32 floats. Index 2 = ch 8.
    
    // Lighting
    let light_dir = normalize(vec3<f32>(1.0, 1.0, 1.0));
    let diff = max(dot(N, light_dir), 0.0);
    let ambient = vec3<f32>(0.1, 0.0, 0.1);
    
    let base_color = vec3<f32>(0.1 + bass, 0.5, 0.8 + treble) * in.uv.x;
    let final_color = base_color * diff + ambient;
    
    // Wireframe edge glow effect based on UVs
    let edge_dist = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
    let edge_glow = smoothstep(0.05, 0.0, edge_dist);
    let edge_color = vec3<f32>(1.0, 0.0, 1.0) * edge_glow * (1.0 + bass * 2.0);
    
    return vec4<f32>(final_color + edge_color, 1.0);
}
