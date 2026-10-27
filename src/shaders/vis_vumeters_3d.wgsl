// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(2) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
};

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) mat: f32,
    @location(4) local_pos: vec3<f32>,
    @location(5) @interpolate(flat) levels: vec4<f32>,
};

const PHI_MIN: f32 = -0.68;
const PHI_MAX: f32 = 0.68;

fn hash21_vu_fast(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}

fn paper_noise_fast(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21_vu_fast(i);
    let b = hash21_vu_fast(i + vec2<f32>(1.0, 0.0));
    let c = hash21_vu_fast(i + vec2<f32>(0.0, 1.0));
    let d = hash21_vu_fast(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Compute responsive Left/Right VU and Peak levels across stereo or multi-channel tracker audio
fn get_stereo_vu_peak() -> vec4<f32> {
    var vu_l = audio.channels[0].x;
    var vu_r = audio.channels[0].y;
    var pk_l = audio.channel_peaks[0].x;
    var pk_r = audio.channel_peaks[0].y;

    let n_ch = min(audio.num_channels, 32u);
    if (n_ch > 2u) {
        var sum_l = 0.0;
        var sum_r = 0.0;
        var max_l = 0.0;
        var max_r = 0.0;
        for (var i = 0u; i < n_ch; i = i + 1u) {
            let v = audio.channels[i / 4u][i % 4u];
            let p = audio.channel_peaks[i / 4u][i % 4u];
            // Amiga LRRL panning or even/odd stereo split
            if ((i & 1u) == 0u) {
                sum_l += v * v;
                max_l = max(max_l, p);
            } else {
                sum_r += v * v;
                max_r = max(max_r, p);
            }
        }
        let half_ch = max(f32(n_ch) * 0.5, 1.0);
        vu_l = max(vu_l, sqrt(sum_l / half_ch) * 1.45);
        vu_r = max(vu_r, sqrt(sum_r / half_ch) * 1.45);
        pk_l = max(pk_l, max_l);
        pk_r = max(pk_r, max_r);
    }

    return vec4<f32>(
        clamp(vu_l * 1.35, 0.0, 1.08),
        clamp(vu_r * 1.35, 0.0, 1.08),
        clamp(pk_l, 0.0, 1.2),
        clamp(pk_r, 0.0, 1.2)
    );
}

// --- Analytical Vector Stroke Font for Vintage VU Dial Faceplate ---
fn draw_vector_char(ch: u32, p: vec2<f32>, origin: vec2<f32>, scale: f32, local_pixel_size: f32) -> f32 {
    let lp = (p - origin) / scale;
    if (lp.x < -0.15 || lp.x > 1.15 || lp.y < -0.15 || lp.y > 2.15) { return 0.0; }

    var d = 1e6;
    switch ch {
        case 10u { // L
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 0.0)));
        }
        case 11u { // R
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.5)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.5), vec2<f32>(0.6, 1.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 1.0), vec2<f32>(0.0, 1.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.4, 1.0), vec2<f32>(0.8, 0.0)));
        }
        case 12u { // -
            d = min(d, sd_line_segment(lp, vec2<f32>(0.1, 1.0), vec2<f32>(0.7, 1.0)));
        }
        case 13u { // +
            d = min(d, sd_line_segment(lp, vec2<f32>(0.1, 1.0), vec2<f32>(0.7, 1.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.4, 0.6), vec2<f32>(0.4, 1.4)));
        }
        case 14u { // D
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.9, 1.5)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.9, 1.5), vec2<f32>(0.9, 0.5)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.9, 0.5), vec2<f32>(0.6, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 15u { // E
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.8, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 1.0), vec2<f32>(0.6, 1.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 0.0)));
        }
        case 16u { // N
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.9, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.9, 0.0), vec2<f32>(0.9, 2.0)));
        }
        case 17u { // O
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.3), vec2<f32>(0.0, 1.7)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 1.7), vec2<f32>(0.3, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.3, 2.0), vec2<f32>(0.7, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.7, 2.0), vec2<f32>(1.0, 1.7)));
            d = min(d, sd_line_segment(lp, vec2<f32>(1.0, 1.7), vec2<f32>(1.0, 0.3)));
            d = min(d, sd_line_segment(lp, vec2<f32>(1.0, 0.3), vec2<f32>(0.7, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.7, 0.0), vec2<f32>(0.3, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.3, 0.0), vec2<f32>(0.0, 0.3)));
        }
        case 18u { // V
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.5, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.5, 0.0), vec2<f32>(1.0, 2.0)));
        }
        case 19u { // U
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.2), vec2<f32>(0.2, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.2, 0.0), vec2<f32>(0.8, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.0), vec2<f32>(1.0, 0.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(1.0, 0.2), vec2<f32>(1.0, 2.0)));
        }
        case 20u { // S
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.7), vec2<f32>(0.8, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.0, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 1.1)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 1.1), vec2<f32>(0.8, 0.9)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.9), vec2<f32>(0.8, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 0.3)));
        }
        case 21u { // I
            d = min(d, sd_line_segment(lp, vec2<f32>(0.4, 2.0), vec2<f32>(0.4, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.1, 2.0), vec2<f32>(0.7, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.1, 0.0), vec2<f32>(0.7, 0.0)));
        }
        case 22u { // G
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.7), vec2<f32>(0.8, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.0, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.0), vec2<f32>(0.8, 1.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.0), vec2<f32>(0.4, 1.0)));
        }
        case 23u { // A
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.4, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.4, 2.0), vec2<f32>(0.8, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.15, 0.8), vec2<f32>(0.65, 0.8)));
        }
        case 24u { // B
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.5)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.5), vec2<f32>(0.6, 1.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 1.0), vec2<f32>(0.0, 1.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 1.0), vec2<f32>(0.8, 0.5)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.5), vec2<f32>(0.6, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 25u { // %
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.05, 1.6), vec2<f32>(0.35, 1.9)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.45, 0.1), vec2<f32>(0.75, 0.4)));
        }
        case 0u { // 0
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.2), vec2<f32>(0.0, 1.8)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 1.8), vec2<f32>(0.2, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.8)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.8), vec2<f32>(0.8, 0.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.2), vec2<f32>(0.6, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.2, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.2, 0.0), vec2<f32>(0.0, 0.2)));
        }
        case 1u { // 1
            d = min(d, sd_line_segment(lp, vec2<f32>(0.1, 1.5), vec2<f32>(0.4, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.4, 2.0), vec2<f32>(0.4, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.1, 0.0), vec2<f32>(0.7, 0.0)));
        }
        case 2u { // 2
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 1.7), vec2<f32>(0.2, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.7)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.7), vec2<f32>(0.8, 1.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.2), vec2<f32>(0.0, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 0.0)));
        }
        case 3u { // 3
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.8, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.8, 1.1)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.1), vec2<f32>(0.3, 1.1)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.1), vec2<f32>(0.8, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 4u { // 4
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.8)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.8), vec2<f32>(0.8, 0.8)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.6, 0.0)));
        }
        case 5u { // 5
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.0, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 1.1)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 1.1), vec2<f32>(0.7, 1.1)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.7, 1.1), vec2<f32>(0.8, 0.9)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.9), vec2<f32>(0.8, 0.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.2), vec2<f32>(0.6, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 6u { // 6
            d = min(d, sd_line_segment(lp, vec2<f32>(0.7, 2.0), vec2<f32>(0.2, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.0, 1.7)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 1.7), vec2<f32>(0.0, 0.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.2), vec2<f32>(0.2, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.2, 0.0), vec2<f32>(0.6, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.8, 0.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.2), vec2<f32>(0.8, 0.9)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.9), vec2<f32>(0.6, 1.1)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 1.1), vec2<f32>(0.0, 1.1)));
        }
        case 7u { // 7
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.8, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.3, 0.0)));
        }
        case 8u { // 8
            d = min(d, sd_line_segment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.7)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 1.7), vec2<f32>(0.8, 0.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.8, 0.2), vec2<f32>(0.6, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.2, 0.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.2, 0.0), vec2<f32>(0.0, 0.2)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 0.2), vec2<f32>(0.0, 1.7)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.0, 1.7), vec2<f32>(0.2, 2.0)));
            d = min(d, sd_line_segment(lp, vec2<f32>(0.1, 1.05), vec2<f32>(0.7, 1.05)));
        }
        default {}
    }

    let thickness = 0.13;
    let char_d = d * scale;
    let blur = local_pixel_size * 0.85;
    return smoothstep_r(thickness * scale + blur, thickness * scale - blur, char_d);
}

