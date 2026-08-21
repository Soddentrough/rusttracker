// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = audio.aspect_ratio;
    let p = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, -1.0);

    let bass_pulse = clamp(audio.spectrum[2].x * 1.5, 0.0, 1.0);

    // Deep dark acoustic room background
    let r = length(p);
    let vig = 1.0 - smoothstep(0.3, 1.3, r);
    
    let base_dark = vec3<f32>(0.003, 0.003, 0.006);
    let center_glow = vec3<f32>(0.015, 0.008, 0.025) * (1.0 + bass_pulse * 1.5) * vig;

    var color = base_dark + center_glow;

    let tonemapped = aces_tonemap(color);
    return vec4<f32>(tonemapped, 1.0);
}
