// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

// --- Constants ---
const PI: f32 = 3.14159265;
const phi_min: f32 = -0.785398; // -45 degrees
const phi_max: f32 = 0.785398;  // +45 degrees

// --- Analytical Vector Stroke Font ---
fn sdSegment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn draw_vector_char(ch: u32, p: vec2<f32>, origin: vec2<f32>, scale: f32, local_pixel_size: f32) -> f32 {
    let lp = (p - origin) / scale;
    if (lp.x < -0.1 || lp.x > 1.1 || lp.y < -0.1 || lp.y > 2.1) { return 0.0; }
    
    var d = 1e6;
    
    switch ch {
        case 10u { // L
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 0.0)));
        }
        case 15u { // E
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.8, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.0), vec2<f32>(0.6, 1.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 0.0)));
        }
        case 18u { // V
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.5, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.5, 0.0), vec2<f32>(1.0, 2.0)));
        }
        case 19u { // U
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.2), vec2<f32>(0.2, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 0.0), vec2<f32>(0.8, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.0), vec2<f32>(1.0, 0.2)));
            d = min(d, sdSegment(lp, vec2<f32>(1.0, 0.2), vec2<f32>(1.0, 2.0)));
        }
        case 14u { // D
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.9, 1.5)));
            d = min(d, sdSegment(lp, vec2<f32>(0.9, 1.5), vec2<f32>(0.9, 0.5)));
            d = min(d, sdSegment(lp, vec2<f32>(0.9, 0.5), vec2<f32>(0.6, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 16u { // N
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.9, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.9, 0.0), vec2<f32>(0.9, 2.0)));
        }
        case 17u { // O
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.3), vec2<f32>(0.0, 1.7)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.7), vec2<f32>(0.3, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.3, 2.0), vec2<f32>(0.7, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.7, 2.0), vec2<f32>(1.0, 1.7)));
            d = min(d, sdSegment(lp, vec2<f32>(1.0, 1.7), vec2<f32>(1.0, 0.3)));
            d = min(d, sdSegment(lp, vec2<f32>(1.0, 0.3), vec2<f32>(0.7, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.7, 0.0), vec2<f32>(0.3, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.3, 0.0), vec2<f32>(0.0, 0.3)));
        }
        case 20u { // S
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.7), vec2<f32>(0.8, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.0, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 1.1)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.1), vec2<f32>(0.8, 0.9)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.9), vec2<f32>(0.8, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 0.3)));
        }
        case 21u { // I
            d = min(d, sdSegment(lp, vec2<f32>(0.4, 2.0), vec2<f32>(0.4, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.1, 2.0), vec2<f32>(0.7, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.1, 0.0), vec2<f32>(0.7, 0.0)));
        }
        case 22u { // G
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.7), vec2<f32>(0.8, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.0, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.0), vec2<f32>(0.8, 1.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.0), vec2<f32>(0.4, 1.0)));
        }
        case 23u { // A
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.4, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.4, 2.0), vec2<f32>(0.8, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.15, 0.8), vec2<f32>(0.65, 0.8)));
        }
        case 24u { // B
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.5)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.5), vec2<f32>(0.6, 1.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 1.0), vec2<f32>(0.0, 1.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 1.0), vec2<f32>(0.8, 0.5)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.5), vec2<f32>(0.6, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 11u { // R
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.5)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.5), vec2<f32>(0.6, 1.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 1.0), vec2<f32>(0.0, 1.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.4, 1.0), vec2<f32>(0.8, 0.0)));
        }
        case 0u { // 0
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.2), vec2<f32>(0.0, 1.8)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.8), vec2<f32>(0.2, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.8)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.8), vec2<f32>(0.8, 0.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.2), vec2<f32>(0.6, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.2, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 0.0), vec2<f32>(0.0, 0.2)));
        }
        case 1u { // 1
            d = min(d, sdSegment(lp, vec2<f32>(0.1, 1.5), vec2<f32>(0.4, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.4, 2.0), vec2<f32>(0.4, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.1, 0.0), vec2<f32>(0.7, 0.0)));
        }
        case 2u { // 2
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.7), vec2<f32>(0.2, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.7)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.7), vec2<f32>(0.8, 1.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.2), vec2<f32>(0.0, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 0.0)));
        }
        case 3u { // 3
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.8, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.8, 1.1)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.1), vec2<f32>(0.3, 1.1)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.1), vec2<f32>(0.8, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 4u { // 4
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 0.8)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.8), vec2<f32>(0.8, 0.8)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.6, 0.0)));
        }
        case 5u { // 5
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.0, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.0, 1.1)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.1), vec2<f32>(0.7, 1.1)));
            d = min(d, sdSegment(lp, vec2<f32>(0.7, 1.1), vec2<f32>(0.8, 0.9)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.9), vec2<f32>(0.8, 0.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.2), vec2<f32>(0.6, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 6u { // 6
            d = min(d, sdSegment(lp, vec2<f32>(0.7, 2.0), vec2<f32>(0.2, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.0, 1.7)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.7), vec2<f32>(0.0, 0.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.2), vec2<f32>(0.2, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 0.0), vec2<f32>(0.6, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.8, 0.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.2), vec2<f32>(0.8, 0.9)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.9), vec2<f32>(0.6, 1.1)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 1.1), vec2<f32>(0.0, 1.1)));
        }
        case 7u { // 7
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 2.0), vec2<f32>(0.8, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.3, 0.0)));
        }
        case 8u { // 8
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.6, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.8, 1.7)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.7), vec2<f32>(0.8, 1.3)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.3), vec2<f32>(0.6, 1.05)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 1.05), vec2<f32>(0.2, 1.05)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 1.05), vec2<f32>(0.0, 1.3)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.3), vec2<f32>(0.0, 1.7)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.7), vec2<f32>(0.2, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 1.05), vec2<f32>(0.0, 0.8)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.8), vec2<f32>(0.0, 0.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.2), vec2<f32>(0.2, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 0.0), vec2<f32>(0.6, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.8, 0.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.2), vec2<f32>(0.8, 0.8)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.8), vec2<f32>(0.6, 1.05)));
        }
        case 9u { // 9
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.8), vec2<f32>(0.8, 0.3)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 0.3), vec2<f32>(0.6, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.2, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 1.8), vec2<f32>(0.6, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 2.0), vec2<f32>(0.2, 2.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 2.0), vec2<f32>(0.0, 1.8)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.8), vec2<f32>(0.0, 1.1)));
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 1.1), vec2<f32>(0.2, 0.9)));
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 0.9), vec2<f32>(0.8, 0.9)));
        }
        case 12u { // -
            d = min(d, sdSegment(lp, vec2<f32>(0.1, 1.0), vec2<f32>(0.7, 1.0)));
        }
        case 13u { // +
            d = min(d, sdSegment(lp, vec2<f32>(0.1, 1.0), vec2<f32>(0.7, 1.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.4, 0.6), vec2<f32>(0.4, 1.4)));
        }
        case 25u { // %
            d = min(d, sdSegment(lp, vec2<f32>(0.0, 0.0), vec2<f32>(0.8, 2.0)));
            // top circle
            d = min(d, sdSegment(lp, vec2<f32>(0.05, 1.6), vec2<f32>(0.05, 1.9)));
            d = min(d, sdSegment(lp, vec2<f32>(0.05, 1.9), vec2<f32>(0.35, 1.9)));
            d = min(d, sdSegment(lp, vec2<f32>(0.35, 1.9), vec2<f32>(0.35, 1.6)));
            d = min(d, sdSegment(lp, vec2<f32>(0.35, 1.6), vec2<f32>(0.05, 1.6)));
            // bottom circle
            d = min(d, sdSegment(lp, vec2<f32>(0.45, 0.1), vec2<f32>(0.45, 0.4)));
            d = min(d, sdSegment(lp, vec2<f32>(0.45, 0.4), vec2<f32>(0.75, 0.4)));
            d = min(d, sdSegment(lp, vec2<f32>(0.75, 0.4), vec2<f32>(0.75, 0.1)));
            d = min(d, sdSegment(lp, vec2<f32>(0.75, 0.1), vec2<f32>(0.45, 0.1)));
        }
        default {}
    }
    
    let thickness = 0.12;
    let char_d = d * scale;
    let blur = local_pixel_size * 0.75;
    return smoothstep(thickness * scale + blur, thickness * scale - blur, char_d);
}

