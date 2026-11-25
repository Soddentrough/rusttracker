// INCLUDE: common

struct FlipDiscParams {
    impulses: array<vec4<f32>, 4>, // (gx, gy, radius, strength)
    audio_levels: vec4<f32>,       // (bass, mids, treble, overall)
    sim_info: vec4<f32>,           // (dt, time_s, aspect_ratio, vis_width)
}

@group(0) @binding(0) var<uniform> params: FlipDiscParams;
@group(0) @binding(1) var<storage, read> wave_in: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> wave_out: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> discs: array<vec4<f32>>;

const GRID_W: u32 = 96u;
const GRID_H: u32 = 64u;
const PI: f32 = 3.14159265;
const WAVE_C2: f32 = 0.30;
const RESTORE_K: f32 = 0.032;

@compute @workgroup_size(16, 16, 1)
fn cs_init(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= GRID_W || gid.y >= GRID_H) {
        return;
    }
    let idx = gid.y * GRID_W + gid.x;
    var h0 = 0.0;
    if (params.audio_levels.w > 0.05) {
        let p = vec2<f32>(f32(gid.x) + 0.5, f32(gid.y) + 0.5);
        let center = vec2<f32>(f32(GRID_W) * 0.5, f32(GRID_H) * 0.5);
        let d = p - center;
        let u = dot(d, d) / 7.5;
        h0 = (1.0 - 0.5 * u) * exp(-0.5 * u) * 1.1;
    }
    wave_out[idx] = vec4<f32>(h0, h0, abs(h0), 0.0);
    discs[idx] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
}

@compute @workgroup_size(16, 16, 1)
fn cs_wave_step(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= GRID_W || y >= GRID_H) {
        return;
    }

    let idx = y * GRID_W + x;
    let xm = max(x, 1u) - 1u;
    let xp = min(x + 1u, GRID_W - 1u);
    let ym = max(y, 1u) - 1u;
    let yp = min(y + 1u, GRID_H - 1u);

    let c = wave_in[idx];
    let h_curr = c.x;
    let h_prev = c.y;
    let env = c.z;

    let n = wave_in[ym * GRID_W + x].x;
    let s = wave_in[yp * GRID_W + x].x;
    let w = wave_in[y * GRID_W + xm].x;
    let e = wave_in[y * GRID_W + xp].x;
    let nw = wave_in[ym * GRID_W + xm].x;
    let ne = wave_in[ym * GRID_W + xp].x;
    let sw = wave_in[yp * GRID_W + xm].x;
    let se = wave_in[yp * GRID_W + xp].x;

    // 9-point isotropic Laplacian ensures circular wavefronts without axis-aligned diamonds
    let lap = (4.0 * (n + s + e + w) + (ne + nw + se + sw) - 20.0 * h_curr) / 6.0;

    // Outer 5-cell absorbing sponge prevents harsh boundary box reflections
    let edge_x = min(f32(x), f32(GRID_W - 1u - x));
    let edge_y = min(f32(y), f32(GRID_H - 1u - y));
    let sponge = smoothstep_r(5.0, 0.0, min(edge_x, edge_y));
    let damp = mix(0.022, 0.24, sponge);

    // Dispersive restoring term eliminates positive 2D wake DC buildup so troughs stay crisp charcoal
    var h_next = h_curr + (h_curr - h_prev) * (1.0 - damp) + WAVE_C2 * lap - RESTORE_K * h_curr;

    let cell_p = vec2<f32>(f32(x) + 0.5, f32(y) + 0.5);
    for (var k = 0u; k < 4u; k = k + 1u) {
        let imp = params.impulses[k];
        if (imp.w > 0.0) {
            let d = cell_p - imp.xy;
            let r = max(imp.z * 0.85, 0.65);
            let u = dot(d, d) / (r * r);
            if (u < 12.0) {
                // Zero-mean 2D Ricker wavelet injects crisp oscillating rings with zero DC drift
                let ricker = (1.0 - 0.5 * u) * exp(-0.5 * u);
                h_next += imp.w * ricker * 0.65;
            }
        }
    }

    h_next = clamp(h_next, -2.8, 2.8);
    let next_env = max(env * 0.94, abs(h_next));
    wave_out[idx] = vec4<f32>(h_next, h_curr, next_env, 0.0);
}

@compute @workgroup_size(16, 16, 1)
fn cs_disc_step(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= GRID_W || y >= GRID_H) {
        return;
    }

    let idx = y * GRID_W + x;
    let h = wave_in[idx].x;
    let dt = clamp(params.sim_info.x, 0.001, 0.033);

    let d_in = discs[idx];
    var angle = d_in.x;
    var vel = d_in.y;
    var target_side = d_in.z;
    var dwell = max(d_in.w - dt, 0.0);

    let cell_coord = vec2<f32>(f32(x), f32(y));
    let jitter = (hash21_crt(cell_coord + vec2<f32>(13.7, 29.1)) - 0.5) * 0.03;
    let speed_var = 0.88 + 0.24 * hash21_crt(cell_coord + vec2<f32>(71.3, 47.9));

    // Schmitt-trigger hysteresis + dwell cooldown avoids high-frequency chatter
    if (dwell <= 0.0) {
        if (target_side < 0.5 && h > 0.14 + jitter) {
            target_side = 1.0;
            dwell = 0.085;
        } else if (target_side >= 0.5 && h < 0.06 + jitter) {
            target_side = 0.0;
            dwell = 0.085;
        }
    }

    let goal = target_side * PI;
    let err = goal - angle;
    let drive = clamp(err * 18.0, -1.0, 1.0);
    let accel = drive * 480.0 * speed_var;

    vel = (vel + accel * dt) * exp(-3.5 * dt);
    let max_vel = 46.0 * speed_var;
    vel = clamp(vel, -max_vel, max_vel);
    angle += vel * dt;

    // Mechanical stop-pin rebound (~6-8 deg chatter before settling)
    if (angle >= PI) {
        angle = PI;
        if (vel > 1.2) {
            vel = -vel * 0.24;
        } else {
            vel = 0.0;
        }
    } else if (angle <= 0.0) {
        angle = 0.0;
        if (vel < -1.2) {
            vel = -vel * 0.24;
        } else {
            vel = 0.0;
        }
    }

    if (abs(goal - angle) < 0.003 && abs(vel) < 0.8) {
        angle = goal;
        vel = 0.0;
    }

    discs[idx] = vec4<f32>(angle, vel, target_side, dwell);
}
