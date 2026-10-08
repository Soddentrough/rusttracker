// INCLUDE: common

// ============================================================================
// Optical Glass & Acoustic Water Chamber — FLIP Fluid Compute Shader
// Hybrid Eulerian-Lagrangian Incompressible Navier-Stokes Fluid Solver
// Acoustic Sub-bass Geyser Impulses, Faraday Standing Waves & Droplet Splatting
// ============================================================================

struct Particle {
    pos: vec4<f32>, // pos.xyz = world pos, pos.w = mass / density
    vel: vec4<f32>, // vel.xyz = world velocity, vel.w = droplet age
};

struct CellAccum {
    u_mom: atomic<i32>,
    v_mom: atomic<i32>,
    w_mom: atomic<i32>,
    weight: atomic<u32>,
};

// --- Constants ---
const NUM_PARTICLES: u32 = 65536u;
const GRID_X: u32 = 64u;
const GRID_Y: u32 = 32u;
const GRID_Z: u32 = 64u;
const TOTAL_CELLS: u32 = 131072u; // 64 * 32 * 64
const RENDER_GRID_SIZE: u32 = 512u;
const TOTAL_RENDER_CELLS: u32 = 262144u; // 512 * 512

const DOMAIN_MIN: vec3<f32> = vec3<f32>(-3.2, 0.0, -3.2);
const DOMAIN_MAX: vec3<f32> = vec3<f32>(3.2, 2.4, 3.2);
const CELL_SIZE: vec3<f32> = vec3<f32>(0.1, 0.075, 0.1); // (6.4/64, 2.4/32, 6.4/64)
const INV_CELL_SIZE: vec3<f32> = vec3<f32>(10.0, 13.333333, 10.0);

const FIXED_SCALE: f32 = 512.0;
const CHAMBER_RADIUS: f32 = 2.65;
const FLUID_DENSITY: f32 = 1.0;
const FLIP_RATIO: f32 = 0.95; // 95% FLIP, 5% PIC

// --- Bindings ---
@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(2) var<storage, read_write> grid_accum: array<CellAccum>;
@group(0) @binding(3) var<storage, read_write> grid_vel_new: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> grid_vel_old: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> pressure_0: array<f32>;
@group(0) @binding(6) var<storage, read_write> pressure_1: array<f32>;
@group(0) @binding(7) var<storage, read_write> render_grid: array<atomic<u32>>;

// --- Utility Functions ---
fn get_cell_idx(x: u32, y: u32, z: u32) -> u32 {
    return x + y * GRID_X + z * (GRID_X * GRID_Y);
}

fn cell_in_bounds(x: i32, y: i32, z: i32) -> bool {
    return x >= 0 && x < i32(GRID_X) && y >= 0 && y < i32(GRID_Y) && z >= 0 && z < i32(GRID_Z);
}

fn world_to_cell(p: vec3<f32>) -> vec3<f32> {
    return (p - DOMAIN_MIN) * INV_CELL_SIZE;
}

fn cell_to_world(c: vec3<f32>) -> vec3<f32> {
    return DOMAIN_MIN + (c + vec3<f32>(0.5)) * CELL_SIZE;
}

// ----------------------------------------------------------------------------
// PASS 1A: Clear Grid Accumulator & Pressure Buffers
// ----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn cs_clear(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= TOTAL_CELLS) { return; }

    atomicStore(&grid_accum[idx].u_mom, 0);
    atomicStore(&grid_accum[idx].v_mom, 0);
    atomicStore(&grid_accum[idx].w_mom, 0);
    atomicStore(&grid_accum[idx].weight, 0u);

    pressure_0[idx] = 0.0;
    pressure_1[idx] = 0.0;
}

// ----------------------------------------------------------------------------
// PASS 1B: Clear Render Heightfield Grid
// ----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn cs_clear_render(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= TOTAL_RENDER_CELLS) { return; }
    atomicStore(&render_grid[idx], 0u);
}

