// =====================================================
// Solar Flare Visualizer
// Raymarched sun with audio-reactive plasma and flares
// =====================================================

const MAX_MARCH_STEPS: i32 = 100;
const MAX_MARCH_DIST: f32 = 20.0;
const HIT_THRESHOLD: f32 = 0.01;
const NORMAL_EPS: f32 = 0.01;
const HDR_WHITE: f32 = 5.0;

var<private> g_flare_rot1_s: array<f32, 12>;
var<private> g_flare_rot1_c: array<f32, 12>;
var<private> g_flare_rot2_s: array<f32, 12>;
var<private> g_flare_rot2_c: array<f32, 12>;
var<private> g_flare_rot3_s: array<f32, 12>;
var<private> g_flare_rot3_c: array<f32, 12>;
var<private> g_flare_loop_height: array<f32, 12>;
var<private> g_flare_loop_thick: array<f32, 12>;
var<private> g_flare_ex1: array<f32, 12>;
var<private> g_flare_ex2: array<f32, 12>;
var<private> g_last_flare_dist: f32;

// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;


fn hash(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }

fn hash3(p: vec3<f32>) -> f32 {
    return hash(dot(floor(p), vec3<f32>(1.0, 57.0, 113.0)));
}

// 3D Simplex noise approximation
fn snoise3(x: vec3<f32>) -> f32 {
    let p = floor(x);
    let f = fract(x);
    let f2 = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(mix(hash3(p + vec3<f32>(0.0,0.0,0.0)), hash3(p + vec3<f32>(1.0,0.0,0.0)), f2.x),
            mix(hash3(p + vec3<f32>(0.0,1.0,0.0)), hash3(p + vec3<f32>(1.0,1.0,0.0)), f2.x), f2.y),
        mix(mix(hash3(p + vec3<f32>(0.0,0.0,1.0)), hash3(p + vec3<f32>(1.0,0.0,1.0)), f2.x),
            mix(hash3(p + vec3<f32>(0.0,1.0,1.0)), hash3(p + vec3<f32>(1.0,1.0,1.0)), f2.x), f2.y), f2.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    var f = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        f += a * snoise3(q);
        q = q * 2.0;
        a *= 0.5;
    }
    return f;
}

// Bounds-clamped channel VU accessor
fn get_vu(i: u32) -> f32 {
    let n = max(1u, audio.num_channels);
    let idx = min(i, n - 1u);
    let v = audio.channels[idx / 4u];
    let c = idx % 4u;
    if (c == 0u) { return v.x; } else if (c == 1u) { return v.y; }
    else if (c == 2u) { return v.z; } else { return v.w; }
}

const BASE_RADIUS: f32 = 3.2;

