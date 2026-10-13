// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

struct Particle {
    pos: vec4<f32>,
    vel: vec4<f32>,
}

@group(0) @binding(1)
var<storage, read_write> particles: array<Particle>;

@group(0) @binding(2)
var<storage, read_write> grid: array<atomic<u32>>;

@compute @workgroup_size(256)
fn clear(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x + global_id.y * 512u;
    if (idx < 262144u) {
        atomicStore(&grid[idx], 0u);
    }
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= 100000u) { return; }

    var p = particles[idx];

    if (length(p.pos.xyz) < 0.001 && length(p.vel.xyz) < 0.001) {
        let rng1 = fract(sin(f32(idx) * 12.9898) * 43758.5453);
        let rng2 = fract(sin(f32(idx) * 78.233) * 43758.5453);
        let angle = rng1 * 6.2831853;
        let r = sqrt(rng2) * 5.0;
        p.pos = vec4<f32>(cos(angle) * r, 0.1, sin(angle) * r, 1.0);
        p.vel = vec4<f32>(0.0);
    }

    let dt = clamp(audio.frame_dt, 0.001, 0.033);
    var force = vec3<f32>(0.0, -9.8, 0.0);

    // Quantize particle azimuth to 16 discrete magnetic poles around the dish
    // so particles coalesce into distinct Rosensweig ferrofluid spikes instead of concentric rings!
    let angle = atan2(p.pos.z, p.pos.x);
    let num_poles = 16.0;
    let sector_step = 6.2831853 / num_poles;
    let pole_idx_signed = round(angle / sector_step);
    let pole_angle = pole_idx_signed * sector_step;
    let abs_angle = abs(pole_angle);

    // Normalize audio.spectrum [0..100] -> [0..1.25] instead of clamping raw [0..100] at 1.0!
    let spec_idx = min(u32((abs_angle / 3.14159265) * 255.0), 255u);
    let raw_spec = clamp(audio.spectrum[spec_idx / 4u][spec_idx % 4u] / 72.0, 0.0, 1.25);

    let dt_scale = dt / 0.016;
    let rate = 1.0 - pow(1.0 - select(0.09, 0.045, raw_spec > p.vel.w), dt_scale);
    p.vel.w = mix(p.vel.w, raw_spec, rate);
    let spec_val = p.vel.w;

    // Two concentric spike rings + central bass pole based on particle ID
    let ring_tier = idx % 3u;
    var mag_r = 1.2 + spec_val * 2.6;
    if ring_tier == 1u {
        mag_r = 2.4 + spec_val * 2.2;
    } else if ring_tier == 2u && (idx % 9u == 0u) {
        mag_r = 0.25 + spec_val * 1.1;
    }
    let mag_h = -0.35 + spec_val * 2.85;
    let mag_pos = vec3<f32>(cos(pole_angle) * mag_r, mag_h, sin(pole_angle) * mag_r);

    let dir = mag_pos - p.pos.xyz;
    let dist_sq = dot(dir, dir);
    let mag_force = (spec_val * 210.0 + 22.0) / (dist_sq + 0.45);
    force += normalize(dir) * mag_force;

    let t_time = audio.smooth_time * 0.35;
    let turbulence = vec3<f32>(
        sin(p.pos.z * 3.0 + t_time) * cos(p.pos.y * 2.0),
        cos(p.pos.x * 3.0 - t_time) * sin(p.pos.z * 2.0),
        sin(p.pos.x * 3.0 + t_time) * cos(p.pos.y * 2.0)
    );
    force += turbulence * (2.8 + spec_val * 7.5);

    let center_dir = vec3<f32>(0.0, -1.0, 0.0) - p.pos.xyz;
    force += normalize(center_dir) * (60.0 / (dot(center_dir, center_dir) + 1.0));

    if (p.pos.y < 0.0) {
        p.pos.y = 0.0;
        p.vel.y *= -0.5;
        force.y += 9.8;
    }

    force -= p.pos.xyz * 5.0;

    p.vel = vec4<f32>((p.vel.xyz + force * dt) * pow(0.91, dt_scale), p.vel.w);
    p.pos = vec4<f32>(p.pos.xyz + p.vel.xyz * dt, p.pos.w);

    particles[idx] = p;

    let grid_size = 512;
    let grid_x = i32(p.pos.x * 25.0 + 256.0);
    let grid_z = i32(p.pos.z * 25.0 + 256.0);

    for (var di = -2; di <= 2; di++) {
        for (var dj = -2; dj <= 2; dj++) {
            let gx = grid_x + di;
            let gz = grid_z + dj;
            if (gx >= 0 && gx < grid_size && gz >= 0 && gz < grid_size) {
                let cell = u32(gz) * u32(grid_size) + u32(gx);
                atomicAdd(&grid[cell], 1u);
            }
        }
    }
}