fn draw_v_string_2(s: array<u32, 2>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, local_pixel_size: f32) -> f32 {
    var intensity = 0.0;
    for (var j = 0u; j < 2u; j++) {
        let char_origin = origin + vec2<f32>(f32(j) * (1.0 * scale + spacing), 0.0);
        intensity = max(intensity, draw_vector_char(s[j], p, char_origin, scale, local_pixel_size));
    }
    return intensity;
}

fn draw_v_string_3(s: array<u32, 3>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, local_pixel_size: f32) -> f32 {
    var intensity = 0.0;
    for (var j = 0u; j < 3u; j++) {
        let char_origin = origin + vec2<f32>(f32(j) * (1.0 * scale + spacing), 0.0);
        intensity = max(intensity, draw_vector_char(s[j], p, char_origin, scale, local_pixel_size));
    }
    return intensity;
}

fn draw_v_string_4(s: array<u32, 4>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, local_pixel_size: f32) -> f32 {
    var intensity = 0.0;
    for (var j = 0u; j < 4u; j++) {
        let char_origin = origin + vec2<f32>(f32(j) * (1.0 * scale + spacing), 0.0);
        intensity = max(intensity, draw_vector_char(s[j], p, char_origin, scale, local_pixel_size));
    }
    return intensity;
}

fn draw_v_string_5(s: array<u32, 5>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, local_pixel_size: f32) -> f32 {
    var intensity = 0.0;
    for (var j = 0u; j < 5u; j++) {
        let char_origin = origin + vec2<f32>(f32(j) * (1.0 * scale + spacing), 0.0);
        intensity = max(intensity, draw_vector_char(s[j], p, char_origin, scale, local_pixel_size));
    }
    return intensity;
}