fn sdTorus(p: vec3<f32>, t: vec2<f32>) -> f32 {
    let q = vec2<f32>(length(p.xy) - t.x, p.z);
    return length(q) - t.y;
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// Returns just the flare distance
fn get_flare_dist(p: vec3<f32>, full_detail: bool) -> f32 {
    var flare_d = 100.0;
    // Reduce flare count from 24 to 12 for performance
    for (var i = 0u; i < 12u; i++) {
        let seed = f32(i) * 123.45;
        
        let s1 = g_flare_rot1_s[i]; let c1 = g_flare_rot1_c[i];
        let s2 = g_flare_rot2_s[i]; let c2 = g_flare_rot2_c[i];
        let s3 = g_flare_rot3_s[i]; let c3 = g_flare_rot3_c[i];
        
        var lp = p;
        lp = vec3<f32>(lp.x * c1 + lp.z * s1, lp.y, -lp.x * s1 + lp.z * c1);
        lp = vec3<f32>(lp.x * c2 + lp.y * s2, -lp.x * s2 + lp.y * c2, lp.z);
        lp = vec3<f32>(lp.x, lp.y * c3 + lp.z * s3, -lp.y * s3 + lp.z * c3);
        
        lp.x -= BASE_RADIUS - 0.02;
        
        let loop_height = g_flare_loop_height[i];
        let loop_thick = g_flare_loop_thick[i];
        
        // BOUNDING VOLUME: Only evaluate expensive 3D noise if the ray is close to the flare
        var t_dist = sdTorus(lp, vec2<f32>(loop_height, loop_thick));
        if full_detail && t_dist < 0.2 {
            let plasma_noise = fbm(lp * 15.0 - vec3<f32>(audio.time * 4.0, audio.time * 2.0, 0.0));
            t_dist -= plasma_noise * 0.035;
        }
        
        let ex1 = g_flare_ex1[i]; 
        let ejecta_p1 = lp - vec3<f32>(ex1, (hash(seed+4.0)-0.5)*ex1, (hash(seed+5.0)-0.5)*ex1);
        var ejecta_dist1 = length(ejecta_p1) - 0.015;
        if full_detail && ejecta_dist1 < 0.1 {
            ejecta_dist1 -= snoise3(ejecta_p1 * 15.0) * 0.02; // Single octave noise for ejecta
        }

        let ex2 = g_flare_ex2[i];
        let ejecta_p2 = lp - vec3<f32>(ex2, (hash(seed+6.0)-0.5)*ex2, (hash(seed+7.0)-0.5)*ex2);
        var ejecta_dist2 = length(ejecta_p2) - 0.01;
        if full_detail && ejecta_dist2 < 0.1 {
            ejecta_dist2 -= snoise3(ejecta_p2 * 20.0) * 0.015;
        }

        var obj = smin(t_dist, ejecta_dist1, 0.04);
        obj = smin(obj, ejecta_dist2, 0.04);
        
        flare_d = min(flare_d, obj);
    }
    return flare_d;
}

fn setup_flares() {
    for (var i = 0u; i < 12u; i++) {
        let vu = get_vu(i % 12u);
        let seed = f32(i) * 123.45;
        
        let angle1 = hash(seed) * 6.28 + audio.time * 0.05 * (hash(seed+1.0)*2.0-1.0); 
        let angle2 = (hash(seed + 2.0) - 0.5) * 3.14; 
        let angle3 = hash(seed + 3.0) * 6.28; 
        
        g_flare_rot1_s[i] = sin(angle1); g_flare_rot1_c[i] = cos(angle1);
        g_flare_rot2_s[i] = sin(angle2); g_flare_rot2_c[i] = cos(angle2);
        g_flare_rot3_s[i] = sin(angle3); g_flare_rot3_c[i] = cos(angle3);
        
        g_flare_loop_height[i] = 0.08 + vu * 0.4;
        g_flare_loop_thick[i] = 0.015 + vu * 0.02;
        
        let t_shoot1 = fract(audio.time * 0.4 + seed);
        g_flare_ex1[i] = t_shoot1 * (0.8 + vu * 1.5); 
        
        let t_shoot2 = fract(audio.time * 0.6 + seed * 1.3);
        g_flare_ex2[i] = t_shoot2 * (0.6 + vu * 2.0);
    }
}

fn map(p: vec3<f32>, full_detail: bool) -> f32 {
    let t_rot = audio.time * 0.02;
    let s = sin(t_rot);
    let c = cos(t_rot);
    let rot_p = vec3<f32>(p.x * c - p.z * s, p.y, p.x * s + p.z * c);

    // Fine-grained Plasma displacement (only when full_detail)
    var d_sphere = length(p) - BASE_RADIUS;
    if full_detail && d_sphere < 0.2 {
        let surface_noise = fbm(rot_p * 8.0 + vec3<f32>(audio.time * 0.1, 0.0, 0.0)) * 0.05;
        d_sphere -= surface_noise;
    }
    
    let flare_d = get_flare_dist(p, full_detail);
    g_last_flare_dist = flare_d;
    
    return smin(d_sphere, flare_d, 0.04);
}

fn calcNormal(p: vec3<f32>) -> vec3<f32> {
    let h = NORMAL_EPS;
    let k = vec2<f32>(1.0, -1.0);
    // Use full_detail = false for normal calculation to save massive performance
    return normalize(
        k.xyy * map(p + k.xyy * h, false) + 
        k.yyx * map(p + k.yyx * h, false) + 
        k.yxy * map(p + k.yxy * h, false) + 
        k.xxx * map(p + k.xxx * h, false)
    );
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    var aspect = 1.7777;
    let dy = abs(dpdy(in.uv.y));
    let dx = abs(dpdx(in.uv.x));
    if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }
    let p = vec2<f32>(uv.x * aspect, -uv.y);

    setup_flares();

    // Camera
    let ro = vec3<f32>(0.0, 0.0, 11.0);
    let cam_target = vec3<f32>(0.0, 0.0, 0.0);

    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = normalize(cross(uu, ww));

    // Telephoto FOV
    let fov = 4.0;
    let rd = normalize(p.x * uu + p.y * vv + fov * ww);

    var col = vec3<f32>(0.0);
    var glow = 0.0;
    var flare_glow = 0.0;
    var t = 0.0;
    var hit = false;
    var final_p = vec3<f32>(0.0);

    for (var i = 0; i < MAX_MARCH_STEPS; i++) {
        let p_current = ro + rd * t;
        let d = map(p_current, true);
        
        let r = length(p_current);
        if r > BASE_RADIUS + 0.05 {
            flare_glow += 0.05 / (1.0 + abs(d) * 40.0);
        } else {
            glow += 0.05 / (1.0 + abs(d) * 20.0);
        }

        if d < HIT_THRESHOLD {
            hit = true;
            final_p = p_current;
            break;
        }

        t += d * 0.7;
        if t > MAX_MARCH_DIST { break; }
    }

    if hit {
        let n = calcNormal(final_p);
        let r = length(final_p);
        let f_dist = g_last_flare_dist;
        
        // Base surface colors to match reference
        let dark_red = vec3<f32>(0.3, 0.02, 0.0);
        let bright_orange = vec3<f32>(1.0, 0.3, 0.0);
        let hot_yellow = vec3<f32>(1.0, 0.8, 0.2) * 2.0;
        
        // High-frequency detail for the granular look
        let detail_noise = fbm(final_p * 15.0 + vec3<f32>(audio.time * 0.2));
        var surface_col = mix(dark_red, bright_orange, smoothstep(0.3, 0.7, detail_noise));
        
        // Low-frequency noise for large bright active regions (plages)
        let active_region = fbm(final_p * 2.0 - vec3<f32>(audio.time * 0.05));
        let is_active = smoothstep(0.6, 0.8, active_region);
        surface_col = mix(surface_col, hot_yellow, is_active);
        
        // Sunspots: extreme dark points at flare anchors
        let anchor_proximity = 1.0 - smoothstep(0.0, 0.08, f_dist);
        let is_surface = 1.0 - smoothstep(BASE_RADIUS, BASE_RADIUS + 0.05, r);
        let sunspot_intensity = anchor_proximity * is_surface;
        
        let sunspot_color = vec3<f32>(0.01, 0.0, 0.0);
        surface_col = mix(surface_col, sunspot_color, sunspot_intensity * 0.9);
        
        // Fresnel limb darkening
        let fresnel = pow(1.0 - max(0.0, dot(n, -rd)), 1.5);
        surface_col = mix(surface_col, vec3<f32>(0.1, 0.0, 0.0), fresnel * 0.9);
        
        // Flare coloring
        let is_flare = smoothstep(BASE_RADIUS + 0.02, BASE_RADIUS + 0.1, r);
        let flare_col = vec3<f32>(1.0, 0.9, 0.6) * 4.0;
        
        col = mix(surface_col, flare_col, is_flare);
    }

    // Add volumetric bloom
    let sun_bloom_color = vec3<f32>(0.8, 0.2, 0.0);
    col += sun_bloom_color * glow * 0.4;
    
    let flare_bloom_color = vec3<f32>(1.0, 0.8, 0.4);
    col += flare_bloom_color * flare_glow * 1.5;

    // Vignette
    let vignette = 1.0 - smoothstep(0.5, 1.5, length(uv));
    col *= vignette;

    // Tonemapping
    col = (col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14);

    return vec4<f32>(col, 1.0);
}
