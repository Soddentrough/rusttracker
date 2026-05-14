struct FFTParams {
    num_channels: u32,
    sample_rate: f32,
    min_freq: f32,
    max_freq: f32,
    num_samples: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read> raw_audio: array<f32>; // Dynamic size based on channels * max_samples
@group(0) @binding(1) var<storage, read_write> spectrum: array<vec2<f32>>; // [32 * 1024]
@group(0) @binding(2) var<uniform> params: FFTParams;

const NUM_BINS: u32 = 1024u;
const PI: f32 = 3.14159265359;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let bin_idx = id.x;
    let ch_idx = id.y;
    
    if (bin_idx >= NUM_BINS || ch_idx >= params.num_channels) {
        return;
    }
    
    // Log-spaced frequency target for this bin
    let t = f32(bin_idx) / f32(NUM_BINS);
    let freq = params.min_freq * pow(params.max_freq / params.min_freq, t);
    
    var re: f32 = 0.0;
    var im: f32 = 0.0;
    
    let k = 2.0 * PI * freq / params.sample_rate;
    
    // Direct log-spaced DFT
    let num_samples = params.num_samples;
    for (var n = 0u; n < num_samples; n = n + 1u) {
        let sample = raw_audio[ch_idx * num_samples + n];
        
        // Apply Hann window
        let window = 0.5 * (1.0 - cos(2.0 * PI * f32(n) / f32(num_samples - 1u)));
        let w_sample = sample * window;
        
        let phase = k * f32(n);
        re += w_sample * cos(phase);
        im -= w_sample * sin(phase);
    }
    
    let norm_re = re / sqrt(f32(num_samples));
    let norm_im = im / sqrt(f32(num_samples));
    
    // Write out normalized complex values
    spectrum[ch_idx * NUM_BINS + bin_idx] = vec2<f32>(norm_re, norm_im);
}
