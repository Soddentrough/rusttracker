// INCLUDE: common

struct Particle {
    pos: vec4<f32>, // pos.xyz = position, pos.w = life
    vel: vec4<f32>, // vel.xyz = velocity, vel.w = energy
}

struct SurfaceSample {
    y: f32,
    folding: f32,
    vel_xz: vec2<f32>,
    vel_y: f32,
}

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read_write> particles: array<Particle>;

const BIOLUM_PARTICLE_COUNT: u32 = 36864u;

// Calibrated 5-wave directional Gerstner spectrum (shared identically with vis_bioluminescence.wgsl):
// vec4(dir.x, dir.y, k = 2*PI/wavelength, w_smooth = exact 2-decimal angular frequency)
// Sum of k_i * qa_i = 0.6962 (max with qa_scale 1.20 = 0.835 < 1.0, preventing self-intersection).
const WAVE_KINEMATICS: array<vec4<f32>, 5> = array<vec4<f32>, 5>(
    vec4<f32>( 0.220000, -0.975500, 0.330694, 0.48),
    vec4<f32>(-0.420000, -0.907524, 0.502655, 0.64),
    vec4<f32>( 0.640000, -0.768375, 0.849079, 0.86),
    vec4<f32>(-0.760000, -0.649923, 1.365910, 1.12),
    vec4<f32>( 0.350000, -0.936750, 2.327106, 1.44)
);

// vec2(amplitude, qa = steepness * amplitude / (k * amplitude))
const WAVE_AMPS: array<vec2<f32>, 5> = array<vec2<f32>, 5>(
    vec2<f32>(0.54, 0.72),
    vec2<f32>(0.34, 0.38),
    vec2<f32>(0.18, 0.16),
    vec2<f32>(0.095, 0.062),
    vec2<f32>(0.048, 0.020)
);

fn hash31(p: f32) -> vec3<f32> {
    var p3 = fract(vec3<f32>(p) * vec3<f32>(0.1031, 0.11369, 0.13787));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xxy + p3.yzz) * p3.zyx);
}

fn hash21_fast(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}

