// =====================================================
// Ferrofluid Particle Simulation Visualizer
// Raymarched heightfield from compute-driven 100k particle grid
// with chrome lighting, orbiting specular, and audio-driven camera
// =====================================================

// --- Tuning Constants ---
const GRID_SIZE: i32 = 512;
const GRID_SCALE: f32 = 25.0;
const GRID_OFFSET: f32 = 256.0;       // GRID_SIZE / 2
const DENSITY_SCALE: f32 = 0.012;     // Compensates for 25× density from 5×5 splat
const HEIGHT_SCALE: f32 = 0.8;
const PUDDLE_RADIUS: f32 = 5.5;        // Radius of the base fluid pool
const BASE_HEIGHT: f32 = 0.08;         // Minimum fluid thickness — surface tension floor
const MAX_HEIGHT: f32 = 2.5;           // Hard cap to prevent runaway spikes
const LIPSCHITZ: f32 = 0.35;          // Step scale for heightfield SDF
const MAX_MARCH_STEPS: i32 = 150;
const MAX_MARCH_DIST: f32 = 25.0;
const HIT_THRESHOLD: f32 = 0.005;
const NORMAL_EPS: f32 = 0.04;         // Wider eps — heightfield is pre-blurred
const HDR_WHITE: f32 = 5.0;
const SPEC_POWER: f32 = 24.0;
const CAM_HEIGHT: f32 = 2.0;
const CAM_DIST: f32 = 3.0;
const CAM_SWAY_AMOUNT: f32 = 0.15;
const FOG_START: f32 = 15.0;

// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;


@group(0) @binding(5)
var<storage, read> grid: array<u32>;

fn get_density_raw(gx: i32, gz: i32) -> f32 {
    if (gx >= 0 && gx < GRID_SIZE && gz >= 0 && gz < GRID_SIZE) {
        let cell = u32(gz) * u32(GRID_SIZE) + u32(gx);
        return f32(grid[cell]);
    }
    return 0.0;
}

fn get_density(p: vec2<f32>) -> f32 {
    let px = p.x * GRID_SCALE + GRID_OFFSET;
    let pz = p.y * GRID_SCALE + GRID_OFFSET;
    
    let ix = i32(floor(px));
    let iz = i32(floor(pz));
    let fx = fract(px);
    let fz = fract(pz);
    
    var sum = 0.0;
    var w_sum = 0.0;
    
    // 5×5 Gaussian kernel at stride 1 — no aliasing artifacts
    // Wide sigma blends the 5×5 compute splat into smooth, continuous mounds
    for (var i = -2; i <= 2; i++) {
        for (var j = -2; j <= 2; j++) {
            let cx = ix + i;
            let cz = iz + j;
            let val = get_density_raw(cx, cz);
            
            let dx = f32(i) - fx;
            let dy = f32(j) - fz;
            let dist_sq = dx*dx + dy*dy;
            
            // Wide Gaussian for smooth blending
            let w = exp(-dist_sq * 0.12);
            sum += val * w;
            w_sum += w;
        }
    }
    
    // Log compression + height clamp for smooth, rounded ferrofluid mounds
    let raw_density = sum / max(w_sum, 0.001);
    let h = log(1.0 + raw_density * DENSITY_SCALE) * HEIGHT_SCALE;
    // Smoothstep near the cap to round off spike tips
    return min(h, MAX_HEIGHT) * smoothstep(MAX_HEIGHT + 0.5, MAX_HEIGHT - 0.3, h);
}

fn map(p: vec3<f32>) -> f32 {
    let particle_h = get_density(p.xz);
    
    // Base puddle: thin continuous fluid layer that prevents holes
    // Simulates surface tension — ferrofluid never has gaps in its surface
    let dist_from_center = length(p.xz);
    let base_h = BASE_HEIGHT * smoothstep(PUDDLE_RADIUS, PUDDLE_RADIUS * 0.5, dist_from_center);
    
    let h = max(particle_h, base_h);
    // Multiply by Lipschitz bound factor to prevent raymarching through thin features
    return (p.y + 0.5 - h) * LIPSCHITZ;
}

fn calcNormal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(NORMAL_EPS, 0.0);
    return normalize(vec3<f32>(
        map(p + e.xyy) - map(p - e.xyy),
        map(p + e.yxy) - map(p - e.yxy),
        map(p + e.yyx) - map(p - e.yyx)
    ));
}

