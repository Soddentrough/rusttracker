// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

fn hash1(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }
fn hash2(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash2(i + vec2<f32>(0.0, 0.0)), hash2(i + vec2<f32>(1.0, 0.0)), u.x),
               mix(hash2(i + vec2<f32>(0.0, 1.0)), hash2(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var shift = vec2<f32>(100.0);
    var p_mut = p;
    for (var i = 0; i < 4; i = i + 1) {
        v = v + a * noise(p_mut);
        p_mut = p_mut * 2.0 + shift;
        a = a * 0.5;
    }
    return v;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = audio.aspect_ratio;
    let p = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, -1.0);

    let bass_pulse = clamp(audio.spectrum[2].x * 1.6, 0.0, 1.0);
    let treble_pulse = clamp(audio.spectrum[80].x * 1.8, 0.0, 1.0);

    // =========================================================================
    // 1. PURE DARK THUNDERSTORM SKY & ROLLING CLOUD ATMOSPHERE
    // =========================================================================
    let uv_cloud = vec2<f32>(p.x * 1.8 + audio.smooth_time * 0.04, p.y * 1.8 + audio.smooth_time * 0.02);
    let cloud_density = fbm(uv_cloud);

    let dark_sky = vec3<f32>(0.008, 0.010, 0.018);
    let cloud_base = vec3<f32>(0.018, 0.022, 0.035);
    var sky_col = mix(dark_sky, cloud_base, cloud_density);

    // Vignette from center
    let vig = 1.0 - smoothstep(0.4, 1.4, length(p));
    sky_col = sky_col * (0.8 + 0.2 * vig);

    // =========================================================================
    // 2. AUDIO-REACTIVE BRANCHING LIGHTNING FLASHES
    // =========================================================================
    let is_lightning = step(0.68, bass_pulse);
    let flash_intensity = max(0.0, (bass_pulse - 0.68) / 0.32);

    if (is_lightning > 0.5) {
        // Main jagged lightning bolt
        let bolt_seed = floor(audio.smooth_time * 10.0);
        let bolt_x = sin(p.y * 16.0 + audio.smooth_time * 60.0) * 0.15 + (hash1(bolt_seed) - 0.5) * 0.9;
        let dist_bolt = abs(p.x - bolt_x);
        let bolt_core = smoothstep(0.015, 0.0, dist_bolt);
        let bolt_glow = 1.0 / (1.0 + dist_bolt * 30.0);

        let lightning_col = vec3<f32>(0.88, 0.94, 1.0) * (bolt_core * 6.0 + bolt_glow * 3.0) * flash_intensity;
        let ambient_flash = vec3<f32>(0.35, 0.45, 0.65) * flash_intensity * (0.6 + cloud_density * 0.4);

        sky_col += lightning_col + ambient_flash;
    }

    let tonemapped = aces_tonemap(sky_col);
    return vec4<f32>(tonemapped, 1.0);
}
