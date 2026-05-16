// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

struct Particle {
    pos: vec3<f32>,
    vel: vec3<f32>,
    mass: f32,
    smooth_spec: f32,  // EMA-smoothed spectrum value — filters transients for standing waves
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
    
    // Initialize if pos is exactly 0
    if (length(p.pos) < 0.001 && length(p.vel) < 0.001) {
        let rng1 = fract(sin(f32(idx) * 12.9898) * 43758.5453);
        let rng2 = fract(sin(f32(idx) * 78.233) * 43758.5453);
        let angle = rng1 * 6.28318;
        let r = sqrt(rng2) * 5.0;
        p.pos = vec3<f32>(cos(angle) * r, 0.1, sin(angle) * r);
        p.vel = vec3<f32>(0.0);
        p.mass = 1.0;
        p.smooth_spec = 0.0;
    }

    // Physics
    let dt = 0.016;
    var force = vec3<f32>(0.0, -9.8, 0.0); // Gravity

    // Frequency-driven magnetic field
    let angle = atan2(p.pos.z, p.pos.x);
    let abs_angle = abs(angle);
    let spec_idx = min(u32((abs_angle / 3.14159) * 127.0), 127u);
    let raw_spec = clamp(audio.spectrum[spec_idx / 4u][spec_idx % 4u], 0.0, 1.0);
    
    // Temporal EMA smoothing — standing wave filter
    // Slow attack (0.03): transient hits are ignored, only sustained frequencies build peaks
    // Moderate decay (0.08): fluid relaxes gracefully when a frequency stops
    let rate = select(0.08, 0.03, raw_spec > p.smooth_spec);
    p.smooth_spec = mix(p.smooth_spec, raw_spec, rate);
    let spec_val = p.smooth_spec;
    
    // Dynamic magnet position based on smoothed frequency
    let mag_r = 0.5 + spec_val * 4.0;
    let mag_h = -0.5 + spec_val * 3.0;
    let mag_pos = vec3<f32>(cos(angle) * mag_r, mag_h, sin(angle) * mag_r);
    
    let dir = mag_pos - p.pos;
    let dist_sq = dot(dir, dir);
    let mag_force = (spec_val * 200.0 + 20.0) / (dist_sq + 0.5);
    force += normalize(dir) * mag_force;
    
    // Subtle fluid turbulence — reduced to avoid fighting standing wave formation
    let t_time = audio.time * 0.3;
    let turbulence = vec3<f32>(
        sin(p.pos.z * 3.0 + t_time) * cos(p.pos.y * 2.0),
        cos(p.pos.x * 3.0 - t_time) * sin(p.pos.z * 2.0),
        sin(p.pos.x * 3.0 + t_time) * cos(p.pos.y * 2.0)
    );
    force += turbulence * (3.0 + spec_val * 8.0);
    
    // Central electromagnet to keep the fluid pooled together
    let center_dir = vec3<f32>(0.0, -1.0, 0.0) - p.pos;
    force += normalize(center_dir) * (60.0 / (dot(center_dir, center_dir) + 1.0));

    // Floor collision
    if (p.pos.y < 0.0) {
        p.pos.y = 0.0;
        p.vel.y *= -0.5;
        force.y += 9.8;
    }

    // Spring back to origin slightly to keep them in bounds
    force -= p.pos * 5.0;

    // Integration with viscous damping (0.91 = thick fluid, resists rapid motion)
    p.vel += force * dt;
    p.vel *= 0.91;
    p.pos += p.vel * dt;

    particles[idx] = p;

    // Scatter to 2D grid with 5×5 splat for continuous fluid coverage
    // Each particle covers a wide blob to prevent gaps in the density field
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
