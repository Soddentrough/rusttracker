@group(0) @binding(0) var<storage, read_write> gpu_spectrum: array<vec2<f32>>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let point_idx = global_id.x; // 0 to 511
    let line_idx = global_id.y;  // 0 to 31
    
    if point_idx >= 512u || line_idx >= 32u {
        return;
    }
    
    let num_lines = 32u;
    let num_points = 512u;
    
    let bins_per_line = 1024u / num_lines; // 32
    let start_bin = line_idx * bins_per_line;
    let end_bin = start_bin + bins_per_line;
    
    let x_norm = f32(point_idx) / f32(num_points - 1u);
    var val = 0.0;
    
    let phase_mult = x_norm * 1.167098;
    
    for (var b = start_bin; b < end_bin; b = b + 1u) {
        let c = gpu_spectrum[b];
        let mag = length(c);
        let t = f32(b) * 0.0009765625;
        let freq = 20.0 * exp2(t * 10.106575);
        let phase = freq * phase_mult;
        
        // Compensate for natural pink noise (1/f) dropoff in music
        // by applying a +3dB/octave tilt (sqrt of frequency ratio)
        let eq_boost = sqrt(freq / 20.0);
        
        // Use magnitude to lock the phase, preventing horizontal scrolling
        val += (mag * eq_boost) * cos(phase);
    }
    
    // Normalize and scale correctly to keep amplitude around 1.0
    // gpu_fft outputs are scaled by sqrt(N). We need to divide by sqrt(N) = 90.5
    // and apply an aesthetic multiplier.
    val = (val / 90.5) * 1.5;
    
    // Store in unused channels 16..31 of gpu_spectrum
    // Each channel holds 1024 vec2s (2048 floats).
    // We need 32 lines * 512 floats = 16384 floats.
    // 16384 floats / 2 = 8192 vec2s.
    // 8192 vec2s / 1024 = 8 channels needed.
    // We use channels 16 to 23.
    let base_offset = 16u * 1024u; 
    let flat_idx = line_idx * 512u + point_idx;
    // Each vec2 holds 1 point (we only use .x to keep it simple, or pack 2 points per vec2)
    // Let's pack 2 points per vec2!
    // flat_idx / 2 is the vec2 index.
    // We can't safely pack 2 points concurrently without atomic writes, so let's just use 1 point per vec2 for simplicity.
    // We have plenty of space (channels 16-31 = 16 * 1024 vec2s = 16384 vec2s).
    // Exactly enough for 1 vec2 per point!
    
    gpu_spectrum[base_offset + flat_idx] = vec2<f32>(val, 0.0);
}
