// INCLUDE: common

// ============================================================================
// Optical Glass Water Basin — FLIP Fluid Compute Shader
// Hybrid Eulerian-Lagrangian Incompressible Navier-Stokes Fluid Solver
// 3D Glass Letter Hydrodynamic Slam, Acoustic Transducers, Faraday Waves & Splatting
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

const DOMAIN_MIN_23: vec3<f32> = vec3<f32>(-16.0, -0.8, -10.0);
const DOMAIN_MAX_23: vec3<f32> = vec3<f32>(16.0, 3.2, 10.0);
const CELL_SIZE_23: vec3<f32> = vec3<f32>(0.5, 0.125, 0.3125); // (32.0/64, 4.0/32, 20.0/64)
const INV_CELL_SIZE_23: vec3<f32> = vec3<f32>(2.0, 8.0, 3.2);

const DOMAIN_MIN_28: vec3<f32> = vec3<f32>(-6.4, 0.0, -6.4);
const DOMAIN_MAX_28: vec3<f32> = vec3<f32>(6.4, 2.4, 6.4);
const CELL_SIZE_28: vec3<f32> = vec3<f32>(0.2, 0.075, 0.2); // (12.8/64, 2.4/32, 12.8/64)
const INV_CELL_SIZE_28: vec3<f32> = vec3<f32>(5.0, 13.333333, 5.0);
const CHAMBER_RADIUS: f32 = 6.0;

const FIXED_SCALE: f32 = 512.0;
const FLUID_DENSITY: f32 = 1.0;
const FLIP_RATIO: f32 = 0.95; // 95% FLIP, 5% PIC
const SOR_OMEGA: f32 = 1.28; // Stable damped Jacobi over-relaxation factor (omega < 4/3)

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
fn is_lyrics_mode() -> bool {
    return audio.mode == 23u;
}

fn get_domain_min() -> vec3<f32> {
    return select(DOMAIN_MIN_28, DOMAIN_MIN_23, is_lyrics_mode());
}

fn get_domain_max() -> vec3<f32> {
    return select(DOMAIN_MAX_28, DOMAIN_MAX_23, is_lyrics_mode());
}

fn get_cell_size() -> vec3<f32> {
    return select(CELL_SIZE_28, CELL_SIZE_23, is_lyrics_mode());
}

fn get_inv_cell_size() -> vec3<f32> {
    return select(INV_CELL_SIZE_28, INV_CELL_SIZE_23, is_lyrics_mode());
}

fn get_cell_idx(x: u32, y: u32, z: u32) -> u32 {
    return x + y * GRID_X + z * (GRID_X * GRID_Y);
}

fn cell_in_bounds(x: i32, y: i32, z: i32) -> bool {
    return x >= 0 && x < i32(GRID_X) && y >= 0 && y < i32(GRID_Y) && z >= 0 && z < i32(GRID_Z);
}

fn world_to_cell(p: vec3<f32>) -> vec3<f32> {
    return (p - get_domain_min()) * get_inv_cell_size();
}

fn cell_to_world(c: vec3<f32>) -> vec3<f32> {
    return get_domain_min() + (c + vec3<f32>(0.5)) * get_cell_size();
}

