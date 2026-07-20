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
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) audio_hit: f32,
    @location(4) local_u: f32,
    @location(5) is_grid: f32,
    @location(6) inst_idx: u32,
}

// HSV to RGB helper for rainbow effects
fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + k.xyz) * 6.0 - k.www);
    return c.z * mix(k.xxx, clamp(p - k.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

// Mathematical definition of the 3D Lissajous curves for each instance
fn get_lissajous_path(t: f32, inst_idx: u32) -> vec3<f32> {
    let time = audio.smooth_time * 0.45;
    var p0 = vec3<f32>(0.0);

    // Audio reactive amplitudes
    let bass = clamp(audio.spectrum[2].x * 1.3, 0.0, 1.5);
    let low_mid = clamp(audio.spectrum[12].x * 1.5, 0.0, 1.5);
    let mid = clamp(audio.spectrum[32].x * 1.5, 0.0, 1.5);
    let high_mid = clamp(audio.spectrum[75].x * 1.8, 0.0, 1.5);
    let treble = clamp(audio.spectrum[160].x * 2.2, 0.0, 1.5);

    if (inst_idx == 0u) {
        // Bass Scope (Far Left) - classic 1:2 figure-8
        let size = 0.95 + bass * 0.4;
        let phi_x = time * 1.5;
        let phi_y = time * 0.9;
        p0.x = sin(t + phi_x);
        p0.y = sin(2.0 * t + phi_y) * 0.75;
        p0.z = cos(t) * 0.25;
        p0 = p0 * size;
    } else if (inst_idx == 1u) {
        // Low-Mids Scope (Mid Left) - 3:2 loops
        let size = 0.85 + low_mid * 0.35;
        let phi_x = time * 1.0;
        let phi_y = time * 0.75 + 1.5;
        let phi_z = time * 0.4;
        p0.x = sin(3.0 * t + phi_x);
        p0.y = sin(2.0 * t + phi_y);
        p0.z = sin(3.0 * t + phi_z) * 0.35;
        p0 = p0 * size;
    } else if (inst_idx == 2u) {
        // Center - Main Complex Torus Knot
        // Modulate torus knot winding numbers dynamically
        let p_wind = 3.0 + sin(time * 0.25) * 0.5;
        let q_wind = 7.0 + cos(time * 0.35) * 1.0;
        let r1 = 1.35 + sin(time * 0.5) * 0.2 + bass * 0.25;
        let r2 = 0.55 + cos(time * 0.7) * 0.1 + mid * 0.15;
        
        let phi = p_wind * t + time;
        let theta = q_wind * t;
        
        let r = r1 + r2 * cos(theta);
        p0.x = r * cos(phi);
        p0.z = r * sin(phi);
        p0.y = r2 * sin(theta);
        p0 = p0 * 0.8;
    } else if (inst_idx == 3u) {
        // High-Mids Scope (Mid Right) - 5:4 grid knot
        let size = 0.8 + high_mid * 0.35;
        let phi_x = time * 1.25;
        let phi_y = time * 0.95;
        let phi_z = time * 0.65;
        p0.x = sin(5.0 * t + phi_x);
        p0.y = sin(4.0 * t + phi_y);
        p0.z = sin(5.0 * t + phi_z) * 0.4;
        p0 = p0 * size;
    } else if (inst_idx == 4u) {
        // Treble Scope (Far Right) - 7:9 dense knot with treble jitter
        let size = 0.75 + treble * 0.3;
        let phi_x = time * 1.8;
        let phi_y = time * 1.3;
        let phi_z = time * 0.95;
        
        // High frequency galvo scanner jitter
        let jitter_phase = t * 180.0 + time * 75.0;
        let jitter = sin(jitter_phase) * treble * 0.1;
        
        p0.x = sin(7.0 * t + phi_x) + jitter;
        p0.y = sin(9.0 * t + phi_y) + jitter;
        p0.z = sin(7.0 * t + phi_z) * 0.45;
        p0 = p0 * size;
    }
    
    return p0;
}

@vertex
fn vs_main_3d(in: VertexInput, @builtin(instance_index) inst_idx: u32) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    let u = in.tex_coords.x; // 0 to 1 along the length
    let v = in.tex_coords.y; // 0 to 1 around the cross section (tube or grid)
    
    if (inst_idx < 5u) {
        // --- Lissajous Tubes ---
        let t = u * 3.14159265 * 2.0;
        let theta = v * 3.14159265 * 2.0;
        
        // Determine sampling frequency bin based on instance index
        var sample_bin = 10;
        if (inst_idx == 0u) { sample_bin = 4; }
        else if (inst_idx == 1u) { sample_bin = 15; }
        else if (inst_idx == 2u) { sample_bin = 35; }
        else if (inst_idx == 3u) { sample_bin = 80; }
        else if (inst_idx == 4u) { sample_bin = 160; }
        
        // Sample audio history for pulsing thickness
        let history_depth = u32(fract(u * 2.0) * 119.0);
        let tex_y = (audio.heatmap_row + 1024u - history_depth) % 1024u;
        let hit_val = textureLoad(history_tex, vec2<i32>(sample_bin, i32(tex_y)), 0).x;
        let clamped_hit = clamp(hit_val, 0.0, 1.5);
        
        // Base tube radius (thinner for sleek laser style)
        let radius = 0.018 + clamped_hit * 0.038;
        
        // Tangent and normal calculations for tube extrusion
        let e = 0.005;
        let p0 = get_lissajous_path(t, inst_idx);
        let p1 = get_lissajous_path(t + e, inst_idx);
        let tangent = normalize(p1 - p0);
        
        var up = vec3<f32>(0.0, 1.0, 0.0);
        if (abs(tangent.y) > 0.99) {
            up = vec3<f32>(1.0, 0.0, 0.0);
        }
        
        let normal_dir = normalize(cross(tangent, up));
        let binormal = cross(normal_dir, tangent);
        
        let tube_offset = normal_dir * cos(theta) + binormal * sin(theta);
        let final_pos_local = p0 + tube_offset * radius;
        
        // Slowly rotate the scopes in 3D
        let rot_time = audio.smooth_time * 0.12;
        let c_y = cos(rot_time);
        let s_y = sin(rot_time);
        let rot_y = mat3x3<f32>(
            c_y, 0.0, s_y,
            0.0, 1.0, 0.0,
            -s_y, 0.0, c_y
        );
        
        let rot_time2 = audio.smooth_time * 0.06;
        let c_x = cos(rot_time2);
        let s_x = sin(rot_time2);
        let rot_x = mat3x3<f32>(
            1.0, 0.0, 0.0,
            0.0, c_x, -s_x,
            0.0, s_x, c_x
        );
        
        let rotated_pos = rot_x * rot_y * final_pos_local;
        
        // Distribute the 5 scopes horizontally across the widescreen area
        let world_center = vec3<f32>((f32(inst_idx) - 2.0) * 2.6, 1.5, 6.0);
        let world_pos = rotated_pos + world_center;
        
        out.world_pos = world_pos;
        out.normal = normalize(rot_x * rot_y * tube_offset);
        out.uv = in.tex_coords;
        out.audio_hit = hit_val;
        out.local_u = u;
        out.is_grid = 0.0;
        out.inst_idx = inst_idx;
        out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(world_pos, 1.0);
    } else {
        // --- Holographic Background Grid ---
        let x_grid = (u * 2.0 - 1.0) * 9.5;
        let y_grid = (v * 2.0 - 1.0) * 4.5 + 1.5;
        
        // Map u to frequency bins, v to history depth for waterfall effect
        let bin = clamp(i32(u * 255.0), 0, 255);
        let history_depth = u32(v * 119.0);
        let tex_y = (audio.heatmap_row + 1024u - history_depth) % 1024u;
        
        let amp = textureLoad(history_tex, vec2<i32>(bin, i32(tex_y)), 0).x;
        let clamped_amp = clamp(amp * 1.5, 0.0, 2.0);
        
        // Warp Z depth based on frequency waterfall amplitude
        let grid_z = 8.2 - clamped_amp * 1.2;
        let final_pos = vec3<f32>(x_grid, y_grid, grid_z);
        
        out.world_pos = final_pos;
        out.normal = vec3<f32>(0.0, 0.0, -1.0);
        out.uv = in.tex_coords;
        out.audio_hit = amp;
        out.local_u = u;
        out.is_grid = 1.0;
        out.inst_idx = inst_idx;
        out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(final_pos, 1.0);
    }
    
    return out;
}

// Anti-aliased grid lines helper using screen-space derivatives
fn grid_line(pos: vec2<f32>, line_width: f32) -> f32 {
    let deriv = fwidth(pos);
    let f = abs(fract(pos - 0.5) - 0.5);
    let grid = smoothstep(deriv * line_width, vec2<f32>(0.0), f);
    return max(grid.x, grid.y);
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let cam_eye = vec3<f32>(0.0, 1.5, -2.0);
    let view_dir = normalize(cam_eye - in.world_pos);

    if (in.is_grid > 0.5) {
        // --- Render Holographic Background Grid & Scope Bezels ---
        
        // 1. Base grid lines (scrolling look)
        let grid_uv = in.uv * vec2<f32>(40.0, 20.0);
        let grid_mask = grid_line(grid_uv, 0.8);
        
        let base_grid_color = vec3<f32>(0.0, 0.05, 0.03);
        let hit_glow = vec3<f32>(0.0, 0.4, 0.28) * clamp(in.audio_hit * 1.5, 0.0, 1.0);
        let grid_color = (base_grid_color + hit_glow) * grid_mask;
        
        // 2. Scope Bezels & Calibration Crosshairs
        var bezel_acc = 0.0;
        var ticks_acc = 0.0;
        
        for (var i = 0u; i < 5u; i = i + 1u) {
            let x_center = (f32(i) - 2.0) * 2.6;
            let center_pos = vec2<f32>(x_center, 1.5);
            let d = length(in.world_pos.xy - center_pos);
            
            var R = 0.95;
            if (i == 2u) { R = 1.25; }
            
            // Neon bezel ring
            let ring = smoothstep_r(0.015, 0.0, abs(d - R));
            let ring_glow = exp(-abs(d - R) * 16.0) * 0.35;
            bezel_acc = max(bezel_acc, ring + ring_glow);
            
            // Crosshair tick marks
            let tick_w = 0.012;
            let is_on_axis_x = smoothstep_r(tick_w, 0.0, abs(in.world_pos.x - x_center));
            let is_on_axis_y = smoothstep_r(tick_w, 0.0, abs(in.world_pos.y - 1.5));
            let ticks = (is_on_axis_x + is_on_axis_y) * step(d, R) * step(R - 0.2, d) * (0.35 + 0.65 * abs(sin(d * 40.0)));
            ticks_acc = max(ticks_acc, ticks);
        }
        
        let bezel_color = vec3<f32>(0.0, 0.3, 0.2) * bezel_acc + vec3<f32>(0.0, 0.2, 0.15) * ticks_acc;
        
        // Combined colors with screen vignette
        var color = grid_color + bezel_color;
        let vignette = smoothstep_r(11.0, 6.0, length(in.world_pos.xy - vec2<f32>(0.0, 1.5)));
        color += vec3<f32>(0.002, 0.008, 0.006) * vignette;
        
        return vec4<f32>(color, 1.0);
    } else {
        // --- Render Lissajous Laser Beams ---
        let n = normalize(in.normal);
        
        // 1. Assign Laser colors to instances
        var laser_color = vec3<f32>(1.0);
        if (in.inst_idx == 0u) {
            laser_color = vec3<f32>(1.0, 0.15, 0.08); // Neon Red-Crimson
        } else if (in.inst_idx == 1u) {
            laser_color = vec3<f32>(1.0, 0.55, 0.05); // Neon Orange-Amber
        } else if (in.inst_idx == 2u) {
            let hue = fract(in.local_u * 1.5 + audio.smooth_time * 0.06);
            laser_color = hsv2rgb(vec3<f32>(hue, 0.95, 1.0)); // Rainbow Center
        } else if (in.inst_idx == 3u) {
            laser_color = vec3<f32>(0.0, 0.95, 0.75); // Neon Cyan-Teal
        } else if (in.inst_idx == 4u) {
            laser_color = vec3<f32>(1.0, 0.05, 0.65); // Neon Magenta-Pink
        }
        
        // 2. Vector Scanner Drawing Persistence (Head & Tail)
        let scan_speed = audio.smooth_time * 1.35;
        let dist_from_head = fract(in.local_u - scan_speed);
        let tail_fade = exp(-dist_from_head * 4.5);
        let scan_intensity = mix(0.22, 1.0, tail_fade);
        
        // 3. Laser core and bloom aesthetics
        let core_align = max(dot(n, view_dir), 0.0);
        let glow_intensity = pow(1.0 - core_align, 3.0) * 0.88 + 0.12;
        let laser_glow = laser_color * glow_intensity * 2.3;
        let laser_core = vec3<f32>(1.0) * pow(core_align, 12.0) * 1.8;
        
        // Head scan spark
        let head_spark = smoothstep_r(0.012, 0.0, dist_from_head) * 3.0;
        let spark_color = (laser_color + vec3<f32>(1.0)) * 0.5 * head_spark;
        
        // Parallel laser filaments wrapping the tube
        let filaments = smoothstep_r(0.12, 0.0, abs(sin(in.uv.y * 3.14159 * 4.0) - 0.5));
        let filament_intensity = mix(0.45, 1.0, filaments);
        
        var color = (laser_glow * filament_intensity + laser_core) * scan_intensity + spark_color;
        
        // 4. Ambient Specularity
        let l1 = normalize(vec3<f32>(1.0, 1.0, 1.0));
        let half1 = normalize(l1 + view_dir);
        let spec = pow(max(dot(n, half1), 0.0), 32.0);
        color += laser_color * spec * 0.5 * scan_intensity;
        
        // Audio reactive brightness
        let safe_hit = clamp(in.audio_hit, 0.0, 1.5);
        color += laser_color * safe_hit * 0.7 * scan_intensity;
        
        // Fog & Tonemapping
        let dist = length(in.world_pos - cam_eye);
        let fog = smoothstep(5.0, 15.0, dist);
        color = mix(color, vec3<f32>(0.0), fog);
        
        color = aces_tonemap(color);
        
        return vec4<f32>(color, 1.0);
    }
}
