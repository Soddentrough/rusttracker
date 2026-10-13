// =====================================================
// Ferrofluid Particle Simulation Visualizer
// Raymarched heightfield from compute-driven 100k particle grid
// with chrome lighting, orbiting specular, and audio-driven camera
// =====================================================

const GRID_SIZE: i32 = 512;
const GRID_SCALE: f32 = 25.0;
const GRID_OFFSET: f32 = 256.0;
const DENSITY_SCALE: f32 = 0.012;
const HEIGHT_SCALE: f32 = 0.8;
const PUDDLE_RADIUS: f32 = 5.5;
const BASE_HEIGHT: f32 = 0.08;
const MAX_HEIGHT: f32 = 2.5;
const LIPSCHITZ: f32 = 0.35;
const MAX_MARCH_STEPS: i32 = 150;
const MAX_MARCH_DIST: f32 = 25.0;
const HIT_THRESHOLD: f32 = 0.005;
const NORMAL_EPS: f32 = 0.04;
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

    // Rational polynomial approximation to exp(-d*d*0.2) saves 6 transcendental calls per sample
    var w_x: array<f32, 3>;
    for (var i = 0; i < 3; i++) {
        let dx = f32(i - 1) - fx;
        w_x[i] = 1.0 / (1.0 + dx * dx * 0.22);
    }

    var w_z: array<f32, 3>;
    for (var j = 0; j < 3; j++) {
        let dz = f32(j - 1) - fz;
        w_z[j] = 1.0 / (1.0 + dz * dz * 0.22);
    }

    var sum = 0.0;
    var w_sum = 0.0;

    for (var i = -1; i <= 1; i++) {
        let cx = ix + i;
        let wx = w_x[i + 1];
        for (var j = -1; j <= 1; j++) {
            let cz = iz + j;
            let val = get_density_raw(cx, cz);
            let w = wx * w_z[j + 1];
            sum += val * w;
            w_sum += w;
        }
    }

    let raw_density = sum / max(w_sum, 0.001);
    let h = log(1.0 + raw_density * DENSITY_SCALE) * HEIGHT_SCALE;
    return min(h, MAX_HEIGHT) * smoothstep_r(MAX_HEIGHT + 0.5, MAX_HEIGHT - 0.3, h);
}

fn map(p: vec3<f32>) -> f32 {
    let dist_from_center = length(p.xz);
    if (dist_from_center > PUDDLE_RADIUS + 0.5) {
        return (p.y + 0.5) * LIPSCHITZ;
    }

    let particle_h = get_density(p.xz);
    let base_h = BASE_HEIGHT * smoothstep_r(PUDDLE_RADIUS, PUDDLE_RADIUS * 0.5, dist_from_center);
    let h = max(particle_h, base_h);
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

fn ray_box_intersect(ro: vec3<f32>, rd: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> vec2<f32> {
    let inv_d = 1.0 / (rd + select(vec3<f32>(1e-6), vec3<f32>(-1e-6), rd < vec3<f32>(0.0)));
    let t0 = (bmin - ro) * inv_d;
    let t1 = (bmax - ro) * inv_d;
    let tmin = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let tmax = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    return vec2<f32>(tmin, tmax);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv * 2.0 - 1.0;
    let aspect = max(audio.aspect_ratio, 0.01);
    let p_xy = vec2<f32>(uv.x * aspect, -uv.y);

    let time = audio.smooth_time;
    let sway_x = sin(time * 0.3) * CAM_SWAY_AMOUNT;
    let sway_z = cos(time * 0.2) * CAM_SWAY_AMOUNT * 0.5;
    let ro = vec3<f32>(sway_x, CAM_HEIGHT, CAM_DIST + sway_z);
    let ta = vec3<f32>(0.0, 0.0, 0.0);
    let ww = normalize(ta - ro);
    let uu = normalize(cross(ww, vec3<f32>(0.0, 1.0, 0.0)));
    let vv = normalize(cross(uu, ww));
    let rd = normalize(p_xy.x * uu + p_xy.y * vv + 1.5 * ww);

    var t = 0.0;
    var hit = false;

    // AABB slab + horizontal extent pre-test skips sky rays and starts marching right at the volume entry
    let b_rad = MAX_MARCH_DIST;
    let t_bounds = ray_box_intersect(ro, rd, vec3<f32>(-b_rad, -0.55, -b_rad), vec3<f32>(b_rad, MAX_HEIGHT - 0.35, b_rad));
    if (t_bounds.y >= max(t_bounds.x, 0.0) && t_bounds.x < MAX_MARCH_DIST) {
        t = max(t_bounds.x, 0.0);
        let t_end = min(t_bounds.y, MAX_MARCH_DIST);
        for (var i = 0; i < MAX_MARCH_STEPS; i++) {
            let p = ro + rd * t;
            let d = map(p);
            if (d < HIT_THRESHOLD) { hit = true; break; }
            if (t > t_end) { break; }
            let min_step = 0.002 + t * 0.006;
            t += max(d, min_step);
        }
    }

    let bg = mix(vec3<f32>(0.08, 0.1, 0.14), vec3<f32>(0.6, 0.65, 0.75), rd.y * 0.5 + 0.5);

    var col = vec3<f32>(0.0);
    if (hit) {
        let p = ro + rd * t;
        let n = calcNormal(p);
        let ref_dir = reflect(rd, n);
        let NdotV = max(0.0, dot(n, -rd));

        let fresnel = pow(1.0 - NdotV, 5.0);
        let env = mix(vec3<f32>(0.05, 0.08, 0.15), vec3<f32>(2.0, 2.5, 3.0), smoothstep(-0.3, 0.6, ref_dir.y));

        let light_time = time * 0.4;
        let light_dir1 = normalize(vec3<f32>(sin(light_time) * 1.0, 3.0, cos(light_time) * 1.0));
        let spec1 = pow(max(0.0, dot(ref_dir, light_dir1)), SPEC_POWER);

        let light_dir2 = normalize(vec3<f32>(-sin(light_time) * 0.8, 2.5, -cos(light_time) * 0.8));
        let spec2 = pow(max(0.0, dot(ref_dir, light_dir2)), 32.0);

        let overhead = max(0.0, dot(n, vec3<f32>(0.0, 1.0, 0.0))) * 0.15;

        // Subtle thin-film chrome iridescence on elevated ferrofluid peaks
        let spike_elev = clamp((p.y + 0.42) * 1.25, 0.0, 1.0);
        let irid = iridescence_cos(NdotV * 2.4 + p.y * 0.9 + time * 0.18);

        var fluid_ref = env * (0.2 + 0.8 * fresnel);
        fluid_ref += vec3<f32>(3.0, 3.5, 4.0) * spec1 * 2.0;
        fluid_ref += vec3<f32>(2.0, 2.0, 2.5) * spec2 * 0.6;
        fluid_ref += vec3<f32>(0.15, 0.2, 0.3) * overhead;
        fluid_ref = mix(fluid_ref, fluid_ref * (0.75 + 0.55 * irid), spike_elev * (1.0 - NdotV) * 0.35);

        col = mix(vec3<f32>(0.03, 0.04, 0.06), fluid_ref, 0.25 + 0.75 * fresnel);

        let ao = calcAO(p, n);
        col *= mix(0.5, 1.0, ao);
    } else {
        col = bg;
    }

    col = mix(col, bg, smoothstep(FOG_START, MAX_MARCH_DIST, t));

    let vignette = smoothstep_r(1.6, 0.7, length(uv));
    col *= vignette;

    col = aces_tonemap(col);

    return vec4<f32>(col, 1.0);
}