fn noise2d_fast(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21_fast(i);
    let b = hash21_fast(i + vec2<f32>(1.0, 0.0));
    let c = hash21_fast(i + vec2<f32>(0.0, 1.0));
    let d = hash21_fast(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm2d_foam_fast(p: vec2<f32>) -> f32 {
    let n0 = noise2d_fast(p);
    let n1 = noise2d_fast(p * 2.07 + vec2<f32>(1.7, 9.2));
    return n0 * 0.65 + n1 * 0.35;
}

fn sample_biolum_audio() -> vec4<f32> {
    let raw_bass = max(
        max(audio.spectrum[0].x, audio.spectrum[0].y),
        max(audio.spectrum[1].x, audio.spectrum[2].x)
    );
    let raw_mids = (audio.spectrum[8].x + audio.spectrum[12].x + audio.spectrum[20].x) * 0.6667;
    let raw_treble = (audio.spectrum[40].x + audio.spectrum[55].x + audio.spectrum[75].x) * 0.6667;

    let bass = clamp(raw_bass / 72.0, 0.0, 1.5);
    let mids = clamp(raw_mids / 125.0, 0.0, 1.3);
    let treble = clamp(raw_treble / 115.0, 0.0, 1.3);

    var ch_l = 0.0;
    var ch_r = 0.0;
    if (audio.num_spatial_channels >= 2u) {
        ch_l = audio.spatial_channels[0].x;
        ch_r = audio.spatial_channels[0].y;
    } else if (audio.num_channels >= 2u) {
        ch_l = audio.channels[0].x;
        ch_r = audio.channels[0].y;
    } else if (audio.num_channels == 1u) {
        ch_l = audio.channels[0].x;
        ch_r = ch_l;
    }
    let stereo_bias = clamp((ch_r - ch_l) * 0.9, -1.0, 1.0);
    return vec4<f32>(bass, mids, treble, stereo_bias);
}

fn eval_gerstner_disp(xz: vec2<f32>, time: f32, amp_scale: f32, qa_scale: f32) -> vec3<f32> {
    var disp = vec3<f32>(0.0);
    for (var i = 0u; i < 5u; i = i + 1u) {
        let kin = WAVE_KINEMATICS[i];
        let amp = WAVE_AMPS[i];
        let dir = kin.xy;
        let k = kin.z;
        let w = kin.w;

        let theta = k * dot(dir, xz) - w * time;
        let sin_t = sin(theta);
        let cos_t = cos(theta);

        let a = amp.x * amp_scale;
        let qa = amp.y * qa_scale;

        disp += vec3<f32>(qa * dir.x * cos_t, a * sin_t, qa * dir.y * cos_t);
    }
    return disp;
}

// 1-step inverse XZ refinement matches the displaced Gerstner heightfield mesh at world (x, z),
// plus horizontal Jacobian compression (folding) and real-time orbital wave velocity (2.88 * w).
fn eval_gerstner_surface(xz_world: vec2<f32>, time: f32, amp_scale: f32, qa_scale: f32) -> SurfaceSample {
    let d0 = eval_gerstner_disp(xz_world, time, amp_scale, qa_scale);
    let x0 = xz_world - d0.xz;

    var y = 0.0;
    var tx = 1.0;
    var tz = 0.0;
    var bz = 1.0;
    var vel_xz = vec2<f32>(0.0);
    var vel_y = 0.0;

    for (var i = 0u; i < 5u; i = i + 1u) {
        let kin = WAVE_KINEMATICS[i];
        let amp = WAVE_AMPS[i];
        let dir = kin.xy;
        let k = kin.z;
        let w = kin.w;

        let theta = k * dot(dir, x0) - w * time;
        let sin_t = sin(theta);
        let cos_t = cos(theta);

        let a = amp.x * amp_scale;
        let qa = amp.y * qa_scale;
        let k_qa = k * qa;
        let w_real = w * 2.88;

        y += a * sin_t;
        tx -= k_qa * dir.x * dir.x * sin_t;
        tz -= k_qa * dir.x * dir.y * sin_t;
        bz -= k_qa * dir.y * dir.y * sin_t;

        vel_xz += dir * (qa * w_real * sin_t);
        vel_y -= a * w_real * cos_t;
    }

    let jacobian = tx * bz - tz * tz;
    let folding = clamp((1.0 - jacobian) * 1.15, 0.0, 1.0);

    var out: SurfaceSample;
    out.y = y;
    out.folding = folding;
    out.vel_xz = vel_xz;
    out.vel_y = vel_y;
    return out;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= BIOLUM_PARTICLE_COUNT) { return; }

    var p = particles[idx];
    let dt = clamp(audio.frame_dt, 0.001, 0.033);
    let time = select(audio.time, audio.smooth_time, audio.smooth_time > 0.0);
    let idx_f = f32(idx);
    let kind = idx & 7u;

    let aud = sample_biolum_audio();
    let bass = aud.x;
    let mids = aud.y;
    let treble = aud.z;
    let stereo_bias = aud.w;

    let amp_scale = min(1.0 + bass * 0.28 + mids * 0.08, 1.42);
    let qa_scale = min(1.0 + bass * 0.16, 1.20);

    let aspect = max(audio.aspect_ratio, 0.5);
    let x_half = clamp(24.0 * max(aspect * 0.56, 1.0), 24.0, 48.0);
    let z_min = -11.0;
    let z_max = 46.0;
    let z_span = z_max - z_min;

    p.pos.w -= dt;

    let uninitialized = p.pos.x == 0.0 && p.pos.y == 0.0 && p.pos.z == 0.0;
    if (p.pos.w <= 0.0 || uninitialized) {
        let seed = fract(idx_f * 0.000731 + fract(time * 0.13) * 17.3);
        let rnd = hash31(idx_f * 0.37 + seed * 97.0);
        let rnd2 = hash31(idx_f * 0.71 + 19.3);

        let sx = rnd.x * 2.0 - 1.0;
        let pos_x = (0.72 * sx + 0.28 * sx * sx * sx) * x_half;
        let sz = rnd.y;
        let pos_z = mix(z_min, z_max, sz * (0.72 + 0.28 * sz));

        let surf = eval_gerstner_surface(vec2<f32>(pos_x, pos_z), time, amp_scale, qa_scale);
        let y_off = select(0.032, 0.14, kind == 7u);

        let radial_dist = length(vec2<f32>(pos_x, pos_z - 6.0));
        let wave_ring = 0.5 + 0.5 * sin(radial_dist * 0.45 - time * 1.8);
        let crest_bias = smoothstep(0.08, 0.85, surf.y);
        let spawn_excite = surf.folding * 1.15 + (bass * (0.35 + 0.65 * wave_ring) + mids * 0.55) * crest_bias;

        var init_energy = 0.025;
        if (kind == 7u) {
            init_energy = select(0.0, clamp(surf.folding * 1.15 + bass * 0.35, 0.0, 1.35), surf.folding > 0.24 || (bass > 0.40 && surf.y > 0.20));
        } else {
            init_energy = clamp(0.025 + spawn_excite * 0.95, 0.025, 1.85);
        }

        p.pos = vec4<f32>(pos_x, surf.y + y_off, pos_z, 3.0 + rnd2.z * 6.0);
        p.vel = vec4<f32>(surf.vel_xz.x, 0.0, surf.vel_xz.y, init_energy);
    } else {
        var pos = p.pos.xyz;
        var vel = p.vel.xyz;
        var energy = p.vel.w;

        let surf = eval_gerstner_surface(pos.xz, time, amp_scale, qa_scale);

        let radial_dist = length(pos.xz - vec2<f32>(0.0, 6.0));
        let wave_ring = 0.5 + 0.5 * sin(radial_dist * 0.45 - time * 1.8);
        let side_x = clamp(pos.x / 18.0, -1.0, 1.0);
        let stereo_excite = max(0.0, side_x * stereo_bias) * 0.45;
        let crest_bias = smoothstep(0.08, 0.85, surf.y);
        let audio_excite = bass * (0.35 + 0.65 * wave_ring) + mids * crest_bias * 0.65 + treble * 0.35 + stereo_excite;

        if (kind == 5u || kind == 6u) {
            // Archetype B: Breaking-crest sea-spray droplets with ballistic arc & splash
            let airborne = pos.y > (surf.y + 0.08) || vel.y > 0.05;
            if (airborne) {
                vel.y -= 6.2 * dt;
                pos += vel * dt;
                energy = max(energy - dt * 0.38, 0.25);

                if (pos.y <= surf.y + 0.04) {
                    pos.y = surf.y + 0.04;
                    vel.y = 0.0;
                    vel.x *= 0.4;
                    vel.z *= 0.4;
                    energy = max(energy, 1.35);
                }
            } else {
                let target_vel = vec3<f32>(surf.vel_xz.x, 0.0, surf.vel_xz.y);
                vel = mix(vel, target_vel, clamp(dt * 8.5, 0.03, 0.38));
                pos.x += vel.x * dt;
                pos.z += vel.z * dt;
                pos.y = surf.y + 0.04;
                vel.y = 0.0;
                energy = max(energy - dt * 0.55, 0.025);

                let rnd_launch = hash31(idx_f * 0.53 + fract(time * 0.37) * 61.0);
                let launch_score = surf.folding + (bass * 0.42 + treble * 0.35) * smoothstep(0.15, 0.75, surf.y);
                if (launch_score > 0.68 && rnd_launch.x > 0.78) {
                    vel.y = 1.65 + rnd_launch.y * 1.85 + bass * 0.75;
                    vel.x = surf.vel_xz.x * 0.65 + (rnd_launch.z - 0.5) * 1.4;
                    vel.z = surf.vel_xz.y * 0.85 - (0.65 + rnd_launch.x * 0.7);
                    pos.y = surf.y + 0.12;
                    energy = max(energy, 1.75 + treble * 0.55);
                } else if (surf.folding > 0.34) {
                    energy = max(energy, clamp(surf.folding * 1.35 + treble * 0.35, 0.0, 2.0));
                }
            }
        } else if (kind == 7u) {
            // Archetype C: Soft crest-hugging bioluminescent halo wisps (silent in calm troughs)
            let target_vel = vec3<f32>(surf.vel_xz.x * 0.85, 0.0, surf.vel_xz.y * 0.85);
            vel = mix(vel, target_vel, clamp(dt * 6.5, 0.02, 0.30));
            pos.x += vel.x * dt;
            pos.z += vel.z * dt;
            pos.y = surf.y + 0.14;
            vel.y = 0.0;

            energy = max(energy - dt * 0.65, 0.0);
            if (surf.folding > 0.25 || (bass > 0.45 && surf.y > 0.25)) {
                energy = max(energy, clamp(surf.folding * 1.25 + bass * 0.45, 0.0, 1.5));
            }
        } else {
            // Archetype A: Surface dinoflagellate plankton & lacy foam filaments
            let curl_phase = pos.xz * 0.18 + vec2<f32>(time * 0.12, -time * 0.09);
            let flow_drift = vec3<f32>(cos(curl_phase.y) * 0.32, 0.0, sin(curl_phase.x) * 0.24 - 0.18);
            let target_vel = vec3<f32>(surf.vel_xz.x, 0.0, surf.vel_xz.y) + flow_drift;
            vel = mix(vel, target_vel, clamp(dt * 8.0, 0.02, 0.35));

            let foam_noise = fbm2d_foam_fast(pos.xz * 1.35 + vec2<f32>(time * 0.38, time * 0.25));
            let shear = surf.folding * (0.70 + 0.75 * foam_noise) + crest_bias * foam_noise * 0.48;
            let trigger = shear * (0.90 + 0.55 * mids) + audio_excite * (0.45 + 0.65 * crest_bias + 0.40 * surf.folding);

            energy = max(energy - dt * 0.48, 0.025);
            if (trigger > 0.36) {
                energy = max(energy, clamp(trigger * 1.45, 0.0, 2.2));
            }

            if (surf.folding > 0.24) {
                let rnd_foam = hash31(idx_f * 0.41 + fract(time * 0.29) * 43.0);
                pos.x += (rnd_foam.x - 0.5) * 0.16 * surf.folding * foam_noise;
                pos.z += (rnd_foam.z - 0.5) * 0.16 * surf.folding * foam_noise;
            }

            pos.x += vel.x * dt;
            pos.z += vel.z * dt;
            pos.y = surf.y + 0.032;
            vel.y = 0.0;
        }

        // Seamless spatial wrap-around within camera-visible ocean bounds
        if (pos.x > x_half) {
            pos.x -= 2.0 * x_half;
        } else if (pos.x < -x_half) {
            pos.x += 2.0 * x_half;
        }
        if (pos.z < z_min) {
            pos.z += z_span;
        } else if (pos.z > z_max) {
            pos.z -= z_span;
        }

        p.pos = vec4<f32>(pos, p.pos.w);
        p.vel = vec4<f32>(vel, energy);
    }

    particles[idx] = p;
}