fn get_lyric_param(idx: u32) -> f32 {
    let vec_idx = idx / 4u;
    let comp_idx = idx % 4u;
    return audio.fire_heat[vec_idx][comp_idx];
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

    if (is_lyrics_mode()) {
        // Seed initial particle pool in the expansive lyrics water stage if uninitialized or switched from another mode
        if (length(p.pos.xyz) < 0.001 || abs(p.pos.w - 1.23) > 0.01) {
            let fi = f32(p_idx);
            let r1 = fract(sin(fi * 12.9898 + 0.123) * 43758.5453);
            let r2 = fract(sin(fi * 78.233 + 0.456) * 43758.5453);
            let r3 = fract(sin(fi * 39.421 + 0.789) * 43758.5453);

            var x = (r1 - 0.5) * 28.0;
            var z = (r2 - 0.5) * 14.0;
            // Concentrate 80% of particles in the active foreground letter & splash zone
            if (p_idx % 5u < 4u) {
                x = (r1 - 0.5) * 12.0;
                z = (r2 - 0.5) * 3.5;
            }
            let y = -0.70 + r3 * 0.52; // Resting submerged below water line (y in [-0.70, -0.18])

            p.pos = vec4<f32>(x, y, z, 1.23);
            p.vel = vec4<f32>(0.0);
            particles[p_idx] = p;
        }
    } else {
        // Seed initial particle pool across the open water domain (ID 28) if uninitialized or switched from another mode
        if (length(p.pos.xyz) < 0.001 || abs(p.pos.w - 1.333) > 0.01) {
            let fi = f32(p_idx);
            let r1 = fract(sin(fi * 12.9898 + 0.123) * 43758.5453);
            let r2 = fract(sin(fi * 78.233 + 0.456) * 43758.5453);
            let r3 = fract(sin(fi * 39.421 + 0.789) * 43758.5453);

            let angle = r1 * 6.2831853;
            let radius = sqrt(r2) * 5.8;
            let y_pos = 0.12 + r3 * 0.30; // Resting water level ~42cm

            p.pos = vec4<f32>(cos(angle) * radius, y_pos, sin(angle) * radius, 1.333);
            p.vel = vec4<f32>(0.0);
            particles[p_idx] = p;
        }
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
// PASS 3: Grid Normalization, Acoustic Forces & Hydrodynamic Coupling
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

    if (is_lyrics_mode()) {
        // Boundary classification: expansive rectangular water basin walls and floor
        if (abs(world_p.x) > 15.6 || abs(world_p.z) > 9.6 || world_p.y < -0.55) {
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

            // --- 1. Downward Gravity ---
            var force = vec3<f32>(0.0, -9.8, 0.0);

            // --- 2. 3D Glass Letter Hydrodynamic Slam & Buoyancy Coupling ---
            let slam_elapsed = get_lyric_param(0u);
            let slam_y = get_lyric_param(1u);
            let bass = get_lyric_param(6u);
            let text_half_w = max(2.5, get_lyric_param(7u));

            let dx_letter = abs(world_p.x);
            let dz_letter = abs(world_p.z);

            if (dx_letter < text_half_w + 0.45 && dz_letter < 0.75) {
                if (slam_elapsed > 0.25 && slam_elapsed < 0.75) {
                    let dt_impact = slam_elapsed - 0.25;
                    let impact_decay = exp(-dt_impact * 6.5);

                    if (dz_letter < 0.32 && world_p.y < slam_y + 0.15) {
                        force.y -= 45.0 * impact_decay;
                    }

                    let z_sign = sign(world_p.z);
                    force.y += 32.0 * impact_decay * smoothstep(0.15, 0.45, dz_letter);
                    force.z += z_sign * 24.0 * impact_decay * smoothstep(0.1, 0.45, dz_letter);
                    if (dx_letter > text_half_w * 0.75) {
                        force.y += 24.0 * impact_decay;
                        force.x += sign(world_p.x) * 20.0 * impact_decay;
                    }
                } else if (slam_elapsed >= 0.75) {
                    let bob_v = cos(audio.time * 2.2) * 0.4 * (1.0 + bass * 0.5);
                    if (world_p.y < 0.05) {
                        force.y += bob_v * 3.5;
                        force.z += sign(world_p.z) * abs(bob_v) * 2.5;
                    }
                }
            }

            // --- 3. 4-Channel Submerged Acoustic Transducers in Basin Floor ---
            let tx_coords = array<f32, 4>(-7.0, -2.3, 2.3, 7.0);
            for (var k = 0u; k < 4u; k++) {
                let tx = tx_coords[k];
                let dist_t2 = (world_p.x - tx) * (world_p.x - tx) + world_p.z * world_p.z;
                let ch_val = select(select(select(audio.channels[0].w, audio.channels[0].z, k == 2u), audio.channels[0].y, k == 1u), audio.channels[0].x, k == 0u);
                let ch_amp = clamp(ch_val, 0.0, 1.5);
                let falloff = exp(-dist_t2 * 0.35);
                let transducer_force = ch_amp * 4.5 * falloff * smoothstep(0.1, -0.4, world_p.y);
                force.y += transducer_force;
            }

            // --- 4. Faraday Acoustic Surface Wave Harmonics (normalized from [0, 100] scale) ---
            let mids = clamp((audio.spectrum[2].x + audio.spectrum[4].x) / 110.0, 0.0, 1.4);
            let highs = clamp((audio.spectrum[8].x + audio.spectrum[12].x) / 110.0, 0.0, 1.2);
            let standing_wave = sin(world_p.x * 1.2 + audio.time * 3.5) * cos(world_p.z * 1.8 - audio.time * 2.8);
            force.y += standing_wave * (mids * 1.8 + highs * 1.2) * smoothstep(-0.15, 0.05, world_p.y);

            // --- 5. Surface Tension & Equilibrium Level Restoring ---
            let excess_h = world_p.y - (-0.18);
            force.y -= excess_h * 24.0;

            vel += force * dt;
            let dt_scale = dt / 0.016;
            vel *= pow(0.985, dt_scale);
        }
    } else {
        let r_xz = length(world_p.xz);

        // Boundary classification: open domain outer margin and seabed floor (ID 28)
        if (r_xz > CHAMBER_RADIUS || world_p.y < 0.08) {
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

            var force = vec3<f32>(0.0, -9.8, 0.0);

            // Cohesive Acoustic Sub-bass Fountain Swell & Toroidal Roll (supports both [0..100] and [0..2] scales)
            let spec0 = select(audio.spectrum[0].x / 55.0, audio.spectrum[0].x, audio.spectrum[0].x > 0.0 && audio.spectrum[0].x <= 3.0);
            let sub_bass = clamp(spec0 + audio.channels[0].x * 0.85, 0.0, 2.2);
            let jet_falloff = exp(-r_xz * r_xz * 0.85);
            let geyser_impulse = sub_bass * 54.0 * jet_falloff * smoothstep(0.72, 0.10, world_p.y);
            force.y += geyser_impulse;

            let spread_factor = smoothstep(0.36, 0.78, world_p.y) * sub_bass * 18.0 * exp(-r_xz * 0.35);
            if (r_xz > 0.01) {
                force.x += (world_p.x / r_xz) * spread_factor;
                force.z += (world_p.z / r_xz) * spread_factor;
            }

            // Faraday Acoustic Surface Waves & Open Ocean Harmonic Excitation
            let azimuth = atan2(world_p.z, world_p.x);
            let raw_m = audio.spectrum[8].x + audio.spectrum[16].x;
            let raw_h = audio.spectrum[32].x + audio.spectrum[48].x;
            let mids = clamp(select(raw_m / 110.0, raw_m * 0.5, raw_m > 0.0 && raw_m <= 4.0) + audio.channels[0].y * 0.4, 0.0, 1.6);
            let highs = clamp(select(raw_h / 110.0, raw_h * 0.5, raw_h > 0.0 && raw_h <= 4.0) + audio.channels[0].x * 0.3, 0.0, 1.4);

            let standing_wave = cos(r_xz * 3.8 - audio.time * 3.8) * cos(azimuth * 4.0)
                              + 0.5 * sin(r_xz * 6.2 + audio.time * 2.9) * cos(azimuth * 6.0);
            let ripple_force = standing_wave * (mids * 14.0 + highs * 10.0) * smoothstep(0.14, 0.45, world_p.y);
            force.y += ripple_force;

            // Smooth hydrostatic equilibrium restoring force around resting level (0.42m)
            let excess_h = world_p.y - 0.42;
            force.y -= excess_h * 26.0;

            vel += force * dt;
            let dt_scale = dt / 0.016;
            vel *= pow(0.975, dt_scale);
        }
    }

    grid_vel_new[idx] = vec4<f32>(vel, cell_type);
    grid_vel_old[idx] = vec4<f32>(vel, cell_type);
}

// ----------------------------------------------------------------------------
// PASS 4A & 4B: Anisotropic Jacobi Incompressibility Pressure Solver (Ping-Pong)
// ----------------------------------------------------------------------------
fn compute_divergence(gx: i32, gy: i32, gz: i32) -> f32 {
    let inv_cell = get_inv_cell_size();
    let inv_2dx = 0.5 * inv_cell.x;
    let inv_2dy = 0.5 * inv_cell.y;
    let inv_2dz = 0.5 * inv_cell.z;

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

    let inv_c = get_inv_cell_size();
    let wx = inv_c.x * inv_c.x;
    let wy = inv_c.y * inv_c.y;
    let wz = inv_c.z * inv_c.z;

    var weighted_sum_p = 0.0;
    var weight_diag = 0.0;

    let neighbors = array<vec3<i32>, 6>(
        vec3<i32>(gx + 1, gy, gz), vec3<i32>(gx - 1, gy, gz),
        vec3<i32>(gx, gy + 1, gz), vec3<i32>(gx, gy - 1, gz),
        vec3<i32>(gx, gy, gz + 1), vec3<i32>(gx, gy, gz - 1)
    );
    let weights = array<f32, 6>(wx, wx, wy, wy, wz, wz);

    for (var i = 0; i < 6; i++) {
        let nb = neighbors[i];
        let w = weights[i];
        if (cell_in_bounds(nb.x, nb.y, nb.z)) {
            let n_idx = get_cell_idx(u32(nb.x), u32(nb.y), u32(nb.z));
            let n_type = grid_vel_new[n_idx].w;
            if (n_type == 1.0) {
                weighted_sum_p += w * pressure_0[n_idx];
                weight_diag += w;
            } else if (n_type == 2.0) {
                weighted_sum_p += w * pressure_0[idx];
                weight_diag += w;
            } else {
                // Free-surface (Dirichlet p = 0 in AIR)
                weight_diag += w;
            }
        }
    }

    if (weight_diag > 0.0) {
        let p_jacobi = (weighted_sum_p - rhs) / weight_diag;
        pressure_1[idx] = mix(pressure_0[idx], p_jacobi, SOR_OMEGA);
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

    let inv_c = get_inv_cell_size();
    let wx = inv_c.x * inv_c.x;
    let wy = inv_c.y * inv_c.y;
    let wz = inv_c.z * inv_c.z;

    var weighted_sum_p = 0.0;
    var weight_diag = 0.0;

    let neighbors = array<vec3<i32>, 6>(
        vec3<i32>(gx + 1, gy, gz), vec3<i32>(gx - 1, gy, gz),
        vec3<i32>(gx, gy + 1, gz), vec3<i32>(gx, gy - 1, gz),
        vec3<i32>(gx, gy, gz + 1), vec3<i32>(gx, gy, gz - 1)
    );
    let weights = array<f32, 6>(wx, wx, wy, wy, wz, wz);

    for (var i = 0; i < 6; i++) {
        let nb = neighbors[i];
        let w = weights[i];
        if (cell_in_bounds(nb.x, nb.y, nb.z)) {
            let n_idx = get_cell_idx(u32(nb.x), u32(nb.y), u32(nb.z));
            let n_type = grid_vel_new[n_idx].w;
            if (n_type == 1.0) {
                weighted_sum_p += w * pressure_1[n_idx];
                weight_diag += w;
            } else if (n_type == 2.0) {
                weighted_sum_p += w * pressure_1[idx];
                weight_diag += w;
            } else {
                // Free-surface (Dirichlet p = 0 in AIR)
                weight_diag += w;
            }
        }
    }

    if (weight_diag > 0.0) {
        let p_jacobi = (weighted_sum_p - rhs) / weight_diag;
        pressure_0[idx] = mix(pressure_1[idx], p_jacobi, SOR_OMEGA);
    } else {
        pressure_0[idx] = 0.0;
    }
}

// ----------------------------------------------------------------------------
// PASS 5: Velocity Projection (Subtract Pressure Gradient)
// ----------------------------------------------------------------------------
fn sample_neighbor_pressure(nx: i32, ny: i32, nz: i32, p_center: f32) -> f32 {
    if (!cell_in_bounds(nx, ny, nz)) {
        return p_center;
    }
    let n_idx = get_cell_idx(u32(nx), u32(ny), u32(nz));
    let n_type = grid_vel_new[n_idx].w;
    if (n_type == 2.0) {
        // SOLID wall: Neumann boundary condition (dp/dn = 0)
        return p_center;
    }
    return pressure_0[n_idx];
}

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
    let inv_c = get_inv_cell_size();

    let p_c = pressure_0[idx];
    let p_xp = sample_neighbor_pressure(gx + 1, gy, gz, p_c);
    let p_xm = sample_neighbor_pressure(gx - 1, gy, gz, p_c);
    let p_yp = sample_neighbor_pressure(gx, gy + 1, gz, p_c);
    let p_ym = sample_neighbor_pressure(gx, gy - 1, gz, p_c);
    let p_zp = sample_neighbor_pressure(gx, gy, gz + 1, p_c);
    let p_zm = sample_neighbor_pressure(gx, gy, gz - 1, p_c);

    let grad_p = vec3<f32>(
        (p_xp - p_xm) * inv_c.x * inv_2rho,
        (p_yp - p_ym) * inv_c.y * inv_2rho,
        (p_zp - p_zm) * inv_c.z * inv_2rho
    );

    var v = cell_info.xyz - grad_p * dt;
    grid_vel_new[idx] = vec4<f32>(v, 1.0);
}

// ----------------------------------------------------------------------------
// PASS 6: G2P Interpolation, Particle Advection & Splatting
// ----------------------------------------------------------------------------
fn sample_grid_vel(pos: vec3<f32>, use_old: bool) -> vec4<f32> {
    let cell_pos = world_to_cell(pos) - vec3<f32>(0.5);
    let base_i = vec3<i32>(floor(cell_pos));
    let frac = fract(cell_pos);

    var vel = vec3<f32>(0.0);
    var total_w = 0.0;
    var liquid_w = 0.0;

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
                    let cell_v = select(grid_vel_new[c_idx], grid_vel_old[c_idx], use_old);
                    vel += cell_v.xyz * w;
                    total_w += w;
                    if (cell_v.w == 1.0) {
                        liquid_w += w;
                    }
                }
            }
        }
    }

    if (total_w > 0.0001) {
        return vec4<f32>(vel / total_w, liquid_w);
    }
    return vec4<f32>(0.0);
}

