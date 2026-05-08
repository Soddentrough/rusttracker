import os
import glob

replacement = """struct AudioUniforms {
    spectrum: array<vec4<f32>, 256>,
    fire_heat: array<vec4<f32>, 256>,
    channels: array<vec4<f32>, 8>,
    channel_peaks: array<vec4<f32>, 8>,
    spatial_channels: array<vec4<f32>, 4>,
    num_channels: u32,
    mode: u32,
    time: f32,
    duration: f32,
    smooth_time: f32,
    heatmap_row: u32,
    fft_channels: u32,
    num_spatial_channels: u32,
    ui_meters_rect: vec4<f32>,
    ui_heatmap_rect: vec4<f32>,
    ui_fire_rect: vec4<f32>,
};"""

target = """struct AudioUniforms {
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
};"""

target2 = target.replace("_pad2: u32,\n    _pad3: u32,", "fft_channels: u32,\n    _pad: u32,")

count = 0
for f in glob.glob("src/shaders/*.wgsl"):
    if "vis_spatial" in f:
        continue
    with open(f, 'r') as file:
        content = file.read()
        
    if target in content:
        content = content.replace(target, replacement)
        with open(f, 'w') as file:
            file.write(content)
        count += 1
    elif target2 in content:
        content = content.replace(target2, replacement)
        with open(f, 'w') as file:
            file.write(content)
        count += 1

print(f"Replaced in {count} files")
