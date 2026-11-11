struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((in_vertex_index << 1u) & 2u);
    let y = f32(in_vertex_index & 2u);
    out.clip_position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

struct VideoParams {
    color_space: u32,
    color_range: u32,
    bit_depth: u32,
    color_trc: u32,
    viewport_width: f32,
    viewport_height: f32,
    video_width: f32,
    video_height: f32,
    rotation: u32,
    chroma_layout: u32,
    _pad2: f32,
    _pad3: f32,
}

@group(0) @binding(0) var t_y: texture_2d<f32>;
@group(0) @binding(1) var t_u: texture_2d<f32>;
@group(0) @binding(2) var t_v: texture_2d<f32>;
@group(0) @binding(3) var s_smp: sampler;
@group(0) @binding(4) var<uniform> params: VideoParams;
@group(0) @binding(5) var t_border_blur: texture_2d<f32>;

fn transform_uv(display_uv: vec2<f32>, rotation: u32) -> vec2<f32> {
    if (rotation == 90u) {
        return vec2<f32>(display_uv.y, 1.0 - display_uv.x);
    } else if (rotation == 180u) {
        return vec2<f32>(1.0 - display_uv.x, 1.0 - display_uv.y);
    } else if (rotation == 270u) {
        return vec2<f32>(1.0 - display_uv.y, display_uv.x);
    }
    return display_uv;
}

fn sample_yuv_raw(tex_uv: vec2<f32>) -> vec3<f32> {
    let y = textureSampleLevel(t_y, s_smp, tex_uv, 0.0).r;
    if (params.chroma_layout == 1u) {
        // Native 2-plane semi-planar NV12 (Rg8Unorm) / P010 (Rg16Unorm) interleaved UV plane
        let uv_pair = textureSampleLevel(t_u, s_smp, tex_uv, 0.0).rg;
        return vec3<f32>(y, uv_pair.x, uv_pair.y);
    }
    if (params.chroma_layout == 2u) {
        // Native 2-plane semi-planar NV21 (Rg8Unorm) interleaved VU plane
        let vu_pair = textureSampleLevel(t_u, s_smp, tex_uv, 0.0).rg;
        return vec3<f32>(y, vu_pair.y, vu_pair.x);
    }
    let u = textureSampleLevel(t_u, s_smp, tex_uv, 0.0).r;
    let v = textureSampleLevel(t_v, s_smp, tex_uv, 0.0).r;
    return vec3<f32>(y, u, v);
}

