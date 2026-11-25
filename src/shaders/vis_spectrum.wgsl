// INCLUDE: common

@group(0) @binding(0)
var<uniform> audio: AudioUniforms;

fn val_to_color(val: f32) -> vec3<f32> {
    let v = clamp(val, 0.0, 100.0);
    if v < 5.0 {
        return vec3<f32>(0.08, 0.08, 0.1);
    } else if v < 20.0 {
        return vec3<f32>(0.31, 0.08, 0.08);
    } else if v < 40.0 {
        return vec3<f32>(0.7, 0.12, 0.08);
    } else if v < 60.0 {
        return vec3<f32>(1.0, 0.39, 0.08);
    } else if v < 85.0 {
        return vec3<f32>(1.0, 0.78, 0.2);
    } else {
        return vec3<f32>(1.0, 1.0, 1.0);
    }
}

fn sample_bar_amplitude(bar_idx: u32, num_bars: u32) -> f32 {
    let b0 = (bar_idx * 1024u) / num_bars;
    let b1 = max(b0 + 1u, ((bar_idx + 1u) * 1024u) / num_bars);

    let v0 = min(b0 >> 2u, 255u);
    let v1 = min((b1 - 1u) >> 2u, 255u);
    let vec_count = min(v1 - v0 + 1u, 8u);

    var max_amp = 0.0;
    for (var i = 0u; i < vec_count; i = i + 1u) {
        let sv = audio.spectrum[v0 + i];
        max_amp = max(max_amp, max(max(sv.x, sv.y), max(sv.z, sv.w)));
    }
    return max_amp;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let duv = max(fwidth(in.uv), vec2<f32>(1.0 / 8192.0));
    let vp_px = vec2<f32>(1.0) / duv;
    let num_bars_f = clamp(floor(vp_px.x / 8.0), 48.0, 256.0);
    let num_leds_f = clamp(floor(vp_px.y / 8.0), 32.0, 160.0);

    let bar_f = clamp(in.uv.x, 0.0, 0.9999) * num_bars_f;
    let bar_idx = u32(floor(bar_f));
    let bar_uv_x = fract(bar_f);
    let amplitude = sample_bar_amplitude(bar_idx, u32(num_bars_f));

    if bar_uv_x < 0.1 || bar_uv_x > 0.9 {
        return vec4<f32>(0.02, 0.02, 0.03, 1.0);
    }

    let led_y = fract((1.0 - in.uv.y) * num_leds_f);
    if led_y < 0.15 || led_y > 0.85 {
        return vec4<f32>(0.02, 0.02, 0.03, 1.0);
    }

    if (1.0 - in.uv.y) < (amplitude / 100.0) {
        return vec4<f32>(val_to_color(amplitude), 1.0);
    } else {
        return vec4<f32>(0.05, 0.05, 0.06, 1.0);
    }
}