fn draw_v_string_2(s: array<u32, 2>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, lps: f32) -> f32 {
    if (p.y < origin.y - 0.15 * scale || p.y > origin.y + 2.15 * scale) { return 0.0; }
    let step_x = scale + spacing;
    let rel_x = p.x - origin.x;
    if (rel_x < -0.15 * scale || rel_x > 2.0 * step_x + 0.15 * scale) { return 0.0; }
    let j = u32(clamp(floor((rel_x + 0.15 * scale) / step_x), 0.0, 1.0));
    let char_origin = origin + vec2<f32>(f32(j) * step_x, 0.0);
    return draw_vector_char(s[j], p, char_origin, scale, lps);
}

fn draw_v_string_3(s: array<u32, 3>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, lps: f32) -> f32 {
    if (p.y < origin.y - 0.15 * scale || p.y > origin.y + 2.15 * scale) { return 0.0; }
    let step_x = scale + spacing;
    let rel_x = p.x - origin.x;
    if (rel_x < -0.15 * scale || rel_x > 3.0 * step_x + 0.15 * scale) { return 0.0; }
    let j = u32(clamp(floor((rel_x + 0.15 * scale) / step_x), 0.0, 2.0));
    let char_origin = origin + vec2<f32>(f32(j) * step_x, 0.0);
    return draw_vector_char(s[j], p, char_origin, scale, lps);
}

