// INCLUDE: common

@group(0) @binding(0) var<uniform> audio: AudioUniforms;
@group(0) @binding(2) var history_tex: texture_2d<f32>;
@group(2) @binding(0) var<uniform> camera: CameraUniforms;

struct CameraUniforms {
    view_matrix: mat4x4<f32>,
    proj_matrix: mat4x4<f32>,
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
}

struct VertexOutput3D {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) mat: f32,
    @location(4) local_z: f32,
}

fn hash1(n: f32) -> f32 { return fract(sin(n) * 43758.5453123); }
fn hash2(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

@vertex
fn vs_main_3d(in: VertexInput) -> VertexOutput3D {
    var out: VertexOutput3D;
    
    let bass_pulse = clamp(audio.spectrum[2].x * 1.3, 0.0, 1.0);
    let mat_id = in.tex_coords.x;
    
    var pos = in.position;

    // Smooth lane steering sway without vertical hopping
    if (mat_id >= 2.5 && mat_id <= 9.5) {
        let drift_sway = sin(audio.smooth_time * 0.45) * 0.25;
        pos.x += drift_sway;
    }

    // Roadside Palm Trees & Streetlamps Streaming Continuously Past the Camera (-Z direction)
    if (mat_id >= 9.8 && mat_id <= 11.8) {
        let anchor_z = in.tex_coords.y;
        let loop_len = 312.0; // 12 props * 26.0 spacing
        let offset_from_anchor = pos.z - anchor_z;
        
        // Speed: 75.0 units/sec moving towards camera (-Z)
        let phase = fract((anchor_z - audio.smooth_time * 75.0) / loop_len);
        let looped_anchor_z = -10.0 + phase * loop_len;
        
        pos.z = looped_anchor_z + offset_from_anchor;
    }

    out.world_pos = pos;
    out.normal = in.normal;
    out.uv = in.tex_coords;
    out.mat = mat_id;
    out.local_z = pos.z;

    out.clip_position = camera.proj_matrix * camera.view_matrix * vec4<f32>(pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput3D) -> @location(0) vec4<f32> {
    let bass_pulse = clamp(audio.spectrum[2].x * 1.4, 0.0, 1.0);
    let treble_pulse = clamp(audio.spectrum[80].x * 1.6, 0.0, 1.0);
    let mat_id = in.mat;

    let v = normalize(vec3<f32>(0.0, 2.0, -4.6) - in.world_pos);
    let n = normalize(in.normal);
    let sun_dir = normalize(vec3<f32>(0.0, 0.25, 0.96)); // Low horizon synthwave sun
    let dx = abs(in.world_pos.x);

    var color = vec3<f32>(0.0);

    if (mat_id < 0.2) {
        // =========================================================================
        // 0.0: WET SPECULAR ASPHALT HIGHWAY (Forward Motion)
        // =========================================================================
        let speed_z = in.world_pos.z + audio.smooth_time * 75.0;

        // Subtle wet asphalt micro-grain texture
        let grain = hash2(vec2<f32>(floor(in.world_pos.x * 20.0), floor(speed_z * 20.0))) * 0.015;
        var asphalt = vec3<f32>(0.012, 0.010, 0.020) + vec3<f32>(grain);

        // 4-Lane Dashed White & Glowing Yellow Centerline
        let is_center = smoothstep(0.24, 0.0, dx);
        let is_center_dash = smoothstep(0.45, 0.85, sin(speed_z * 0.65));
        let yellow_line = vec3<f32>(1.0, 0.75, 0.05) * is_center * is_center_dash * (1.8 + treble_pulse * 0.8);

        let is_lane_l = smoothstep(0.14, 0.0, abs(dx - 4.5));
        let is_lane_dash = smoothstep(0.45, 0.85, sin(speed_z * 0.65));
        let white_line = vec3<f32>(0.95, 0.95, 1.0) * is_lane_l * is_lane_dash * 1.2;

        // Specular Mirror Reflection of the Giant Sun onto the wet road ahead
        let sun_refl_x = smoothstep(6.5, 0.0, dx);
        let sun_refl_z = smoothstep(12.0, 220.0, in.world_pos.z);
        let sun_refl_c = vec3<f32>(1.0, 0.25, 0.45) * sun_refl_x * sun_refl_z * (0.8 + bass_pulse * 0.5);

        // Brilliant Cyan Nitrous Ground Illumination (directly trailing car exhaust)
        let rel_car_z = in.world_pos.z - 5.2;
        var nitrous_pool = vec3<f32>(0.0);
        if (rel_car_z < 0.0 && rel_car_z > -10.0 && dx < 1.8) {
            let n_falloff = smoothstep(-10.0, 0.0, rel_car_z) * smoothstep(1.8, 0.0, dx);
            nitrous_pool = vec3<f32>(0.0, 0.92, 1.0) * n_falloff * (1.6 + bass_pulse * 2.5);
        }

        // Quad Taillight Red Ground Reflection Streak
        var taillight_streak = vec3<f32>(0.0);
        if (rel_car_z < 0.0 && rel_car_z > -22.0 && dx < 2.0) {
            let t_falloff = smoothstep(-22.0, 0.0, rel_car_z) * smoothstep(2.0, 0.0, dx);
            taillight_streak = vec3<f32>(0.99, 0.02, 0.02) * t_falloff * 1.5;
        }

        // Headlight Forward Beam Cone (Soft and realistic specular falloff)
        var headlight_cone = vec3<f32>(0.0);
        if (rel_car_z > 0.0 && rel_car_z < 65.0) {
            let beam_w = 2.0 + rel_car_z * 0.10;
            let beam_f = smoothstep(beam_w, 0.0, dx) * smoothstep(65.0, 0.0, rel_car_z);
            headlight_cone = vec3<f32>(0.95, 0.92, 0.80) * beam_f * 0.35;
        }

        color = asphalt + yellow_line + white_line + sun_refl_c + nitrous_pool + taillight_streak + headlight_cone;
    } else if (mat_id < 0.8) {
        // =========================================================================
        // 0.5: MIDDLEGROUND SYNTHWAVE OCEAN / WATER EXPANSE (Motion Parallax)
        // =========================================================================
        let dist_z = in.world_pos.z;
        let ground_dark = vec3<f32>(0.012, 0.004, 0.022);
        let ground_horizon = vec3<f32>(0.14, 0.02, 0.15);
        let ground_base = mix(ground_dark, ground_horizon, smoothstep(10.0, 320.0, dist_z));

        // Slow wave drift (15.0 units/sec = 20% of road speed 75.0, creating motion parallax)
        let water_z = in.world_pos.z + audio.smooth_time * 15.0;
        
        // Multi-frequency smooth undulating ocean ripples (no aliased dots)
        let wave1 = sin(in.world_pos.x * 0.08 + audio.smooth_time * 1.0) * cos(water_z * 0.06);
        let wave2 = sin(in.world_pos.x * 0.20 - water_z * 0.12 + audio.smooth_time * 1.6);
        let wave_h = (wave1 * 0.6 + wave2 * 0.4);

        // Specular sunset reflection across the ocean surface
        let refl_falloff = smoothstep(100.0, 9.0, dx);
        let sunset_spec = mix(vec3<f32>(0.85, 0.05, 0.45), vec3<f32>(1.0, 0.65, 0.15), smoothstep(20.0, 240.0, dist_z));
        let ocean_sheen = sunset_spec * (0.12 + 0.35 * smoothstep(-0.2, 0.8, wave_h)) * refl_falloff * (0.8 + bass_pulse * 0.6);

        // Subtle glowing magenta wave crests with distance anti-aliasing fade
        let crest_fade = smoothstep(220.0, 15.0, dist_z);
        let crest = smoothstep(0.60, 0.95, wave_h) * vec3<f32>(0.98, 0.02, 0.52) * 0.65 * crest_fade;

        color = ground_base + ocean_sheen + crest;
    } else if (mat_id < 1.5) {
        // 1.0: Elevated Bevel Curbs (Neon Magenta / Cyan Rumble Strips)
        let speed_z = in.world_pos.z + audio.smooth_time * 75.0;
        let curb_seg = ((i32(floor(speed_z * 0.45))) % 2) == 0;
        let c1 = vec3<f32>(0.98, 0.02, 0.52) * (1.6 + bass_pulse * 0.8);
        let c2 = vec3<f32>(0.0, 0.90, 0.98) * 1.5;
        color = select(c1, c2, curb_seg);
    } else if (mat_id < 2.5) {
        // 2.0: Concrete Barriers with Neon Edge Strip
        let concrete = vec3<f32>(0.08, 0.06, 0.10);
        let neon_strip = smoothstep(0.18, 0.0, abs(in.world_pos.y - 0.70)) * vec3<f32>(0.0, 0.92, 1.0) * 1.5;
        color = concrete + neon_strip;
    } else if (mat_id < 3.5) {
        // =========================================================================
        // 3.0: SUPERCAR CORSA RED GLOSS PAINT (Rich Shaded 3D Body)
        // =========================================================================
        let diff = max(dot(n, sun_dir), 0.0);
        let rear_light = max(dot(n, normalize(vec3<f32>(0.0, 0.5, -1.0))), 0.0);
        let h = normalize(sun_dir + v);
        let spec = pow(max(dot(n, h), 0.0), 24.0);
        let fresnel = pow(1.0 - max(dot(n, v), 0.0), 2.5);

        let body_base = vec3<f32>(0.92, 0.05, 0.03);
        let diffuse_term = body_base * (0.35 + diff * 0.45 + rear_light * 0.45);
        let spec_highlight = vec3<f32>(1.0, 0.85, 0.85) * spec * 1.2;
        let rim_glow = vec3<f32>(1.0, 0.15, 0.45) * fresnel * 0.8;
        color = diffuse_term + spec_highlight + rim_glow;
    } else if (mat_id < 4.5) {
        // 4.0: Cockpit Canopy Glass with Glowing Vector Green HUD
        let glass_base = vec3<f32>(0.02, 0.03, 0.05);
        let hud_lines = smoothstep(0.85, 0.98, sin(in.world_pos.y * 38.0)) * vec3<f32>(0.0, 1.0, 0.45) * 1.8;
        color = glass_base + hud_lines;
    } else if (mat_id < 6.5) {
        // =========================================================================
        // 6.0: QUAD CIRCULAR GLOWING RED TAILLIGHTS
        // =========================================================================
        let pulse = 2.0 + bass_pulse * 1.2;
        color = vec3<f32>(1.0, 0.02, 0.02) * pulse;
    } else if (mat_id < 7.2) {
        // 7.0: Dark Rubber Tire Treads
        color = vec3<f32>(0.015, 0.015, 0.018);
    } else if (mat_id < 7.8) {
        // 7.5: Cyan Glowing 5-Spoke Wheel Rim Stars
        color = vec3<f32>(0.0, 0.88, 0.98) * (1.6 + treble_pulse * 0.8);
    } else if (mat_id < 8.5) {
        // 8.0: Carbon Aero Splitters, Rear Fastback Louvers & Diffuser
        let louver = step(0.45, fract(in.world_pos.z * 18.0));
        let base_c = vec3<f32>(0.028, 0.028, 0.035);
        let slot_c = vec3<f32>(0.008, 0.008, 0.012);
        color = mix(base_c, slot_c, louver);
    } else if (mat_id < 9.5) {
        // =========================================================================
        // 9.0: DUAL NITROUS EXHAUST TIPS & CYAN PLASMA FLAME BLAST
        // =========================================================================
        let flame = sin(audio.smooth_time * 65.0) * 0.25 + 0.75;
        color = vec3<f32>(0.0, 0.95, 1.0) * (2.2 + bass_pulse * 2.5) * flame;
    } else if (mat_id < 10.8) {
        // 10.0 / 10.5: Roadside Palm Trees
        let is_crown = mat_id > 10.2;
        if (is_crown) {
            color = vec3<f32>(0.0, 1.0, 0.5) * (1.5 + bass_pulse * 1.2);
        } else {
            color = vec3<f32>(0.14, 0.05, 0.24);
        }
    } else if (mat_id < 11.8) {
        // 11.0 / 11.5: Cobra-Head Highway Streetlamps (.OBJ Mesh)
        let is_lamp_glow = mat_id > 11.2;
        if (is_lamp_glow) {
            color = vec3<f32>(0.99, 0.95, 0.85) * (3.5 + bass_pulse * 1.5);
        } else {
            color = vec3<f32>(0.15, 0.08, 0.22);
        }
    } else if (mat_id < 13.0) {
        // 12.0 / 12.5: Distant Horizon Skyscrapers (.OBJ Meshes)
        let is_spire = mat_id > 12.2;
        if (is_spire) {
            let blink = step(0.5, sin(audio.smooth_time * 4.0));
            color = vec3<f32>(1.0, 0.05, 0.02) * blink * 3.5;
        } else {
            let win_u = fract(in.world_pos.x * 0.25);
            let win_v = fract(in.world_pos.y * 0.25);
            let win_lit = step(0.4, win_u) * step(0.4, win_v) * step(0.35, hash2(vec2<f32>(floor(in.world_pos.x * 0.25), floor(in.world_pos.y * 0.25))));
            let win_col = mix(vec3<f32>(0.0, 0.85, 0.98), vec3<f32>(0.98, 0.02, 0.52), hash1(floor(in.world_pos.x * 0.3)));
            color = vec3<f32>(0.015, 0.005, 0.03) + win_col * win_lit * 2.2;
        }
    } else {
        color = vec3<f32>(0.1, 0.1, 0.1);
    }

    // Distance Atmospheric Vaporwave Fog (Blending into sunset horizon)
    let fog_f = smoothstep(45.0, 240.0, in.world_pos.z);
    let fog_c = vec3<f32>(0.28, 0.04, 0.32);
    color = mix(color, fog_c, fog_f);

    let tonemapped = aces_tonemap(color);
    return vec4<f32>(tonemapped, 1.0);
}
