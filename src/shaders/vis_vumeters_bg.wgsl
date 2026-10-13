// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = audio.aspect_ratio;
    let p = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, -1.0);

    // Warm vintage studio acoustic walnut wood slat wall background with fine grain
    let slat_w = 0.08;
    let slat_id = floor(p.x / slat_w);
    let slat_phase = fract(p.x / slat_w);
    let grain = value_noise2d(vec2<f32>(p.x * 45.0, p.y * 8.0 + slat_id * 3.7)) * 0.025;
    let wood_dark = vec3<f32>(0.025, 0.010, 0.005);
    let wood_walnut = vec3<f32>(0.095, 0.046, 0.022) + vec3<f32>(grain, grain * 0.5, grain * 0.2);
    let slat = smoothstep(0.08, 0.20, slat_phase) * smoothstep_r(0.92, 0.80, slat_phase);
    var bg = mix(wood_dark, wood_walnut, slat);

    // Studio soft key light vignette + warm incandescent spill from VU meters
    let vig = 1.0 - smoothstep(0.4, 1.3, length(p));
    let bass_warmth = clamp((audio.spectrum[1].x + audio.spectrum[2].x) / 140.0, 0.0, 1.0);
    let vu_energy = max(audio.channels[0].x + audio.channels[0].y, bass_warmth * 1.4);
    let ambient_glow = vec3<f32>(0.28, 0.16, 0.07) * (0.8 + vu_energy * 0.22);
    
    var color = bg * vig * 0.78 + ambient_glow * vig * 0.25;

    let tonemapped = aces_tonemap(color);
    return vec4<f32>(tonemapped, 1.0);
}
