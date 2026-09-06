struct FireParams {
    bass: f32,
    mids: f32,
    highs: f32,
    time: f32,
    cooling_factor: f32,
    turb_spread_f: f32, // fire_intensity: 1.0 when playing, decays to 0.0 when stopped/paused
    width: u32,
    height: u32,
    num_channels: u32,
    lfe_idx: u32,
    fft_channels: u32,
    dt: f32,
    display_order: array<vec4<u32>, 4>,
    channels: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<storage, read> input_grid: array<f32>;
@group(0) @binding(1) var<storage, read_write> output_grid: array<f32>;
@group(0) @binding(2) var<storage, read_write> coal_bed: array<f32>;
@group(0) @binding(3) var<uniform> params: FireParams;

fn pcg_hash(input: u32) -> u32 {
    var state = input * 747796405u + 2891336453u;
    var word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn read_channel(idx: u32) -> f32 {
    let vec_idx = idx / 4u;
    let comp_idx = idx % 4u;
    if (vec_idx >= 8u) { return 0.0; }
    let v = params.channels[vec_idx];
    if (comp_idx == 0u) { return v.x; }
    if (comp_idx == 1u) { return v.y; }
    if (comp_idx == 2u) { return v.z; }
    return v.w;
}

fn get_channel_energy(norm_x: f32) -> f32 {
    let n_ch = params.num_channels;
    if (n_ch <= 1u) {
        return read_channel(0u);
    }
    let fx = norm_x * f32(n_ch) - 0.5;
    let i0 = clamp(i32(floor(fx)), 0, i32(n_ch) - 1);
    let i1 = clamp(i0 + 1, 0, i32(n_ch) - 1);
    let frac = fract(fx);
    let v0 = read_channel(u32(i0));
    let v1 = read_channel(u32(i1));
    return mix(v0, v1, smoothstep(0.0, 1.0, frac));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let y = id.y;
    let W = params.width;
    let H = params.height;
    if (x >= W || y >= H) { return; }

    let idx = y * W + x;

    // Simulation grid dimensions (authentic DOOM resolution: 320 x 180)
    let SIM_W = 320u;
    let SIM_H = 180u;
    let cx = (x * SIM_W) / W;
    let cy = (y * SIM_H) / H;

    // Frame-rate scaling (calibrated for 60fps)
    let dt_scale = clamp(params.dt * 60.0, 0.5, 2.0);
    let intensity = clamp(params.turb_spread_f, 0.0, 1.0);

    // If completely stopped and cold, decay to black and return
    if (intensity < 0.001) {
        output_grid[idx] = max(input_grid[idx] - 0.05 * dt_scale, 0.0);
        if (y == H - 1u) { coal_bed[x] = 0.0; }
        return;
    }

    let norm_x = (f32(cx) + 0.5) / f32(SIM_W);
    let ch_energy = get_channel_energy(norm_x);

    // === Bottom rows: Combustion source bed across the full width ===
    if (cy >= SIM_H - 2u) {
        // High-frequency boiling combustion noise per column
        let seed_base = cx + u32(params.time * 80.0) * 8191u;
        let crackle_h = pcg_hash(seed_base);
        let crackle = (f32(crackle_h % 1000u) / 1000.0 - 0.5) * 0.12;

        // Roaring baseline bed across the entire width:
        // Baseline combustion is ALWAYS incandescent white-hot (1.0 / palette 36)
        // during active playback across the full screen width.
        let white_bed = clamp(1.0 + crackle, 0.90, 1.0) * intensity;

        if (cy == SIM_H - 1u) {
            output_grid[idx] = white_bed;
            if (y == H - 1u) { coal_bed[x] = white_bed; }
        } else {
            let hearth = clamp(0.95 * intensity + crackle * 0.8, 0.80 * intensity, 1.0);
            output_grid[idx] = hearth;
        }
        return;
    }

    // === Cellular Automaton Flame Propagation (Authentic DOOM Fire Algorithm) ===
    let seed = cx + cy * SIM_W + u32(params.time * 60.0) * 1973u;
    let r_hash = pcg_hash(seed);
    let rand_idx = r_hash & 3u; // 0, 1, 2, 3

    // Wind / flutter swaying flames horizontally, modulated by treble/highs
    let wind = i32(sin(params.time * 1.5) * (0.8 + params.highs * 1.5));
    let offset = (i32(rand_idx) - 1) + wind;
    let src_cx = clamp(i32(cx) + offset, 0, i32(SIM_W) - 1);
    let src_cy = cy + 1u; // row directly below

    // Read source cell from input_grid (sampled at cell center)
    let src_sample_x = (u32(src_cx) * W) / SIM_W + (W / SIM_W / 2u);
    let src_sample_y = (src_cy * H) / SIM_H + (H / SIM_H / 2u);
    let src_val = input_grid[src_sample_y * W + src_sample_x];

    // Read previous frame value at (cx, cy) for persistence
    let prev_sample_x = (cx * W) / SIM_W + (W / SIM_W / 2u);
    let prev_sample_y = (cy * H) / SIM_H + (H / SIM_H / 2u);
    let prev_val = input_grid[prev_sample_y * W + prev_sample_x];

    // Arrival probability (scatter hit chance)
    let hit_rand = (r_hash >> 2u) & 255u;
    let is_hit = hit_rand < 215u;

    // Gentle altitude factor (only active above 50% screen height to taper smoke)
    let let_alt = max(0.0, f32(SIM_H - cy) / f32(SIM_H) - 0.5) * 0.20;

    // Dynamic cooling:
    // Low cooling near the base and active channels so flame tongues leap high (80-85% screen),
    // higher cooling in quiet zones to create deep valleys and dancing spires.
    let cool_rate = (0.32 - params.bass * 0.14 - ch_energy * 0.18 + let_alt) * dt_scale;
    let cool_threshold = u32(clamp(cool_rate * 255.0, 15.0, 245.0));
    let cool_rand = (r_hash >> 10u) & 255u;

    // Palette unit in normalized float: 1.0 / 36.0 ≈ 0.027778
    let pal_unit = 1.0 / 36.0;

    var new_heat = 0.0;
    if (is_hit && src_val > 0.001) {
        let decay = select(0.0, pal_unit, cool_rand < cool_threshold);
        new_heat = max(src_val - decay, 0.0);
    } else {
        new_heat = max(prev_val - pal_unit * 0.4 * dt_scale, 0.0);
    }

    // Occasional spark voids / turbulent tears in hot zones
    let spark_rand = (r_hash >> 18u) & 255u;
    if (spark_rand == 0u && new_heat > 0.45) {
        new_heat = max(new_heat - pal_unit * 2.0, 0.0);
    }

    output_grid[idx] = new_heat;
}
