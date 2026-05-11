// =====================================================
// Neon Room Visualizer — Multi-Channel Edition
// Raymarched architectural space with N neon frames
// scaling dynamically with audio channel count.
// =====================================================

const MAX_FRAMES: u32 = 5u;
const FRAME_HALF_HEIGHT: f32 = 2.5;
const FRAME_THICKNESS: f32 = 0.08;
const FRAME_Y: f32 = 1.5;
const FRAME_Z: f32 = 2.0;
const MAX_AMBIENTS: u32 = 7u;

// Per-invocation frame state (set once in fs_main, read everywhere)
var<private> g_frame_pos: array<vec3<f32>, 5>;
var<private> g_frame_light: array<vec3<f32>, 5>;
var<private> g_frame_w: f32;
var<private> g_num_frames: u32;

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
    display_order: array<vec4<u32>, 4>,
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

// --- Utility ---

fn get_vu(i: u32) -> f32 {
    let n = max(1u, audio.num_channels);
    let idx = min(i, n - 1u);
    let v = audio.channels[idx / 4u];
    let c = idx % 4u;
    if (c == 0u) { return v.x; } else if (c == 1u) { return v.y; }
    else if (c == 2u) { return v.z; } else { return v.w; }
}

fn hash(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }

fn noise(x: vec3<f32>) -> f32 {
    let p = floor(x);
    let f = fract(x);
    let f2 = f * f * (3.0 - 2.0 * f);
    let n = p.x + p.y * 57.0 + 113.0 * p.z;
    return mix(
        mix(mix(hash(n + 0.0), hash(n + 1.0), f2.x),
            mix(hash(n + 57.0), hash(n + 58.0), f2.x), f2.y),
        mix(mix(hash(n + 113.0), hash(n + 114.0), f2.x),
            mix(hash(n + 170.0), hash(n + 171.0), f2.x), f2.y), f2.z
    );
}

fn fbm(p_in: vec3<f32>) -> f32 {
    var p = p_in;
    var f = 0.0;
    var amp = 0.5;
    for(var i=0; i<4; i++) {  // 4 octaves (was 5 — negligible visual diff in dark smoke)
        f += amp * noise(p);
        p *= 2.1;
        amp *= 0.5;
    }
    return f;
}

// --- Channel color palette (up to 12 channels, neon-saturated) ---

fn channel_color(i: u32) -> vec3<f32> {
    switch i {
        case 0u  { return vec3<f32>(1.0, 0.05, 0.15); } // L   — Red
        case 1u  { return vec3<f32>(0.05, 0.4, 1.0);  } // R   — Blue
        case 2u  { return vec3<f32>(1.0, 0.85, 0.6);  } // C   — Warm White
        case 3u  { return vec3<f32>(0.9, 0.05, 0.25); } // LFE — Deep Magenta
        case 4u  { return vec3<f32>(0.0, 0.9, 0.65);  } // Ls  — Teal
        case 5u  { return vec3<f32>(0.65, 0.1, 1.0);  } // Rs  — Violet
        case 6u  { return vec3<f32>(1.0, 0.5, 0.05);  } // Lrs — Orange
        case 7u  { return vec3<f32>(1.0, 0.8, 0.1);   } // Rrs — Gold
        case 8u  { return vec3<f32>(1.0, 0.3, 0.55);  } // Ltf — Pink
        case 9u  { return vec3<f32>(0.55, 0.4, 1.0);  } // Rtf — Lavender
        case 10u { return vec3<f32>(0.45, 1.0, 0.2);  } // Ltr — Lime
        default  { return vec3<f32>(1.0, 0.6, 0.15);  } // Rtr — Amber
    }
}

// --- SDF Primitives ---

