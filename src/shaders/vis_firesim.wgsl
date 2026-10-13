// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(3) var fire_grid_tex: texture_2d<f32>;
@group(0) @binding(4) var<storage, read> multi_spectrum: array<vec2<f32>>;

// INCLUDE: glyph_font

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)),
                      dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453) * 2.0 - 1.0;
}

fn perlin_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);

    let a = dot(hash22(i + vec2<f32>(0.0, 0.0)), f - vec2<f32>(0.0, 0.0));
    let b = dot(hash22(i + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(hash22(i + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(hash22(i + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    let shift = vec2<f32>(100.0);
    var p2 = p;
    let rot = mat2x2<f32>(0.866, -0.5, 0.5, 0.866);
    for (var i = 0; i < 4; i = i + 1) {
        v += a * perlin_noise(p2);
        p2 = rot * p2 * 2.0 + shift;
        a *= 0.5;
    }
    return v;
}

fn get_base_heat(uv: vec2<f32>) -> f32 {
    let tex_size = vec2<f32>(1024.0, 576.0);
    let p = uv * tex_size - 0.5;
    let ip = floor(p);
    let fp = fract(p);

    let x0 = clamp(i32(ip.x), 0, 1023);
    let x1 = clamp(x0 + 1, 0, 1023);
    let y0 = clamp(i32(ip.y), 0, 575);
    let y1 = clamp(y0 + 1, 0, 575);

    let h00 = textureLoad(fire_grid_tex, vec2<i32>(x0, y0), 0).r;
    let h10 = textureLoad(fire_grid_tex, vec2<i32>(x1, y0), 0).r;
    let h01 = textureLoad(fire_grid_tex, vec2<i32>(x0, y1), 0).r;
    let h11 = textureLoad(fire_grid_tex, vec2<i32>(x1, y1), 0).r;

    return mix(mix(h00, h10, fp.x), mix(h01, h11, fp.x), fp.y);
}

fn blackbody(temperature: f32) -> vec3<f32> {
    let t = clamp(temperature, 0.0, 1.0);
    let c1 = vec3<f32>(0.0, 0.0, 0.0);
    let c2 = vec3<f32>(0.5, 0.05, 0.0);
    let c3 = vec3<f32>(0.9, 0.25, 0.0);
    let c4 = vec3<f32>(1.0, 0.6, 0.05);
    let c5 = vec3<f32>(1.0, 0.85, 0.3);
    let c6 = vec3<f32>(1.0, 1.0, 0.85);

    var color = c1;
    color = mix(color, c2, smoothstep(0.0,  0.15, t));
    color = mix(color, c3, smoothstep(0.15, 0.30, t));
    color = mix(color, c4, smoothstep(0.30, 0.50, t));
    color = mix(color, c5, smoothstep(0.50, 0.75, t));
    color = mix(color, c6, smoothstep(0.75, 1.0,  t));
    return color;
}

fn get_spectrum_val(idx: u32) -> f32 {
    return audio.spectrum[idx >> 2u][idx & 3u];
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var bass_sum = 0.0;
    for (var b = 0u; b < 64u; b += 8u) { bass_sum += get_spectrum_val(b); }
    let bass = clamp(bass_sum / 8.0 / 80.0, 0.0, 1.0);

    var highs_sum = 0.0;
    for (var b = 512u; b < 1024u; b += 32u) { highs_sum += get_spectrum_val(b); }
    let highs = clamp(highs_sum / 16.0 / 60.0, 0.0, 1.0);

    let time = audio.smooth_time;
    let uv = in.uv;
    let aspect = max(audio.aspect_ratio, 0.01);
    let uv_c = vec2<f32>(uv.x * aspect, uv.y);

    let h_up = 1.0 - uv.y;
    let sway = (sin(time * 0.9 + h_up * 3.0) + 0.5 * sin(time * 1.7 + h_up * 5.0)) * 0.012 * h_up * h_up;

    let phys_val = get_base_heat(uv + vec2<f32>(sway, 0.0));
    let base_heat = max(phys_val, 0.0);
    let smoke_density = max(-phys_val, 0.0);

    var heat = base_heat;
    var emission = vec3<f32>(0.0);

    let n_ch = max(1u, audio.num_channels);
    var lfe_idx = 999u;
    if (n_ch == 6u || n_ch == 8u || n_ch == 16u) && audio.num_spatial_channels == n_ch { lfe_idx = 3u; }
    var n_spatial_ch = n_ch;
    if lfe_idx < n_ch { n_spatial_ch = n_ch - 1u; }
    let hover_spatial_idx = min(u32(uv.x * f32(n_spatial_ch)), n_spatial_ch - 1u);

    var hover_display_idx = hover_spatial_idx;
    if lfe_idx < n_ch {
        var spatial_idx = 0u;
        for (var i = 0u; i < n_ch; i = i + 1u) {
            if i == lfe_idx { continue; }
            if spatial_idx == hover_spatial_idx {
                hover_display_idx = i;
                break;
            }
            spatial_idx = spatial_idx + 1u;
        }
    }
    let raw_ch = audio.display_order[hover_display_idx / 4u][hover_display_idx % 4u];

    if (base_heat > 0.003) {
        let col_id = floor(uv.x * 24.0);
        let flick_noise = perlin_noise(vec2<f32>(col_id * 7.3, time * 2.4));
        let flicker = 1.0 + flick_noise * (0.10 + bass * 0.15);

        let detail_uv = vec2<f32>(uv_c.x * 9.0, uv.y * 4.5 + time * 3.0);
        let flame_noise = fbm(detail_uv) * 0.5 + 0.5;

        let tongue_noise = perlin_noise(vec2<f32>(uv_c.x * 3.0, time * 1.3));
        let tongue_gain = 0.7 + 0.6 * tongue_noise;

        let erosion = mix(0.9, 0.18, smoothstep(0.0, 0.8, base_heat));
        heat = base_heat * tongue_gain * (1.0 - flame_noise * erosion) * flicker;

        emission = blackbody(pow(min(heat, 1.0), 1.35)) * 0.9;

        // Blue combustion fringe (CH-radical chemiluminescence) at column base
        let heat_mid = smoothstep(0.12, 0.28, base_heat) * (1.0 - smoothstep(0.35, 0.6, base_heat));
        let blue_zone = heat_mid * smoothstep(0.85, 0.97, uv.y);
        emission += vec3<f32>(0.05, 0.20, 1.0) * blue_zone * 0.45;

        // Embers riding the fluid
        let ember_glow = smoothstep(1.5, 4.0, heat);
        emission += vec3<f32>(1.0, 0.9, 0.4) * ember_glow * (3.5 + highs * 2.0);

        // Only sample multi_spectrum for white-hot flame cores (heat > 0.95),
        // skipping ~98% of full-screen storage buffer reads!
        if (heat > 0.95) {
            var fft_ch = raw_ch;
            var vu_scale = 1.0;
            if audio.fft_channels < n_ch {
                fft_ch = raw_ch % max(audio.fft_channels, 1u);
                let val = audio.channels[hover_display_idx / 4u][hover_display_idx % 4u];
                vu_scale = max(val * 2.0, 0.05);
            }

            let offset = fft_ch * 1024u;
            var high_energy = 0.0;
            for (var b = 780u; b < 1000u; b = b + 20u) {
                let c = multi_spectrum[offset + b];
                high_energy += clamp(length(c) * 100.0, 0.0, 100.0);
            }
            high_energy = min((high_energy / 11.0) / 100.0 * vu_scale, 1.0);
            emission += vec3<f32>(0.1, 0.35, 1.0) * (high_energy * smoothstep(0.95, 1.3, heat) * 0.5);
        }
    }

    // 3. Render Smoke (Beer's Law)
    let absorption_coeff = 2.4;
    let transmittance = exp(-smoke_density * absorption_coeff);
    let scatter_color = mix(vec3<f32>(0.03, 0.04, 0.06), vec3<f32>(1.0, 0.35, 0.08), exp(-(1.0 - uv.y) * 3.0));
    let in_scattering = scatter_color * (1.0 - transmittance) * 1.8;

    // 4. Bloom and Halation
    let halation_intensity = smoothstep(0.15, 0.55, min(heat, 1.0)) * 0.3;
    let halation = vec3<f32>(1.0, 0.1, 0.02) * halation_intensity;

    let bloom_intensity = smoothstep(0.4, 0.8, min(heat, 1.0)) * 0.45;
    let bloom = vec3<f32>(1.0, 0.85, 0.25) * bloom_intensity;

    // 4b. Rising ember sparks with horizontal proximity early-out
    var ember_sparks = vec3<f32>(0.0);
    let ember_gate = smoothstep(0.15, 0.5, highs + bass * 0.4) * clamp(audio.fire_intensity, 0.0, 1.0);
    if (ember_gate > 0.01 && uv.y > 0.12) {
        for (var i = 0u; i < 24u; i++) {
            let fi = f32(i);
            let seed1 = fract(sin(fi * 7.13) * 43758.5);
            let seed2 = fract(sin(fi * 13.71) * 24634.6);
            let speed = 0.08 + seed2 * 0.10;
            let life = fract(time * speed + seed1);
            let wobble = sin(life * 9.0 + fi * 3.3) * 0.012;
            let ex = (fi + 0.5) / 24.0 + wobble;
            if abs(uv.x - ex) > 0.08 {
                continue;
            }
            let ey = 1.0 - life * 0.85;
            let d = distance(uv_c, vec2<f32>(ex * aspect, ey));
            let fade = (1.0 - life) * (1.0 - life);
            ember_sparks += vec3<f32>(1.0, 0.5, 0.12) * (exp(-d * d * 9000.0) * 2.5 + 0.0002 / (d * d + 0.003)) * fade;
        }
        ember_sparks *= ember_gate;
    }

    var final_color = aces_tonemap(emission + in_scattering + halation + bloom + ember_sparks);

    // --- Draw Channel Labels along Hearth ---
    if (uv.y > 0.95) {
        let channel_width = 1.0 / f32(max(n_spatial_ch, 1u));
        let center_x = (f32(hover_spatial_idx) + 0.5) * channel_width;
        let center_xc = center_x * aspect;

        var lbl = array<u32, 3>(19u, 19u, 19u);
        var lbl_len = 1u;

        if audio.num_spatial_channels < n_ch {
            let ch_num = raw_ch + 1u;
            if ch_num < 10u {
                lbl[0] = ch_num;
                lbl_len = 1u;
            } else {
                lbl[0] = ch_num / 10u;
                lbl[1] = ch_num % 10u;
                lbl_len = 2u;
            }
        } else {
            let smpte = get_smpte_channel_label(raw_ch);
            lbl[0] = smpte.x;
            lbl[1] = smpte.y;
            lbl[2] = smpte.z;
            lbl_len = smpte.w;
        }

        let px_size = 0.003;
        let char_w = px_size * 4.0;
        let total_w = char_w * f32(lbl_len) - px_size;
        let label_origin = vec2<f32>(center_xc - total_w * 0.5, 0.97);

        let box_w = total_w + 0.01;
        let box_h = px_size * 5.0 + 0.01;
        let dx = abs(uv_c.x - center_xc);
        let dy = abs(uv_c.y - (0.97 + px_size * 2.5));

        if (dx < box_w * 0.5 && dy < box_h * 0.5) {
            final_color = vec3<f32>(0.0);
            var text_alpha = 0.0;
            for (var c_idx = 0u; c_idx < lbl_len; c_idx = c_idx + 1u) {
                let char_origin = label_origin + vec2<f32>(char_w * f32(c_idx), 0.0);
                text_alpha = max(text_alpha, draw_label_char(lbl[c_idx], uv_c, char_origin, px_size));
            }
            if (text_alpha > 0.0) {
                final_color = vec3<f32>(1.0);
            }
        }
    }

    return vec4<f32>(final_color, 1.0);
}
