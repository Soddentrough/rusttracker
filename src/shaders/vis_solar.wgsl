// =====================================================
// Solar Flare Visualizer
// Raymarched sun with audio-reactive plasma, sunspots, and coronal prominences
// =====================================================

const MAX_MARCH_STEPS: i32 = 64;
const MAX_MARCH_DIST: f32 = 20.0;
const HIT_THRESHOLD: f32 = 0.01;
const NORMAL_EPS: f32 = 0.01;
const BASE_RADIUS: f32 = 3.2;

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
var<private> g_bass_energy: f32;
var<private> g_high_energy: f32;

// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

fn hash(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }

fn hash3(p: vec3<f32>) -> f32 {
    return hash(dot(floor(p), vec3<f32>(1.0, 57.0, 113.0)));
}

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

// Bounds-clamped channel VU accessor with stereo 12-flare upmixing
fn get_vu(i: u32) -> f32 {
    let n = max(1u, audio.num_channels);
    if n > 2u {
        let idx = i % n;
        return audio.channels[idx / 4u][idx % 4u];
    }
    let ch_l = audio.channels[0].x;
    let ch_r = select(ch_l, audio.channels[0].y, n >= 2u);
    let ph_l = audio.channel_phases[0].x;
    let ph_r = audio.channel_phases[0].y;
    let lowmid = clamp((audio.spectrum[4].x + audio.spectrum[6].x) / 140.0, 0.0, 1.0);
    let mid = clamp((audio.spectrum[16].x + audio.spectrum[24].x) / 130.0, 0.0, 1.0);
    return stereo_upmix_12(i, ch_l, ch_r, ph_l, ph_r, g_bass_energy, lowmid, mid, g_high_energy);
}

fn sdTorus(p: vec3<f32>, t: vec2<f32>) -> f32 {
    let q = vec2<f32>(length(p.xy) - t.x, p.z);
    return length(q) - t.y;
}

fn get_flare_dist(p: vec3<f32>, full_detail: bool) -> f32 {
    var flare_d = 100.0;
    let t_anim = audio.smooth_time;
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

        var t_dist = sdTorus(lp, vec2<f32>(loop_height, loop_thick));
        if full_detail && t_dist < 0.2 {
            let plasma_noise = snoise3(lp * 15.0 - vec3<f32>(t_anim * 4.0, t_anim * 2.0, 0.0));
            t_dist -= plasma_noise * 0.035;
        }

        let ex1 = g_flare_ex1[i];
        let ejecta_p1 = lp - vec3<f32>(ex1, (hash(seed+4.0)-0.5)*ex1, (hash(seed+5.0)-0.5)*ex1);
        var ejecta_dist1 = length(ejecta_p1) - 0.015;
        if full_detail && ejecta_dist1 < 0.1 {
            ejecta_dist1 -= snoise3(ejecta_p1 * 15.0) * 0.02;
        }

        let ex2 = g_flare_ex2[i];
        let ejecta_p2 = lp - vec3<f32>(ex2, (hash(seed+6.0)-0.5)*ex2, (hash(seed+7.0)-0.5)*ex2);
        var ejecta_dist2 = length(ejecta_p2) - 0.01;
        if full_detail && ejecta_dist2 < 0.1 {
            ejecta_dist2 -= snoise3(ejecta_p2 * 20.0) * 0.015;
        }

        var obj = smin_poly(t_dist, ejecta_dist1, 0.04);
        obj = smin_poly(obj, ejecta_dist2, 0.04);

        flare_d = min(flare_d, obj);
    }
    return flare_d;
}

fn setup_flares() {
    let t_anim = audio.smooth_time;
    for (var i = 0u; i < 12u; i++) {
        let vu = get_vu(i);
        let seed = f32(i) * 123.45;

        let angle1 = hash(seed) * 6.28 + t_anim * 0.05 * (hash(seed+1.0)*2.0-1.0);
        let angle2 = (hash(seed + 2.0) - 0.5) * 3.14;
        let angle3 = hash(seed + 3.0) * 6.28;

        g_flare_rot1_s[i] = sin(angle1); g_flare_rot1_c[i] = cos(angle1);
        g_flare_rot2_s[i] = sin(angle2); g_flare_rot2_c[i] = cos(angle2);
        g_flare_rot3_s[i] = sin(angle3); g_flare_rot3_c[i] = cos(angle3);

        g_flare_loop_height[i] = 0.09 + vu * 0.44;
        g_flare_loop_thick[i] = 0.015 + vu * 0.022;

        let t_shoot1 = fract(t_anim * 0.4 + seed);
        let pulse1 = sin(t_shoot1 * 3.14159);
        g_flare_ex1[i] = pulse1 * pulse1 * (0.75 + vu * 1.55);

        let t_shoot2 = fract(t_anim * 0.6 + seed * 1.3);
        let pulse2 = sin(t_shoot2 * 3.14159);
        g_flare_ex2[i] = pulse2 * pulse2 * (0.55 + vu * 2.0);
    }
}

fn map(p: vec3<f32>, full_detail: bool) -> f32 {
    let r = length(p);
    if r > 6.0 {
        return r - 5.85;
    }

    let t_rot = audio.smooth_time * 0.02;
    let s = sin(t_rot);
    let c = cos(t_rot);
    let rot_p = vec3<f32>(p.x * c - p.z * s, p.y, p.x * s + p.z * c);

    // Photosphere radius breathes subtly on heavy bass transients
    let dynamic_radius = BASE_RADIUS + g_bass_energy * 0.04;
    var d_sphere = r - dynamic_radius;
    if full_detail && d_sphere < 0.2 {
        let surface_noise = snoise3(rot_p * 8.0 + vec3<f32>(audio.smooth_time * 0.12, 0.0, 0.0)) * (0.045 + g_bass_energy * 0.025);
        d_sphere -= surface_noise;
    }

    let flare_d = get_flare_dist(p, full_detail);
    g_last_flare_dist = flare_d;

    return smin_poly(d_sphere, flare_d, 0.04);
}