fn sdBox(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn sdFrame(p: vec3<f32>, w: f32, h: f32, t: f32) -> f32 {
    let b1 = sdBox(p - vec3<f32>(0.0, h, 0.0), vec3<f32>(w, t, t));
    let b2 = sdBox(p - vec3<f32>(0.0, -h, 0.0), vec3<f32>(w, t, t));
    let b3 = sdBox(p - vec3<f32>(-w, 0.0, 0.0), vec3<f32>(t, h+t, t));
    let b4 = sdBox(p - vec3<f32>(w, 0.0, 0.0), vec3<f32>(t, h+t, t));
    return min(min(b1, b2), min(b3, b4));
}

// Returns (distance, material_id).  mat_id: -1=floor, 0=wall/ceil, 1..N=frame index (1-based)
fn map_scene(p: vec3<f32>) -> vec2<f32> {
    let d_floor = p.y + 1.0;
    let d_wall = 10.0 - p.z;
    let d_ceil = 5.0 - p.y;
    let d_left = p.x + 8.0;
    let d_right = 8.0 - p.x;

    var d_room = min(d_floor, d_wall);
    d_room = min(d_room, min(d_ceil, min(d_left, d_right)));

    var best_d = d_room;
    var best_mat = 0.0;
    if (d_room == d_floor) { best_mat = -1.0; }

    // Test all active frames
    for (var i = 0u; i < g_num_frames; i++) {
        let d_f = sdFrame(p - g_frame_pos[i], g_frame_w, FRAME_HALF_HEIGHT, FRAME_THICKNESS);
        if (d_f < best_d) {
            best_d = d_f;
            best_mat = f32(i) + 1.0; // 1-based frame index
        }
    }

    return vec2<f32>(best_d, best_mat);
}

fn calc_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.01, 0.0);
    return normalize(vec3<f32>(
        map_scene(p + e.xyy).x - map_scene(p - e.xyy).x,
        map_scene(p + e.yxy).x - map_scene(p - e.yxy).x,
        map_scene(p + e.yyx).x - map_scene(p - e.yyx).x
    ));
}

fn get_smoke_density(p: vec3<f32>, time: f32, audio_activity: f32) -> f32 {
    let center = vec3<f32>(0.0, -0.5, FRAME_Z);
    let dist = length(p - center);

    // Early-out: skip FBM outside the smoke bounding volume
    if (dist > 6.0 || p.y > 3.0) { return 0.0; }

    var mask = smoothstep(6.0, 0.0, dist);
    mask *= smoothstep(3.0, -1.0, p.y);

    if (mask < 0.01) { return 0.0; }

    let np = p * 1.5 - vec3<f32>(0.0, time * 0.2, time * 0.1);
    let n = fbm(np);

    // Higher threshold (0.45) to create distinct wispy chunks
    var dens = (n - 0.45) * (4.0 + audio_activity * 5.0);
    return max(dens, 0.0) * mask;
}

// --- Scene Setup (called once per fragment) ---

