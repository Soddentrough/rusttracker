struct AudioUniforms {
    spectrum: array<vec4<f32>, 256>,
    fire_heat: array<vec4<f32>, 256>,
    channels: array<vec4<f32>, 8>,
    channel_peaks: array<vec4<f32>, 8>,
    num_channels: u32,
    mode: u32,
    time: f32,
    duration: f32,
    smooth_time: f32,
    heatmap_row: u32,
    _pad2: u32,
    _pad3: u32,
    ui_meters_rect: vec4<f32>,
    ui_heatmap_rect: vec4<f32>,
    ui_fire_rect: vec4<f32>,
};

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

    // Write to the current row
    let y = uniforms.heatmap_row;
    textureStore(heatmap_tex, vec2<i32>(i32(x), i32(y)), vec4<f32>(max_val, 0.0, 0.0, 0.0));
}