fn draw_v_string_6(s: array<u32, 6>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32, local_pixel_size: f32) -> f32 {
    var intensity = 0.0;
    for (var j = 0u; j < 6u; j++) {
        let char_origin = origin + vec2<f32>(f32(j) * (1.0 * scale + spacing), 0.0);
        intensity = max(intensity, draw_vector_char(s[j], p, char_origin, scale, local_pixel_size));
    }
    return intensity;
}

// --- SDF Primitives ---
fn sdBox(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

// --- Layout ---
struct MeterLayout {
    scale: f32,
    cols: u32,
    rows: u32,
    spacing_x: f32,
    spacing_y: f32,
};

fn get_meter_layout(num_meters: u32) -> MeterLayout {
    var mlayout: MeterLayout;
    mlayout.scale = 1.0;
    mlayout.cols = 2u;
    mlayout.rows = 1u;
    mlayout.spacing_x = 2.16;
    mlayout.spacing_y = 0.0;
    return mlayout;
}

fn get_meter_center(idx: u32, num_meters: u32, mlayout: MeterLayout) -> vec3<f32> {
    let col = f32(idx % mlayout.cols);
    let grid_w = f32(mlayout.cols - 1u) * mlayout.spacing_x;
    let x = col * mlayout.spacing_x - grid_w * 0.5;
    return vec3<f32>(x, 0.0, 0.0);
}

fn get_needle_angle(i: u32) -> f32 {
    let idx = min(i, 31u);
    let target_idx = select(idx, 0u, idx >= audio.num_channels);
    let vec_idx = target_idx / 4u;
    let comp_idx = target_idx % 4u;
    let v = audio.channels[vec_idx];
    if (comp_idx == 0u) { return v.x; }
    else if (comp_idx == 1u) { return v.y; }
    else if (comp_idx == 2u) { return v.z; }
    else { return v.w; }
}

// Returns (distance, material_id, index)
fn map_scene(p: vec3<f32>) -> vec3<f32> {
    let actual_num = 2u;
    let mlayout = get_meter_layout(actual_num);
    
    var closest_idx = 0u;
    var min_dist_xy = 1e6;
    for (var i = 0u; i < 2u; i++) {
        let center = get_meter_center(i, actual_num, mlayout);
        let dist_xy = length(p.xy - center.xy);
        if (dist_xy < min_dist_xy) {
            min_dist_xy = dist_xy;
            closest_idx = i;
        }
    }
    
    let center = get_meter_center(closest_idx, actual_num, mlayout);
    let q = (p - center) / mlayout.scale;
    
    // Cutout: wider aspect ratio to match reference (roughly 2:1)
    let d_cutout = sdBox(q - vec3<f32>(0.0, 0.0, -0.15), vec3<f32>(0.98, 0.52, 0.25));
    
    if (d_cutout >= 0.0) {
        return vec3<f32>(p.z, 0.0, 0.0);
    } else {
        let d_sidewalls = min(0.98 - abs(q.x), 0.52 - abs(q.y));
        let d_backplate = q.z - (-0.285);
        
        var d_cavity = d_sidewalls;
        var mat_cavity = 1.0;
        if (d_backplate < d_sidewalls) {
            d_cavity = d_backplate;
            mat_cavity = 2.0;
        }
        
        // Needle geometry — tapered
        let angle_val = get_needle_angle(closest_idx);
        let phi = phi_min + angle_val * (phi_max - phi_min);
        let needle_dir = vec3<f32>(sin(phi), cos(phi), 0.0);
        let pivot = vec3<f32>(0.0, -0.55, -0.25);
        
        let needle_len = 0.95;
        
        let pa = q - pivot;
        let ba = needle_dir * needle_len;
        let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
        // Taper: thicker at base, thin at tip
        let needle_thickness = mix(0.012, 0.003, h);
        let d_needle = length(pa - ba * h) - needle_thickness;
        
        // Counterweight: sphere behind pivot
        let cw_center = pivot - needle_dir * 0.12;
        let d_counterweight = length(q - cw_center) - 0.035;
        let d_needle_total = min(d_needle, d_counterweight);
        
        let d_cavity_world = d_cavity * mlayout.scale;
        let d_needle_world = d_needle_total * mlayout.scale;
        
        if (d_cavity_world < d_needle_world) {
            return vec3<f32>(d_cavity_world, mat_cavity, f32(closest_idx));
        } else {
            return vec3<f32>(d_needle_world, 3.0, f32(closest_idx));
        }
    }
}

fn calc_normal(p: vec3<f32>) -> vec3<f32> {
    let eps = 0.0005;
    let d = map_scene(p).x;
    let nx = map_scene(p + vec3<f32>(eps, 0.0, 0.0)).x - d;
    let ny = map_scene(p + vec3<f32>(0.0, eps, 0.0)).x - d;
    let nz = map_scene(p + vec3<f32>(0.0, 0.0, eps)).x - d;
    return normalize(vec3<f32>(nx, ny, nz));
}

fn hash2d(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn noise2d(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2d(i);
    let b = hash2d(i + vec2<f32>(1.0, 0.0));
    let c = hash2d(i + vec2<f32>(0.0, 1.0));
    let d = hash2d(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// --- Draw a label along the arc at a given angle ---
fn draw_arc_label_1(ch: u32, q_xy: vec2<f32>, pivot: vec2<f32>, theta: f32, radius: f32, char_scale: f32, lps: f32) -> f32 {
    let cx = pivot.x + sin(theta) * radius;
    let cy = pivot.y + cos(theta) * radius;
    let half_w = char_scale * 0.5;
    return draw_vector_char(ch, q_xy, vec2<f32>(cx - half_w, cy - char_scale), char_scale, lps);
}

fn draw_arc_label_2(s: array<u32, 2>, q_xy: vec2<f32>, pivot: vec2<f32>, theta: f32, radius: f32, char_scale: f32, lps: f32) -> f32 {
    let cx = pivot.x + sin(theta) * radius;
    let cy = pivot.y + cos(theta) * radius;
    let total_w = 2.0 * char_scale * 1.1;
    return draw_v_string_2(s, q_xy, vec2<f32>(cx - total_w * 0.5, cy - char_scale), char_scale, char_scale * 0.1, lps);
}

fn draw_arc_label_3(s: array<u32, 3>, q_xy: vec2<f32>, pivot: vec2<f32>, theta: f32, radius: f32, char_scale: f32, lps: f32) -> f32 {
    let cx = pivot.x + sin(theta) * radius;
    let cy = pivot.y + cos(theta) * radius;
    let total_w = 3.0 * char_scale * 1.1;
    return draw_v_string_3(s, q_xy, vec2<f32>(cx - total_w * 0.5, cy - char_scale), char_scale, char_scale * 0.1, lps);
}

fn get_panel_color(p_hit: vec3<f32>, q: vec3<f32>, N: vec3<f32>, L: vec3<f32>, V: vec3<f32>, edge_d: f32, pixel_size_Z0: f32, local_pixel_size: f32) -> vec3<f32> {
    // Dark panel — near-black to match reference
    let metal_grain = noise2d(p_hit.xy * vec2<f32>(2000.0, 6.0));
    let panel_col = mix(vec3<f32>(0.02, 0.02, 0.025), vec3<f32>(0.05, 0.05, 0.06), metal_grain * 0.15);
    
    let diff_metal = max(dot(N, L), 0.0);
    let spec_metal = pow(max(dot(reflect(-L, N), V), 0.0), 32.0) * 0.15;
    let dark_panel = panel_col * (diff_metal * 0.4 + 0.06) + vec3<f32>(spec_metal * 0.5);
    
    // Subtle thin bezel highlight right at the edge
    let bezel_glow = smoothstep(0.02, 0.0, abs(edge_d)) * 0.08;
    
    return dark_panel + vec3<f32>(bezel_glow);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    
    var aspect = 1.7777;
    let dy = abs(dpdy(in.uv.y));
    let dx = abs(dpdx(in.uv.x));
    if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }
    
    let p_coord = vec2<f32>(uv.x * aspect, -uv.y);
    
    // Camera: tight framing to fill viewport with the meters
    let t_sway = audio.smooth_time * 0.5;
    let ro = vec3<f32>(sin(t_sway) * 0.015, cos(t_sway * 0.7) * 0.01 - 0.05, 1.45);
    let look_at = vec3<f32>(sin(t_sway) * 0.004, cos(t_sway * 0.7) * 0.003 - 0.05, 0.0);
    
    let cw = normalize(look_at - ro);
    let cu = normalize(cross(cw, vec3<f32>(0.0, 1.0, 0.0)));
    let cv = normalize(cross(cu, cw));
    
    let rd = normalize(p_coord.x * cu + p_coord.y * cv + 1.8 * cw);
    
    let actual_num = 2u;
    let mlayout = get_meter_layout(actual_num);
    let pixel_size_Z0 = abs(dpdy(in.uv.y)) * 2.0;
    let local_pixel_size = pixel_size_Z0 / mlayout.scale;
    
    // Raymarching
    var t = 0.05;
    var hit = false;
    var p_hit = vec3<f32>(0.0);
    var res = vec3<f32>(0.0);
    
    for (var step = 0; step < 80; step++) {
        p_hit = ro + rd * t;
        res = map_scene(p_hit);
        let d = res.x;
        if (d < 0.0008) {
            hit = true;
            break;
        }
        t += d;
        if (t > 10.0) { break; }
    }
    
    // Dark background
    var color = vec3<f32>(0.01, 0.01, 0.015);
    
    if (hit) {
        let mat = res.y;
        let meter_idx = u32(res.z);
        let center = get_meter_center(meter_idx, actual_num, mlayout);
        let q = (p_hit - center) / mlayout.scale;
        let N = calc_normal(p_hit);
        
        let light_pos = vec3<f32>(0.5, 2.0, 3.0);
        let L = normalize(light_pos - p_hit);
        let V = -rd;
        
        let border_x = abs(q.x) - 0.98;
        let border_y = abs(q.y) - 0.52;
        let edge_d = max(border_x, border_y);
        let cutout_edge_blur = local_pixel_size * 1.5;
        
        var mat_color = vec3<f32>(0.0);
        
        if (mat == 0.0) {
            // --- Dark Panel / Chassis ---
            let panel_col_raw = get_panel_color(p_hit, q, N, L, V, edge_d, pixel_size_Z0, local_pixel_size);
            let edge_shadow = smoothstep(-cutout_edge_blur, cutout_edge_blur, edge_d);
            mat_color = mix(vec3<f32>(0.005), panel_col_raw, edge_shadow);
            
        } else {
            if (mat == 1.0) {
                // --- Cavity Walls ---
                let diff = max(dot(N, L), 0.0);
                let ao = smoothstep(-0.3, 0.0, q.z);
                mat_color = vec3<f32>(0.008, 0.006, 0.004) * diff * (0.1 + 0.9 * ao);
                
            } else if (mat == 2.0) {
                // --- Backplate (Dial Face) with WARM AMBER BACKLIGHT ---
                let pivot = vec2<f32>(0.0, -0.55);
                let v_scale = q.xy - pivot;
                let d_scale = length(v_scale);
                let theta = atan2(v_scale.x, v_scale.y);
                
                // Warm amber backlight glow — bright at bottom, darker at top
                let y_norm = clamp((q.y + 0.52) / 1.04, 0.0, 1.0);
                
                // Radial distance from lamp source (bottom center)
                let lamp_pos = vec2<f32>(0.0, -0.52);
                let d_lamp = length(q.xy - lamp_pos);
                let lamp_falloff = exp(-d_lamp * 1.2);
                
                // Rich amber gradient: bright warm gold at bottom → deep amber at top
                let warm_gold = vec3<f32>(1.0, 0.72, 0.18);
                let deep_amber = vec3<f32>(0.90, 0.38, 0.04);
                let dark_edge = vec3<f32>(0.25, 0.08, 0.01);
                
                var back_color = mix(warm_gold, deep_amber, y_norm * 0.7);
                back_color = mix(back_color, dark_edge, smoothstep(0.2, 1.0, y_norm * y_norm));
                
                // Apply lamp glow (brighter near lamp) — more intense
                let lamp_intensity = 0.8 + 0.6 * lamp_falloff;
                back_color *= lamp_intensity;
                
                // Darken edges for lamp falloff vignette  
                let edge_vignette = smoothstep(0.95, 0.5, max(abs(q.x) / 0.98, abs(q.y) / 0.52));
                back_color *= mix(0.3, 1.0, edge_vignette);
                
                // === TICK MARKS — Full VU standard scale ===
                // dB scale: -20, -10, -7, -5, -3, -2, -1, 0, +1, +2, +3
                // Normalized positions along the -45° to +45° arc
                let db_tick_count = 11u;
                let db_vals = array<f32, 11>(0.0, 0.20, 0.32, 0.42, 0.54, 0.62, 0.70, 0.77, 0.84, 0.91, 1.0);
                let db_is_red = array<bool, 11>(false, false, false, false, false, false, false, true, true, true, true);
                let db_is_major = array<bool, 11>(true, true, true, true, true, false, false, true, false, false, true);
                
                var tick_mask = 0.0;
                var tick_color = vec3<f32>(0.05);
                
                let angular_pixel_size = local_pixel_size / max(d_scale, 0.1);
                let tick_w = 0.003;
                let outer_r = 1.10;
                
                for (var ti = 0u; ti < db_tick_count; ti++) {
                    let theta_tick = phi_min + db_vals[ti] * (phi_max - phi_min);
                    let diff_t = abs(theta - theta_tick);
                    let tick_len = select(0.06, 0.10, db_is_major[ti]);
                    let inner_r = outer_r - tick_len;
                    
                    if (d_scale >= inner_r - local_pixel_size && d_scale <= outer_r + local_pixel_size) {
                        let w_mask = smoothstep(tick_w + angular_pixel_size, tick_w - angular_pixel_size, diff_t);
                        let r_mask = smoothstep(outer_r + local_pixel_size, outer_r - local_pixel_size, d_scale) *
                                     smoothstep(inner_r - local_pixel_size, inner_r + local_pixel_size, d_scale);
                        let current_tick_mask = w_mask * r_mask;
                        
                        if (current_tick_mask > 0.0) {
                            let current_tick_color = select(vec3<f32>(0.03, 0.02, 0.01), vec3<f32>(0.75, 0.05, 0.02), db_is_red[ti]);
                            tick_color = mix(tick_color, current_tick_color, current_tick_mask);
                            tick_mask = max(tick_mask, current_tick_mask);
                        }
                    }
                }
                
                // === Baseline Arc (dB scale) ===
                var arc_mask = 0.0;
                var arc_color = vec3<f32>(0.03, 0.02, 0.01);
                let arc_r = 1.05;
                if (d_scale >= arc_r - 0.008 && d_scale <= arc_r + 0.008 && theta >= phi_min - local_pixel_size && theta <= phi_max + local_pixel_size) {
                    let rad_diff = abs(d_scale - arc_r);
                    let r_mask = smoothstep(0.006 + local_pixel_size, 0.006 - local_pixel_size, rad_diff);
                    let end_mask = smoothstep(phi_min - local_pixel_size, phi_min + local_pixel_size, theta) *
                                   smoothstep(phi_max + local_pixel_size, phi_max - local_pixel_size, theta);
                    arc_mask = r_mask * end_mask;
                    // Red section for positive dB
                    let is_red_arc = theta > (phi_min + 0.77 * (phi_max - phi_min));
                    arc_color = select(vec3<f32>(0.03, 0.02, 0.01), vec3<f32>(0.75, 0.05, 0.02), is_red_arc);
                }
                
                // === Percentage scale: smaller ticks and arc below the dB arc ===
                // Percent values: 0, 20, 40, 60, 80, 100
                let pct_vals = array<f32, 6>(0.0, 0.20, 0.42, 0.62, 0.77, 0.91);
                let pct_outer = 0.96;
                let pct_inner = 0.90;
                let pct_arc_r = 0.93;
                
                var pct_tick_mask = 0.0;
                let pct_tick_color = vec3<f32>(0.03, 0.02, 0.01);
                for (var pi = 0u; pi < 6u; pi++) {
                    let theta_pct = phi_min + pct_vals[pi] * (phi_max - phi_min);
                    let diff_p = abs(theta - theta_pct);
                    if (d_scale >= pct_inner - local_pixel_size && d_scale <= pct_outer + local_pixel_size) {
                        let w_m = smoothstep(0.003 + angular_pixel_size, 0.003 - angular_pixel_size, diff_p);
                        let r_m = smoothstep(pct_outer + local_pixel_size, pct_outer - local_pixel_size, d_scale) *
                                  smoothstep(pct_inner - local_pixel_size, pct_inner + local_pixel_size, d_scale);
                        pct_tick_mask = max(pct_tick_mask, w_m * r_m);
                    }
                }
                
                // Percentage baseline arc
                var pct_arc_mask = 0.0;
                if (d_scale >= pct_arc_r - 0.006 && d_scale <= pct_arc_r + 0.006 && theta >= phi_min - local_pixel_size && theta <= phi_max + local_pixel_size) {
                    let pct_rad_diff = abs(d_scale - pct_arc_r);
                    let pct_r_mask = smoothstep(0.004 + local_pixel_size, 0.004 - local_pixel_size, pct_rad_diff);
                    let pct_end = smoothstep(phi_min - local_pixel_size, phi_min + local_pixel_size, theta) *
                                  smoothstep(phi_max + local_pixel_size, phi_max - local_pixel_size, theta);
                    pct_arc_mask = pct_r_mask * pct_end;
                }
                
                // === dB number labels along the outer arc ===
                let label_r = 1.20;
                let lbl_scale = 0.032;
                let lps = local_pixel_size;
                
                // -20
                let theta_20 = phi_min + 0.0 * (phi_max - phi_min);
                var db_num_ink = draw_arc_label_2(array<u32, 2>(2u, 0u), q.xy, pivot, theta_20, label_r, lbl_scale, lps);
                // -10
                let theta_10 = phi_min + 0.20 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_2(array<u32, 2>(1u, 0u), q.xy, pivot, theta_10, label_r, lbl_scale, lps));
                // -7
                let theta_7 = phi_min + 0.32 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(7u, q.xy, pivot, theta_7, label_r, lbl_scale, lps));
                // -5
                let theta_5 = phi_min + 0.42 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(5u, q.xy, pivot, theta_5, label_r, lbl_scale, lps));
                // -3
                let theta_3 = phi_min + 0.54 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(3u, q.xy, pivot, theta_3, label_r, lbl_scale, lps));
                // -2
                let theta_2 = phi_min + 0.62 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(2u, q.xy, pivot, theta_2, label_r, lbl_scale, lps));
                // -1
                let theta_1 = phi_min + 0.70 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(1u, q.xy, pivot, theta_1, label_r, lbl_scale, lps));
                // 0
                let theta_0 = phi_min + 0.77 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(0u, q.xy, pivot, theta_0, label_r, lbl_scale, lps));
                // +1
                let theta_p1 = phi_min + 0.84 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(1u, q.xy, pivot, theta_p1, label_r, lbl_scale, lps));
                // +2
                let theta_p2 = phi_min + 0.91 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(2u, q.xy, pivot, theta_p2, label_r, lbl_scale, lps));
                // +3
                let theta_p3 = phi_min + 1.0 * (phi_max - phi_min);
                db_num_ink = max(db_num_ink, draw_arc_label_1(3u, q.xy, pivot, theta_p3, label_r, lbl_scale, lps));
                
                // Determine if the dB label is in the red zone
                // We check which label we're closest to by checking if the pixel is in the red region
                let pixel_theta_norm = (theta - phi_min) / (phi_max - phi_min);
                let db_num_is_red = pixel_theta_norm > 0.80;
                let db_num_color = select(vec3<f32>(0.03, 0.02, 0.01), vec3<f32>(0.75, 0.05, 0.02), db_num_is_red);
                
                // === Percentage number labels ===
                let pct_label_r = 0.84;
                let pct_lbl_scale = 0.026;
                
                // 0
                var pct_num_ink = draw_arc_label_1(0u, q.xy, pivot, phi_min, pct_label_r, pct_lbl_scale, lps);
                // 20
                let pct_theta_20 = phi_min + 0.20 * (phi_max - phi_min);
                pct_num_ink = max(pct_num_ink, draw_arc_label_2(array<u32, 2>(2u, 0u), q.xy, pivot, pct_theta_20, pct_label_r, pct_lbl_scale, lps));
                // 40
                let pct_theta_40 = phi_min + 0.42 * (phi_max - phi_min);
                pct_num_ink = max(pct_num_ink, draw_arc_label_2(array<u32, 2>(4u, 0u), q.xy, pivot, pct_theta_40, pct_label_r, pct_lbl_scale, lps));
                // 60
                let pct_theta_60 = phi_min + 0.62 * (phi_max - phi_min);
                pct_num_ink = max(pct_num_ink, draw_arc_label_2(array<u32, 2>(6u, 0u), q.xy, pivot, pct_theta_60, pct_label_r, pct_lbl_scale, lps));
                // 80
                let pct_theta_80 = phi_min + 0.77 * (phi_max - phi_min);
                pct_num_ink = max(pct_num_ink, draw_arc_label_2(array<u32, 2>(8u, 0u), q.xy, pivot, pct_theta_80, pct_label_r, pct_lbl_scale, lps));
                // 100%
                let pct_theta_100 = phi_min + 0.91 * (phi_max - phi_min);
                pct_num_ink = max(pct_num_ink, draw_arc_label_3(array<u32, 3>(1u, 0u, 0u), q.xy, pivot, pct_theta_100, pct_label_r, pct_lbl_scale, lps));
                // % symbol at far right
                pct_num_ink = max(pct_num_ink, draw_arc_label_1(25u, q.xy, pivot, phi_max, pct_label_r - 0.04, pct_lbl_scale, lps));
                
                // === String Labels ===
                let denon_ink = draw_v_string_5(array<u32, 5>(14u, 15u, 16u, 17u, 16u), q.xy, vec2(-0.62, -0.38), 0.048, 0.014, lps);
                let vu_ink = draw_v_string_2(array<u32, 2>(18u, 19u), q.xy, vec2(0.40, -0.38), 0.048, 0.014, lps);
                let signal_ink = draw_v_string_6(array<u32, 6>(20u, 21u, 22u, 16u, 23u, 10u), q.xy, vec2(-0.25, -0.16), 0.038, 0.010, lps);
                
                // L/R - LEVEL dB header
                let is_right = (meter_idx % 2u) == 1u;
                let first_char = select(10u, 11u, is_right);
                
                let label_ink = draw_vector_char(first_char, q.xy, vec2(-0.48, 0.30), 0.038, lps) +
                                 draw_vector_char(12u, q.xy, vec2(-0.34, 0.30), 0.038, lps) +
                                 draw_v_string_5(array<u32, 5>(10u, 15u, 18u, 15u, 10u), q.xy, vec2(-0.20, 0.30), 0.038, 0.008, lps) +
                                 draw_v_string_2(array<u32, 2>(14u, 24u), q.xy, vec2(0.30, 0.30), 0.038, 0.008, lps);
                
                // - and + signs at left/right extremes
                let minus_ink = draw_vector_char(12u, q.xy, vec2(-0.78, 0.06), 0.045, lps);
                let plus_ink = draw_vector_char(13u, q.xy, vec2(0.72, 0.06), 0.045, lps);
                
                // Composite ink layers
                let all_text_ink = max(denon_ink, max(vu_ink, max(signal_ink, max(label_ink, max(minus_ink, plus_ink)))));
                let all_db_ink = db_num_ink;
                let all_pct_ink = pct_num_ink;
                
                // Compose the faceplate
                var faceplate = back_color;
                
                // Apply tick marks
                faceplate = mix(faceplate, tick_color, tick_mask);
                // Apply baseline arc
                faceplate = mix(faceplate, arc_color, arc_mask);
                // Apply percentage ticks and arc
                faceplate = mix(faceplate, pct_tick_color, pct_tick_mask);
                faceplate = mix(faceplate, pct_tick_color, pct_arc_mask);
                
                // Apply dB numbers
                faceplate = mix(faceplate, db_num_color, all_db_ink * 0.95);
                // Apply percentage numbers
                faceplate = mix(faceplate, vec3<f32>(0.03, 0.02, 0.01), all_pct_ink * 0.9);
                
                // Apply text labels (dark ink)
                let is_red_text = (plus_ink > 0.5);
                let text_ink_col = select(vec3<f32>(0.03, 0.02, 0.01), vec3<f32>(0.75, 0.05, 0.02), is_red_text);
                faceplate = mix(faceplate, text_ink_col, all_text_ink * 0.95);
                
                // Needle shadow
                let angle_val = get_needle_angle(meter_idx);
                let phi = phi_min + angle_val * (phi_max - phi_min);
                let needle_dir = vec2<f32>(sin(phi), cos(phi));
                
                let pivot_shadow = pivot + vec2<f32>(0.025, -0.025);
                let pa_s = q.xy - pivot_shadow;
                let ba_s = needle_dir * 0.95;
                let h_s = clamp(dot(pa_s, ba_s) / dot(ba_s, ba_s), 0.0, 1.0);
                let d_shadow = length(pa_s - ba_s * h_s);
                
                let shadow_thickness = mix(0.015, 0.005, h_s);
                let shadow_mask = smoothstep(0.003, shadow_thickness + 0.01, d_shadow);
                faceplate = mix(faceplate * 0.5, faceplate, shadow_mask);
                
                mat_color = faceplate;
                
            } else if (mat == 3.0) {
                // --- Needle with taper and counterweight ---
                let angle_val = get_needle_angle(meter_idx);
                let phi = phi_min + angle_val * (phi_max - phi_min);
                let needle_dir = vec2<f32>(sin(phi), cos(phi));
                let pivot = vec2<f32>(0.0, -0.55);
                
                let pa_n = q.xy - pivot;
                let ba_n = needle_dir * 0.95;
                let h_n = clamp(dot(pa_n, ba_n) / dot(ba_n, ba_n), 0.0, 1.0);
                let dist_from_pivot = h_n * 0.95;
                
                // Is this the counterweight (behind pivot)?
                let cw_center = pivot - needle_dir * 0.12;
                let d_cw = length(q.xy - cw_center);
                let is_counterweight = d_cw < 0.04;
                
                let is_tip = dist_from_pivot > 0.78 && !is_counterweight;
                
                // Metallic highlight along needle length
                let cross_dist = abs(dot(pa_n, vec2<f32>(-needle_dir.y, needle_dir.x)));
                let metallic_highlight = smoothstep(0.008, 0.002, cross_dist) * 0.15;
                
                var needle_col = vec3<f32>(0.02, 0.02, 0.025);
                if (is_tip) {
                    needle_col = vec3<f32>(0.80, 0.05, 0.02);
                }
                if (is_counterweight) {
                    needle_col = vec3<f32>(0.04, 0.04, 0.05);
                }
                
                let diff = max(dot(N, L), 0.0);
                mat_color = needle_col * (diff * 0.6 + 0.15) + vec3<f32>(metallic_highlight);
            }
            
            // Blend with panel at the edge
            if (edge_d > -cutout_edge_blur) {
                let panel_col_raw = get_panel_color(p_hit, q, N, L, V, edge_d, pixel_size_Z0, local_pixel_size);
                let edge_shadow = smoothstep(-cutout_edge_blur, cutout_edge_blur, edge_d);
                mat_color = mix(mat_color, panel_col_raw, edge_shadow);
            }
        }
        
        color = mat_color;
        
        // --- Glass Layer: only within meter cutout, subtle highlight ---
        if (p_hit.z < -0.01 && abs(q.x) < 0.95 && abs(q.y) < 0.50) {
            let q_glass = q.xy;
            // Elongated highlight streak in upper-left quadrant
            let spec_center = vec2<f32>(-0.25, 0.20);
            let spec_delta = q_glass - spec_center;
            let spec_stretched = vec2<f32>(spec_delta.x * 1.5, spec_delta.y * 0.7);
            let spec_dist = length(spec_stretched);
            let spec = exp(-spec_dist * spec_dist * 12.0) * 0.06;
            
            // Subtle warm-tinted Fresnel at the glass edges
            let edge_fresnel = smoothstep(0.3, 0.98, max(abs(q_glass.x) / 0.98, abs(q_glass.y) / 0.52)) * 0.03;
            
            let glass_reflection = vec3<f32>(spec * 1.2, spec * 1.1, spec * 0.9) + vec3<f32>(edge_fresnel * 0.8, edge_fresnel * 0.6, edge_fresnel * 0.3);
            color = color * 0.97 + glass_reflection;
        }
    }
    
    // --- Post Processing ---
    // Subtle vignette
    let vr = length(uv);
    color *= smoothstep(2.0, 0.6, vr);
    
    // Very subtle scanlines
    let scanline = 0.98 + 0.02 * cos(in.uv.y * 2.0 * PI * 240.0);
    color *= scanline;
    
    // ACES-ish tonemap
    var final_col = aces_tonemap(color);
    final_col = max(final_col, vec3<f32>(0.0));
    
    return vec4<f32>(final_col, 1.0);
}
