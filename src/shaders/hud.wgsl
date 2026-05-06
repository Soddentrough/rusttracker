struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let u = f32((in_vertex_index << 1u) & 2u);
    let v = f32(in_vertex_index & 2u);
    out.clip_position = vec4<f32>(u * 2.0 - 1.0, -(v * 2.0 - 1.0), 0.0, 1.0);
    out.uv = vec2<f32>(u, v);
    return out;
}

struct AudioUniforms {
    spectrum: array<vec4<f32>, 128>,
    fire_heat: array<vec4<f32>, 128>,
    channels: array<vec4<f32>, 8>,
    channel_peaks: array<vec4<f32>, 8>,
    num_channels: u32,
    mode: u32,
    time: f32,
    duration: f32,
    smooth_time: f32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
    ui_meters_rect: vec4<f32>,
    ui_heatmap_rect: vec4<f32>,
    ui_fire_rect: vec4<f32>,
};

struct HeatmapHistoryStorage {
    history: array<array<f32, 64>, 120>,
};

@group(0) @binding(0) var<uniform> uniforms: AudioUniforms;
@group(0) @binding(2) var<storage, read> heatmap_storage: HeatmapHistoryStorage;

fn hash(p: vec2<f32>) -> f32 {
    let p3  = fract(vec3<f32>(p.xyx) * 0.1031);
    var p3_mut = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3_mut.x + p3_mut.y) * p3_mut.z);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i + vec2<f32>(0.0, 0.0)), 
                   hash(i + vec2<f32>(1.0, 0.0)), u.x),
               mix(hash(i + vec2<f32>(0.0, 1.0)), 
                   hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var pp = p;
    let rot = mat2x2<f32>(0.87758, 0.47942, -0.47942, 0.87758);
    for (var i = 0; i < 5; i = i + 1) {
        v = v + a * noise(pp);
        pp = rot * pp * 2.0 + vec2<f32>(100.0, 100.0);
        a = a * 0.5;
    }
    return v;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // We now use dynamic rects passed from egui!
    // However, we still need to make sure the rect isn't completely zero (first frame)
    let meters_rect_min = vec2<f32>(uniforms.ui_meters_rect.x, uniforms.ui_meters_rect.y);
    let meters_rect_max = vec2<f32>(uniforms.ui_meters_rect.z, uniforms.ui_meters_rect.w);
    
    let hm_rect_min = vec2<f32>(uniforms.ui_heatmap_rect.x, uniforms.ui_heatmap_rect.y);
    let hm_rect_max = vec2<f32>(uniforms.ui_heatmap_rect.z, uniforms.ui_heatmap_rect.w);
    
    let fire_rect_min = vec2<f32>(uniforms.ui_fire_rect.x, uniforms.ui_fire_rect.y);
    let fire_rect_max = vec2<f32>(uniforms.ui_fire_rect.z, uniforms.ui_fire_rect.w);
    
    let uv = in.uv;
    
    // Default transparent background
    var out_color = vec4<f32>(0.0);
    
    // Only render if rect is valid
    if (meters_rect_max.x > 0.0 && uv.x >= meters_rect_min.x && uv.x <= meters_rect_max.x && uv.y >= meters_rect_min.y && uv.y <= meters_rect_max.y) {
        let num_ch = uniforms.num_channels;
        if (num_ch > 0u) {
            let meter_width_uv = (meters_rect_max.x - meters_rect_min.x) / f32(num_ch);
            
            // Determine which channel we are in based on x
            let ch_f = (uv.x - meters_rect_min.x) / meter_width_uv;
            let ch_idx = u32(ch_f);
            
            if (ch_idx < num_ch) {
                // Determine position within the specific meter
                let local_x = fract(ch_f);
                
                // Keep some padding between meters
                let padding_left = 0.2;
                let padding_right = 0.8;
                
                if (local_x >= padding_left && local_x <= padding_right) {
                    let local_y = (meters_rect_max.y - uv.y) / (meters_rect_max.y - meters_rect_min.y); // 0 at bottom, 1 at top
                    
                    let vec_idx = ch_idx / 4u;
                    let component_idx = ch_idx % 4u;
                    
                    var vu = 0.0;
                    var peak = 0.0;
                    
                    let ch_vec = uniforms.channels[vec_idx];
                    let peak_vec = uniforms.channel_peaks[vec_idx];
                    
                    if component_idx == 0u {
                        vu = ch_vec.x; peak = peak_vec.x;
                    } else if component_idx == 1u {
                        vu = ch_vec.y; peak = peak_vec.y;
                    } else if component_idx == 2u {
                        vu = ch_vec.z; peak = peak_vec.z;
                    } else {
                        vu = ch_vec.w; peak = peak_vec.w;
                    }
                    
                    vu = clamp(vu, 0.0, 1.0);
                    peak = clamp(peak, 0.0, 1.0);
                    
                    // Discretize into 30 segments
                    let num_segments = 30.0;
                    let segment_idx = floor(local_y * num_segments);
                    let segment_local_y = fract(local_y * num_segments);
                    
                    // Segment padding
                    if (segment_local_y < 0.75) {
                        let active_segments = ceil(vu * num_segments);
                        let peak_segment = floor(peak * num_segments);
                        
                        let is_active = segment_idx < active_segments;
                        let is_peak = segment_idx == peak_segment && peak_segment > 0.0;
                        
                        let segment_t = segment_idx / num_segments;
                        
                        // Fire color gradient: Deep Red -> Orange -> Yellow -> White
                        var lit_color = vec3<f32>(0.0);
                        if (segment_t > 0.9) {
                            lit_color = vec3<f32>(1.0, 1.0, 1.0);
                        } else if (segment_t > 0.7) {
                            let t = (segment_t - 0.7) / 0.2;
                            lit_color = vec3<f32>(1.0, (150.0 + 105.0 * t) / 255.0, (50.0 + 205.0 * t) / 255.0);
                        } else if (segment_t > 0.3) {
                            let t = (segment_t - 0.3) / 0.4;
                            lit_color = vec3<f32>(1.0, (50.0 + 100.0 * t) / 255.0, (0.0 + 50.0 * t) / 255.0);
                        } else {
                            let t = segment_t / 0.3;
                            lit_color = vec3<f32>((150.0 + 105.0 * t) / 255.0, (0.0 + 50.0 * t) / 255.0, 0.0);
                        }
                        
                        if (is_active || is_peak) {
                            if (is_peak && !is_active) {
                                out_color = vec4<f32>(1.0, 1.0, 1.0, 1.0); // White peak
                            } else {
                                out_color = vec4<f32>(lit_color, 1.0);
                            }
                        } else {
                            // Unlit LED appearance
                            let unlit_base = 25.0 / 255.0;
                            out_color = vec4<f32>(
                                unlit_base + lit_color.r * 0.08,
                                unlit_base + lit_color.g * 0.08,
                                unlit_base + lit_color.b * 0.08,
                                1.0
                            );
                        }
                    } else {
                        // Background track for the meter
                        out_color = vec4<f32>(15.0/255.0, 15.0/255.0, 18.0/255.0, 1.0);
                    }
                } else {
                    // Draw the background track container if close to the meter
                    if (local_x >= padding_left - 0.05 && local_x <= padding_right + 0.05) {
                        out_color = vec4<f32>(15.0/255.0, 15.0/255.0, 18.0/255.0, 1.0);
                    }
                }
            }
        }
    } else if (hm_rect_max.x > 0.0 && uv.x >= hm_rect_min.x && uv.x <= hm_rect_max.x && uv.y >= hm_rect_min.y && uv.y <= hm_rect_max.y) {
        // Pattern Heatmap
        
        let ch_f = (uv.x - hm_rect_min.x) / (hm_rect_max.x - hm_rect_min.x);
        let x_idx = u32(clamp(ch_f * 64.0, 0.0, 63.99));
        
        let y_f = (uv.y - hm_rect_min.y) / (hm_rect_max.y - hm_rect_min.y);
        let y_idx = u32(clamp(y_f * 120.0, 0.0, 119.99));
        
        // time_idx 0 is newest (bottom of UI). time_idx 119 is oldest (top of UI).
        let time_idx = 119u - y_idx;
        let val = heatmap_storage.history[time_idx][x_idx];
        
        if (val > 5.0) {
            if (val > 60.0) {
                out_color = vec4<f32>(180.0/255.0, 180.0/255.0, 180.0/255.0, 1.0);
            } else if (val > 30.0) {
                out_color = vec4<f32>(1.0, 140.0/255.0, 0.0, 1.0);
            } else {
                out_color = vec4<f32>(180.0/255.0, 20.0/255.0, 20.0/255.0, 1.0);
            }
        } else {
            out_color = vec4<f32>(20.0/255.0, 20.0/255.0, 22.0/255.0, 1.0);
        }
    } else if (fire_rect_max.x > 0.0 && uv.x >= fire_rect_min.x && uv.x <= fire_rect_max.x && uv.y >= fire_rect_min.y && uv.y <= fire_rect_max.y) {
        // Fire FX Progress Bar
        
        var progress = 0.0;
        if (uniforms.duration > 0.0) {
            progress = clamp(uniforms.time / uniforms.duration, 0.0, 1.0);
        }
        
        let local_x = (uv.x - fire_rect_min.x) / (fire_rect_max.x - fire_rect_min.x);
        let local_y = (fire_rect_max.y - uv.y) / (fire_rect_max.y - fire_rect_min.y); // 0 at bottom, 1 at top
        
        let dist_behind = progress - local_x;
        
        // Base backgrounds (charred vs unplayed fuse)
        if (local_x < progress) {
            if (abs(local_y - 0.1) < 0.02) {
                out_color = vec4<f32>(1.0, 0.3, 0.0, 0.5); // Glowing fuse wire behind
            } else {
                out_color = vec4<f32>(0.0);
            }
        } else {
            if (abs(local_y - 0.1) < 0.02) {
                out_color = vec4<f32>(40.0/255.0, 40.0/255.0, 40.0/255.0, 0.8); // Unplayed fuse wire ahead
            } else {
                out_color = vec4<f32>(0.0);
            }
        }
        
        // Procedural Fire Overlay
        // Fire trails behind for 0.1, and leads ahead for 0.015
        if (dist_behind > -0.015 && dist_behind < 0.1) {
            var intensity = 0.0;
            if (dist_behind >= 0.0) {
                intensity = pow(1.0 - (dist_behind / 0.1), 2.0);
            } else {
                intensity = pow(1.0 - (abs(dist_behind) / 0.015), 2.0);
            }
            
            // Procedural FBM noise for organic fire
            let px = local_x * 80.0;
            let py = local_y * 15.0;
            let t = uniforms.smooth_time * 2.0;
            
            let n1 = fbm(vec2<f32>(px, py - t * 2.0));
            let n2 = fbm(vec2<f32>(px * 2.0 - t, py * 2.0 - t * 3.0));
            let noise_mask = (n1 * 0.7 + n2 * 0.3);
            
            // Base heat comes from proximity to playhead + noise
            var heat = intensity * noise_mask * 2.5;
            
            // Vertical falloff (flames go up)
            heat *= pow(1.0 - local_y, 1.5);
            
            var fire_color = vec4<f32>(0.0);
            if (heat > 0.8) {
                fire_color = vec4<f32>(1.0, 1.0, 1.0, 1.0); // White hot
            } else if (heat > 0.5) {
                let n = (heat - 0.5) / 0.3;
                fire_color = vec4<f32>(1.0, n, n * 0.4, 1.0); // Yellow/Orange
            } else if (heat > 0.2) {
                let n = (heat - 0.2) / 0.3;
                fire_color = vec4<f32>(1.0, n * 0.4, 0.0, 0.9); // Red/Orange
            } else if (heat > 0.05) {
                let n = (heat - 0.05) / 0.15;
                fire_color = vec4<f32>(0.4 + n * 0.6, 0.0, 0.0, 0.8); // Deep Red
            } else {
                let n = clamp(heat / 0.05, 0.0, 1.0);
                fire_color = vec4<f32>(n * 0.4, 0.0, 0.0, n * 0.8); 
            }
            
            // Alpha blend the fire over the background
            if (fire_color.a > 0.0) {
                out_color = vec4<f32>(
                    mix(out_color.rgb, fire_color.rgb, fire_color.a),
                    max(out_color.a, fire_color.a)
                );
            }
        }
        
        // Embers at the bottom, falling far behind
        if (local_x < progress && local_y < 0.2) {
            let px = local_x * 100.0;
            let static_seed = hash(vec2<f32>(floor(px), 0.0));
            if (static_seed > 0.85) {
                let pulse = sin(uniforms.smooth_time * 4.0 + px * 0.5) * 0.5 + 0.5;
                let brightness = ((static_seed - 0.85) / 0.15) * (0.3 + 0.7 * pulse);
                // Blend ember
                let ember_color = vec4<f32>(1.0, 0.25 * brightness, 0.0, brightness);
                out_color = vec4<f32>(
                    mix(out_color.rgb, ember_color.rgb, ember_color.a),
                    max(out_color.a, ember_color.a)
                );
            }
        }
    }
    return out_color;
}