// ----------------------------------------------------------------------------
// PASS 2: Particle-to-Grid (P2G) Momentum Transfer
// ----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn cs_p2g(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let p_idx = global_id.x;
    if (p_idx >= NUM_PARTICLES) { return; }

    var p = particles[p_idx];

    // Seed initial particle pool in the cylindrical glass vessel if uninitialized
    if (length(p.pos.xyz) < 0.001 || abs(p.pos.w - 1.333) > 0.01) {
        let fi = f32(p_idx);
        let r1 = fract(sin(fi * 12.9898 + 0.123) * 43758.5453);
        let r2 = fract(sin(fi * 78.233 + 0.456) * 43758.5453);
        let r3 = fract(sin(fi * 39.421 + 0.789) * 43758.5453);

        let angle = r1 * 6.2831853;
        let radius = sqrt(r2) * 2.55;
        let y_pos = 0.05 + r3 * 0.28; // Resting water level ~30cm

        p.pos = vec4<f32>(cos(angle) * radius, y_pos, sin(angle) * radius, 1.333);
        p.vel = vec4<f32>(0.0);
        particles[p_idx] = p;
    }

    let cell_pos = world_to_cell(p.pos.xyz) - vec3<f32>(0.5);
    let base_i = vec3<i32>(floor(cell_pos));
    let frac = fract(cell_pos);

    // Trilinear interpolation splat to 8 adjacent grid cell vertices
    for (var dz = 0; dz <= 1; dz++) {
        let wz = select(1.0 - frac.z, frac.z, dz == 1);
        let cz = base_i.z + dz;

        for (var dy = 0; dy <= 1; dy++) {
            let wy = select(1.0 - frac.y, frac.y, dy == 1);
            let cy = base_i.y + dy;

            for (var dx = 0; dx <= 1; dx++) {
                let wx = select(1.0 - frac.x, frac.x, dx == 1);
                let cx = base_i.x + dx;

                if (cell_in_bounds(cx, cy, cz)) {
                    let weight = wx * wy * wz;
                    if (weight > 0.0001) {
                        let c_idx = get_cell_idx(u32(cx), u32(cy), u32(cz));
                        let scaled_u = i32(p.vel.x * weight * FIXED_SCALE);
                        let scaled_v = i32(p.vel.y * weight * FIXED_SCALE);
                        let scaled_w = i32(p.vel.z * weight * FIXED_SCALE);
                        let scaled_wt = u32(weight * FIXED_SCALE);

                        atomicAdd(&grid_accum[c_idx].u_mom, scaled_u);
                        atomicAdd(&grid_accum[c_idx].v_mom, scaled_v);
                        atomicAdd(&grid_accum[c_idx].w_mom, scaled_w);
                        atomicAdd(&grid_accum[c_idx].weight, scaled_wt);
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------
// PASS 3: Grid Normalization, Acoustic Forces & Boundary Walls
// ----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn cs_grid_forces(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= TOTAL_CELLS) { return; }

    let gx = idx % GRID_X;
    let gy = (idx / GRID_X) % GRID_Y;
    let gz = idx / (GRID_X * GRID_Y);

    let world_p = cell_to_world(vec3<f32>(f32(gx), f32(gy), f32(gz)));
    let dt = clamp(audio.frame_dt, 0.001, 0.033);

    let raw_wt = f32(atomicLoad(&grid_accum[idx].weight)) / FIXED_SCALE;
    var vel = vec3<f32>(0.0);
    var cell_type = 0.0; // 0 = AIR

    let r_xz = length(world_p.xz);

    // Boundary classification: glass cylinder walls and transducer basin floor
    if (r_xz > CHAMBER_RADIUS || world_p.y < 0.02) {
        cell_type = 2.0; // SOLID
        grid_vel_new[idx] = vec4<f32>(0.0, 0.0, 0.0, cell_type);
        grid_vel_old[idx] = vec4<f32>(0.0, 0.0, 0.0, cell_type);
        return;
    }

    if (raw_wt > 0.002) {
        cell_type = 1.0; // LIQUID
        let inv_wt = 1.0 / raw_wt;
        vel.x = (f32(atomicLoad(&grid_accum[idx].u_mom)) / FIXED_SCALE) * inv_wt;
        vel.y = (f32(atomicLoad(&grid_accum[idx].v_mom)) / FIXED_SCALE) * inv_wt;
        vel.z = (f32(atomicLoad(&grid_accum[idx].w_mom)) / FIXED_SCALE) * inv_wt;

        // --- Apply Physical & Acoustic Forces on Fluid ---
        // 1. Gravity
        var force = vec3<f32>(0.0, -9.8, 0.0);

        // 2. Acoustic Sub-bass Geyser Fountain Impulses
        // Powerful acoustic piston transducer located at basin center (0, 0, 0)
        let sub_bass = clamp(audio.spectrum[0].x * 1.8 + audio.channels[0].x * 0.9, 0.0, 2.5);
        let jet_falloff = exp(-r_xz * r_xz * 2.2);
        let geyser_impulse = sub_bass * 280.0 * jet_falloff * smoothstep(0.75, 0.02, world_p.y);
        force.y += geyser_impulse;

        // Lateral splash dispersal: when fountain reaches top, fluid spreads outward
        let spread_factor = smoothstep(0.4, 1.2, world_p.y) * sub_bass * 35.0;
        if (r_xz > 0.01) {
            force.x += (world_p.x / r_xz) * spread_factor;
            force.z += (world_p.z / r_xz) * spread_factor;
        }

        // 3. Faraday Acoustic Surface Waves (Parametric Standing Waves)
        // Driven by mid/high spectrum frequencies creating geometric Bessel-mode ripples
        let azimuth = atan2(world_p.z, world_p.x);
        let mids = clamp(audio.spectrum[8].x * 1.4 + audio.spectrum[16].x * 1.0, 0.0, 1.6);
        let highs = clamp(audio.spectrum[32].x * 1.2 + audio.spectrum[48].x * 1.0, 0.0, 1.4);
        
        let wave_k = 7.5;
        let mode_m = 4.0;
        let standing_wave = cos(r_xz * wave_k - audio.time * 3.5) * cos(azimuth * mode_m);
        let ripple_force = standing_wave * (mids * 24.0 + highs * 18.0) * smoothstep(0.08, 0.40, world_p.y);
        force.y += ripple_force;

        // 4. Hydrodynamic Surface Tension & Basin Cohesion
        // Restores liquid equilibrium and creates clean water meniscus at the glass walls
        let excess_h = max(0.0, world_p.y - 0.38);
        force.y -= excess_h * 16.0;

        // Inward meniscus pull near glass walls
        let wall_dist = CHAMBER_RADIUS - r_xz;
        if (wall_dist < 0.35 && world_p.y < 0.5) {
            let meniscus_pull = smoothstep(0.35, 0.0, wall_dist) * 10.0;
            force.y += meniscus_pull;
        }

        // Integrate forces
        vel += force * dt;

        // Water viscosity damping
        let dt_scale = dt / 0.016;
        vel *= pow(0.985, dt_scale);
    }

    grid_vel_new[idx] = vec4<f32>(vel, cell_type);
    grid_vel_old[idx] = vec4<f32>(vel, cell_type);
}

// ----------------------------------------------------------------------------
// PASS 4A & 4B: Jacobi Incompressibility Pressure Solver (Ping-Pong)
// ----------------------------------------------------------------------------
fn compute_divergence(gx: i32, gy: i32, gz: i32) -> f32 {
    let inv_2dx = 0.5 * INV_CELL_SIZE.x;
    let inv_2dy = 0.5 * INV_CELL_SIZE.y;
    let inv_2dz = 0.5 * INV_CELL_SIZE.z;

    let x_plus = select(0.0, grid_vel_new[get_cell_idx(u32(gx + 1), u32(gy), u32(gz))].x, gx + 1 < i32(GRID_X));
    let x_minus = select(0.0, grid_vel_new[get_cell_idx(u32(gx - 1), u32(gy), u32(gz))].x, gx - 1 >= 0);

    let y_plus = select(0.0, grid_vel_new[get_cell_idx(u32(gx), u32(gy + 1), u32(gz))].y, gy + 1 < i32(GRID_Y));
    let y_minus = select(0.0, grid_vel_new[get_cell_idx(u32(gx), u32(gy - 1), u32(gz))].y, gy - 1 >= 0);

    let z_plus = select(0.0, grid_vel_new[get_cell_idx(u32(gx), u32(gy), u32(gz + 1))].z, gz + 1 < i32(GRID_Z));
    let z_minus = select(0.0, grid_vel_new[get_cell_idx(u32(gx), u32(gy), u32(gz - 1))].z, gz - 1 >= 0);

    return (x_plus - x_minus) * inv_2dx + (y_plus - y_minus) * inv_2dy + (z_plus - z_minus) * inv_2dz;
}

@compute @workgroup_size(256)
fn cs_pressure_0_to_1(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= TOTAL_CELLS) { return; }

    let cell_info = grid_vel_new[idx];
    if (cell_info.w != 1.0) {
        pressure_1[idx] = 0.0;
        return;
    }

    let gx = i32(idx % GRID_X);
    let gy = i32((idx / GRID_X) % GRID_Y);
    let gz = i32(idx / (GRID_X * GRID_Y));

    let div = compute_divergence(gx, gy, gz);
    let dt = clamp(audio.frame_dt, 0.001, 0.033);
    let rhs = (FLUID_DENSITY / dt) * div;

    var sum_p = 0.0;
    var num_valid = 0.0;

    let neighbors = array<vec3<i32>, 6>(
        vec3<i32>(gx + 1, gy, gz), vec3<i32>(gx - 1, gy, gz),
        vec3<i32>(gx, gy + 1, gz), vec3<i32>(gx, gy - 1, gz),
        vec3<i32>(gx, gy, gz + 1), vec3<i32>(gx, gy, gz - 1)
    );

    for (var i = 0; i < 6; i++) {
        let nb = neighbors[i];
        if (cell_in_bounds(nb.x, nb.y, nb.z)) {
            let n_idx = get_cell_idx(u32(nb.x), u32(nb.y), u32(nb.z));
            let n_type = grid_vel_new[n_idx].w;
            if (n_type == 1.0) {
                sum_p += pressure_0[n_idx];
                num_valid += 1.0;
            } else if (n_type == 2.0) {
                sum_p += pressure_0[idx];
                num_valid += 1.0;
            }
        }
    }

    if (num_valid > 0.0) {
        let inv_dx2 = INV_CELL_SIZE.x * INV_CELL_SIZE.x;
        pressure_1[idx] = (sum_p - rhs / inv_dx2) / num_valid;
    } else {
        pressure_1[idx] = 0.0;
    }
}

@compute @workgroup_size(256)
fn cs_pressure_1_to_0(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= TOTAL_CELLS) { return; }

    let cell_info = grid_vel_new[idx];
    if (cell_info.w != 1.0) {
        pressure_0[idx] = 0.0;
        return;
    }

    let gx = i32(idx % GRID_X);
    let gy = i32((idx / GRID_X) % GRID_Y);
    let gz = i32(idx / (GRID_X * GRID_Y));

    let div = compute_divergence(gx, gy, gz);
    let dt = clamp(audio.frame_dt, 0.001, 0.033);
    let rhs = (FLUID_DENSITY / dt) * div;

    var sum_p = 0.0;
    var num_valid = 0.0;

    let neighbors = array<vec3<i32>, 6>(
        vec3<i32>(gx + 1, gy, gz), vec3<i32>(gx - 1, gy, gz),
        vec3<i32>(gx, gy + 1, gz), vec3<i32>(gx, gy - 1, gz),
        vec3<i32>(gx, gy, gz + 1), vec3<i32>(gx, gy, gz - 1)
    );

    for (var i = 0; i < 6; i++) {
        let nb = neighbors[i];
        if (cell_in_bounds(nb.x, nb.y, nb.z)) {
            let n_idx = get_cell_idx(u32(nb.x), u32(nb.y), u32(nb.z));
            let n_type = grid_vel_new[n_idx].w;
            if (n_type == 1.0) {
                sum_p += pressure_1[n_idx];
                num_valid += 1.0;
            } else if (n_type == 2.0) {
                sum_p += pressure_1[idx];
                num_valid += 1.0;
            }
        }
    }

    if (num_valid > 0.0) {
        let inv_dx2 = INV_CELL_SIZE.x * INV_CELL_SIZE.x;
        pressure_0[idx] = (sum_p - rhs / inv_dx2) / num_valid;
    } else {
        pressure_0[idx] = 0.0;
    }
}

// ----------------------------------------------------------------------------
// PASS 5: Velocity Projection (Subtract Pressure Gradient)
// ----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn cs_grid_project(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= TOTAL_CELLS) { return; }

    let cell_info = grid_vel_new[idx];
    if (cell_info.w != 1.0) { return; }

    let gx = i32(idx % GRID_X);
    let gy = i32((idx / GRID_X) % GRID_Y);
    let gz = i32(idx / (GRID_X * GRID_Y));

    let dt = clamp(audio.frame_dt, 0.001, 0.033);
    let inv_2rho = 0.5 / FLUID_DENSITY;

    let p_c = pressure_0[idx];
    let p_xp = select(p_c, pressure_0[get_cell_idx(u32(gx + 1), u32(gy), u32(gz))], gx + 1 < i32(GRID_X));
    let p_xm = select(p_c, pressure_0[get_cell_idx(u32(gx - 1), u32(gy), u32(gz))], gx - 1 >= 0);
    let p_yp = select(p_c, pressure_0[get_cell_idx(u32(gx), u32(gy + 1), u32(gz))], gy + 1 < i32(GRID_Y));
    let p_ym = select(p_c, pressure_0[get_cell_idx(u32(gx), u32(gy - 1), u32(gz))], gy - 1 >= 0);
    let p_zp = select(p_c, pressure_0[get_cell_idx(u32(gx), u32(gy), u32(gz + 1))], gz + 1 < i32(GRID_Z));
    let p_zm = select(p_c, pressure_0[get_cell_idx(u32(gx), u32(gy), u32(gz - 1))], gz - 1 >= 0);

    let grad_p = vec3<f32>(
        (p_xp - p_xm) * INV_CELL_SIZE.x * inv_2rho,
        (p_yp - p_ym) * INV_CELL_SIZE.y * inv_2rho,
        (p_zp - p_zm) * INV_CELL_SIZE.z * inv_2rho
    );

    var v = cell_info.xyz - grad_p * dt;
    grid_vel_new[idx] = vec4<f32>(v, 1.0);
}

// ----------------------------------------------------------------------------
// PASS 6: G2P Interpolation, Particle Advection & Splatting
// ----------------------------------------------------------------------------
fn sample_grid_vel(pos: vec3<f32>, use_old: bool) -> vec3<f32> {
    let cell_pos = world_to_cell(pos) - vec3<f32>(0.5);
    let base_i = vec3<i32>(floor(cell_pos));
    let frac = fract(cell_pos);

    var vel = vec3<f32>(0.0);
    var total_w = 0.0;

    for (var dz = 0; dz <= 1; dz++) {
        let wz = select(1.0 - frac.z, frac.z, dz == 1);
        let cz = base_i.z + dz;

        for (var dy = 0; dy <= 1; dy++) {
            let wy = select(1.0 - frac.y, frac.y, dy == 1);
            let cy = base_i.y + dy;

            for (var dx = 0; dx <= 1; dx++) {
                let wx = select(1.0 - frac.x, frac.x, dx == 1);
                let cx = base_i.x + dx;

                if (cell_in_bounds(cx, cy, cz)) {
                    let w = wx * wy * wz;
                    let c_idx = get_cell_idx(u32(cx), u32(cy), u32(cz));
                    let sample_v = select(grid_vel_new[c_idx].xyz, grid_vel_old[c_idx].xyz, use_old);
                    vel += sample_v * w;
                    total_w += w;
                }
            }
        }
    }

    if (total_w > 0.0001) {
        return vel / total_w;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(256)
fn cs_g2p_advect_splat(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let p_idx = global_id.x;
    if (p_idx >= NUM_PARTICLES) { return; }

    var p = particles[p_idx];
    let dt = clamp(audio.frame_dt, 0.001, 0.033);

    // 1. G2P Interpolation
    let v_new = sample_grid_vel(p.pos.xyz, false);
    let v_old = sample_grid_vel(p.pos.xyz, true);

    let v_flip = p.vel.xyz + (v_new - v_old);
    let v_pic = v_new;

    var v_cur = mix(v_pic, v_flip, FLIP_RATIO);
    let r_p = length(p.pos.xz);

    // Acoustic Sub-bass Geyser Fountain Impulses
    let sub_bass = clamp(audio.spectrum[0].x * 1.8 + audio.channels[0].x * 0.9, 0.0, 2.5);
    let jet_falloff = exp(-r_p * r_p * 3.5);
    let geyser_kick = sub_bass * 14.5 * jet_falloff * smoothstep(0.70, 0.05, p.pos.y);
    v_cur.y += geyser_kick * (dt / 0.016);

    // Lateral spray dispersal at fountain apex
    if (r_p > 0.01 && p.pos.y > 0.45) {
        let spread = smoothstep(0.45, 1.4, p.pos.y) * sub_bass * 5.5 * (dt / 0.016);
        v_cur.x += (p.pos.x / r_p) * spread;
        v_cur.z += (p.pos.z / r_p) * spread;
    }

    // Faraday standing waves ripple impulse
    let azimuth = atan2(p.pos.z, p.pos.x);
    let mids = clamp(audio.spectrum[8].x * 1.4 + audio.spectrum[16].x * 1.0, 0.0, 1.6);
    let highs = clamp(audio.spectrum[32].x * 1.2 + audio.spectrum[48].x * 1.0, 0.0, 1.4);
    let standing_wave = cos(r_p * 8.0 - audio.time * 4.0) * cos(azimuth * 4.0);
    v_cur.y += standing_wave * (mids * 1.8 + highs * 1.2) * smoothstep(0.1, 0.45, p.pos.y) * (dt / 0.016);

    // Gravity & restoring buoyancy
    v_cur.y -= 9.8 * dt;
    let excess_h = max(0.0, p.pos.y - 0.35);
    v_cur.y -= excess_h * 12.0 * dt;

    // Blended FLIP/PIC update
    p.vel = vec4<f32>(v_cur, p.vel.w);

    // 2. Advect Particle Position
    p.pos = vec4<f32>(p.pos.xyz + p.vel.xyz * dt, p.pos.w);

    // 3. Boundary Collisions (Cylindrical Glass Vessel & Acoustic Floor)
    let r_xz = length(p.pos.xz);
    let floor_y = 0.025;

    // Floor collision
    if (p.pos.y < floor_y) {
        p.pos.y = floor_y;
        if (p.vel.y < 0.0) {
            p.vel.y = -p.vel.y * 0.30;
        }
    }

    // Cylindrical glass wall collision
    if (r_xz > CHAMBER_RADIUS) {
        let normal_xz = normalize(p.pos.xz);
        p.pos.x = normal_xz.x * CHAMBER_RADIUS;
        p.pos.z = normal_xz.y * CHAMBER_RADIUS;
        let v_radial = dot(p.vel.xz, normal_xz);
        if (v_radial > 0.0) {
            p.vel.x -= 1.35 * v_radial * normal_xz.x;
            p.vel.z -= 1.35 * v_radial * normal_xz.y;
        }
    }

    // Top open ceiling cap
    if (p.pos.y > DOMAIN_MAX.y - 0.05) {
        p.pos.y = DOMAIN_MAX.y - 0.05;
        p.vel.y = -abs(p.vel.y) * 0.4;
    }

    particles[p_idx] = p;

    // 4. Splat Particle to 512x512 Render Heightfield Grid
    // World [-3.2, 3.2] -> [0, 512]
    let rx = ((p.pos.x + 3.2) / 6.4) * f32(RENDER_GRID_SIZE);
    let rz = ((p.pos.z + 3.2) / 6.4) * f32(RENDER_GRID_SIZE);

    let ix = i32(round(rx));
    let iz = i32(round(rz));

    let height_val = clamp(p.pos.y, 0.0, 3.0);

    // 5x5 bell-shaped kernel producing continuous organic water surface and droplets
    for (var dz = -2; dz <= 2; dz++) {
        let gz = iz + dz;
        if (gz < 0 || gz >= i32(RENDER_GRID_SIZE)) { continue; }

        for (var dx = -2; dx <= 2; dx++) {
            let gx = ix + dx;
            if (gx < 0 || gx >= i32(RENDER_GRID_SIZE)) { continue; }

            let d2 = f32(dx * dx + dz * dz);
            let falloff = max(0.0, 1.0 - d2 * 0.18);
            let splat_u = u32((height_val * falloff) * 1000.0);

            let cell = u32(gz) * RENDER_GRID_SIZE + u32(gx);
            atomicMax(&render_grid[cell], splat_u);
        }
    }
}