// SDF-based ambient occlusion for contact shadows
fn calcAO(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var ao = 0.0;
    var s = 1.0;
    for (var i = 0; i < 5; i++) {
        let h = 0.01 + 0.12 * f32(i);
        let d = map(p + n * h);
        ao += (h - d) * s;
        s *= 0.75;
    }
    return clamp(1.0 - 12.0 * ao, 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    
    // Fix aspect ratio
    var aspect = 1.7777;
    let dy = abs(dpdy(in.uv.y));
    let dx = abs(dpdx(in.uv.x));
    if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }
    let p_xy = vec2<f32>(uv.x * aspect, -uv.y);
    
    // Camera with subtle audio-driven sway
    let sway_x = sin(audio.time * 0.3) * CAM_SWAY_AMOUNT;
    let sway_z = cos(audio.time * 0.2) * CAM_SWAY_AMOUNT * 0.5;
    let ro = vec3<f32>(sway_x, CAM_HEIGHT, CAM_DIST + sway_z);
    let ta = vec3<f32>(0.0, 0.0, 0.0);
    let ww = normalize(ta - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = normalize(cross(uu, ww));
    let rd = normalize(p_xy.x * uu + p_xy.y * vv + 1.5 * ww);

    var t = 0.0;
    var hit = false;
    for(var i=0; i<MAX_MARCH_STEPS; i++) {
        let p = ro + rd * t;
        let d = map(p);
        if (d < HIT_THRESHOLD) { hit = true; break; }
        if (t > MAX_MARCH_DIST) { break; }
        t += max(d, 0.002);
    }

    // Background gradient (used for both miss rays and fog target)
    let bg = mix(vec3<f32>(0.08, 0.1, 0.14), vec3<f32>(0.6, 0.65, 0.75), rd.y * 0.5 + 0.5);

    var col = vec3<f32>(0.0);
    if (hit) {
        let p = ro + rd * t;
        let n = calcNormal(p);
        let ref_dir = reflect(rd, n);
        
        // Fresnel
        let fresnel = pow(1.0 - max(0.0, dot(n, -rd)), 5.0);
        
        // Fake HDRI environment dome: bright above, dark below
        let env = mix(vec3<f32>(0.05, 0.08, 0.15), vec3<f32>(2.0, 2.5, 3.0), smoothstep(-0.3, 0.6, ref_dir.y));
        
        // Orbiting key light — raised high to illuminate spike tops from this top-down camera
        let light_time = audio.time * 0.4;
        let light_dir1 = normalize(vec3<f32>(sin(light_time) * 1.0, 3.0, cos(light_time) * 1.0));
        let spec1 = pow(max(0.0, dot(ref_dir, light_dir1)), SPEC_POWER);
        
        // Counter-orbiting fill light
        let light_dir2 = normalize(vec3<f32>(-sin(light_time) * 0.8, 2.5, -cos(light_time) * 0.8));
        let spec2 = pow(max(0.0, dot(ref_dir, light_dir2)), 32.0);
        
        // Overhead ambient to prevent fully black surfaces
        let overhead = max(0.0, dot(n, vec3<f32>(0.0, 1.0, 0.0))) * 0.15;
        
        // Compose chrome material
        var fluid_ref = env * (0.2 + 0.8 * fresnel);
        fluid_ref += vec3<f32>(3.0, 3.5, 4.0) * spec1 * 2.0;  // Key light (cool white)
        fluid_ref += vec3<f32>(2.0, 2.0, 2.5) * spec2 * 0.6;   // Fill light
        fluid_ref += vec3<f32>(0.15, 0.2, 0.3) * overhead;      // Ambient fill
        
        col = mix(vec3<f32>(0.03, 0.04, 0.06), fluid_ref, 0.25 + 0.75 * fresnel);
        
        // Gentle SDF-based ambient occlusion (softened to avoid over-darkening crevices)
        let ao = calcAO(p, n);
        col *= mix(0.5, 1.0, ao);
    } else {
        col = bg;
    }
    
    // Distance fog — fade to background gradient (not HDR white)
    col = mix(col, bg, smoothstep(FOG_START, MAX_MARCH_DIST, t));
    
    // Soft vignette
    let vignette = 1.0 - smoothstep(0.7, 1.6, length(uv));
    col *= vignette;
    
    // Narkowicz ACES fitted tonemap
    col = (col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14);
    
    return vec4<f32>(col, 1.0);
}