fn draw_v_string_5(s: array<u32, 5>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, lps: f32) -> f32 {
    if (p.y < origin.y - 0.15 * scale || p.y > origin.y + 2.15 * scale) { return 0.0; }
    let step_x = scale + spacing;
    let rel_x = p.x - origin.x;
    if (rel_x < -0.15 * scale || rel_x > 5.0 * step_x + 0.15 * scale) { return 0.0; }
    let j = u32(clamp(floor((rel_x + 0.15 * scale) / step_x), 0.0, 4.0));
    let char_origin = origin + vec2<f32>(f32(j) * step_x, 0.0);
    return draw_vector_char(s[j], p, char_origin, scale, lps);
}

fn draw_v_string_6(s: array<u32, 6>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, lps: f32) -> f32 {
    if (p.y < origin.y - 0.15 * scale || p.y > origin.y + 2.15 * scale) { return 0.0; }
    let step_x = scale + spacing;
    let rel_x = p.x - origin.x;
    if (rel_x < -0.15 * scale || rel_x > 6.0 * step_x + 0.15 * scale) { return 0.0; }
    let j = u32(clamp(floor((rel_x + 0.15 * scale) / step_x), 0.0, 5.0));
    let char_origin = origin + vec2<f32>(f32(j) * step_x, 0.0);
    return draw_vector_char(s[j], p, char_origin, scale, lps);
}

fn draw_arc_label_1(ch: u32, p: vec2<f32>, pivot: vec2<f32>, p_theta: f32, theta: f32, radius: f32, scale: f32, lps: f32) -> f32 {
    if (abs(p_theta - theta) > 0.065) { return 0.0; }
    let center = pivot + vec2<f32>(sin(theta), cos(theta)) * radius;
    let origin = center - vec2<f32>(0.4 * scale, 1.0 * scale);
    return draw_vector_char(ch, p, origin, scale, lps);
}

fn draw_arc_label_2(chars: array<u32, 2>, p: vec2<f32>, pivot: vec2<f32>, p_theta: f32, theta: f32, radius: f32, scale: f32, lps: f32) -> f32 {
    if (abs(p_theta - theta) > 0.095) { return 0.0; }
    let center = pivot + vec2<f32>(sin(theta), cos(theta)) * radius;
    let spacing = scale * 0.25;
    let total_w = 2.0 * scale + spacing;
    let origin = center - vec2<f32>(total_w * 0.5, 1.0 * scale);
    return draw_v_string_2(chars, p, origin, scale, spacing, lps);
}

