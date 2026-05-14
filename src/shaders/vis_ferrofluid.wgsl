// =====================================================
// Chrome Ferrofluid Visualizer
// Raymarched liquid metal puddle with audio-reactive spikes
// =====================================================

// --- Tuning Constants ---
const PUDDLE_RADIUS: f32 = 4.0;
const STEP_SCALE: f32 = 0.45;       // Lipschitz correction for pow-8 lobes + smax(k=0.3)
const MAX_MARCH_STEPS: i32 = 120;   // Safe ceiling with corrected step scale
const HIT_THRESHOLD: f32 = 0.005;
const NORMAL_EPS: f32 = 0.015;      // Increased epsilon for smoother, anti-aliased normals
const HDR_WHITE: f32 = 5.0;         // Blown-out white that tonemaps to ~1.0
const SPEC_POWER: f32 = 24.0;
const MAX_MARCH_DIST: f32 = 30.0;

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

// --- Utility Functions ---

// Bounds-clamped channel VU accessor (consistent with vis_neon.wgsl)
fn get_vu(i: u32) -> f32 {
    let n = max(1u, audio.num_channels);
    let idx = min(i, n - 1u);
    let v = audio.channels[idx / 4u];
    let c = idx % 4u;
    if (c == 0u) { return v.x; } else if (c == 1u) { return v.y; }
    else if (c == 2u) { return v.z; } else { return v.w; }
}

fn hash(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }

fn hash3_smooth(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec3<f32>(3.0) - 2.0 * f);
    
    let n = i.x + i.y * 57.0 + i.z * 113.0;
    
    let a = hash(n + 0.0);
    let b = hash(n + 1.0);
    let c = hash(n + 57.0);
    let d = hash(n + 58.0);
    let e = hash(n + 113.0);
    let f_val = hash(n + 114.0);
    let g = hash(n + 170.0);
    let h_val = hash(n + 171.0);
    
    return mix(
        mix(mix(a, b, u.x), mix(c, d, u.x), u.y),
        mix(mix(e, f_val, u.x), mix(g, h_val, u.x), u.y),
        u.z
    );
}

fn smax(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (a - b) / k, 0.0, 1.0);
    return mix(b, a, h) + k * h * (1.0 - h);
}

// --- Speaker Layout (up to 7.1.4) ---
// Note: This shader maps channels[] (instrument/track data) to speaker positions.
// For surround content where spatial_channels[] carries the speaker mix,
// swap get_vu(i) for a spatial accessor if needed.
fn get_speaker_dir(i: u32) -> vec3<f32> {
    switch i {
        case 0u  { return vec3<f32>(-0.5, 0.0, -0.866); } // L
        case 1u  { return vec3<f32>( 0.5, 0.0, -0.866); } // R
        case 2u  { return vec3<f32>( 0.0, 0.0, -1.0);   } // C
        case 3u  { return vec3<f32>( 0.0, 0.0,  0.0);   } // LFE (center blob)
        case 4u  { return vec3<f32>(-0.94, 0.0, 0.34);  } // Ls
        case 5u  { return vec3<f32>( 0.94, 0.0, 0.34);  } // Rs
        case 6u  { return vec3<f32>(-0.5, 0.0,  0.866); } // Lrs
        case 7u  { return vec3<f32>( 0.5, 0.0,  0.866); } // Rrs
        case 8u  { return vec3<f32>(-0.7, 0.0, -0.7);   } // Ltf
        case 9u  { return vec3<f32>( 0.7, 0.0, -0.7);   } // Rtf
        case 10u { return vec3<f32>(-0.7, 0.0,  0.7);   } // Ltr
        default  { return vec3<f32>( 0.7, 0.0,  0.7);   } // Rtr
    }
}

// --- SDF: Distance-only (used by calcNormal — no glow computation) ---

