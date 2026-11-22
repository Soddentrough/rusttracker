pub(in crate::engine) struct UiOverlayContext<'a> {
    pub central_rect: &'a mut egui::Rect,
    pub engine_actions: &'a mut Vec<EngineAction>,
    pub out_meters_rect: &'a mut Option<egui::Rect>,
    pub out_fire_rect: &'a mut Option<egui::Rect>,
    pub out_heatmap_rect: &'a mut Option<egui::Rect>,
    pub out_track_info_rect: &'a mut Option<egui::Rect>,
    pub out_top_panel_rect: &'a mut Option<egui::Rect>,
    pub out_video_rect: &'a mut Option<egui::Rect>,
    pub vis_name: &'a str,
    pub video_info_str: &'a Option<String>,
}

#[derive(Clone, PartialEq)]
pub enum EngineAction {
    None,
    Quit,
    OpenFile,
    LoadFiles(Vec<String>, bool),
    Seek(f32),
    ScrubPreview(f32, f64),
    ScrubEnd,
    SetForceStereo(bool),
    SetPassthrough(bool),
    SetSplitRatio(f32),
    SetAppendToPlaylist(bool),
    VisPickerSelect(usize),
    VisPickerToggleEnabled(usize),
    VisPickerSetCursor(usize),
    VisPickerEnableAll,
    VisPickerEnableNone,
    OpenUrlDialog,
    CloseUrlDialog,
    SetUrlInput(String),
    LoadUrl(String),
    LoadInputDevice(String),
    EditUrl(String),
    ClearFocusUrlInput,
    SetAudioDevice(String),
    ToggleAudioTrackInMix(usize),
    SetAudioTrackVolume(usize, f32),
    SetAudioMixTracks(Vec<(usize, f32)>),
    ToggleVisPicker,
    ToggleSurroundUpmix(bool),
    ToggleMpeghBitstream(bool),
    SetSurroundUpmix(crate::state::SurroundUpmixConfig),
    SetShowSurroundEq(bool),
    ToggleRoomVolumeSmoke,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AudioUniforms {
    pub spectrum: [f32; 1024],
    pub fire_heat: [f32; 1024],
    pub channels: [f32; 32],
    pub channel_peaks: [f32; 32],
    pub spatial_channels: [f32; 16],
    pub display_order: [u32; 16],
    pub channel_phases: [f32; 32],
    pub vu_segment_glows: [u32; 256],
    pub num_channels: u32,
    pub mode: u32,
    pub time: f32,
    pub duration: f32,
    pub smooth_time: f32,
    pub heatmap_row: u32,
    pub fft_channels: u32,
    pub num_spatial_channels: u32,
    pub ui_meters_rect: [f32; 4],
    pub ui_heatmap_rect: [f32; 4],
    pub ui_fire_rect: [f32; 4],
    pub waveform_resolution: u32,
    pub waveform_history_size: u32,
    pub frame_count: u32,
    pub step_fraction: f32,
    pub steps_to_fill: u32,
    pub aspect_ratio: f32,
    pub frame_dt: f32,
    pub history_cam_z: f32,
    pub fire_intensity: f32,
    pub waveform_history_ring_base: u32,
    pub _pad2: f32,
    pub _pad3: f32,
}

pub const WAVEFORM_HISTORY_RING_CAP: usize = 144;

#[inline]
pub fn compute_waveform_ring_base(ring_head: usize, logical_rows: usize) -> u32 {
    if logical_rows == 0 {
        0
    } else {
        ((ring_head + WAVEFORM_HISTORY_RING_CAP - (logical_rows % WAVEFORM_HISTORY_RING_CAP))
            % WAVEFORM_HISTORY_RING_CAP) as u32
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub fn is_waveform_history_dirty(
    push_count: u64,
    last_push_count: u64,
    hist_len: usize,
    last_hist_len: usize,
    vis_width: u32,
    last_vis_width: u32,
    vis_id: u32,
    last_vis_id: u32,
) -> bool {
    push_count != last_push_count
        || hist_len != last_hist_len
        || vis_width != last_vis_width
        || vis_id != last_vis_id
}

/// Default linear-space clear color for the application window and transparent top HUD panel.
/// In sRGB surface formats, `(0.1, 0.1, 0.1)` encodes to the standard `#595959` dark grey UI background.
///
/// IMPORTANT: Do NOT change this to black/dark-blue for 3D visualizers or startup; visualizer viewports
/// are cleared independently via `clear_black_pipeline` (or custom sky pipelines) within their scissor rect.
pub const APP_DEFAULT_BG_CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.1,
    g: 0.1,
    b: 0.1,
    a: 1.0,
};

pub fn is_vsync_present_mode(present_mode: wgpu::PresentMode) -> bool {
    matches!(
        present_mode,
        wgpu::PresentMode::Fifo | wgpu::PresentMode::FifoRelaxed | wgpu::PresentMode::AutoVsync
    )
}

pub fn format_vsync_status(present_mode: wgpu::PresentMode) -> &'static str {
    if is_vsync_present_mode(present_mode) {
        "VSYNC: ON"
    } else {
        "VSYNC: OFF"
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FireParams {
    pub bass: f32,
    pub mids: f32,
    pub highs: f32,
    pub time: f32,
    pub cooling_factor: f32,
    pub turb_spread_f: f32,
    pub width: u32,
    pub height: u32,
    pub num_channels: u32,
    pub lfe_idx: u32,
    pub fft_channels: u32,
    pub dt: f32,
    pub display_order: [u32; 16],
    pub channels: [[f32; 4]; 8],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VideoParams {
    pub color_space: u32,
    pub color_range: u32,
    pub bit_depth: u32,
    pub color_trc: u32,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub video_width: f32,
    pub video_height: f32,
    pub rotation: u32,
    pub chroma_layout: u32,
    pub _pad2: f32,
    pub _pad3: f32,
}

pub struct VideoState {
    pub y_texture: wgpu::Texture,
    pub u_texture: wgpu::Texture,
    pub v_texture: wgpu::Texture,
    pub border_blur_texture: wgpu::Texture,
    pub border_blur_view: wgpu::TextureView,
    pub blur_bind_group: wgpu::BindGroup,
    pub bind_group: wgpu::BindGroup,
    pub params_buffer: wgpu::Buffer,
    pub last_uploaded_params: Option<[u8; std::mem::size_of::<VideoParams>()]>,
    pub width: u32,
    pub height: u32,
    pub rotation: u32,
    pub color_space: u32,
    pub color_range: u32,
    pub bit_depth: u32,
    pub color_trc: u32,
    pub chroma_layout: crate::state::VideoChromaLayout,
    pub use_16bit_textures: bool,
    pub frame_generation: u64,
    pub blurred_generation: u64,
    pub blurred_color_key: (u32, u32, u32, u32, u32),
}

/// Optional modern `wgpu::Features` negotiated cleanly against adapter support with
/// graceful baseline fallback across Vulkan, Metal, DX12, and Android GPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegotiatedGpuFeatures {
    pub required_features: wgpu::Features,
    pub shader_f16: bool,
    pub subgroup: bool,
    pub subgroup_barrier: bool,
    pub indirect_first_instance: bool,
    pub multi_draw_indirect_count: bool,
    pub texture_format_16bit_norm: bool,
    pub pipeline_cache: bool,
    pub timestamp_query: bool,
}

pub fn negotiate_optional_gpu_features(
    adapter_features: wgpu::Features,
    enable_timestamps: bool,
    enable_pipeline_cache: bool,
) -> NegotiatedGpuFeatures {
    let mut required_features = wgpu::Features::empty();
    let mut try_enable = |feat: wgpu::Features, cond: bool| -> bool {
        if cond && adapter_features.contains(feat) {
            required_features |= feat;
            true
        } else {
            false
        }
    };

    let _ = try_enable(
        wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        true,
    );
    let texture_format_16bit_norm = try_enable(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM, true);
    let shader_f16 = try_enable(wgpu::Features::SHADER_F16, true);
    let subgroup = try_enable(wgpu::Features::SUBGROUP, true);
    let subgroup_barrier = try_enable(wgpu::Features::SUBGROUP_BARRIER, subgroup);
    let indirect_first_instance = try_enable(wgpu::Features::INDIRECT_FIRST_INSTANCE, true);
    let multi_draw_indirect_count = try_enable(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT, true);
    let pipeline_cache = try_enable(wgpu::Features::PIPELINE_CACHE, enable_pipeline_cache);
    let timestamp_query = try_enable(wgpu::Features::TIMESTAMP_QUERY, enable_timestamps);

    NegotiatedGpuFeatures {
        required_features,
        shader_f16,
        subgroup,
        subgroup_barrier,
        indirect_first_instance,
        multi_draw_indirect_count,
        texture_format_16bit_norm,
        pipeline_cache,
        timestamp_query,
    }
}

pub fn gpu_adapter_profile_from_wgpu(info: &wgpu::AdapterInfo) -> crate::state::GpuAdapterProfile {
    let device_type = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => crate::state::GpuDeviceType::DiscreteGpu,
        wgpu::DeviceType::IntegratedGpu => crate::state::GpuDeviceType::IntegratedGpu,
        wgpu::DeviceType::VirtualGpu => crate::state::GpuDeviceType::VirtualGpu,
        wgpu::DeviceType::Cpu => crate::state::GpuDeviceType::Cpu,
        wgpu::DeviceType::Other => crate::state::GpuDeviceType::Other,
    };
    crate::state::GpuAdapterProfile {
        name: info.name.clone(),
        device_type,
    }
}

pub fn touch_event_from_winit(touch: &winit::event::Touch) -> crate::touch::TouchEvent {
    let phase = match touch.phase {
        winit::event::TouchPhase::Started => crate::touch::TouchPhase::Started,
        winit::event::TouchPhase::Moved => crate::touch::TouchPhase::Moved,
        winit::event::TouchPhase::Ended => crate::touch::TouchPhase::Ended,
        winit::event::TouchPhase::Cancelled => crate::touch::TouchPhase::Cancelled,
    };
    crate::touch::TouchEvent {
        id: touch.id,
        phase,
        x: touch.location.x,
        y: touch.location.y,
    }
}

/// Exact `2π * 1000` (`2000π` ≈ 6283.185307179586s) host phase-wrapping period:
/// - Every 1-, 2-, and 3-decimal-place shader frequency (`0.18`, `0.225`, `0.35`, `0.45`, `0.86`,
///   `0.92`, `1.2`, `3.2`, etc.) completes an exact integer multiple of `2π` cycles across `P`.
/// - Values `>= P` wrap into `[P, 2*P)` (`max < 12,567s`, `< 1 ms` `f32` ULP) so `smooth_time > 0.0`
///   never dips back to `0.0` across wrap boundaries in shaders gating `select(audio.time, audio.smooth_time, audio.smooth_time > 0.0)`.
pub const HOST_TIME_WRAP_PERIOD_SECS: f64 = std::f64::consts::TAU * 1000.0;

#[inline]
pub fn wrap_host_phase_time(time_secs: f64) -> f32 {
    if !time_secs.is_finite() || time_secs <= 0.0 {
        return 0.0;
    }
    if time_secs < HOST_TIME_WRAP_PERIOD_SECS {
        time_secs as f32
    } else {
        (HOST_TIME_WRAP_PERIOD_SECS
            + (time_secs - HOST_TIME_WRAP_PERIOD_SECS).rem_euclid(HOST_TIME_WRAP_PERIOD_SECS))
            as f32
    }
}

pub const NEON_STUDIO_CAM_SHOT_SECS: f64 = 9.0;
pub const NEON_STUDIO_CAM_CYCLE_SECS: f64 = 54.0;

#[inline]
fn quintic_hermite_vec3(
    p0: glam::Vec3,
    p1: glam::Vec3,
    m0: glam::Vec3,
    m1: glam::Vec3,
    s: f32,
) -> glam::Vec3 {
    let s2 = s * s;
    let s3 = s2 * s;
    let s4 = s2 * s2;
    let s5 = s3 * s2;
    let h00 = 1.0 - 10.0 * s3 + 15.0 * s4 - 6.0 * s5;
    let h10 = s - 6.0 * s3 + 8.0 * s4 - 3.0 * s5;
    let h01 = 10.0 * s3 - 15.0 * s4 + 6.0 * s5;
    let h11 = -4.0 * s3 + 7.0 * s4 - 3.0 * s5;
    p0 * h00 + m0 * h10 + p1 * h01 + m1 * h11
}

/// Computes a 6-shot, 54-second `C2`-smooth "Dwell & Glide" camera tour `(eye, target)` for
/// Visualizer 31 (`Neon Acoustic Studio`), given `play_time` (which runs at `2.88x` real-time seconds).
pub fn compute_neon_acoustic_studio_camera(play_time: f64) -> (glam::Vec3, glam::Vec3) {
    #[derive(Clone, Copy)]
    struct Shot {
        eye_start: glam::Vec3,
        eye_end: glam::Vec3,
        target_start: glam::Vec3,
        target_end: glam::Vec3,
    }

    const fn shot(
        eye_start: [f32; 3],
        eye_end: [f32; 3],
        target_start: [f32; 3],
        target_end: [f32; 3],
    ) -> Shot {
        Shot {
            eye_start: glam::Vec3::from_array(eye_start),
            eye_end: glam::Vec3::from_array(eye_end),
            target_start: glam::Vec3::from_array(target_start),
            target_end: glam::Vec3::from_array(target_end),
        }
    }

    const SHOTS: [Shot; 6] = [
        // 0: Wide Rear Control-Booth Master
        shot(
            [2.60, 2.60, -7.85],
            [-1.80, 2.45, -7.65],
            [-0.20, 0.55, 3.80],
            [0.15, 0.65, 3.80],
        ),
        // 1: Low Sweet-Spot to Dual-15" Subwoofer & Stage Push-In
        shot(
            [-1.65, 0.22, -1.35],
            [-2.25, -0.08, 1.55],
            [0.10, -0.28, 5.35],
            [0.25, -0.20, 5.35],
        ),
        // 2: Front-Left Flank Reverse Studio & Control Room Live EQ Window Reveal
        shot(
            [-7.60, 1.55, 3.45],
            [-7.35, 2.05, 5.45],
            [0.45, 1.20, -5.40],
            [-0.10, 1.40, -6.50],
        ),
        // 3: High Stage-Catwalk Overhead Panorama (Atmos Cones, 3D Wave Volume & EQ Window)
        shot(
            [-2.40, 3.55, 6.95],
            [2.40, 3.55, 6.95],
            [0.25, 0.35, -2.20],
            [-0.25, 0.35, -2.20],
        ),
        // 4: Right Surround Gallery & Sweet-Spot Lateral Cross-Fire
        shot(
            [8.35, 1.05, 2.15],
            [8.05, 1.25, -2.85],
            [-1.20, 0.45, 1.80],
            [-1.30, 0.55, -0.20],
        ),
        // 5: High Rear-Right Corner Diagonal Studio Panorama
        shot(
            [7.15, 3.25, -7.35],
            [4.60, 2.95, -7.80],
            [-1.60, 0.45, 2.60],
            [-0.80, 0.50, 3.40],
        ),
    ];

    const DWELL_FRAC: f32 = 0.72;
    const TRANS_FRAC: f32 = 0.28;

    let real_secs = if play_time.is_finite() && play_time > 0.0 {
        (play_time / 2.88).rem_euclid(NEON_STUDIO_CAM_CYCLE_SECS)
    } else {
        0.0
    };
    let shot_pos = real_secs / NEON_STUDIO_CAM_SHOT_SECS;
    let shot_idx = (shot_pos.floor() as usize).min(SHOTS.len() - 1);
    let local_u = (shot_pos - shot_idx as f64) as f32;
    let curr = SHOTS[shot_idx];

    if local_u <= DWELL_FRAC {
        let d = (local_u / DWELL_FRAC).clamp(0.0, 1.0);
        (
            curr.eye_start.lerp(curr.eye_end, d),
            curr.target_start.lerp(curr.target_end, d),
        )
    } else {
        let next = SHOTS[(shot_idx + 1) % SHOTS.len()];
        let s = ((local_u - DWELL_FRAC) / TRANS_FRAC).clamp(0.0, 1.0);
        let scale = TRANS_FRAC / DWELL_FRAC;
        let eye_m0 = (curr.eye_end - curr.eye_start) * scale;
        let eye_m1 = (next.eye_end - next.eye_start) * scale;
        let tgt_m0 = (curr.target_end - curr.target_start) * scale;
        let tgt_m1 = (next.target_end - next.target_start) * scale;
        (
            quintic_hermite_vec3(curr.eye_end, next.eye_start, eye_m0, eye_m1, s),
            quintic_hermite_vec3(curr.target_end, next.target_start, tgt_m0, tgt_m1, s),
        )
    }
}

/// IEEE 754 binary32 -> binary16 (`f16`) round-to-nearest-even bit conversion matching
/// WGSL `pack2x16float` / `unpack2x16float` without requiring `enable f16;`.
#[inline]
pub fn f32_to_f16_bits(val: f32) -> u16 {
    let bits = val.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mant = bits & 0x007F_FFFF;

    if exp == 255 {
        return if mant != 0 {
            sign | 0x7E00
        } else {
            sign | 0x7C00
        };
    }

    let unbiased = exp - 127 + 15;
    if unbiased >= 31 {
        return sign | 0x7C00;
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign;
        }
        let full_mant = mant | 0x0080_0000;
        let shift = (14 - unbiased) as u32;
        let half_mant = (full_mant >> shift) as u16;
        let round_mask = 1u32 << (shift - 1);
        let rem_mask = round_mask - 1;
        let round_up =
            (full_mant & round_mask) != 0 && ((full_mant & rem_mask) != 0 || (half_mant & 1) != 0);
        return sign | half_mant.wrapping_add(u16::from(round_up));
    }

    let half_exp = (unbiased as u16) << 10;
    let half_mant = (mant >> 13) as u16;
    let round_bit = (mant & 0x1000) != 0;
    let sticky = (mant & 0x0FFF) != 0;
    let round_up = round_bit && (sticky || (half_mant & 1) != 0);
    sign | (half_exp + half_mant).wrapping_add(u16::from(round_up))
}

/// IEEE 754 binary16 (`f16`) -> binary32 exact conversion matching WGSL `unpack2x16float`.
#[inline]
pub fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = ((bits & 0x8000) as u32) << 16;
    let exp = ((bits >> 10) & 0x1F) as u32;
    let mant = (bits & 0x03FF) as u32;

