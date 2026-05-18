@group(0) @binding(0)
var smoke_out: texture_storage_3d<rgba16float, write>;

// Uniforms for time
struct Params {
    time: f32,
    audio_activity: f32,
    _pad1: f32,
    _pad2: f32,
}
@group(0) @binding(1)
var<uniform> params: Params;

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
    for(var i = 0; i < 4; i++) {
        f += amp * noise(p);
        p *= 2.1;
        amp *= 0.5;
    }
    return f;
}

@compute @workgroup_size(4, 4, 4)
fn cs_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dim = textureDimensions(smoke_out);
    if (global_id.x >= dim.x || global_id.y >= dim.y || global_id.z >= dim.z) {
        return;
    }

    // Map 3D voxel coordinate to world space bounding box
    // Bounding box for smoke: x in [-6, 6], y in [-1, 4], z in [-4, 8]
    let normalized = vec3<f32>(global_id) / vec3<f32>(dim - vec3<u32>(1u, 1u, 1u));
    let p = vec3<f32>(
        mix(-6.0, 6.0, normalized.x),
        mix(-1.0, 4.0, normalized.y),
        mix(-4.0, 8.0, normalized.z)
    );

    let time = params.time;
    let np = p * 1.5 - vec3<f32>(0.0, time * 0.2, time * 0.1);
    let n = fbm(np);

    // Calculate density
    let dens_base = n - 0.45;
    
    // Store in texture (R channel)
    textureStore(smoke_out, global_id, vec4<f32>(dens_base, 0.0, 0.0, 1.0));
}
