// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var<storage, read> discs: array<vec4<f32>>;

const GRID_W: i32 = 128;
const GRID_H: i32 = 96;
const HALF_W: i32 = 64;
const HALF_H: i32 = 48;
const PI: f32 = 3.14159265;

// Authentic 45-degree diagonal flip axis (+1, +1, 0) / sqrt(2)
const FLIP_AXIS: vec3<f32> = vec3<f32>(0.70710678, 0.70710678, 0.0);
const DISC_R: f32 = 0.442;
const DISC_H: f32 = 0.026;
const NOTCH_C: vec2<f32> = vec2<f32>(0.095, -0.405);
const NOTCH_R: f32 = 0.096;
const PIN_A: vec2<f32> = vec2<f32>(0.095, -0.405);
const PIN_B: vec2<f32> = vec2<f32>(-0.405, 0.095);

const L_KEY_DIR: vec3<f32> = vec3<f32>(-0.48324, 0.62418, 0.61412);
const L_FILL_DIR: vec3<f32> = vec3<f32>(0.73307, -0.28195, 0.61928);

fn rotate_axis(v: vec3<f32>, axis: vec3<f32>, cos_a: f32, sin_a: f32) -> vec3<f32> {
    return v * cos_a + cross(axis, v) * sin_a + axis * dot(axis, v) * (1.0 - cos_a);
}

fn disc_2d_sdf(q: vec2<f32>) -> f32 {
    let d_outer = length(q) - DISC_R;
    let d_notch = NOTCH_R - length(q - NOTCH_C);
    return max(d_outer, d_notch);
}

struct DiscHit {
    hit: bool,
    t: f32,
    cov: f32,
    normal: vec3<f32>,
    local_p: vec3<f32>,
    kind: u32, // 0: charcoal cap, 1: white cap, 2: rim/notch edge
}

