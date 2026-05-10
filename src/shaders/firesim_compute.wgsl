struct FireParams {
    bass: f32,
    mids: f32,
    highs: f32,
    time: f32,
    cooling_factor: f32,
    turb_spread_f: f32,
    width: u32,
    height: u32,
    num_channels: u32,
    lfe_idx: u32,
    fft_channels: u32,
    _pad1: u32,
    display_order: array<vec4<u32>, 2>,
    channels: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<storage, read> input_grid: array<f32>;
@group(0) @binding(1) var<storage, read_write> output_grid: array<f32>;
@group(0) @binding(2) var<storage, read_write> coal_bed: array<f32>;
@group(0) @binding(3) var<uniform> params: FireParams;
@group(0) @binding(4) var<storage, read> multi_spectrum: array<vec2<f32>>;

fn pcg_hash(input: u32) -> u32 {
    var state = input * 747796405u + 2891336453u;
    var word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)),
                      dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453) * 2.0 - 1.0;
}

fn perlin_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    
    let a = dot(hash22(i + vec2<f32>(0.0, 0.0)), f - vec2<f32>(0.0, 0.0));
    let b = dot(hash22(i + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(hash22(i + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(hash22(i + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));
    
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn curl_noise(p: vec2<f32>) -> vec2<f32> {
    let e = 0.05;
    let dx = (perlin_noise(p + vec2<f32>(e, 0.0)) - perlin_noise(p - vec2<f32>(e, 0.0))) / (2.0 * e);
    let dy = (perlin_noise(p + vec2<f32>(0.0, e)) - perlin_noise(p - vec2<f32>(0.0, e))) / (2.0 * e);
    return vec2<f32>(dy, -dx);
}

fn sample_grid(x: f32, y: f32) -> f32 {
    let cx = clamp(x, 0.0, f32(params.width) - 1.0);
    let cy = clamp(y, 0.0, f32(params.height) - 1.0);
    
    let x0 = i32(cx);
    let y0 = i32(cy);
    let x1 = min(x0 + 1, i32(params.width) - 1);
    let y1 = min(y0 + 1, i32(params.height) - 1);
    
    let fx = cx - f32(x0);
    let fy = cy - f32(y0);
    
    let v00 = input_grid[u32(y0) * params.width + u32(x0)];
    let v10 = input_grid[u32(y0) * params.width + u32(x1)];
    let v01 = input_grid[u32(y1) * params.width + u32(x0)];
    let v11 = input_grid[u32(y1) * params.width + u32(x1)];
    
    let top = mix(v00, v10, fx);
    let bot = mix(v01, v11, fx);
    return mix(top, bot, fy);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let y = id.y;
    if (x >= params.width || y >= params.height) { return; }

    let w = params.width;
    let bottom = params.height - 1u;
    let idx = y * w + x;

    // Seed RNG from position + time
    let seed = pcg_hash(x + y * 1337u + bitcast<u32>(params.time * 100.0));

    // === Bottom row: update coal bed with thermal inertia ===
    if (y == bottom) {
        var activity = 0.0;
        let n_ch = params.num_channels;
        let lfe_idx = params.lfe_idx;
        var n_spatial_ch = n_ch;
        if lfe_idx < n_ch {
            n_spatial_ch = n_ch - 1u;
        }
        let channel_width = 1024.0 / f32(max(n_spatial_ch, 1u));
        var sigma_scale = 0.18;
        if n_spatial_ch <= 2u {
            sigma_scale = 0.4;
        }
        
        var spatial_idx = 0u;
        for (var i = 0u; i < n_ch; i = i + 1u) {
            if i == lfe_idx { continue; }
            let center_x = (f32(spatial_idx) + 0.5) * channel_width;
            let dist = f32(x) - center_x;
            let sigma = channel_width * sigma_scale;
            let influence = exp(-(dist * dist) / (2.0 * sigma * sigma));
            
            var fft_ch = i;
            var raw_ch = params.display_order[i / 4u][i % 4u];
            var vu_scale = 1.0;
            if (params.fft_channels < params.num_channels) {
                fft_ch = raw_ch % max(params.fft_channels, 1u);
                
                let vec_idx = i / 4u;
                let elem_idx = i % 4u;
                var val = 0.0;
                if (elem_idx == 0u) { val = params.channels[vec_idx].x; }
                else if (elem_idx == 1u) { val = params.channels[vec_idx].y; }
                else if (elem_idx == 2u) { val = params.channels[vec_idx].z; }
                else { val = params.channels[vec_idx].w; }
                
                vu_scale = max(val * 1.5, 0.05);
            }
            
            // Sample low frequencies (bins 10 to 350) for this specific channel (approx 20Hz - 200Hz)
            var ch_bass = 0.0;
            let offset = fft_ch * 1024u;
            for (var b = 10u; b < 350u; b = b + 10u) {
                let c = multi_spectrum[offset + b];
                ch_bass += clamp(length(c) * 100.0, 0.0, 100.0);
            }
            ch_bass = (ch_bass / 34.0) / 100.0;
            
            activity += pow(ch_bass, 1.5) * influence * 2.5 * vu_scale;
            spatial_idx += 1u;
        }
        if lfe_idx < n_ch {
            let lfe_dist = f32(x) - 512.0;
            let lfe_influence = exp(-(lfe_dist * lfe_dist) / (2.0 * 90.0 * 90.0));
            
            var lfe_bass = 0.0;
            let offset = lfe_idx * 1024u;
            for (var b = 0u; b < 200u; b = b + 5u) {
                let c = multi_spectrum[offset + b];
                lfe_bass += clamp(length(c) * 100.0, 0.0, 100.0);
            }
            lfe_bass = (lfe_bass / 40.0) / 100.0;
            
            activity += lfe_bass * lfe_influence * 1.5;
        }

        let coal_target = min(params.bass * 0.25 + activity * 2.5, 1.0);
        let current = coal_bed[x];
        if (coal_target > current) {
            coal_bed[x] = current + (coal_target - current) * 0.18;
        } else {
            coal_bed[x] = current + (coal_target - current) * 0.008;
        }
        
        let jitter = (perlin_noise(vec2<f32>(f32(x) * 0.5, params.time * 10.0)) + 1.0) * 0.5;
        output_grid[idx] = min(coal_bed[x] * (0.7 + 0.3 * jitter), 1.0);
        return;
    }

    // === Hearth rows: inject coal heat ===
    if (y >= bottom - 2u) {
        let coal_heat = min(coal_bed[x], 1.0);
        let jitter = (perlin_noise(vec2<f32>(f32(x) * 0.5, params.time * 10.0)) + 1.0) * 0.5;
        
        if (y == bottom - 1u) {
            output_grid[idx] = coal_heat * (0.85 + 0.15 * jitter);
        } else {
            output_grid[idx] = coal_heat * (0.7 + 0.15 * jitter);
        }
        return;
    }

    // === ADVECTION FLUID SIMULATION ===
    let p1 = vec2<f32>(f32(x), f32(y));
    let uv = p1 / vec2<f32>(1024.0, 576.0);
    
    // Upward buoyancy based on heat
    let current_heat = input_grid[idx];
    let buoyancy_vel = vec2<f32>(0.0, -1.0) * (1.0 + current_heat * 2.0 + params.bass * 3.0);
    
    // Curl noise for swirling
    let noise_scale = 0.012;
    let scroll = vec2<f32>(0.0, params.time * 0.8);
    let curl = curl_noise((p1 * noise_scale) - scroll);
    
    let vel = buoyancy_vel + curl * (1.2 + params.highs * 2.5);
    
    // Reverse advection (where did this pixel come from?)
    let dt = 1.0;
    let src_p = p1 - vel * dt;
    
    let advected_heat = sample_grid(src_p.x, src_p.y);
    
    // Cooling
    let cool_noise = perlin_noise(p1 * 0.04 - vec2<f32>(0.0, params.time * 2.5));
    let base_cooling = 0.002 + (cool_noise + 1.0) * 0.0015;
    
    output_grid[idx] = max(advected_heat - base_cooling * params.cooling_factor, 0.0);
}