@compute @workgroup_size(256)
fn cs_g2p_advect_splat(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let p_idx = global_id.x;
    if (p_idx >= NUM_PARTICLES) { return; }

    var p = particles[p_idx];
    let dt = clamp(audio.frame_dt, 0.001, 0.033);
    let dom_max = get_domain_max();

    // 1. G2P Interpolation
    let v_new_sample = sample_grid_vel(p.pos.xyz, false);
    let v_old_sample = sample_grid_vel(p.pos.xyz, true);
    let v_new = v_new_sample.xyz;
    let v_old = v_old_sample.xyz;
    let ballistic_mask = 1.0 - clamp(v_new_sample.w, 0.0, 0.85);

    let v_flip = p.vel.xyz + (v_new - v_old);
    let v_pic = v_new;

    var v_cur = mix(v_pic, v_flip, FLIP_RATIO);

    if (is_lyrics_mode()) {
        // Ballistic gravity & surface restoring buoyancy for airborne/surface particles
        v_cur.y -= 9.8 * dt * ballistic_mask;
        let excess_h = max(0.0, p.pos.y - (-0.15));
        v_cur.y -= excess_h * 10.0 * dt * ballistic_mask;

        p.vel = vec4<f32>(v_cur, p.vel.w);
        p.pos = vec4<f32>(p.pos.xyz + p.vel.xyz * dt, p.pos.w);

        // Rectangular Basin Boundary Collisions
        let floor_y = -0.55;
        if (p.pos.y < floor_y) {
            p.pos.y = floor_y;
            if (p.vel.y < 0.0) {
                p.vel.y = -p.vel.y * 0.25;
            }
        }

        if (abs(p.pos.x) > 15.6) {
            p.pos.x = sign(p.pos.x) * 15.6;
            p.vel.x = -p.vel.x * 0.35;
        }

        if (abs(p.pos.z) > 9.6) {
            p.pos.z = sign(p.pos.z) * 9.6;
            p.vel.z = -p.vel.z * 0.35;
        }

        if (p.pos.y > dom_max.y - 0.05) {
            p.pos.y = dom_max.y - 0.05;
            p.vel.y = -abs(p.vel.y) * 0.4;
        }

        // Physical Letterform Collision & Hydrodynamic Splash Launch
        let slam_elapsed = get_lyric_param(0u);
        let slam_y = get_lyric_param(1u);
        let text_half_w = max(2.5, get_lyric_param(7u));

        let dx_letter = abs(p.pos.x);
        let dz_letter = abs(p.pos.z);

        if (slam_elapsed > 0.25 && slam_elapsed < 0.70) {
            let dt_impact = slam_elapsed - 0.25;
            if (dx_letter < text_half_w + 0.35 && dz_letter < 0.42 && p.pos.y > slam_y - 0.08 && p.pos.y < slam_y + 0.40) {
                let fi = f32(p_idx);
                let rnd_y = fract(sin(fi * 17.13 + 0.2) * 43758.5453);
                let rnd_x = fract(sin(fi * 31.41 + 0.5) * 43758.5453);
                let rnd_z = fract(sin(fi * 67.89 + 0.8) * 43758.5453);

                let z_dir = select(1.0, -1.0, p.pos.z < 0.0);
                p.pos.z = z_dir * (0.38 + rnd_z * 0.18);
                p.pos.y = max(p.pos.y, 0.04);

                let launch_speed = (1.0 - smoothstep(0.0, 0.32, dt_impact)) * 2.6;
                p.vel.y = launch_speed * (0.65 + rnd_y * 0.45);
                p.vel.z = z_dir * launch_speed * (0.45 + rnd_z * 0.35);
                p.vel.x += (rnd_x - 0.5) * launch_speed * 0.5;
            }
        }

        particles[p_idx] = p;

        // Splat both FLIP surface elevation (upper 16 bits, millimeter height above floor)
        // and airborne splash density (lower 16 bits) into the 512x512 Render Grid
        let rx = ((p.pos.x + 16.0) / 32.0) * f32(RENDER_GRID_SIZE);
        let rz = ((p.pos.z + 10.0) / 20.0) * f32(RENDER_GRID_SIZE);
        let ix = i32(round(rx));
        let iz = i32(round(rz));

        if (ix >= 0 && ix < i32(RENDER_GRID_SIZE) && iz >= 0 && iz < i32(RENDER_GRID_SIZE)) {
            let cell = u32(iz) * RENDER_GRID_SIZE + u32(ix);
            let h_mm = u32(clamp((p.pos.y + 0.55) * 1000.0, 0.0, 3500.0));
            atomicMax(&render_grid[cell], h_mm << 16u);
            if (p.pos.y > 0.05) {
                atomicAdd(&render_grid[cell], 1u);
            }
        }
    } else {
        // Visualizer ID 28: Open Water / Ocean Surface
        let r_p = length(p.pos.xz);

        // Cohesive Acoustic Sub-bass Fountain Swell (normalized from [0, 100] or [0, 2] scale)
        let spec0 = select(audio.spectrum[0].x / 55.0, audio.spectrum[0].x, audio.spectrum[0].x > 0.0 && audio.spectrum[0].x <= 3.0);
        let sub_bass = clamp(spec0 + audio.channels[0].x * 0.85, 0.0, 2.2);
        let jet_falloff = exp(-r_p * r_p * 1.1);
        let geyser_kick = sub_bass * 2.4 * jet_falloff * smoothstep(0.68, 0.12, p.pos.y);
        v_cur.y += geyser_kick * (dt / 0.016);

        if (r_p > 0.01 && p.pos.y > 0.38) {
            let spread = smoothstep(0.38, 0.85, p.pos.y) * sub_bass * 1.8 * exp(-r_p * 0.35) * (dt / 0.016);
            v_cur.x += (p.pos.x / r_p) * spread;
            v_cur.z += (p.pos.z / r_p) * spread;
        }

        // Faraday standing waves ripple impulse
        let azimuth = atan2(p.pos.z, p.pos.x);
        let raw_m = audio.spectrum[8].x + audio.spectrum[16].x;
        let raw_h = audio.spectrum[32].x + audio.spectrum[48].x;
        let mids = clamp(select(raw_m / 110.0, raw_m * 0.5, raw_m > 0.0 && raw_m <= 4.0) + audio.channels[0].y * 0.4, 0.0, 1.6);
        let highs = clamp(select(raw_h / 110.0, raw_h * 0.5, raw_h > 0.0 && raw_h <= 4.0) + audio.channels[0].x * 0.3, 0.0, 1.4);
        let standing_wave = cos(r_p * 3.8 - audio.time * 3.8) * cos(azimuth * 4.0);
        v_cur.y += standing_wave * (mids * 0.65 + highs * 0.45) * smoothstep(0.15, 0.48, p.pos.y) * (dt / 0.016);

        // Restoring gravity & equilibrium surface tension around resting level (0.42m)
        v_cur.y -= 9.8 * dt * ballistic_mask;
        let excess_h = p.pos.y - 0.42;
        v_cur.y -= excess_h * 14.0 * dt;

        p.vel = vec4<f32>(v_cur, p.vel.w);
        p.pos = vec4<f32>(p.pos.xyz + p.vel.xyz * dt, p.pos.w);

        let r_xz = length(p.pos.xz);
        let floor_y = 0.09;

        if (p.pos.y < floor_y) {
            p.pos.y = floor_y;
            if (p.vel.y < 0.0) {
                p.vel.y = -p.vel.y * 0.25;
            }
        }

        if (r_xz > CHAMBER_RADIUS) {
            let normal_xz = normalize(p.pos.xz);
            p.pos.x = normal_xz.x * CHAMBER_RADIUS;
            p.pos.z = normal_xz.y * CHAMBER_RADIUS;
            let v_radial = dot(p.vel.xz, normal_xz);
            if (v_radial > 0.0) {
                p.vel.x -= 1.15 * v_radial * normal_xz.x;
                p.vel.z -= 1.15 * v_radial * normal_xz.y;
            }
        }

        if (p.pos.y > 0.92) {
            p.pos.y = 0.92;
            p.vel.y = -abs(p.vel.y) * 0.35;
        }

        particles[p_idx] = p;

        // Skip 5x5 atomicMax splat for deeply submerged bottom-layer particles
        if (p.pos.y <= 0.36) {
            return;
        }

        // Splat Particle to 512x512 Render Heightfield Grid (World [-6.4, 6.4] -> [0, 512])
        let rx = ((p.pos.x + 6.4) / 12.8) * f32(RENDER_GRID_SIZE);
        let rz = ((p.pos.z + 6.4) / 12.8) * f32(RENDER_GRID_SIZE);

        let ix = i32(round(rx));
        let iz = i32(round(rz));

        // Splat only excess elevation above resting level with a C1-continuous zero-boundary kernel
        let excess = clamp(p.pos.y - 0.38, 0.0, 0.45);
        if (excess <= 0.002) {
            return;
        }

        for (var dz = -2; dz <= 2; dz++) {
            let gz = iz + dz;
            if (gz < 0 || gz >= i32(RENDER_GRID_SIZE)) { continue; }

            for (var dx = -2; dx <= 2; dx++) {
                let gx = ix + dx;
                if (gx < 0 || gx >= i32(RENDER_GRID_SIZE)) { continue; }

                let d2 = f32(dx * dx + dz * dz);
                let w = max(0.0, 1.0 - d2 / 9.0);
                let falloff = w * w;
                let splat_u = u32((excess * falloff) * 1000.0);

                if (splat_u > 0u) {
                    let cell = u32(gz) * RENDER_GRID_SIZE + u32(gx);
                    atomicMax(&render_grid[cell], splat_u);
                }
            }
        }
    }
}
