// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn sdWireframeBox(p: vec3<f32>, b: vec3<f32>, e: f32) -> f32 {
    let d = abs(p) - b;
    let q = abs(d + vec3<f32>(e)) - vec3<f32>(e);
    
    let dx = length(max(vec3<f32>(d.x, q.y, q.z), vec3<f32>(0.0))) + min(max(d.x, max(q.y, q.z)), 0.0);
    let dy = length(max(vec3<f32>(q.x, d.y, q.z), vec3<f32>(0.0))) + min(max(q.x, max(d.y, q.z)), 0.0);
    let dz = length(max(vec3<f32>(q.x, q.y, d.z), vec3<f32>(0.0))) + min(max(q.x, max(q.y, d.z)), 0.0);
    
    return min(min(dx, dy), dz);
}

fn get_history_amplitude(bin: u32, steps_ago: u32) -> f32 {
    let row = (audio.heatmap_row + 1024u - (steps_ago & 1023u)) & 1023u;
    return textureLoad(history_tex, vec2<i32>(i32(bin), i32(row)), 0).x;
}

struct SceneResult {
    d: f32,
    hit_type: i32,
    amp: f32,
};

fn evaluate_cell(p: vec3<f32>, ix: i32, iz: i32, res_in: SceneResult) -> SceneResult {
    var res = res_in;
    let x_spacing = 1.35;
    let z_spacing = 1.35;
    let cx = f32(ix) * x_spacing;
    let cz = f32(iz) * z_spacing;
    
    let dist_from_center = sqrt(f32(ix * ix + iz * iz));
    let bin = clamp(u32(dist_from_center * 10.0) + 4u, 0u, 255u);
    let steps_ago = u32(dist_from_center * 3.0);
    let amp = get_history_amplitude(bin, steps_ago);
    let shift = clamp(amp / 100.0, 0.0, 1.0) * 1.5;
    
    let thick = 0.009 + clamp(amp / 100.0, 0.0, 1.0) * 0.007;
    let b = vec3<f32>(0.22, 0.85, 0.22);
    
    // Mathematically proven: Only the box on the same side of y=0.0 as p.y can possibly be the closest.
    if (p.y < 0.0) {
        let y_center_b = -2.9 + shift;
        let d_b = sdWireframeBox(p - vec3<f32>(cx, y_center_b, cz), b, thick);
        if (d_b < res.d) {
            res.d = d_b;
            res.hit_type = 1;
            res.amp = amp;
        }
    } else {
        let y_center_t = 2.9 - shift;
        let d_t = sdWireframeBox(p - vec3<f32>(cx, y_center_t, cz), b, thick);
        if (d_t < res.d) {
            res.d = d_t;
            res.hit_type = 2;
            res.amp = amp;
        }
    }
    
    return res;
}