fn intersect_cell_disc(
    ro_cell: vec3<f32>,
    rd: vec3<f32>,
    cos_th: f32,
    sin_th: f32,
    ray_cone: f32,
) -> DiscHit {
    var res: DiscHit;
    res.hit = false;
    res.t = 1e9;
    res.cov = 0.0;
    res.normal = vec3<f32>(0.0, 0.0, 1.0);
    res.local_p = vec3<f32>(0.0);
    res.kind = 0u;

    // Transform ray into disc-local space by rotating -theta around FLIP_AXIS
    let p0 = rotate_axis(ro_cell, FLIP_AXIS, cos_th, -sin_th);
    let d = rotate_axis(rd, FLIP_AXIS, cos_th, -sin_th);

    // Quick bounding sphere check (R_bound = 0.45)
    let bs_b = dot(p0, d);
    let bs_c = dot(p0, p0) - 0.45 * 0.45;
    if (bs_b * bs_b - bs_c < 0.0) {
        return res;
    }

    var tz_min = -1e9;
    var tz_max = 1e9;
    if (abs(d.z) > 1e-6) {
        let tz0 = (-DISC_H - p0.z) / d.z;
        let tz1 = (DISC_H - p0.z) / d.z;
        tz_min = min(tz0, tz1);
        tz_max = max(tz0, tz1);
    } else if (abs(p0.z) > DISC_H) {
        return res;
    }

    if (tz_max <= 0.001 || tz_min >= tz_max) {
        return res;
    }

    var best_t = 1e9;
    var best_cov = 0.0;
    var best_n_loc = vec3<f32>(0.0, 0.0, 1.0);
    var best_p_loc = vec3<f32>(0.0);
    var best_kind = 0u;

    // 1. Entry face cap (z = +DISC_H charcoal or z = -DISC_H white)
    var cap_sdf = 1e9;
    var cap_n_loc = vec3<f32>(0.0, 0.0, 1.0);
    var cap_p_loc = vec3<f32>(0.0);
    var cap_kind = 0u;
    if (tz_min > 0.001) {
        let q = p0.xy + d.xy * tz_min;
        cap_sdf = disc_2d_sdf(q);
        let p_cap = p0 + d * tz_min;
        let cap_sign = select(-1.0, 1.0, p_cap.z >= 0.0);
        let edge_blend = smoothstep(-0.018, 0.0, cap_sdf);
        let edge_dir = select(
            normalize(vec2<f32>(q.x, q.y) + vec2<f32>(1e-5)),
            -normalize((q - NOTCH_C) + vec2<f32>(1e-5)),
            (NOTCH_R - length(q - NOTCH_C)) > (length(q) - DISC_R)
        );
        cap_n_loc = normalize(vec3<f32>(edge_dir * edge_blend * 0.22, cap_sign));
        cap_p_loc = p_cap;
        cap_kind = select(1u, 0u, cap_sign > 0.0);
        if (cap_sdf <= 0.0) {
            best_t = tz_min;
            best_cov = 1.0;
            best_n_loc = cap_n_loc;
            best_p_loc = cap_p_loc;
            best_kind = cap_kind;
        }
    }

    // 2. Cylindrical outer rim & inner notch wall (visible along disc edges & mid-flip)
    if (best_cov < 0.995) {
        let a_xy = dot(d.xy, d.xy);
        if (a_xy > 1e-7) {
            let b_xy = dot(p0.xy, d.xy);
            let c_xy = dot(p0.xy, p0.xy) - DISC_R * DISC_R;
            let disc_outer = b_xy * b_xy - a_xy * c_xy;
            if (disc_outer >= 0.0) {
                let t_c0 = (-b_xy - sqrt(disc_outer)) / a_xy;
                if (t_c0 > 0.001 && t_c0 >= tz_min - 1e-4 && t_c0 <= tz_max + 1e-4 && t_c0 < best_t) {
                    let p_c = p0 + d * t_c0;
                    if (length(p_c.xy - NOTCH_C) >= NOTCH_R) {
                        let aa = max(t_c0 * ray_cone * 0.65, 0.0018);
                        let rim_cov = smoothstep_r(DISC_H + aa, DISC_H - aa * 0.25, abs(p_c.z));
                        if (rim_cov > best_cov) {
                            best_t = t_c0;
                            best_cov = rim_cov;
                            best_n_loc = normalize(vec3<f32>(p_c.xy, 0.0));
                            best_p_loc = p_c;
                            best_kind = 2u;
                        }
                    }
                }
            }

            let pn0 = p0.xy - NOTCH_C;
            let bn = dot(pn0, d.xy);
            let cn = dot(pn0, pn0) - NOTCH_R * NOTCH_R;
            let disc_notch = bn * bn - a_xy * cn;
            if (disc_notch >= 0.0) {
                let t_n1 = (-bn + sqrt(disc_notch)) / a_xy;
                if (t_n1 > 0.001 && t_n1 >= tz_min - 1e-4 && t_n1 <= tz_max + 1e-4 && t_n1 < best_t) {
                    let p_n = p0 + d * t_n1;
                    if (length(p_n.xy) <= DISC_R) {
                        let aa = max(t_n1 * ray_cone * 0.65, 0.0018);
                        let rim_cov = smoothstep_r(DISC_H + aa, DISC_H - aa * 0.25, abs(p_n.z));
                        if (rim_cov > best_cov) {
                            best_t = t_n1;
                            best_cov = rim_cov;
                            best_n_loc = normalize(vec3<f32>(NOTCH_C - p_n.xy, 0.0));
                            best_p_loc = p_n;
                            best_kind = 2u;
                        }
                    }
                }
            }
        }
    }

    // 3. Exterior pixel-cone AA feather around the curved rim & notch
    if (best_cov < 0.995 && tz_min > 0.001) {
        let aa = max(tz_min * ray_cone * 0.70, 0.002);
        if (cap_sdf > 0.0 && cap_sdf < aa) {
            let feather = smoothstep_r(aa, 0.0, cap_sdf);
            if (feather > best_cov) {
                best_t = tz_min;
                best_cov = feather;
                best_n_loc = cap_n_loc;
                best_p_loc = cap_p_loc;
                best_kind = cap_kind;
            }
        }
    }

    if (best_cov > 0.002) {
        res.hit = true;
        res.t = best_t;
        res.cov = clamp(best_cov, 0.0, 1.0);
        res.normal = normalize(rotate_axis(best_n_loc, FLIP_AXIS, cos_th, sin_th));
        res.local_p = best_p_loc;
        res.kind = best_kind;
    }
    return res;
}