fn yuv_to_linear_rgb(yuv_in: vec3<f32>) -> vec3<f32> {
    var y = yuv_in.x;
    var u = yuv_in.y;
    var v = yuv_in.z;

    var bit_depth_scale = 1.0;
    var max_code = 255.0;
    var code_shift_scale = 1.0;
    if (params.bit_depth == 10u) {
        bit_depth_scale = 65535.0 / 1023.0;
        max_code = 1023.0;
        code_shift_scale = 4.0;
    } else if (params.bit_depth == 12u) {
        bit_depth_scale = 65535.0 / 4095.0;
        max_code = 4095.0;
        code_shift_scale = 16.0;
    } else if (params.bit_depth == 16u) {
        // MSB-aligned 10-bit hardware surface (P010LE: 10-bit codes << 6, max 65472)
        bit_depth_scale = 65535.0 / 65472.0;
        max_code = 1023.0;
        code_shift_scale = 4.0;
    }
    y *= bit_depth_scale;
    u *= bit_depth_scale;
    v *= bit_depth_scale;

    // Exact UNORM neutral chroma midpoint (e.g. 128/255 for 8-bit, 512/1023 for 10-bit & P010, 2048/4095 for 12-bit)
    let chroma_mid = (128.0 * code_shift_scale) / max_code;

    var y_adj: f32;
    var u_adj: f32;
    var v_adj: f32;

    if (params.color_range == 2u) {
        // PC / Full Range (AVCOL_RANGE_JPEG)
        y_adj = y;
        u_adj = u - chroma_mid;
        v_adj = v - chroma_mid;
    } else {
        // TV / Limited Range (AVCOL_RANGE_MPEG) -> Default
        let luma_black = (16.0 * code_shift_scale) / max_code;
        let luma_scale = max_code / (219.0 * code_shift_scale);
        let chroma_scale = max_code / (224.0 * code_shift_scale);
        y_adj = (y - luma_black) * luma_scale;
        u_adj = (u - chroma_mid) * chroma_scale;
        v_adj = (v - chroma_mid) * chroma_scale;
    }

    var r = 0.0;
    var g = 0.0;
    var b = 0.0;

    var is_hdr = false;
    if (params.color_space == 9u || params.color_space == 10u) {
        // BT.2020 (HDR)
        r = y_adj + 1.4746 * v_adj;
        g = y_adj - 0.16455 * u_adj - 0.57135 * v_adj;
        b = y_adj + 1.8814 * u_adj;

        // 1. EOTF: Convert from PQ / HLG to Linear light
        if (params.color_trc == 16u) { // PQ
            let m1 = 0.1593017578125;
            let m2 = 78.84375;
            let c1 = 0.8359375;
            let c2 = 18.8515625;
            let c3 = 18.6875;

            let pow_r = pow(max(r, 0.0), 1.0 / m2);
            let pow_g = pow(max(g, 0.0), 1.0 / m2);
            let pow_b = pow(max(b, 0.0), 1.0 / m2);

            r = pow(max(pow_r - c1, 0.0) / (c2 - c3 * pow_r), 1.0 / m1);
            g = pow(max(pow_g - c1, 0.0) / (c2 - c3 * pow_g), 1.0 / m1);
            b = pow(max(pow_b - c1, 0.0) / (c2 - c3 * pow_b), 1.0 / m1);

            // Output is in [0, 1] relative to 10,000 nits.
            // Scale so that 1.0 roughly equals diffuse white (around 100 nits)
            r *= 100.0;
            g *= 100.0;
            b *= 100.0;
        } else if (params.color_trc == 18u) { // HLG
            let a = 0.17883277;
            let b_hlg = 0.28466892;
            let c = 0.55991073;

            var l_r = 0.0; var l_g = 0.0; var l_b = 0.0;
            if (r <= 0.5) { l_r = (r * r) / 3.0; } else { l_r = (exp((r - c) / a) + b_hlg) / 12.0; }
            if (g <= 0.5) { l_g = (g * g) / 3.0; } else { l_g = (exp((g - c) / a) + b_hlg) / 12.0; }
            if (b <= 0.5) { l_b = (b * b) / 3.0; } else { l_b = (exp((b - c) / a) + b_hlg) / 12.0; }
            r = max(l_r, 0.0) * 3.0;
            g = max(l_g, 0.0) * 3.0;
            b = max(l_b, 0.0) * 3.0;
        } else {
            r = pow(max(r, 0.0), 2.2);
            g = pow(max(g, 0.0), 2.2);
            b = pow(max(b, 0.0), 2.2);
        }

        // 2. BT.2020 to BT.709 linear matrix
        let r_709 =  1.6605 * r - 0.5876 * g - 0.0728 * b;
        let g_709 = -0.1246 * r + 1.1329 * g - 0.0083 * b;
        let b_709 = -0.0182 * r - 0.1006 * g + 1.1187 * b;
        r = max(r_709, 0.0);
        g = max(g_709, 0.0);
        b = max(b_709, 0.0);

        // 3. ACES Filmic Tonemap
        let a_aces = 2.51;
        let b_aces = 0.03;
        let c_aces = 2.43;
        let d_aces = 0.59;
        let e_aces = 0.14;
        r = clamp((r * (a_aces * r + b_aces)) / (r * (c_aces * r + d_aces) + e_aces), 0.0, 1.0);
        g = clamp((g * (a_aces * g + b_aces)) / (g * (c_aces * g + d_aces) + e_aces), 0.0, 1.0);
        b = clamp((b * (a_aces * b + b_aces)) / (b * (c_aces * b + d_aces) + e_aces), 0.0, 1.0);

        is_hdr = true;
    } else if (params.color_space == 5u || params.color_space == 6u) {
        // BT.601 (SD)
        r = y_adj + 1.402 * v_adj;
        g = y_adj - 0.344136 * u_adj - 0.714136 * v_adj;
        b = y_adj + 1.772 * u_adj;
    } else {
        // BT.709 (HD) -> Default
        r = y_adj + 1.5748 * v_adj;
        g = y_adj - 0.187324 * u_adj - 0.468124 * v_adj;
        b = y_adj + 1.8556 * u_adj;
    }

    if (!is_hdr) {
        // Convert from Gamma-encoded video space to Linear RGB.
        r = pow(max(r, 0.0), 2.2);
        g = pow(max(g, 0.0), 2.2);
        b = pow(max(b, 0.0), 2.2);
    }

    return clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Quarter-resolution pre-blurred ambient border pass executed ONLY when a new decoded video
// frame (generation) arrives. Renders into Rgba8UnormSrgb (hardware sRGB encode + hardware
// sRGB-to-linear decode on textureSampleLevel) to eliminate 8-bit dark shadow banding while
// replacing 93 per-pixel fetches every display refresh with a single bilinear fetch in fs_main.
@fragment
fn fs_border_blur(in: VertexOutput) -> @location(0) vec4<f32> {
    var yuv_accum = vec3<f32>(0.0);
    var weight_sum = 0.0;
    for (var dy = -3i; dy <= 3i; dy++) {
        let ty = f32(dy) / 3.0;
        let wy = exp(-ty * ty * 2.2);
        for (var dx = -3i; dx <= 3i; dx++) {
            let tx = f32(dx) / 3.0;
            let weight = wy * exp(-tx * tx * 2.2);
            let s_uv = clamp(
                in.uv + vec2<f32>(f32(dx), f32(dy)) * 0.045,
                vec2<f32>(0.015),
                vec2<f32>(0.985)
            );
            yuv_accum += sample_yuv_raw(s_uv) * weight;
            weight_sum += weight;
        }
    }
    let linear_rgb = yuv_to_linear_rgb(yuv_accum / max(weight_sum, 1e-5));
    return vec4<f32>(linear_rgb, 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let vp_aspect = params.viewport_width / params.viewport_height;
    var vid_aspect = params.video_width / params.video_height;
    if (params.rotation == 90u || params.rotation == 270u) {
        vid_aspect = params.video_height / params.video_width;
    }

    var uv = in.uv;
    var is_edge = false;
    var dist = 0.0;

    if (vp_aspect > vid_aspect) {
        // Viewport is wider than video (Pillarboxing)
        let scale = vid_aspect / vp_aspect;
        uv.x = (uv.x - 0.5) / scale + 0.5;
        if (uv.x < 0.0) { is_edge = true; dist = -uv.x; }
        if (uv.x > 1.0) { is_edge = true; dist = uv.x - 1.0; }
    } else {
        // Viewport is taller than video (Letterboxing)
        let scale = vp_aspect / vid_aspect;
        uv.y = (uv.y - 0.5) / scale + 0.5;
        if (uv.y < 0.0) { is_edge = true; dist = -uv.y; }
        if (uv.y > 1.0) { is_edge = true; dist = uv.y - 1.0; }
    }

    let sample_uv = vec2<f32>(clamp(uv.x, 0.0, 1.0), clamp(uv.y, 0.0, 1.0));

    if (is_edge) {
        // Sample pre-blurred quarter-resolution border texture slightly inward (2%)
        var inward_uv = sample_uv;
        if (vp_aspect > vid_aspect) {
            if (uv.x < 0.0) { inward_uv.x = 0.02; }
            if (uv.x > 1.0) { inward_uv.x = 0.98; }
        } else {
            if (uv.y < 0.0) { inward_uv.y = 0.02; }
            if (uv.y > 1.0) { inward_uv.y = 0.98; }
        }
        let b_uv = transform_uv(inward_uv, params.rotation);
        let blurred_rgb = textureSampleLevel(t_border_blur, s_smp, b_uv, 0.0).rgb;
        let fade = clamp(1.0 - (dist * 2.5), 0.0, 1.0) * 0.4;
        return vec4<f32>(clamp(blurred_rgb * fade, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
    }

    let tex_uv = transform_uv(sample_uv, params.rotation);
    let rgb = yuv_to_linear_rgb(sample_yuv_raw(tex_uv));
    return vec4<f32>(rgb, 1.0);
}
