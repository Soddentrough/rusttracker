fn main() {
    let source = std::fs::read_to_string("src/shaders/vis_starfield.wgsl").unwrap();
    // we need to include common
    let common = "struct VertexOutput { @builtin(position) clip_position: vec4<f32>, @location(0) uv: vec2<f32>, }; @vertex fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput { var out: VertexOutput; return out; } struct AudioUniforms { spectrum: array<vec4<f32>, 256>, fire_heat: array<vec4<f32>, 256>, channels: array<vec4<f32>, 8>, channel_peaks: array<vec4<f32>, 8>, spatial_channels: array<vec4<f32>, 4>, display_order: array<vec4<u32>, 4>, num_channels: u32, mode: u32, time: f32, duration: f32, smooth_time: f32, heatmap_row: u32, fft_channels: u32, num_spatial_channels: u32, ui_meters_rect: vec4<f32>, ui_heatmap_rect: vec4<f32>, ui_fire_rect: vec4<f32>, waveform_resolution: u32, waveform_history_size: u32, _pad0: u32, _pad1: u32, };";
    let full_source = source.replace("// INCLUDE: common", common);
    let mut frontend = naga::front::wgsl::Frontend::new();
    match frontend.parse(&full_source) {
        Ok(_) => println!("WGSL Parsed Successfully"),
        Err(e) => {
            e.emit_to_stderr_with_path(&full_source, "vis_starfield.wgsl");
            std::process::exit(1);
        }
    }
}
