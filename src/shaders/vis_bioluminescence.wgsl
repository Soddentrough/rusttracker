// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(1) @binding(0) var<uniform> camera: CameraUniforms;
@group(2) @binding(0) var<storage, read> particles: array<Particle>;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
}

struct Particle {
    pos: vec4<f32>, // pos.xyz = position, pos.w = life
    vel: vec4<f32>, // vel.xyz = velocity, vel.w = energy
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
}

struct CameraBasis {
    right: vec3<f32>,
    up: vec3<f32>,
    forward: vec3<f32>,
    eye: vec3<f32>,
    is_top_down: bool,
}

struct BgVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

struct OceanVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) ocean_params: vec4<f32>, // x: wave_h, y: folding, z: dist_cam, w: audio_crest_boost
}

struct ParticleVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) core_col: vec3<f32>,
    @location(2) @interpolate(flat) halo_col: vec3<f32>,
    @location(3) @interpolate(flat) intensity: f32,
}

// Calibrated 5-wave directional Gerstner spectrum (shared identically with biolum_compute.wgsl)
const WAVE_KINEMATICS: array<vec4<f32>, 5> = array<vec4<f32>, 5>(
    vec4<f32>( 0.220000, -0.975500, 0.330694, 0.48),
    vec4<f32>(-0.420000, -0.907524, 0.502655, 0.64),
    vec4<f32>( 0.640000, -0.768375, 0.849079, 0.86),
    vec4<f32>(-0.760000, -0.649923, 1.365910, 1.12),
    vec4<f32>( 0.350000, -0.936750, 2.327106, 1.44)
);

const WAVE_AMPS: array<vec2<f32>, 5> = array<vec2<f32>, 5>(
    vec2<f32>(0.54, 0.72),
    vec2<f32>(0.34, 0.38),
    vec2<f32>(0.18, 0.16),
    vec2<f32>(0.095, 0.062),
    vec2<f32>(0.048, 0.020)
);

const MOON_DIR: vec3<f32> = vec3<f32>(0.240000, 0.155000, 0.958319);

fn hash21_biolum(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}