fn disc_cast_shadow(ro_cell_p: vec3<f32>, cos_th: f32, sin_th: f32) -> f32 {
    let p0 = rotate_axis(ro_cell_p, FLIP_AXIS, cos_th, -sin_th);
    let d = rotate_axis(L_KEY_DIR, FLIP_AXIS, cos_th, -sin_th);
    if (abs(d.z) < 1e-5) {
        return 1.0;
    }
    let t = -p0.z / d.z;
    if (t <= 0.0) {
        return 1.0;
    }
    let q = p0.xy + d.xy * t;
    let sdf = disc_2d_sdf(q);
    return smoothstep(-0.025, 0.025, sdf) * 0.86 + 0.14;
}

fn shade_studio_surface(
    albedo: vec3<f32>,
    normal: vec3<f32>,
    view_dir: vec3<f32>,
    roughness: f32,
    metallic: f32,
    ao: f32,
    key_shadow: f32,
) -> vec3<f32> {
    let n = normalize(normal);
    let v = normalize(view_dir);
    let n_dot_v = max(dot(n, v), 0.02);

    // Key light (warm-neutral overhead-left studio softbox)
    let ndl_key = max(dot(n, L_KEY_DIR), 0.0);
    let h_key = normalize(L_KEY_DIR + v);
    let ndh_key = max(dot(n, h_key), 0.0);
    let spec_pow = mix(96.0, 12.0, roughness * roughness);
    let f0 = mix(vec3<f32>(0.036), albedo, metallic);
    let omt = 1.0 - n_dot_v;
    let omt2 = omt * omt;
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * (omt2 * omt2);
    let spec_norm = (spec_pow + 8.0) * 0.028;
    let spec_key = fresnel * pow(max(ndh_key, 1e-4), spec_pow) * spec_norm * ndl_key;

    // Cool subtle right fill light
    let ndl_fill = max(dot(n, L_FILL_DIR), 0.0);
    let h_fill = normalize(L_FILL_DIR + v);
    let spec_fill = fresnel * pow(max(dot(n, h_fill), 1e-4), spec_pow * 0.65) * spec_norm * 0.28 * ndl_fill;

    let diff_weight = (1.0 - metallic) * 0.92;
    let key_light = vec3<f32>(1.16, 1.14, 1.11) * key_shadow;
    let fill_light = vec3<f32>(0.24, 0.26, 0.30);
    let hemi_up = n.y * 0.5 + 0.5;
    let ambient = mix(vec3<f32>(0.10, 0.10, 0.11), vec3<f32>(0.22, 0.22, 0.23), hemi_up) * ao;

    let diffuse = albedo * diff_weight * (key_light * ndl_key + fill_light * ndl_fill + ambient);
    let specular = key_light * spec_key + fill_light * spec_fill;
    return diffuse + specular;
}

fn shade_disc_material(
    hit: DiscHit,
    view_dir: vec3<f32>,
    cell_hash: f32,
) -> vec3<f32> {
    let grain = (value_noise2d(hit.local_p.xy * 42.0 + vec2<f32>(cell_hash * 37.0)) - 0.5) * 0.018;
    let charcoal_albedo = vec3<f32>(0.050, 0.052, 0.056) + vec3<f32>(grain * 0.35);
    let white_albedo = vec3<f32>(0.76, 0.75, 0.73) + vec3<f32>(grain * 0.6);

    var albedo = charcoal_albedo;
    var roughness = 0.54;
    var metallic = 0.02;

    if (hit.kind == 0u) {
        // Charcoal side: tiny metallic magnet dimple near 8:30 o'clock + subtle hinge pin markers
        let dot_dist = length(hit.local_p.xy - vec2<f32>(-0.365, -0.02));
        let magnet_mask = smoothstep_r(0.040, 0.026, dot_dist);
        albedo = mix(charcoal_albedo, vec3<f32>(0.26, 0.27, 0.28), magnet_mask);
        roughness = mix(0.54, 0.30, magnet_mask);
        metallic = mix(0.02, 0.70, magnet_mask);
    } else if (hit.kind == 1u) {
        // White side: clean matte-satin off-white coating
        albedo = white_albedo;
        roughness = 0.39;
        metallic = 0.01;
    } else {
        // Molded dark polymer disc edge / notch wall (eliminates white halo around charcoal discs)
        albedo = vec3<f32>(0.075, 0.077, 0.082);
        roughness = 0.38;
        metallic = 0.06;
    }

    return shade_studio_surface(albedo, hit.normal, view_dir, roughness, metallic, 1.0, 1.0);
}

struct HouseHit {
    t: f32,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    ao: f32,
}

