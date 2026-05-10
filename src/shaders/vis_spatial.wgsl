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
};

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

fn rotate2d(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

fn hash_f(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }

fn get_spatial_vu(i: u32) -> f32 {
    let v = audio.spatial_channels[i / 4u];
    let c = i % 4u;
    if c == 0u { return clamp(v.x, 0.0, 1.0); }
    else if c == 1u { return clamp(v.y, 0.0, 1.0); }
    else if c == 2u { return clamp(v.z, 0.0, 1.0); }
    else { return clamp(v.w, 0.0, 1.0); }
}

fn project_3d(p3: vec3<f32>, ro: vec3<f32>, cu: vec3<f32>, cv: vec3<f32>, cw: vec3<f32>) -> vec3<f32> {
    let dir = p3 - ro;
    let dist_w = dot(dir, cw);
    if dist_w <= 0.001 { return vec3<f32>(999.0, 999.0, dist_w); }

    let proj_x = dot(dir, cu) / dist_w;
    let proj_y = dot(dir, cv) / dist_w;
    return vec3<f32>(proj_x, -proj_y, dist_w);
}

// --- 3x5 bitmap font for debug labels ---
fn glyph_bitmap(ch: u32) -> u32 {
    // 3x5 pixel font. 15 bits per glyph, MSB = top-left.
    // Index: 0-9=digits, 10=L, 11=R, 12=C, 13=S, 14=F, 15=E, 16=space
    switch ch {
        case 0u  { return 31599u; } // 0
        case 1u  { return 11415u; } // 1
        case 2u  { return 29671u; } // 2
        case 3u  { return 29647u; } // 3
        case 4u  { return 23497u; } // 4
        case 5u  { return 31183u; } // 5
        case 6u  { return 31215u; } // 6
        case 7u  { return 29330u; } // 7
        case 8u  { return 31727u; } // 8
        case 9u  { return 31695u; } // 9
        case 10u { return 18727u; } // L: #.. #.. #.. #.. ###
        case 11u { return 31733u; } // R: ### #.# ### ##. #.#
        case 12u { return 29257u; } // C: ### #.. #.. #.. ###
        case 13u { return 31183u; } // S (same as 5)
        case 14u { return 29576u; } // F: ### #.. ### #.. #..
        case 15u { return 29671u; } // E (same as 2 shape): ### #.. ### #.. ###
        default  { return 0u; }     // space
    }
}

fn draw_label_char(ch: u32, frag: vec2<f32>, origin: vec2<f32>, px: f32) -> f32 {
    let local = frag - origin;
    if local.x < 0.0 || local.x >= px * 3.0 || local.y < 0.0 || local.y >= px * 5.0 { return 0.0; }
    let col = u32(floor(local.x / px));
    let row = u32(floor(local.y / px));
    let bit = (4u - row) * 3u + (2u - col);
    return f32((glyph_bitmap(ch) >> bit) & 1u);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let dx = dpdx(in.uv.x);
    let dy = dpdy(in.uv.y);
    let aspect = dy / max(dx, 0.00001);

    let render_channels = min(audio.num_spatial_channels, 12u);

    // --- Pre-compute LFE VU and total volume for global effects ---
    var lfe_vu = 0.0;
    if render_channels > 3u {
        lfe_vu = get_spatial_vu(3u);
    }

    var total_volume = 0.0;
    for (var i = 0u; i < render_channels; i = i + 1u) {
        total_volume += get_spatial_vu(i);
    }
    total_volume = total_volume / max(f32(render_channels), 1.0);

    // --- Wide panoramic camera with LFE micro-shake ---
    let fov_scale = 0.75;
    let p = vec2<f32>(uv.x * aspect, uv.y) * fov_scale;

    let cam_dist = 2.4;
    let cam_height = 0.65;
    let shake_x = sin(audio.time * 7.3) * lfe_vu * 0.012;
    let shake_z = sin(audio.time * 5.1 + 1.3) * lfe_vu * 0.008;
    let ro = vec3<f32>(shake_x, -cam_dist, cam_height + shake_z);
    let cam_target = vec3<f32>(0.0, 0.3, 0.15);

    let cw = normalize(cam_target - ro);
    let cu = normalize(cross(cw, vec3<f32>(0.0, 0.0, 1.0)));
    let cv = cross(cu, cw);

    let rd = normalize(cw + p.x * cu - p.y * cv);

    let room_angle = 0.0;

    // Deep dark background with subtle vertical gradient
    let sky_grad = smoothstep(-1.0, 1.0, uv.y) * 0.008;
    var color = vec3<f32>(0.012 + sky_grad, 0.012 + sky_grad * 0.5, 0.018 + sky_grad * 1.5);

    // --- Raytraced Perspective Floor ---
    var floor_xy = vec2<f32>(0.0, 0.0);
    var floor_t = 0.0;
    if rd.z < 0.0 {
        floor_t = -ro.z / rd.z;
        let hit = ro + floor_t * rd;
        floor_xy = rotate2d(hit.xy, -room_angle);

        // LFE grid warp: push grid coordinates radially outward with bass
        let dome_r = length(floor_xy);
        let warp_dir = floor_xy / max(dome_r, 0.01);
        let warp_strength = lfe_vu * 0.08 / max(dome_r, 0.2);
        let warped_xy = floor_xy + warp_dir * warp_strength;

        // Rectangular tile grid with warped coordinates
        let cell_size = 0.4;
        let grid_x = smoothstep(0.015, 0.0, abs(fract(warped_xy.x / cell_size + 0.5) - 0.5) * cell_size);
        let grid_y = smoothstep(0.015, 0.0, abs(fract(warped_xy.y / cell_size + 0.5) - 0.5) * cell_size);
        let grid_lines = max(grid_x, grid_y) * 0.4;

        // Extended perspective distance fade for wider view
        let grid_mask = 1.0 - smoothstep(1.5, 5.5, floor_t);

        color = color + vec3<f32>(0.04, 0.07, 0.1) * grid_lines * grid_mask;

        // --- LFE Pressure Dome: room-scale bass phenomenon ---
        let dome_pulse = smoothstep(2.8 + lfe_vu * 1.5, 0.0, dome_r);
        let dome_breathe = 1.0 + sin(audio.time * 6.0) * lfe_vu * 0.15;
        let lfe_color = vec3<f32>(1.0, 0.2, 0.4);
        color = color + lfe_color * dome_pulse * lfe_vu * lfe_vu * dome_breathe * 0.4;

        // LFE expanding pressure rings
        let ring_phase = audio.time * 3.0;
        let ring1 = smoothstep(0.06, 0.0, abs(dome_r - fract(ring_phase) * 3.5)) * lfe_vu;
        let ring2 = smoothstep(0.06, 0.0, abs(dome_r - fract(ring_phase + 0.5) * 3.5)) * lfe_vu;
        color = color + lfe_color * (ring1 + ring2) * 0.2 * grid_mask;

        // Ambient bass glow
        let bass = clamp(audio.spectrum[1].x * 2.0 + audio.spectrum[2].x, 0.0, 1.0);
        color = color + vec3<f32>(0.05, 0.02, 0.08) * bass * smoothstep(3.5, 0.0, dome_r);
    }

    // Dolby Atmos 7.1.4 speaker layout (SMPTE channel order)
    // Listener at origin. +Y = front wall, -Y = rear. Z = height above ear level.
    // Positions tuned to stay within viewport (camera at Y=-2.4, fov=0.75).
    //
    // NOTE: In 5.1, Ls/Rs are the ONLY surround speakers — placed at ≈110-120°
    // (behind the listener). In 7.1+, when Lrs/Rrs are added at the rear,
    // Ls/Rs shift forward to ≈90° (the sides). We handle this dynamically below.
    var speaker_data = array<vec3<f32>, 12>(
        vec3<f32>(-2.5, 2.0, 0.0),    // 0: L   — Front left              (≈30°)
        vec3<f32>(2.5, 2.0, 0.0),     // 1: R   — Front right             (≈30°)
        vec3<f32>(0.0, 2.5, 0.0),     // 2: C   — Center                  (0°)
        vec3<f32>(0.0, 0.5, 0.0),     // 3: LFE — Subwoofer               (room pressure)
        vec3<f32>(-2.2, -0.5, 0.0),   // 4: Ls  — Surround left (7.1: side ≈90°)
        vec3<f32>(2.2, -0.5, 0.0),    // 5: Rs  — Surround right (7.1: side ≈90°)
        vec3<f32>(-1.2, -1.2, 0.0),   // 6: Lrs — Surround back left      (≈140°)
        vec3<f32>(1.2, -1.2, 0.0),    // 7: Rrs — Surround back right     (≈140°)
        vec3<f32>(-2.2, 1.5, 0.7),    // 8: Ltf — Front left height       (above front)
        vec3<f32>(2.2, 1.5, 0.7),     // 9: Rtf — Front right height      (above front)
        vec3<f32>(-1.2, -1.2, 0.7),   // 10: Ltr — Back left height       (above Lrs)
        vec3<f32>(1.2, -1.2, 0.7)     // 11: Rtr — Back right height      (above Rrs)
    );

    // In 5.1 (≤6 channels), Ls/Rs are the rear speakers (≈110-120°).
    // Move them to the back position since there are no separate Lrs/Rrs.
    if render_channels <= 6u {
        speaker_data[4] = vec3<f32>(-1.2, -1.2, 0.0);   // Ls → rear position
        speaker_data[5] = vec3<f32>(1.2, -1.2, 0.0);    // Rs → rear position
    }

    // Always render all 12 speaker positions; inactive channels shown as dull grey
    for (var i = 0u; i < 12u; i = i + 1u) {
        let room_xy = vec2<f32>(speaker_data[i].x, speaker_data[i].y);
        var height = speaker_data[i].z;
        let world_xy = rotate2d(room_xy, room_angle);

        let is_active = i < render_channels;
        let is_lfe = i == 3u;

        // VU level — 0 for inactive channels, subdued for LFE dot
        var vu = 0.0;
        if is_active { vu = get_spatial_vu(i); }
        if is_lfe { vu = vu * 0.3; } // LFE dot stays subtle; pressure dome handles impact

        // Physical bounce (active only)
        height = height + (vu * 0.08);

        let p3_base = vec3<f32>(world_xy.x, world_xy.y, 0.0);
        let p3_speaker = vec3<f32>(world_xy.x, world_xy.y, height);

        let proj_b = project_3d(p3_base, ro, cu, cv, cw);
        let proj_s = project_3d(p3_speaker, ro, cu, cv, cw);

        let proj_base_2d = proj_b.xy;
        let proj_speaker_2d = proj_s.xy;
        let depth = proj_s.z;

        // Color: active = channel color, inactive = dull grey
        var base_color = vec3<f32>(0.15, 0.15, 0.18);
        if is_active {
            if i <= 2u { base_color = vec3<f32>(0.0, 0.8, 1.0); }         // Front — cyan
            else if is_lfe { base_color = vec3<f32>(1.0, 0.2, 0.4); }     // LFE — pink
            else if i <= 7u { base_color = vec3<f32>(0.0, 0.8, 1.0); }    // Surrounds — cyan
            else { base_color = vec3<f32>(1.0, 0.7, 0.1); }               // Heights — gold
        }

        // Depth cue
        let depth_fade = smoothstep(6.0, 1.5, depth);
        let ch_color = base_color * (0.3 + 0.7 * depth_fade);

        // Size = audio importance. Front L/C/R are the primary sound source and must
        // dominate visually despite being furthest from camera. Factor must overcome
        // perspective distance reduction (front scale ≈ 0.44 vs rear ≈ 1.5).
        var size_factor = 1.0;
        if i <= 2u { size_factor = 5.0; }           // Front L/C/R — main PA stacks
        else if is_lfe { size_factor = 0.8; }       // LFE — small dot (room pressure)
        else if i <= 5u { size_factor = 2.0; }      // Side surrounds — medium
        else if i <= 7u { size_factor = 1.0; }      // Rear surrounds — natural perspective
        else { size_factor = 0.8; }                  // Heights — smallest

        // Floor ripples (active non-LFE only)
        if rd.z < 0.0 && is_active && !is_lfe {
            let time_offset = audio.time * 8.0;
            let dist_room = length(floor_xy - room_xy);
            let ripple = sin(dist_room * 20.0 - time_offset) * 0.5 + 0.5;
            let ripple_radius = 0.8 + (size_factor - 1.0) * 0.5;
            let ripple_mask = smoothstep(ripple_radius, 0.0, dist_room) * vu * 0.4;
            let grid_fade = 1.0 - smoothstep(2.0, 7.0, length(floor_xy));
            color = color + ch_color * ripple * ripple_mask * grid_fade;
        }

        // Perspective scaling — clamped to prevent extreme size distortion
        let raw_scale = 2.0 / max(depth, 0.1);
        let scale = clamp(raw_scale, 0.3, 1.5);

        // Floor anchor for elevated speakers
        if height > 0.0 {
            let dist_base = length(p - proj_base_2d);
            let anchor_s = size_factor * 0.5 + 0.5;
            let base_core = smoothstep(0.03 * scale * anchor_s, 0.0, dist_base) * 0.4;
            let base_ring = smoothstep(0.005 * scale, 0.0, abs(dist_base - 0.04 * scale * anchor_s)) * 0.4;
            color = color + ch_color * (base_core + base_ring);
        }

        // --- Speaker Orb ---
        let dist = length(p - proj_speaker_2d);
        let noise_phase = hash_f(f32(i) * 17.3 + audio.time * 3.0) * 0.004 * vu * size_factor;
        let organic_dist = dist + noise_phase;

        let dot_size = (0.03 + vu * 0.04) * scale * size_factor;

        // Inactive: dim static orb. Active: full bright with effects.
        let active_mult = select(0.4, 1.0, is_active);

        let core_hot = smoothstep(dot_size * 0.3, 0.0, dist) * 1.2 * active_mult;
        let core = smoothstep(dot_size, dot_size * 0.5, dist);

        let rim_inner = smoothstep(dot_size * 0.6, dot_size * 0.85, dist);
        let rim_outer = smoothstep(dot_size * 1.1, dot_size * 0.85, dist);
        let rim = rim_inner * rim_outer * max(vu, 0.15) * 1.5 * active_mult;

        let glow = smoothstep(dot_size * 3.0, 0.0, organic_dist) * max(vu, 0.08) * 0.5;

        let shock_radius = dot_size + (0.01 + vu * 0.05) * scale * size_factor;
        let shockwave = smoothstep(0.006 * scale, 0.0, abs(dist - shock_radius)) * (vu * vu);

        // Composite
        color = color + vec3<f32>(1.0) * core_hot;
        color = color + ch_color * core * (0.8 + 1.2 * active_mult);
        color = color + ch_color * rim;
        color = color + ch_color * glow;
        if is_active { color = color + ch_color * shockwave; }

        // --- Debug label below speaker ---
        // Labels: L, R, C, LFE, Ls, Rs, Lrs, Rrs, Ltf, Rtf, Ltr, Rtr
        // Encoded as arrays of glyph indices (10=L,11=R,12=C,13=S,14=F,15=E,16=space)
        var lbl = array<u32, 3>(16u, 16u, 16u); // default: spaces
        var lbl_len = 1u;
        switch i {
            case 0u  { lbl[0]=10u; }                                // L
            case 1u  { lbl[0]=11u; }                                // R
            case 2u  { lbl[0]=12u; }                                // C
            case 3u  { lbl[0]=10u; lbl[1]=14u; lbl[2]=15u; lbl_len=3u; } // LFE
            case 4u  { lbl[0]=10u; lbl[1]=13u; lbl_len=2u; }       // LS
            case 5u  { lbl[0]=11u; lbl[1]=13u; lbl_len=2u; }       // RS
            case 6u  { lbl[0]=10u; lbl[1]=11u; lbl[2]=13u; lbl_len=3u; } // LRS (Lrs)
            case 7u  { lbl[0]=11u; lbl[1]=11u; lbl[2]=13u; lbl_len=3u; } // RRS (Rrs)
            case 8u  { lbl[0]=10u; lbl[1]=14u; lbl_len=2u; }       // LF (Ltf)
            case 9u  { lbl[0]=11u; lbl[1]=14u; lbl_len=2u; }       // RF (Rtf)
            case 10u { lbl[0]=10u; lbl[1]=11u; lbl_len=2u; }       // LR (Ltr)
            case 11u { lbl[0]=11u; lbl[1]=11u; lbl_len=2u; }       // RR (Rtr)
            default  { }
        }

        let px_size = 0.004 * scale;
        let char_w = px_size * 4.0; // 3px + 1px gap
        let total_w = char_w * f32(lbl_len) - px_size;
        let label_origin = proj_speaker_2d + vec2<f32>(-total_w * 0.5, dot_size + px_size * 3.0);

        var label_alpha = 0.0;
        for (var c = 0u; c < lbl_len; c = c + 1u) {
            let char_origin = label_origin + vec2<f32>(char_w * f32(c), 0.0);
            label_alpha = max(label_alpha, draw_label_char(lbl[c], p, char_origin, px_size));
        }
        let label_color = select(vec3<f32>(0.35, 0.35, 0.4), ch_color, is_active);
        color = color + label_color * label_alpha * 0.8;
    }

    // --- LFE room edge glow (simulates wall/ceiling vibration bleed) ---
    if render_channels > 3u {
        let lfe_color = vec3<f32>(1.0, 0.2, 0.4);
        let edge_x = smoothstep(0.6, 1.0, abs(uv.x));
        let edge_y = smoothstep(0.7, 1.0, abs(uv.y));
        let edge_glow = max(edge_x, edge_y) * lfe_vu * lfe_vu;
        color = color + lfe_color * edge_glow * 0.15;
    }

    // --- Atmospheric edge fog, blown away by volume ---
    let fog_color = vec3<f32>(0.025, 0.02, 0.035);
    let fog_clear = smoothstep(0.0, 0.8, total_volume);
    let radial_dist = length(uv);
    let fog_edge = smoothstep(0.4 + fog_clear * 0.6, 1.8, radial_dist);
    let fog_turb = sin(uv.x * 3.0 + audio.time * 0.5) * cos(uv.y * 2.0 + audio.time * 0.3) * 0.08;
    let fog_density = clamp(fog_edge + fog_turb, 0.0, 1.0) * (1.0 - fog_clear * 0.7);
    color = mix(color, fog_color, fog_density);

    // Vignette
    let vignette = 1.0 - smoothstep(0.5, 2.0, length(uv));
    color = color * vignette;

    // ACES Narkowicz tonemapping (matches other visualizers)
    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));

    return vec4<f32>(final_col, 1.0);
}
