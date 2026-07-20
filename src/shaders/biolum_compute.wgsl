// INCLUDE: common

struct Particle {
    pos: vec4<f32>, // pos.xyz = position, pos.w = life
    vel: vec4<f32>, // vel.xyz = velocity, vel.w = energy
}

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

@group(0) @binding(1)
var<storage, read_write> particles: array<Particle>;

struct GerstnerWave {
    direction: vec2<f32>,
    amplitude: f32,
    wavelength: f32,
    speed: f32,
    steepness: f32,
}

fn hash11(p: f32) -> f32 {
    var p2 = fract(p * 0.1031);
    p2 = p2 * (p2 + 33.33);
    return fract(2.0 * p2 * p2);
}

fn hash31(p: f32) -> vec3<f32> {
    var p3 = fract(vec3<f32>(p) * vec3<f32>(0.1031, 0.11369, 0.13787));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xxy + p3.yzz) * p3.zyx);
}

fn noise2d(p: vec2<f32>) -> f32 {
    let ip = floor(p);
    let fp = fract(p);
    
    let u = fp * fp * (3.0 - 2.0 * fp);
    
    let a = hash11(ip.x + ip.y * 57.0);
    let b = hash11(ip.x + 1.0 + ip.y * 57.0);
    let c = hash11(ip.x + (ip.y + 1.0) * 57.0);
    let d = hash11(ip.x + 1.0 + (ip.y + 1.0) * 57.0);
    
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm2d(p: vec2<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var pos = p;
    for (var i = 0; i < 3; i = i + 1) {
        value += amplitude * noise2d(pos);
        pos = pos * 2.0;
        amplitude = amplitude * 0.5;
    }
    return value;
}

fn get_gerstner_wave(wave: GerstnerWave, xz: vec2<f32>, time: f32) -> vec3<f32> {
    let k = 2.0 * 3.14159265 / wave.wavelength;
    let c = wave.speed;
    let w = c * k;
    let dir = normalize(wave.direction);
    let theta = k * dot(dir, xz) - w * time;
    
    let cos_t = cos(theta);
    let sin_t = sin(theta);
    
    let q = wave.steepness / (wave.amplitude * k + 0.0001);
    
    let dx = q * wave.amplitude * dir.x * cos_t;
    let dz = q * wave.amplitude * dir.y * cos_t;
    let dy = wave.amplitude * sin_t;
    
    return vec3<f32>(dx, dy, dz);
}

fn get_wave_displacement(xz: vec2<f32>, time: f32) -> vec3<f32> {
    var displacement = vec3<f32>(0.0);
    
    var waves: array<GerstnerWave, 5>;
    waves[0] = GerstnerWave(vec2<f32>(0.1, 0.99), 0.55, 16.0, 2.2, 0.7);
    waves[1] = GerstnerWave(vec2<f32>(0.6, 0.8), 0.35, 8.5, 1.6, 0.55);
    waves[2] = GerstnerWave(vec2<f32>(-0.65, 0.75), 0.25, 5.0, 1.3, 0.45);
    waves[3] = GerstnerWave(vec2<f32>(0.9, 0.4), 0.1, 2.8, 1.0, 0.35);
    waves[4] = GerstnerWave(vec2<f32>(-0.8, 0.5), 0.06, 1.5, 0.8, 0.25);
    
    for (var i = 0; i < 5; i = i + 1) {
        displacement += get_gerstner_wave(waves[i], xz, time);
    }
    
    return displacement;
}

fn get_wave_velocity(xz: vec2<f32>, time: f32) -> vec3<f32> {
    var velocity = vec3<f32>(0.0);
    
    var waves: array<GerstnerWave, 5>;
    waves[0] = GerstnerWave(vec2<f32>(0.1, 0.99), 0.55, 16.0, 2.2, 0.7);
    waves[1] = GerstnerWave(vec2<f32>(0.6, 0.8), 0.35, 8.5, 1.6, 0.55);
    waves[2] = GerstnerWave(vec2<f32>(-0.65, 0.75), 0.25, 5.0, 1.3, 0.45);
    waves[3] = GerstnerWave(vec2<f32>(0.9, 0.4), 0.1, 2.8, 1.0, 0.35);
    waves[4] = GerstnerWave(vec2<f32>(-0.8, 0.5), 0.06, 1.5, 0.8, 0.25);
    
    for (var i = 0; i < 5; i = i + 1) {
        let wave = waves[i];
        let k = 2.0 * 3.14159265 / wave.wavelength;
        let c = wave.speed;
        let w = c * k;
        let dir = normalize(wave.direction);
        let theta = k * dot(dir, xz) - w * time;
        
        let sin_t = sin(theta);
        let cos_t = cos(theta);
        
        let q = wave.steepness / (wave.amplitude * k + 0.0001);
        
        let vx = q * wave.amplitude * dir.x * w * sin_t;
        let vz = q * wave.amplitude * dir.y * w * sin_t;
        let vy = -w * wave.amplitude * cos_t;
        
        velocity += vec3<f32>(vx, vy, vz);
    }
    
    return velocity;
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
        let disp = get_wave_displacement(vec2<f32>(pos_x, pos_z), time);
        
        p.pos = vec4<f32>(pos_x + disp.x, disp.y, pos_z + disp.z, rnd.z * 4.0 + 1.0); // life: 1.0 to 5.0s
        p.vel = vec4<f32>((rnd.x - 0.5) * 0.4, 0.0, (rnd.y - 0.5) * 0.4, 0.12); // energy starts at ground state
    } else {
        var pos = p.pos.xyz;
        var vel = p.vel.xyz;
        var energy = p.vel.w;
        
        // Decay energy glow slowly (takes ~2-3 seconds to fade to ground state of 0.12)
        energy = max(energy - dt * 0.5, 0.12);
        
        // Local wave physics at particle coordinate
        let disp = get_wave_displacement(pos.xz, time);
        let wave_h = disp.y;
        
        // Smoothly interpolate towards the true orbital wave velocity + drift current
        let wave_vel = get_wave_velocity(pos.xz, time);
        let drift_vel = vec3<f32>(0.15, 0.0, 0.0); // slow global current drift
        let target_vel = wave_vel + drift_vel;
        
        // High drag keeps particles flowing smoothly with currents rather than zipping around
        // (0.15 per 16ms step, converted to the real dt)
        vel = mix(vel, target_vel, 1.0 - pow(0.85, dt / 0.016));
        
        // Audio reactivity - Bass agitation triggers glow directly
        let bass = max(audio.spectrum[0].x, audio.spectrum[1].x) * 0.02;
        if (bass > 0.15) {
            energy = max(energy, clamp(bass * 1.5, 0.0, 2.0));
            
            // Add a tiny localized horizontal puff (agitation), not a high-velocity launch
            let rnd_impulse = hash31(idx_f + time);
            pos.x += (rnd_impulse.x - 0.5) * bass * 0.08;
            pos.z += (rnd_impulse.y - 0.5) * bass * 0.08;
        }
        
        // Integrate horizontal position
        pos.x += vel.x * dt;
        pos.z += vel.z * dt;
        
        // Snap vertical position to the surface wave height
        // (reuse disp from above — the field is smooth and the horizontal
        // integration step is tiny, so a second evaluation is wasted work)
        pos.y = wave_h;
        vel.y = 0.0;
        
        // Wave breaks and foam structure simulation (active at crests)
        let crest_factor = smoothstep(0.35, 1.2, wave_h);
        if (crest_factor > 0.0) {
            // High frequency fBm noise creates organic foam lines and breaks
            let noise_coord = pos.xz * 2.5 + vec2<f32>(time * 1.5, time * 1.0);
            let foam_noise = fbm2d(noise_coord);
            
            // horizontal micro-agitation/diffusion clusters particles into foam filaments
            let foam_disp = vec3<f32>(
                hash11(idx_f + time) - 0.5,
                0.0,
                hash11(idx_f + time * 1.3) - 0.5
            ) * 0.22 * crest_factor * foam_noise;
            pos += foam_disp;
            
            // Excite glow in the breaking foam zones
            let shear_agitation = foam_noise * 1.3 * crest_factor;
            if (shear_agitation > 0.3) {
                energy = max(energy, 1.6);
            }
        }
        
        p.pos = vec4<f32>(pos, p.pos.w);
        p.vel = vec4<f32>(vel, energy);
    }
    
    particles[idx] = p;
}
