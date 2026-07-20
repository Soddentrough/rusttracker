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
    @location(2) hit_val: f32,
    @location(3) is_sky: f32,
    @location(4) local_z: f32,
}

fn hash1(n: f32) -> f32 { return fract(sin(n) * 43758.5453); }

// Integer-based hash: no sin(), no precision loss at large coordinates.
// Operates on the integer lattice so f32 magnitude doesn't matter.
fn ihash2(ix: i32, iy: i32) -> f32 {
    var n = u32(ix) * 1597334677u ^ u32(iy) * 3812015801u;
    n = n ^ (n >> 16u);
    n = n * 2654435769u;
    n = n ^ (n >> 16u);
    return f32(n & 0x00FFFFFFu) / f32(0x01000000);
}

fn noise2d(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = ihash2(i.x,     i.y);
    let b = ihash2(i.x + 1, i.y);
    let c = ihash2(i.x,     i.y + 1);
    let d = ihash2(i.x + 1, i.y + 1);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn ridge(p: vec2<f32>) -> f32 {
    return 1.0 - abs(noise2d(p) * 2.0 - 1.0);
}

// Base terrain height (static low-poly hills)
fn terrain_h(wx: f32, wz: f32, dist_z: f32) -> f32 {
    let road_half = 9.0;
    let dx = max(abs(wx) - road_half, 0.0);
    let slope = dx * 0.6;
    
    // Evaluate noise directly on continuous world coordinates to avoid vertex popping
    let r1 = ridge(vec2<f32>(wx, wz) * 0.015) * 4.0;
    let r2 = ridge(vec2<f32>(wx, wz) * 0.05) * 1.0;
    let h = r1 + r2;
    
    let horizon_fade = smoothstep_r(240.0, 60.0, dist_z);
    
    return slope * h * 0.05 * horizon_fade - 1.0;
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    // Calculate camera position
    let cam_z = audio.history_cam_z; // history-locked: 0.5 world units per history row
    
    // Map grid vertex coordinates
    let norm_x = in.position.x / 100.0;
    let local_x = (norm_x * 400.0) + (norm_x * norm_x * norm_x * 1200.0);
    
    let grid_spacing = 2.0;
    let shift_z = cam_z % grid_spacing;
    
    let is_backdrop = in.position.z > 99.5;
    let is_backdrop_base = in.position.z > 98.5 && in.position.z <= 99.5;
    
    var local_z = 0.0;
    if is_backdrop || is_backdrop_base {
        local_z = 398.0;
    } else {
        local_z = (in.position.z + 100.0) * grid_spacing - shift_z;
    }
    
    let world_x = local_x;
    var world_z_snapped = 0.0;
    if is_backdrop || is_backdrop_base {
        world_z_snapped = (local_z + cam_z) % 6000.0;
    } else {
        world_z_snapped = ((in.position.z + 100.0) * grid_spacing + grid_spacing * floor(cam_z / grid_spacing)) % 6000.0;
    }
    
    let world_z_smooth = (local_z + cam_z) % 6000.0;
    
    var h = 0.0;
    var hit_val = 0.0;
    
    if is_backdrop {
        h = 400.0;
    } else if is_backdrop_base {
        h = -1.0;
    } else {
        // Base static terrain height using snapped world Z to prevent polygon wobble
        h = terrain_h(world_x, world_z_snapped, local_z);
        
        // Use smooth continuous steps_ago to sample history_tex smoothly.
        let steps_ago_float = (400.0 - local_z) / 0.5;
        let continuous_heatmap_row = f32(audio.heatmap_row) + audio.step_fraction;
        let center_row = ((continuous_heatmap_row - steps_ago_float) % 1024.0 + 1024.0) % 1024.0;
        
        // Spatial frequency mapping: Left channel on Left side, Right channel on Right side.
        // Bass frequencies are placed on the outside edges, mids/highs closer to the road.
        var freq_bin_float = 0.0;
        if (world_x < 0.0) {
            freq_bin_float = clamp(127.0 - abs(world_x) * 1.5, 0.0, 127.0);
        } else {
            freq_bin_float = clamp(255.0 - abs(world_x) * 1.5, 128.0, 255.0);
        }
        
        let bin1 = i32(floor(freq_bin_float));
        let is_left = world_x < 0.0;
        let bin2 = clamp(bin1 + 1, select(128, 0, is_left), select(255, 127, is_left));
        let fract_bin = fract(freq_bin_float);
        
        // Average over 8 adjacent rows (a 4-world-unit window) to low-pass filter
        // the temporal audio data and eliminate row-transition jitter.
        let base_row = i32(floor(center_row));
        let fract_row = fract(center_row);
        
        var acc = 0.0;
        for (var r = 0; r < 8; r++) {
            let row1 = u32((base_row - 3 + r + 1024) % 1024);
            let row2 = (row1 + 1u) % 1024u;
            
            // Sample bin1 at row1 and row2
            let s1_r1 = textureLoad(history_tex, vec2<i32>(bin1, i32(row1)), 0).x;
            let s1_r2 = textureLoad(history_tex, vec2<i32>(bin1, i32(row2)), 0).x;
            let sample_bin1 = mix(s1_r1, s1_r2, fract_row);
            
            // Sample bin2 at row1 and row2
            let s2_r1 = textureLoad(history_tex, vec2<i32>(bin2, i32(row1)), 0).x;
            let s2_r2 = textureLoad(history_tex, vec2<i32>(bin2, i32(row2)), 0).x;
            let sample_bin2 = mix(s2_r1, s2_r2, fract_row);
            
            // Blend between the two adjacent frequency bins
            acc += mix(sample_bin1, sample_bin2, fract_bin);
        }
        let raw_hit = acc / 8.0;
        hit_val = clamp(raw_hit * 0.015, 0.0, 1.5);
        
        // Elevate the terrain into reactive peaks outside the road limits
        let road_mask = smoothstep(12.0, 30.0, abs(world_x));
        h += hit_val * 12.0 * road_mask;
    }
    
    let view_pos = vec3<f32>(local_x, h, local_z);
    
    out.world_pos = vec3<f32>(world_x, h, world_z_smooth);
    out.uv = in.tex_coords;
    out.is_sky = select(0.0, 1.0, is_backdrop);
    out.hit_val = hit_val;
    out.local_z = local_z;
    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(view_pos, 1.0);
    
    return out;
}

// Anti-aliased grid lines function using screen-space derivatives to completely eliminate moire patterns
fn grid_line(pos: vec2<f32>, line_width: f32) -> f32 {
    let deriv = fwidth(pos);
    let f = abs(fract(pos - 0.5) - 0.5);
    let grid = smoothstep(deriv * line_width, vec2<f32>(0.0), f);
    return max(grid.x, grid.y);
}

fn get_street_lamps(world_pos: vec3<f32>, cam_z: f32, bass_pulse: f32) -> vec3<f32> {
    let E = vec3<f32>(0.0, 1.5, -2.0 + cam_z); // Camera position in world space
    let V = normalize(world_pos - E);          // Ray direction in world space
    
    let road_half = 9.0;
    let sodium_color = vec3<f32>(1.0, 0.42, 0.03); // Rich sodium amber color
    
    // We loop through segments of length L = 60.0
    let L = 60.0;
    let current_seg_float = cam_z / L;
    let start_seg = i32(floor(current_seg_float)) - 1;
    let end_seg = start_seg + 8;
    
    var color_acc = vec3<f32>(0.0);
    
    for (var seg = start_seg; seg <= end_seg; seg++) {
        // Hash for left and right lamps
        let h_left = ihash2(seg, 7);
        let h_right = ihash2(seg, 42);
        
        let min_dist = 22.0;
        let z_left = f32(seg) * L + h_left * (L - min_dist);
        let z_right = f32(seg) * L + h_right * (L - min_dist);
        
        // Bass-reactive light intensity
        let lamp_intensity = 0.2 + bass_pulse * 1.8;
        
        // Left Lamp
        {
            let pole_x = -road_half - 1.2;
            let terrain_y = terrain_h(pole_x, z_left, 100.0);
            let light_pos = vec3<f32>(pole_x + 2.0, 10.6 + terrain_y, z_left);
            let dist_to_lamp = light_pos.z - cam_z;
            let lamp_fog = smoothstep_r(220.0, 50.0, dist_to_lamp);
            
            if (lamp_fog > 0.0) {
                // Ground lighting cone (sodium light pool on ground)
                let dist_xz = length(world_pos.xz - light_pos.xz);
                let cone_radius = 14.0;
                if (dist_xz < cone_radius && world_pos.y < light_pos.y) {
                    let attenuation = smoothstep_r(cone_radius, 0.0, dist_xz);
                    let dist_3d = length(world_pos - light_pos);
                    let light_pool = attenuation * (10.0 / (dist_3d * dist_3d + 1.0));
                    color_acc += sodium_color * light_pool * lamp_intensity * lamp_fog;
                }
                
                // Volumetric lighting and post/bulb rendering
                let dx = light_pos.x - E.x;
                let dz = light_pos.z - E.z;
                let t = (dx * V.x + dz * V.z) / (1.0 - V.y * V.y + 1e-6);
                if (t > 0.0 && t < length(world_pos - E)) {
                    let Q = E + t * V;
                    
                    let clamped_y = clamp(Q.y, terrain_y, light_pos.y);
                    let C = vec3<f32>(pole_x, clamped_y, light_pos.z);
                    let dist_to_post = length(Q - C);
                    
                    // Render the pole itself (dark silhouette with ambient neon orange highlight)
                    if (dist_to_post < 0.15) {
                        let post_intensity = smoothstep_r(0.15, 0.0, dist_to_post);
                        color_acc += vec3<f32>(0.05, 0.02, 0.01) * post_intensity * lamp_fog;
                    }
                    
                    // Render the bulb (bright glowing sphere at light_pos)
                    let dist_to_bulb = length(Q - light_pos);
                    let max_bulb_dist = 3.0;
                    if (dist_to_bulb < max_bulb_dist) {
                        let fade = smoothstep_r(max_bulb_dist, 0.0, dist_to_bulb);
                        var bulb_glow = (exp(-dist_to_bulb * 4.0) * 12.0 + exp(-dist_to_bulb * 1.8) * 1.2) * fade;
                        if (Q.y > light_pos.y) {
                            bulb_glow *= smoothstep_r(0.02, 0.0, Q.y - light_pos.y);
                        }
                        color_acc += sodium_color * bulb_glow * lamp_intensity * lamp_fog;
                    }
                    
                    // Render volumetric light cone in the air
                    if (Q.y < light_pos.y && Q.y > terrain_y) {
                        let height_t = (light_pos.y - Q.y) / (light_pos.y - terrain_y);
                        let cone_r = height_t * 6.0;
                        let dist_from_center = length(Q.xz - light_pos.xz);
                        if (dist_from_center < cone_r) {
                            let cone_glow = (1.0 - dist_from_center / cone_r) * smoothstep(0.0, 0.2, height_t) * (1.0 - height_t);
                            let volume_contrib = sodium_color * cone_glow * 0.18 * lamp_intensity / (1.0 + t * 0.005);
                            color_acc += volume_contrib * lamp_fog;
                        }
                    }
                }
            }
        }
        
        // Right Lamp
        {
            let pole_x = road_half + 1.2;
            let terrain_y = terrain_h(pole_x, z_right, 100.0);
            let light_pos = vec3<f32>(pole_x - 2.0, 10.6 + terrain_y, z_right);
            let dist_to_lamp = light_pos.z - cam_z;
            let lamp_fog = smoothstep_r(220.0, 50.0, dist_to_lamp);
            
            if (lamp_fog > 0.0) {
                // Ground lighting cone (sodium light pool on ground)
                let dist_xz = length(world_pos.xz - light_pos.xz);
                let cone_radius = 14.0;
                if (dist_xz < cone_radius && world_pos.y < light_pos.y) {
                    let attenuation = smoothstep_r(cone_radius, 0.0, dist_xz);
                    let dist_3d = length(world_pos - light_pos);
                    let light_pool = attenuation * (10.0 / (dist_3d * dist_3d + 1.0));
                    color_acc += sodium_color * light_pool * lamp_intensity * lamp_fog;
                }
                
                // Volumetric lighting and post/bulb rendering
                let dx = light_pos.x - E.x;
                let dz = light_pos.z - E.z;
                let t = (dx * V.x + dz * V.z) / (1.0 - V.y * V.y + 1e-6);
                if (t > 0.0 && t < length(world_pos - E)) {
                    let Q = E + t * V;
                    
                    let clamped_y = clamp(Q.y, terrain_y, light_pos.y);
                    let C = vec3<f32>(pole_x, clamped_y, light_pos.z);
                    let dist_to_post = length(Q - C);
                    
                    if (dist_to_post < 0.15) {
                        let post_intensity = smoothstep_r(0.15, 0.0, dist_to_post);
                        color_acc += vec3<f32>(0.05, 0.02, 0.01) * post_intensity * lamp_fog;
                    }
                    
                    let dist_to_bulb = length(Q - light_pos);
                    let max_bulb_dist = 3.0;
                    if (dist_to_bulb < max_bulb_dist) {
                        let fade = smoothstep_r(max_bulb_dist, 0.0, dist_to_bulb);
                        var bulb_glow = (exp(-dist_to_bulb * 4.0) * 12.0 + exp(-dist_to_bulb * 1.8) * 1.2) * fade;
                        if (Q.y > light_pos.y) {
                            bulb_glow *= smoothstep_r(0.02, 0.0, Q.y - light_pos.y);
                        }
                        color_acc += sodium_color * bulb_glow * lamp_intensity * lamp_fog;
                    }
                    
                    if (Q.y < light_pos.y && Q.y > terrain_y) {
                        let height_t = (light_pos.y - Q.y) / (light_pos.y - terrain_y);
                        let cone_r = height_t * 6.0;
                        let dist_from_center = length(Q.xz - light_pos.xz);
                        if (dist_from_center < cone_r) {
                            let cone_glow = (1.0 - dist_from_center / cone_r) * smoothstep(0.0, 0.2, height_t) * (1.0 - height_t);
                            let volume_contrib = sodium_color * cone_glow * 0.18 * lamp_intensity / (1.0 + t * 0.005);
                            color_acc += volume_contrib * lamp_fog;
                        }
                    }
                }
            }
        }
    }
    
    return color_acc;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let bass_pulse = clamp(audio.spectrum[2].x * 1.2, 0.0, 1.0);
    let treble_pulse = clamp(audio.spectrum[80].x * 1.5, 0.0, 1.0);
    
    var final_color = vec3<f32>(0.0);
    
    if in.is_sky > 0.01 {
        // --- Retro Vaporwave Sky, Clouds & Sun ---
        let p = vec2<f32>(in.world_pos.x * 0.015, in.world_pos.y * 0.015 - 0.2);
        let sky_t = clamp(p.y * 1.2 + 0.3, 0.0, 1.0);
        
        var sky_color = mix(
            vec3<f32>(0.01, 0.0, 0.04), // Deep purple
            mix(vec3<f32>(0.08, 0.0, 0.15), vec3<f32>(0.15, 0.02, 0.12), sky_t),
            sky_t
        );
        
        // Sun glow
        let sun_pos = vec2<f32>(0.0, 0.15);
        let sun_dist = length(p - sun_pos);
        let sun_radius = 0.35;
        
        let mid_energy = clamp(audio.spectrum[20].x * 0.8, 0.0, 1.0);
        sky_color += vec3<f32>(0.4, 0.1, 0.2) * exp(-sun_dist * 2.5) * (0.25 + mid_energy * 0.35);
        
        // Sun body
        if sun_dist < sun_radius && p.y > -0.05 {
            let cut = fract((p.y - sun_pos.y) * 20.0 - audio.smooth_time * (0.8 + bass_pulse * 1.5));
            let cut_threshold = mix(0.3, 0.9, clamp((p.y - sun_pos.y + 0.2) * 2.5, 0.0, 1.0));
            if cut > cut_threshold || p.y > sun_pos.y + 0.05 {
                let sun_t = clamp((p.y - sun_pos.y + 0.2) * 2.0, 0.0, 1.0);
                let sun_col = mix(vec3<f32>(1.0, 0.05, 0.4), vec3<f32>(1.0, 0.85, 0.1), sun_t);
                sky_color = mix(sky_color, sun_col, smoothstep(sun_radius, sun_radius - 0.02, sun_dist));
            }
        }
        
        // Clouds
        if p.y > 0.0 && p.y < 3.5 {
            let cu = vec2<f32>(p.x * 1.5 + audio.smooth_time * 0.025, p.y * 1.5);
            let cloud = noise2d(cu * 2.5) * 0.6 + noise2d(cu * 5.0 + vec2<f32>(3.7, 1.2)) * 0.4;
            let alt_mask = smoothstep(0.0, 0.5, p.y) * smoothstep_r(3.5, 1.5, p.y);
            let cloud_alpha = smoothstep(0.4, 0.6, cloud) * alt_mask * 0.55;
            sky_color = mix(sky_color, vec3<f32>(0.08, 0.02, 0.12), cloud_alpha);
            sky_color += vec3<f32>(0.2, 0.05, 0.1) * smoothstep_r(0.55, 0.4, cloud) * alt_mask * exp(-sun_dist * 1.5) * 0.5;
        }

        // Stars
        let star_uv = p * 80.0;
        let star_id = floor(star_uv);
        let star_rnd = hash1(star_id.x * 127.1 + star_id.y * 311.7);
        let star_b = step(0.97, star_rnd) * smoothstep_r(0.04, 0.0, length(fract(star_uv) - 0.5)) * clamp(p.y - 0.1, 0.0, 1.0);
        sky_color += vec3<f32>(star_b) * (0.8 + treble_pulse * 0.4);
        
        final_color = sky_color;
    } else {
        // --- Terrain & Road ---
        let road_half = 9.0;
        let dist_from_center = abs(in.world_pos.x);
        let hit_road = dist_from_center < road_half;
        
        var color = vec3<f32>(0.0);
        
        if hit_road {
            // Road Grid layout (elongated in Z for retro speed sensation)
            let road_grid = grid_line(vec2<f32>(in.world_pos.x * 0.25, in.world_pos.z * 0.15), 1.5);
            let asphalt_c = vec3<f32>(0.005, 0.005, 0.015);
            let road_line_c = vec3<f32>(0.0, 0.8, 1.0) * (0.8 + bass_pulse * 0.5); // Neon cyan grid lines
            color = mix(asphalt_c, road_line_c, road_grid);
            
            // High speed dashed yellow centerline (moves backward with world_pos.z)
            let is_center_line = dist_from_center < 0.25;
            if is_center_line {
                let center_dash = smoothstep(0.7, 0.9, sin(in.world_pos.z * 0.5));
                let center_c = vec3<f32>(1.0, 0.6, 0.0) * center_dash * (1.0 + treble_pulse * 1.0);
                color = mix(color, center_c, center_dash);
            }
            
            // Neon shoulder guardrails
            let is_shoulder = dist_from_center > (road_half - 0.4);
            if is_shoulder {
                let shoulder_glow = vec3<f32>(1.0, 0.0, 0.7) * (1.5 + bass_pulse * 1.5);
                color = mix(color, shoulder_glow, 0.8);
            }
        } else {
            // Mountain lighting using screen-space derivatives for flat low-poly normal calculation
            let face_normal = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
            let l = normalize(vec3<f32>(-0.3, 0.9, -0.2));
            let diff = max(dot(face_normal, l), 0.0);
            
            // Faceted terrain coloring
            let mtn_c = mix(vec3<f32>(0.01, 0.0, 0.02), vec3<f32>(0.035, 0.0, 0.07), diff);
            
            // Mountain grid lines (anti-aliased)
            let mtn_grid = grid_line(vec2<f32>(in.world_pos.x * 0.1, in.world_pos.z * 0.1), 1.2);
            
            // Audio reactive color shifting (Magenta to Cyan based on music frequency)
            let line_c = mix(vec3<f32>(1.0, 0.0, 0.6), vec3<f32>(0.0, 0.8, 1.0), in.hit_val * 0.5 + bass_pulse * 0.3);
            color = mix(mtn_c, line_c, mtn_grid);
        }
        
        // Depth fog to blend cleanly with horizon sky and hide clipping
        let dist = length(vec3<f32>(in.world_pos.x, in.world_pos.y, in.local_z) - vec3<f32>(0.0, 1.5, -2.0));
        let fog_f = smoothstep(50.0, 220.0, dist);
        let fog_c = vec3<f32>(0.04, 0.0, 0.08); // Vaporwave background purple
        final_color = mix(color, fog_c, fog_f);
    }
    
    // Add street lamps (lighting + volumetric + posts + bulbs)
    let cam_z = audio.history_cam_z; // history-locked: 0.5 world units per history row
    let lamps_color = get_street_lamps(in.world_pos, cam_z, bass_pulse);
    final_color += lamps_color;
    
    return vec4<f32>(final_color, 1.0);
}

@vertex
fn vs_lamp(in: VertexInput, @builtin(instance_index) inst_idx: u32) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    let cam_z = audio.history_cam_z; // history-locked: 0.5 world units per history row
    
    // Determine segment index and side from inst_idx
    // We have 8 segments (from current_seg to current_seg + 7)
    // inst_idx goes from 0 to 15.
    let seg_offset = i32(inst_idx) / 2;
    let is_right = (inst_idx % 2u) == 1u;
    
    let current_seg = i32(floor(cam_z / 60.0)) - 1 + seg_offset;
    
    let h_left = ihash2(current_seg, 7);
    let h_right = ihash2(current_seg, 42);
    
    let min_dist = 22.0;
    let z_left = f32(current_seg) * 60.0 + h_left * 38.0;
    let z_right = f32(current_seg) * 60.0 + h_right * 38.0;
    
    let world_z = select(z_left, z_right, is_right);
    let road_half = 9.0;
    let world_x = select(-road_half - 1.2, road_half + 1.2, is_right);
    
    // Terrain height at the base of the lamp
    let terrain_y = terrain_h(world_x, world_z, 100.0);
    
    var pos = in.position;
    
    // Mirror the overhang arm for the right side (it should point towards -X, i.e., inward/left)
    if is_right {
        pos.x = -pos.x;
    }
    
    // Offset position to world coordinates
    let world_pos = vec3<f32>(pos.x + world_x, pos.y + terrain_y, pos.z + world_z);
    
    // Project to view space
    let view_pos = vec3<f32>(world_pos.x, world_pos.y, world_pos.z - cam_z);
    
    out.world_pos = world_pos;
    out.uv = in.tex_coords;
    out.is_sky = 0.0;
    out.hit_val = in.tex_coords.x; // Pass the color_flag (0.0, 1.0, 2.0) via hit_val
    out.local_z = view_pos.z;
    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(view_pos, 1.0);
    
    return out;
}

