// INCLUDE: common

// ============================================================================
// Quantum FLIP Ferrofluid Compute Shader
// Hybrid Eulerian-Lagrangian Incompressible Navier-Stokes Fluid Simulation
// with Audio-Driven Rosensweig Magnetic Body Forces & Heightfield Splatting
// ============================================================================

struct Particle {
    pos: vec4<f32>, // pos.xyz = world pos, pos.w = mass / density
    vel: vec4<f32>, // vel.xyz = world velocity, vel.w = smoothed audio spec
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

const DOMAIN_MIN: vec3<f32> = vec3<f32>(-3.6, 0.0, -3.6);
const DOMAIN_MAX: vec3<f32> = vec3<f32>(3.6, 2.4, 3.6);
const CELL_SIZE: vec3<f32> = vec3<f32>(0.1125, 0.075, 0.1125); // (7.2/64, 2.4/32, 7.2/64)
const INV_CELL_SIZE: vec3<f32> = vec3<f32>(8.888889, 13.333333, 8.888889);

const FIXED_SCALE: f32 = 512.0;
const DISH_RADIUS: f32 = 3.35;
const FLUID_DENSITY: f32 = 1.0;
const FLIP_RATIO: f32 = 0.95; // 95% FLIP, 5% PIC

// --- Bindings ---
@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(2) var<storage, read_write> grid_accum: array<CellAccum>;
// grid_vel_new stores (vel.xyz, cell_type): 0.0 = AIR, 1.0 = LIQUID, 2.0 = SOLID
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

