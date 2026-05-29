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
    @location(1) ndc: vec2<f32>,
    @location(2) amp: f32,
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
    
    // Scale unit box to match raymarching size b = (0.22, 0.85, 0.22)
    // UnitBox is [-0.5, 0.5]
    let scaled = in.position * vec3<f32>(0.44, 1.7, 0.44);
    let world_pos = scaled + vec3<f32>(world_x, y_center, world_z);
    
    // Store local position for face edge detection
    out.local_pos = scaled;
    out.amp = amp;
    
    // Project position
    var clip_pos = camera.proj_matrix * camera.view_matrix * vec4<f32>(world_pos, 1.0);
    
    // Apply barrel distortion in clip space (NDC)
    let ndc = clip_pos.xy / clip_pos.w;
    let r2 = dot(ndc, ndc);
    let distorted_ndc = ndc * (1.0 + r2 * 0.055);
    
    clip_pos = vec4<f32>(distorted_ndc * clip_pos.w, clip_pos.z, clip_pos.w);
    
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
    let box_half = vec3<f32>(0.22, 0.85, 0.22);
    let d = box_half - abs(in.local_pos);
    
    // Find the second smallest element of d (distance to nearest edge on this face)
    let dist_to_edge = min(max(d.x, d.y), max(min(d.x, d.y), d.z));
    
    let thick = 0.009 + clamp(in.amp / 100.0, 0.0, 1.0) * 0.007;
    // Wide glow boundary for rich visual aesthetics
    let glow_width = 0.09 + clamp(in.amp / 100.0, 0.0, 1.0) * 0.04;
    
    if (dist_to_edge > glow_width) {
        discard;
    }
    
    // 3. Color & Alpha calculation
    let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x + audio.spectrum[2].x, 0.0, 1.0);
    
    let base_green = vec3<f32>(0.02, 1.0, 0.38);
    let neon_green = mix(base_green, vec3<f32>(1.0, 1.0, 1.0), clamp(bass * 0.45, 0.0, 1.0));
    
    var final_color = vec3<f32>(0.0);
    var alpha = 0.0;
    
    if (dist_to_edge < thick) {
        // Bright phosphor core
        let core_intensity = 1.3 + clamp(in.amp / 100.0, 0.0, 1.0) * 0.7;
        final_color = vec3<f32>(0.7, 1.0, 0.88) * core_intensity;
        alpha = 1.0;
    } else {
        // Outer glow
        let glow_factor = smoothstep(glow_width, thick, dist_to_edge);
        let glow_intensity = 0.35 + clamp(bass * 0.35, 0.0, 0.6);
        final_color = neon_green * glow_factor * glow_intensity;
        alpha = 0.85 * glow_factor;
    }
    
    // 4. CRT Filter: Scanlines (applied to color & alpha so background shows through gaps)
    let scanline = 0.86 + 0.14 * cos(in.clip_position.y * 3.14159);
    final_color = final_color * scanline;
    alpha = alpha * scanline;
    
    // 5. CRT Filter: Flicker
    let flicker = 0.98 + 0.02 * sin(audio.time * 115.0);
    final_color = final_color * flicker;
    alpha = alpha * flicker;
    
    // 6. CRT Filter: Analog static noise
    let noise_val = hash21(in.clip_position.xy + fract(audio.smooth_time) * 149.0);
    let static_noise = noise_val * 0.022 * bezel_mask;
    final_color = final_color + vec3<f32>(static_noise);
    alpha = clamp(alpha + static_noise * 0.5, 0.0, 1.0);
    
    // Apply bezel mask
    final_color = final_color * bezel_mask;
    alpha = alpha * bezel_mask;
    
    // 7. Fitted ACES Tonemap
    var final_col = (final_color * (2.51 * final_color + 0.03)) / (final_color * (2.43 * final_color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));
    
    return vec4<f32>(final_col, alpha);
}