fn draw_arc_label_3(chars: array<u32, 3>, p: vec2<f32>, pivot: vec2<f32>, p_theta: f32, theta: f32, radius: f32, scale: f32, lps: f32) -> f32 {
    if (abs(p_theta - theta) > 0.125) { return 0.0; }
    let center = pivot + vec2<f32>(sin(theta), cos(theta)) * radius;
    let spacing = scale * 0.22;
    let total_w = 3.0 * scale + 2.0 * spacing;
    let origin = center - vec2<f32>(total_w * 0.5, 1.0 * scale);
    return draw_v_string_3(chars, p, origin, scale, spacing, lps);
}

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    let mat_id = in.tex_coords.x;
    var pos = in.position;
    var norm = in.normal;

    let levels = get_stereo_vu_peak();
    let vu_l = levels.x;
    let vu_r = levels.y;

    // Needle Mechanical Rotation around Exact OBJ Pivot Hub (lx=-2.70, ly=-1.15 and rx=2.70, ry=-1.15)
    if (mat_id >= 3.8 && mat_id <= 4.2) {
        let pivot = vec2<f32>(-2.70, -1.15);
        let ang = mix(0.68, -0.68, vu_l);
        let cos_a = cos(ang);
        let sin_a = sin(ang);
        let rel = pos.xy - pivot;
        pos.x = pivot.x + (rel.x * cos_a - rel.y * sin_a);
        pos.y = pivot.y + (rel.x * sin_a + rel.y * cos_a);
        let nx = norm.x * cos_a - norm.y * sin_a;
        let ny = norm.x * sin_a + norm.y * cos_a;
        norm = vec3<f32>(nx, ny, norm.z);
    } else if (mat_id >= 4.8 && mat_id <= 5.2) {
        let pivot = vec2<f32>(2.70, -1.15);
        let ang = mix(0.68, -0.68, vu_r);
        let cos_a = cos(ang);
        let sin_a = sin(ang);
        let rel = pos.xy - pivot;
        pos.x = pivot.x + (rel.x * cos_a - rel.y * sin_a);
        pos.y = pivot.y + (rel.x * sin_a + rel.y * cos_a);
        let nx = norm.x * cos_a - norm.y * sin_a;
        let ny = norm.x * sin_a + norm.y * cos_a;
        norm = vec3<f32>(nx, ny, norm.z);
    }

    out.world_pos = pos;
    out.normal = norm;
    out.uv = in.tex_coords;
    out.mat = mat_id;
    out.local_pos = in.position;
    out.levels = levels;

    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let mat_id = in.mat;
    let n = normalize(in.normal);
    let v = normalize(vec3<f32>(0.0, 0.0, 7.5) - in.world_pos);
    let levels = in.levels;

    // Studio ambient & key lighting
    let key_light_dir = normalize(vec3<f32>(0.3, 0.8, 1.0));
    let key_diffuse = max(dot(n, key_light_dir), 0.0);

    var color = vec3<f32>(0.0);

    if (mat_id < 1.5) {
        // =========================================================================
        // 1.0: BRUSHED ALUMINUM RACK FACEPLATE
        // =========================================================================
        let brush = sin(in.world_pos.y * 140.0) * 0.015;
        let base_al = vec3<f32>(0.13, 0.14, 0.16) + brush;
        let spec = pow(max(dot(reflect(-key_light_dir, n), v), 0.0), 16.0) * 0.40;
        let bevel = smoothstep(5.4, 5.5, abs(in.world_pos.x)) * 0.25;
        color = base_al * (0.65 + key_diffuse * 0.35) + vec3<f32>(spec + bevel);

    } else if (mat_id < 2.5) {
        // =========================================================================
        // 2.0: MATTE BLACK RECESSED METER CAVITY BEZELS
        // =========================================================================
        let black_bezel = vec3<f32>(0.02, 0.02, 0.025);
        color = black_bezel * (0.5 + key_diffuse * 0.5);

    } else if (mat_id < 3.5) {
        // =========================================================================
        // 3.0: VINTAGE CREAM PARCHMENT DIAL SCALE (Full VU Scale + Tungsten Glow)
        // =========================================================================
        let is_left = in.world_pos.x < 0.0;
        let center_x = select(2.70, -2.70, is_left);
        let vu_val = select(levels.y, levels.x, is_left);
        let pk_val = select(levels.w, levels.z, is_left);

        // Map world dial coordinates (center ±2.70, pivot y=-1.15, scale 1.95) into normalized q space
        let pivot = vec2<f32>(0.0, -0.55);
        let v_scale = vec2<f32>(in.world_pos.x - center_x, in.world_pos.y - (-1.15)) / 1.95;
        let q = pivot + v_scale;
        let d_scale = length(v_scale);
        let theta = atan2(v_scale.x, v_scale.y);
        let local_pixel_size = clamp(length(vec2<f32>(dpdx(q.x), dpdy(q.y))), 0.001, 0.015);

        // Warm amber incandescent backlight — bright warm gold at bottom -> deep amber at top
        let y_norm = clamp((q.y + 0.52) / 1.25, 0.0, 1.0);
        let lamp_pos = vec2<f32>(0.0, -0.50);
        let d_lamp = length(q - lamp_pos);
        let lamp_falloff = exp(-d_lamp * 1.15);

        let warm_gold = vec3<f32>(1.0, 0.74, 0.22);
        let deep_amber = vec3<f32>(0.92, 0.42, 0.06);
        let dark_edge = vec3<f32>(0.28, 0.10, 0.02);

        var faceplate = mix(warm_gold, deep_amber, y_norm * 0.68);
        faceplate = mix(faceplate, dark_edge, smoothstep(0.25, 1.0, y_norm * y_norm));
        faceplate *= (0.85 + 0.65 * lamp_falloff) * (0.94 + 0.18 * vu_val);

        // Subtle ivory/paper grain so the dial reads as printed card
        faceplate *= 0.97 + 0.05 * paper_noise_fast(q * 200.0);

        // Soft cavity edge vignette
        let edge_vignette = smoothstep_r(1.08, 0.60, max(abs(q.x) / 1.05, abs(q.y - 0.10) / 0.68));
        faceplate *= mix(0.35, 1.0, edge_vignette);

        // === TICK MARKS — Full VU standard scale (-20, -10, -7, -5, -3, -2, -1, 0, +1, +2, +3) ===
        let db_vals = array<f32, 11>(0.0, 0.20, 0.32, 0.42, 0.54, 0.62, 0.70, 0.77, 0.84, 0.91, 1.0);
        let db_is_red = array<bool, 11>(false, false, false, false, false, false, false, true, true, true, true);
        let db_is_major = array<bool, 11>(true, true, true, true, true, false, false, true, false, false, true);

        var tick_mask = 0.0;
        var tick_color = vec3<f32>(0.05);
        let angular_pixel_size = local_pixel_size / max(d_scale, 0.1);
        let tick_w = 0.0035;
        let outer_r = 1.10;

        if (d_scale >= outer_r - 0.105 - local_pixel_size && d_scale <= outer_r + local_pixel_size) {
            for (var ti = 0u; ti < 11u; ti++) {
                let theta_tick = PHI_MIN + db_vals[ti] * (PHI_MAX - PHI_MIN);
                let diff_t = abs(theta - theta_tick);
                if (diff_t > tick_w + angular_pixel_size) { continue; }
                let tick_len = select(0.06, 0.105, db_is_major[ti]);
                let inner_r = outer_r - tick_len;

                if (d_scale >= inner_r - local_pixel_size) {
                    let w_mask = smoothstep_r(tick_w + angular_pixel_size, tick_w - angular_pixel_size, diff_t);
                    let r_mask = smoothstep_r(outer_r + local_pixel_size, outer_r - local_pixel_size, d_scale) *
                                 smoothstep(inner_r - local_pixel_size, inner_r + local_pixel_size, d_scale);
                    let current_tick_mask = w_mask * r_mask;
                    if (current_tick_mask > 0.0) {
                        let current_tick_color = select(vec3<f32>(0.03, 0.02, 0.01), vec3<f32>(0.78, 0.05, 0.02), db_is_red[ti]);
                        tick_color = mix(tick_color, current_tick_color, current_tick_mask);
                        tick_mask = max(tick_mask, current_tick_mask);
                    }
                }
            }
        }

        // === Baseline Arc (dB scale + red overload bar) ===
        var arc_mask = 0.0;
        var arc_color = vec3<f32>(0.03, 0.02, 0.01);
        let arc_r = 1.05;
        if (d_scale >= arc_r - 0.012 && d_scale <= arc_r + 0.012 && theta >= PHI_MIN - local_pixel_size && theta <= PHI_MAX + local_pixel_size) {
            let rad_diff = abs(d_scale - arc_r);
            let is_red_arc = theta > (PHI_MIN + 0.77 * (PHI_MAX - PHI_MIN));
            let arc_half_w = select(0.006, 0.010, is_red_arc);
            let r_mask = smoothstep_r(arc_half_w + local_pixel_size, arc_half_w - local_pixel_size, rad_diff);
            let end_mask = smoothstep(PHI_MIN - local_pixel_size, PHI_MIN + local_pixel_size, theta) *
                           smoothstep_r(PHI_MAX + local_pixel_size, PHI_MAX - local_pixel_size, theta);
            arc_mask = r_mask * end_mask;
            arc_color = select(vec3<f32>(0.03, 0.02, 0.01), vec3<f32>(0.80, 0.05, 0.02), is_red_arc);
        }

        // === Percentage scale: smaller ticks and arc below the dB arc (0, 20, 40, 60, 80, 100%) ===
        let pct_vals = array<f32, 6>(0.0, 0.20, 0.42, 0.62, 0.77, 0.91);
        let pct_outer = 0.96;
        let pct_inner = 0.90;
        let pct_arc_r = 0.93;
        var pct_tick_mask = 0.0;
        let pct_tick_color = vec3<f32>(0.03, 0.02, 0.01);

        if (d_scale >= pct_inner - local_pixel_size && d_scale <= pct_outer + local_pixel_size) {
            for (var pi = 0u; pi < 6u; pi++) {
                let theta_pct = PHI_MIN + pct_vals[pi] * (PHI_MAX - PHI_MIN);
                let diff_p = abs(theta - theta_pct);
                if (diff_p > 0.003 + angular_pixel_size) { continue; }
                let w_m = smoothstep_r(0.003 + angular_pixel_size, 0.003 - angular_pixel_size, diff_p);
                let r_m = smoothstep_r(pct_outer + local_pixel_size, pct_outer - local_pixel_size, d_scale) *
                          smoothstep(pct_inner - local_pixel_size, pct_inner + local_pixel_size, d_scale);
                pct_tick_mask = max(pct_tick_mask, w_m * r_m);
            }
        }

        var pct_arc_mask = 0.0;
        if (d_scale >= pct_arc_r - 0.006 && d_scale <= pct_arc_r + 0.006 && theta >= PHI_MIN - local_pixel_size && theta <= PHI_MAX + local_pixel_size) {
            let pct_rad_diff = abs(d_scale - pct_arc_r);
            let pct_r_mask = smoothstep_r(0.004 + local_pixel_size, 0.004 - local_pixel_size, pct_rad_diff);
            let pct_end = smoothstep(PHI_MIN - local_pixel_size, PHI_MIN + local_pixel_size, theta) *
                          smoothstep_r(PHI_MAX + local_pixel_size, PHI_MAX - local_pixel_size, theta);
            pct_arc_mask = pct_r_mask * pct_end;
        }

        // === dB number labels along the outer arc ===
        let label_r = 1.19;
        let lbl_scale = 0.032;
        let lps = local_pixel_size;
        var db_num_ink = 0.0;
        if (d_scale > 1.12 && d_scale < 1.26) {
            db_num_ink = max(db_num_ink, draw_arc_label_2(array<u32, 2>(2u, 0u), q, pivot, theta, PHI_MIN + 0.00 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_2(array<u32, 2>(1u, 0u), q, pivot, theta, PHI_MIN + 0.20 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(7u, q, pivot, theta, PHI_MIN + 0.32 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(5u, q, pivot, theta, PHI_MIN + 0.42 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(3u, q, pivot, theta, PHI_MIN + 0.54 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(2u, q, pivot, theta, PHI_MIN + 0.62 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(1u, q, pivot, theta, PHI_MIN + 0.70 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(0u, q, pivot, theta, PHI_MIN + 0.77 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(1u, q, pivot, theta, PHI_MIN + 0.84 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(2u, q, pivot, theta, PHI_MIN + 0.91 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
            db_num_ink = max(db_num_ink, draw_arc_label_1(3u, q, pivot, theta, PHI_MIN + 1.00 * (PHI_MAX - PHI_MIN), label_r, lbl_scale, lps));
        }

        let pixel_theta_norm = (theta - PHI_MIN) / (PHI_MAX - PHI_MIN);
        let db_num_color = select(vec3<f32>(0.03, 0.02, 0.01), vec3<f32>(0.78, 0.05, 0.02), pixel_theta_norm > 0.77);

        // === Percentage number labels ===
        let pct_label_r = 0.83;
        let pct_lbl_scale = 0.025;
        var pct_num_ink = 0.0;
        if (d_scale > 0.75 && d_scale < 0.89) {
            pct_num_ink = max(pct_num_ink, draw_arc_label_1(0u, q, pivot, theta, PHI_MIN, pct_label_r, pct_lbl_scale, lps));
            pct_num_ink = max(pct_num_ink, draw_arc_label_2(array<u32, 2>(2u, 0u), q, pivot, theta, PHI_MIN + 0.20 * (PHI_MAX - PHI_MIN), pct_label_r, pct_lbl_scale, lps));
            pct_num_ink = max(pct_num_ink, draw_arc_label_2(array<u32, 2>(4u, 0u), q, pivot, theta, PHI_MIN + 0.42 * (PHI_MAX - PHI_MIN), pct_label_r, pct_lbl_scale, lps));
            pct_num_ink = max(pct_num_ink, draw_arc_label_2(array<u32, 2>(6u, 0u), q, pivot, theta, PHI_MIN + 0.62 * (PHI_MAX - PHI_MIN), pct_label_r, pct_lbl_scale, lps));
            pct_num_ink = max(pct_num_ink, draw_arc_label_2(array<u32, 2>(8u, 0u), q, pivot, theta, PHI_MIN + 0.77 * (PHI_MAX - PHI_MIN), pct_label_r, pct_lbl_scale, lps));
            pct_num_ink = max(pct_num_ink, draw_arc_label_3(array<u32, 3>(1u, 0u, 0u), q, pivot, theta, PHI_MIN + 0.91 * (PHI_MAX - PHI_MIN), pct_label_r, pct_lbl_scale, lps));
            pct_num_ink = max(pct_num_ink, draw_arc_label_1(25u, q, pivot, theta, PHI_MAX, pct_label_r - 0.04, pct_lbl_scale, lps));
        }

        // === Dial Face Typography ("AUDIO", "VU", "SIGNAL", "L/R - LEVEL dB") ===
        let brand_ink = draw_v_string_5(array<u32, 5>(23u, 19u, 14u, 21u, 17u), q, vec2<f32>(-0.62, -0.36), 0.046, 0.014, lps);
        let vu_ink = draw_v_string_2(array<u32, 2>(18u, 19u), q, vec2<f32>(0.40, -0.36), 0.048, 0.014, lps);
        let signal_ink = draw_v_string_6(array<u32, 6>(20u, 21u, 22u, 16u, 23u, 10u), q, vec2<f32>(-0.24, -0.14), 0.036, 0.010, lps);
        let first_char = select(11u, 10u, is_left);
        let label_ink = draw_vector_char(first_char, q, vec2<f32>(-0.46, 0.14), 0.036, lps) +
                        draw_vector_char(12u, q, vec2<f32>(-0.32, 0.14), 0.036, lps) +
                        draw_v_string_5(array<u32, 5>(10u, 15u, 18u, 15u, 10u), q, vec2<f32>(-0.18, 0.14), 0.036, 0.008, lps) +
                        draw_v_string_2(array<u32, 2>(14u, 24u), q, vec2<f32>(0.30, 0.14), 0.036, 0.008, lps);
        let minus_ink = draw_vector_char(12u, q, vec2<f32>(-0.76, 0.06), 0.044, lps);
        let plus_ink = draw_vector_char(13u, q, vec2<f32>(0.70, 0.06), 0.044, lps);

        let all_text_ink = max(brand_ink, max(vu_ink, max(signal_ink, max(label_ink, max(minus_ink, plus_ink)))));

        // Composite printed ink layers onto backlit parchment
        faceplate = mix(faceplate, tick_color, tick_mask);
        faceplate = mix(faceplate, arc_color, arc_mask);
        faceplate = mix(faceplate, pct_tick_color, pct_tick_mask);
        faceplate = mix(faceplate, pct_tick_color, pct_arc_mask);
        faceplate = mix(faceplate, db_num_color, db_num_ink * 0.95);
        faceplate = mix(faceplate, vec3<f32>(0.03, 0.02, 0.01), pct_num_ink * 0.90);
        let text_ink_col = select(vec3<f32>(0.03, 0.02, 0.01), vec3<f32>(0.78, 0.05, 0.02), plus_ink > 0.5);
        faceplate = mix(faceplate, text_ink_col, all_text_ink * 0.95);

        // Soft cast drop-shadow from the pivoting needle onto the dial card
        let phi = mix(PHI_MIN, PHI_MAX, vu_val);
        let needle_dir = vec2<f32>(sin(phi), cos(phi));
        let pivot_shadow = pivot + vec2<f32>(0.024, -0.024);
        let d_shadow = sd_line_segment(q, pivot_shadow, pivot_shadow + needle_dir * 1.15);
        let shadow_mask = smoothstep(0.003, 0.022, d_shadow);
        faceplate = mix(faceplate * 0.52, faceplate, shadow_mask);

        // Peak red LED glow spilling onto the top-inner corner of the dial face
        let led_spill_pos = vec2<f32>(select(-0.88, 0.88, is_left), 0.65);
        let is_clipping = smoothstep(0.75, 0.98, max(pk_val, vu_val));
        let spill = exp(-length(q - led_spill_pos) * 3.5) * is_clipping * 0.45;
        faceplate += vec3<f32>(1.0, 0.08, 0.02) * spill;

        // Subtle curved meter cover glass specular streak
        let spec_delta = (q - vec2<f32>(-0.28, 0.25)) * vec2<f32>(1.4, 0.75);
        let glass_spec = exp(-dot(spec_delta, spec_delta) * 10.0) * 0.075;
        faceplate += vec3<f32>(1.2, 1.1, 0.95) * glass_spec;

        color = faceplate;

    } else if (mat_id < 5.5) {
        // =========================================================================
        // 4.0 & 5.0: MECHANICAL NEEDLES & PIVOT HUB CAP
        // =========================================================================
        let is_left = mat_id < 4.5;
        let pivot = vec2<f32>(select(2.70, -2.70, is_left), -1.15);
        let d_hub = length(in.world_pos.xy - pivot);
        if (d_hub < 0.16) {
            // Machined dark bronze/aluminum pivot collar with center jewel highlight
            let jewel = smoothstep_r(0.055, 0.02, d_hub);
            let collar = vec3<f32>(0.04, 0.035, 0.03) * (0.5 + key_diffuse * 0.5);
            color = mix(collar, vec3<f32>(0.75, 0.68, 0.52) * (0.4 + key_diffuse * 0.8), jewel);
        } else {
            let is_tip = d_hub > 1.75;
            let needle_body = vec3<f32>(0.08, 0.06, 0.06);
            let needle_tip = vec3<f32>(0.95, 0.08, 0.05);
            let spec = pow(max(dot(reflect(-key_light_dir, n), v), 0.0), 20.0) * 0.25;
            color = select(needle_body, needle_tip, is_tip) * (0.55 + key_diffuse * 0.45) + vec3<f32>(spec);
        }

    } else if (mat_id < 6.5) {
        // =========================================================================
        // 6.0: MACHINED ALUMINUM MASTER GAIN KNOB
        // =========================================================================
        let knob_al = vec3<f32>(0.22, 0.24, 0.26);
        let knob_ang = atan2(in.world_pos.x, in.world_pos.y + 0.65);
        let fluting = sin(knob_ang * 24.0) * 0.04;
        let spec = pow(max(dot(reflect(-key_light_dir, n), v), 0.0), 32.0) * 0.8;
        // Glowing position indicator notch on the front face of the knob
        let is_front = n.z > 0.8;
        let notch_ang = mix(-0.8, 0.8, clamp((levels.x + levels.y) * 0.5, 0.0, 1.0));
        let notch_dir = vec2<f32>(sin(notch_ang), cos(notch_ang));
        let rel_k = in.world_pos.xy - vec2<f32>(0.0, -0.65);
        let d_notch = sd_line_segment(rel_k, notch_dir * 0.18, notch_dir * 0.44);
        let notch_glow = select(0.0, smoothstep_r(0.025, 0.008, d_notch), is_front);
        color = (knob_al + fluting) * (0.5 + key_diffuse * 0.5) + vec3<f32>(spec) + vec3<f32>(1.0, 0.55, 0.12) * notch_glow * 1.4;

    } else if (mat_id < 8.5) {
        // =========================================================================
        // 7.0 / 8.0: LED STATUS INDICATORS (Power Emerald/Amber, Peak Clipping Red)
        // =========================================================================
        let led_rim = pow(max(dot(n, v), 0.0), 1.5);
        if (in.world_pos.y < -0.2) {
            // Power LED at (-5.15, -0.85): Warm Vintage Emerald Green with Fresnel dome
            let pulse = 0.9 + 0.15 * (levels.x + levels.y);
            color = mix(vec3<f32>(0.02, 0.35, 0.08), vec3<f32>(0.15, 1.0, 0.38), led_rim) * 2.6 * pulse;
        } else {
            // Left/Right Peak Warning LEDs at (±0.95, 1.25): Flash bright ruby red on transients/peaks
            let is_left = in.world_pos.x < 0.0;
            let sig = max(select(levels.w, levels.z, is_left), select(levels.y, levels.x, is_left));
            let is_clipping = smoothstep(0.68, 0.95, sig);
            let unlit_ruby = vec3<f32>(0.18, 0.015, 0.02) * (0.5 + key_diffuse * 0.5);
            let lit_ruby = mix(vec3<f32>(1.0, 0.02, 0.01), vec3<f32>(1.0, 0.55, 0.25), pow(led_rim, 3.0)) * 4.2;
            color = mix(unlit_ruby, lit_ruby, is_clipping);
        }

    } else {
        // =========================================================================
        // 9.0: BLACK ANODIZED HEX MOUNTING SCREWS
        // =========================================================================
        let spec = pow(max(dot(reflect(-key_light_dir, n), v), 0.0), 24.0) * 0.25;
        color = vec3<f32>(0.04, 0.04, 0.05) * (0.5 + key_diffuse * 0.5) + vec3<f32>(spec);
    }

    let tonemapped = aces_tonemap(color);
    return vec4<f32>(tonemapped, 1.0);
}
