// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = audio.aspect_ratio;
    let p = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, -1.0);

    // Warm vintage studio acoustic wood slat wall background
    let slat_w = 0.08;
    let slat_phase = fract(p.x / slat_w);
    let wood_dark = vec3<f32>(0.035, 0.015, 0.008);
    let wood_walnut = vec3<f32>(0.09, 0.045, 0.022);
    let slat = smoothstep(0.08, 0.20, slat_phase) * smoothstep(0.92, 0.80, slat_phase);
    var bg = mix(wood_dark, wood_walnut, slat);

    // Studio soft key light vignette
    let vig = 1.0 - smoothstep(0.4, 1.3, length(p));
    let ambient_glow = vec3<f32>(0.25, 0.15, 0.08) * (0.8 + (audio.channels[0].x + audio.channels[0].y) * 0.15);
    
    var color = bg * vig * 0.75 + ambient_glow * vig * 0.25;

    let tonemapped = aces_tonemap(color);
    return vec4<f32>(tonemapped, 1.0);
}
