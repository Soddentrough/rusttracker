// INCLUDE: common

@group(0) @binding(0) var<uniform> uniforms: AudioUniforms;
@group(0) @binding(2) var heatmap_tex: texture_2d<f32>;

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var pp = p;
    let rot = mat2x2<f32>(0.87758, 0.47942, -0.47942, 0.87758);
    for (var i = 0; i < 3; i = i + 1) { // Reduced to 3 octaves for performance
        v = v + a * value_noise2d(pp);
        pp = rot * pp * 2.0 + vec2<f32>(100.0, 100.0);
        a = a * 0.5;
    }
    return v;
}

// Smooth blackbody thermal ramp (0.0 -> deep ember red -> orange -> yellow -> white-hot)
fn fuse_blackbody(heat: f32) -> vec3<f32> {
    let h = max(heat, 0.0);
    var col = vec3<f32>(
        smoothstep(0.02, 0.32, h),
        smoothstep(0.22, 0.68, h),
        smoothstep(0.55, 0.98, h)
    );
    col.r = max(col.r, smoothstep(0.0, 0.25, h) * 0.85);
    col.g = pow(col.g, 1.15);
    return clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));
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
    
    // Only render if rect is valid (width AND height non-zero to avoid div-by-zero below)
    if (meters_rect_max.x > meters_rect_min.x && meters_rect_max.y > meters_rect_min.y && uv.x >= meters_rect_min.x && uv.x <= meters_rect_max.x && uv.y >= meters_rect_min.y && uv.y <= meters_rect_max.y) {
        let num_ch = uniforms.num_channels;
        if (num_ch > 0u) {
            let meter_width_uv = (meters_rect_max.x - meters_rect_min.x) / f32(num_ch);

            // Determine which channel we are in based on x
            let ch_f = (uv.x - meters_rect_min.x) / meter_width_uv;
            let ch_idx = min(u32(ch_f), num_ch - 1u);

            {
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
                        // Clamp so a full-scale (0 dBFS) peak lands on the top segment (30 -> 29)
                        let peak_segment = min(floor(peak * num_segments), num_segments - 1.0);
                        
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
    } else if (hm_rect_max.x > hm_rect_min.x && hm_rect_max.y > hm_rect_min.y && uv.x >= hm_rect_min.x && uv.x <= hm_rect_max.x && uv.y >= hm_rect_min.y && uv.y <= hm_rect_max.y) {
        // Pattern Heatmap
        
        let ch_f = (uv.x - hm_rect_min.x) / (hm_rect_max.x - hm_rect_min.x);
        let x_idx = u32(clamp(ch_f * 256.0, 0.0, 255.99));
        
        let y_f = (uv.y - hm_rect_min.y) / (hm_rect_max.y - hm_rect_min.y);
        let y_idx = u32(clamp(y_f * 120.0, 0.0, 119.99));
        
        let physical_row = (uniforms.heatmap_row + 1024u - y_idx) % 1024u;
        let val = textureLoad(heatmap_tex, vec2<i32>(i32(x_idx), i32(physical_row)), 0).r;
        
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
    } else if (fire_rect_max.x > fire_rect_min.x && fire_rect_max.y > fire_rect_min.y && uv.x >= fire_rect_min.x && uv.x <= fire_rect_max.x && uv.y >= fire_rect_min.y && uv.y <= fire_rect_max.y) {
        // Lit Fuse FX Progress Bar (Braided Cannon Fuse + Ballistic Pyro Sparks)
        var progress = 0.0;
        if (uniforms.duration > 0.0) {
            progress = clamp(uniforms.time / uniforms.duration, 0.0, 1.0);
        }

        let rect_uv_size = fire_rect_max - fire_rect_min;
        let local_x = (uv.x - fire_rect_min.x) / rect_uv_size.x;
        let local_y = (fire_rect_max.y - uv.y) / rect_uv_size.y; // 0 at bottom, 1 at top

        // Derive physical pixel aspect ratio of the progress bar from clip_position / uv
        let screen_px = in.clip_position.xy / max(uv, vec2<f32>(1e-4));
        let rect_px = rect_uv_size * screen_px;
        let aspect = clamp(rect_px.x / max(rect_px.y, 1.0), 8.0, 150.0);

        // Aspect-normalized horizontal distance from playhead (1.0 == 25 * bar_height physical pixels)
        let dx = (progress - local_x) * (aspect / 25.0);
        let fuse_y = 0.28;
        let dy = local_y - fuse_y;
        let cord_half_h = 0.11;

        let fire_act = clamp(uniforms.fire_intensity, 0.0, 1.0);
        let audio_vu = clamp((uniforms.channels[0].x + uniforms.channels[0].y) * 0.5, 0.0, 1.0);
        let audio_peak = clamp(max(uniforms.channel_peaks[0].x, uniforms.channel_peaks[0].y), 0.0, 1.0);
        let audio_pulse = clamp(audio_vu * 0.6 + max(audio_peak - audio_vu, 0.0) * 1.4, 0.0, 1.0) * fire_act;
        let audio_boost = 1.0 + audio_pulse * 0.40;

        // Accumulate in premultiplied linear RGB to prevent double-alpha darkening
        var premult_rgb = vec3<f32>(0.0);
        var accum_a = 0.0;

        // 1. Fuse Cord: Unburnt braided hemp cord ahead vs. cooling charred ash behind
        let cord_dist = abs(dy) / cord_half_h;
        if (cord_dist < 1.3) {
            let cord_mask = smoothstep_r(1.25, 0.55, cord_dist);
            let clamped_cd = min(cord_dist, 1.0);
            let cyl_shade = sqrt(max(1.0 - clamped_cd * clamped_cd, 0.0));

            // Diagonal braided rope twist pattern
            let braid_phase = local_x * aspect * 4.2 + dy * 14.0;
            let braid = 0.5 + 0.5 * sin(braid_phase * 6.2831853);
            let cross_braid = 0.5 + 0.5 * sin((local_x * aspect * 4.2 - dy * 14.0) * 6.2831853);

            if (local_x >= progress) {
                // Unburnt braided fuse cord ahead of the flame
                let rope_base = mix(
                    vec3<f32>(0.22, 0.18, 0.13),
                    vec3<f32>(0.48, 0.39, 0.28),
                    braid * 0.75 + cross_braid * 0.25
                );
                let preheat = exp(-max(-dx, 0.0) * 90.0) * (0.4 + 0.6 * fire_act);
                var cord_col = rope_base * (0.35 + 0.65 * cyl_shade);
                cord_col = mix(cord_col, vec3<f32>(1.0, 0.45, 0.08), preheat);
                let c_a = cord_mask * 0.95;
                premult_rgb = cord_col * c_a;
                accum_a = c_a;
            } else {
                // Burnt fuse behind the flame: cools exponentially with distance into dark ash
                let cool_dist = max(dx, 0.0);
                let char_dist = abs(dy + 0.02) / (cord_half_h * 0.85);
                let char_mask = smoothstep_r(1.05, 0.35, char_dist);
                let char_tex = hash21_crt(floor(vec2<f32>(local_x * aspect * 18.0, local_y * 16.0)));

                let thermal = exp(-cool_dist * (18.0 + (1.0 - fire_act) * 35.0));
                let ember_cracks = smoothstep(0.45, 0.9, braid) * exp(-cool_dist * 9.0) * (0.25 + 0.75 * fire_act);
                let total_wire_heat = clamp(thermal * (0.55 + 0.65 * fire_act) + ember_cracks * 0.75, 0.0, 1.2);

                let ash_col = vec3<f32>(0.035 + 0.025 * char_tex, 0.03, 0.032) * (0.5 + 0.5 * cyl_shade);
                let glow_col = fuse_blackbody(total_wire_heat);
                let burnt_col = mix(ash_col, glow_col, clamp(total_wire_heat * 1.3, 0.0, 1.0));
                let c_a = char_mask * 0.92;
                premult_rgb = burnt_col * c_a;
                accum_a = c_a;
            }
        }

        // 2. Active Flame Plume & Ballistic Sparks (early-out outside compact playhead window)
        if (fire_act > 0.001 && dx > -0.03 && dx < 0.16) {
            if (dx < 0.14 && dy > -0.10) {
                var x_env = 0.0;
                if (dx >= 0.0) {
                    x_env = pow(clamp(1.0 - dx / (0.11 * audio_boost), 0.0, 1.0), 1.7);
                } else {
                    x_env = pow(clamp(1.0 - abs(dx) / 0.02, 0.0, 1.0), 2.0);
                }

                // Flame rises upward (-t in y) and trails leftward (+t in x) behind the advancing fuse tip
                let t = uniforms.smooth_time * 2.4;
                let np = vec2<f32>(local_x * aspect * 3.2 + t * 1.4, dy * 6.5 - t * 2.8);
                let n1 = fbm(np);
                let n2 = fbm(np * 2.1 + vec2<f32>(t * 0.8, -t * 1.9));
                let turb = n1 * 0.65 + n2 * 0.35;

                let y_env = smoothstep(-0.08, 0.06, dy) * pow(clamp(1.0 - max(dy, 0.0) / 0.72, 0.0, 1.0), 1.35);
                let heat = x_env * y_env * turb * 2.7 * fire_act * audio_boost;
                if (heat > 0.02) {
                    let f_col = fuse_blackbody(heat);
                    let f_a = clamp(smoothstep(0.02, 0.35, heat) * 0.95, 0.0, 1.0);
                    premult_rgb = f_col * f_a + premult_rgb * (1.0 - f_a);
                    accum_a = f_a + accum_a * (1.0 - f_a);
                }
            }

            // Ballistic Spark Shower (Option B sparks ejecting from ignition point with parabolic gravity arc)
            let p_local = vec2<f32>(-dx * 25.0, dy);
            var spark_accum = 0.0;
            for (var i = 0; i < 10; i = i + 1) {
                let fi = f32(i);
                let cycle = uniforms.smooth_time * (0.95 + fi * 0.08) + fi * 1.73;
                let id = floor(cycle);
                let age = fract(cycle);
                let seed = hash21_crt(vec2<f32>(fi, id));
                let angle = mix(1.3, 4.2, seed);
                let speed = (1.2 + 1.8 * hash21_crt(vec2<f32>(id, fi))) * audio_boost;
                let vel = vec2<f32>(cos(angle) * speed, sin(angle) * speed * 0.85);
                let spark_pos = vel * age + vec2<f32>(0.0, -1.3 * age * age);
                let diff = (p_local - spark_pos) * vec2<f32>(1.0, 2.2);
                let d2 = dot(diff, diff);
                if (d2 < 0.08) {
                    let life = (1.0 - age) * fire_act;
                    spark_accum = spark_accum + exp(-d2 * 95.0) * life * 1.35;
                }
            }
            if (spark_accum > 0.01) {
                let s_col = fuse_blackbody(0.65 + spark_accum * 0.5);
                let s_a = clamp(spark_accum, 0.0, 1.0);
                premult_rgb = s_col * s_a + premult_rgb * (1.0 - s_a);
                accum_a = s_a + accum_a * (1.0 - s_a);
            }
        }

        // Convert premultiplied linear RGB back to straight alpha for wgpu::BlendState::ALPHA_BLENDING
        if (accum_a > 1e-4) {
            out_color = vec4<f32>(premult_rgb / accum_a, accum_a);
        } else {
            out_color = vec4<f32>(0.0);
        }
    }
    return out_color;
}