fn noise2d_biolum(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21_biolum(i);
    let b = hash21_biolum(i + vec2<f32>(1.0, 0.0));
    let c = hash21_biolum(i + vec2<f32>(0.0, 1.0));
    let d = hash21_biolum(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm2d_biolum(p_in: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.55;
    var p = p_in;
    let rot = mat2x2<f32>(0.80, 0.60, -0.60, 0.80);
    for (var i = 0u; i < 3u; i = i + 1u) {
        v += a * noise2d_biolum(p);
        p = rot * p * 2.03 + vec2<f32>(17.1, 31.7);
        a *= 0.48;
    }
    return v;
}

fn sample_biolum_audio() -> vec4<f32> {
    let raw_bass = max(
        max(audio.spectrum[0].x, audio.spectrum[0].y),
        max(audio.spectrum[1].x, audio.spectrum[2].x)
    );
    let raw_mids = (audio.spectrum[8].x + audio.spectrum[12].x + audio.spectrum[20].x) * 0.6667;
    let raw_treble = (audio.spectrum[40].x + audio.spectrum[55].x + audio.spectrum[75].x) * 0.6667;

    let bass = clamp(raw_bass / 72.0, 0.0, 1.5);
    let mids = clamp(raw_mids / 125.0, 0.0, 1.3);
    let treble = clamp(raw_treble / 115.0, 0.0, 1.3);

    var ch_l = 0.0;
    var ch_r = 0.0;
    if (audio.num_spatial_channels >= 2u) {
        ch_l = audio.spatial_channels[0].x;
        ch_r = audio.spatial_channels[0].y;
    } else if (audio.num_channels >= 2u) {
        ch_l = audio.channels[0].x;
        ch_r = audio.channels[0].y;
    } else if (audio.num_channels == 1u) {
        ch_l = audio.channels[0].x;
        ch_r = ch_l;
    }
    let stereo_bias = clamp((ch_r - ch_l) * 0.9, -1.0, 1.0);
    return vec4<f32>(bass, mids, treble, stereo_bias);
}

fn extract_camera_basis() -> CameraBasis {
    let v = camera.view_matrix;
    let right = normalize(vec3<f32>(v[0][0], v[1][0], v[2][0]));
    let up = normalize(vec3<f32>(v[0][1], v[1][1], v[2][1]));
    let forward = normalize(-vec3<f32>(v[0][2], v[1][2], v[2][2]));
    let eye = forward * v[3][2] - right * v[3][0] - up * v[3][1];

    var out: CameraBasis;
    out.right = right;
    out.up = up;
    out.forward = forward;
    out.eye = eye;
    out.is_top_down = forward.y < -0.75;
    return out;
}

fn eval_night_sky(rd: vec3<f32>, time: f32, bass: f32, mids: f32, treble: f32) -> vec3<f32> {
    let up_y = max(rd.y, 0.0);

    // Deep midnight indigo-navy gradient
    let sky_zenith = vec3<f32>(0.0015, 0.0045, 0.016);
    let sky_mid = vec3<f32>(0.0045, 0.014, 0.040);
    let sky_horizon = vec3<f32>(0.008, 0.034, 0.082);

    var sky = mix(sky_horizon, sky_mid, smoothstep(0.0, 0.24, up_y));
    sky = mix(sky, sky_zenith, smoothstep(0.20, 0.72, up_y));

    // Electric-cyan bioluminescent surf mist hovering just above the horizon
    let hy = abs(rd.y) * 12.5;
    let surf_mist = 1.0 / (1.0 + hy * hy * 1.6);
    sky += vec3<f32>(0.007, 0.095, 0.175) * surf_mist * (0.45 + bass * 0.40 + mids * 0.18);

    // Lunar disc & atmospheric corona
    let moon_dir = normalize(MOON_DIR);
    let moon_cos = max(dot(rd, moon_dir), 0.0);
    let moon_dist = length(rd - moon_dir * moon_cos);

    // Subtle moonlit cirrus clouds
    if (rd.y > 0.008) {
        let cloud_uv = rd.xz / (rd.y + 0.26) * vec2<f32>(0.85, 1.15) + vec2<f32>(time * 0.008, time * 0.004);
        let c0 = fbm2d_biolum(cloud_uv);
        let c1 = fbm2d_biolum(cloud_uv * 1.9 + vec2<f32>(3.4, -2.1));
        let cloud_density = smoothstep(0.46, 0.86, c0 * 0.7 + c1 * 0.3) * smoothstep(0.01, 0.14, rd.y);
        let moon_scatter = 0.25 + 1.35 * (moon_cos * moon_cos * moon_cos * moon_cos);
        let cloud_col = vec3<f32>(0.025, 0.085, 0.165) * moon_scatter * (1.0 + bass * 0.15);
        sky = mix(sky, sky + cloud_col, cloud_density * 0.55);
    }

    // Crisp multi-layer twinkling stars
    if (rd.y > 0.015) {
        let star_fade = smoothstep(0.015, 0.16, rd.y) * smoothstep_r(0.994, 0.965, moon_cos);
        let star_uv = rd.xz / (rd.y + 0.38);

        let cell_a = floor(star_uv * 135.0);
        let loc_a = fract(star_uv * 135.0) - vec2<f32>(0.5);
        let rnd_a = hash21_biolum(cell_a);
        if (rnd_a > 0.968) {
            let off = vec2<f32>(hash21_biolum(cell_a + 7.1), hash21_biolum(cell_a + 19.3)) - 0.5;
            let d = length(loc_a - off * 0.55);
            let pt = smoothstep_r(0.20, 0.0, d);
            let tw = 0.60 + 0.40 * sin(time * 2.2 + rnd_a * 62.8);
            let tint = mix(vec3<f32>(0.65, 0.88, 1.0), vec3<f32>(0.92, 0.98, 1.0), fract(rnd_a * 31.7));
            sky += tint * pt * tw * star_fade * (0.65 + treble * 0.55);
        }

        let cell_b = floor(star_uv * 240.0 + vec2<f32>(11.3, 5.7));
        let loc_b = fract(star_uv * 240.0 + vec2<f32>(11.3, 5.7)) - vec2<f32>(0.5);
        let rnd_b = hash21_biolum(cell_b);
        if (rnd_b > 0.982) {
            let d = length(loc_b);
            let pt = smoothstep_r(0.18, 0.0, d);
            sky += vec3<f32>(0.55, 0.80, 0.98) * pt * star_fade * 0.45;
        }
    }

    // Dual-layer lunar glow + crisp moon disc
    if (moon_cos > 0.0) {
        let m_inner = 1.0 / (1.0 + moon_dist * moon_dist * 480.0);
        let m_outer = 1.0 / (1.0 + moon_dist * moon_dist * 26.0);
        sky += vec3<f32>(0.18, 0.48, 0.72) * m_inner * 0.85;
        sky += vec3<f32>(0.025, 0.095, 0.175) * m_outer * 0.52;

        let disc = smoothstep_r(0.0235, 0.0185, moon_dist);
        if (disc > 0.0) {
            let crater_uv = (rd.xy - moon_dir.xy) * 95.0;
            let maria = 0.86 + 0.14 * fbm2d_biolum(crater_uv);
            sky = mix(sky, vec3<f32>(0.82, 0.94, 1.05) * maria * 1.45, disc);
        }
    }

    return sky;
}

// =============================================================================
// Layer 1: Midnight Sky, Moon & Horizon Backdrop (Fullscreen Triangle)
// =============================================================================
@vertex
fn vs_bg(@builtin(vertex_index) in_vertex_index: u32) -> BgVertexOutput {
    var out: BgVertexOutput;
    let u = f32((in_vertex_index << 1u) & 2u);
    let v = f32(in_vertex_index & 2u);
    let ndc = vec2<f32>(u * 2.0 - 1.0, -(v * 2.0 - 1.0));
    out.clip_position = vec4<f32>(ndc, 1.0, 1.0);
    out.ndc = ndc;
    return out;
}

@fragment
fn fs_bg(in: BgVertexOutput) -> @location(0) vec4<f32> {
    let cam = extract_camera_basis();
    let time = select(audio.time, audio.smooth_time, audio.smooth_time > 0.0);
    let aud = sample_biolum_audio();

    if (cam.is_top_down) {
        // Deep midnight ocean base under the displaced ocean mesh
        return vec4<f32>(aces_tonemap(vec3<f32>(0.002, 0.009, 0.024)), 1.0);
    }

    let proj_x = max(camera.proj_matrix[0][0], 0.05);
    let proj_y = max(camera.proj_matrix[1][1], 0.05);
    let rd = normalize(
        cam.right * (in.ndc.x / proj_x)
        + cam.up * (in.ndc.y / proj_y)
        + cam.forward
    );

    var col = eval_night_sky(rd, time, aud.x, aud.y, aud.z);
    if (rd.y < 0.0) {
        // Below horizon fallback matches ocean horizon fog seamlessly
        let deep_sea = vec3<f32>(0.008, 0.036, 0.082);
        let horizon_col = eval_night_sky(normalize(vec3<f32>(rd.x, 0.0, rd.z)), time, aud.x, aud.y, aud.z);
        col = mix(horizon_col, deep_sea, smoothstep(0.0, 0.14, -rd.y));
    }

    return vec4<f32>(aces_tonemap(col), 1.0);
}

// =============================================================================
// Layer 2: 3D Displaced Gerstner Ocean Surface Mesh
// =============================================================================
@vertex
fn vs_ocean(in: VertexInput) -> OceanVertexOutput {
    var out: OceanVertexOutput;
    let cam = extract_camera_basis();
    let time = select(audio.time, audio.smooth_time, audio.smooth_time > 0.0);
    let aud = sample_biolum_audio();
    let bass = aud.x;
    let mids = aud.y;

    let tan_half_fovx = 1.0 / max(camera.proj_matrix[0][0], 0.1);
    let tan_half_fovy = 1.0 / max(camera.proj_matrix[1][1], 0.1);

    let u = in.tex_coords.x;
    let v = in.tex_coords.y;

    var x0 = 0.0;
    var z0 = 0.0;
    if (cam.is_top_down) {
        let half_x = max(24.0, (cam.eye.y + 3.0) * tan_half_fovx * 1.25) + 3.0;
        let half_z = max(16.0, (cam.eye.y + 3.0) * tan_half_fovy * 1.25) + 3.0;
        x0 = (u * 2.0 - 1.0) * half_x;
        z0 = 6.0 + (v * 2.0 - 1.0) * half_z;
    } else {
        // Stationary frustum-adaptive perspective grid (no camera sway vertex swimming)
        let dist_z = 1.5 + 32.0 * v + 152.0 * v * v * v;
        z0 = -13.5 + dist_z;
        let s = u * 2.0 - 1.0;
        let half_span_x = 14.0 + dist_z * tan_half_fovx * 1.32;
        x0 = s * (0.68 + 0.32 * s * s) * half_span_x;
    }

    let rest_xz = vec2<f32>(x0, z0);
    let approx_dist = length(vec3<f32>(x0, 0.0, z0) - cam.eye);
    let far_damp = select(smoothstep_r(165.0, 82.0, approx_dist), 1.0, cam.is_top_down);

    let amp_scale = min(1.0 + bass * 0.28 + mids * 0.08, 1.42) * far_damp;
    let qa_scale = min(1.0 + bass * 0.16, 1.20) * far_damp;

    var disp = vec3<f32>(0.0);
    var T = vec3<f32>(1.0, 0.0, 0.0);
    var B = vec3<f32>(0.0, 0.0, 1.0);

    for (var i = 0u; i < 5u; i = i + 1u) {
        let kin = WAVE_KINEMATICS[i];
        let amp = WAVE_AMPS[i];
        let dir = kin.xy;
        let k = kin.z;
        let w = kin.w;

        let theta = k * dot(dir, rest_xz) - w * time;
        let sin_t = sin(theta);
        let cos_t = cos(theta);

        let a = amp.x * amp_scale;
        let qa = amp.y * qa_scale;
        let k_qa = k * qa;
        let k_a = k * a;

        disp += vec3<f32>(qa * dir.x * cos_t, a * sin_t, qa * dir.y * cos_t);

        let dxy_sin = k_qa * dir.x * dir.y * sin_t;
        T += vec3<f32>(-k_qa * dir.x * dir.x * sin_t, k_a * dir.x * cos_t, -dxy_sin);
        B += vec3<f32>(-dxy_sin, k_a * dir.y * cos_t, -k_qa * dir.y * dir.y * sin_t);
    }

    let world_pos = vec3<f32>(x0 + disp.x, disp.y, z0 + disp.z);
    let world_normal = normalize(cross(B, T));
    let jacobian = T.x * B.z - T.z * B.x;
    let folding = clamp((1.0 - jacobian) * 1.15, 0.0, 1.0);

    let view_pos = camera.view_matrix * vec4<f32>(world_pos, 1.0);
    out.clip_position = camera.proj_matrix * view_pos;
    out.world_pos = world_pos;
    out.world_normal = world_normal;
    out.ocean_params = vec4<f32>(world_pos.y, folding, length(cam.eye - world_pos), bass * 0.65 + mids * 0.35);
    return out;
}

@fragment
fn fs_ocean(in: OceanVertexOutput) -> @location(0) vec4<f32> {
    let cam = extract_camera_basis();
    let time = select(audio.time, audio.smooth_time, audio.smooth_time > 0.0);
    let aud = sample_biolum_audio();
    let bass = aud.x;
    let mids = aud.y;
    let treble = aud.z;
    let stereo_bias = aud.w;

    let world_p = in.world_pos;
    let wave_h = in.ocean_params.x;
    let folding = in.ocean_params.y;
    let dist_cam = in.ocean_params.z;

    // Distance-attenuated 3-octave capillary micro-ripples so troughs & slopes glisten
    let ripple_fade = select(smoothstep_r(72.0, 14.0, dist_cam), 1.0, cam.is_top_down);
    var N = normalize(in.world_normal);
    if (ripple_fade > 0.005) {
        let r_uv = world_p.xz * 1.65 + vec2<f32>(time * 0.24, -time * 0.42);
        let eps = 0.08;
        let h_c = fbm2d_biolum(r_uv);
        let h_x = fbm2d_biolum(r_uv + vec2<f32>(eps, 0.0));
        let h_z = fbm2d_biolum(r_uv + vec2<f32>(0.0, eps));
        let ripple_amp = (0.14 + treble * 0.08) * ripple_fade;
        let pert = vec3<f32>((h_c - h_x) / eps, 0.0, (h_c - h_z) / eps) * ripple_amp;
        N = normalize(N + pert);
    }

    let V = normalize(cam.eye - world_p);
    let NdotV = max(dot(N, V), 0.001);

    // Schlick dielectric Fresnel for seawater (F0 = 0.025)
    let nv1 = 1.0 - NdotV;
    let nv2 = nv1 * nv1;
    let fresnel = 0.025 + 0.975 * (nv2 * nv2 * nv1);

    let ref_dir = reflect(-V, N);
    let sky_refl = eval_night_sky(vec3<f32>(ref_dir.x, max(ref_dir.y, 0.002), ref_dir.z), time, bass, mids, treble);

    // Dual-lobe moonlit specular glitter path across wave slopes
    let moon_dir = normalize(MOON_DIR);
    let H = normalize(moon_dir + V);
    let NdotH = max(dot(N, H), 0.0);
    let h2 = NdotH * NdotH;
    let h4 = h2 * h2;
    let h8 = h4 * h4;
    let h16 = h8 * h8;
    let h32 = h16 * h16;
    let h64 = h32 * h32;
    let h128 = h64 * h64;
    let h256 = h128 * h128;
    let spec_glint = h256 * 2.8 + (h16 * h8) * 0.18;
    let moon_spec = vec3<f32>(0.42, 0.78, 1.0) * spec_glint * max(dot(N, moon_dir), 0.0);

    // Deep abyssal water body & crest subsurface scattering (SSS)
    let trough_col = vec3<f32>(0.0018, 0.007, 0.022);
    let slope_col = vec3<f32>(0.004, 0.024, 0.058);
    let base_water = mix(trough_col, slope_col, smoothstep(-0.45, 0.55, wave_h));

    let crest_elev = clamp((wave_h + 0.10) * 0.95, 0.0, 1.0);
    let grazing_sss = 1.0 - NdotV * 0.65;
    let topdown_boost = select(1.0, 1.45, cam.is_top_down);
    let sss_strength = (crest_elev * 0.55 + folding * 0.70) * grazing_sss * topdown_boost * (0.65 + bass * 0.70 + mids * 0.30);
    let sss_col = vec3<f32>(0.008, 0.28, 0.48) * sss_strength;

    // Audio-driven radial acoustic pressure wave & stereo left/right surge across crests
    let radial_dist = length(world_p.xz - vec2<f32>(0.0, 6.0));
    let acoustic_ring = 0.5 + 0.5 * sin(radial_dist * 0.45 - time * 1.8);
    let side_x = clamp(world_p.x / 18.0, -1.0, 1.0);
    let stereo_boost = max(0.0, side_x * stereo_bias) * 0.50;
    let acoustic_wave = (bass * (0.35 + 0.65 * acoustic_ring) + stereo_boost) * smoothstep(-0.05, 0.65, wave_h);

    // Domain-warped lacy dinoflagellate bioluminescent foam veins along breaking crests
    let warp_uv = world_p.xz * 0.85 + vec2<f32>(time * 0.14, -time * 0.22);
    let warp = fbm2d_biolum(warp_uv) * 1.25;
    let vein_raw = fbm2d_biolum(world_p.xz * 2.15 + vec2<f32>(warp, -warp * 0.8) + vec2<f32>(0.0, -time * 0.30));
    let lacy_filaments = smoothstep(0.46, 0.85, vein_raw) + 0.28 * smoothstep(0.64, 0.92, vein_raw);

    let foam_gate = smoothstep(0.18, 0.70, folding + acoustic_wave * 0.32 + crest_elev * 0.16);
    let biolum_drive = foam_gate * lacy_filaments * select(1.0, 1.22, cam.is_top_down) * (0.55 + bass * 0.52 + mids * 0.42 + treble * 0.22);

    let biolum_teal = vec3<f32>(0.012, 0.48, 0.78);
    let biolum_cyan = vec3<f32>(0.06, 0.82, 0.98);
    let biolum_hot = vec3<f32>(0.40, 0.95, 1.0);
    let vein_col = mix(biolum_teal, biolum_cyan, clamp(biolum_drive * 0.72, 0.0, 1.0))
                 + biolum_hot * smoothstep(0.88, 1.60, biolum_drive) * 0.45;

    // Subtle top-down ridge illumination so overhead drone view shows sculpted 3D swells
    var topdown_ridge = vec3<f32>(0.0);
    if (cam.is_top_down) {
        let td_light = normalize(vec3<f32>(0.42, 0.58, 0.70));
        let ridge_diff = max(dot(N, td_light), 0.0);
        let slope_mag = length(N.xz);
        topdown_ridge = vec3<f32>(0.02, 0.14, 0.28) * ridge_diff * slope_mag * 1.35;
    }

    let water_body = base_water + sss_col + vein_col * biolum_drive + topdown_ridge;
    var ocean_col = mix(water_body, sky_refl, fresnel * 0.88 + 0.03);
    ocean_col += moon_spec * (0.45 + 0.55 * fresnel);

    if (!cam.is_top_down) {
        // Atmospheric horizon mist seamlessly merges distant waves into sky horizon
        let horiz_dir = normalize(vec3<f32>(world_p.x - cam.eye.x, 0.0, world_p.z - cam.eye.z));
        let horizon_col = eval_night_sky(horiz_dir, time, bass, mids, treble);
        let fog_t = smoothstep(42.0, 152.0, dist_cam);
        ocean_col = mix(ocean_col, horizon_col, fog_t);
    }

    return vec4<f32>(aces_tonemap(ocean_col), 1.0);
}

// =============================================================================
// Layer 3: GPU-Compute Bioluminescent Plankton, Streaks & Sea Spray
// =============================================================================
@vertex
fn vs_main_3d(in: VertexInput, @builtin(instance_index) inst_idx: u32) -> ParticleVertexOutput {
    var out: ParticleVertexOutput;

    if (inst_idx >= arrayLength(&particles)) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.uv = vec2<f32>(0.0);
        out.core_col = vec3<f32>(0.0);
        out.halo_col = vec3<f32>(0.0);
        out.intensity = 0.0;
        return out;
    }

    let p = particles[inst_idx];
    let energy = p.vel.w;
    let kind = inst_idx & 7u;

    if (p.pos.w <= 0.0 || energy < 0.01 || (kind == 7u && energy < 0.25)) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.uv = vec2<f32>(0.0);
        out.core_col = vec3<f32>(0.0);
        out.halo_col = vec3<f32>(0.0);
        out.intensity = 0.0;
        return out;
    }

    var view_center = camera.view_matrix * vec4<f32>(p.pos.xyz, 1.0);
    let depth = -view_center.z;

    if (depth < 1.0 || depth > 68.0) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.uv = vec2<f32>(0.0);
        out.core_col = vec3<f32>(0.0);
        out.halo_col = vec3<f32>(0.0);
        out.intensity = 0.0;
        return out;
    }

    // Pull billboard slightly forward in view space (+Z) to avoid self-clipping against
    // the ocean triangle directly beneath it while still being occluded by foreground crests.
    view_center.z += 0.22;

    // Velocity-aligned billboard axes in view space
    let vel_view = (camera.view_matrix * vec4<f32>(p.vel.xyz, 0.0)).xy;
    let speed_xy = length(vel_view);
    var axis_u = vec2<f32>(1.0, 0.0);
    var axis_v = vec2<f32>(0.0, 1.0);
    var stretch = 1.0;

    if (speed_xy > 0.03 && kind != 7u) {
        axis_u = vel_view / speed_xy;
        axis_v = vec2<f32>(-axis_u.y, axis_u.x);
        let max_stretch = select(1.45, 2.25, kind == 5u || kind == 6u);
        stretch = 1.0 + clamp(speed_xy * 0.35, 0.0, max_stretch - 1.0);
    }

    var size = 0.030 + clamp(energy, 0.0, 2.0) * 0.030;
    let calm_atten = smoothstep(0.02, 0.18, energy);
    var base_intensity = (0.008 + clamp(energy, 0.0, 2.2) * 0.15) * (0.45 + 0.55 * calm_atten);

    if (kind == 5u || kind == 6u) {
        // Airborne breaking-crest spray droplets
        size = 0.026 + clamp(energy, 0.0, 2.2) * 0.026;
        base_intensity = (0.07 + clamp(energy, 0.0, 2.4) * 0.19) * (0.50 + 0.50 * calm_atten);
    } else if (kind == 7u) {
        // Soft low-alpha crest halo wisps
        size = 0.14 + clamp(energy, 0.0, 1.5) * 0.14;
        base_intensity = 0.015 + clamp(energy - 0.25, 0.0, 1.25) * 0.024;
    }

    // Perspective distance scaling guard: keep far particles cleanly readable without turning huge
    let near_fade = smoothstep(1.0, 3.0, depth);
    let far_fade = smoothstep_r(64.0, 24.0, depth);

    let local_offset = (axis_u * (in.position.x * stretch) + axis_v * in.position.y) * size;
    var view_pos = view_center;
    view_pos.x += local_offset.x;
    view_pos.y += local_offset.y;

    let clip_pos = camera.proj_matrix * view_pos;
    if (clip_pos.w <= 0.05) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.uv = vec2<f32>(0.0);
        out.core_col = vec3<f32>(0.0);
        out.halo_col = vec3<f32>(0.0);
        out.intensity = 0.0;
        return out;
    }

    let ndc = clip_pos.xy / clip_pos.w;
    if (abs(ndc.x) > 1.15 || abs(ndc.y) > 1.15) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.uv = vec2<f32>(0.0);
        out.core_col = vec3<f32>(0.0);
        out.halo_col = vec3<f32>(0.0);
        out.intensity = 0.0;
        return out;
    }

    // Authentic marine bioluminescence palette: deep sapphire -> electric cyan -> turquoise -> ice-white-cyan
    let excite = clamp((energy - 0.05) / 1.65, 0.0, 1.0);
    let core_col = mix(vec3<f32>(0.18, 0.85, 1.0), vec3<f32>(0.78, 0.99, 1.0), excite);
    let halo_teal = vec3<f32>(0.01, 0.38, 0.82);
    let halo_cyan = vec3<f32>(0.04, 0.76, 1.0);
    let halo_col = mix(halo_teal, halo_cyan, excite);

    out.clip_position = clip_pos;
    out.uv = in.tex_coords;
    out.core_col = core_col;
    out.halo_col = halo_col;
    out.intensity = base_intensity * near_fade * far_fade;
    return out;
}

@fragment
fn fs_main(in: ParticleVertexOutput) -> @location(0) vec4<f32> {
    if (in.intensity <= 0.0005) {
        discard;
    }

    let d_uv = in.uv - vec2<f32>(0.5, 0.5);
    let r2 = dot(d_uv, d_uv) * 4.0;
    if (r2 >= 1.0) {
        discard;
    }

    // Smooth C2 Gaussian-like falloff reaching exact 0.0 at rim (zero dark rings under additive blend)
    let w = 1.0 - r2;
    let w2 = w * w;
    let w4 = w2 * w2;
    let halo = w2 * w;
    let core = w4 * w4;

    var col = (in.halo_col * halo * 0.65 + in.core_col * core * 0.95) * in.intensity;
    // Soft highlight rolloff prevents harsh channel blowout when many sprites overlap
    col = col / (vec3<f32>(1.0) + col * 0.35);

    return vec4<f32>(col, 1.0);
}