fn intersect_cell_housing(
    ro_cell: vec3<f32>,
    rd: vec3<f32>,
    t_enter: f32,
    t_exit: f32,
) -> HouseHit {
    var hh: HouseHit;
    hh.t = 1e9;
    hh.normal = vec3<f32>(0.0, 0.0, 1.0);
    hh.albedo = vec3<f32>(0.018, 0.019, 0.021);
    hh.roughness = 0.65;
    hh.metallic = 0.02;
    hh.ao = 0.55;

    // 1. Molded housing top plane (corner gussets & outer cell divider ribs) at z = 0.024
    let z_frame = 0.024;
    if (rd.z < -1e-5) {
        let t_f = (z_frame - ro_cell.z) / rd.z;
        if (t_f >= t_enter - 1e-4 && t_f <= t_exit + 1e-4) {
            let pf = ro_cell.xy + rd.xy * t_f;
            let ax = abs(pf.x);
            let ay = abs(pf.y);
            if (ax <= 0.502 && ay <= 0.502) {
                let diag = ax + ay;
                let max_c = max(ax, ay);
                if (diag > 0.768 || max_c > 0.474) {
                    let seam = smoothstep_r(0.495, 0.486, max_c);
                    let sx = select(-1.0, 1.0, pf.x >= 0.0);
                    let sy = select(-1.0, 1.0, pf.y >= 0.0);
                    let bevel = smoothstep_r(0.825, 0.768, diag);
                    let n_frame = normalize(vec3<f32>(-sx * bevel * 0.48, -sy * bevel * 0.48, 1.0));
                    hh.t = t_f;
                    hh.normal = n_frame;
                    hh.albedo = vec3<f32>(0.036, 0.038, 0.042) * (0.45 + 0.55 * seam);
                    hh.roughness = 0.40;
                    hh.metallic = 0.05;
                    hh.ao = 0.78 + 0.22 * seam;
                    return hh;
                }
            }
        }
    }

    // 2. Inner triangular corner gusset sidewalls & divider walls (z in [-0.18, 0.024])
    let p_mid = ro_cell.xy + rd.xy * clamp((-0.08 - ro_cell.z) / min(rd.z, -1e-4), t_enter, t_exit);
    let sx = select(-1.0, 1.0, p_mid.x >= 0.0);
    let sy = select(-1.0, 1.0, p_mid.y >= 0.0);
    let diag_den = sx * rd.x + sy * rd.y;
    if (diag_den > 1e-5) {
        let t_wall = (0.768 - (sx * ro_cell.x + sy * ro_cell.y)) / diag_den;
        if (t_wall >= t_enter && t_wall <= t_exit && t_wall < hh.t) {
            let pw = ro_cell + rd * t_wall;
            if (pw.z <= z_frame && pw.z >= -0.185 && abs(pw.x) <= 0.502 && abs(pw.y) <= 0.502) {
                hh.t = t_wall;
                hh.normal = normalize(vec3<f32>(-sx, -sy, 0.18));
                hh.albedo = vec3<f32>(0.026, 0.027, 0.030);
                hh.roughness = 0.52;
                hh.metallic = 0.04;
                hh.ao = smoothstep(-0.20, 0.04, pw.z) * 0.65 + 0.25;
            }
        }
    }
    if (abs(rd.x) > 1e-5) {
        let tx_rib = (sx * 0.474 - ro_cell.x) / rd.x;
        if (tx_rib >= t_enter && tx_rib <= t_exit && tx_rib < hh.t) {
            let pw = ro_cell + rd * tx_rib;
            if (pw.z <= z_frame && pw.z >= -0.185 && abs(pw.y) <= 0.476) {
                hh.t = tx_rib;
                hh.normal = normalize(vec3<f32>(-sx, 0.0, 0.12));
                hh.albedo = vec3<f32>(0.024, 0.025, 0.028);
                hh.roughness = 0.55;
                hh.ao = smoothstep(-0.20, 0.04, pw.z) * 0.6 + 0.25;
            }
        }
    }
    if (abs(rd.y) > 1e-5) {
        let ty_rib = (sy * 0.474 - ro_cell.y) / rd.y;
        if (ty_rib >= t_enter && ty_rib <= t_exit && ty_rib < hh.t) {
            let pw = ro_cell + rd * ty_rib;
            if (pw.z <= z_frame && pw.z >= -0.185 && abs(pw.x) <= 0.476) {
                hh.t = ty_rib;
                hh.normal = normalize(vec3<f32>(0.0, -sy, 0.12));
                hh.albedo = vec3<f32>(0.024, 0.025, 0.028);
                hh.roughness = 0.55;
                hh.ao = smoothstep(-0.20, 0.04, pw.z) * 0.6 + 0.25;
            }
        }
    }

    // 3. Electromagnetic coil pole pins (peeking inside the notch) at z = 0.014
    let z_pin = 0.014;
    if (rd.z < -1e-5) {
        let t_p = (z_pin - ro_cell.z) / rd.z;
        if (t_p >= t_enter && t_p <= t_exit && t_p < hh.t) {
            let pp = ro_cell.xy + rd.xy * t_p;
            let da = length(pp - PIN_A);
            let db = length(pp - PIN_B);
            if (da < 0.052) {
                hh.t = t_p;
                hh.normal = normalize(vec3<f32>((pp - PIN_A) * 4.0, 1.0));
                hh.albedo = vec3<f32>(0.34, 0.35, 0.37);
                hh.roughness = 0.28;
                hh.metallic = 0.78;
                hh.ao = 0.85;
                return hh;
            }
            if (db < 0.052) {
                hh.t = t_p;
                hh.normal = normalize(vec3<f32>((pp - PIN_B) * 4.0, 1.0));
                hh.albedo = vec3<f32>(0.10, 0.11, 0.12);
                hh.roughness = 0.36;
                hh.metallic = 0.55;
                hh.ao = 0.80;
                return hh;
            }
        }
    }

    // 4. Recessed cell pocket backplate at z = -0.18
    if (rd.z < -1e-5) {
        let t_b = (-0.18 - ro_cell.z) / rd.z;
        if (t_b >= t_enter - 1e-4 && t_b <= t_exit + 1e-4 && t_b < hh.t) {
            let pb = ro_cell.xy + rd.xy * t_b;
            if (abs(pb.x) <= 0.502 && abs(pb.y) <= 0.502) {
                hh.t = t_b;
                hh.normal = vec3<f32>(0.0, 0.0, 1.0);
                let wall_ao = smoothstep(0.50, 0.18, max(abs(pb.x), abs(pb.y)));
                hh.albedo = vec3<f32>(0.014, 0.015, 0.017);
                hh.roughness = 0.72;
                hh.metallic = 0.02;
                hh.ao = 0.28 + 0.42 * wall_ao;
            }
        }
    }

    return hh;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Compute derivatives strictly in uniform control flow
    let uv_fw = max(fwidth(in.uv.y), 1e-4);

    let time = select(audio.time, audio.smooth_time, audio.smooth_time > 0.0);
    let aspect = max(audio.aspect_ratio, 0.35);

    // Full-area camera framing: fills 100% of visualization area with zero borders at all aspects
    let vis_h = clamp(30.0 / sqrt(aspect), 17.5, 42.0);
    let tan_half_fov_y = clamp(0.31 / max(1.0, aspect * 0.57), 0.14, 0.35);
    let tan_half_fov_x = tan_half_fov_y * aspect;
    let cam_dist = (vis_h * 0.5) / tan_half_fov_y;
    let ray_cone = uv_fw * (2.0 * tan_half_fov_y) * 0.9;

    // Gentle oblique architectural angle inspired by physical flip-disc gallery walls
    let yaw = 0.28 + 0.05 * sin(time * 0.11);
    let pitch = -0.085 + 0.025 * cos(time * 0.085);
    let cos_p = cos(pitch);
    let eye = cam_dist * vec3<f32>(sin(yaw) * cos_p, sin(pitch), cos(yaw) * cos_p);

    let forward = normalize(-eye);
    let right = normalize(cross(forward, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, forward);

    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let rd = normalize(forward + right * (ndc.x * tan_half_fov_x) + up * (ndc.y * tan_half_fov_y));
    let view_dir = -rd;

    // Intersect active cell slab z in [-0.22, +0.46]
    let z_top = 0.46;
    let z_bot = -0.22;
    let t_start = max((z_top - eye.z) / rd.z, 0.0);
    let t_end = (z_bot - eye.z) / rd.z;

    let p_enter = eye + rd * t_start;
    let u_pos = p_enter.xy + vec2<f32>(0.5, 0.5);
    var cx_i = i32(floor(u_pos.x));
    var cy_i = i32(floor(u_pos.y));

    let step_x = select(-1, 1, rd.x >= 0.0);
    let step_y = select(-1, 1, rd.y >= 0.0);
    let inv_dx = select(1e9, 1.0 / abs(rd.x), abs(rd.x) > 1e-6);
    let inv_dy = select(1e9, 1.0 / abs(rd.y), abs(rd.y) > 1e-6);
    let fx = u_pos.x - floor(u_pos.x);
    let fy = u_pos.y - floor(u_pos.y);
    var t_max_x = t_start + select(fx, 1.0 - fx, rd.x >= 0.0) * inv_dx;
    var t_max_y = t_start + select(fy, 1.0 - fy, rd.y >= 0.0) * inv_dy;

    var t_cell_enter = t_start;
    var accum_col = vec3<f32>(0.0);
    var accum_alpha = 0.0;

    for (var step_idx = 0u; step_idx < 4u; step_idx = step_idx + 1u) {
        let t_cell_exit = min(min(t_max_x, t_max_y), t_end);
        let gx = clamp(cx_i + HALF_W, 0, GRID_W - 1);
        let gy = clamp(cy_i + HALF_H, 0, GRID_H - 1);
        let disc_idx = u32(gy) * u32(GRID_W) + u32(gx);
        let disc_state = discs[disc_idx];

        let cell_f = vec2<f32>(f32(cx_i), f32(cy_i));
        let cell_hash = hash21_crt(cell_f + vec2<f32>(17.3, 41.9));
        let micro_tilt = (cell_hash - 0.5) * 0.048;
        let theta = clamp(disc_state.x + micro_tilt, -0.04, PI + 0.04);
        let cos_th = cos(theta);
        let sin_th = sin(theta);

        let ro_cell = eye - vec3<f32>(cell_f.x, cell_f.y, 0.0);
        let d_hit = intersect_cell_disc(ro_cell, rd, cos_th, sin_th, ray_cone);
        let h_hit = intersect_cell_housing(ro_cell, rd, t_cell_enter, t_cell_exit);

        if (d_hit.hit && d_hit.t <= h_hit.t) {
            let d_col = shade_disc_material(d_hit, view_dir, cell_hash);
            let a = d_hit.cov;
            accum_col += (1.0 - accum_alpha) * a * d_col;
            accum_alpha += (1.0 - accum_alpha) * a;

            if (accum_alpha < 0.995 && h_hit.t < 1e8) {
                let p_house = ro_cell + rd * h_hit.t;
                let shadow = disc_cast_shadow(p_house, cos_th, sin_th);
                let h_col = shade_studio_surface(
                    h_hit.albedo,
                    h_hit.normal,
                    view_dir,
                    h_hit.roughness,
                    h_hit.metallic,
                    h_hit.ao,
                    shadow,
                );
                accum_col += (1.0 - accum_alpha) * h_col;
                accum_alpha = 1.0;
            }
        } else if (h_hit.t < 1e8) {
            let p_house = ro_cell + rd * h_hit.t;
            let shadow = disc_cast_shadow(p_house, cos_th, sin_th);
            let h_col = shade_studio_surface(
                h_hit.albedo,
                h_hit.normal,
                view_dir,
                h_hit.roughness,
                h_hit.metallic,
                h_hit.ao,
                shadow,
            );
            accum_col += (1.0 - accum_alpha) * h_col;
            accum_alpha = 1.0;
        }

        if (accum_alpha >= 0.995 || t_cell_exit >= t_end - 1e-4) {
            break;
        }

        t_cell_enter = t_cell_exit;
        if (t_max_x < t_max_y) {
            t_max_x += inv_dx;
            cx_i += step_x;
        } else {
            t_max_y += inv_dy;
            cy_i += step_y;
        }
    }

    if (accum_alpha < 0.995) {
        let fallback_col = shade_studio_surface(
            vec3<f32>(0.014, 0.015, 0.017),
            vec3<f32>(0.0, 0.0, 1.0),
            view_dir,
            0.72,
            0.02,
            0.35,
            0.45,
        );
        accum_col += (1.0 - accum_alpha) * fallback_col;
    }

    // Subtle photographic edge vignette (~8%)
    let vig = 1.0 - dot(ndc, ndc) * 0.045;
    let final_col = aces_tonemap(accum_col * clamp(vig, 0.85, 1.0));
    return vec4<f32>(final_col, 1.0);
}
