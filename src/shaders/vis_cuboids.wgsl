// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;
@group(2) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
}

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec3<f32>,
    @location(1) @interpolate(linear) ndc: vec2<f32>,
    @location(2) amp: f32,
    @location(3) depth: f32,
}

@vertex
fn vs_main_3d(in: VertexInput, @builtin(instance_index) inst_idx: u32) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    // 1674 instances: 837 floor, 837 ceiling
    let is_top = inst_idx >= 837u;
    let box_idx = select(inst_idx, inst_idx - 837u, is_top);
    
    // Grid coordinate calculation (matches raymarching)
    let ix = i32(box_idx % 31u) - 15;
    let iz = i32(box_idx / 31u) - 20;
    
    // Spacing
    let x_spacing = 1.35;
    
    let dist_from_center = sqrt(f32(ix * ix + iz * iz));
    let bin = clamp(u32(dist_from_center * 10.0) + 4u, 0u, 255u);
    let steps_ago = u32(dist_from_center * 3.0);
    
    // Fetch audio amplitude from history texture
    let row = (audio.heatmap_row + 1024u - (steps_ago & 1023u)) & 1023u;
    let amp = textureLoad(history_tex, vec2<i32>(i32(bin), i32(row)), 0).x;
    
    let shift = clamp(amp / 100.0, 0.0, 1.0) * 1.5;
    
    // Position within grid
    let world_x = f32(ix) * x_spacing;
    let world_z = 5.2 - f32(iz) * x_spacing;
    
    let y_center = select(-1.4 + shift, 4.4 - shift, is_top);
    
    // Scale unit box to match spacing exactly (eliminating gaps)
    // UnitBox is [-0.5, 0.5]
    let scaled = in.position * vec3<f32>(x_spacing, 1.7, x_spacing);
    let world_pos = scaled + vec3<f32>(world_x, y_center, world_z);
    
    // Store local position for face edge detection
    out.local_pos = scaled;
    out.amp = amp;
    
    // Project position
    var clip_pos = camera.proj_matrix * camera.view_matrix * vec4<f32>(world_pos, 1.0);
    
    out.depth = clip_pos.w;
    
    // Project instance center to evaluate culling
    let inst_center = vec3<f32>(world_x, y_center, world_z);
    let center_clip = camera.proj_matrix * camera.view_matrix * vec4<f32>(inst_center, 1.0);
    
    var should_cull = center_clip.w < 1.0;
    if (!should_cull) {
        let center_ndc = center_clip.xy / center_clip.w;
        should_cull = abs(center_ndc.x) > 2.2 || abs(center_ndc.y) > 2.2;
    }
    
    var distorted_ndc = vec2<f32>(0.0);
    if (should_cull) {
        clip_pos = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    } else {
        // Apply barrel distortion in clip space (NDC)
        let ndc = clamp(clip_pos.xy / clip_pos.w, vec2<f32>(-2.0), vec2<f32>(2.0));
        let r2 = min(2.0, dot(ndc, ndc));
        distorted_ndc = ndc * (1.0 + r2 * 0.055);
        clip_pos = vec4<f32>(distorted_ndc * clip_pos.w, clip_pos.z, clip_pos.w);
    }
    
    out.clip_position = clip_pos;
    out.ndc = distorted_ndc;
    
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    // 1. Bezel boundary check
    if (abs(in.ndc.x) > 1.0 || abs(in.ndc.y) > 1.0) {
        discard;
    }
    
    let border_dist = min(1.0 - abs(in.ndc.x), 1.0 - abs(in.ndc.y));
    let bezel_mask = smoothstep(0.0, 0.03, border_dist);
    
    // 2. Wireframe / Edge Detection on the Box Face
    let box_half = vec3<f32>(0.675, 0.85, 0.675);
    let d = box_half - abs(in.local_pos);
    
    // Compute distance to nearest edge in pixels using screen-space derivatives
    let fw = fwidth(in.local_pos);
    var dist_pixels = vec3<f32>(1e6, 1e6, 1e6);
    if (fw.x > 1e-5) { dist_pixels.x = d.x / fw.x; }
    if (fw.y > 1e-5) { dist_pixels.y = d.y / fw.y; }
    if (fw.z > 1e-5) { dist_pixels.z = d.z / fw.z; }
    let pixel_dist = min(min(dist_pixels.x, dist_pixels.y), dist_pixels.z);
    
    // 3. Core and bloom/glow calculations (reacting to audio amplitude)
    let amp_factor = 1.0 + clamp(in.amp / 30.0, 0.0, 3.0);
    
    let line_width = 1.5; // 1.5 pixels wide core
    let core_glow = smoothstep_r(line_width + 1.0, line_width - 1.0, pixel_dist);
    
    // Exponential bloom/glow falloffs
    let bloom_glow = exp(-pixel_dist * 0.15) * amp_factor; // neon green glow
    let wide_bloom = exp(-pixel_dist * 0.05) * 0.4 * amp_factor; // wide faint bloom
    
    // 4. Color calculation: Emissive bright core -> neon green glow -> deep solid green base fill
    let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x + audio.spectrum[2].x, 0.0, 1.0);
    
    let core_col = vec3<f32>(0.8, 1.0, 0.9); // bright green-white core
    let neon_green = vec3<f32>(0.0, 1.0, 0.4); // vibrant neon green
    let deep_green = vec3<f32>(0.0, 0.08, 0.03); // deep solid green base fill
    
    let base_fill = deep_green * (0.6 + 0.8 * bass);
    
    var final_color = base_fill + neon_green * (bloom_glow * 1.5 + wide_bloom * 0.8) + core_col * (core_glow * 2.0);
    
    // 5. Distance fade out (depth fog)
    let depth = in.depth;
    // Fade out to black between depth 8.0 and 32.0
    let fade = clamp((32.0 - depth) / (32.0 - 8.0), 0.0, 1.0);
    let fade_smooth = smoothstep(0.0, 1.0, fade);
    final_color = final_color * fade_smooth;
    
    // 6. CRT Filter: Scanlines
    let scanline = 0.86 + 0.14 * cos(in.clip_position.y * 3.14159);
    final_color = final_color * scanline;
    
    // 7. CRT Filter: Flicker
    let flicker = 0.98 + 0.02 * sin(audio.time * 115.0);
    final_color = final_color * flicker;
    
    // 8. CRT Filter: Analog static noise
    let noise_val = hash21(in.clip_position.xy + fract(audio.smooth_time) * 149.0);
    let static_noise = noise_val * 0.022 * bezel_mask;
    final_color = final_color + vec3<f32>(static_noise);
    
    // Apply bezel mask
    final_color = final_color * bezel_mask;
    
    // 9. Fitted ACES Tonemap
    var final_col = aces_tonemap(final_color);
    final_col = max(final_col, vec3<f32>(0.0));
    
    return vec4<f32>(final_col, 1.0);
}