fn setup_scene() {
    let num_ch = max(audio.num_channels, 1u);

    // Determine frame count and channel mapping
    // Frame order is always spatial L-to-R for visual consistency
    if (num_ch <= 2u) {
        // Stereo: 2 frames (L, R)
        g_num_frames = min(num_ch, 2u);
        g_frame_light[0] = channel_color(0u) * (2.0 + clamp(get_vu(0u), 0.0, 1.0) * 6.0);
        if (num_ch > 1u) {
            g_frame_light[1] = channel_color(1u) * (2.0 + clamp(get_vu(1u), 0.0, 1.0) * 6.0);
        }
    } else if (num_ch <= 6u) {
        // 3–6 channels (up to 5.1): 3 frames → L, C, R
        g_num_frames = 3u;
        // Ordered left-to-right: L, C, R
        g_frame_light[0] = channel_color(0u) * (2.0 + clamp(get_vu(0u), 0.0, 1.0) * 6.0);
        g_frame_light[1] = channel_color(2u) * (2.0 + clamp(get_vu(2u), 0.0, 1.0) * 6.0);
        g_frame_light[2] = channel_color(1u) * (2.0 + clamp(get_vu(1u), 0.0, 1.0) * 6.0);
    } else {
        // 7+ channels (7.1+): 5 frames → L, Ls, C, Rs, R
        g_num_frames = 5u;
        g_frame_light[0] = channel_color(0u) * (2.0 + clamp(get_vu(0u), 0.0, 1.0) * 6.0);
        g_frame_light[1] = channel_color(4u) * (2.0 + clamp(get_vu(4u), 0.0, 1.0) * 6.0);
        g_frame_light[2] = channel_color(2u) * (2.0 + clamp(get_vu(2u), 0.0, 1.0) * 6.0);
        g_frame_light[3] = channel_color(5u) * (2.0 + clamp(get_vu(5u), 0.0, 1.0) * 6.0);
        g_frame_light[4] = channel_color(1u) * (2.0 + clamp(get_vu(1u), 0.0, 1.0) * 6.0);
    }

    // Distribute frames evenly across the wall
    var spread = 2.0;
    g_frame_w = 0.6;
    if (g_num_frames == 3u) { spread = 3.5; g_frame_w = 0.55; }
    else if (g_num_frames >= 5u) { spread = 4.5; g_frame_w = 0.42; }

    for (var i = 0u; i < g_num_frames; i++) {
        var frac = 0.5;
        if (g_num_frames > 1u) {
            frac = f32(i) / f32(g_num_frames - 1u);
        }
        g_frame_pos[i] = vec3<f32>(mix(-spread, spread, frac), FRAME_Y, FRAME_Z);
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;

    var aspect = 1.7777;
    let dy = abs(dpdy(in.uv.y));
    let dx = abs(dpdx(in.uv.x));
    if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }

    let p = vec2<f32>(uv.x * aspect, -uv.y);

    let ro = vec3<f32>(0.0, 0.0, -4.0);
    let look_at = vec3<f32>(sin(audio.time * 0.2) * 0.1, 0.5 + cos(audio.time * 0.3) * 0.05, FRAME_Z);

    let cw = normalize(look_at - ro);
    let cu = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), cw));
    let cv = normalize(cross(cw, cu));

    let rd = normalize(p.x * cu + p.y * cv + 1.2 * cw);

    // --- Setup scene geometry based on channel count ---
    setup_scene();

    let num_ch = max(audio.num_channels, 1u);

    // --- Ambient wash lights (channels not assigned to frames) ---
    // These have no SDF geometry; they only contribute diffuse + smoke lighting.
    var amb_pos: array<vec3<f32>, 7>;
    var amb_light: array<vec3<f32>, 7>;
    var num_amb = 0u;

    // LFE (ch 3) — floor-centered bass pulse (always present for 4+ ch)
    if (num_ch > 3u) {
        let lfe_vu = clamp(get_vu(3u), 0.0, 1.0);
        amb_pos[num_amb] = vec3<f32>(0.0, -0.9, FRAME_Z);
        amb_light[num_amb] = channel_color(3u) * lfe_vu * 8.0;
        num_amb += 1u;
    }

    // For 5.1 (<=6 ch): Ls/Rs are ambient washes from side walls
    if (num_ch >= 5u && num_ch <= 6u) {
        amb_pos[num_amb] = vec3<f32>(-7.0, 1.5, 0.0);
        amb_light[num_amb] = channel_color(4u) * (1.0 + clamp(get_vu(4u), 0.0, 1.0) * 5.0);
        num_amb += 1u;
        amb_pos[num_amb] = vec3<f32>(7.0, 1.5, 0.0);
        amb_light[num_amb] = channel_color(5u) * (1.0 + clamp(get_vu(5u), 0.0, 1.0) * 5.0);
        num_amb += 1u;
    }

    // For 7.1+ (>=7 ch): Lrs/Rrs are ambient from rear corners
    if (num_ch >= 7u) {
        amb_pos[num_amb] = vec3<f32>(-5.0, 1.5, -3.0);
        amb_light[num_amb] = channel_color(6u) * (1.0 + clamp(get_vu(6u), 0.0, 1.0) * 5.0);
        num_amb += 1u;
        if (num_ch >= 8u) {
            amb_pos[num_amb] = vec3<f32>(5.0, 1.5, -3.0);
            amb_light[num_amb] = channel_color(7u) * (1.0 + clamp(get_vu(7u), 0.0, 1.0) * 5.0);
            num_amb += 1u;
        }
    }

    // Height channels (ch 8–11) — ceiling washes
    if (num_ch >= 9u) {
        amb_pos[num_amb] = vec3<f32>(-3.0, 4.5, FRAME_Z);
        amb_light[num_amb] = channel_color(8u) * (0.5 + clamp(get_vu(8u), 0.0, 1.0) * 4.0);
        num_amb += 1u;
    }
    if (num_ch >= 10u) {
        amb_pos[num_amb] = vec3<f32>(3.0, 4.5, FRAME_Z);
        amb_light[num_amb] = channel_color(9u) * (0.5 + clamp(get_vu(9u), 0.0, 1.0) * 4.0);
        num_amb += 1u;
    }
    // Cap at MAX_AMBIENTS — channels 10,11 fold into existing washes

    // Total audio activity for smoke density modulation
    var act = 0.0;
    for (var i = 0u; i < min(num_ch, 8u); i++) {
        act += clamp(get_vu(i), 0.0, 1.0);
    }
    act /= f32(max(num_ch, 1u));

    // --- Raymarching ---
    var t = 0.0;
    var T = 1.0;  // smoke transmittance
    var color = vec3<f32>(0.0);
    var neon_glow = vec3<f32>(0.0);

    for(var i=0; i<100; i++) {
        let p_hit = ro + rd * t;
        let res = map_scene(p_hit);
        let d = res.x;

        // Accumulate bloom from all neon frames (smooth, no Voronoi seams)
        for (var f = 0u; f < g_num_frames; f++) {
            let d_f = sdFrame(p_hit - g_frame_pos[f], g_frame_w, FRAME_HALF_HEIGHT, FRAME_THICKNESS);
            neon_glow += g_frame_light[f] * 0.03 / (1.0 + d_f * d_f * 20.0);
        }

        if (d < 0.01) {
            var surf_col = vec3<f32>(0.0);
            let mat_id = i32(res.y);

            if (mat_id >= 1) {
                // Hit a neon frame — solid white core + frame color
                let fi = u32(mat_id - 1);
                surf_col = vec3<f32>(3.0) + g_frame_light[fi] * 1.5;
            } else {
                // Hit room geometry (floor or wall/ceiling)
                let n = calc_normal(p_hit);

                // Accumulate diffuse lighting from all frames
                var room_light = vec3<f32>(0.002);
                for (var f = 0u; f < g_num_frames; f++) {
                    let dir = g_frame_pos[f] - p_hit;
                    let dist = length(dir);
                    let diff = max(dot(n, dir/dist), 0.0) / (dist * dist * 1.5 + 1.0);
                    room_light += g_frame_light[f] * diff * 0.02;
                }

                // Diffuse from ambient wash lights
                for (var a = 0u; a < num_amb; a++) {
                    let dir = amb_pos[a] - p_hit;
                    let dist = length(dir);
                    let diff = max(dot(n, dir/dist), 0.0) / (dist * dist * 2.0 + 1.0);
                    room_light += amb_light[a] * diff * 0.015;
                }

                surf_col = room_light;

                // Floor reflections
                if (mat_id == -1) {
                    let r_rd = reflect(rd, n);

                    // Subtle bump distortion
                    let bump = (noise(vec3<f32>(p_hit.x * 6.0, 0.0, p_hit.z * 6.0)) - 0.5) * 0.02;
                    let r_rd_b = normalize(r_rd + vec3<f32>(bump, 0.0, bump));

                    var ref_col = vec3<f32>(0.0);
                    var rt = 0.05;

                    // Track minimum distance to each frame for smooth reflection bleed
                    var min_d: array<f32, 5>;
                    for (var f = 0u; f < g_num_frames; f++) { min_d[f] = 100.0; }

                    for(var j=0; j<15; j++) {
                        let rp = p_hit + r_rd_b * rt;
                        var d_min_all = 100.0;

                        for (var f = 0u; f < g_num_frames; f++) {
                            let d_f = sdFrame(rp - g_frame_pos[f], g_frame_w, FRAME_HALF_HEIGHT, FRAME_THICKNESS);
                            min_d[f] = min(min_d[f], d_f);
                            d_min_all = min(d_min_all, d_f);
                        }

                        if (d_min_all < 0.01) { break; }
                        rt += d_min_all;
                        if (rt > 12.0 || rp.y > 4.0) { break; }
                    }

                    // Smooth glowing reflection bleed from each frame
                    for (var f = 0u; f < g_num_frames; f++) {
                        ref_col += g_frame_light[f] * 0.8 / (1.0 + min_d[f] * min_d[f] * 15.0);
                        if (min_d[f] < 0.01) {
                            ref_col += vec3<f32>(2.0) + g_frame_light[f];
                        }
                    }

                    let fresnel = pow(1.0 - max(dot(n, -rd), 0.0), 3.0);
                    let reflectivity = mix(0.15, 0.8, fresnel);
                    surf_col += ref_col * reflectivity;
                }
            }

            color += T * surf_col;
            break;
        }

        // Volumetric smoke — lit by ALL light sources (frames + ambients)
        let smoke_bounding = length(p_hit - vec3<f32>(0.0, -0.5, FRAME_Z)) - 5.5;

        if (smoke_bounding < 0.0 && T > 0.01) {
            let dens = get_smoke_density(p_hit, audio.time, act);
            if (dens > 0.0) {
                // Accumulate lighting from all frames
                var smoke_light = vec3<f32>(0.0);
                for (var f = 0u; f < g_num_frames; f++) {
                    let d_l = length(p_hit - g_frame_pos[f]);
                    smoke_light += g_frame_light[f] * exp(-d_l * 0.7);
                }
                // Plus ambient wash lights
                for (var a = 0u; a < num_amb; a++) {
                    let d_l = length(p_hit - amb_pos[a]);
                    smoke_light += amb_light[a] * exp(-d_l * 0.7);
                }
                smoke_light *= 2.5;

                let step_len = 0.1;
                let alpha = 1.0 - exp(-dens * step_len * 4.0);

                color += T * alpha * smoke_light;
                T *= (1.0 - alpha);

                if (T < 0.01) { break; }
            }
        }

        var step_size = d;
        if (smoke_bounding < 0.0 && T > 0.01) {
            step_size = min(step_size, 0.1);
        } else {
            step_size = min(step_size, smoke_bounding + 0.05);
        }

        t += max(step_size, 0.02);
        if (t > 20.0) { break; }
    }

    color += neon_glow * T;

    // LFE floor pulse — bass-reactive glow centered under the frames
    if (num_ch > 3u) {
        let lfe_vu = clamp(get_vu(3u), 0.0, 1.0);
        let lfe_col = channel_color(3u);
        // Project the camera ray to the floor plane
        if (rd.y < 0.0) {
            let t_floor = -(ro.y + 1.0) / rd.y;
            let floor_hit = ro + rd * t_floor;
            let dist_center = length(floor_hit.xz - vec2<f32>(0.0, FRAME_Z));
            let pulse = smoothstep(3.0, 0.0, dist_center) * lfe_vu * lfe_vu;
            let pulse_ring = smoothstep(0.08, 0.0, abs(dist_center - 1.5 - lfe_vu * 2.0)) * lfe_vu;
            color += lfe_col * (pulse * 0.6 + pulse_ring * 0.4) * T;
        }
    }

    let vr = length(uv);
    color *= smoothstep(2.5, 0.5, vr);

    // Narkowicz ACES fitted tonemap (sRGB gamma applied by WGPU surface)
    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));

    return vec4<f32>(final_col, 1.0);
}
