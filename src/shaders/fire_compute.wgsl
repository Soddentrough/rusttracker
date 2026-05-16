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
            let raw_ch = params.display_order[i / 4u][i % 4u];
            if raw_ch == lfe_idx { continue; }
            let center_x = (f32(spatial_idx) + 0.5) * channel_width;
            let dist = f32(x) - center_x;
            let sigma = channel_width * sigma_scale;
            let influence = exp(-(dist * dist) / (2.0 * sigma * sigma));
            
            let vec_idx = i / 4u;
            let comp_idx = i % 4u;
            var ch_val = params.channels[vec_idx].x;
            if comp_idx == 1u { ch_val = params.channels[vec_idx].y; }
            else if comp_idx == 2u { ch_val = params.channels[vec_idx].z; }
            else if comp_idx == 3u { ch_val = params.channels[vec_idx].w; }
            
            activity += pow(ch_val, 1.5) * influence;
            spatial_idx += 1u;
        }
        if lfe_idx < n_ch {
            let lfe_dist = f32(x) - 512.0;
            let lfe_influence = exp(-(lfe_dist * lfe_dist) / (2.0 * 90.0 * 90.0));
            var lfe_disp_idx = 999u;
            for (var i = 0u; i < n_ch; i = i + 1u) {
                if params.display_order[i / 4u][i % 4u] == lfe_idx {
                    lfe_disp_idx = i;
                    break;
                }
            }
            if lfe_disp_idx < n_ch {
                let vec_idx = lfe_disp_idx / 4u;
                let comp_idx = lfe_disp_idx % 4u;
                var lfe_val = params.channels[vec_idx].x;
                if comp_idx == 1u { lfe_val = params.channels[vec_idx].y; }
                else if comp_idx == 2u { lfe_val = params.channels[vec_idx].z; }
                else if comp_idx == 3u { lfe_val = params.channels[vec_idx].w; }
                
                activity += lfe_val * lfe_influence * 0.6;
            }
        }

        // Use exponential soft-clipping for AGC (Automatic Gain Control)
        // This ensures quiet tracks still generate nice flames, while brickwalled tracks smoothly approach 1.0 without hard clipping.
        let coal_target = 1.0 - exp(-(params.bass * 0.5 + activity * 3.0));
        let current = coal_bed[x];
        if (coal_target > current) {
            coal_bed[x] = current + (coal_target - current) * 0.18;
        } else {
            coal_bed[x] = current + (coal_target - current) * 0.008;
        }
        
        // Use lower frequency noise to create wide, organic hotspots rather than single-pixel static
        let jitter = (perlin_noise(vec2<f32>(f32(x) * 0.05, params.time * 10.0)) + 1.0) * 0.5;
        output_grid[idx] = min(coal_bed[x] * (0.7 + 0.3 * jitter), 1.0);
        return;
    }

    // === Hearth rows: inject coal heat ===
    if (y >= bottom - 2u) {
        let coal_heat = min(coal_bed[x], 1.0);
        let jitter = (perlin_noise(vec2<f32>(f32(x) * 0.05, params.time * 10.0)) + 1.0) * 0.5;
        
        if (y == bottom - 1u) {
            output_grid[idx] = coal_heat * (0.85 + 0.15 * jitter);
        } else {
            output_grid[idx] = coal_heat * (0.7 + 0.15 * jitter);
        }
        return;
    }

    // === Normal propagation: Cellular Automaton Simulation ===
    
    // Base turbulence coordinates
    let p1 = vec2<f32>(f32(x), f32(y)) * 0.02;
    
    // Spread heat randomly but coherently to form organic flame tongues
    let spread_noise = perlin_noise(p1 * 1.5 + vec2<f32>(params.time * 4.0, -params.time * 10.0));
    let spread = i32(spread_noise * (1.5 + params.highs * 1.5));
    
    // Global wind swaying the fire
    let wind = i32(sin(params.time * 1.5 + f32(x) * 0.003) * params.highs * 2.0);
    
    // Classic DOS fire averaging: blend adjacent pixels to allow heat to diffuse into organic shapes
    let src_x = clamp(i32(x) + spread + wind, 0i, i32(w) - 1);
    let xl = max(src_x - 1, 0i);
    let xr = min(src_x + 1, i32(w) - 1);
    
    let h1 = input_grid[(y + 1u) * w + u32(xl)];
    let h2 = input_grid[(y + 1u) * w + u32(src_x)];
    let h3 = input_grid[(y + 1u) * w + u32(xr)];
    
    // Weighted horizontal blur favors the center pixel
    let heat = (h1 + h2 * 2.0 + h3) / 4.0;

    // Cooling varies spatially to create uneven tips and licks
    // Stretch the noise vertically and move it upwards to simulate rising flame tongues
    let p_cool = vec2<f32>(f32(x) * 0.015, f32(y) * 0.03);
    let cool_noise = perlin_noise(p_cool + vec2<f32>(0.0, params.time * 6.0));
    
    // Higher variance in cooling forces the flame to tear into distinct licks
    let base_cooling = 0.0015 + (cool_noise + 1.0) * 0.0015;
    
    // Occasional sparks (holes in the flame) for realism, clumped by noise
    let spark = select(0.0, 0.06, cool_noise > 0.8 && y > 100u);

    // Normalize the cooling factor so heavily compressed tracks don't turn off cooling entirely
    let dynamic_cooling = mix(0.6, 1.0, params.cooling_factor);

    output_grid[idx] = max(heat - base_cooling * dynamic_cooling - spark, 0.0);
}
