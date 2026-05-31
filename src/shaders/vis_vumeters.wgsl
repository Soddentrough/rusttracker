// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

// --- Constants ---
const phi_min: f32 = -0.785398; // -45 degrees
const phi_max: f32 = 0.785398;  // +45 degrees

// --- Analytical Vector Stroke Font ---
fn sdSegment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn draw_vector_char(ch: u32, p: vec2<f32>, origin: vec2<f32>, scale: f32) -> f32 {
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
            d = min(d, sdSegment(lp, vec2<f32>(0.2, 0.5), vec2<f32>(0.6, 1.5)));
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
            d = min(d, sdSegment(lp, vec2<f32>(0.8, 2.0), vec2<f32>(0.4, 1.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.4, 1.2), vec2<f32>(0.7, 1.2)));
            d = min(d, sdSegment(lp, vec2<f32>(0.7, 1.2), vec2<f32>(0.9, 0.9)));
            d = min(d, sdSegment(lp, vec2<f32>(0.9, 0.9), vec2<f32>(0.9, 0.3)));
            d = min(d, sdSegment(lp, vec2<f32>(0.9, 0.3), vec2<f32>(0.6, 0.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.6, 0.0), vec2<f32>(0.0, 0.0)));
        }
        case 12u { // -
            d = min(d, sdSegment(lp, vec2<f32>(0.1, 1.0), vec2<f32>(0.7, 1.0)));
        }
        case 13u { // +
            d = min(d, sdSegment(lp, vec2<f32>(0.1, 1.0), vec2<f32>(0.7, 1.0)));
            d = min(d, sdSegment(lp, vec2<f32>(0.4, 0.6), vec2<f32>(0.4, 1.4)));
        }
        default {}
    }
    
    let thickness = 0.06;
    let world_d = d * scale;
    let blur = 0.0015;
    return smoothstep(thickness * scale + blur, thickness * scale - blur, world_d);
}

fn draw_v_string_2(s: array<u32, 2>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32) -> f32 {
    var intensity = 0.0;
    for (var j = 0u; j < 2u; j++) {
        let char_origin = origin + vec2<f32>(f32(j) * (1.0 * scale + spacing), 0.0);
        intensity = max(intensity, draw_vector_char(s[j], p, char_origin, scale));
    }
    return intensity;
}

fn draw_v_string_5(s: array<u32, 5>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32) -> f32 {
    var intensity = 0.0;
    for (var j = 0u; j < 5u; j++) {
        let char_origin = origin + vec2<f32>(f32(j) * (1.0 * scale + spacing), 0.0);
        intensity = max(intensity, draw_vector_char(s[j], p, char_origin, scale));
    }
    return intensity;
}

fn draw_v_string_6(s: array<u32, 6>, p: vec2<f32>, origin: vec2<f32>, scale: f32, spacing: f32) -> f32 {
    var intensity = 0.0;
    for (var j = 0u; j < 6u; j++) {
        let char_origin = origin + vec2<f32>(f32(j) * (1.0 * scale + spacing), 0.0);
        intensity = max(intensity, draw_vector_char(s[j], p, char_origin, scale));
    }
    return intensity;
}

