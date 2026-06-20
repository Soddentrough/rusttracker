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
    
    var waves: array<GerstnerWave, 3>;
    waves[0] = GerstnerWave(vec2<f32>(0.0, 1.0), 0.6, 14.0, 2.5, 0.5);
    waves[1] = GerstnerWave(vec2<f32>(0.7, 0.7), 0.35, 7.0, 1.8, 0.4);
    waves[2] = GerstnerWave(vec2<f32>(-0.7, 0.7), 0.15, 3.5, 1.2, 0.3);
    
    for (var i = 0; i < 3; i = i + 1) {
        displacement += get_gerstner_wave(waves[i], xz, time);
    }
    
    return displacement;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= 65536u) { return; }

    var p = particles[idx];
    let dt = 0.016;
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
        
        p.pos = vec4<f32>(pos_x + disp.x, disp.y + rnd.z * 0.4, pos_z + disp.z, rnd.z * 4.0 + 1.0); // life: 1.0 to 5.0s
        p.vel = vec4<f32>((rnd.x - 0.5) * 0.4, 0.0, (rnd.y - 0.5) * 0.4, rnd.x * 0.2); // energy
    } else {
        var pos = p.pos.xyz;
        var vel = p.vel.xyz;
        var energy = p.vel.w;
        
        let disp = get_wave_displacement(pos.xz, time);
        let wave_h = disp.y;
        
        var force = vec3<f32>(0.0);
        
        // Fluid vs Air Physics
        if (pos.y <= wave_h + 0.1) {
            // Under/on the water surface
            let depth = wave_h - pos.y;
            // Buoyancy force
            force.y += max(depth, 0.0) * 35.0;
            // Align back to wave displacement slightly
            force.x += (disp.x - (pos.x - (pos.x - disp.x))) * 0.5;
            force.z += (disp.z - (pos.z - (pos.z - disp.z))) * 0.5;
            
            // Drag
            vel *= 0.93;
            
            // Flow field (currents + curl noise simulation)
            let wave_current = vec3<f32>(disp.x, 0.0, disp.z) * 1.5;
            force += wave_current;
            
            let swell = vec3<f32>(
                sin(pos.z * 0.4 + time) * 1.2,
                cos(pos.x * 0.4 - time) * 0.3,
                cos(pos.y * 0.4 + time) * 1.2
            );
            force += swell;
            
            // Audio reactivity - Bass disruption
            // In WGPU, AudioUniforms.spectrum holds values from 0.0 to ~100.0
            let bass = max(audio.spectrum[0].x, audio.spectrum[1].x) * 0.02;
            if (bass > 0.15) {
                let rnd_impulse = hash31(idx_f + time);
                vel.y += bass * 4.0 * rnd_impulse.z;
                vel.x += (rnd_impulse.x - 0.5) * bass * 2.0;
                vel.z += (rnd_impulse.y - 0.5) * bass * 2.0;
                energy = clamp(max(energy, bass * 2.0), 0.0, 1.5);
            }
            
            // Glow when near wave crests due to high shear stress
            let crest = smoothstep(0.1, 0.8, wave_h);
            energy = max(energy, crest * 0.95);
        } else {
            // In the air (sea spray)
            force.y -= 9.8; // Gravity
            force.x += 1.5; // Wind blowing right
            vel *= 0.98;    // Air resistance
        }
        
        // Decay energy glow
        energy = max(energy - dt * 0.25, 0.0);
        
        // Integrate
        vel += force * dt;
        pos += vel * dt;
        
        p.pos = vec4<f32>(pos, p.pos.w);
        p.vel = vec4<f32>(vel, energy);
    }
    
    particles[idx] = p;
}
