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
        // Sharp fuel sources for distinct columns
        var sigma_scale = 0.08;
        if n_spatial_ch <= 2u { sigma_scale = 0.2; }
        
        var spatial_idx = 0u;
        for (var i = 0u; i < n_ch; i = i + 1u) {
            if i == lfe_idx { continue; }
            let center_x = (f32(spatial_idx) + 0.5) * channel_width;
            let dist = f32(x) - center_x;
            let sigma = channel_width * sigma_scale;
            let influence = exp(-(dist * dist) / (2.0 * sigma * sigma));
            
            var fft_ch = params.display_order[i / 4u][i % 4u];
            if (params.fft_channels < params.num_channels) {
                fft_ch = fft_ch % max(params.fft_channels, 1u);
            }
            
            var ch_bass = 0.0;
            let offset = fft_ch * 1024u;
            for (var b = 10u; b < 350u; b = b + 10u) {
                let c = multi_spectrum[offset + b];
                ch_bass += clamp(length(c) * 100.0, 0.0, 100.0);
            }
            ch_bass = (ch_bass / 34.0) / 100.0;
            
            activity += pow(ch_bass, 1.5) * influence * 2.5;
            spatial_idx += 1u;
        }
        if lfe_idx < n_ch {
            var lfe_bass = 0.0;
            let offset = lfe_idx * 1024u;
            for (var b = 0u; b < 200u; b = b + 5u) {
                let c = multi_spectrum[offset + b];
                lfe_bass += clamp(length(c) * 100.0, 0.0, 100.0);
            }
            lfe_bass = (lfe_bass / 40.0) / 100.0;
            
            // Apply LFE fuel globally across the entire bottom bed, but scaled down so it doesn't max out the whole screen and wash out the spatial channels
            activity += lfe_bass * 0.4;
        }

        // Reduce global params.bass so spatial channels control local flame height
        let coal_target = min(params.bass * 0.05 + activity * 1.2, 1.0);
        let current = coal_bed[x];
        if (coal_target > current) {
            coal_bed[x] = current + (coal_target - current) * 0.18;
        } else {
            coal_bed[x] = current + (coal_target - current) * 0.008;
        }
        
        // The high frequencies drive the random jitter at the solid fuel layer!
        let hf_bin = 400u + (x % 300u);
        var hf_noise = 0.0;
        for (var i = 0u; i < n_ch; i = i + 1u) {
            hf_noise += length(multi_spectrum[i * 1024u + hf_bin]);
        }
        hf_noise = clamp(hf_noise * 60.0, 0.0, 1.0);
        
        output_grid[idx] = min(coal_bed[x] * (0.6 + 0.4 * hf_noise), 1.0);
        return;
    }

    // === Hearth rows: Heat rising from the coals ===
    if (y >= bottom - 2u) {
        let coal_heat = min(coal_bed[x], 1.0);
        
        // High frequencies directly tear the flames at the source
        let hf_bin = 500u + (x % 400u);
        var hf_noise = 0.0;
        for (var i = 0u; i < params.num_channels; i = i + 1u) {
            hf_noise += length(multi_spectrum[i * 1024u + hf_bin]);
        }
        hf_noise = clamp(hf_noise * 80.0, 0.0, 1.0);
        var out_val = coal_heat * (0.5 + 0.5 * hf_noise);
        
        // Embers are directly spawned by extreme high-frequency transients (snares, hi-hats)
        let spark_bin = 800u + (x % 200u);
        var spark_noise = 0.0;
        for (var i = 0u; i < params.num_channels; i = i + 1u) {
            spark_noise += length(multi_spectrum[i * 1024u + spark_bin]);
        }
        
        if (coal_heat > 0.4 && spark_noise * 150.0 > 1.0) {
            out_val = 6.0; // Crisp ember packet
        }
        
        output_grid[idx] = out_val;
        return;
    }
    // === PHYSICAL FLUID DYNAMICS SIMULATION ===
    let p1 = vec2<f32>(f32(x), f32(y));
    let current_val = input_grid[idx];
    
    // Positive values = Heat (Fire/Embers). Negative values = Smoke density.
    let heat = max(current_val, 0.0);
    let smoke = max(-current_val, 0.0);
    
    // 1. Thermal Buoyancy: Hot air and warm smoke rise.
    var buoyancy = 0.5; // Stronger base updraft
    buoyancy += heat * 6.5; // Roaring vertical speed
    buoyancy += smoke * 1.5;
    let buoyancy_vel = vec2<f32>(0.0, -1.0) * buoyancy;
    
    // 2. Vorticity Confinement & Multi-Octave Turbulence
    let g_left = sample_grid(p1.x - 1.0, p1.y);
    let g_right = sample_grid(p1.x + 1.0, p1.y);
    let g_up = sample_grid(p1.x, p1.y - 1.0);
    let g_down = sample_grid(p1.x, p1.y + 1.0);
    
    let grad_mag = clamp(length(vec2<f32>(g_right - g_left, g_down - g_up)), 0.0, 1.0);
    
    // Fractal curl noise creates sharp, chaotic tearing just like real fire plasma
    let curl1 = curl_noise(p1 * 0.015 - vec2<f32>(0.0, params.time * 1.5));
    let curl2 = curl_noise(p1 * 0.04 - vec2<f32>(0.0, params.time * 3.0));
    let raw_curl = curl1 + curl2 * 0.5;
    
    // Pure physics: turbulence driven by fluid gradient, NO global audio reactivity!
    let turbulence = raw_curl * (0.5 + grad_mag * 6.0);
    
    // 3. Advection
    // Keep horizontal turbulence constrained so flames remain distinctly columnated,
    // and reduce vertical turbulence so it doesn't fight the upward buoyancy!
    let controlled_turb = turbulence * vec2<f32>(0.6, 0.2);
    let vel = clamp(buoyancy_vel + controlled_turb, vec2<f32>(-3.0, -10.0), vec2<f32>(3.0, 2.0));
    let dt = 1.0;
    let src_p = p1 - vel * dt;
    
    var new_val = sample_grid(src_p.x, src_p.y);
    
    if (new_val > 0.0) {
        // Fire cooling
        let cool_noise = perlin_noise(p1 * 0.05 - vec2<f32>(0.0, params.time * 3.0));
        let base_cooling = 0.008 + (cool_noise + 1.0) * 0.006;
        
        var cooling_rate = base_cooling * params.cooling_factor;
        // Embers (val > 1.0) cool logarithmically so they survive the journey upwards
        if (new_val > 1.0) { cooling_rate = new_val * 0.02; } 
        
        new_val -= cooling_rate;
        
        // COMBUSTION: When fire runs out of heat (crosses 0), the burnt fuel chemically turns into smoke!
        if (new_val <= 0.0) {
            // Multiply the remainder to represent expansive smoke generation
            new_val = new_val * 150.0; 
            new_val = clamp(new_val, -3.0, 0.0);
        }
    } else if (new_val < 0.0) {
        // Smoke dissipating slowly as it mixes with cold air
        new_val += 0.001; 
        if (new_val > 0.0) { new_val = 0.0; }
    }
    
    output_grid[idx] = new_val;
}