    if exp == 0 {
        if mant == 0 {
            return f32::from_bits(sign);
        }
        let mut m = mant;
        let mut e = 127u32 - 14;
        while (m & 0x0400) == 0 {
            m <<= 1;
            e -= 1;
        }
        m &= 0x03FF;
        return f32::from_bits(sign | (e << 23) | (m << 13));
    }
    if exp == 31 {
        let f_mant = if mant != 0 { 0x0040_0000 } else { 0 };
        return f32::from_bits(sign | 0x7F80_0000 | f_mant);
    }
    let f_exp = (exp + 127 - 15) << 23;
    f32::from_bits(sign | f_exp | (mant << 13))
}

#[inline]
pub fn pack_f32_pair_to_u32(even: f32, odd: f32) -> u32 {
    (f32_to_f16_bits(even) as u32) | ((f32_to_f16_bits(odd) as u32) << 16)
}

#[inline]
pub fn pack_wave_row_f16(src_row: &[f32], dst_packed: &mut [u32]) {
    debug_assert_eq!(src_row.len(), dst_packed.len() * 2);
    for (word, pair) in dst_packed.iter_mut().zip(src_row.as_chunks::<2>().0) {
        *word = pack_f32_pair_to_u32(pair[0], pair[1]);
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniforms {
    pub view_matrix: [[f32; 4]; 4],
    pub proj_matrix: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tex_coords: [f32; 2],
}

impl Vertex {
    pub const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub(in crate::engine) struct MeshBuffers {
    pub(in crate::engine) vertex_buffer: wgpu::Buffer,
    pub(in crate::engine) index_buffer: wgpu::Buffer,
    pub(in crate::engine) vertex_capacity: u64,
    pub(in crate::engine) index_capacity: u64,
    pub(in crate::engine) index_count: u32,
}
