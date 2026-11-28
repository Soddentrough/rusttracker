// INCLUDE: common

struct FlipDiscParams {
    impulses: array<vec4<f32>, 4>,
    audio_levels: vec4<f32>, // (bass, mids, treble, overall)
    sim_info: vec4<f32>,     // (dt, time_s, aspect_ratio, vis_width)
    mode_info: vec4<f32>,    // (mode_id, vis_height, stereo_bias, reset_flag)
    bands: array<vec4<f32>, 16>,
    peaks: array<vec4<f32>, 16>,
    agents: array<vec4<f32>, 24>,
}

@group(0) @binding(0) var<uniform> params: FlipDiscParams;
@group(0) @binding(1) var<storage, read> wave_in: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> wave_out: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> discs: array<vec4<f32>>;

const GRID_W: u32 = 128u;
const GRID_H: u32 = 96u;
const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318530;
const WAVE_C2: f32 = 0.30;
const RESTORE_K: f32 = 0.032;

fn sample_band(idx: u32) -> f32 {
    let clamped = min(idx, 63u);
    let v = params.bands[clamped >> 2u];
    switch (clamped & 3u) {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

fn sample_peak(idx: u32) -> f32 {
    let clamped = min(idx, 63u);
    let v = params.peaks[clamped >> 2u];
    switch (clamped & 3u) {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

@compute @workgroup_size(16, 16, 1)
fn cs_init(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= GRID_W || gid.y >= GRID_H) {
        return;
    }
    let idx = gid.y * GRID_W + gid.x;
    var h0 = 0.0;
    if (u32(round(params.mode_info.x)) == 0u && params.audio_levels.w > 0.05) {
        let p = vec2<f32>(f32(gid.x) + 0.5, f32(gid.y) + 0.5);
        let center = vec2<f32>(f32(GRID_W) * 0.5, f32(GRID_H) * 0.5);
        let d = p - center;
        let u = dot(d, d) / 9.5;
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
    if (params.mode_info.w > 0.5) {
        wave_out[idx] = vec4<f32>(0.0);
        return;
    }

    let mode = u32(round(params.mode_info.x));
    let cell_p = vec2<f32>(f32(x) + 0.5, f32(y) + 0.5);
    let center = vec2<f32>(f32(GRID_W) * 0.5, f32(GRID_H) * 0.5);
    let vis_w = max(params.sim_info.w, 1.0);
    let vis_h = max(params.mode_info.y, 1.0);
    let overall = params.audio_levels.w;

    var h_next = 0.0;
    let c = wave_in[idx];
    let h_curr = c.x;

    switch (mode) {
        case 1u: {
            // Flocking murmuration: smoothly decaying trails behind steering boids
            h_next = h_curr * select(0.78, 0.885, overall >= 0.015);
            for (var k = 0u; k < 24u; k = k + 1u) {
                let b = params.agents[k];
                if (b.w > 0.001) {
                    let d = cell_p - b.xy;
                    let r = max(b.z, 0.6);
                    let u = dot(d, d) / (r * r);
                    if (u < 6.0) {
                        h_next = max(h_next, exp(-u) * b.w);
                    }
                }
            }
        }
        case 2u: {
            // Sonar Sweep: clockwise rotating radar arm imprinting radial spectrum arcs
            let dec = select(0.86, 0.966, overall >= 0.015);
            h_next = h_curr * dec;

            let sweep = params.impulses[0];
            let curr_angle = sweep.y;
            let swept_delta = sweep.z;
            let outer_r = max(sweep.w, 1.0);
            let bass_pulse = params.impulses[1].w;

            let rel = cell_p - center;
            let r = length(rel);
            var theta = atan2(rel.y, rel.x);
            if (theta < 0.0) {
                theta += TAU;
            }
            var behind = theta - curr_angle;
            if (behind < 0.0) {
                behind += TAU;
            }
            if (behind >= TAU) {
                behind -= TAU;
            }

            if (overall >= 0.015) {
                let sweep_arc = max(swept_delta * 1.35, 0.065);
                if (swept_delta > 0.0 && behind <= sweep_arc && r <= outer_r && r >= 1.0) {
                    let r_norm = clamp(r / outer_r, 0.0, 1.0);
                    let b_idx = u32(clamp(floor(r_norm * 63.0), 0.0, 63.0));
                    let band_val = sample_band(b_idx);
                    let arm = 0.34 * clamp(overall * 2.4, 0.0, 1.0) * select(0.0, 1.0, behind <= 0.07);
                    h_next = max(h_next, arm);
                    if (band_val > 0.16 + 0.08 * r_norm) {
                        let echo = select(0.95, 1.45, band_val > 0.52);
                        h_next = max(h_next, echo);
                    }
                }
                if (bass_pulse > 0.25) {
                    let ring_r = 2.0 + bass_pulse * 3.2;
                    if (abs(r - ring_r) < 0.85) {
                        h_next = max(h_next, bass_pulse * 0.85);
                    }
                }
            }
        }
        case 3u: {
            // Mechanical Gravity: 64-band log-spaced spectrum columns with falling peak caps
            let u = (cell_p.x - (center.x - vis_w * 0.48)) / max(vis_w * 0.96, 1.0);
            let v = (cell_p.y - (center.y - vis_h * 0.45)) / max(vis_h * 0.90, 1.0);
            if (u >= 0.0 && u <= 1.0 && v >= 0.0 && v <= 1.0) {
                let col_idx = u32(clamp(floor(u * 64.0), 0.0, 63.0));
                let bar_h = sample_band(col_idx);
                let peak_h = sample_peak(col_idx);
                let row_norm = 1.0 / max(vis_h * 0.90, 1.0);
                if (bar_h > 0.02 && v <= bar_h) {
                    h_next = 0.85;
                } else if (peak_h > 0.05 && abs(v - peak_h) <= 0.62 * row_norm && peak_h > bar_h + 0.85 * row_norm) {
                    h_next = 0.85;
                }
            }
        }
        case 4u: {
            // Kinetic Rain & Splash Curtains
            h_next = h_curr * select(0.65, 0.72, overall >= 0.015);
            for (var k = 0u; k < 24u; k = k + 1u) {
                let drop = params.agents[k];
                if (drop.w > 0.5) {
                    if (abs(cell_p.x - drop.x) < 0.62 && cell_p.y >= drop.y && cell_p.y <= drop.y + drop.z) {
                        h_next = max(h_next, 0.92);
                    }
                }
            }
            for (var s = 0u; s < 4u; s = s + 1u) {
                let imp = params.impulses[s];
                if (imp.w > 0.10 && cell_p.y >= imp.y - 0.5) {
                    let dist = length(cell_p - imp.xy);
                    if (abs(dist - imp.z) < 0.82) {
                        h_next = max(h_next, imp.w);
                    }
                }
            }
            let bass_swell = params.bands[0].x;
            let y_floor = params.bands[0].y;
            if (overall >= 0.015 && bass_swell > 0.15) {
                let wave_env = 0.5 + 0.5 * sin(cell_p.x * 0.45 + params.sim_info.y * 5.0);
                let floor_h = y_floor + bass_swell * 1.8 * wave_env;
                if (cell_p.y >= y_floor - 0.5 && cell_p.y <= floor_h) {
                    h_next = max(h_next, 0.65);
                }
            }
        }
        default: {
            // Mode 0: Audio Ripples (2D isotropic wave equation + zero-mean Ricker impulses)
            let xm = max(x, 1u) - 1u;
            let xp = min(x + 1u, GRID_W - 1u);
            let ym = max(y, 1u) - 1u;
            let yp = min(y + 1u, GRID_H - 1u);
            let h_prev = c.y;

            let n = wave_in[ym * GRID_W + x].x;
            let s = wave_in[yp * GRID_W + x].x;
            let w = wave_in[y * GRID_W + xm].x;
            let e = wave_in[y * GRID_W + xp].x;
            let nw = wave_in[ym * GRID_W + xm].x;
            let ne = wave_in[ym * GRID_W + xp].x;
            let sw = wave_in[yp * GRID_W + xm].x;
            let se = wave_in[yp * GRID_W + xp].x;

            let lap = (4.0 * (n + s + e + w) + (ne + nw + se + sw) - 20.0 * h_curr) / 6.0;
            let edge_x = min(f32(x), f32(GRID_W - 1u - x));
            let edge_y = min(f32(y), f32(GRID_H - 1u - y));
            let sponge = smoothstep_r(6.0, 0.0, min(edge_x, edge_y));
            let damp = mix(0.022, 0.24, sponge);

            h_next = h_curr + (h_curr - h_prev) * (1.0 - damp) + WAVE_C2 * lap - RESTORE_K * h_curr;

            for (var k = 0u; k < 4u; k = k + 1u) {
                let imp = params.impulses[k];
                if (imp.w > 0.0) {
                    let d = cell_p - imp.xy;
                    let r = max(imp.z * 0.85, 0.65);
                    let u = dot(d, d) / (r * r);
                    if (u < 12.0) {
                        let ricker = (1.0 - 0.5 * u) * exp(-0.5 * u);
                        h_next += imp.w * ricker * 0.65;
                    }
                }
            }
        }
    }

    h_next = clamp(h_next, -2.8, 2.8);
    let next_env = max(c.z * 0.94, abs(h_next));
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
    let dwell_t = select(0.085, 0.045, params.mode_info.x > 0.5);

    if (dwell <= 0.0) {
        if (target_side < 0.5 && h > 0.14 + jitter) {
            target_side = 1.0;
            dwell = dwell_t;
        } else if (target_side >= 0.5 && h < 0.06 + jitter) {
            target_side = 0.0;
            dwell = dwell_t;
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