fn map_dist(p: vec3<f32>) -> f32 {
    let dist_xz = length(p.xz);

    // Base infinite plane thickness
    var fluid_h = 0.0;

    let num_ch = min(audio.num_channels, 12u);
    var total_displacement = 0.0;

    // Normalized xz for angle alignment
    let p_xz_norm = p.xz / max(dist_xz, 0.0001);

    for (var i = 0u; i < num_ch; i++) {
        let vu = clamp(get_vu(i), 0.0, 1.0);

        var alignment = 1.0;
        var spike_pos_r = 1.5;

        if i == 3u { // LFE channel — center blob
            alignment = 1.0;
            spike_pos_r = 0.0;
        } else {
            let dir2d = normalize(get_speaker_dir(i).xz);
            alignment = max(0.0, dot(p_xz_norm, dir2d));
            spike_pos_r = 1.5;
        }
        
        // Removed the continue statement here because skipping smax introduces radial tears.

        let dist_to_spike = abs(dist_xz - spike_pos_r);
        let spatial_falloff = exp(-dist_to_spike * 3.0);

        // Soften spike shape (pow 8) to keep SDF slopes within Lipschitz bound
        var lobe = pow(alignment, 8.0) * vu * 1.5 * spatial_falloff;
        
        // Attenuate directional lobes at the center to prevent radial crease artifacts
        if i != 3u {
            lobe *= smoothstep(0.1, 0.5, dist_xz);
        }
        
        total_displacement = smax(total_displacement, lobe, 0.3);
    }

    // Subtle ripples from spectrum bass
    let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x, 0.0, 2.0);
    let ripple = sin(dist_xz * 12.0 - audio.time * 8.0) * 0.015 * bass * smoothstep(PUDDLE_RADIUS, 0.0, dist_xz);

    // Organic surface perturbation (smooth magnetic domain noise)
    let noise_p = p * 4.0 + vec3<f32>(audio.time * 0.5, 0.0, audio.time * 0.3);
    let surface_noise = (hash3_smooth(noise_p) - 0.5) * 0.05;

    fluid_h += total_displacement + ripple + surface_noise;

    let d = p.y + 0.5 - fluid_h;

    // Lipschitz-corrected step size
    return d * STEP_SCALE;
}

// --- SDF: Full map with glow (used only in march loop) ---

struct MapData {
    d: f32,
    mat_id: i32,
    glow: vec3<f32>,
}

fn map(p: vec3<f32>) -> MapData {
    let dist_xz = length(p.xz);

    // Base infinite plane thickness
    var fluid_h = 0.0;
    var glow = vec3<f32>(0.0);

    let num_ch = min(audio.num_channels, 12u);
    var total_displacement = 0.0;

    let p_xz_norm = p.xz / max(dist_xz, 0.0001);

    for (var i = 0u; i < num_ch; i++) {
        let vu = clamp(get_vu(i), 0.0, 1.0);

        var alignment = 1.0;
        var spike_pos_r = 1.5;

        if i == 3u {
            alignment = 1.0;
            spike_pos_r = 0.0;
        } else {
            let dir2d = normalize(get_speaker_dir(i).xz);
            alignment = max(0.0, dot(p_xz_norm, dir2d));
            spike_pos_r = 1.5;
        }
        
        // Removed the continue statement here because skipping smax introduces radial tears.

        let dist_to_spike = abs(dist_xz - spike_pos_r);
        let spatial_falloff = exp(-dist_to_spike * 3.0);

        var lobe = pow(alignment, 8.0) * vu * 1.5 * spatial_falloff;
        
        // Attenuate directional lobes at the center to prevent radial crease artifacts
        if i != 3u {
            lobe *= smoothstep(0.1, 0.5, dist_xz);
        }
        
        total_displacement = smax(total_displacement, lobe, 0.3);

        // Inner glow for active spikes
        if lobe > 0.1 {
            // Warm tones for front channels, cool for surround, red for LFE
            var ch_color = vec3<f32>(0.2, 0.6, 1.0);
            if i < 3u { ch_color = vec3<f32>(1.0, 0.4, 0.1); }
            if i == 3u { ch_color = vec3<f32>(1.0, 0.1, 0.2); }
            glow += ch_color * pow(lobe, 2.0) * 2.0;
        }
    }

    // Subtle ripples from spectrum bass
    let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x, 0.0, 2.0);
    let ripple = sin(dist_xz * 12.0 - audio.time * 8.0) * 0.015 * bass * smoothstep(PUDDLE_RADIUS, 0.0, dist_xz);

    // Organic surface perturbation (smooth magnetic domain noise)
    let noise_p = p * 4.0 + vec3<f32>(audio.time * 0.5, 0.0, audio.time * 0.3);
    let surface_noise = (hash3_smooth(noise_p) - 0.5) * 0.05;

    fluid_h += total_displacement + ripple + surface_noise;

    let d = p.y + 0.5 - fluid_h;

    return MapData(d * STEP_SCALE, 1, glow);
}