fn calcNormal(p: vec3<f32>) -> vec3<f32> {
    let h = NORMAL_EPS;
    let k = vec2<f32>(1.0, -1.0);
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
    let aspect = max(audio.aspect_ratio, 0.1);
    let fit_scale = min(aspect / 1.33, 1.0);
    let p = vec2<f32>(uv.x * aspect, -uv.y) / fit_scale;

    g_bass_energy = clamp((audio.spectrum[0].x + audio.spectrum[1].x + audio.spectrum[2].x) / 210.0, 0.0, 1.0);
    g_high_energy = clamp((audio.spectrum[48].x + audio.spectrum[72].x + audio.spectrum[96].x) / 180.0, 0.0, 1.0);

    // Camera
    let ro = vec3<f32>(0.0, 0.0, 11.0);
    let cam_target = vec3<f32>(0.0, 0.0, 0.0);

    let ww = normalize(cam_target - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = normalize(cross(uu, ww));

    let fov = 4.0;
    let rd = normalize(p.x * uu + p.y * vv + fov * ww);

    var col = vec3<f32>(0.0);
    var glow = 0.0;
    var flare_glow = 0.0;
    var t = 0.0;
    var hit = false;
    var final_p = vec3<f32>(0.0);
    var hit_flare_dist = 0.0;

    let ray_miss_dist = length(cross(ro, rd));
    // Expanded bounding cylinder (5.85 vs 5.0) so tall flares are never clipped,
    // and setup_flares() is ONLY called for rays intersecting the solar corona!
    if (ray_miss_dist < 5.85) {
        setup_flares();

        let half_chord = sqrt(max(0.0, 34.2225 - ray_miss_dist * ray_miss_dist));
        let t_start = max(0.0, 11.0 - half_chord);
        t = t_start;
        let t_end = 11.0 + half_chord;

        for (var i = 0; i < 48; i++) {
            let p_current = ro + rd * t;
            let d = map(p_current, true);

            let r = length(p_current);
            if (r > BASE_RADIUS + 0.05) {
                flare_glow += 0.05 / (1.0 + abs(d) * 40.0);
            } else {
                glow += 0.05 / (1.0 + abs(d) * 20.0);
            }

            if (d < HIT_THRESHOLD) {
                hit = true;
                final_p = p_current;
                hit_flare_dist = g_last_flare_dist;
                break;
            }

            t += d * 0.95;
            if (t > t_end) { break; }
        }
    } else {
        let outer_glow = 1.0 / (1.0 + (ray_miss_dist - 4.5) * 7.5);
        glow = outer_glow * (0.15 + g_bass_energy * 0.08);
    }

    if hit {
        let n = calcNormal(final_p);
        let r = length(final_p);
        let f_dist = hit_flare_dist;

        let dark_red = vec3<f32>(0.3, 0.02, 0.0);
        let bright_orange = vec3<f32>(1.0, 0.32 + g_bass_energy * 0.12, 0.02);
        let hot_yellow = vec3<f32>(1.0, 0.82, 0.25) * (2.0 + g_bass_energy * 0.8);

        let detail_noise = fbm(final_p * 15.0 + vec3<f32>(audio.smooth_time * 0.22));
        var surface_col = mix(dark_red, bright_orange, smoothstep(0.3, 0.7, detail_noise));

        let active_region = fbm(final_p * 2.0 - vec3<f32>(audio.smooth_time * 0.05));
        let is_active = smoothstep(0.60 - g_bass_energy * 0.08, 0.78, active_region);
        surface_col = mix(surface_col, hot_yellow, is_active);

        let anchor_proximity = smoothstep_r(0.08, 0.0, f_dist);
        let is_surface = smoothstep_r(BASE_RADIUS + 0.05, BASE_RADIUS, r);
        let sunspot_intensity = anchor_proximity * is_surface;

        let sunspot_color = vec3<f32>(0.01, 0.0, 0.0);
        surface_col = mix(surface_col, sunspot_color, sunspot_intensity * 0.9);

        let fresnel = pow(1.0 - max(0.0, dot(n, -rd)), 1.5);
        surface_col = mix(surface_col, vec3<f32>(0.12, 0.01, 0.0), fresnel * 0.9);

        let is_flare = smoothstep(BASE_RADIUS + 0.02, BASE_RADIUS + 0.1, r);
        let flare_col = vec3<f32>(1.0, 0.9, 0.6) * (4.0 + g_high_energy * 1.5);

        col = mix(surface_col, flare_col, is_flare);
    }

    // Coronal radial streamers modulated by high-frequency energy
    let corona_angle = atan2(p.y, p.x);
    let streamer = (0.5 + 0.5 * sin(corona_angle * 9.0 + audio.smooth_time * 0.4))
                 * (0.5 + 0.5 * cos(corona_angle * 15.0 - audio.smooth_time * 0.7));
    let sun_bloom_color = vec3<f32>(0.85, 0.22 + g_bass_energy * 0.08, 0.02);
    col += sun_bloom_color * glow * (0.4 + streamer * g_high_energy * 0.22);

    let flare_bloom_color = vec3<f32>(1.0, 0.82, 0.42);
    col += flare_bloom_color * flare_glow * (1.5 + g_bass_energy * 0.4);

    let vignette = smoothstep_r(1.5, 0.5, length(uv));
    col *= vignette;

    col = aces_tonemap(col);

    return vec4<f32>(col, 1.0);
}