// --- SDF Primitives ---
fn sdBox(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

// --- Dynamic Grid Layout Structures ---
struct MeterLayout {
    scale: f32,
    cols: u32,
    rows: u32,
    spacing_x: f32,
    spacing_y: f32,
};

fn get_meter_layout(num_meters: u32) -> MeterLayout {
    var mlayout: MeterLayout;
    if (num_meters <= 2u) {
        mlayout.scale = 1.0;
        mlayout.cols = 2u;
        mlayout.rows = 1u;
        mlayout.spacing_x = 2.16;
        mlayout.spacing_y = 0.0;
    } else if (num_meters <= 4u) {
        mlayout.scale = 0.65;
        mlayout.cols = 2u;
        mlayout.rows = 2u;
        mlayout.spacing_x = 1.40;
        mlayout.spacing_y = 0.95;
    } else if (num_meters <= 6u) {
        mlayout.scale = 0.55;
        mlayout.cols = 3u;
        mlayout.rows = 2u;
        mlayout.spacing_x = 1.15;
        mlayout.spacing_y = 0.85;
    } else {
        mlayout.scale = 0.42;
        mlayout.cols = 4u;
        mlayout.rows = 2u;
        mlayout.spacing_x = 0.90;
        mlayout.spacing_y = 0.70;
    }
    return mlayout;
}

fn get_meter_center(idx: u32, num_meters: u32, mlayout: MeterLayout) -> vec3<f32> {
    let col = f32(idx % mlayout.cols);
    let row = f32(idx / mlayout.cols);
    
    // Center the grid around the origin
    let grid_w = f32(mlayout.cols - 1u) * mlayout.spacing_x;
    let grid_h = f32(mlayout.rows - 1u) * mlayout.spacing_y;
    
    let x = col * mlayout.spacing_x - grid_w * 0.5;
    let y = (f32(mlayout.rows - 1u) - row) * mlayout.spacing_y - grid_h * 0.5;
    
    return vec3<f32>(x, y, 0.0);
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
    
    // Find the closest window index in the grid
    var closest_idx = 0u;
    var min_dist_xy = 1e6;
    for (var i = 0u; i < 8u; i++) {
        if (i >= actual_num) { break; }
        let center = get_meter_center(i, actual_num, mlayout);
        let dist_xy = length(p.xy - center.xy);
        if (dist_xy < min_dist_xy) {
            min_dist_xy = dist_xy;
            closest_idx = i;
        }
    }
    
    let center = get_meter_center(closest_idx, actual_num, mlayout);
    let q = (p - center) / mlayout.scale;
    
    // Cutout check (extends from Z = -0.40 to Z = +0.10 in local coords)
    let d_cutout = sdBox(q - vec3<f32>(0.0, 0.0, -0.15), vec3<f32>(0.96, 0.66, 0.25));
    
    if (d_cutout >= 0.0) {
        // Outside the window: hit the console panel plane
        return vec3<f32>(p.z, 0.0, 0.0);
    } else {
        // Inside the window cutout cavity (in local coords)
        let d_sidewalls = min(0.96 - abs(q.x), 0.66 - abs(q.y));
        let d_backplate = q.z - (-0.285);
        
        var d_cavity = d_sidewalls;
        var mat_cavity = 1.0; // side walls
        if (d_backplate < d_sidewalls) {
            d_cavity = d_backplate;
            mat_cavity = 2.0; // backplate
        }
        
        // Needle geometry
        let angle_val = get_needle_angle(closest_idx);
        let phi = phi_min + angle_val * (phi_max - phi_min);
        let needle_dir = vec3<f32>(sin(phi), cos(phi), 0.0);
        let pivot = vec3<f32>(0.0, -0.65, -0.25);
        
        let needle_len = 1.05;
        let needle_thickness = 0.006;
        
        let pa = q - pivot;
        let ba = needle_dir * needle_len;
        let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
        let d_needle = length(pa - ba * h) - needle_thickness;
        
        // Scale distances back to world coordinates
        let d_cavity_world = d_cavity * mlayout.scale;
        let d_needle_world = d_needle * mlayout.scale;
        
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    
    var aspect = 1.7777;
    let dy = abs(dpdy(in.uv.y));
    let dx = abs(dpdx(in.uv.x));
    if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }
    
    let p_coord = vec2<f32>(uv.x * aspect, -uv.y);
    
    // Add subtle, realistic camera movement (breathing/idle)
    let t_sway = audio.smooth_time * 0.5;
    let ro = vec3<f32>(sin(t_sway) * 0.04, cos(t_sway * 0.7) * 0.03, 2.3);
    let look_at = vec3<f32>(sin(t_sway) * 0.01, cos(t_sway * 0.7) * 0.008, 0.0);
    
    let cw = normalize(look_at - ro);
    let cu = normalize(cross(cw, vec3<f32>(0.0, 1.0, 0.0)));
    let cv = normalize(cross(cu, cw));
    
    let rd = normalize(p_coord.x * cu + p_coord.y * cv + 1.6 * cw);
    
    let actual_num = 2u;
    let mlayout = get_meter_layout(actual_num);
    
    // Raymarching
    var t = 0.05;
    var hit = false;
    var p_hit = vec3<f32>(0.0);
    var res = vec3<f32>(0.0);
    
    for (var step = 0; step < 45; step++) {
        p_hit = ro + rd * t;
        res = map_scene(p_hit);
        let d = res.x;
        if (d < 0.0008) {
            hit = true;
            break;
        }
        t += d;
        if (t > 4.5) { break; }
    }
    
    var color = vec3<f32>(0.02, 0.02, 0.035); // Ambient background if miss
    
    if (hit) {
        let mat = res.y;
        let meter_idx = u32(res.z);
        let center = get_meter_center(meter_idx, actual_num, mlayout);
        let q = (p_hit - center) / mlayout.scale;
        let N = calc_normal(p_hit);
        
        let light_pos = vec3<f32>(1.5, 2.5, 3.5);
        let L = normalize(light_pos - p_hit);
        let V = -rd;
        
        if (mat == 0.0) {
            // --- Console Panel / Chassis ---
            let border_x = abs(q.x) - 0.96;
            let border_y = abs(q.y) - 0.66;
            let edge_d = max(border_x, border_y);
            
            // Perturb normal for Chrome Bevel around the cutouts
            var normal_perturbed = N;
            var is_chrome = false;
            if (edge_d > 0.0 && edge_d < 0.05) {
                is_chrome = true;
                let chamfer_w = 0.025;
                if (edge_d < chamfer_w) {
                    let factor = edge_d / chamfer_w;
                    normal_perturbed = normalize(N + vec3<f32>(sign(q.x) * (1.0 - factor) * 0.5, sign(q.y) * (1.0 - factor) * 0.5, 0.0));
                } else {
                    let factor = (edge_d - chamfer_w) / chamfer_w;
                    normal_perturbed = normalize(N - vec3<f32>(sign(q.x) * (1.0 - factor) * 0.3, sign(q.y) * (1.0 - factor) * 0.3, 0.0));
                }
            }
            
            let abs_x = abs(p_hit.x);
            if (abs_x > 2.05) {
                // Wood panel side trim
                let edge_factor = clamp((abs_x - 2.05) / 0.5, 0.0, 1.0);
                let wood_N = normalize(N + vec3<f32>(sign(p_hit.x) * edge_factor * 0.25, 0.0, 0.0));
                let wood_grain = noise2d(p_hit.yx * vec2<f32>(12.0, 1.2)) * 0.6 + noise2d(p_hit.yx * vec2<f32>(35.0, 3.5)) * 0.25;
                let wood_col = mix(vec3<f32>(0.14, 0.06, 0.03), vec3<f32>(0.32, 0.15, 0.07), wood_grain);
                
                let diff_wood = max(dot(wood_N, L), 0.0);
                let spec_wood = pow(max(dot(reflect(-L, wood_N), V), 0.0), 80.0) * 0.45;
                color = wood_col * (diff_wood * 0.8 + 0.15) + vec3<f32>(spec_wood);
            } else if (abs_x > 2.0) {
                // Divider strip
                let divider_N = normalize(N + vec3<f32>(sign(p_hit.x) * 0.6, 0.0, 0.0));
                let diff_div = max(dot(divider_N, L), 0.0);
                let spec_div = pow(max(dot(reflect(-L, divider_N), V), 0.0), 64.0) * 0.8;
                color = vec3<f32>(0.85, 0.85, 0.90) * (diff_div * 0.6 + 0.3) + vec3<f32>(spec_div);
            } else if (is_chrome) {
                // Shiny chrome bezel
                let bezel_col = vec3<f32>(0.85, 0.85, 0.90);
                let bezel_spec = pow(max(dot(reflect(-L, normal_perturbed), V), 0.0), 64.0) * 0.9;
                color = bezel_col * (max(dot(normal_perturbed, L), 0.0) * 0.5 + 0.3) + vec3<f32>(bezel_spec);
            } else {
                // Brushed metallic panel face
                let metal_grain = noise2d(p_hit.xy * vec2<f32>(2000.0, 6.0));
                let panel_col = mix(vec3<f32>(0.08, 0.08, 0.09), vec3<f32>(0.14, 0.14, 0.16), metal_grain * 0.18);
                
                let edge_highlight = smoothstep(0.012, 0.001, abs(edge_d)) * pow(max(dot(N, normalize(vec3<f32>(1.0, 1.0, 1.0))), 0.0), 30.0) * 0.45;
                let diff_metal = max(dot(normal_perturbed, L), 0.0);
                let spec_metal = pow(max(dot(reflect(-L, normal_perturbed), V), 0.0), 24.0) * 0.3 * (1.0 - metal_grain * 0.15);
                
                color = panel_col * (diff_metal + 0.08) + vec3<f32>(spec_metal) + vec3<f32>(edge_highlight);
            }
            
            // Draw grooved screws at corners
            let screw_pos = abs(q.xy) - vec2<f32>(1.02, 0.72);
            let d_screw_local = length(screw_pos) - 0.028;
            if (d_screw_local < 0.0 && abs_x <= 2.0) {
                let screw_uv = screw_pos;
                let rot_uv = vec2<f32>(screw_uv.x * 0.707 - screw_uv.y * 0.707, screw_uv.x * 0.707 + screw_uv.y * 0.707);
                let slot = max(abs(rot_uv.x) - 0.004, abs(rot_uv.y) - 0.018);
                let screw_N = normalize(N + vec3<f32>(-screw_pos * 8.0, 0.0));
                
                if (slot < 0.0) {
                    color = vec3<f32>(0.02, 0.02, 0.03);
                } else {
                    let diff_screw = max(dot(screw_N, L), 0.0);
                    let spec_screw = pow(max(dot(reflect(-L, screw_N), V), 0.0), 32.0) * 0.6;
                    color = vec3<f32>(0.7, 0.7, 0.75) * diff_screw + vec3<f32>(spec_screw);
                }
                let ao_screw = smoothstep(-0.028, 0.0, d_screw_local);
                color *= ao_screw;
            }
            
        } else if (mat == 1.0) {
            // --- Cavity Walls ---
            let diff = max(dot(N, L), 0.0);
            let ao = smoothstep(-0.3, 0.0, q.z);
            color = vec3<f32>(0.015, 0.015, 0.018) * diff * (0.1 + 0.9 * ao);
            
        } else if (mat == 2.0) {
            // --- Backplate (Dial Face) ---
            let y_norm = (q.y + 0.6) / 1.2;
            let glow = exp(-y_norm * 1.6);
            let back_color = mix(vec3<f32>(1.0, 0.88, 0.65), vec3<f32>(1.0, 0.48, 0.02), glow * 0.9);
            
            let pivot = vec2<f32>(0.0, -0.65);
            let v_scale = q.xy - pivot;
            let d_scale = length(v_scale);
            let theta = atan2(v_scale.x, v_scale.y);
            
            var tick_mask = 0.0;
            var tick_color = vec3<f32>(0.05);
            
            let tick_vals = array<f32, 11>(0.0, 0.25, 0.35, 0.45, 0.57, 0.63, 0.69, 0.75, 0.83, 0.91, 1.0);
            let tick_red = array<bool, 11>(false, false, false, false, false, false, false, true, true, true, true);
            
            for (var t = 0u; t < 11u; t++) {
                let theta_tick = phi_min + tick_vals[t] * (phi_max - phi_min);
                let diff_t = abs(theta - theta_tick);
                let is_major = (t == 0u || t == 7u || t == 10u);
                let tick_min = select(1.23, 1.20, is_major);
                
                if (diff_t < 0.0035 && d_scale >= tick_min && d_scale <= 1.30) {
                    tick_mask = 1.0;
                    tick_color = select(vec3<f32>(0.05), vec3<f32>(0.85, 0.05, 0.05), tick_red[t]);
                }
            }
            
            var arc_mask = 0.0;
            var arc_color = vec3<f32>(0.05);
            if (d_scale >= 1.25 && d_scale <= 1.265 && theta >= phi_min && theta <= phi_max) {
                arc_mask = 1.0;
                let is_red_arc = theta > (phi_min + 0.75 * (phi_max - phi_min));
                arc_color = select(vec3<f32>(0.05), vec3<f32>(0.85, 0.05, 0.05), is_red_arc);
            }
            
            // 3. String Labels using clean vector stroke font
            let denon_ink = draw_v_string_5(array<u32, 5>(14u, 15u, 16u, 17u, 16u), q.xy, vec2(-0.78, -0.38), 0.040, 0.012);
            let vu_ink = draw_v_string_2(array<u32, 2>(18u, 19u), q.xy, vec2(0.55, -0.38), 0.040, 0.012);
            let signal_ink = draw_v_string_6(array<u32, 6>(20u, 21u, 22u, 16u, 23u, 10u), q.xy, vec2(-0.25, -0.12), 0.032, 0.010);
            
            // L/R LEVEL dB
            let is_right = (meter_idx % 2u) == 1u;
            let first_char = select(10u, 11u, is_right);
            
            let label_ink = draw_vector_char(first_char, q.xy, vec2(-0.45, 0.35), 0.035) +
                             draw_vector_char(12u, q.xy, vec2(-0.33, 0.35), 0.035) +
                             draw_v_string_5(array<u32, 5>(10u, 15u, 18u, 15u, 10u), q.xy, vec2(-0.21, 0.35), 0.035, 0.008) +
                             draw_v_string_2(array<u32, 2>(14u, 24u), q.xy, vec2(0.24, 0.35), 0.035, 0.008);
            
            let minus_ink = draw_vector_char(12u, q.xy, vec2(-0.84, 0.08), 0.045);
            let plus_ink = draw_vector_char(13u, q.xy, vec2(0.78, 0.08), 0.045);
            
            // Numbers next to ticks
            let num1 = draw_vector_char(12u, q.xy, vec2(-0.73, 0.16), 0.030) +
                       draw_vector_char(2u, q.xy, vec2(-0.63, 0.16), 0.030) +
                       draw_vector_char(0u, q.xy, vec2(-0.53, 0.16), 0.030); // -20
            
            let num2 = draw_vector_char(0u, q.xy, vec2(0.33, 0.24), 0.030); // 0
            
            let num3 = draw_vector_char(13u, q.xy, vec2(0.62, 0.12), 0.030) +
                       draw_vector_char(3u, q.xy, vec2(0.72, 0.12), 0.030); // +3
            
            let ink_mask = max(tick_mask, max(arc_mask, max(denon_ink, max(vu_ink, max(signal_ink, max(label_ink, max(minus_ink, max(plus_ink, max(num1, max(num2, num3))))))))));
            
            let is_red_ink = (plus_ink > 0.5) || (num3 > 0.5) || (tick_mask > 0.5 && tick_color.x > 0.5) || (arc_mask > 0.5 && arc_color.x > 0.5);
            let final_ink_col = select(vec3<f32>(0.05, 0.05, 0.07), vec3<f32>(0.85, 0.05, 0.05), is_red_ink);
            
            var faceplate_col = mix(back_color, final_ink_col, ink_mask * 0.95);
            
            // 4. Needle shadow cast onto faceplate
            let angle_val = get_needle_angle(meter_idx);
            let phi = phi_min + angle_val * (phi_max - phi_min);
            let needle_dir = vec2<f32>(sin(phi), cos(phi));
            
            let pivot_shadow = pivot + vec2<f32>(0.035, -0.035);
            let pa_s = q.xy - pivot_shadow;
            let ba_s = needle_dir * 1.05;
            let h_s = clamp(dot(pa_s, ba_s) / dot(ba_s, ba_s), 0.0, 1.0);
            let d_shadow = length(pa_s - ba_s * h_s);
            
            let shadow_blur = mix(0.005, 0.025, h_s);
            let shadow_mask = smoothstep(0.004, shadow_blur, d_shadow);
            
            faceplate_col = mix(faceplate_col * 0.42, faceplate_col, shadow_mask);
            color = faceplate_col;
            
        } else if (mat == 3.0) {
            // --- Needle ---
            let angle_val = get_needle_angle(meter_idx);
            let phi = phi_min + angle_val * (phi_max - phi_min);
            let needle_dir = vec2<f32>(sin(phi), cos(phi));
            let pivot = vec2<f32>(0.0, -0.65);
            
            let dist_from_pivot = dot(q.xy - pivot, needle_dir);
            let is_tip = dist_from_pivot > 0.85;
            
            let needle_col = select(vec3<f32>(0.03, 0.03, 0.04), vec3<f32>(0.85, 0.05, 0.05), is_tip);
            let diff = max(dot(N, L), 0.0);
            color = needle_col * diff + vec3<f32>(0.015);
        }
        
        // --- Analytical Glass Layer ---
        if (p_hit.z < -0.01) {
            let t_glass = -ro.z / rd.z;
            let p_glass = ro + rd * t_glass;
            
            let V_glass = -rd;
            let N_glass = vec3<f32>(0.0, 0.0, 1.0);
            let L_glass = normalize(light_pos - p_glass);
            let H_glass = normalize(L_glass + V_glass);
            
            let spec = pow(max(dot(N_glass, H_glass), 0.0), 120.0) * 0.38;
            
            let R_glass = reflect(rd, N_glass);
            let env = mix(vec3<f32>(0.01, 0.01, 0.03), vec3<f32>(0.10, 0.12, 0.18), clamp(R_glass.y * 0.5 + 0.5, 0.0, 1.0)) * 0.15;
            
            let q_glass = (p_glass - center) / mlayout.scale;
            let dust_noise = noise2d(q_glass.xy * 25.0);
            let dust = dust_noise * 0.008;
            
            let glass_reflection = vec3<f32>(spec) + env + vec3<f32>(dust);
            color = color * 0.90 + glass_reflection;
        }
    }
    
    // --- Post Processing ---
    let vr = length(uv);
    color *= smoothstep(2.2, 0.7, vr);
    
    let scanline = 0.94 + 0.06 * cos(in.clip_position.y * 3.14159);
    color *= scanline;
    
    var final_col = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    final_col = max(final_col, vec3<f32>(0.0));
    
    return vec4<f32>(final_col, 1.0);
}