fn map_scene(p: vec3<f32>) -> SceneResult {
    let x_spacing = 1.35;
    let z_spacing = 1.35;
    
    let ix_center = i32(round(p.x / x_spacing));
    let iz_center = i32(round(p.z / z_spacing));
    
    var res: SceneResult;
    res.d = 1e10;
    res.hit_type = 0;
    res.amp = 0.0;
    
    // Early return if completely out of bounds of the grid
    if (ix_center < -16 || ix_center > 16 || iz_center < -21 || iz_center > 7) {
        return res;
    }
    
    let cx_center = f32(ix_center) * x_spacing;
    let cz_center = f32(iz_center) * z_spacing;
    
    // 1. Evaluate center cell first to establish a tight bound
    if (ix_center >= -15 && ix_center <= 15 && iz_center >= -20 && iz_center <= 6) {
        res = evaluate_cell(p, ix_center, iz_center, res);
    }
    
    // 2. Early return if the current best distance is closer than any neighbor cell could possibly be.
    // The closest horizontal distance to any box in a neighbor is at least 1.13 - distance_to_center_cell.
    let dx_center_dist = abs(p.x - cx_center);
    let dz_center_dist = abs(p.z - cz_center);
    let d_neighbor_min = min(1.13 - dx_center_dist, 1.13 - dz_center_dist);
    if (res.d < d_neighbor_min) {
        return res;
    }
    
    // 3. Evaluate neighbor cells only if they are within the dynamic bounds and closer.
    // If res.d <= 1.13, we can skip neighbors on the opposite side of the center cell.
    var dx_start = -1;
    var dx_end = 1;
    if (res.d <= 1.13) {
        if (p.x >= cx_center) {
            dx_start = 0;
        } else {
            dx_end = 0;
        }
    }
    
    var dz_start = -1;
    var dz_end = 1;
    if (res.d <= 1.13) {
        if (p.z >= cz_center) {
            dz_start = 0;
        } else {
            dz_end = 0;
        }
    }
    
    for (var dx = dx_start; dx <= dx_end; dx = dx + 1) {
        for (var dz = dz_start; dz <= dz_end; dz = dz + 1) {
            if (dx == 0 && dz == 0) { continue; }
            
            let ix = ix_center + dx;
            let iz = iz_center + dz;
            
            if (ix < -15 || ix > 15 || iz < -20 || iz > 6) { continue; }
            
            let cx = f32(ix) * x_spacing;
            let cz = f32(iz) * z_spacing;
            
            // Strict mathematical lower bound check: distance to cell boundary
            let d_horizontal = max(abs(p.x - cx) - 0.22, abs(p.z - cz) - 0.22);
            if (d_horizontal >= res.d) { continue; }
            
            res = evaluate_cell(p, ix, iz, res);
        }
    }
    
    return res;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // 1. CRT Barrel Distortion
    let crt_uv = in.uv * 2.0 - 1.0;
    let r2 = dot(crt_uv, crt_uv);
    let distorted_uv = crt_uv * (1.0 + r2 * 0.055);
    
    // Render black bezel border
    if (abs(distorted_uv.x) > 1.0 || abs(distorted_uv.y) > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    
    // Smooth bezel glow boundary
    let border_dist = min(1.0 - abs(distorted_uv.x), 1.0 - abs(distorted_uv.y));
    let bezel_mask = smoothstep(0.0, 0.03, border_dist);
    
    // 2. Aspect Ratio Correction
    var aspect = 1.7777;
    let dy = abs(dpdy(in.uv.y));
    let dx = abs(dpdx(in.uv.x));
    if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }
    let p = vec2<f32>(distorted_uv.x * aspect, -distorted_uv.y);
    
    // 3. Camera (Fixed, motionless)
    let ro = vec3<f32>(0.0, 0.0, 7.2);
    let look_at = vec3<f32>(0.0, 0.0, 0.0);
    
    let cw = normalize(look_at - ro);
    let cu = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), cw));
    let cv = normalize(cross(cw, cu));
    let rd = normalize(p.x * cu + p.y * cv + 1.5 * cw);
    
    // Audio Reactivity Calculations
    let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x + audio.spectrum[2].x, 0.0, 1.0);
    let mid_high = clamp((audio.spectrum[10].x + audio.spectrum[30].x + audio.spectrum[50].x) * 0.33, 0.0, 1.0);
    
    // Brightness shifts from green to white on bass hits
    let base_green = vec3<f32>(0.02, 1.0, 0.38);
    let neon_green = mix(base_green, vec3<f32>(1.0, 1.0, 1.0), clamp(bass * 0.45, 0.0, 1.0));
    
    // 4. Raymarching
    var t = 0.02;
    var glow = 0.0;
    var hit = false;
    var hit_amp = 0.0;
    var hit_type = 0;
    
    for (var i = 0; i < 90; i = i + 1) {
        let p_hit = ro + rd * t;
        let res = map_scene(p_hit);
        
        // Glow accumulation (inverse square falloff)
        glow = glow + 0.0013 / (res.d * res.d + 0.0007);
        
        if (res.d < 0.0025) {
            hit = true;
            hit_amp = res.amp;
            hit_type = res.hit_type;
            break;
        }
        
        t = t + res.d * 0.85;
        if (t > 30.0) { break; }
    }
    
    var final_color = vec3<f32>(0.0);
    
    if (hit) {
        // Bright phosphor core
        let core_intensity = 1.3 + clamp(hit_amp / 100.0, 0.0, 1.0) * 0.7;
        final_color = vec3<f32>(0.7, 1.0, 0.88) * core_intensity;
    }
    
    // Add accumulated neon glow
    let glow_intensity = 0.015 + clamp(bass * 0.015, 0.0, 0.02);
    final_color = final_color + neon_green * glow * glow_intensity;
    
    // 5. Floor & Ceiling Grid planes
    var t_floor = -1.0;
    if (rd.y < -0.001) { t_floor = (-3.2 - ro.y) / rd.y; }
    var t_ceil = -1.0;
    if (rd.y > 0.001) { t_ceil = (3.2 - ro.y) / rd.y; }
    
    let x_spacing = 1.35;
    
    var grid_intensity = 0.0;
    if (t_floor > 0.0 && t_floor < 25.0 && (!hit || t_floor < t)) {
        let p_floor = ro + rd * t_floor;
        // Compute anti-aliased grid lines on the plane
        let grid_uv = fract(p_floor.xz / x_spacing - 0.5) - 0.5;
        let dist_to_line = min(abs(grid_uv.x), abs(grid_uv.y));
        let line_w = 0.02 * (1.0 + t_floor * 0.05);
        let grid_line = smoothstep(line_w, 0.0, dist_to_line);
        let fade = smoothstep(25.0, 4.0, t_floor);
        grid_intensity = grid_intensity + grid_line * fade;
    }
    if (t_ceil > 0.0 && t_ceil < 25.0 && (!hit || t_ceil < t)) {
        let p_ceil = ro + rd * t_ceil;
        let grid_uv = fract(p_ceil.xz / x_spacing - 0.5) - 0.5;
        let dist_to_line = min(abs(grid_uv.x), abs(grid_uv.y));
        let line_w = 0.02 * (1.0 + t_ceil * 0.05);
        let grid_line = smoothstep(line_w, 0.0, dist_to_line);
        let fade = smoothstep(25.0, 4.0, t_ceil);
        grid_intensity = grid_intensity + grid_line * fade;
    }
    final_color = final_color + neon_green * grid_intensity * 0.35;
    // 6. Phosphor background glow wash
    let center_dist = length(distorted_uv);
    let bg_glow = vec3<f32>(0.005, 0.038, 0.016) * (1.0 - center_dist * 0.55);
    final_color = final_color + bg_glow;
    
    // 7. Bezel fade vignette
    final_color = final_color * bezel_mask;
    
    // 8. CRT Filter: Scanlines
    let scanline = 0.86 + 0.14 * cos(in.clip_position.y * 3.14159);
    final_color = final_color * scanline;
    
    // 9. CRT Filter: Flicker
    let flicker = 0.98 + 0.02 * sin(audio.time * 115.0);
    final_color = final_color * flicker;
    
    // 10. CRT Filter: Analog static noise
    let noise_val = hash21(in.clip_position.xy + fract(audio.smooth_time) * 149.0);
    let static_noise = noise_val * 0.022 * bezel_mask;
    final_color = final_color + vec3<f32>(static_noise);
    
    // 11. Fitted ACES Tonemap
    var final_col = (final_color * (2.51 * final_color + 0.03)) / (final_color * (2.43 * final_color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));
    
    return vec4<f32>(final_col, 1.0);
}
