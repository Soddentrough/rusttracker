// INCLUDE: common

@group(0) @binding(0) var<uniform> uniforms: AudioUniforms;
@group(0) @binding(1) var heatmap_tex: texture_storage_2d<r32float, write>;

@compute @workgroup_size(256, 1, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    if (x >= 256u) { return; }

    // Read the spectrum data from the uniform buffer.
    // In engine.rs, we use 1024 bins, but we only have 256 chunks.
    // Each chunk handles 4 bins.
    let vec_idx = x;
    let spec_vec = uniforms.spectrum[vec_idx];

    // Find the max value within these 4 bins
    var max_val = spec_vec.x;
    max_val = max(max_val, spec_vec.y);
    max_val = max(max_val, spec_vec.z);
    max_val = max(max_val, spec_vec.w);

    let y = i32(uniforms.heatmap_row);
    let steps = i32(uniforms.steps_to_fill);
    
    // Write the spectrum data to the current row, any skipped intermediate rows,
    // and the next row (future row) to prevent stale history sampling.
    // Limit steps to 16 to avoid GPU timeout in extreme lag cases.
    let fill_steps = clamp(steps, 0, 16);
    for (var i = -fill_steps; i <= 1; i++) {
        let target_y = u32((y + i + 1024) % 1024);
        textureStore(heatmap_tex, vec2<i32>(i32(x), i32(target_y)), vec4<f32>(max_val, 0.0, 0.0, 0.0));
    }
}
