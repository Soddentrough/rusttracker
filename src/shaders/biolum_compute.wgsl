// INCLUDE: common

struct Particle {
    pos: vec4<f32>, // pos.xyz = position, pos.w = life
    vel: vec4<f32>, // vel.xyz = velocity, vel.w = energy
}

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read_write> particles: array<Particle>;

// Pre-normalized Gerstner wave parameters:
// vec4(dir.x, dir.y, k = 2*PI/wavelength, w = speed * k)
const WAVE_KINEMATICS: array<vec4<f32>, 5> = array<vec4<f32>, 5>(
    vec4<f32>(0.100499, 0.994937, 0.392699, 0.863938),
    vec4<f32>(0.600000, 0.800000, 0.739198, 1.182717),
    vec4<f32>(-0.654931, 0.755689, 1.256637, 1.633628),
    vec4<f32>(0.913812, 0.406138, 2.243995, 2.243995),
    vec4<f32>(-0.847998, 0.529999, 4.188790, 3.351032)
);

// vec2(amplitude, q * amplitude = steepness / k)
const WAVE_AMPS: array<vec2<f32>, 5> = array<vec2<f32>, 5>(
    vec2<f32>(0.55, 1.782536),
    vec2<f32>(0.35, 0.744050),
    vec2<f32>(0.25, 0.358099),
    vec2<f32>(0.10, 0.155972),
    vec2<f32>(0.06, 0.059683)
);

fn hash31(p: f32) -> vec3<f32> {
    var p3 = fract(vec3<f32>(p) * vec3<f32>(0.1031, 0.11369, 0.13787));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xxy + p3.yzz) * p3.zyx);
}

// Evaluates both 3D Gerstner wave displacement (col 0) and 3D orbital velocity (col 1)
// in a single pass with pre-normalized directions and wavenumbers (halving trig & sqrt ops).
fn eval_gerstner_waves(xz: vec2<f32>, time: f32) -> mat2x3<f32> {
    var disp = vec3<f32>(0.0);
    var vel = vec3<f32>(0.0);

    for (var i = 0u; i < 5u; i = i + 1u) {
        let kin = WAVE_KINEMATICS[i];
        let amp = WAVE_AMPS[i];
        let dir = kin.xy;
        let k = kin.z;
        let w = kin.w;

        let theta = k * dot(dir, xz) - w * time;
        let sin_t = sin(theta);
        let cos_t = cos(theta);

        let qa = amp.y;
        let a = amp.x;

        disp += vec3<f32>(qa * dir.x * cos_t, a * sin_t, qa * dir.y * cos_t);
        vel += vec3<f32>(qa * dir.x * w * sin_t, -w * a * cos_t, qa * dir.y * w * sin_t);
    }

    return mat2x3<f32>(disp, vel);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= 65536u) { return; }

    var p = particles[idx];
    // Framerate-independent timestep (real frame dt, clamped)
    let dt = clamp(audio.frame_dt, 0.001, 0.033);
    let time = audio.smooth_time;
    let idx_f = f32(idx);
    
    // Decay life
    p.pos.w -= dt;
    
    // Re-spawn if life is depleted or coordinate is uninitialized
    if (p.pos.w <= 0.0 || (p.pos.x == 0.0 && p.pos.y == 0.0 && p.pos.z == 0.0)) {
        let rnd = hash31(idx_f + time * 13.0);
        let pos_x = (rnd.x - 0.5) * 50.0;
        let pos_z = (rnd.y - 0.5) * 50.0;
        let wave_state = eval_gerstner_waves(vec2<f32>(pos_x, pos_z), time);
        let disp = wave_state[0];
        
        p.pos = vec4<f32>(pos_x + disp.x, disp.y, pos_z + disp.z, rnd.z * 4.0 + 1.0); // life: 1.0 to 5.0s
        p.vel = vec4<f32>((rnd.x - 0.5) * 0.4, 0.0, (rnd.y - 0.5) * 0.4, 0.12); // energy starts at ground state
    } else {
        var pos = p.pos.xyz;
        var vel = p.vel.xyz;
        var energy = p.vel.w;
        
        // Decay energy glow slowly (takes ~2-3 seconds to fade to ground state of 0.12)
        energy = max(energy - dt * 0.55, 0.12);
        
        // Evaluate local wave displacement and orbital velocity in a single pass
        let wave_state = eval_gerstner_waves(pos.xz, time);
        let disp = wave_state[0];
        let wave_vel = wave_state[1];
        let wave_h = disp.y;
        
        // Smoothly interpolate towards the true orbital wave velocity + drift current
        let drift_vel = vec3<f32>(0.15, 0.0, 0.0); // slow global current drift
        let target_vel = wave_vel + drift_vel;
        
        // High drag keeps particles flowing smoothly with currents rather than zipping around
        vel = mix(vel, target_vel, 1.0 - pow(0.85, dt / 0.016));
        
        // Multi-band audio reactivity (normalized from 0..100 spectrum range)
        let bass = clamp(max(audio.spectrum[0].x, max(audio.spectrum[1].x, audio.spectrum[2].x)) / 75.0, 0.0, 1.4);
        let mids = clamp((audio.spectrum[12].x + audio.spectrum[20].x) / 140.0, 0.0, 1.2);
        let treble = clamp((audio.spectrum[55].x + audio.spectrum[75].x) / 130.0, 0.0, 1.2);

        // Radial acoustic pressure waves propagating across the ocean swells
        let radial_dist = length(pos.xz);
        let wave_ring = 0.5 + 0.5 * sin(radial_dist * 0.55 - time * 5.5);
        let crest_bias = smoothstep(-0.2, 0.75, wave_h);
        let audio_excite = (bass * (0.45 + 0.55 * wave_ring) + mids * crest_bias * 0.65 + treble * 0.35);

        if (audio_excite > 0.18) {
            energy = max(energy, clamp(audio_excite * 1.55, 0.0, 2.0));
            
            // Localized horizontal shear agitation on strong bass hits
            let rnd_impulse = hash31(idx_f + time);
            pos.x += (rnd_impulse.x - 0.5) * bass * 0.09;
            pos.z += (rnd_impulse.y - 0.5) * bass * 0.09;
        }
        
        // Integrate horizontal position
        pos.x += vel.x * dt;
        pos.z += vel.z * dt;
        
        // Snap vertical position to the surface wave height
        pos.y = wave_h;
        vel.y = 0.0;
        
        // Wave breaks and dinoflagellate foam structure simulation (active at crests)
        let crest_factor = smoothstep(0.30, 1.15, wave_h);
        if (crest_factor > 0.0) {
            // High frequency fBm noise creates organic foam lines and breaks
            let noise_coord = pos.xz * 2.5 + vec2<f32>(time * 1.5, time * 1.0);
            let foam_noise = fbm2d_4(noise_coord);
            
            // Horizontal micro-agitation/diffusion clusters particles into foam filaments
            let foam_disp = vec3<f32>(
                hash11_common(idx_f + time) - 0.5,
                0.0,
                hash11_common(idx_f + time * 1.3) - 0.5
            ) * 0.22 * crest_factor * foam_noise;
            pos += foam_disp;
            
            // Excite glow in the breaking foam zones, boosted by treble/mids
            let shear_agitation = foam_noise * (1.25 + mids * 0.45) * crest_factor;
            if (shear_agitation > 0.28) {
                energy = max(energy, 1.55 + treble * 0.45);
            }
        }
        
        p.pos = vec4<f32>(pos, p.pos.w);
        p.vel = vec4<f32>(vel, energy);
    }
    
    particles[idx] = p;
}