@fragment
fn fs_lamp(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let bass_pulse = clamp(audio.spectrum[2].x * 1.2, 0.0, 1.0);
    
    let color_flag = in.hit_val;
    let sodium_color = vec3<f32>(1.0, 0.42, 0.03);
    
    var color = vec3<f32>(0.0);
    if color_flag > 1.5 {
        // Fixture (dark grey fitting with a subtle orange sodium reflection underneath)
        let local_y = in.uv.y;
        let reflection = sodium_color * (0.2 + bass_pulse * 0.5) * (1.0 - smoothstep(0.96, 0.99, local_y));
        color = vec3<f32>(0.08, 0.08, 0.12) + reflection;
    } else if color_flag > 0.5 {
        // Emissive bulb (bright sodium glow)
        let lamp_intensity = 0.5 + bass_pulse * 1.5;
        color = sodium_color * lamp_intensity * 2.5;
    } else {
        // Pole & Arm (dark metallic with a neon grid/laser glow highlight)
        let height_glow = in.uv.y; // 0.0 at base, 1.0 at top
        let pole_base = vec3<f32>(0.01, 0.005, 0.03);
        let pole_top = vec3<f32>(0.1, 0.02, 0.25) * (1.0 + bass_pulse * 0.8);
        
        // Add a neon pink racing stripe along the vertical edge of the post
        let stripe = smoothstep(0.93, 0.98, sin(in.uv.y * 35.0)); // subtle retro ridges
        let ridge_glow = vec3<f32>(1.0, 0.0, 0.6) * stripe * 0.3;
        
        color = mix(pole_base, pole_top, height_glow) + ridge_glow;
    }
    
    // Depth fog
    let cam_z = audio.history_cam_z; // history-locked: 0.5 world units per history row
    let dist_to_lamp = in.world_pos.z - cam_z;
    let fog_f = smoothstep(50.0, 220.0, dist_to_lamp);
    let fog_c = vec3<f32>(0.04, 0.0, 0.08);
    let final_color = mix(color, fog_c, fog_f);
    
    return vec4<f32>(final_color, 1.0);
}