// 4-sample tetrahedron normal
fn calcNormal(p: vec3<f32>) -> vec3<f32> {
    let h = NORMAL_EPS;
    let k = vec2<f32>(1.0, -1.0);
    return normalize(
        k.xyy * map_dist(p + k.xyy * h) + 
        k.yyx * map_dist(p + k.yyx * h) + 
        k.yxy * map_dist(p + k.yxy * h) + 
        k.xxx * map_dist(p + k.xxx * h)
    );
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;

    // Fix aspect ratio with safe division guard, and invert Y
    var aspect = 1.7777;
    let dy = abs(dpdy(in.uv.y));
    let dx = abs(dpdx(in.uv.x));
    if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }
    let p = vec2<f32>(uv.x * aspect, -uv.y);

    // Camera looking slightly down at the puddle
    let ro = vec3<f32>(0.0, 1.25, 2.0);
    let cam_target = vec3<f32>(0.0, -0.25, 0.0);

    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = normalize(cross(uu, ww));

    let fov = 1.0;
    let rd = normalize(p.x * uu + p.y * vv + fov * ww);

    var col = vec3<f32>(0.0);
    var glow = vec3<f32>(0.0);

    var t = 0.0;
    var hit = false;
    var final_p = vec3<f32>(0.0);

    for (var i = 0; i < MAX_MARCH_STEPS; i++) {
        let p_current = ro + rd * t;
        let map_data = map(p_current);
        let d = map_data.d;

        // Accumulate glow with distance attenuation to prevent oversaturation on long rays
        glow += map_data.glow * 0.05 / (1.0 + abs(d) * 20.0 + t * 0.5);

        if d < HIT_THRESHOLD {
            hit = true;
            final_p = p_current;
            break;
        }

        t += d;
        if t > MAX_MARCH_DIST {
            break;
        }
    }

    if hit {
        let n = calcNormal(final_p);

        // Fluid Material (Chrome Liquid with Environment Reflection)
        let ref_dir = reflect(rd, n);
        let fresnel = pow(1.0 - max(0.0, dot(n, -rd)), 5.0);

        // Remove white floor, render fluid infinitely
        // Orbiting key light
        let light_time = audio.time * 0.4;
        let light_dir1 = normalize(vec3<f32>(sin(light_time) * 1.5, 1.5, cos(light_time) * 1.5));
        let light1 = pow(max(0.0, dot(ref_dir, light_dir1)), SPEC_POWER);

        // Counter-orbiting fill light to eliminate periodic blackouts
        let light_dir2 = normalize(vec3<f32>(-sin(light_time) * 1.0, 2.0, -cos(light_time) * 1.0));
        let light2 = pow(max(0.0, dot(ref_dir, light_dir2)), 32.0);

        // Fake environment map: white above, dark below (simulates HDRI dome)
        let env = mix(vec3<f32>(0.02), vec3<f32>(HDR_WHITE), smoothstep(-0.2, 0.5, ref_dir.y));

        var fluid_ref = env * (0.15 + 0.85 * fresnel);       // Environment reflection
        fluid_ref += vec3<f32>(HDR_WHITE) * light1 * 2.0;    // Key light specular
        fluid_ref += vec3<f32>(3.0) * light2 * 0.5;          // Fill light specular

        col = mix(vec3<f32>(0.0), fluid_ref, 0.2 + 0.8 * fresnel);
    } else {
        // Background sky/void color
        col = vec3<f32>(HDR_WHITE);
    }

    // Fade out to pure white environment to hide the sharp horizon
    col = mix(col, vec3<f32>(HDR_WHITE), smoothstep(15.0, MAX_MARCH_DIST, t));

    col += glow;

    // Vignette
    let vignette = 1.0 - smoothstep(0.5, 1.5, length(uv));
    col *= vignette;

    // Narkowicz ACES fitted tonemap (sRGB gamma applied by WGPU surface)
    col = (col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14);

    // Output Linear RGB. WGPU Srgb surface will apply the sRGB gamma curve automatically.
    return vec4<f32>(col, 1.0);
}
