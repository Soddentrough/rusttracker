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
}

@group(0) @binding(0) var t_y: texture_2d<f32>;
@group(0) @binding(1) var t_u: texture_2d<f32>;
@group(0) @binding(2) var t_v: texture_2d<f32>;
@group(0) @binding(3) var s_smp: sampler;
@group(0) @binding(4) var<uniform> params: VideoParams;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let vp_aspect = params.viewport_width / params.viewport_height;
    let vid_aspect = params.video_width / params.video_height;
    
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
    
    var sample_uv = vec2<f32>(clamp(uv.x, 0.0, 1.0), clamp(uv.y, 0.0, 1.0));

    var y = textureSample(t_y, s_smp, sample_uv).r;
    var u = textureSample(t_u, s_smp, sample_uv).r;
    var v = textureSample(t_v, s_smp, sample_uv).r;

    if (is_edge) {
        y = 0.0;
        u = 0.0;
        v = 0.0;
        let half_s = 15;
        var weight_sum = 0.0;
        
        // Push the sampling point slightly INWARDS (2%) from the edge 
        // to avoid sampling a black border artifact encoded in the video file
        var inward_uv = sample_uv;
        if (vp_aspect > vid_aspect) {
            // Pillarboxing: we are sampling from the left/right edges
            if (uv.x < 0.0) { inward_uv.x = 0.02; }
            if (uv.x > 1.0) { inward_uv.x = 0.98; }
        } else {
            // Letterboxing: we are sampling from the top/bottom edges
            if (uv.y < 0.0) { inward_uv.y = 0.02; }
            if (uv.y > 1.0) { inward_uv.y = 0.98; }
        }
        
        for (var i = -half_s; i <= half_s; i++) {
            var offset = vec2<f32>(0.0, 0.0);
            
            // Blur PARALLEL to the edge to hide pixel streaks
            if (vp_aspect > vid_aspect) {
                // Pillarboxing: blur vertically
                offset.y = f32(i) * 0.015;
            } else {
                // Letterboxing: blur horizontally
                offset.x = f32(i) * 0.015;
            }
            
            var s_uv = inward_uv + offset;
            
            // Clamp the blurred axis so we don't sample outside the video texture
            s_uv = vec2<f32>(clamp(s_uv.x, 0.0, 1.0), clamp(s_uv.y, 0.0, 1.0));
            
            // Gaussian weight for an extremely soft, abstract gradient
            let t = f32(i) / f32(half_s);
            let weight = exp(-t * t * 3.0);
            
            y += textureSample(t_y, s_smp, s_uv).r * weight;
            u += textureSample(t_u, s_smp, s_uv).r * weight;
            v += textureSample(t_v, s_smp, s_uv).r * weight;
            weight_sum += weight;
        }
        
        y /= weight_sum;
        u /= weight_sum;
        v /= weight_sum;
    }

    var bit_depth_scale = 1.0;
    if (params.bit_depth == 10u) {
        bit_depth_scale = 65535.0 / 1023.0;
    } else if (params.bit_depth == 12u) {
        bit_depth_scale = 65535.0 / 4095.0;
    }
    y *= bit_depth_scale;
    u *= bit_depth_scale;
    v *= bit_depth_scale;

    var y_adj: f32;
    var u_adj: f32;
    var v_adj: f32;

    if (params.color_range == 2u) { 
        // PC / Full Range (AVCOL_RANGE_JPEG)
        y_adj = y;
        u_adj = u - 0.5;
        v_adj = v - 0.5;
    } else { 
        // TV / Limited Range (AVCOL_RANGE_MPEG) -> Default
        y_adj = (y - 16.0/255.0) * 1.164383; // 255/219
        u_adj = (u - 0.5) * 1.138140; // 255/224
        v_adj = (v - 0.5) * 1.138140; // 255/224
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
            
            let pow_r = pow(max(r, 0.0), 1.0/m2);
            let pow_g = pow(max(g, 0.0), 1.0/m2);
            let pow_b = pow(max(b, 0.0), 1.0/m2);
            
            r = pow(max(pow_r - c1, 0.0) / (c2 - c3 * pow_r), 1.0/m1);
            g = pow(max(pow_g - c1, 0.0) / (c2 - c3 * pow_g), 1.0/m1);
            b = pow(max(pow_b - c1, 0.0) / (c2 - c3 * pow_b), 1.0/m1);
            
            // Output is in [0, 1] relative to 10,000 nits.
            // Scale so that 1.0 roughly equals diffuse white (around 100 nits)
            r *= 100.0;
            g *= 100.0;
            b *= 100.0;
        } else if (params.color_trc == 18u) { // HLG
            let a = 0.17883277;
            let b_hlg = 0.28466892;
            let c = 0.55991073;
            
            // HLG EOTF piecewise logic
            var l_r = 0.0; var l_g = 0.0; var l_b = 0.0;
            if (r <= 0.5) { l_r = (r * r) / 3.0; } else { l_r = (exp((r - c) / a) + b_hlg) / 12.0; }
            if (g <= 0.5) { l_g = (g * g) / 3.0; } else { l_g = (exp((g - c) / a) + b_hlg) / 12.0; }
            if (b <= 0.5) { l_b = (b * b) / 3.0; } else { l_b = (exp((b - c) / a) + b_hlg) / 12.0; }
            r = max(l_r, 0.0) * 3.0;
            g = max(l_g, 0.0) * 3.0;
            b = max(l_b, 0.0) * 3.0;
        } else {
            // Assume gamma 2.2
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
        // The WGPU surface is sRGB and will automatically apply the sRGB gamma curve.
        r = pow(max(r, 0.0), 2.2);
        g = pow(max(g, 0.0), 2.2);
        b = pow(max(b, 0.0), 2.2);
    }

    if (is_edge) {
        // Create an ambient gradient glow for the borders (smooth fade to black)
        let fade = clamp(1.0 - (dist * 2.5), 0.0, 1.0);
        // Apply a subtle darkening (0.4x) for the abstract "ambient" feel
        r *= fade * 0.4;
        g *= fade * 0.4;
        b *= fade * 0.4;
    }

    return vec4<f32>(clamp(r, 0.0, 1.0), clamp(g, 0.0, 1.0), clamp(b, 0.0, 1.0), 1.0);
}