    // Seed initial particle pool in a circular dish if uninitialized
    if (length(p.pos.xyz) < 0.001) {
        let fi = f32(p_idx);
        let r1 = fract(sin(fi * 12.9898 + 0.123) * 43758.5453);
        let r2 = fract(sin(fi * 78.233 + 0.456) * 43758.5453);
        let r3 = fract(sin(fi * 39.421 + 0.789) * 43758.5453);

        let angle = r1 * 6.2831853;
        let radius = sqrt(r2) * 2.85;
        let y_pos = 0.08 + r3 * 0.22;

        p.pos = vec4<f32>(cos(angle) * radius, y_pos, sin(angle) * radius, 1.0);
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
// PASS 3: Grid Normalization, External Magnetic Forces & Boundary Walls
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
    let dish_floor = 0.02 + 0.022 * (r_xz * r_xz);

    // Boundary classification: dish container walls
    if (r_xz > DISH_RADIUS || world_p.y < dish_floor) {
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

        // --- Apply External Forces on Fluid ---
        // 1. Gravity
        var force = vec3<f32>(0.0, -9.8, 0.0);

        // 2. Audio-reactive Rosensweig Electromagnetism
        // Central Sub-bass Coil with apex pull
        let sub_bass = clamp(audio.spectrum[0].x * 1.6 + audio.channels[0].x * 0.8, 0.0, 2.0);
        let center_apex = vec3<f32>(0.0, 0.45 + sub_bass * 0.85, 0.0);
        let center_dir = center_apex - world_p;
        let dist_sq_center = dot(center_dir, center_dir) * 0.4 + 0.22;
        let center_mag_strength = (sub_bass * 380.0 + 20.0) / dist_sq_center;
        force += normalize(center_dir) * center_mag_strength;

        // Azimuthal Spectrum Electromagnetic Ring (16 staggered poles in concentric hexagonal pattern)
        // Continuous angular interpolation between adjacent poles
        let azimuth = atan2(world_p.z, world_p.x);
        let norm_azimuth = (azimuth + 3.14159265) / 6.2831853; // [0, 1]
        let frac_pole = norm_azimuth * 16.0;
        let pole_idx0 = u32(floor(frac_pole)) % 16u;
        let pole_idx1 = (pole_idx0 + 1u) % 16u;
        let t_pole = fract(frac_pole);

        // Compute force from pole 0
        let pole_angle0 = (f32(pole_idx0) + 0.5) / 16.0 * 6.2831853 - 3.14159265;
        let is_outer0 = (pole_idx0 % 2u) == 1u;
        let pole_r0 = select(1.65, 2.65, is_outer0);
        let spec_bin0 = pole_idx0 * 8u + 4u;
        let spec0 = clamp(audio.spectrum[spec_bin0 / 4u][spec_bin0 % 4u], 0.0, 1.5);
        let pole_h0 = select(0.5 + spec0 * 0.85, 0.4 + spec0 * 0.7, is_outer0);
        let pole_pos0 = vec3<f32>(cos(pole_angle0) * pole_r0, pole_h0, sin(pole_angle0) * pole_r0);
        let pole_dir0 = pole_pos0 - world_p;
        let dist_sq0 = dot(pole_dir0, pole_dir0) + 0.22;
        let pole_force0 = (spec0 * 180.0 + 10.0) / dist_sq0;

        // Compute force from pole 1
        let pole_angle1 = (f32(pole_idx1) + 0.5) / 16.0 * 6.2831853 - 3.14159265;
        let is_outer1 = (pole_idx1 % 2u) == 1u;
        let pole_r1 = select(1.65, 2.65, is_outer1);
        let spec_bin1 = pole_idx1 * 8u + 4u;
        let spec1 = clamp(audio.spectrum[spec_bin1 / 4u][spec_bin1 % 4u], 0.0, 1.5);
        let pole_h1 = select(0.5 + spec1 * 0.85, 0.4 + spec1 * 0.7, is_outer1);
        let pole_pos1 = vec3<f32>(cos(pole_angle1) * pole_r1, pole_h1, sin(pole_angle1) * pole_r1);
        let pole_dir1 = pole_pos1 - world_p;
        let dist_sq1 = dot(pole_dir1, pole_dir1) + 0.22;
        let pole_force1 = (spec1 * 180.0 + 10.0) / dist_sq1;

        let f0 = normalize(pole_dir0) * pole_force0;
        let f1 = normalize(pole_dir1) * pole_force1;
        force += mix(f0, f1, smoothstep(0.0, 1.0, t_pole));

        // 3. Central Cohesion (Surface Tension restoring force)
        let dish_center_pull = vec3<f32>(-world_p.x, 0.0, -world_p.z);
        force += dish_center_pull * (2.8 + (1.0 - clamp(world_p.y * 0.8, 0.0, 1.0)) * 2.5);

        // Integrate forces
        vel += force * dt;

        // Viscous damping
        let dt_scale = dt / 0.016;
        vel *= pow(0.965, dt_scale);
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

fn sample_p_0(nx: i32, ny: i32, nz: i32, c_idx: u32) -> f32 {
    if (!cell_in_bounds(nx, ny, nz)) { return 0.0; }
    let n_idx = get_cell_idx(u32(nx), u32(ny), u32(nz));
    let n_type = grid_vel_new[n_idx].w;
    if (n_type == 0.0) { return 0.0; } // AIR
    if (n_type == 2.0) { return pressure_0[c_idx]; } // SOLID
    return pressure_0[n_idx];
}

fn sample_p_1(nx: i32, ny: i32, nz: i32, c_idx: u32) -> f32 {
    if (!cell_in_bounds(nx, ny, nz)) { return 0.0; }
    let n_idx = get_cell_idx(u32(nx), u32(ny), u32(nz));
    let n_type = grid_vel_new[n_idx].w;
    if (n_type == 0.0) { return 0.0; } // AIR
    if (n_type == 2.0) { return pressure_1[c_idx]; } // SOLID
    return pressure_1[n_idx];
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

    let cx = INV_CELL_SIZE.x * INV_CELL_SIZE.x;
    let cy = INV_CELL_SIZE.y * INV_CELL_SIZE.y;
    let cz = INV_CELL_SIZE.z * INV_CELL_SIZE.z;
    let c_diag = 2.0 * (cx + cy + cz);

    var p_sum = 0.0;
    p_sum += cx * (sample_p_0(gx + 1, gy, gz, idx) + sample_p_0(gx - 1, gy, gz, idx));
    p_sum += cy * (sample_p_0(gx, gy + 1, gz, idx) + sample_p_0(gx, gy - 1, gz, idx));
    p_sum += cz * (sample_p_0(gx, gy, gz + 1, idx) + sample_p_0(gx, gy, gz - 1, idx));

    let p_rhs = (FLUID_DENSITY / dt) * div;
    pressure_1[idx] = (p_sum - p_rhs) / c_diag;
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

    let cx = INV_CELL_SIZE.x * INV_CELL_SIZE.x;
    let cy = INV_CELL_SIZE.y * INV_CELL_SIZE.y;
    let cz = INV_CELL_SIZE.z * INV_CELL_SIZE.z;
    let c_diag = 2.0 * (cx + cy + cz);

    var p_sum = 0.0;
    p_sum += cx * (sample_p_1(gx + 1, gy, gz, idx) + sample_p_1(gx - 1, gy, gz, idx));
    p_sum += cy * (sample_p_1(gx, gy + 1, gz, idx) + sample_p_1(gx, gy - 1, gz, idx));
    p_sum += cz * (sample_p_1(gx, gy, gz + 1, idx) + sample_p_1(gx, gy, gz - 1, idx));

    let p_rhs = (FLUID_DENSITY / dt) * div;
    pressure_0[idx] = (p_sum - p_rhs) / c_diag;
}

// ----------------------------------------------------------------------------
// PASS 5: Velocity Projection (Subtract Pressure Gradient)
// ----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn cs_grid_project(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= TOTAL_CELLS) { return; }

    var vel_info = grid_vel_new[idx];
    if (vel_info.w != 1.0) { return; } // Only project LIQUID cells

    let gx = i32(idx % GRID_X);
    let gy = i32((idx / GRID_X) % GRID_Y);
    let gz = i32(idx / (GRID_X * GRID_Y));

    let dp_dx = (sample_p_0(gx + 1, gy, gz, idx) - sample_p_0(gx - 1, gy, gz, idx)) * 0.5 * INV_CELL_SIZE.x;
    let dp_dy = (sample_p_0(gx, gy + 1, gz, idx) - sample_p_0(gx, gy - 1, gz, idx)) * 0.5 * INV_CELL_SIZE.y;
    let dp_dz = (sample_p_0(gx, gy, gz + 1, idx) - sample_p_0(gx, gy, gz - 1, idx)) * 0.5 * INV_CELL_SIZE.z;

    let dt = clamp(audio.frame_dt, 0.001, 0.033);
    let scale = dt / FLUID_DENSITY;

    vel_info.x -= dp_dx * scale;
    vel_info.y -= dp_dy * scale;
    vel_info.z -= dp_dz * scale;

    grid_vel_new[idx] = vel_info;
}

// ----------------------------------------------------------------------------
// PASS 6: Grid-to-Particle (G2P), Particle Advection & Heightfield Splat
// ----------------------------------------------------------------------------
fn sample_grid_vel(p: vec3<f32>, use_old: bool) -> vec3<f32> {
    let cell_pos = world_to_cell(p) - vec3<f32>(0.5);
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

    // Blended FLIP/PIC update (95% FLIP prevents numerical dissipation, 5% PIC stabilizes)
    p.vel = vec4<f32>(mix(v_pic, v_flip, FLIP_RATIO), p.vel.w);

    // 2. Advect Particle Position
    p.pos = vec4<f32>(p.pos.xyz + p.vel.xyz * dt, p.pos.w);

    // 3. Boundary Collisions (Parabolic Dish Container)
    let r_xz = length(p.pos.xz);
    let floor_y = 0.025 + 0.022 * (r_xz * r_xz);

    if (p.pos.y < floor_y) {
        p.pos.y = floor_y;
        if (p.vel.y < 0.0) {
            p.vel.y = -p.vel.y * 0.35;
        }
    }

    if (r_xz > DISH_RADIUS) {
        let normal_xz = normalize(p.pos.xz);
        p.pos.x = normal_xz.x * DISH_RADIUS;
        p.pos.z = normal_xz.y * DISH_RADIUS;
        let v_radial = dot(p.vel.xz, normal_xz);
        if (v_radial > 0.0) {
            p.vel.x -= 1.35 * v_radial * normal_xz.x;
            p.vel.z -= 1.35 * v_radial * normal_xz.y;
        }
    }

    // Cap ceiling
    if (p.pos.y > DOMAIN_MAX.y - 0.05) {
        p.pos.y = DOMAIN_MAX.y - 0.05;
        p.vel.y = -abs(p.vel.y) * 0.5;
    }

    particles[p_idx] = p;

    // 4. Splat Particle to 512x512 Render Heightfield
    // World [-3.6, 3.6] -> [0, 512]
    let rx = ((p.pos.x + 3.6) / 7.2) * f32(RENDER_GRID_SIZE);
    let rz = ((p.pos.z + 3.6) / 7.2) * f32(RENDER_GRID_SIZE);

    let ix = i32(round(rx));
    let iz = i32(round(rz));

    let height_val = clamp(p.pos.y, 0.0, 3.0);

    // 5x5 splat with smooth bell-shaped falloff to produce a continuous, organic liquid cone
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
