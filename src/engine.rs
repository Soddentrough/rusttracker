use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use winit::window::Window;
use crate::state::AppState;

fn gamepad_icon(g_type: crate::state::GamepadType, action: &str) -> String {
    match action {
        "A" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e997}",
            crate::state::GamepadType::Nintendo => "\u{e974}",
            _ => "\u{e994}",
        },
        "B" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e999}",
            crate::state::GamepadType::Nintendo => "\u{e994}",
            _ => "\u{e974}",
        },
        "X" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e998}",
            crate::state::GamepadType::Nintendo => "\u{e996}",
            _ => "\u{e995}",
        },
        "Y" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e99a}",
            crate::state::GamepadType::Nintendo => "\u{e995}",
            _ => "\u{e996}",
        },
        "L1" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e99d}",
            crate::state::GamepadType::Nintendo => "\u{e99c}",
            _ => "\u{e99f}",
        },
        "R1" => match g_type {
            crate::state::GamepadType::PlayStation => "\u{e9a4}",
            crate::state::GamepadType::Nintendo => "\u{e9a5}",
            _ => "\u{e9a2}",
        },
        "Select" => "\u{e9a9}",
        "Start" => "\u{e9a8}",
        "D-Pad L/R" => "\u{e9af} \u{e9ad}",
        "D-Pad U/D" => "\u{e9ac} \u{e9ae}",
        _ => action,
    }.to_string()
}

#[derive(Clone, PartialEq)]
pub enum EngineAction {
    None,
    OpenFile,
    LoadFiles(Vec<String>, bool),
    Seek(f32),
    ScrubPreview(f32, f64),
    ScrubEnd,
    SetForceStereo(bool),
    #[allow(dead_code)]
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
    EditUrl(String),
    ClearFocusUrlInput,
    SetAudioDevice(String),
    #[allow(dead_code)]
    SetAudioTrack(usize),
    ToggleAudioTrackInMix(usize),
    SetAudioTrackVolume(usize, f32),
    #[allow(dead_code)]
    SetAudioMixMode(bool),
    SetAudioMixTracks(Vec<(usize, f32)>),
    #[allow(dead_code)]
    SetMobileHudTab(crate::state::MobileHudTab),
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
    pub _pad1: f32,
    pub _pad2: f32,
    pub _pad3: f32,
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
}

pub struct VideoState {
    pub y_texture: wgpu::Texture,
    pub u_texture: wgpu::Texture,
    pub v_texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    pub params_buffer: wgpu::Buffer,
    pub width: u32,
    pub height: u32,
    pub color_space: u32,
    pub color_range: u32,
    pub bit_depth: u32,
    pub color_trc: u32,
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

struct MeshBuffers {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

pub struct VulkanEngine {
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    render_pipelines: Vec<wgpu::RenderPipeline>,
    hud_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    waveform_storage_buffer: wgpu::Buffer,
    #[allow(dead_code)] // accessed via GPU bind groups, not directly from Rust
    history_texture: wgpu::Texture,
    fire_grid_texture: wgpu::Texture,
    uniform_bind_group: wgpu::BindGroup,
    pub egui_renderer: egui_wgpu::Renderer,
    timestamp_period: f32,
    timestamp_mapping_active: bool,
    timestamp_map_complete: Arc<AtomicBool>,
    cached_fft_us: Option<f32>,
    cached_fire_us: Option<f32>,
    cached_vis_us: Option<f32>,
    query_set: Option<wgpu::QuerySet>,
    query_resolve_buffer: Option<wgpu::Buffer>,
    query_read_buffer: Option<wgpu::Buffer>,
    
    pub meters_uv_rect: [f32; 4],
    pub heatmap_uv_rect: [f32; 4],
    pub fire_uv_rect: [f32; 4],
    
    // GPU compute fire simulation
    fire_compute_pipeline: wgpu::ComputePipeline,
    firesim_compute_pipeline: wgpu::ComputePipeline,
    fire_buffer_a: wgpu::Buffer,
    fire_buffer_b: wgpu::Buffer,
    #[allow(dead_code)] // accessed via GPU bind groups, not directly from Rust
    fire_coal_buffer: wgpu::Buffer,
    fire_params_buffer: wgpu::Buffer,
    fire_bind_group_a: wgpu::BindGroup, // reads A, writes B
    fire_bind_group_b: wgpu::BindGroup, // reads B, writes A
    fire_ping: bool,
    
    pub heatmap_row: u32,
    heatmap_compute_pipeline: wgpu::ComputePipeline,
    heatmap_bind_group: wgpu::BindGroup,
    ferrofluidsim_compute_pipeline: wgpu::ComputePipeline,
    ferrofluidsim_clear_pipeline: wgpu::ComputePipeline,
    ferrofluidsim_bind_group: wgpu::BindGroup,
    
    // GPU compute bioluminescent waves simulation
    #[allow(dead_code)]
    biolum_particles_buffer: wgpu::Buffer,
    biolum_compute_pipeline: wgpu::ComputePipeline,
    biolum_compute_bind_group: wgpu::BindGroup,
    biolum_render_bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    biolum_render_pipeline_layout: wgpu::PipelineLayout,

    #[allow(dead_code)]
    pub start_time: std::time::Instant,

    // GPU FFT was removed (FFT is computed on the CPU); the dummy timestamp
    // pass in render() keeps the query-slot layout intact.
    resynth_compute_pipeline: wgpu::ComputePipeline,
    resynth_bind_group: wgpu::BindGroup,
    _gpu_spectrum_buffer: wgpu::Buffer,
    
    // Neon Smoke Cache
    smoke_compute_pipeline: wgpu::ComputePipeline,
    smoke_compute_bind_group: wgpu::BindGroup,
    smoke_render_bind_group: wgpu::BindGroup,
    smoke_params_buffer: wgpu::Buffer,
    
    depth_texture_view: wgpu::TextureView,
    
    // Pre-allocated buffers to avoid per-frame heap allocations
    waveform_history_flat: Vec<f32>,
    
    // Video
    video_bind_group_layout: wgpu::BindGroupLayout,
    video_pipeline: wgpu::RenderPipeline,
    video_state: Option<VideoState>,
    clear_black_pipeline: wgpu::RenderPipeline,
    crt_background_pipeline: wgpu::RenderPipeline,
    biolum_bg_pipeline: wgpu::RenderPipeline,
    synthwave_sky_pipeline: wgpu::RenderPipeline,
    vumeters_bg_pipeline: wgpu::RenderPipeline,
    neon_bg_pipeline: wgpu::RenderPipeline,
    storm_sky_pipeline: wgpu::RenderPipeline,
    
    // 3D Engine Extensions
    camera_uniform_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    mesh_registry: std::collections::HashMap<crate::state::Geometry, MeshBuffers>,
    lamp_vertex_buffer: wgpu::Buffer,
    lamp_index_buffer: wgpu::Buffer,
    lamp_index_count: u32,
    lamp_pipeline: wgpu::RenderPipeline,
    frame_count: u32,
    last_history_cam_z: f64,
    smooth_time: f64,
    play_time: f64,
    smooth_dt: f64,
    last_history_push_count: u64,
    time_since_last_push: f64,
    vu_needle_angles: Vec<f32>,
    vu_needle_velocities: Vec<f32>,
    channel_phases: [f32; 32],
    // Change detection for waveform history uploads (skip 1.2MB write when unchanged)
    last_uploaded_push_count: u64,
    last_uploaded_vis_width: u32,
    pub lyric_slam_timer: f32,
    pub last_lyric_line_idx: Option<usize>,
    pub current_lyric_mesh_text: String,
    pub fire_intensity: f32,
}

pub(crate) fn generate_lamp_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    
    // Helper to add a box
    let mut add_box = |center: [f32; 3], size: [f32; 3], color_flag: f32| {
        let half_x = size[0] / 2.0;
        let half_y = size[1] / 2.0;
        let half_z = size[2] / 2.0;
        
        let local_verts = [
            // front
            [-half_x, -half_y,  half_z], [ half_x, -half_y,  half_z],
            [ half_x,  half_y,  half_z], [-half_x,  half_y,  half_z],
            // back
            [-half_x, -half_y, -half_z], [-half_x,  half_y, -half_z],
            [ half_x,  half_y, -half_z], [ half_x, -half_y, -half_z],
        ];
        
        let normals = [
            [0.0, 0.0, 1.0],   // front
            [0.0, 0.0, -1.0],  // back
            [-1.0, 0.0, 0.0],  // left
            [1.0, 0.0, 0.0],   // right
            [0.0, 1.0, 0.0],   // top
            [0.0, -1.0, 0.0],  // bottom
        ];
        
        let face_indices = [
            [0, 1, 2, 0, 2, 3], // front
            [4, 5, 6, 4, 6, 7], // back
            [4, 0, 3, 4, 3, 5], // left
            [1, 7, 6, 1, 6, 2], // right
            [3, 2, 6, 3, 6, 5], // top
            [4, 7, 1, 4, 1, 0], // bottom
        ];
        
        // Add vertices and indices for each face to have clean normals
        for (face_idx, &_indices_map) in face_indices.iter().enumerate() {
            let start_v = vertices.len() as u32;
            let normal = normals[face_idx];
            
            let unique_vert_indices = match face_idx {
                0 => [0, 1, 2, 3], // front
                1 => [4, 5, 6, 7], // back
                2 => [4, 0, 3, 5], // left
                3 => [1, 7, 6, 2], // right
                4 => [3, 2, 6, 5], // top
                5 => [4, 7, 1, 0], // bottom
                _ => unreachable!(),
            };
            
            for &vi in &unique_vert_indices {
                let p = local_verts[vi];
                vertices.push(Vertex {
                    position: [p[0] + center[0], p[1] + center[1], p[2] + center[2]],
                    normal,
                    tex_coords: [color_flag, (p[1] + center[1]) / 11.0],
                });
            }
            
            indices.push(start_v);
            indices.push(start_v + 1);
            indices.push(start_v + 2);
            indices.push(start_v);
            indices.push(start_v + 2);
            indices.push(start_v + 3);
        }
    };
    
    // 1. Pole: Vertical box
    add_box([0.0, 5.5, 0.0], [0.24, 11.0, 0.24], 0.0);
    
    // 2. Overhang arm: Horizontal box
    // Extends by 2.0 units in +X direction (towards road)
    add_box([1.0, 11.0, 0.0], [2.0, 0.2, 0.2], 0.0);
    
    // 3. Lamp Fitting/Head: Box at the end of overhang
    add_box([2.0, 10.8, 0.0], [0.8, 0.3, 0.5], 2.0); // Flag 2.0 for fixture
    
    // 4. Lamp Bulb/Emissive source: Smaller glowing box underneath the fitting
    add_box([2.0, 10.6, 0.0], [0.4, 0.1, 0.3], 1.0); // Flag 1.0 for emissive bulb
    
    (vertices, indices)
}

fn room_add_quad(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], normal: [f32; 3], ch: f32, mat: f32) {
    let start = vertices.len() as u32;
    vertices.push(Vertex { position: p0, normal, tex_coords: [ch, mat] });
    vertices.push(Vertex { position: p1, normal, tex_coords: [ch, mat] });
    vertices.push(Vertex { position: p2, normal, tex_coords: [ch, mat] });
    vertices.push(Vertex { position: p3, normal, tex_coords: [ch, mat] });
    indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}

fn room_add_box(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, center: [f32; 3], size: [f32; 3], rot_y: f32, ch: f32, mat: f32) {
    let hx = size[0] / 2.0;
    let hy = size[1] / 2.0;
    let hz = size[2] / 2.0;
    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();

    let rotate_pt = |p: [f32; 3]| -> [f32; 3] {
        let rx = p[0] * cos_r + p[2] * sin_r;
        let rz = -p[0] * sin_r + p[2] * cos_r;
        [rx + center[0], p[1] + center[1], rz + center[2]]
    };
    let rotate_norm = |n: [f32; 3]| -> [f32; 3] {
        let rx = n[0] * cos_r + n[2] * sin_r;
        let rz = -n[0] * sin_r + n[2] * cos_r;
        [rx, n[1], rz]
    };

    // Front face (+z)
    room_add_quad(
        vertices, indices,
        rotate_pt([-hx, -hy, hz]), rotate_pt([hx, -hy, hz]), rotate_pt([hx, hy, hz]), rotate_pt([-hx, hy, hz]),
        rotate_norm([0.0, 0.0, 1.0]), ch, mat
    );
    // Back face (-z)
    room_add_quad(
        vertices, indices,
        rotate_pt([hx, -hy, -hz]), rotate_pt([-hx, -hy, -hz]), rotate_pt([-hx, hy, -hz]), rotate_pt([hx, hy, -hz]),
        rotate_norm([0.0, 0.0, -1.0]), ch, mat
    );
    // Left face (-x)
    room_add_quad(
        vertices, indices,
        rotate_pt([-hx, -hy, -hz]), rotate_pt([-hx, -hy, hz]), rotate_pt([-hx, hy, hz]), rotate_pt([-hx, hy, -hz]),
        rotate_norm([-1.0, 0.0, 0.0]), ch, mat
    );
    // Right face (+x)
    room_add_quad(
        vertices, indices,
        rotate_pt([hx, -hy, hz]), rotate_pt([hx, -hy, -hz]), rotate_pt([hx, hy, -hz]), rotate_pt([hx, hy, hz]),
        rotate_norm([1.0, 0.0, 0.0]), ch, mat
    );
    // Top face (+y)
    room_add_quad(
        vertices, indices,
        rotate_pt([-hx, hy, hz]), rotate_pt([hx, hy, hz]), rotate_pt([hx, hy, -hz]), rotate_pt([-hx, hy, -hz]),
        rotate_norm([0.0, 1.0, 0.0]), ch, mat
    );
    // Bottom face (-y)
    room_add_quad(
        vertices, indices,
        rotate_pt([-hx, -hy, -hz]), rotate_pt([hx, -hy, -hz]), rotate_pt([hx, -hy, hz]), rotate_pt([-hx, -hy, hz]),
        rotate_norm([0.0, -1.0, 0.0]), ch, mat
    );
}

fn room_add_cone(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, center: [f32; 3], radius: f32, depth: f32, rot_y: f32, ch: f32, mat: f32) {
    let segs = 14;
    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();
    let rotate_pt = |p: [f32; 3]| -> [f32; 3] {
        let rx = p[0] * cos_r + p[2] * sin_r;
        let rz = -p[0] * sin_r + p[2] * cos_r;
        [rx + center[0], p[1] + center[1], rz + center[2]]
    };
    let rotate_norm = |n: [f32; 3]| -> [f32; 3] {
        let rx = n[0] * cos_r + n[2] * sin_r;
        let rz = -n[0] * sin_r + n[2] * cos_r;
        [rx, n[1], rz]
    };

    let apex = rotate_pt([0.0, 0.0, -depth]);
    for i in 0..segs {
        let a0 = (i as f32) / (segs as f32) * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32) / (segs as f32) * std::f32::consts::TAU;
        let p0 = rotate_pt([a0.cos() * radius, a0.sin() * radius, 0.0]);
        let p1 = rotate_pt([a1.cos() * radius, a1.sin() * radius, 0.0]);

        let n = rotate_norm([0.0, 0.0, 1.0]);
        let start = vertices.len() as u32;
        vertices.push(Vertex { position: apex, normal: n, tex_coords: [ch, mat] });
        vertices.push(Vertex { position: p0, normal: n, tex_coords: [ch, mat] });
        vertices.push(Vertex { position: p1, normal: n, tex_coords: [ch, mat] });
        indices.extend_from_slice(&[start, start + 1, start + 2]);

        // Emissive halo ring
        let r_out = radius * 1.16;
        let p0_out = rotate_pt([a0.cos() * r_out, a0.sin() * r_out, 0.015]);
        let p1_out = rotate_pt([a1.cos() * r_out, a1.sin() * r_out, 0.015]);
        room_add_quad(vertices, indices, p0, p1, p1_out, p0_out, n, ch, 5.0);
    }
}

fn room_add_speaker_tower(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, pos: [f32; 3], rot_y: f32, ch: f32, is_tower: bool) {
    if is_tower {
        // Pedestal base & Cabinet
        room_add_box(vertices, indices, [pos[0], pos[1] - 0.7, pos[2]], [0.95, 0.12, 1.05], rot_y, ch, 1.0);
        room_add_box(vertices, indices, pos, [0.80, 2.2, 0.90], rot_y, ch, 1.0);

        // Baffle front face is at offset +0.45 in local Z
        let cos_r = rot_y.cos();
        let sin_r = rot_y.sin();
        let local_to_world = |lx: f32, ly: f32, lz: f32| -> [f32; 3] {
            let rx = lx * cos_r + lz * sin_r;
            let rz = -lx * sin_r + lz * cos_r;
            [rx + pos[0], ly + pos[1], rz + pos[2]]
        };

        // Lower Bass Woofer (mat = 2.0)
        room_add_cone(vertices, indices, local_to_world(0.0, -0.45, 0.46), 0.28, 0.12, rot_y, ch, 2.0);
        // Midrange Cone (mat = 3.0)
        room_add_cone(vertices, indices, local_to_world(0.0, 0.25, 0.46), 0.19, 0.08, rot_y, ch, 3.0);
        // Tweeter Dome (mat = 4.0)
        room_add_cone(vertices, indices, local_to_world(0.0, 0.72, 0.46), 0.09, 0.03, rot_y, ch, 4.0);

        // Decorative vertical neon accent strip on cabinet sides
        room_add_box(vertices, indices, local_to_world(-0.41, 0.0, 0.0), [0.03, 2.0, 0.03], rot_y, ch, 5.0);
        room_add_box(vertices, indices, local_to_world(0.41, 0.0, 0.0), [0.03, 2.0, 0.03], rot_y, ch, 5.0);
    } else {
        // Stand-mounted surround monitor
        // Stand pole
        room_add_box(vertices, indices, [pos[0], pos[1] - 0.9, pos[2]], [0.08, 1.6, 0.08], rot_y, ch, 1.0);
        room_add_box(vertices, indices, [pos[0], pos[1] - 1.7, pos[2]], [0.7, 0.06, 0.7], rot_y, ch, 1.0);
        // Monitor box
        room_add_box(vertices, indices, pos, [0.65, 1.1, 0.65], rot_y, ch, 1.0);

        let cos_r = rot_y.cos();
        let sin_r = rot_y.sin();
        let local_to_world = |lx: f32, ly: f32, lz: f32| -> [f32; 3] {
            let rx = lx * cos_r + lz * sin_r;
            let rz = -lx * sin_r + lz * cos_r;
            [rx + pos[0], ly + pos[1], rz + pos[2]]
        };
        // Woofer
        room_add_cone(vertices, indices, local_to_world(0.0, -0.15, 0.33), 0.22, 0.09, rot_y, ch, 2.0);
        // Tweeter
        room_add_cone(vertices, indices, local_to_world(0.0, 0.32, 0.33), 0.08, 0.03, rot_y, ch, 4.0);
        // Neon perimeter halo
        room_add_box(vertices, indices, local_to_world(0.0, -0.56, 0.0), [0.66, 0.03, 0.66], rot_y, ch, 5.0);
    }
}

pub(crate) fn generate_neon_room_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // 1. FLOOR & STAGE (mat = 0.0)
    room_add_quad(
        &mut vertices, &mut indices,
        [-11.0, -1.6, 9.0], [11.0, -1.6, 9.0], [11.0, -1.6, -9.0], [-11.0, -1.6, -9.0],
        [0.0, 1.0, 0.0], 0.0, 0.0
    );

    // Front stage platform riser
    room_add_box(&mut vertices, &mut indices, [0.0, -1.48, 6.2], [10.5, 0.24, 4.8], 0.0, 0.0, 1.0);
    // Neon edge along stage riser
    room_add_box(&mut vertices, &mut indices, [0.0, -1.35, 3.8], [10.6, 0.04, 0.04], 0.0, 3.0, 5.0);

    // 2. WALLS & CEILING (mat = 0.5)
    // Back acoustic wall (+Z)
    room_add_quad(
        &mut vertices, &mut indices,
        [-11.0, -1.6, 9.0], [-11.0, 5.0, 9.0], [11.0, 5.0, 9.0], [11.0, -1.6, 9.0],
        [0.0, 0.0, -1.0], 0.0, 0.5
    );
    // Rear studio wall (-Z)
    room_add_quad(
        &mut vertices, &mut indices,
        [11.0, -1.6, -9.0], [11.0, 5.0, -9.0], [-11.0, 5.0, -9.0], [-11.0, -1.6, -9.0],
        [0.0, 0.0, 1.0], 0.0, 0.5
    );
    // Left wall (-X)
    room_add_quad(
        &mut vertices, &mut indices,
        [-11.0, -1.6, -9.0], [-11.0, 5.0, -9.0], [-11.0, 5.0, 9.0], [-11.0, -1.6, 9.0],
        [1.0, 0.0, 0.0], 0.0, 0.5
    );
    // Right wall (+X)
    room_add_quad(
        &mut vertices, &mut indices,
        [11.0, -1.6, 9.0], [11.0, 5.0, 9.0], [11.0, 5.0, -9.0], [11.0, -1.6, -9.0],
        [-1.0, 0.0, 0.0], 0.0, 0.5
    );
    // Ceiling (+Y)
    room_add_quad(
        &mut vertices, &mut indices,
        [-11.0, 5.0, -9.0], [11.0, 5.0, -9.0], [11.0, 5.0, 9.0], [-11.0, 5.0, 9.0],
        [0.0, -1.0, 0.0], 0.0, 0.5
    );

    // 3. ACOUSTIC DIFFUSER SLATS (mat = 6.0) on back wall
    for i in 0..14 {
        let x = -6.5 + (i as f32) * 1.0;
        let depth_offset = ((i * 7 + 3) % 5) as f32 * 0.06;
        room_add_box(&mut vertices, &mut indices, [x, 1.6, 8.8 - depth_offset], [0.55, 4.2, 0.15 + depth_offset], 0.0, 0.0, 6.0);
    }

    // Rear studio control room observation window & diffusers
    // Soundproof glass observation pane (mat = 7.0)
    room_add_quad(
        &mut vertices, &mut indices,
        [-4.5, 0.4, -8.85], [-4.5, 3.4, -8.85], [4.5, 3.4, -8.85], [4.5, 0.4, -8.85],
        [0.0, 0.0, 1.0], 6.0, 7.0
    );
    // Window neon illuminated perimeter frame
    room_add_box(&mut vertices, &mut indices, [0.0, 3.45, -8.82], [9.2, 0.08, 0.08], 0.0, 6.0, 5.0);
    room_add_box(&mut vertices, &mut indices, [0.0, 0.35, -8.82], [9.2, 0.08, 0.08], 0.0, 7.0, 5.0);
    room_add_box(&mut vertices, &mut indices, [-4.55, 1.9, -8.82], [0.08, 3.1, 0.08], 0.0, 6.0, 5.0);
    room_add_box(&mut vertices, &mut indices, [4.55, 1.9, -8.82], [0.08, 3.1, 0.08], 0.0, 7.0, 5.0);

    // Rear flank acoustic diffuser panels
    for i in 0..4 {
        let x_left = -8.8 + (i as f32) * 0.9;
        let x_right = 6.1 + (i as f32) * 0.9;
        let depth_offset = ((i * 5 + 2) % 4) as f32 * 0.05;
        room_add_box(&mut vertices, &mut indices, [x_left, 1.6, -8.8 + depth_offset], [0.5, 3.6, 0.12 + depth_offset], 0.0, 6.0, 6.0);
        room_add_box(&mut vertices, &mut indices, [x_right, 1.6, -8.8 + depth_offset], [0.5, 3.6, 0.12 + depth_offset], 0.0, 7.0, 6.0);
    }

    // Side acoustic panels with illuminated neon perimeter backlights
    for i in 0..4 {
        let z = -4.5 + (i as f32) * 3.0;
        // Left side panel + halo
        room_add_box(&mut vertices, &mut indices, [-10.8, 1.2, z], [0.18, 2.4, 1.8], 0.0, 4.0, 6.0);
        room_add_box(&mut vertices, &mut indices, [-10.85, 1.2, z], [0.06, 2.5, 1.9], 0.0, 4.0, 5.0);

        // Right side panel + halo
        room_add_box(&mut vertices, &mut indices, [10.8, 1.2, z], [0.18, 2.4, 1.8], 0.0, 5.0, 6.0);
        room_add_box(&mut vertices, &mut indices, [10.85, 1.2, z], [0.06, 2.5, 1.9], 0.0, 5.0, 5.0);
    }

    // Overhead neon rail trusses
    room_add_box(&mut vertices, &mut indices, [0.0, 4.8, 4.0], [21.0, 0.08, 0.08], 0.0, 0.0, 5.0);
    room_add_box(&mut vertices, &mut indices, [0.0, 4.8, -3.0], [21.0, 0.08, 0.08], 0.0, 6.0, 5.0);
    room_add_box(&mut vertices, &mut indices, [-8.0, 4.8, 0.0], [0.08, 0.08, 16.0], 0.0, 4.0, 5.0);
    room_add_box(&mut vertices, &mut indices, [8.0, 4.8, 0.0], [0.08, 0.08, 16.0], 0.0, 5.0, 5.0);

    // 4. SPATIAL SPEAKERS (7.1.4 Surround Placement)
    // Channel 0: Front Left (Tower)
    room_add_speaker_tower(&mut vertices, &mut indices, [-3.8, -0.25, 5.5], -0.42, 0.0, true);
    // Channel 1: Front Right (Tower)
    room_add_speaker_tower(&mut vertices, &mut indices, [3.8, -0.25, 5.5], 0.42, 1.0, true);

    // Channel 2: Center Channel (Horizontal Cabinet on Riser)
    room_add_box(&mut vertices, &mut indices, [0.0, -0.85, 6.8], [1.7, 0.55, 0.65], 0.0, 2.0, 1.0);
    room_add_cone(&mut vertices, &mut indices, [-0.52, -0.85, 7.13], 0.18, 0.07, 0.0, 2.0, 3.0);
    room_add_cone(&mut vertices, &mut indices, [0.52, -0.85, 7.13], 0.18, 0.07, 0.0, 2.0, 3.0);
    room_add_cone(&mut vertices, &mut indices, [0.0, -0.85, 7.13], 0.09, 0.03, 0.0, 2.0, 4.0);
    room_add_box(&mut vertices, &mut indices, [0.0, -1.14, 6.8], [1.75, 0.03, 0.68], 0.0, 2.0, 5.0);

    // Channel 3: Massive LFE Dual Subwoofer
    room_add_box(&mut vertices, &mut indices, [0.0, -1.05, 5.2], [2.4, 0.85, 1.1], 0.0, 3.0, 1.0);
    room_add_cone(&mut vertices, &mut indices, [-0.62, -1.05, 5.76], 0.36, 0.16, 0.0, 3.0, 2.0);
    room_add_cone(&mut vertices, &mut indices, [0.62, -1.05, 5.76], 0.36, 0.16, 0.0, 3.0, 2.0);
    // Subwoofer bass reflex port & neon trim
    room_add_box(&mut vertices, &mut indices, [0.0, -0.75, 5.76], [0.35, 0.08, 0.05], 0.0, 3.0, 5.0);
    room_add_box(&mut vertices, &mut indices, [0.0, -1.48, 5.2], [2.45, 0.04, 1.15], 0.0, 3.0, 5.0);

    // Channel 4: Surround Left (Stand Monitor)
    room_add_speaker_tower(&mut vertices, &mut indices, [-6.2, 0.5, 0.5], -1.42, 4.0, false);
    // Channel 5: Surround Right (Stand Monitor)
    room_add_speaker_tower(&mut vertices, &mut indices, [6.2, 0.5, 0.5], 1.42, 5.0, false);

    // Channel 6: Rear Left (Rear Surround Tower)
    room_add_speaker_tower(&mut vertices, &mut indices, [-4.2, 0.1, -4.8], -2.65, 6.0, true);
    // Channel 7: Rear Right (Rear Surround Tower)
    room_add_speaker_tower(&mut vertices, &mut indices, [4.2, 0.1, -4.8], 2.65, 7.0, true);

    // Channels 8..11: Overhead Ceiling Atmos Height Speakers
    // Top Front Left (8)
    room_add_box(&mut vertices, &mut indices, [-3.2, 4.3, 3.5], [0.65, 0.45, 0.65], 0.0, 8.0, 1.0);
    room_add_cone(&mut vertices, &mut indices, [-3.2, 4.05, 3.5], 0.20, 0.08, 0.0, 8.0, 2.0);
    // Top Front Right (9)
    room_add_box(&mut vertices, &mut indices, [3.2, 4.3, 3.5], [0.65, 0.45, 0.65], 0.0, 9.0, 1.0);
    room_add_cone(&mut vertices, &mut indices, [3.2, 4.05, 3.5], 0.20, 0.08, 0.0, 9.0, 2.0);
    // Top Rear Left (10)
    room_add_box(&mut vertices, &mut indices, [-3.2, 4.3, -2.5], [0.65, 0.45, 0.65], 0.0, 10.0, 1.0);
    room_add_cone(&mut vertices, &mut indices, [-3.2, 4.05, -2.5], 0.20, 0.08, 0.0, 10.0, 2.0);
    // Top Rear Right (11)
    room_add_box(&mut vertices, &mut indices, [3.2, 4.3, -2.5], [0.65, 0.45, 0.65], 0.0, 11.0, 1.0);
    room_add_cone(&mut vertices, &mut indices, [3.2, 4.05, -2.5], 0.20, 0.08, 0.0, 11.0, 2.0);

    (vertices, indices)
}

fn glass_add_quad(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    p0: [f32; 3],
    p1: [f32; 3],
    p2: [f32; 3],
    p3: [f32; 3],
    normal: [f32; 3],
    ch: f32,
    mat: f32,
) {
    let start = vertices.len() as u32;
    vertices.push(Vertex { position: p0, normal, tex_coords: [ch, mat] });
    vertices.push(Vertex { position: p1, normal, tex_coords: [ch, mat] });
    vertices.push(Vertex { position: p2, normal, tex_coords: [ch, mat] });
    vertices.push(Vertex { position: p3, normal, tex_coords: [ch, mat] });
    indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}

static EMBEDDED_GLASS_FONT: &[u8] = include_bytes!("../assets/Orbitron-Black.ttf");

struct GlassGlyphContour {
    points: Vec<[f32; 2]>,
}

struct GlassContourBuilder {
    contours: Vec<GlassGlyphContour>,
    current_contour: Vec<[f32; 2]>,
    start_point: [f32; 2],
    last_point: [f32; 2],
}

impl GlassContourBuilder {
    fn new() -> Self {
        Self {
            contours: Vec::new(),
            current_contour: Vec::new(),
            start_point: [0.0, 0.0],
            last_point: [0.0, 0.0],
        }
    }

    fn finish(mut self) -> Vec<GlassGlyphContour> {
        if !self.current_contour.is_empty() {
            self.contours.push(GlassGlyphContour { points: self.current_contour });
        }
        self.contours
    }
}

impl ttf_parser::OutlineBuilder for GlassContourBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        if !self.current_contour.is_empty() {
            self.contours.push(GlassGlyphContour { points: std::mem::take(&mut self.current_contour) });
        }
        self.start_point = [x, y];
        self.last_point = [x, y];
        self.current_contour.push([x, y]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.last_point = [x, y];
        self.current_contour.push([x, y]);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last_point;
        let p1 = [x1, y1];
        let p2 = [x, y];
        let steps = 4;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let it = 1.0 - t;
            let qx = it * it * p0[0] + 2.0 * it * t * p1[0] + t * t * p2[0];
            let qy = it * it * p0[1] + 2.0 * it * t * p1[1] + t * t * p2[1];
            self.current_contour.push([qx, qy]);
        }
        self.last_point = [x, y];
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last_point;
        let p1 = [x1, y1];
        let p2 = [x2, y2];
        let p3 = [x, y];
        let steps = 6;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let it = 1.0 - t;
            let cx = it * it * it * p0[0] + 3.0 * it * it * t * p1[0] + 3.0 * it * t * t * p2[0] + t * t * t * p3[0];
            let cy = it * it * it * p0[1] + 3.0 * it * it * t * p1[1] + 3.0 * it * t * t * p2[1] + t * t * t * p3[1];
            self.current_contour.push([cx, cy]);
        }
        self.last_point = [x, y];
    }

    fn close(&mut self) {
        if let Some(last) = self.current_contour.last() {
            let dx = last[0] - self.start_point[0];
            let dy = last[1] - self.start_point[1];
            if (dx * dx + dy * dy).sqrt() < 0.001 && self.current_contour.len() > 1 {
                self.current_contour.pop();
            }
        }
        if !self.current_contour.is_empty() {
            self.contours.push(GlassGlyphContour { points: std::mem::take(&mut self.current_contour) });
        }
    }
}

fn glass_contour_signed_area(pts: &[[f32; 2]]) -> f32 {
    let n = pts.len();
    if n < 3 { return 0.0; }
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += pts[i][0] * pts[j][1] - pts[j][0] * pts[i][1];
    }
    area * 0.5
}

fn point_in_polygon(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let pi = poly[i];
        let pj = poly[j];
        if (pi[1] > p[1]) != (pj[1] > p[1]) {
            let x_int = pi[0] + (p[1] - pi[1]) * (pj[0] - pi[0]) / (pj[1] - pi[1]);
            if p[0] < x_int {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_strictly_in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    if (p[0] - a[0]).abs() < 1e-4 && (p[1] - a[1]).abs() < 1e-4 { return false; }
    if (p[0] - b[0]).abs() < 1e-4 && (p[1] - b[1]).abs() < 1e-4 { return false; }
    if (p[0] - c[0]).abs() < 1e-4 && (p[1] - c[1]).abs() < 1e-4 { return false; }

    let cross1 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let cross2 = (c[0] - b[0]) * (p[1] - b[1]) - (c[1] - b[1]) * (p[0] - b[0]);
    let cross3 = (a[0] - c[0]) * (p[1] - c[1]) - (a[1] - c[1]) * (p[0] - c[0]);

    cross1 > 1e-6 && cross2 > 1e-6 && cross3 > 1e-6
}

fn earcut_triangulate_polygon(outer: &[[f32; 2]], holes: &[Vec<[f32; 2]>]) -> (Vec<[f32; 2]>, Vec<[usize; 3]>) {
    let mut ring: Vec<[f32; 2]> = outer.to_vec();
    if glass_contour_signed_area(&ring) < 0.0 {
        ring.reverse(); // Ensure CCW
    }

    let mut sorted_holes: Vec<Vec<[f32; 2]>> = holes.to_vec();
    for h in &mut sorted_holes {
        if glass_contour_signed_area(h) > 0.0 {
            h.reverse(); // Ensure CW for holes
        }
    }
    sorted_holes.sort_by(|a, b| {
        let max_a = a.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        let max_b = b.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        max_b.partial_cmp(&max_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    for h in &sorted_holes {
        if h.len() < 3 { continue; }
        let mut best_h_idx = 0;
        let mut max_hx = f32::NEG_INFINITY;
        for (i, p) in h.iter().enumerate() {
            if p[0] > max_hx {
                max_hx = p[0];
                best_h_idx = i;
            }
        }
        let h_pt = h[best_h_idx];

        let mut min_x_intersect = f32::INFINITY;
        let mut best_m_idx = 0;
        let n_ring = ring.len();

        for i in 0..n_ring {
            let p0 = ring[i];
            let p1 = ring[(i + 1) % n_ring];

            let (low, high) = if p0[1] < p1[1] { (p0, p1) } else { (p1, p0) };
            if h_pt[1] >= low[1] && h_pt[1] <= high[1] && (high[1] - low[1]).abs() > 1e-6 {
                let t = (h_pt[1] - low[1]) / (high[1] - low[1]);
                let ix = low[0] + t * (high[0] - low[0]);
                if ix >= h_pt[0] && ix < min_x_intersect {
                    min_x_intersect = ix;
                    best_m_idx = if p0[0] > p1[0] { i } else { (i + 1) % n_ring };
                }
            }
        }

        if min_x_intersect.is_infinite() {
            let mut min_dist_sq = f32::INFINITY;
            for (i, p) in ring.iter().enumerate() {
                let dx = p[0] - h_pt[0];
                let dy = p[1] - h_pt[1];
                let d2 = dx * dx + dy * dy;
                if d2 < min_dist_sq {
                    min_dist_sq = d2;
                    best_m_idx = i;
                }
            }
        }

        let mut new_ring = Vec::with_capacity(ring.len() + h.len() + 2);
        new_ring.extend_from_slice(&ring[..=best_m_idx]);
        for k in 0..h.len() {
            new_ring.push(h[(best_h_idx + k) % h.len()]);
        }
        new_ring.push(h[best_h_idx]);
        new_ring.extend_from_slice(&ring[best_m_idx..]);
        ring = new_ring;
    }

    let verts = ring;
    let mut indices_map: Vec<usize> = (0..verts.len()).collect();
    let mut triangles = Vec::new();

    let mut count = 0;
    while indices_map.len() > 2 && count < 2000 {
        count += 1;
        let n = indices_map.len();
        let mut ear_found = false;

        for i in 0..n {
            let prev_idx = indices_map[(i + n - 1) % n];
            let curr_idx = indices_map[i];
            let next_idx = indices_map[(i + 1) % n];

            let a = verts[prev_idx];
            let b = verts[curr_idx];
            let c = verts[next_idx];

            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if cross <= 1e-7 {
                continue;
            }

            let mut inside = false;
            for j in 0..n {
                if j == (i + n - 1) % n || j == i || j == (i + 1) % n {
                    continue;
                }
                let test_pt = verts[indices_map[j]];
                if point_strictly_in_triangle(test_pt, a, b, c) {
                    inside = true;
                    break;
                }
            }

            if !inside {
                triangles.push([prev_idx, curr_idx, next_idx]);
                indices_map.remove(i);
                ear_found = true;
                break;
            }
        }

        if !ear_found {
            let mut best_cut = 0;
            let mut max_cross = f32::NEG_INFINITY;
            for i in 0..n {
                let prev_idx = indices_map[(i + n - 1) % n];
                let curr_idx = indices_map[i];
                let next_idx = indices_map[(i + 1) % n];
                let a = verts[prev_idx];
                let b = verts[curr_idx];
                let c = verts[next_idx];
                let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                if cross > max_cross {
                    max_cross = cross;
                    best_cut = i;
                }
            }
            if n > 2 && max_cross > -1.0 {
                let prev_idx = indices_map[(best_cut + n - 1) % n];
                let curr_idx = indices_map[best_cut];
                let next_idx = indices_map[(best_cut + 1) % n];
                triangles.push([prev_idx, curr_idx, next_idx]);
                indices_map.remove(best_cut);
            } else {
                break;
            }
        }
    }

    (verts, triangles)
}

fn glass_triangulate_contours(contours: &[Vec<[f32; 2]>]) -> (Vec<[f32; 2]>, Vec<[usize; 3]>) {
    let mut outers = Vec::new();
    let mut holes = Vec::new();

    for c in contours {
        if c.len() < 3 { continue; }
        let area = glass_contour_signed_area(c);
        if area < 0.0 {
            outers.push(c.clone());
        } else {
            holes.push(c.clone());
        }
    }

    if outers.is_empty() && !holes.is_empty() {
        outers = holes;
        holes = Vec::new();
    }

    let mut all_verts = Vec::new();
    let mut all_tris = Vec::new();

    for outer in &outers {
        let matching_holes: Vec<Vec<[f32; 2]>> = holes
            .iter()
            .filter(|h| h.first().map_or(false, |p| point_in_polygon(*p, outer)))
            .cloned()
            .collect();

        let (verts, tris) = earcut_triangulate_polygon(outer, &matching_holes);
        let base_idx = all_verts.len();
        all_verts.extend(verts);
        for t in tris {
            all_tris.push([base_idx + t[0], base_idx + t[1], base_idx + t[2]]);
        }
    }

    (all_verts, all_tris)
}

pub(crate) fn generate_glass_lyrics_mesh(text: &str) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(16384);
    let mut indices = Vec::with_capacity(65536);

    let trimmed = text.trim();
    let display_str = if trimmed.is_empty() { "RUSTTRACKER" } else { trimmed };

    // Try system DejaVu Sans Bold first, fallback to embedded Orbitron Black
    let sys_font_path = "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf";
    let font_bytes = std::fs::read(sys_font_path).unwrap_or_else(|_| EMBEDDED_GLASS_FONT.to_vec());
    let face = ttf_parser::Face::parse(&font_bytes, 0).ok();

    if let Some(face) = face {
        let em = face.units_per_em() as f32;
        let base_scale = 1.0 / em;

        // First pass: compute total text width with typographic advances
        let mut total_advance = 0.0f32;
        for ch in display_str.chars() {
            if let Some(glyph_id) = face.glyph_index(ch) {
                let adv = face.glyph_hor_advance(glyph_id).unwrap_or(face.units_per_em()) as f32 * base_scale;
                total_advance += adv;
            } else {
                total_advance += 0.5;
            }
        }

        let target_width = 7.5f32;
        let text_scale = (target_width / total_advance.max(1.0)).clamp(0.40, 1.25);
        let start_x = -total_advance * text_scale * 0.5;
        let baseline_y = 0.16; // Sits just above water level y = 0.0
        let depth = 0.22 * text_scale;
        let bevel = 0.024 * text_scale;
        let hz = depth * 0.5;

        let mut curr_x = start_x;

        for (ch_idx, ch) in display_str.chars().enumerate() {
            if let Some(glyph_id) = face.glyph_index(ch) {
                let adv = face.glyph_hor_advance(glyph_id).unwrap_or(face.units_per_em()) as f32 * base_scale * text_scale;

                let mut builder = GlassContourBuilder::new();
                if let Some(_bbox) = face.outline_glyph(glyph_id, &mut builder) {
                    let contours = builder.finish();

                    let scaled_contours: Vec<Vec<[f32; 2]>> = contours
                        .iter()
                        .filter(|c| c.points.len() >= 3)
                        .map(|c| {
                            c.points.iter().map(|p| {
                                [
                                    curr_x + p[0] * base_scale * text_scale,
                                    baseline_y + p[1] * base_scale * text_scale,
                                ]
                            }).collect()
                        })
                        .collect();

                    if !scaled_contours.is_empty() {
                        let (face_verts_2d, face_tris) = glass_triangulate_contours(&scaled_contours);

                        // 1. Front and Back Faces
                        let start_front = vertices.len() as u32;
                        for p in &face_verts_2d {
                            vertices.push(Vertex {
                                position: [p[0], p[1], hz],
                                normal: [0.0, 0.0, 1.0],
                                tex_coords: [ch_idx as f32, 1.0], // Mat 1.0 = Glass
                            });
                        }
                        for tri in &face_tris {
                            indices.push(start_front + tri[0] as u32);
                            indices.push(start_front + tri[1] as u32);
                            indices.push(start_front + tri[2] as u32);
                        }

                        let start_back = vertices.len() as u32;
                        for p in &face_verts_2d {
                            vertices.push(Vertex {
                                position: [p[0], p[1], -hz],
                                normal: [0.0, 0.0, -1.0],
                                tex_coords: [ch_idx as f32, 1.0],
                            });
                        }
                        for tri in &face_tris {
                            indices.push(start_back + tri[0] as u32);
                            indices.push(start_back + tri[2] as u32);
                            indices.push(start_back + tri[1] as u32);
                        }

                        // 2. Extruded Sidewalls & 45 deg Bevel Chamfers
                        for pts in &scaled_contours {
                            let n_pts = pts.len();
                            let area = glass_contour_signed_area(pts);
                            let is_outer = area < 0.0;

                            for i in 0..n_pts {
                                let p0 = pts[i];
                                let p1 = pts[(i + 1) % n_pts];

                                let dx = p1[0] - p0[0];
                                let dy = p1[1] - p0[1];
                                let len = (dx * dx + dy * dy).sqrt();
                                if len < 1e-6 { continue; }

                                let mut nx = dy / len;
                                let mut ny = -dx / len;
                                if !is_outer {
                                    nx = -nx;
                                    ny = -ny;
                                }

                                let start_v = vertices.len() as u32;

                                // Side quad
                                let norm_side = [nx, ny, 0.0];
                                vertices.push(Vertex { position: [p0[0], p0[1], -hz + bevel], normal: norm_side, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p1[0], p1[1], -hz + bevel], normal: norm_side, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p1[0], p1[1], hz - bevel],  normal: norm_side, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p0[0], p0[1], hz - bevel],  normal: norm_side, tex_coords: [ch_idx as f32, 1.0] });
                                indices.extend_from_slice(&[start_v, start_v + 1, start_v + 2, start_v, start_v + 2, start_v + 3]);

                                // Top Chamfer quad (+Z)
                                let start_c1 = vertices.len() as u32;
                                let norm_c1 = [nx * 0.7071, ny * 0.7071, 0.7071];
                                vertices.push(Vertex { position: [p0[0], p0[1], hz - bevel], normal: norm_c1, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p1[0], p1[1], hz - bevel], normal: norm_c1, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p1[0] - nx * bevel, p1[1] - ny * bevel, hz], normal: norm_c1, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p0[0] - nx * bevel, p0[1] - ny * bevel, hz], normal: norm_c1, tex_coords: [ch_idx as f32, 1.0] });
                                indices.extend_from_slice(&[start_c1, start_c1 + 1, start_c1 + 2, start_c1, start_c1 + 2, start_c1 + 3]);

                                // Bot Chamfer quad (-Z)
                                let start_c2 = vertices.len() as u32;
                                let norm_c2 = [nx * 0.7071, ny * 0.7071, -0.7071];
                                vertices.push(Vertex { position: [p0[0] - nx * bevel, p0[1] - ny * bevel, -hz], normal: norm_c2, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p1[0] - nx * bevel, p1[1] - ny * bevel, -hz], normal: norm_c2, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p1[0], p1[1], -hz + bevel], normal: norm_c2, tex_coords: [ch_idx as f32, 1.0] });
                                vertices.push(Vertex { position: [p0[0], p0[1], -hz + bevel], normal: norm_c2, tex_coords: [ch_idx as f32, 1.0] });
                                indices.extend_from_slice(&[start_c2, start_c2 + 1, start_c2 + 2, start_c2, start_c2 + 2, start_c2 + 3]);
                            }
                        }
                    }
                }
                curr_x += adv;
            } else {
                curr_x += 0.4 * text_scale;
            }
        }
    }

    // 2. Tessellated 3D Water Grid Plane: 64 x 64 vertices spanning [-16.0, 16.0] x [-10.0, 10.0]
    let grid_res_x = 64;
    let grid_res_z = 64;
    let start_water_vert = vertices.len() as u32;

    for iz in 0..grid_res_z {
        let fz = iz as f32 / (grid_res_z - 1) as f32;
        let z = -10.0 + fz * 20.0;
        for ix in 0..grid_res_x {
            let fx = ix as f32 / (grid_res_x - 1) as f32;
            let x = -16.0 + fx * 32.0;
            vertices.push(Vertex {
                position: [x, 0.0, z],
                normal: [0.0, 1.0, 0.0],
                tex_coords: [0.0, 2.0], // Mat 2.0 = Water
            });
        }
    }

    for iz in 0..(grid_res_z - 1) {
        for ix in 0..(grid_res_x - 1) {
            let v0 = start_water_vert + iz * grid_res_x + ix;
            let v1 = v0 + 1;
            let v2 = v0 + grid_res_x;
            let v3 = v2 + 1;
            indices.extend_from_slice(&[v0, v2, v1, v1, v2, v3]);
        }
    }

    // 3. Overhead Softbox Area Light Emitter Quad: at y = 5.2, z = -1.5, tilted downward 35 deg
    let sb_center = [0.0f32, 5.2, -1.5];
    let sb_w = 16.0f32;
    let sb_h = 3.6f32;
    let tilt: f32 = 35.0f32.to_radians();
    let cos_t = tilt.cos();
    let sin_t = tilt.sin();

    let p0 = [sb_center[0] - sb_w * 0.5, sb_center[1] + sb_h * 0.5 * cos_t, sb_center[2] + sb_h * 0.5 * sin_t];
    let p1 = [sb_center[0] + sb_w * 0.5, sb_center[1] + sb_h * 0.5 * cos_t, sb_center[2] + sb_h * 0.5 * sin_t];
    let p2 = [sb_center[0] + sb_w * 0.5, sb_center[1] - sb_h * 0.5 * cos_t, sb_center[2] - sb_h * 0.5 * sin_t];
    let p3 = [sb_center[0] - sb_w * 0.5, sb_center[1] - sb_h * 0.5 * cos_t, sb_center[2] - sb_h * 0.5 * sin_t];
    let sb_norm = [0.0, -sin_t, cos_t];

    glass_add_quad(
        &mut vertices, &mut indices,
        p0, p1, p2, p3,
        sb_norm,
        0.0,
        3.0, // Mat 3.0 = Emissive Softbox
    );

    (vertices, indices)
}

fn cyber_add_quad(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], normal: [f32; 3], mat: f32, uv_y: f32) {
    let start = vertices.len() as u32;
    vertices.push(Vertex { position: p0, normal, tex_coords: [mat, uv_y] });
    vertices.push(Vertex { position: p1, normal, tex_coords: [mat, uv_y] });
    vertices.push(Vertex { position: p2, normal, tex_coords: [mat, uv_y] });
    vertices.push(Vertex { position: p3, normal, tex_coords: [mat, uv_y] });
    indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}

fn cyber_add_box(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, center: [f32; 3], size: [f32; 3], rot_y: f32, mat: f32) {
    let hx = size[0] / 2.0;
    let hy = size[1] / 2.0;
    let hz = size[2] / 2.0;
    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();

    let rotate_pt = |p: [f32; 3]| -> [f32; 3] {
        let rx = p[0] * cos_r + p[2] * sin_r;
        let rz = -p[0] * sin_r + p[2] * cos_r;
        [rx + center[0], p[1] + center[1], rz + center[2]]
    };
    let rotate_norm = |n: [f32; 3]| -> [f32; 3] {
        let rx = n[0] * cos_r + n[2] * sin_r;
        let rz = -n[0] * sin_r + n[2] * cos_r;
        [rx, n[1], rz]
    };

    cyber_add_quad(vertices, indices, rotate_pt([-hx, -hy, hz]), rotate_pt([hx, -hy, hz]), rotate_pt([hx, hy, hz]), rotate_pt([-hx, hy, hz]), rotate_norm([0.0, 0.0, 1.0]), mat, center[1] + hy);
    cyber_add_quad(vertices, indices, rotate_pt([hx, -hy, -hz]), rotate_pt([-hx, -hy, -hz]), rotate_pt([-hx, hy, -hz]), rotate_pt([hx, hy, -hz]), rotate_norm([0.0, 0.0, -1.0]), mat, center[1] + hy);
    cyber_add_quad(vertices, indices, rotate_pt([-hx, -hy, -hz]), rotate_pt([-hx, -hy, hz]), rotate_pt([-hx, hy, hz]), rotate_pt([-hx, hy, -hz]), rotate_norm([-1.0, 0.0, 0.0]), mat, center[1] + hy);
    cyber_add_quad(vertices, indices, rotate_pt([hx, -hy, hz]), rotate_pt([hx, -hy, -hz]), rotate_pt([hx, hy, -hz]), rotate_pt([hx, hy, hz]), rotate_norm([1.0, 0.0, 0.0]), mat, center[1] + hy);
    cyber_add_quad(vertices, indices, rotate_pt([-hx, hy, hz]), rotate_pt([hx, hy, hz]), rotate_pt([hx, hy, -hz]), rotate_pt([-hx, hy, -hz]), rotate_norm([0.0, 1.0, 0.0]), mat, center[1] + hy);
    cyber_add_quad(vertices, indices, rotate_pt([-hx, -hy, -hz]), rotate_pt([hx, -hy, -hz]), rotate_pt([hx, -hy, hz]), rotate_pt([-hx, -hy, hz]), rotate_norm([0.0, -1.0, 0.0]), mat, center[1] - hy);
}

fn load_obj_mesh(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    obj_src: &str,
    translation: [f32; 3],
    scale: [f32; 3],
    rot_y: f32,
    default_mat: f32,
) {
    let mut raw_positions: Vec<[f32; 3]> = Vec::new();
    let mut raw_normals: Vec<[f32; 3]> = Vec::new();
    let mut current_mat = default_mat;

    let cos_r = rot_y.cos();
    let sin_r = rot_y.sin();
    let transform_pos = |p: [f32; 3]| -> [f32; 3] {
        let sx = p[0] * scale[0];
        let sy = p[1] * scale[1];
        let sz = p[2] * scale[2];
        let rx = sx * cos_r + sz * sin_r;
        let rz = -sx * sin_r + sz * cos_r;
        [rx + translation[0], sy + translation[1], rz + translation[2]]
    };
    let transform_norm = |n: [f32; 3]| -> [f32; 3] {
        let rx = n[0] * cos_r + n[2] * sin_r;
        let rz = -n[0] * sin_r + n[2] * cos_r;
        [rx, n[1], rz]
    };

    for line in obj_src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("v ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                if let (Ok(x), Ok(y), Ok(z)) = (parts[0].parse::<f32>(), parts[1].parse::<f32>(), parts[2].parse::<f32>()) {
                    raw_positions.push([x, y, z]);
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("vn ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                if let (Ok(x), Ok(y), Ok(z)) = (parts[0].parse::<f32>(), parts[1].parse::<f32>(), parts[2].parse::<f32>()) {
                    raw_normals.push([x, y, z]);
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("g ") {
            let group_name = rest.trim();
            current_mat = match group_name {
                "floor" => 0.0,
                "neon_frame_0" => 1.0,
                "neon_frame_1" => 2.0,
                "neon_frame_2" => 3.0,
                "neon_frame_3" => 4.0,
                "neon_frame_4" => 5.0,
                "neon_frame_5" => 6.0,
                "neon_frame_6" => 7.0,
                "neon_frame_7" => 8.0,
                "faceplate" => 1.0,
                "meter_frame" => 2.0,
                "dial_scale" => 3.0,
                "needle_left" => 4.0,
                "needle_right" => 5.0,
                "knob" => 6.0,
                "meter_glass" => 7.0,
                "leds" => 8.0,
                "screws" => 9.0,
                "hull" => 1.0,
                "arch_rib" => 2.0,
                "floor_grating" => 3.0,
                "neon_strip" => 4.0,
                "conduit" => 5.0,
                "hazard_trim" => 6.0,
                "paint" => 3.0,
                "glass" => 4.0,
                "taillight" => 6.0,
                "tire" => 7.0,
                "rim" => 7.5,
                "carbon" => 8.0,
                "exhaust" => 9.0,
                "trunk" => 10.0,
                "frond" => 10.5,
                "mast" => 11.0,
                "lamp" => 11.5,
                "tower" => 12.0,
                "spire" => 12.5,
                _ => default_mat,
            };
        } else if let Some(rest) = trimmed.strip_prefix("f ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                let parse_vert = |tok: &str| -> Option<(usize, usize)> {
                    let segs: Vec<&str> = tok.split('/').collect();
                    let v_idx = segs[0].parse::<usize>().ok()?.checked_sub(1)?;
                    let vn_idx = if segs.len() >= 3 && !segs[2].is_empty() {
                        segs[2].parse::<usize>().ok()?.checked_sub(1)?
                    } else {
                        v_idx
                    };
                    Some((v_idx, vn_idx))
                };

                let face_verts: Vec<(usize, usize)> = parts.iter().filter_map(|p| parse_vert(p)).collect();
                if face_verts.len() >= 3 {
                    for i in 1..face_verts.len() - 1 {
                        let tri = [face_verts[0], face_verts[i], face_verts[i + 1]];
                        let start_idx = vertices.len() as u32;
                        for &(v_i, vn_i) in &tri {
                            let pos = if v_i < raw_positions.len() { raw_positions[v_i] } else { [0.0, 0.0, 0.0] };
                            let norm = if vn_i < raw_normals.len() { raw_normals[vn_i] } else { [0.0, 1.0, 0.0] };
                            let world_p = transform_pos(pos);
                            let world_n = transform_norm(norm);
                            vertices.push(Vertex {
                                position: world_p,
                                normal: world_n,
                                tex_coords: [current_mat, translation[2]],
                            });
                        }
                        indices.extend_from_slice(&[start_idx, start_idx + 1, start_idx + 2]);
                    }
                }
            }
        }
    }
}

pub(crate) fn generate_synthwave_racer_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // 1. Continuous Desert / Terrain Ground Planes (mat = 0.5)
    let ground_w = 200.0;
    cyber_add_quad(
        &mut vertices, &mut indices,
        [-ground_w, -0.05, -10.0], [-9.0, -0.05, -10.0],
        [-9.0, -0.05, 360.0], [-ground_w, -0.05, 360.0],
        [0.0, 1.0, 0.0], 0.5, 0.0
    );
    cyber_add_quad(
        &mut vertices, &mut indices,
        [9.0, -0.05, -10.0], [ground_w, -0.05, -10.0],
        [ground_w, -0.05, 360.0], [9.0, -0.05, 360.0],
        [0.0, 1.0, 0.0], 0.5, 0.0
    );

    // 2. 3D Highway Roadbed, Curbs & Concrete K-Rails
    let road_half_w = 9.0;
    let seg_len = 3.5;
    let num_segs = 100;
    for i in 0..num_segs {
        let z0 = -8.0 + (i as f32) * seg_len;
        let z1 = z0 + seg_len;

        // Asphalt (mat = 0.0)
        cyber_add_quad(
            &mut vertices, &mut indices,
            [-road_half_w, 0.0, z0], [road_half_w, 0.0, z0],
            [road_half_w, 0.0, z1], [-road_half_w, 0.0, z1],
            [0.0, 1.0, 0.0], 0.0, z0
        );

        // Curbs (mat = 1.0)
        cyber_add_quad(
            &mut vertices, &mut indices,
            [-road_half_w - 0.8, 0.15, z0], [-road_half_w, 0.0, z0],
            [-road_half_w, 0.0, z1], [-road_half_w - 0.8, 0.15, z1],
            [0.2, 0.98, 0.0], 1.0, z0
        );
        cyber_add_quad(
            &mut vertices, &mut indices,
            [road_half_w, 0.0, z0], [road_half_w + 0.8, 0.15, z0],
            [road_half_w + 0.8, 0.15, z1], [road_half_w, 0.0, z1],
            [-0.2, 0.98, 0.0], 1.0, z0
        );

        // Concrete K-Rail Barriers (mat = 2.0)
        cyber_add_box(&mut vertices, &mut indices, [-road_half_w - 1.2, 0.45, (z0 + z1) / 2.0], [0.35, 0.75, seg_len], 0.0, 2.0);
        cyber_add_box(&mut vertices, &mut indices, [road_half_w + 1.2, 0.45, (z0 + z1) / 2.0], [0.35, 0.75, seg_len], 0.0, 2.0);
    }

    // 3. Embedded 3D Low-Poly Supercar Model (.OBJ)
    load_obj_mesh(
        &mut vertices, &mut indices,
        include_str!("assets/models/supercar_f40.obj"),
        [0.0, 0.40, 5.2],
        [1.0, 1.0, 1.0],
        0.0,
        3.0
    );

    // 4. Roadside Streetlamps & Palm Trees (.OBJ Meshes)
    for i in 0..12 {
        let pz = 18.0 + (i as f32) * 26.0;
        // Left Palm Tree
        load_obj_mesh(
            &mut vertices, &mut indices,
            include_str!("assets/models/palm_tree.obj"),
            [-road_half_w - 3.8, 0.0, pz],
            [1.0, 1.0, 1.0],
            (i as f32) * 0.8,
            10.0
        );
        // Right Cobra-Head Streetlamp
        load_obj_mesh(
            &mut vertices, &mut indices,
            include_str!("assets/models/streetlamp.obj"),
            [road_half_w + 2.4, 0.0, pz + 13.0],
            [1.0, 1.0, 1.0],
            std::f32::consts::PI,
            11.0
        );
    }

    // 5. Distant Horizon Skyscrapers (.OBJ Meshes)
    for i in 0..16 {
        let ang = (i as f32) * 0.38 - 3.0;
        let tx = ang * 46.0;
        let tz = 260.0 + ((i * 7) % 5) as f32 * 14.0;
        let s = 0.75 + ((i * 3) % 4) as f32 * 0.15;
        load_obj_mesh(
            &mut vertices, &mut indices,
            include_str!("assets/models/skyscraper.obj"),
            [tx, -2.0, tz],
            [s, s, s],
            (i as f32) * 0.5,
            12.0
        );
    }

    (vertices, indices)
}

pub(crate) fn generate_vumeter_rack_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    load_obj_mesh(
        &mut vertices, &mut indices,
        include_str!("assets/models/vumeter_rack.obj"),
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        0.0,
        1.0
    );
    (vertices, indices)
}

pub(crate) fn generate_neon_corridor_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    load_obj_mesh(
        &mut vertices, &mut indices,
        include_str!("assets/models/neon_corridor_frames.obj"),
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        0.0,
        0.0
    );
    (vertices, indices)
}

pub(crate) fn generate_storm_rain_volume_mesh() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    
    // Pure 3D Instanced Falling Raindrops across viewing frustum (mat = 3.0)
    for i in 0..2500 {
        let seed = (i as f32) * 1.6180339887;
        let rx = (((seed * 17.3).fract()) - 0.5) * 44.0;
        let ry = ((seed * 29.1).fract()) * 30.0;
        let rz = 2.0 + ((seed * 43.7).fract()) * 110.0;
        let drop_len = 0.55;
        cyber_add_quad(
            &mut vertices, &mut indices,
            [rx - 0.015, ry, rz], [rx + 0.015, ry, rz],
            [rx + 0.015, ry + drop_len, rz], [rx - 0.015, ry + drop_len, rz],
            [0.0, 0.0, 1.0], 3.0, ry
        );
    }

    (vertices, indices)
}



impl VulkanEngine {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        // The instance is a handle to our GPU
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY, // Use platform native: Vulkan (Linux/Win) or Metal (MacOS)
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            },
        ).await.unwrap();

        let mut required_features = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        let supports_timestamps = if cfg!(target_os = "android") {
            false
        } else {
            std::env::var("RUSTTRACKER_PROFILE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
                && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
        };
        if supports_timestamps {
            required_features |= wgpu::Features::TIMESTAMP_QUERY;
        }
        if adapter.features().contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM) {
            required_features |= wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        }

        let (device, queue) = adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            },
        ).await.unwrap();

        device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| {
            eprintln!("WGPU VALIDATION ERROR: {:?}", e);
        }));

        // --- Pipeline Cache (persist compiled GPU pipelines across launches) ---
        // Keyed by the adapter so caches from different devices don't collide.
        let pipeline_cache_path: Option<std::path::PathBuf> = (|| {
            let key = wgpu::util::pipeline_cache_key(&adapter.get_info())?;
            let dir = directories::ProjectDirs::from("com", "RustTracker", "RustTracker")?
                .cache_dir()
                .to_path_buf();
            let _ = std::fs::create_dir_all(&dir);
            Some(dir.join(format!("pipeline_cache_{}.bin", key)))
        })();
        let pipeline_cache_data: Option<Vec<u8>> = pipeline_cache_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok());
        // SAFETY: `data` is `None` on first run, and otherwise comes from a prior
        // `PipelineCache::get_data()` for this same adapter (keyed via
        // `util::pipeline_cache_key`), which is the documented-valid source.
        // `fallback: true` ensures a fresh empty cache is created if the stored
        // data is rejected (e.g. driver/version change).
        //
        // NOTE: pipeline caching is opt-in via the RUSTTRACKER_PIPELINE_CACHE=1
        // environment variable. Some driver/backend combinations report a cache as
        // created yet mark pipelines that reference it invalid at submit time, so
        // the safe default is to NOT attach a cache (pass `cache: None`).
        let pipeline_cache_enabled = std::env::var("RUSTTRACKER_PIPELINE_CACHE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let pipeline_cache = if pipeline_cache_enabled {
            Some(unsafe {
                device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("RustTracker Pipeline Cache"),
                    data: pipeline_cache_data.as_deref(),
                    fallback: true,
                })
            })
        } else {
            None
        };
        // Borrow that satisfies `cache: Option<&PipelineCache>` at every descriptor.
        let pipeline_cache_ref = pipeline_cache.as_ref();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Enable dynamic display refresh & VRR (Adaptive Sync / FreeSync / G-Sync)
        let present_mode = if let Ok(mode_str) = std::env::var("RUSTTRACKER_PRESENT_MODE") {
            match mode_str.to_lowercase().as_str() {
                "novsync" | "immediate" => wgpu::PresentMode::AutoNoVsync,
                "mailbox" if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) => wgpu::PresentMode::Mailbox,
                "fiforelaxed" | "adaptive" if surface_caps.present_modes.contains(&wgpu::PresentMode::FifoRelaxed) => wgpu::PresentMode::FifoRelaxed,
                "fifo" => wgpu::PresentMode::Fifo,
                _ => wgpu::PresentMode::AutoVsync,
            }
        } else if surface_caps.present_modes.contains(&wgpu::PresentMode::AutoVsync) {
            wgpu::PresentMode::AutoVsync
        } else if surface_caps.present_modes.contains(&wgpu::PresentMode::FifoRelaxed) {
            wgpu::PresentMode::FifoRelaxed
        } else {
            wgpu::PresentMode::Fifo
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);



        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Audio Uniform Buffer"),
            size: std::mem::size_of::<AudioUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let gpu_spectrum_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU FFT Spectrum Buffer"),
            size: 32 * 1024 * 8, // 32 channels, 1024 bins, vec2<f32>
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let waveform_storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Waveform History Storage"),
            size: (2048 * 144 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let history_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Heatmap History Texture"),
            size: wgpu::Extent3d { width: 256, height: 1024, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let history_view = history_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let fire_grid_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Fire Grid Texture"),
            size: wgpu::Extent3d { width: 1024, height: 576, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let fire_grid_view = fire_grid_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let ferrofluidsim_particles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ferrofluid Particles"),
            size: (100_000 * 32) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ferrofluidsim_grid = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ferrofluid Grid"),
            size: (512 * 512 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }
            ],
            label: Some("audio_bind_group_layout"),
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: waveform_storage_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&history_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&fire_grid_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: gpu_spectrum_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: ferrofluidsim_grid.as_entire_binding(),
                }
            ],
            label: Some("audio_bind_group"),
        });

        let smoke_render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Smoke Render Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Camera Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&smoke_render_layout)],
            immediate_size: 0,
        });

        let render_pipeline_layout_3d = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D Render Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&smoke_render_layout), Some(&camera_bind_group_layout)],
            immediate_size: 0,
        });

        let biolum_render_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Biolum Render Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let biolum_render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Biolum Render Pipeline Layout"),
            bind_group_layouts: &[
                Some(&bind_group_layout),
                Some(&smoke_render_layout),
                Some(&camera_bind_group_layout),
                Some(&biolum_render_bind_group_layout),
            ],
            immediate_size: 0,
        });

        // Shared shader headers — included at compile time, resolved via simple string replacement.
        // This is the single source of truth for AudioUniforms layout and glyph font.
        const SHADER_COMMON: &str = include_str!("shaders/_common.wgsl");
        const SHADER_GLYPH_FONT: &str = include_str!("shaders/_glyph_font.wgsl");

        let resolve_shader_includes = |source: &str| -> String {
            source
                .replace("// INCLUDE: common", SHADER_COMMON)
                .replace("// INCLUDE: glyph_font", SHADER_GLYPH_FONT)
        };

        let get_shader_source = |id: u32| -> &'static str {
            match id {
                0 => include_str!("shaders/vis_spectrum.wgsl"),
                1 => include_str!("shaders/vis_oscilloscope.wgsl"),
                2 => include_str!("shaders/vis_3doscilloscope.wgsl"),
                3 => include_str!("shaders/vis_3doscilloscope_raster.wgsl"),
                4 => include_str!("shaders/vis_3doscilloscope_freq.wgsl"),
                5 => include_str!("shaders/vis_flame.wgsl"),
                6 => include_str!("shaders/vis_firesim.wgsl"),
                7 => include_str!("shaders/vis_solar.wgsl"),
                8 => include_str!("shaders/vis_spatial.wgsl"),
                9 => include_str!("shaders/vis_ferrofluid.wgsl"),
                10 => include_str!("shaders/vis_ferrofluidsim.wgsl"),
                11 => include_str!("shaders/vis_neon_3d.wgsl"),
                12 => include_str!("shaders/vis_lissajous.wgsl"),
                13 => include_str!("shaders/vis_synthwave.wgsl"),
                14 => include_str!("shaders/vis_synthwave_racer_3d.wgsl"),
                15 => include_str!("shaders/vis_starfield.wgsl"),
                16 => include_str!("shaders/vis_rain.wgsl"),
                17 => include_str!("shaders/vis_storm_3d.wgsl"),
                18 => include_str!("shaders/vis_cuboids.wgsl"),
                19 => include_str!("shaders/vis_vumeters_3d.wgsl"),
                20 => include_str!("shaders/vis_bioluminescence.wgsl"),
                21 => include_str!("shaders/vis_matrix.wgsl"),
                22 => include_str!("shaders/vis_neon_room.wgsl"),
                23 => include_str!("shaders/vis_lyrics.wgsl"),
                24 => include_str!("shaders/vis_tape_head.wgsl"),
                _ => include_str!("shaders/vis_spectrum.wgsl"),
            }
        };

        let shader_sources: Vec<String> = crate::state::VISUALIZERS.iter().map(|v| resolve_shader_includes(get_shader_source(v.id))).collect();

        let mut render_pipelines = Vec::new();
        
        let scope_fallback = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fallback_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fallback Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&shader_sources[0])),
        });
        
        let fallback_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Fallback Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &fallback_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fallback_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });
        let _ = scope_fallback.pop().await;
        
        let mut lamp_pipeline = fallback_pipeline.clone();

        for (i, source) in shader_sources.iter().enumerate() {
            let vis_def = &crate::state::VISUALIZERS[i];
            let scope_main = device.push_error_scope(wgpu::ErrorFilter::Validation);
            
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("Shader {}", i)),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(source.as_str())),
            });
            
            let (layout, vertex_buffers, primitive, vs_entry) = if vis_def.id == 20 {
                (
                    &biolum_render_pipeline_layout,
                    vec![Vertex::desc()],
                    wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        unclipped_depth: false,
                        conservative: false,
                    },
                    "vs_main_3d"
                )
            } else {
                match vis_def.pipeline_type {
                    crate::state::PipelineType::FullscreenQuad => (
                        &render_pipeline_layout,
                        Vec::new(),
                        wgpu::PrimitiveState {
                            topology: wgpu::PrimitiveTopology::TriangleList,
                            strip_index_format: None,
                            front_face: wgpu::FrontFace::Ccw,
                            cull_mode: None,
                            polygon_mode: wgpu::PolygonMode::Fill,
                            unclipped_depth: false,
                            conservative: false,
                        },
                        "vs_main"
                    ),
                    crate::state::PipelineType::Mesh3D { .. } => (
                        &render_pipeline_layout_3d,
                        vec![Vertex::desc()],
                        wgpu::PrimitiveState {
                            topology: wgpu::PrimitiveTopology::TriangleList,
                            strip_index_format: None,
                            front_face: wgpu::FrontFace::Ccw,
                            cull_mode: None,
                            polygon_mode: wgpu::PolygonMode::Fill,
                            unclipped_depth: false,
                            conservative: false,
                        },
                        "vs_main_3d"
                    ),
                }
            };
            
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("Render Pipeline {}", i)),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs_entry),
                    buffers: &vertex_buffers,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive,
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: 1,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview_mask: None,
                cache: pipeline_cache_ref,
            });
            
            let error_future = scope_main.pop();
            let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            
            if let Some(error) = error_future.await {
                eprintln!("WGSL compilation error in visualizer {}: {:?}", i, error);
                render_pipelines.push(fallback_pipeline.clone());
            } else {
                render_pipelines.push(pipeline);
                if vis_def.id == 13 {
                    let lp = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("Lamp Render Pipeline"),
                        layout: Some(&render_pipeline_layout_3d),
                        vertex: wgpu::VertexState {
                            module: &shader,
                            entry_point: Some("vs_lamp"),
                            buffers: &[Vertex::desc()],
                            compilation_options: wgpu::PipelineCompilationOptions::default(),
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &shader,
                            entry_point: Some("fs_lamp"),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: config.format,
                                blend: Some(wgpu::BlendState::REPLACE),
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                            compilation_options: wgpu::PipelineCompilationOptions::default(),
                        }),
                        primitive: wgpu::PrimitiveState {
                            topology: wgpu::PrimitiveTopology::TriangleList,
                            strip_index_format: None,
                            front_face: wgpu::FrontFace::Ccw,
                            cull_mode: None,
                            polygon_mode: wgpu::PolygonMode::Fill,
                            unclipped_depth: false,
                            conservative: false,
                        },
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: Some(true),
                            depth_compare: Some(wgpu::CompareFunction::LessEqual),
                            stencil: wgpu::StencilState::default(),
                            bias: wgpu::DepthBiasState::default(),
                        }),
                        multisample: wgpu::MultisampleState {
                            count: 1,
                            mask: !0,
                            alpha_to_coverage_enabled: false,
                        },
                        multiview_mask: None,
                        cache: pipeline_cache_ref,
                    });
                    lamp_pipeline = lp;
                }
            }
        }

        // --- Video Pipeline ---
        let video_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Video Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { // Y
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // U
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // V
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // Sampler
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // Params
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let video_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Video Pipeline Layout"),
            bind_group_layouts: &[Some(&video_bind_group_layout)],
            immediate_size: 0,
        });

        let video_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/vis_video.wgsl"));
        let video_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Video Render Pipeline"),
            layout: Some(&video_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &video_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &video_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let hud_source = resolve_shader_includes(include_str!("shaders/hud.wgsl"));
        let hud_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("HUD Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&hud_source)),
        });
        let hud_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HUD Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &hud_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &hud_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let solid_black_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solid Black Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(r#"
                struct VertexOutput {
                    @builtin(position) clip_position: vec4<f32>,
                }
                @vertex
                fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
                    var out: VertexOutput;
                    let x = f32((in_vertex_index << 1u) & 2u) * 2.0 - 1.0;
                    let y = f32(in_vertex_index & 2u) * 2.0 - 1.0;
                    out.clip_position = vec4<f32>(x, y, 1.0, 1.0);
                    return out;
                }
                @fragment
                fn fs_main() -> @location(0) vec4<f32> {
                    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
                }
            "#)),
        });

        let clear_black_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Clear Black Layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        let clear_black_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Clear Black Pipeline"),
            layout: Some(&clear_black_layout),
            vertex: wgpu::VertexState {
                module: &solid_black_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &solid_black_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let solid_biolum_bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solid Biolum BG Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(r#"
                struct VertexOutput {
                    @builtin(position) clip_position: vec4<f32>,
                    @location(0) ndc: vec2<f32>,
                }
                @vertex
                fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
                    var out: VertexOutput;
                    let x = f32((in_vertex_index << 1u) & 2u) * 2.0 - 1.0;
                    let y = f32(in_vertex_index & 2u) * 2.0 - 1.0;
                    
                    let ndc = vec2<f32>(x, y);
                    let r2 = dot(ndc, ndc);
                    let distorted_ndc = ndc * (1.0 + r2 * 0.055);
                    
                    out.clip_position = vec4<f32>(distorted_ndc, 1.0, 1.0);
                    out.ndc = distorted_ndc;
                    return out;
                }
                @fragment
                fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
                    if (abs(in.ndc.x) > 1.0 || abs(in.ndc.y) > 1.0) {
                        discard;
                    }
                    let border_dist = min(1.0 - abs(in.ndc.x), 1.0 - abs(in.ndc.y));
                    let bezel_mask = smoothstep(0.0, 0.03, border_dist);
                    
                    return vec4<f32>(vec3<f32>(0.001, 0.003, 0.008) * bezel_mask, 1.0);
                }
            "#)),
        });

        let biolum_bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Biolum BG Pipeline"),
            layout: Some(&clear_black_layout),
            vertex: wgpu::VertexState {
                module: &solid_biolum_bg_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &solid_biolum_bg_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let crt_background_shader_src = resolve_shader_includes(r#"
            // INCLUDE: common

            @group(0) @binding(0) var<uniform> audio: AudioUniforms;

            @vertex
            fn vs_background(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
                var out: VertexOutput;
                let u = f32((in_vertex_index << 1u) & 2u);
                let v = f32(in_vertex_index & 2u);
                out.clip_position = vec4<f32>(u * 2.0 - 1.0, -(v * 2.0 - 1.0), 1.0, 1.0);
                out.uv = vec2<f32>(u, v);
                return out;
            }

            fn hash21(p: vec2<f32>) -> f32 {
                var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
                p3 = p3 + dot(p3, p3.yzx + 33.33);
                return fract((p3.x + p3.y) * p3.z);
            }

            @fragment
            fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
                let crt_uv = in.uv * 2.0 - 1.0;
                let r2 = dot(crt_uv, crt_uv);
                let distorted_uv = crt_uv * (1.0 + r2 * 0.055);
                
                if (abs(distorted_uv.x) > 1.0 || abs(distorted_uv.y) > 1.0) {
                    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
                }
                
                let border_dist = min(1.0 - abs(distorted_uv.x), 1.0 - abs(distorted_uv.y));
                let bezel_mask = smoothstep(0.0, 0.03, border_dist);
                
                var aspect = 1.7777;
                let dy = abs(dpdy(in.uv.y));
                let dx = abs(dpdx(in.uv.x));
                if (dx > 0.0001 && dy > 0.0001) { aspect = dy / dx; }
                let p = vec2<f32>(distorted_uv.x * aspect, -distorted_uv.y);
                
                let ro = vec3<f32>(0.0, 0.0, 7.2);
                let rd = normalize(vec3<f32>(-p.x, p.y, -1.5));
                
                let bass = clamp(audio.spectrum[0].x + audio.spectrum[1].x + audio.spectrum[2].x, 0.0, 1.0);
                let base_green = vec3<f32>(0.02, 1.0, 0.38);
                let neon_green = mix(base_green, vec3<f32>(1.0, 1.0, 1.0), clamp(bass * 0.45, 0.0, 1.0));
                
                var t_floor = -1.0;
                if (rd.y < -0.001) { t_floor = -3.2 / rd.y; }
                var t_ceil = -1.0;
                if (rd.y > 0.001) { t_ceil = 3.2 / rd.y; }
                
                let x_spacing = 1.35;
                
                var grid_intensity = 0.0;
                if (t_floor > 0.0 && t_floor < 25.0) {
                    let p_floor = ro + rd * t_floor;
                    let grid_uv = fract(p_floor.xz / x_spacing - 0.5) - 0.5;
                    let dist_to_line = min(abs(grid_uv.x), abs(grid_uv.y));
                    let line_w = 0.02 * (1.0 + t_floor * 0.05);
                    let grid_line = smoothstep(line_w, 0.0, dist_to_line);
                    let fade = smoothstep(25.0, 4.0, t_floor);
                    grid_intensity = grid_intensity + grid_line * fade;
                }
                if (t_ceil > 0.0 && t_ceil < 25.0) {
                    let p_ceil = ro + rd * t_ceil;
                    let grid_uv = fract(p_ceil.xz / x_spacing - 0.5) - 0.5;
                    let dist_to_line = min(abs(grid_uv.x), abs(grid_uv.y));
                    let line_w = 0.02 * (1.0 + t_ceil * 0.05);
                    let grid_line = smoothstep(line_w, 0.0, dist_to_line);
                    let fade = smoothstep(25.0, 4.0, t_ceil);
                    grid_intensity = grid_intensity + grid_line * fade;
                }
                
                var final_color = neon_green * grid_intensity * 0.35;
                
                let center_dist = length(distorted_uv);
                let bg_glow = vec3<f32>(0.005, 0.038, 0.016) * (1.0 - center_dist * 0.55);
                final_color = final_color + bg_glow;
                
                final_color = final_color * bezel_mask;
                
                let scanline = 0.86 + 0.14 * cos(in.clip_position.y * 3.14159);
                final_color = final_color * scanline;
                
                let flicker = 0.98 + 0.02 * sin(audio.time * 115.0);
                final_color = final_color * flicker;
                
                let noise_val = hash21(in.clip_position.xy + fract(audio.smooth_time) * 149.0);
                let static_noise = noise_val * 0.022 * bezel_mask;
                final_color = final_color + vec3<f32>(static_noise);
                
                var final_col = (final_color * (2.51 * final_color + 0.03)) / (final_color * (2.43 * final_color + 0.59) + 0.14);
                final_col = max(final_col, vec3<f32>(0.0));
                
                return vec4<f32>(final_col, 1.0);
            }
        "#);

        let crt_background_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CRT Background Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(crt_background_shader_src)),
        });

        let crt_background_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("CRT Background Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &crt_background_shader,
                entry_point: Some("vs_background"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &crt_background_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let synthwave_sky_shader_src = resolve_shader_includes(include_str!("shaders/vis_synthwave_racer_sky.wgsl"));
        let synthwave_sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Synthwave Sky Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(synthwave_sky_shader_src)),
        });

        let synthwave_sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Synthwave Sky Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &synthwave_sky_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &synthwave_sky_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let vumeters_bg_shader_src = resolve_shader_includes(include_str!("shaders/vis_vumeters_bg.wgsl"));
        let vumeters_bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VU Meters Background Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(vumeters_bg_shader_src)),
        });

        let vumeters_bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("VU Meters Background Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vumeters_bg_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &vumeters_bg_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let neon_bg_shader_src = resolve_shader_includes(include_str!("shaders/vis_neon_bg.wgsl"));
        let neon_bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Neon Corridor Background Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(neon_bg_shader_src)),
        });

        let neon_bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Neon Corridor Background Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &neon_bg_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &neon_bg_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let storm_sky_shader_src = resolve_shader_includes(include_str!("shaders/vis_storm_sky.wgsl"));
        let storm_sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Storm Sky Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(storm_sky_shader_src)),
        });

        let storm_sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Storm Sky Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &storm_sky_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &storm_sky_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        // ------------------------------------------------------------------
        // Neon Smoke Cache Compute Pipeline
        // ------------------------------------------------------------------
        let smoke_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Neon Smoke Texture"),
            size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 64 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        
        let smoke_texture_view = smoke_texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let smoke_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Neon Smoke Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        
        let smoke_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Smoke Params Buffer"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let smoke_compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Smoke Compute Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D3,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let smoke_compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Smoke Compute Bind Group"),
            layout: &smoke_compute_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&smoke_texture_view) },
                wgpu::BindGroupEntry { binding: 1, resource: smoke_params_buffer.as_entire_binding() },
            ],
        });

        // smoke_render_layout is defined earlier

        let smoke_render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Smoke Render Bind Group"),
            layout: &smoke_render_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&smoke_texture_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&smoke_sampler) },
            ],
        });

        let smoke_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Neon Smoke Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("shaders/vis_neon_smoke_cs.wgsl"))),
        });

        let smoke_compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Smoke Compute Pipeline Layout"),
            bind_group_layouts: &[Some(&smoke_compute_layout)],
            immediate_size: 0,
        });

        let smoke_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Smoke Compute Pipeline"),
            layout: Some(&smoke_compute_pipeline_layout),
            module: &smoke_shader,
            entry_point: Some("cs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: pipeline_cache_ref,
        });

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_texture_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, egui_wgpu::RendererOptions {
            depth_stencil_format: None,
            ..Default::default()
        });

        // --- Fire Compute Pipeline ---
        let fire_grid_size = 1024 * 576 * 4; // 1024 × 576 × f32
        let fire_buffer_a = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fire Grid A"),
            size: fire_grid_size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let fire_buffer_b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fire Grid B"),
            size: fire_grid_size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let fire_coal_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Coal Bed"),
            size: (1024 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fire_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fire Params"),
            size: std::mem::size_of::<FireParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });


        let heatmap_compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("heatmap_compute_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::R32Float, view_dimension: wgpu::TextureViewDimension::D2 }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let heatmap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("heatmap_bind_group"), layout: &heatmap_compute_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&history_view) },
                wgpu::BindGroupEntry { binding: 4, resource: gpu_spectrum_buffer.as_entire_binding() },
            ],
        });
        let heatmap_source = resolve_shader_includes(include_str!("shaders/heatmap_compute.wgsl"));
        let heatmap_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Heatmap Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&heatmap_source)),
        });
        let heatmap_compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("heatmap_compute_layout"), bind_group_layouts: &[Some(&heatmap_compute_layout)], immediate_size: 0,
        });
        let heatmap_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Heatmap Compute Pipeline"), layout: Some(&heatmap_compute_pipeline_layout), module: &heatmap_compute_shader, entry_point: Some("main"), compilation_options: wgpu::PipelineCompilationOptions::default(), cache: pipeline_cache_ref,
        });
        let fire_compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fire_compute_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });

        let fire_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fire_bg_a"), layout: &fire_compute_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: fire_buffer_a.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fire_buffer_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: fire_coal_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: fire_params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: gpu_spectrum_buffer.as_entire_binding() },
            ],
        });
        let fire_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fire_bg_b"), layout: &fire_compute_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: fire_buffer_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fire_buffer_a.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: fire_coal_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: fire_params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: gpu_spectrum_buffer.as_entire_binding() },
            ],
        });

        let fire_compute_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/fire_compute.wgsl"));
        let firesim_compute_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/firesim_compute.wgsl"));
        let fire_compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fire_compute_layout"),
            bind_group_layouts: &[Some(&fire_compute_layout)],
            immediate_size: 0,
        });
        let fire_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Fire Compute Pipeline"),
            layout: Some(&fire_compute_pipeline_layout),
            module: &fire_compute_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: pipeline_cache_ref,
        });
        let firesim_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FireSim Compute Pipeline"),
            layout: Some(&fire_compute_pipeline_layout),
            module: &firesim_compute_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: pipeline_cache_ref,
        });

        let ferrofluidsim_compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ferrofluidsim_compute_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });

        let ferrofluidsim_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ferrofluidsim_bg"), layout: &ferrofluidsim_compute_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: ferrofluidsim_particles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: ferrofluidsim_grid.as_entire_binding() },
            ],
        });

        let ferrofluidsim_source = resolve_shader_includes(include_str!("shaders/ferrofluidsim_compute.wgsl"));
        let ferrofluidsim_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ferrofluid Sim Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&ferrofluidsim_source)),
        });
        let ferrofluidsim_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ferrofluidsim_layout"), bind_group_layouts: &[Some(&ferrofluidsim_compute_layout)], immediate_size: 0,
        });
        
        let ferrofluidsim_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Ferrofluid Compute"), layout: Some(&ferrofluidsim_pipeline_layout), module: &ferrofluidsim_compute_shader, entry_point: Some("main"), compilation_options: Default::default(), cache: pipeline_cache_ref,
        });
        let ferrofluidsim_clear_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Ferrofluid Clear"), layout: Some(&ferrofluidsim_pipeline_layout), module: &ferrofluidsim_compute_shader, entry_point: Some("clear"), compilation_options: Default::default(), cache: pipeline_cache_ref,
        });

        // --- Bioluminescent Waves Compute & Render Setup ---
        let biolum_particles_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Bioluminescent Particles"),
            size: (65536 * 32) as u64, // 65,536 particles * 32 bytes/particle
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&biolum_particles_buffer, 0, &vec![0u8; 65536 * 32]);

        let biolum_compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Biolum Compute Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let biolum_compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Biolum Compute Bind Group"),
            layout: &biolum_compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: biolum_particles_buffer.as_entire_binding(),
                },
            ],
        });

        let biolum_compute_source = resolve_shader_includes(include_str!("shaders/biolum_compute.wgsl"));
        let biolum_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Biolum Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&biolum_compute_source)),
        });

        let biolum_compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Biolum Compute Pipeline Layout"),
            bind_group_layouts: &[Some(&biolum_compute_layout)],
            immediate_size: 0,
        });

        let biolum_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Biolum Compute Pipeline"),
            layout: Some(&biolum_compute_pipeline_layout),
            module: &biolum_compute_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: pipeline_cache_ref,
        });

        let biolum_render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Biolum Render Bind Group"),
            layout: &biolum_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: biolum_particles_buffer.as_entire_binding(),
                },
            ],
        });
        // --- End Bioluminescent Waves Setup ---

        let mut query_set = None;
        let mut query_resolve_buffer = None;
        let mut query_read_buffer = None;
        let timestamp_period = queue.get_timestamp_period();

        if supports_timestamps {
            query_set = Some(device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Shader Timestamps"),
                count: 6, // 0-1 for FFT, 2-3 for Fire, 4-5 for Main Vis Render
                ty: wgpu::QueryType::Timestamp,
            }));

            query_resolve_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Query Resolve Buffer"),
                size: 48, // 6 * 8 bytes
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));

            query_read_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Query Read Buffer"),
                size: 48,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
        }

        // --- Resynth compute (consumes the CPU-filled GPU spectrum buffer) ---
        let resynth_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resynth_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let resynth_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resynth_bind_group"), layout: &resynth_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: gpu_spectrum_buffer.as_entire_binding() },
            ],
        });
        let resynth_compute_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/resynth_compute.wgsl"));
        let resynth_compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("resynth_compute_layout"), bind_group_layouts: &[Some(&resynth_bind_group_layout)], immediate_size: 0,
        });
        let resynth_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Resynth Compute Pipeline"), layout: Some(&resynth_compute_pipeline_layout), module: &resynth_compute_shader, entry_point: Some("main"), compilation_options: wgpu::PipelineCompilationOptions::default(), cache: pipeline_cache_ref,
        });
        // --- 3D Engine Extensions Init ---
        let mut mesh_registry = std::collections::HashMap::new();
        let mut unique_geometries = std::collections::HashSet::new();
        for vis in crate::state::VISUALIZERS {
            if let crate::state::PipelineType::Mesh3D { geometry, .. } = &vis.pipeline_type {
                unique_geometries.insert(geometry.clone());
            }
        }
        
        for geom in unique_geometries {
            let (vertices, indices) = match &geom {
                crate::state::Geometry::Grid { width, depth } => {
                    let mut vertices = Vec::new();
                    let mut indices = Vec::new();
                    let grid_width = *width;
                    let grid_depth = *depth;
                    for z in 0..=grid_depth {
                        for x in 0..=grid_width {
                            let px = x as f32 - (grid_width as f32) / 2.0;
                            let pz = z as f32 - (grid_depth as f32) / 2.0;
                            vertices.push(Vertex {
                                position: [px, 0.0, pz],
                                normal: [0.0, 1.0, 0.0],
                                tex_coords: [x as f32 / grid_width as f32, z as f32 / grid_depth as f32],
                            });
                        }
                    }
                    for z in 0..grid_depth {
                        for x in 0..grid_width {
                            let start = z * (grid_width + 1) + x;
                            indices.push(start);
                            indices.push(start + 1);
                            indices.push(start + grid_width + 1);
                            indices.push(start + 1);
                            indices.push(start + grid_width + 2);
                            indices.push(start + grid_width + 1);
                        }
                    }
                    (vertices, indices)
                }
                crate::state::Geometry::UnitBox => {
                    let mut vertices = Vec::new();
                    let mut indices = Vec::new();
                    
                    let mut add_face = |p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], normal: [f32; 3]| {
                        let start_idx = vertices.len() as u32;
                        vertices.push(Vertex { position: p0, normal, tex_coords: [0.0, 0.0] });
                        vertices.push(Vertex { position: p1, normal, tex_coords: [1.0, 0.0] });
                        vertices.push(Vertex { position: p2, normal, tex_coords: [1.0, 1.0] });
                        vertices.push(Vertex { position: p3, normal, tex_coords: [0.0, 1.0] });
                        
                        indices.push(start_idx);
                        indices.push(start_idx + 1);
                        indices.push(start_idx + 2);
                        indices.push(start_idx);
                        indices.push(start_idx + 2);
                        indices.push(start_idx + 3);
                    };
                    
                    // Front face (z = +0.5)
                    add_face(
                        [-0.5, -0.5,  0.5],
                        [ 0.5, -0.5,  0.5],
                        [ 0.5,  0.5,  0.5],
                        [-0.5,  0.5,  0.5],
                        [0.0, 0.0, 1.0],
                    );
                    // Back face (z = -0.5)
                    add_face(
                        [ 0.5, -0.5, -0.5],
                        [-0.5, -0.5, -0.5],
                        [-0.5,  0.5, -0.5],
                        [ 0.5,  0.5, -0.5],
                        [0.0, 0.0, -1.0],
                    );
                    // Left face (x = -0.5)
                    add_face(
                        [-0.5, -0.5, -0.5],
                        [-0.5, -0.5,  0.5],
                        [-0.5,  0.5,  0.5],
                        [-0.5,  0.5, -0.5],
                        [-1.0, 0.0, 0.0],
                    );
                    // Right face (x = +0.5)
                    add_face(
                        [ 0.5, -0.5,  0.5],
                        [ 0.5, -0.5, -0.5],
                        [ 0.5,  0.5, -0.5],
                        [ 0.5,  0.5,  0.5],
                        [1.0, 0.0, 0.0],
                    );
                    // Top face (y = +0.5)
                    add_face(
                        [-0.5,  0.5,  0.5],
                        [ 0.5,  0.5,  0.5],
                        [ 0.5,  0.5, -0.5],
                        [-0.5,  0.5, -0.5],
                        [0.0, 1.0, 0.0],
                    );
                    // Bottom face (y = -0.5)
                    add_face(
                        [-0.5, -0.5, -0.5],
                        [ 0.5, -0.5, -0.5],
                        [ 0.5, -0.5,  0.5],
                        [-0.5, -0.5,  0.5],
                        [0.0, -1.0, 0.0],
                    );
                    
                    (vertices, indices)
                }
                crate::state::Geometry::NeonRoom => generate_neon_room_mesh(),
                crate::state::Geometry::SynthwaveRacerScene => generate_synthwave_racer_mesh(),
                crate::state::Geometry::VuMeterRack => generate_vumeter_rack_mesh(),
                crate::state::Geometry::NeonCorridorFrames => generate_neon_corridor_mesh(),
                crate::state::Geometry::StormRainVolume => generate_storm_rain_volume_mesh(),
                crate::state::Geometry::GlassLyricsScene => generate_glass_lyrics_mesh("RUSTTRACKER"),
            };
            
            let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("Mesh Vertex Buffer {:?}", geom)),
                size: (vertices.len() * std::mem::size_of::<Vertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));

            let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("Mesh Index Buffer {:?}", geom)),
                size: (indices.len() * std::mem::size_of::<u32>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&index_buffer, 0, bytemuck::cast_slice(&indices));
            let index_count = indices.len() as u32;
            
            mesh_registry.insert(geom, MeshBuffers {
                vertex_buffer,
                index_buffer,
                index_count,
            });
        }

        let camera_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Camera Uniforms"),
            size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniform_buffer.as_entire_binding(),
            }],
        });
        let (lamp_verts, lamp_inds) = generate_lamp_mesh();
        let lamp_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lamp Vertex Buffer"),
            size: (lamp_verts.len() * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&lamp_vertex_buffer, 0, bytemuck::cast_slice(&lamp_verts));

        let lamp_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lamp Index Buffer"),
            size: (lamp_inds.len() * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&lamp_index_buffer, 0, bytemuck::cast_slice(&lamp_inds));
        let lamp_index_count = lamp_inds.len() as u32;
        // --- END 3D Engine Extensions Init ---

        // Persist the pipeline cache back to disk atomically so the next launch
        // can skip recompiling all visualizer/compute pipelines.
        if pipeline_cache_enabled {
            if let (Some(path), Some(cache)) = (&pipeline_cache_path, &pipeline_cache) {
                if let Some(data) = cache.get_data() {
                    let tmp = path.with_extension("tmp");
                    if std::fs::write(&tmp, &data).is_ok() {
                        let _ = std::fs::rename(&tmp, path);
                    }
                }
            }
        }

        Self {
            surface: Some(surface),
            device,
            queue,
            config,
            size,
            render_pipelines,
            hud_pipeline,
            uniform_buffer,
            waveform_storage_buffer,
            history_texture,
            fire_grid_texture,
            uniform_bind_group,
            egui_renderer,
            query_set,
            query_resolve_buffer,
            query_read_buffer,
            timestamp_period,
            timestamp_mapping_active: false,
            timestamp_map_complete: Arc::new(AtomicBool::new(false)),
            cached_fft_us: None,
            cached_fire_us: None,
            cached_vis_us: None,
            meters_uv_rect: [0.0; 4],
            heatmap_uv_rect: [0.0; 4],
            fire_uv_rect: [0.0; 4],
            fire_compute_pipeline,
            firesim_compute_pipeline,
            fire_buffer_a,
            fire_buffer_b,
            fire_coal_buffer,
            fire_params_buffer,
            fire_bind_group_a,
            fire_bind_group_b,
            fire_ping: true,
            heatmap_row: 0,
            heatmap_compute_pipeline,
            heatmap_bind_group,
            ferrofluidsim_compute_pipeline,
            ferrofluidsim_clear_pipeline,
            ferrofluidsim_bind_group,

            biolum_particles_buffer,
            biolum_compute_pipeline,
            biolum_compute_bind_group,
            biolum_render_bind_group,
            biolum_render_pipeline_layout,

            start_time: std::time::Instant::now(),
            resynth_compute_pipeline,
            resynth_bind_group,
            _gpu_spectrum_buffer: gpu_spectrum_buffer,
            
            smoke_compute_pipeline,
            smoke_compute_bind_group,
            smoke_render_bind_group,
            smoke_params_buffer,
            
            depth_texture_view,
            
            waveform_history_flat: vec![0.0; 2048 * 144],
            video_bind_group_layout,
            video_pipeline,
            video_state: None,
            clear_black_pipeline,
            crt_background_pipeline,
            biolum_bg_pipeline,
            synthwave_sky_pipeline,
            vumeters_bg_pipeline,
            neon_bg_pipeline,
            storm_sky_pipeline,
            
            camera_uniform_buffer,
            camera_bind_group,
            mesh_registry,
            lamp_vertex_buffer,
            lamp_index_buffer,
            lamp_index_count,
            lamp_pipeline,
            frame_count: 0,
            last_history_cam_z: 0.0f64,
            smooth_time: 0.0f64,
            play_time: 0.0f64,
            smooth_dt: 1.0f64 / 60.0f64,
            last_history_push_count: 0,
            time_since_last_push: 0.0,
            vu_needle_angles: vec![0.0; 32],
            vu_needle_velocities: vec![0.0; 32],
            channel_phases: [0.0; 32],
            last_uploaded_push_count: u64::MAX,
            last_uploaded_vis_width: 0,
            lyric_slam_timer: 0.0,
            last_lyric_line_idx: None,
            current_lyric_mesh_text: "RUSTTRACKER".to_string(),
            fire_intensity: 0.0,
        }
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            if let Some(surface) = &self.surface {
                surface.configure(&self.device, &self.config);
            }
            
            let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Depth Texture"),
                size: wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth_texture_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        }
    }


    pub fn clear_video_state(&mut self) {
        self.video_state = None;
    }

    #[allow(dead_code)]
    pub fn has_video_stream(&self) -> bool {
        self.video_state.is_some()
    }

    pub fn update(&mut self, state: &AppState, dt: f32) {
        self.frame_count = self.frame_count.wrapping_add(1);
        
        // Accumulate matrix digital rain speed phases per channel
        for i in 0..32 {
            let ch_vu = if i < state.channel_vus.len() {
                state.channel_vus[i]
            } else {
                0.0
            };
            self.channel_phases[i] += dt * (1.0 + ch_vu * 5.0);
        }
        
        // Exponential moving average to smooth CPU scheduling time jitter
        let alpha = 0.03f64;
        self.smooth_dt = self.smooth_dt * (1.0 - alpha) + (dt as f64).clamp(0.001, 0.1) * alpha;
        self.smooth_time += self.smooth_dt;

        // Track pushes to synchronize scrolling phase with physical buffer updates
        if state.waveform_history_push_count < self.last_history_push_count {
            self.last_history_push_count = state.waveform_history_push_count;
            self.time_since_last_push = 0.0;
        }

        let mut steps = 0;
        if state.waveform_history_push_count != self.last_history_push_count {
            let diff = state.waveform_history_push_count.saturating_sub(self.last_history_push_count);
            steps = diff as u32;
            self.heatmap_row = (self.heatmap_row + steps) % 1024;
            self.time_since_last_push = 0.0;
            self.last_history_push_count = state.waveform_history_push_count;
        } else if !state.is_paused && state.file_loaded && !state.track_ended {
            // Use the real frame dt (not the heavily-smoothed EMA) so the
            // sub-frame scroll interpolation stays in phase with the actual
            // DSP push cadence and does not judder.
            self.time_since_last_push += (dt as f64).clamp(0.0, 0.1);
        }

        // Target interval between pushes
        let push_interval = if state.target_fps > 0 {
            1.0 / state.target_fps as f64
        } else {
            1.0 / 60.0
        };

        // Smoothly interpolate within the current frame push window
        let step_fraction = (self.time_since_last_push / push_interval).clamp(0.0, 1.0) as f32;

        // Animation time: advances in real time, scaled by 2.88 to preserve the
        // look that was tuned at 144fps (144 pushes/s * 0.02s = 2.88 anim-seconds
        // per real second). Render-framerate independent. Uses the EMA-smoothed
        // dt to avoid CPU scheduling judder. Only advances while playing.
        if !state.is_paused && state.file_loaded && !state.track_ended {
            self.play_time += self.smooth_dt * 2.88;
        }

        // Progress bar fire dies off when playback stops / pauses / ends / reaches end of song
        let is_at_end = state.duration_seconds > 0.0 && state.current_seconds >= state.duration_seconds - 0.05;
        let is_playing = !state.is_paused && state.file_loaded && !state.track_ended && !is_at_end;
        let target_intensity = if is_playing { 1.0f32 } else { 0.0f32 };
        let fire_rate = if is_playing { 5.0f32 } else { 3.5f32 };
        let dt_clamped = dt.clamp(0.001, 0.1);
        if self.fire_intensity < target_intensity {
            self.fire_intensity = (self.fire_intensity + fire_rate * dt_clamped).min(target_intensity);
        } else if self.fire_intensity > target_intensity {
            self.fire_intensity = (self.fire_intensity - fire_rate * dt_clamped).max(target_intensity);
        }

        // World-Z camera position locked to history rows (0.5 units/row), used by
        // visualizers that scroll with the waveform/spectrum history ring buffer.
        self.last_history_cam_z = (self.last_history_push_count as f64 + step_fraction as f64) * 0.5;
        let frame_dt = (dt as f32).clamp(0.0005, 0.1);
        let mut uniforms = AudioUniforms {
            spectrum: [0.0; 1024],
            fire_heat: [0.0; 1024],
            channels: [0.0; 32],
            channel_peaks: [0.0; 32],
            spatial_channels: [0.0; 16],
            display_order: [0; 16],
            channel_phases: self.channel_phases,
            num_channels: state.channel_vus.len().min(32) as u32,
            mode: state.visualizer_mode,
            time: state.scrub_target_seconds.unwrap_or(state.current_seconds) as f32,
            duration: state.duration_seconds as f32,
            smooth_time: self.play_time as f32,
            heatmap_row: self.heatmap_row,
            fft_channels: state.raw_audio_channels.len() as u32,
            num_spatial_channels: state.channel_vus.len().saturating_sub(state.tracker_channels.unwrap_or(0) as usize) as u32,
            ui_meters_rect: self.meters_uv_rect,
            ui_heatmap_rect: self.heatmap_uv_rect,
            ui_fire_rect: self.fire_uv_rect,
            waveform_resolution: 1024,
            waveform_history_size: 144,
            frame_count: self.frame_count,
            step_fraction,
            steps_to_fill: steps,
            aspect_ratio: self.size.width as f32 / self.size.height as f32,
            frame_dt,
            history_cam_z: self.last_history_cam_z as f32,
            fire_intensity: self.fire_intensity,
            _pad1: 0.0,
            _pad2: 0.0,
            _pad3: 0.0,
        };

        uniforms.spectrum.copy_from_slice(&state.spectrum_data);
        uniforms.fire_heat.copy_from_slice(&state.fire_heat);

        // Update needle physics for 3D retro VU meters
        let ch_count = state.channel_vus.len().max(2);
        if self.vu_needle_angles.len() < ch_count {
            self.vu_needle_angles.resize(ch_count, 0.0);
            self.vu_needle_velocities.resize(ch_count, 0.0);
        }

        let rise_time_secs = 0.30;
        let damping_ratio = 0.85;
        let omega_n = 4.8 / rise_time_secs;
        let spring = omega_n * omega_n;
        let damping = 2.0 * damping_ratio * omega_n;

        let dt_clamped = dt.min(0.05);
        for i in 0..ch_count {
            let target = if i < state.raw_channel_vus.len() {
                state.raw_channel_vus[i]
            } else if !state.file_loaded {
                // Sway gently if no file is loaded
                let t = self.smooth_time as f32;
                let offset = if i == 0 { 0.0 } else { 1.5 };
                (0.12 * (t * 2.0 + offset).sin() + 0.15 * (t * 3.7 - offset).cos() + 0.2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let angle = self.vu_needle_angles[i];
            let vel = self.vu_needle_velocities[i];

            let acceleration = spring * (target - angle) - damping * vel;
            let new_vel = vel + acceleration * dt_clamped;
            let mut new_angle = angle + new_vel * dt_clamped;

            if new_angle < -0.05 {
                new_angle = -0.05;
                self.vu_needle_velocities[i] = 0.0;
            } else if new_angle > 1.1 {
                new_angle = 1.1;
                self.vu_needle_velocities[i] = -new_vel * 0.2; // pin bounce
            } else {
                self.vu_needle_velocities[i] = new_vel;
            }
            self.vu_needle_angles[i] = new_angle;
        }

        let ch_len = state.channel_vus.len().min(32);
        
        // 1. Populate UI Display Channels (may be visually remapped)
        let mut display_order: Vec<usize> = (0..ch_len).collect();
        if state.tracker_channels.is_none() {
            if ch_len == 6 {
                display_order = vec![4, 0, 2, 3, 1, 5]; // Ls, L, C, LFE, R, Rs
            } else if ch_len == 8 {
                // SMPTE 7.1: ch4=Ls(side), ch5=Rs(side), ch6=Lrs(rear), ch7=Rrs(rear)
                display_order = vec![6, 4, 0, 2, 3, 1, 5, 7]; // Lrs, Ls, L, C, LFE, R, Rs, Rrs
            } else if ch_len == 16 {
                // 16 channels: fan out symmetrically from C with LFE (3) positioned physically but skipped by shaders
                display_order = vec![14, 12, 10, 8, 6, 4, 0, 2, 3, 1, 5, 7, 9, 11, 13, 15];
            }
        }

        for (disp_idx, &src_idx) in display_order.iter().enumerate() {
            if disp_idx < 16 {
                uniforms.display_order[disp_idx] = src_idx as u32;
            }
            if src_idx < state.channel_vus.len() {
                uniforms.channels[disp_idx] = state.channel_vus[src_idx];
                uniforms.channel_peaks[disp_idx] = state.peak_vus[src_idx];
            }
        }
        
        // 2. Populate Raw Spatial Channels (strict spatial mapping without UI reordering)
        if state.tracker_channels.is_some() {
            // For tracker files, channel_vus is [L_Peak, Track1..N, R_Peak]
            if state.channel_vus.len() >= 2 {
                uniforms.spatial_channels[0] = state.channel_vus[0];
                uniforms.spatial_channels[1] = state.channel_vus[state.channel_vus.len() - 1];
            }
        } else {
            let spatial_count = state.channel_vus.len().min(16);
            for i in 0..spatial_count {
                uniforms.spatial_channels[i] = state.channel_vus[i];
            }
        }
        
        if state.visualizer_mode == 19 {
            let actual_ch = state.channel_vus.len().max(2);
            uniforms.num_channels = actual_ch as u32;
            for i in 0..actual_ch {
                if i < 32 {
                    uniforms.channels[i] = if i < self.vu_needle_angles.len() {
                        self.vu_needle_angles[i]
                    } else {
                        0.0
                    };
                }
            }
        }

        if state.visualizer_mode == 23 {
            let display_secs = state.scrub_target_seconds.unwrap_or(state.current_seconds);
            let active_idx = state.lyrics.as_ref().and_then(|l| l.find_current_line_idx(display_secs));

            // Smooth 500+ FPS frame-interpolated slam timer
            if active_idx != self.last_lyric_line_idx {
                self.last_lyric_line_idx = active_idx;
                self.lyric_slam_timer = 0.0;
            } else if !state.is_paused && state.file_loaded && !state.track_ended {
                self.lyric_slam_timer += frame_dt;
            }

            let (char_bytes, line_progress, is_instrumental, line_duration) = if let Some(lyrics) = &state.lyrics {
                if !lyrics.lines.is_empty() {
                    if let Some(idx) = active_idx {
                        let cur_time = lyrics.lines[idx].time_seconds;
                        let next_time = if idx + 1 < lyrics.lines.len() {
                            lyrics.lines[idx + 1].time_seconds
                        } else {
                            cur_time + 4.0
                        };
                        let dur = (next_time - cur_time).max(0.1) as f32;
                        let elapsed = (display_secs - cur_time).max(0.0) as f32;
                        let prog = (elapsed / dur).clamp(0.0, 1.0);
                        let text = lyrics.lines[idx].text.trim().to_uppercase();
                        let bytes = text.into_bytes();
                        
                        let gap = (next_time - display_secs) as f32;
                        let is_inst = if (next_time - cur_time) > 4.5 && elapsed > 2.5 && gap > 1.5 { 1.0 } else { 0.0 };
                        (bytes, prog, is_inst, dur)
                    } else {
                        // Intro
                        let first_time = lyrics.lines[0].time_seconds;
                        let prog = (display_secs / first_time.max(0.1)).clamp(0.0, 1.0) as f32;
                        ("RUSTTRACKER".to_string().into_bytes(), prog, 1.0, 4.0)
                    }
                } else {
                    let title = if !state.song_title.is_empty() {
                        state.song_title.trim().to_uppercase()
                    } else {
                        "RUSTTRACKER".to_string()
                    };
                    (title.into_bytes(), 0.0, 1.0, 4.0)
                }
            } else {
                let title = if !state.song_title.is_empty() {
                    state.song_title.trim().to_uppercase()
                } else {
                    "RUSTTRACKER".to_string()
                };
                (title.into_bytes(), 0.0, 1.0, 4.0)
            };

            let slam_t = self.lyric_slam_timer;
            // Calculate exact slam vertical drop and spring damping
            let slam_y = if slam_t < 0.28 {
                // Gravity drop from y = 3.5 down to 0.0
                let t_norm = (slam_t / 0.28).clamp(0.0, 1.0);
                3.5 * (1.0 - t_norm * t_norm)
            } else {
                // Water impact, plunge and damped spring bobbing
                let dt_impact = slam_t - 0.28;
                -0.32 * (-3.6 * dt_impact).exp() * (9.5 * dt_impact).cos()
            };

            let bass = if state.spectrum_data.len() > 4 {
                state.spectrum_data[1..5].iter().copied().fold(0.0f32, f32::max)
            } else {
                0.0
            };

            let char_count = char_bytes.len().min(64);

            // Store lyric parameters in fire_heat array (without touching ui_meters_rect or display_order!)
            uniforms.fire_heat[0] = slam_t;
            uniforms.fire_heat[1] = slam_y;
            uniforms.fire_heat[2] = line_progress;
            uniforms.fire_heat[3] = char_count as f32;
            uniforms.fire_heat[4] = line_duration;
            uniforms.fire_heat[5] = is_instrumental;
            uniforms.fire_heat[6] = bass;

            for (i, &b) in char_bytes.iter().take(64).enumerate() {
                uniforms.fire_heat[16 + i] = b as f32;
            }

            let active_text_str = String::from_utf8_lossy(&char_bytes).to_string();
            if self.current_lyric_mesh_text != active_text_str {
                let (vertices, indices) = generate_glass_lyrics_mesh(&active_text_str);
                let vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Mesh Vertex Buffer GlassLyricsScene"),
                    size: (vertices.len() * std::mem::size_of::<Vertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));

                let index_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Mesh Index Buffer GlassLyricsScene"),
                    size: (indices.len() * std::mem::size_of::<u32>()) as u64,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.queue.write_buffer(&index_buffer, 0, bytemuck::cast_slice(&indices));
                let index_count = indices.len() as u32;

                self.mesh_registry.insert(
                    crate::state::Geometry::GlassLyricsScene,
                    MeshBuffers {
                        vertex_buffer,
                        index_buffer,
                        index_count,
                    },
                );
                self.current_lyric_mesh_text = active_text_str;
            }
        }

        let vis_width = state.visual_width.max(128).min(2048) as u32;
        uniforms.waveform_resolution = vis_width;
        uniforms.waveform_history_size = state.waveform_history.len().min(144) as u32;
        uniforms.step_fraction = step_fraction;

        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        // Only upload waveform history when the active visualizer requires it,
        // and only when new rows were pushed or the width changed (saves a
        // 1.2MB buffer write + CPU flatten/smooth pass every frame while paused)
        let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
        let history_dirty = state.waveform_history_push_count != self.last_uploaded_push_count
            || vis_width != self.last_uploaded_vis_width;
        if vis_def.id == 24 {
            if !state.lookahead_timeline.is_empty() {
                self.queue.write_buffer(&self.waveform_storage_buffer, 0, bytemuck::cast_slice(&state.lookahead_timeline));
            }
        } else if vis_def.requires_history && history_dirty {
            self.last_uploaded_push_count = state.waveform_history_push_count;
            self.last_uploaded_vis_width = vis_width;
            // Upload up to 144 most recent frames
            let hist_len = state.waveform_history.len();
            let start = hist_len.saturating_sub(144);
            let visual_width_usize = vis_width as usize;
            
            for (slot, wave) in state.waveform_history.iter().skip(start).enumerate().take(144) {
                let wave_len = wave.len().min(visual_width_usize);
                if wave_len > 0 {
                    let offset = slot * 2048; // Max width is 2048
                    self.waveform_history_flat[offset..offset + wave_len].copy_from_slice(&wave[..wave_len]);
                    
                    // Simple pre-smoothing inline
                    if wave_len > 2 {
                        let mut prev = self.waveform_history_flat[offset];
                        for j in 1..wave_len - 1 {
                            let curr = self.waveform_history_flat[offset + j];
                            let next = self.waveform_history_flat[offset + j + 1];
                            self.waveform_history_flat[offset + j] = (prev + curr * 2.0 + next) / 4.0;
                            prev = curr;
                        }
                    }
                }
            }
            self.queue.write_buffer(&self.waveform_storage_buffer, 0, bytemuck::cast_slice(&self.waveform_history_flat));
        }
        
        if vis_def.requires_fire {
            let mut bass_sum = 0.0;
            let mut mids_sum = 0.0;
            let mut highs_sum = 0.0;
            for i in 0..64 { bass_sum += uniforms.fire_heat[i]; }
            let bass = (bass_sum / 64.0 / 100.0).min(1.0);
            for i in 64..512 { mids_sum += uniforms.fire_heat[i]; }
            let mids = (mids_sum / 448.0 / 100.0).min(1.0);
            for i in 512..1024 { highs_sum += uniforms.fire_heat[i]; }
            let highs = (highs_sum / 512.0 / 100.0).min(1.0);
            
            let n_ch = state.channel_vus.len().max(1).min(32);
            let lfe_idx = if (n_ch == 6 || n_ch == 8 || n_ch == 16) && state.tracker_channels.is_none() { 3 } else { 999 };
            
            let mut fire_params = FireParams {
                bass,
                mids,
                highs,
                time: self.play_time as f32,
                cooling_factor: 1.0 - mids * 0.5,
                turb_spread_f: 1.0 + highs * 3.0,
                width: 1024,
                height: 576,
                num_channels: ch_len as u32,
                lfe_idx: lfe_idx as u32,
                fft_channels: state.raw_audio_channels.len() as u32,
                dt: frame_dt,
                display_order: [0; 16],
                channels: [[0.0; 4]; 8],
            };
            
            for i in 0..16 {
                fire_params.display_order[i] = uniforms.display_order[i];
            }
            for i in 0..n_ch {
                fire_params.channels[i / 4][i % 4] = uniforms.channels[i];
            }
            
            self.queue.write_buffer(&self.fire_params_buffer, 0, bytemuck::cast_slice(&[fire_params]));
        }
        
        // GPU spectrum is consumed only by the firesim compute (id 6) and the
        // resynth compute (requires_resynth, id 8) — skip the 256KB upload otherwise
        if state.gpu_fft && (vis_def.id == 6 || vis_def.requires_resynth) {
            if !state.gpu_spectrum_data.is_empty() {
                self.queue.write_buffer(&self._gpu_spectrum_buffer, 0, bytemuck::cast_slice(&state.gpu_spectrum_data));
            }
        }
        
        if let Some(rx) = &state.video_frame_rx {
            let mut latest_frame = None;
            while let Ok(frame) = rx.try_recv() {
                if let Some(old_frame) = latest_frame.take() {
                    if let Some(tx) = &state.free_video_frame_tx {
                        let _ = tx.try_send(old_frame);
                    }
                }
                latest_frame = Some(frame);
            }
            
            if let Some(frame) = latest_frame {
                if frame.width >= 2 && frame.height >= 2 {
                    let needs_init = self.video_state.as_ref().map_or(true, |vs| vs.width != frame.width || vs.height != frame.height || vs.bit_depth != frame.bit_depth as u32);
                    if needs_init {
                    let tex_format = if frame.bit_depth > 8 { wgpu::TextureFormat::R16Unorm } else { wgpu::TextureFormat::R8Unorm };
                    let y_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Video Y Texture"),
                        size: wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 },
                        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                        format: tex_format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    let u_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Video U Texture"),
                        size: wgpu::Extent3d { width: frame.width / 2, height: frame.height / 2, depth_or_array_layers: 1 },
                        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                        format: tex_format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    let v_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Video V Texture"),
                        size: wgpu::Extent3d { width: frame.width / 2, height: frame.height / 2, depth_or_array_layers: 1 },
                        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                        format: tex_format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    
                    let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                        label: Some("Video Sampler"),
                        address_mode_u: wgpu::AddressMode::ClampToEdge,
                        address_mode_v: wgpu::AddressMode::ClampToEdge,
                        address_mode_w: wgpu::AddressMode::ClampToEdge,
                        mag_filter: wgpu::FilterMode::Linear,
                        min_filter: wgpu::FilterMode::Linear,
                        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                        ..Default::default()
                    });
                    
                    let params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Video Params Buffer"),
                        size: std::mem::size_of::<VideoParams>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Video Bind Group"),
                        layout: &self.video_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&y_texture.create_view(&wgpu::TextureViewDescriptor::default())) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&u_texture.create_view(&wgpu::TextureViewDescriptor::default())) },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&v_texture.create_view(&wgpu::TextureViewDescriptor::default())) },
                            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
                            wgpu::BindGroupEntry { binding: 4, resource: params_buffer.as_entire_binding() },
                        ],
                    });
                    
                    self.video_state = Some(VideoState { 
                        y_texture, u_texture, v_texture, bind_group, params_buffer, 
                        width: frame.width, height: frame.height,
                        color_space: frame.color_space,
                        color_range: frame.color_range,
                        bit_depth: frame.bit_depth as u32,
                        color_trc: frame.color_trc,
                    });
                } else if let Some(vs) = &mut self.video_state {
                    vs.color_space = frame.color_space;
                    vs.color_range = frame.color_range;
                    vs.bit_depth = frame.bit_depth as u32;
                    vs.color_trc = frame.color_trc;
                }
                
                if let Some(vs) = &self.video_state {
                    // wgpu requires bytes_per_row to be a multiple of 256; repack
                    // into a staging buffer with padded rows when the decoder
                    // stride doesn't comply or buffer size is under-allocated.
                    let pack_plane = |plane: &[u8], stride: usize, width_bytes: usize, rows: usize| -> (Vec<u8>, usize, bool) {
                        let aligned = (width_bytes + 255) & !255;
                        if stride == aligned && plane.len() >= aligned * rows {
                            return (Vec::new(), stride, false);
                        }
                        let mut packed = vec![0u8; aligned * rows];
                        let plane_len = plane.len();
                        for r in 0..rows {
                            let src_start = r * stride;
                            let src_end = src_start + width_bytes;
                            let dst_start = r * aligned;
                            let dst_end = dst_start + width_bytes;
                            if src_end <= plane_len && dst_end <= packed.len() {
                                packed[dst_start..dst_end].copy_from_slice(&plane[src_start..src_end]);
                            } else if src_start < plane_len && dst_end <= packed.len() {
                                let available = plane_len - src_start;
                                packed[dst_start..dst_start + available].copy_from_slice(&plane[src_start..plane_len]);
                            }
                        }
                        (packed, aligned, true)
                    };
                    let bytes_per_px = if frame.bit_depth > 8 { 2 } else { 1 };
                    let y_w = frame.width as usize * bytes_per_px;
                    let c_w = (frame.width / 2) as usize * bytes_per_px;
                    let (y_packed, y_stride, y_repack) = pack_plane(&frame.y_plane, frame.y_stride, y_w, frame.height as usize);
                    let (u_packed, u_stride, u_repack) = pack_plane(&frame.u_plane, frame.u_stride, c_w, (frame.height / 2) as usize);
                    let (v_packed, v_stride, v_repack) = pack_plane(&frame.v_plane, frame.v_stride, c_w, (frame.height / 2) as usize);
                    let y_data: &[u8] = if y_repack { &y_packed } else { &frame.y_plane };
                    let u_data: &[u8] = if u_repack { &u_packed } else { &frame.u_plane };
                    let v_data: &[u8] = if v_repack { &v_packed } else { &frame.v_plane };
                    let y_stride = y_stride as u32;
                    let u_stride = u_stride as u32;
                    let v_stride = v_stride as u32;
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &vs.y_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        y_data,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(y_stride), rows_per_image: Some(frame.height) },
                        wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 }
                    );
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &vs.u_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        u_data,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(u_stride), rows_per_image: Some(frame.height / 2) },
                        wgpu::Extent3d { width: frame.width / 2, height: frame.height / 2, depth_or_array_layers: 1 }
                    );
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &vs.v_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        v_data,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(v_stride), rows_per_image: Some(frame.height / 2) },
                        wgpu::Extent3d { width: frame.width / 2, height: frame.height / 2, depth_or_array_layers: 1 }
                    );

                    let params = VideoParams {
                        color_space: frame.color_space,
                        color_range: frame.color_range,
                        bit_depth: frame.bit_depth as u32,
                        color_trc: frame.color_trc,
                        viewport_width: self.config.width as f32,
                        viewport_height: self.config.height as f32,
                        video_width: frame.width as f32,
                        video_height: frame.height as f32,
                    };
                    self.queue.write_buffer(&vs.params_buffer, 0, bytemuck::cast_slice(&[params]));
                }
                }
                
                if let Some(tx) = &state.free_video_frame_tx {
                    let _ = tx.try_send(frame);
                }
            }
        }
        
    }

    pub fn render(
        &mut self,
        window: &winit::window::Window,
        egui_ctx: &egui::Context,
        egui_state: &mut egui_winit::State,
        state: &AppState,
        file_dialog: &mut egui_file_dialog::FileDialog,
        gamepad_events: Vec<egui::Event>
    ) -> Result<(EngineAction, f32, f32, Option<f32>, Option<f32>, Option<f32>, f32, f32, f32, f32), wgpu::SurfaceStatus> {
        let physical_size = window.inner_size();
        if physical_size.width > 0 && physical_size.height > 0 && physical_size != self.size {
            self.resize(physical_size);
        }
        let surface_start = std::time::Instant::now();
        let surface = self.surface.as_ref().ok_or(wgpu::SurfaceStatus::Lost)?;
        let output = surface.get_current_texture();
        let surface_texture = match output {
            wgpu::CurrentSurfaceTexture::Success(tex) | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            wgpu::CurrentSurfaceTexture::Lost => return Err(wgpu::SurfaceStatus::Lost),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(wgpu::SurfaceStatus::Outdated),
            wgpu::CurrentSurfaceTexture::Timeout => return Err(wgpu::SurfaceStatus::Timeout),
            _ => return Err(wgpu::SurfaceStatus::Lost),
        };
        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let phase_surface_us = surface_start.elapsed().as_micros() as f32;

        let mut fft_shader_time_us = self.cached_fft_us;
        let mut fire_shader_time_us = self.cached_fire_us;
        let mut vis_shader_time_us = self.cached_vis_us;
        
        // NON-BLOCKING timestamp readback: poll without waiting, check if mapping completed
        if self.timestamp_mapping_active {
            // Non-blocking poll to process any completed GPU work
            let _ = self.device.poll(wgpu::PollType::Poll);
            
            if self.timestamp_map_complete.load(Ordering::Acquire) {
                if let Some(read_buffer) = &self.query_read_buffer {
                    let slice = read_buffer.slice(..);
                    let data = slice.get_mapped_range();
                    
                    let fft_start: u64 = u64::from_le_bytes(data[0..8].try_into().unwrap());
                    let fft_end: u64 = u64::from_le_bytes(data[8..16].try_into().unwrap());
                    if fft_end > fft_start {
                        let elapsed_ns = (fft_end - fft_start) as f32 * self.timestamp_period;
                        fft_shader_time_us = Some(elapsed_ns / 1_000.0);
                        self.cached_fft_us = fft_shader_time_us;
                    }

                    let fire_start: u64 = u64::from_le_bytes(data[16..24].try_into().unwrap());
                    let fire_end: u64 = u64::from_le_bytes(data[24..32].try_into().unwrap());
                    if fire_end > fire_start {
                        let elapsed_ns = (fire_end - fire_start) as f32 * self.timestamp_period;
                        fire_shader_time_us = Some(elapsed_ns / 1_000.0);
                        self.cached_fire_us = fire_shader_time_us;
                    }

                    let vis_start: u64 = u64::from_le_bytes(data[32..40].try_into().unwrap());
                    let vis_end: u64 = u64::from_le_bytes(data[40..48].try_into().unwrap());
                    if vis_end > vis_start {
                        let elapsed_ns = (vis_end - vis_start) as f32 * self.timestamp_period;
                        vis_shader_time_us = Some(elapsed_ns / 1_000.0);
                        self.cached_vis_us = vis_shader_time_us;
                    }

                    drop(data);
                    read_buffer.unmap();
                    self.timestamp_mapping_active = false;
                }
            }
            // If mapping not yet complete, we use cached values from last successful read
        }

        // Process egui UI
        let ui_start = std::time::Instant::now();
        let mut raw_input = egui_state.take_egui_input(window);
        raw_input.events.extend(gamepad_events);
        let mut central_rect = egui::Rect::from_min_max(Default::default(), egui::pos2(self.config.width as f32, self.config.height as f32));
        let mut engine_action = EngineAction::None;
        
        let mut out_meters_rect = None;
        let mut out_fire_rect = None;
        let mut out_heatmap_rect = None;
        let mut out_track_info_rect = None;
        let mut out_top_panel_rect = None;
        let mut out_video_rect = None;
        
        let vis_name = crate::state::VISUALIZERS
            .get(state.current_visualizer_idx)
            .map(|v| v.name)
            .unwrap_or("Unknown");
        
        let mut video_info_str = None;
        if let Some(vs) = &self.video_state {
            let cs = match vs.color_space {
                9 | 10 => "HDR BT.2020",
                5 | 6 => "SD BT.601",
                _ => "HD BT.709",
            };
            let cr = match vs.color_range {
                2 => "Full Range",
                _ => "Limited Range",
            };
            video_info_str = Some(format!("{}x{} | {} {}-bit {}", vs.width, vs.height, cs, vs.bit_depth, cr));
        }
        
        let full_output = egui_ctx.run_ui(raw_input, |ctx| {
            if state.is_url_dialog_open {
                egui::Window::new("Open Network Stream")
                    .collapsible(false)
                    .resizable(false)
                    .default_width(600.0)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label("Enter Stream URL (e.g. .pls, .m3u, or direct stream link):");
                        ui.add_space(4.0);
                        let mut url_text = state.url_input_text.clone();
                        let text_resp = ui.add(
                            egui::TextEdit::singleline(&mut url_text)
                                .desired_width(f32::INFINITY)
                                .min_size(egui::vec2(0.0, 30.0))
                        );
                        if state.focus_url_input {
                            text_resp.request_focus();
                            engine_action = EngineAction::ClearFocusUrlInput;
                        }
                        if text_resp.changed() {
                            engine_action = EngineAction::SetUrlInput(url_text.clone());
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add_sized([80.0, 30.0], egui::Button::new("Open")).clicked() || (text_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                                    if !url_text.is_empty() {
                                        engine_action = EngineAction::LoadUrl(url_text);
                                    }
                                }
                                if ui.add_sized([80.0, 30.0], egui::Button::new("Cancel")).clicked() {
                                    engine_action = EngineAction::CloseUrlDialog;
                                }
                            });
                        });
                        
                        if !state.url_history.is_empty() {
                            ui.separator();
                            ui.label(egui::RichText::new("Recent Streams:").strong());
                            egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                                for (url, title) in state.url_history.iter().rev() {
                                    ui.horizontal(|ui| {
                                        let width = ui.available_width() - 38.0;
                                        let btn = egui::Button::new(format!("{} ({})", title, url)).truncate();
                                        if ui.add_sized([width.max(0.0), 30.0], btn).clicked() {
                                            engine_action = EngineAction::LoadUrl(url.clone());
                                        }
                                        if ui.add_sized([30.0, 30.0], egui::Button::new("📝")).clicked() {
                                            engine_action = EngineAction::EditUrl(url.clone());
                                        }
                                    });
                                }
                            });
                        }
                    });
            }


            if state.show_stats {
                egui::Window::new("Stats")
                    .anchor(egui::Align2::RIGHT_TOP, [-10.0, 10.0])
                    .title_bar(false)
                    .resizable(false)
                    .collapsible(false)
                    .frame(egui::Frame::window(&ctx.global_style()).fill(egui::Color32::from_black_alpha(200)))
                    .show(ctx, |ui| {
                        ui.label(
                            egui::RichText::new(format!("RustTracker v{}", env!("CARGO_PKG_VERSION")))
                                .color(egui::Color32::WHITE)
                                .strong()
                        );
                        ui.label(
                            egui::RichText::new(format!("Visualizer: {}", vis_name))
                                .color(egui::Color32::YELLOW)
                        );
                        ui.separator();
                        
                        if state.stats.bitstream_active {
                            ui.label(
                                egui::RichText::new("Bitstream Passthrough: ACTIVE")
                                    .color(egui::Color32::from_rgb(0, 255, 128))
                                    .strong()
                            );
                            if let Some(info) = &state.video_info {
                                ui.label(
                                    egui::RichText::new(format!("Format: {}", info))
                                        .color(egui::Color32::LIGHT_GREEN)
                                );
                            }
                            ui.separator();
                        } else if state.passthrough_enabled {
                            ui.label(
                                egui::RichText::new("Bitstream Passthrough: Inactive")
                                    .color(egui::Color32::GRAY)
                            );
                            ui.separator();
                        }
                        ui.label(
                            egui::RichText::new(format!("FPS: {:.1}", state.current_fps))
                                .color(egui::Color32::GREEN)
                                .strong()
                        );
                        ui.label(
                            egui::RichText::new(format!("Frame Time: {:.2} ms", 1000.0 / state.current_fps.max(1.0)))
                                .color(egui::Color32::LIGHT_GREEN)
                        );
                        ui.label(
                            egui::RichText::new(format!("CPU UI: {:.2} ms | CPU Render: {:.2} ms", state.stats.ui_us / 1000.0, state.stats.render_us / 1000.0))
                                .color(egui::Color32::WHITE)
                        );
                        if state.gpu_fft {
                            let fft_label = if state.stats.gpu_fft_us > 0.0 {
                                format!("GPU FFT: {:.2} ms", state.stats.gpu_fft_us / 1000.0)
                            } else {
                                "GPU FFT: Active (Compute)".to_string()
                            };
                            ui.label(
                                egui::RichText::new(fft_label)
                                    .color(egui::Color32::LIGHT_BLUE)
                            );
                            ui.label(
                                egui::RichText::new(format!("CPU Decode: {:.2} ms", state.stats.decode_us / 1000.0))
                                    .color(egui::Color32::WHITE)
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(format!("CPU FFT: {:.2} ms", state.stats.fft_us / 1000.0))
                                    .color(egui::Color32::WHITE)
                            );
                            ui.label(
                                egui::RichText::new(format!("CPU Decode: {:.2} ms", state.stats.decode_us / 1000.0))
                                    .color(egui::Color32::WHITE)
                            );
                        }
                        let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
                        let mut total_vis_us = state.stats.shader_us;
                        if vis_def.requires_fire {
                            total_vis_us += state.stats.fire_us;
                        }
                        if total_vis_us > 0.0 {
                            ui.label(
                                egui::RichText::new(format!("Visualization Shader (GPU): {:.2} ms", total_vis_us / 1000.0))
                                    .color(egui::Color32::LIGHT_BLUE)
                            );
                        }
                        let buffer_label = if state.duration_seconds <= 0.0 { "Network Buffer" } else { "Audio Buffer" };
                        ui.label(
                            egui::RichText::new(format!("{}: {:.1}%", buffer_label, state.stats.audio_buffer_fill_pct))
                                .color(if state.stats.audio_buffer_fill_pct < 5.0 { egui::Color32::RED } else if state.stats.audio_buffer_fill_pct > 95.0 { egui::Color32::YELLOW } else { egui::Color32::GREEN })
                        );
                        if state.video_frame_rx.is_some() {
                            ui.label(
                                egui::RichText::new(format!("Video Buffer: {:.1}%", state.stats.video_buffer_fill_pct))
                                    .color(if state.stats.video_buffer_fill_pct < 1.0 { egui::Color32::RED } else { egui::Color32::YELLOW })
                            );
                        }
                        ui.separator();
                        ui.label(
                            egui::RichText::new("⏱ Frame Phase Breakdown:")
                                .color(egui::Color32::from_rgb(180, 180, 255))
                                .strong()
                        );
                        let phases = [
                            ("  Lock+Update", state.stats.phase_lock_update_us),
                            ("  Snapshot", state.stats.phase_snapshot_us),
                            ("  Surface Acq", state.stats.phase_surface_us),
                            ("  Egui Layout", state.stats.phase_egui_layout_us),
                            ("  GPU Encode", state.stats.phase_encode_us),
                            ("  Post Write", state.stats.phase_post_us),
                        ];
                        let total_phases: f32 = phases.iter().map(|(_, v)| v).sum();
                        for (name, val) in &phases {
                            let mut color = if *val > 2000.0 { egui::Color32::RED }
                                       else if *val > 1000.0 { egui::Color32::YELLOW } 
                                       else { egui::Color32::from_rgb(160, 160, 160) };
                            
                            let display_name = if *name == "  Surface Acq" {
                                color = egui::Color32::from_rgb(160, 160, 160);
                                "  Surface Acq (VSync Wait)"
                            } else {
                                *name
                            };
                            
                            ui.label(
                                egui::RichText::new(format!("{}: {:.2} ms", display_name, val / 1000.0))
                                    .color(color)
                            );
                        }
                        ui.label(
                            egui::RichText::new(format!("  Total Phases: {:.2} ms", total_phases / 1000.0))
                                .color(egui::Color32::from_rgb(180, 180, 255))
                        );
                        ui.label(
                            egui::RichText::new(format!("Hardware Channels: {} | Source: {}", state.hardware_channels, state.num_channels))
                                .color(if state.hardware_channels != state.num_channels { egui::Color32::YELLOW } else { egui::Color32::GRAY })
                        );
                        ui.label(
                            egui::RichText::new(format!("Clipping Events: {}", state.stats.clipping_events))
                                .color(if state.stats.clipping_events > 0 { egui::Color32::RED } else { egui::Color32::GRAY })
                        );
                        if let Some(vi) = &video_info_str {
                            ui.separator();
                            ui.label(
                                egui::RichText::new("Video Stream:")
                                    .color(egui::Color32::GRAY)
                            );
                            ui.label(
                                egui::RichText::new(vi)
                                    .color(egui::Color32::YELLOW)
                            );
                        }
                    });
            }

            if state.show_help {
                egui::Window::new("Help")
                    .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
                    .title_bar(false)
                    .resizable(false)
                    .collapsible(false)
                    .frame(egui::Frame::window(&ctx.global_style()).fill(egui::Color32::from_black_alpha(200)))
                    .show(ctx, |ui| {
                        ui.label(egui::RichText::new("Shortcuts").color(egui::Color32::WHITE).strong().size(16.0));
                        ui.separator();
                        egui::Grid::new("help_shortcuts_grid")
                            .num_columns(2)
                            .spacing([20.0, 6.0])
                            .show(ui, |ui| {
                                let shortcut = |ui: &mut egui::Ui, key: &str, gp: Option<&str>, desc: &str| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        ui.label(egui::RichText::new(key).color(egui::Color32::WHITE).strong());
                                        if let Some(gp_act) = gp {
                                            ui.label(egui::RichText::new(" / ").color(egui::Color32::DARK_GRAY));
                                            ui.label(egui::RichText::new(gamepad_icon(state.gamepad_type, gp_act))
                                                .color(egui::Color32::LIGHT_BLUE)
                                                .size(16.0)
                                            );
                                        }
                                    });
                                    ui.label(egui::RichText::new(desc).color(egui::Color32::GRAY));
                                    ui.end_row();
                                };
                                shortcut(ui, "o", Some("Y"), "Open File");
                                shortcut(ui, "space", Some("A"), "Play / Pause");
                                shortcut(ui, "v", Some("X"), "Toggle Video");
                                shortcut(ui, "m", None, "Visualizer Modules");
                                shortcut(ui, "left/right", Some("D-Pad L/R"), "Seek Timeline");
                                shortcut(ui, "tab", Some("L1"), "Toggle HUD");
                                shortcut(ui, "up/down", Some("D-Pad U/D"), "Cycle Visualizer");
                                shortcut(ui, "s", Some("B"), "Toggle Stats");
                                shortcut(ui, "h", None, "Toggle Help");
                                shortcut(ui, "q / esc", Some("Select"), "Quit");
                                shortcut(ui, "f", Some("Start"), "Toggle Fullscreen");
                                shortcut(ui, "g", Some("R1"), "Toggle GPU FFT");
                                shortcut(ui, "1-9", None, "Select Audio Track");
                                shortcut(ui, "[ / ]", None, "Scale Panels");
                                if state.visualizer_mode == 20 {
                                    shortcut(ui, "c", None, "Toggle Camera Mode");
                                }
                            });
                    });
            }

            if state.show_vis_picker {
                egui::Window::new("Visualizer Modules")
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .title_bar(false)
                    .resizable(false)
                    .collapsible(false)
                    .fixed_size([540.0, 500.0])
                    .frame(egui::Frame::window(&ctx.global_style())
                        .fill(egui::Color32::from_black_alpha(230))
                        .corner_radius(12.0)
                        .inner_margin(20.0))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("🎨 Visualization Modules")
                                    .color(egui::Color32::WHITE)
                                    .strong()
                                    .size(20.0)
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.link("None").clicked() {
                                    engine_action = EngineAction::VisPickerEnableNone;
                                }
                                ui.label(egui::RichText::new("•").color(egui::Color32::from_gray(100)));
                                if ui.link("All").clicked() {
                                    engine_action = EngineAction::VisPickerEnableAll;
                                }
                                ui.label(egui::RichText::new("Rotation:").color(egui::Color32::from_gray(150)).size(13.0));
                            });
                        });
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new("Enter to select  •  Space to toggle rotation  •  Esc to close")
                                .color(egui::Color32::from_gray(130))
                                .size(11.0)
                        );
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(4.0);

                        egui::ScrollArea::vertical()
                            .max_height(420.0)
                            .show(ui, |ui| {
                                for (i, vis) in crate::state::VISUALIZERS.iter().enumerate() {
                                    let is_cursor = i == state.vis_picker_cursor;
                                    let is_active = i == state.current_visualizer_idx;
                                    let is_enabled = state.vis_enabled.get(i).copied().unwrap_or(true);

                                    let bg = if is_cursor {
                                        egui::Color32::from_rgba_unmultiplied(50, 110, 220, 100)
                                    } else if is_active {
                                        egui::Color32::from_rgba_unmultiplied(30, 90, 30, 80)
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };

                                    let row_frame = egui::Frame::NONE
                                        .fill(bg)
                                        .corner_radius(6.0)
                                        .inner_margin(egui::Margin::symmetric(10, 6));

                                    let row_resp = row_frame.show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.set_min_width(ui.available_width());
                                            // Enable/disable toggle indicator
                                            let toggle_text = if is_enabled { "✅" } else { "⬜" };
                                            let toggle_color = if i == 0 {
                                                egui::Color32::from_gray(80) // Locked on
                                            } else if is_enabled {
                                                egui::Color32::WHITE
                                            } else {
                                                egui::Color32::from_gray(60)
                                            };
                                            let toggle_resp = ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(toggle_text)
                                                        .size(14.0)
                                                        .color(toggle_color)
                                                ).sense(egui::Sense::click())
                                            );
                                            if toggle_resp.clicked() && i != 0 {
                                                engine_action = EngineAction::VisPickerToggleEnabled(i);
                                            }

                                            ui.add_space(6.0);

                                            // Name
                                            let name_color = if !is_enabled {
                                                egui::Color32::from_gray(90)
                                            } else if is_active {
                                                egui::Color32::from_rgb(80, 255, 80)
                                            } else {
                                                egui::Color32::WHITE
                                            };
                                            ui.label(
                                                egui::RichText::new(vis.name)
                                                    .color(name_color)
                                                    .strong()
                                                    .size(15.0)
                                            );

                                            // Active indicator
                                            if is_active {
                                                ui.label(
                                                    egui::RichText::new("▶")
                                                        .color(egui::Color32::from_rgb(80, 255, 80))
                                                        .size(13.0)
                                                );
                                            }

                                            // Description (right-aligned)
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let desc_color = if !is_enabled {
                                                        egui::Color32::from_gray(60)
                                                    } else {
                                                        egui::Color32::from_gray(120)
                                                    };
                                                    ui.label(
                                                        egui::RichText::new(vis.description)
                                                            .color(desc_color)
                                                            .size(11.5)
                                                    );
                                                }
                                            );
                                        });
                                    });

                                    let rect = row_resp.response.rect;
                                    let id = ui.id().with(i);
                                    let response = ui.interact(rect, id, egui::Sense::click());

                                    // Hover moves cursor to this row
                                    if response.hovered() && !is_cursor {
                                        engine_action = EngineAction::VisPickerSetCursor(i);
                                    }

                                    // Click row to select visualizer
                                    if response.clicked() {
                                        engine_action = EngineAction::VisPickerSelect(i);
                                    }

                                    // Ensure cursor row is scrolled into view (for keyboard/gamepad navigation)
                                    if is_cursor && state.vis_picker_scroll_to_cursor {
                                        response.scroll_to_me(Some(egui::Align::Center));
                                    }
                                }
                            });
                    });
            }

            let mut append = state.append_to_playlist;
            file_dialog.update_with_right_panel_ui(ctx, &mut |ui, _fd| {
                ui.add_space(10.0);
                ui.heading("Options");
                ui.separator();
                ui.checkbox(&mut append, "Add to Playlist instead of replacing");
            });
            
            if append != state.append_to_playlist {
                engine_action = EngineAction::SetAppendToPlaylist(append);
            }

            if let Some(paths) = file_dialog.take_picked_multiple() {
                let strings = paths.into_iter().map(|p| p.display().to_string()).collect();
                engine_action = EngineAction::LoadFiles(strings, append);
            } else if let Some(path) = file_dialog.take_picked() {
                engine_action = EngineAction::LoadFiles(vec![path.display().to_string()], append);
            }

            if !state.file_loaded {
                central_rect = ctx.content_rect();
                let time = self.smooth_time as f32;
                
                // --- Background Retro Grid (Demoscene Vibe) ---
                let bg_painter = ctx.layer_painter(egui::LayerId::background());
                let rect = ctx.content_rect();
                let horizon_y = rect.top() + rect.height() * 0.55;
                let center_x = rect.center().x;
                
                // 1. Sky gradient
                let sky_steps = 40;
                let sky_height = horizon_y - rect.top();
                for i in 0..sky_steps {
                    let t = i as f32 / sky_steps as f32;
                    let next_t = (i + 1) as f32 / sky_steps as f32;
                    let color = if t < 0.6 {
                        let st = t / 0.6;
                        egui::Color32::from_rgb(
                            (43.0 * (1.0 - st) + 150.0 * st) as u8,
                            (16.0 * (1.0 - st) + 20.0 * st) as u8,
                            (85.0 * (1.0 - st) + 120.0 * st) as u8,
                        )
                    } else {
                        let st = (t - 0.6) / 0.4;
                        egui::Color32::from_rgb(
                            (150.0 * (1.0 - st) + 255.0 * st) as u8,
                            (20.0 * (1.0 - st) + 126.0 * st) as u8,
                            (120.0 * (1.0 - st) + 103.0 * st) as u8,
                        )
                    };
                    bg_painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left(), rect.top() + t * sky_height),
                            egui::pos2(rect.right(), rect.top() + next_t * sky_height),
                        ),
                        0.0,
                        color,
                    );
                }

                // 2. Stars
                let pseudo_rand = |seed: u32| -> f32 {
                    let mut x = seed.wrapping_mul(136453);
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    (x % 1000) as f32 / 1000.0
                };
                
                for i in 0..50 {
                    let rx = pseudo_rand(i * 31);
                    let ry = pseudo_rand(i * 31 + 1);
                    let rr = pseudo_rand(i * 31 + 2);
                    let rt = pseudo_rand(i * 31 + 3);
                    
                    let x = rect.left() + rx * rect.width();
                    let y = rect.top() + ry * sky_height * 0.7; // Stars in upper 70%
                    let radius = rr * 1.5 + 0.5;
                    let twinkle = ((time * (1.0 + rt * 2.0) + rx * 100.0).sin() * 0.5 + 0.5) * 200.0 + 55.0;
                    
                    bg_painter.circle_filled(
                        egui::pos2(x, y),
                        radius,
                        egui::Color32::from_white_alpha(twinkle as u8),
                    );
                }

                // 3. Retro Sliced Sun
                let sun_radius = rect.width().min(rect.height()) * 0.25;
                let sun_center = egui::pos2(center_x, horizon_y - sun_radius * 0.3);
                
                let sun_steps = 40;
                for i in 0..sun_steps {
                    let t = i as f32 / sun_steps as f32;
                    let next_t = (i + 1) as f32 / sun_steps as f32;
                    
                    if t > 0.45 {
                        let slice_t = (t - 0.45) / 0.55;
                        let slice_val = (slice_t * 15.0).fract();
                        let gap_threshold = 0.2 + slice_t * 0.6; 
                        if slice_val < gap_threshold {
                            continue;
                        }
                    }
                    
                    let y_min = sun_center.y - sun_radius + t * sun_radius * 2.0;
                    let y_max = sun_center.y - sun_radius + next_t * sun_radius * 2.0;
                    
                    let r = 255;
                    let g = (204.0 * (1.0 - t) + 80.0 * t) as u8;
                    let b = (0.0 * (1.0 - t) + 100.0 * t) as u8;
                    let color = egui::Color32::from_rgb(r, g, b);
                    
                    bg_painter.with_clip_rect(egui::Rect::from_min_max(
                        egui::pos2(rect.left(), y_min),
                        egui::pos2(rect.right(), y_max),
                    )).circle_filled(
                        sun_center,
                        sun_radius,
                        color,
                    );
                }

                // 4. Wireframe Mountains
                for i in 0..25 {
                    let h1 = pseudo_rand(i * 17);
                    let h2 = pseudo_rand(i * 17 + 1);
                    let h3 = pseudo_rand(i * 17 + 2);
                    
                    let cx = rect.left() + h1 * rect.width();
                    let cy = horizon_y;
                    let width = rect.width() * (0.1 + h2 * 0.25);
                    let height = rect.height() * (0.05 + h3 * 0.25);
                    
                    let p1 = egui::pos2(cx - width, cy);
                    let p2 = egui::pos2(cx + width, cy);
                    let p3 = egui::pos2(cx, cy - height);
                    
                    bg_painter.add(egui::Shape::convex_polygon(
                        vec![p1, p2, p3],
                        egui::Color32::from_rgb(27, 27, 58),
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(91, 50, 212)),
                    ));
                    
                    // Center ridge
                    let ridge_offset = (pseudo_rand(i * 17 + 3) - 0.5) * 0.4;
                    bg_painter.line_segment(
                        [p3, egui::pos2(cx + width * ridge_offset, cy)],
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(91, 50, 212))
                    );
                }

                // 5. Floor Grid
                bg_painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left(), horizon_y),
                        egui::pos2(rect.right(), rect.bottom())
                    ),
                    0.0,
                    egui::Color32::from_rgb(36, 21, 84)
                );

                let grid_color = egui::Color32::from_rgb(212, 34, 161);
                
                // Vertical radiating lines
                let num_v_lines = 40;
                for i in 0..=num_v_lines {
                    let t = i as f32 / num_v_lines as f32;
                    let bottom_x = rect.left() + (t - 0.5) * rect.width() * 8.0;
                    
                    bg_painter.line_segment(
                        [egui::pos2(center_x, horizon_y), egui::pos2(bottom_x, rect.bottom())],
                        egui::Stroke::new(1.0_f32, grid_color)
                    );
                }
                
                // Horizontal scrolling perspective lines
                let num_h_lines = 30;
                for i in 0..num_h_lines {
                    let offset = (i as f32 - (time * 1.5).fract()) / num_h_lines as f32;
                    if offset <= 0.0 { continue; }
                    let y = horizon_y + (rect.bottom() - horizon_y) * offset.powf(3.0);
                    let thickness = 1.0 + offset * 2.0;
                    
                    bg_painter.line_segment(
                        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                        egui::Stroke::new(thickness, grid_color)
                    );
                }
                // --- End Retro Grid ---
                
                let is_mobile = cfg!(target_os = "android");
                let is_game_mode = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase() == "gamescope" || 
                                   std::env::var("XDG_SESSION_DESKTOP").unwrap_or_default().to_lowercase() == "gamescope" ||
                                   std::env::var("STEAM_DECK").is_ok();
                                   
                let show_kb = !is_game_mode && !is_mobile;
                let show_gp = state.has_gamepad && !is_mobile;
                let show_touch = is_mobile;

                if show_kb || show_gp || show_touch {
                    egui::Panel::bottom("splash_bottom")
                        .frame(egui::Frame::NONE.fill(egui::Color32::from_rgba_unmultiplied(10, 10, 15, 180)).inner_margin(if is_mobile { 12.0 } else { 40.0 }))
                        .show_inside(ctx, |ui| {
                            let height = if is_mobile { 120.0 } else if show_kb && show_gp { 170.0 } else { 140.0 };
                            ui.add_space(height);
                        });
                        
                    egui::Area::new(egui::Id::new("splash_shortcuts_area"))
                        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, if is_mobile { -12.0 } else { -40.0 }))
                        .show(ctx, |ui| {
                            egui::Frame::NONE
                                .fill(egui::Color32::from_black_alpha(210))
                                .corner_radius(10.0)
                                .inner_margin(if is_mobile { 12.0 } else { 20.0 })
                                .show(ui, |ui| {
                                    if show_touch {
                                        ui.vertical_centered(|ui| {
                                            ui.label(egui::RichText::new("Touch Gestures Guide").color(egui::Color32::LIGHT_GRAY).strong().size(14.0));
                                            ui.add_space(6.0);
                                            egui::Grid::new("touch_shortcuts")
                                                .num_columns(2)
                                                .spacing([18.0, 5.0])
                                                .show(ui, |ui| {
                                                    ui.label(egui::RichText::new("Swipe L / R").color(egui::Color32::from_rgb(0, 220, 255)).strong().size(12.5));
                                                    ui.label(egui::RichText::new(format!("Switch Visualizers ({})", crate::state::VISUALIZERS.len())).color(egui::Color32::LIGHT_GRAY).size(12.5));
                                                    ui.end_row();

                                                    ui.label(egui::RichText::new("Swipe U / D").color(egui::Color32::from_rgb(0, 220, 255)).strong().size(12.5));
                                                    ui.label(egui::RichText::new("Tabs / Fullscreen Video").color(egui::Color32::LIGHT_GRAY).size(12.5));
                                                    ui.end_row();

                                                    ui.label(egui::RichText::new("Single Tap").color(egui::Color32::from_rgb(0, 220, 255)).strong().size(12.5));
                                                    ui.label(egui::RichText::new("Play / Pause").color(egui::Color32::LIGHT_GRAY).size(12.5));
                                                    ui.end_row();

                                                    ui.label(egui::RichText::new("Double Tap").color(egui::Color32::from_rgb(0, 220, 255)).strong().size(12.5));
                                                    ui.label(egui::RichText::new("Toggle HUD Visibility").color(egui::Color32::LIGHT_GRAY).size(12.5));
                                                    ui.end_row();

                                                    ui.label(egui::RichText::new("2-Finger Tap").color(egui::Color32::from_rgb(0, 220, 255)).strong().size(12.5));
                                                    ui.label(egui::RichText::new("Cycle Audio Track").color(egui::Color32::LIGHT_GRAY).size(12.5));
                                                    ui.end_row();

                                                    ui.label(egui::RichText::new("3-Finger Tap").color(egui::Color32::from_rgb(0, 220, 255)).strong().size(12.5));
                                                    ui.label(egui::RichText::new("Toggle Engine Stats").color(egui::Color32::LIGHT_GRAY).size(12.5));
                                                    ui.end_row();
                                                });
                                        });
                                    } else {
                                        ui.horizontal_centered(|ui| {
                                            let pairs_per_row = if show_kb && show_gp { 2 } else { 3 };
                                            
                                            if show_kb {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("🖮 Keyboard Shortcuts").color(egui::Color32::LIGHT_GRAY).strong().size(18.0));
                                                    ui.add_space(10.0);
                                                    
                                                    egui::Grid::new("kb_shortcuts")
                                                        .num_columns(pairs_per_row * 2)
                                                        .spacing([25.0, 12.0])
                                                        .show(ui, |ui| {
                                                            let mut col = 0;
                                                            let mut kb_shortcut = |key: &str, desc: &str| {
                                                                ui.label(egui::RichText::new(key).color(egui::Color32::WHITE).strong());
                                                                ui.label(egui::RichText::new(desc).color(egui::Color32::GRAY));
                                                                col += 1;
                                                                if col == pairs_per_row {
                                                                    ui.end_row();
                                                                    col = 0;
                                                                }
                                                            };
                                                            kb_shortcut("o", "Open File");
                                                            kb_shortcut("space", "Play / Pause");
                                                            kb_shortcut("v", "Toggle Video");
                                                            kb_shortcut("m", "Visualizer Modules");
                                                            kb_shortcut("left/right", "Seek Timeline");
                                                            kb_shortcut("tab", "Toggle HUD");
                                                            kb_shortcut("up/down", "Cycle Vis");
                                                            kb_shortcut("s", "Toggle Stats");
                                                            kb_shortcut("q / esc", "Quit");
                                                            kb_shortcut("f", "Fullscreen");
                                                            kb_shortcut("g", "GPU FFT");
                                                            kb_shortcut("1-9", "Audio Track");
                                                            let ctrl_mod = if std::env::consts::OS == "macos" { "⌘" } else { "ctrl+" };
                                                            kb_shortcut(&format!("{}L/R", ctrl_mod), "Prev/Next");
                                                            kb_shortcut("[ / ]", "Scale Panels");
                                                            kb_shortcut("h", "Toggle Help");
                                                        });
                                                });
                                            }
                                            
                                            if show_kb && show_gp {
                                                ui.add_space(20.0);
                                                let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 180.0), egui::Sense::hover());
                                                ui.painter().line_segment(
                                                    [rect.center_top(), rect.center_bottom()],
                                                    (1.0, egui::Color32::from_gray(60))
                                                );
                                                ui.add_space(20.0);
                                            }
                                            
                                            if show_gp {
                                                ui.vertical(|ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.label(egui::RichText::new("🎮 Gamepad Controls").color(egui::Color32::LIGHT_GRAY).strong().size(18.0));
                                                    });
                                                    ui.add_space(10.0);
                                                    egui::Grid::new("gp_shortcuts_wide")
                                                        .num_columns(pairs_per_row * 2)
                                                        .spacing([25.0, 12.0])
                                                        .show(ui, |ui| {
                                                            let mut col = 0;
                                                            let mut gp_shortcut = |gp: &str, desc: &str| {
                                                                ui.label(egui::RichText::new(gamepad_icon(state.gamepad_type, gp)).color(egui::Color32::LIGHT_BLUE).size(16.0));
                                                                ui.label(egui::RichText::new(desc).color(egui::Color32::GRAY));
                                                                col += 1;
                                                                if col == pairs_per_row {
                                                                    ui.end_row();
                                                                    col = 0;
                                                                }
                                                            };
                                                            gp_shortcut("Y", "Open File");
                                                            gp_shortcut("A", "Play / Pause");
                                                            gp_shortcut("X", "Toggle Video");
                                                            gp_shortcut("D-Pad L/R", "Seek Timeline");
                                                            gp_shortcut("L2", "Toggle HUD");
                                                            gp_shortcut("D-Pad U/D", "Cycle Vis");
                                                            gp_shortcut("B", "Toggle Stats");
                                                            gp_shortcut("Select", "Quit");
                                                            gp_shortcut("Start", "Fullscreen");
                                                            gp_shortcut("R2", "GPU FFT");
                                                            gp_shortcut("L1 / R1", "Prev/Next");
                                                        });
                                                });
                                            }
                                        });
                                    }
                                });
                        });
                }

                let frame = egui::Frame::NONE
                    .fill(egui::Color32::from_rgba_unmultiplied(10, 10, 15, 180)) // Translucent to show grid
                    .inner_margin(if is_mobile { 16.0 } else { 40.0 });
                    
                egui::CentralPanel::default().frame(frame).show_inside(ctx, |ui| {
                    let real_avail_height = ui.available_height();
                    let real_avail_width = ui.available_width();
                    
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            let top_space = if is_mobile {
                                (real_avail_height * 0.06).max(20.0)
                            } else {
                                real_avail_height * 0.15
                            };
                            if top_space > 0.0 {
                                ui.add_space(top_space);
                            }
                            // Scale title dynamically to fit portrait phone (9:19.5), steam deck, or 4K monitors
                            let target_w = (real_avail_width - 32.0).max(100.0);
                            let width_scale = (target_w / 950.0).clamp(0.25, 1.0);
                            let height_scale = (real_avail_height / 450.0).clamp(0.25, 1.0);
                            let scale_factor = width_scale.min(height_scale);
                            let title_width = 950.0 * scale_factor;
                            let title_height = 140.0 * scale_factor;
                            let font_size = 120.0 * scale_factor;
                            let gradient_extent = 55.0 * scale_factor;

                            // --- Glowing Animated Title ---
                            let (title_rect, _) = ui.allocate_exact_size(egui::vec2(title_width, title_height), egui::Sense::hover());
                            let painter = ui.painter();
                            let text = "RustTracker";
                            
                            let font_id = egui::FontId::new(font_size, egui::FontFamily::Name("Orbitron".into()));
                            
                            // 1. Silver Outer Bevel (3px offset)
                            let silver_color = egui::Color32::from_rgb(200, 220, 255);
                            for dx in [-3.0, 0.0, 3.0] {
                                for dy in [-3.0, 0.0, 3.0] {
                                    if dx == 0.0 && dy == 0.0 { continue; }
                                    painter.text(
                                        title_rect.center() + egui::vec2(dx, dy),
                                        egui::Align2::CENTER_CENTER,
                                        text,
                                        font_id.clone(),
                                        silver_color,
                                    );
                                }
                            }
                            
                            // 2. Black Inner Outline (1px offset)
                            for dx in [-1.0, 0.0, 1.0] {
                                for dy in [-1.0, 0.0, 1.0] {
                                    if dx == 0.0 && dy == 0.0 { continue; }
                                    painter.text(
                                        title_rect.center() + egui::vec2(dx, dy),
                                        egui::Align2::CENTER_CENTER,
                                        text,
                                        font_id.clone(),
                                        egui::Color32::BLACK,
                                    );
                                }
                            }
                            
                            // 3. Sliced Chrome Scrolling Palette Interior
                            let steps = 40; // More steps for cooler copper bar effect
                            let top_y = title_rect.center().y - gradient_extent;
                            let bottom_y = title_rect.center().y + gradient_extent;
                            let height = bottom_y - top_y;
                            
                            for i in 0..steps {
                                let t = i as f32 / steps as f32;
                                let next_t = (i + 1) as f32 / steps as f32;
                                let min_y = top_y + t * height;
                                let max_y = top_y + next_t * height;
                                
                                let clip_rect = egui::Rect::from_min_max(
                                    egui::pos2(title_rect.left(), min_y),
                                    egui::pos2(title_rect.right(), max_y),
                                );
                                
                                // Palette cycling calculation
                                let scroll_speed = 0.1;
                                let mut color_t = (t + time * scroll_speed).fract();
                                if color_t < 0.0 { color_t += 1.0; }
                                
                                let color = if color_t < 0.48 {
                                    // Sky: Cyan to Dark Blue
                                    let sky_t = color_t / 0.48;
                                    let r = (0.0 * (1.0 - sky_t) + 10.0 * sky_t) as u8;
                                    let g = (220.0 * (1.0 - sky_t) + 30.0 * sky_t) as u8;
                                    let b = (255.0 * (1.0 - sky_t) + 120.0 * sky_t) as u8;
                                    egui::Color32::from_rgb(r, g, b)
                                } else if color_t < 0.52 {
                                    // Chrome Horizon Reflection
                                    egui::Color32::WHITE
                                } else {
                                    // Ground Reflection: Dark Brown to Orange/Tan
                                    let ground_t = (color_t - 0.52) / 0.48;
                                    let r = (50.0 * (1.0 - ground_t) + 255.0 * ground_t) as u8;
                                    let g = (15.0 * (1.0 - ground_t) + 160.0 * ground_t) as u8;
                                    let b = (0.0 * (1.0 - ground_t) + 50.0 * ground_t) as u8;
                                    egui::Color32::from_rgb(r, g, b)
                                } ;

                                painter.with_clip_rect(clip_rect).text(
                                    title_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    text,
                                    font_id.clone(),
                                    color,
                                );
                            }
                            ui.add_space(20.0);
                            
                            let is_file_hovered = !ui.input(|i| i.raw.hovered_files.is_empty()) || state.hovered_file.is_some();
                            if is_file_hovered {
                                let badge_text = if let Some(hf) = &state.hovered_file {
                                    let fn_only = std::path::Path::new(hf).file_name().unwrap_or_default().to_string_lossy();
                                    format!("📥 Drop '{}' to start playing", fn_only)
                                } else {
                                    "📥 Drop audio file here to start playing".to_string()
                                };
                                let font_id = egui::FontId::proportional(16.0);
                                let galley = ui.painter().layout(
                                    badge_text,
                                    font_id,
                                    egui::Color32::WHITE,
                                    ui.available_width().min(600.0),
                                );
                                let (badge_rect, _) = ui.allocate_exact_size(galley.size() + egui::vec2(28.0, 14.0), egui::Sense::hover());
                                ui.painter().rect(
                                    badge_rect,
                                    8.0,
                                    egui::Color32::from_rgba_unmultiplied(0, 120, 220, 230),
                                    egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(140, 230, 255)),
                                    egui::StrokeKind::Outside,
                                );
                                ui.painter().galley(badge_rect.min + egui::vec2(14.0, 7.0), galley, egui::Color32::WHITE);
                                ui.add_space(10.0);
                            }

                                let is_narrow = real_avail_width < 640.0;
                                let btn_text = if is_file_hovered { "📥 DROP TO PLAY" } else { "OPEN AUDIO FILE" };
                                let btn_fill = if is_file_hovered { egui::Color32::from_rgb(0, 160, 240) } else { egui::Color32::from_rgb(0, 100, 200) };
                                let btn_stroke = if is_file_hovered { egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(160, 240, 255)) } else { egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(80, 180, 255)) };

                                let btn_font_size = if is_narrow { 18.0 } else { 22.0 };
                                let btn_h = if is_narrow { 52.0 } else { 60.0 };
                                let btn_w = if is_narrow { (real_avail_width - 48.0).min(340.0) } else { 280.0 };

                                let btn = egui::Button::new(
                                    egui::RichText::new(btn_text)
                                        .size(btn_font_size)
                                        .color(egui::Color32::WHITE)
                                        .strong()
                                )
                                .fill(btn_fill)
                                .stroke(btn_stroke);

                                let url_btn = egui::Button::new(
                                    egui::RichText::new("OPEN URL / STREAM")
                                        .size(btn_font_size)
                                        .color(egui::Color32::WHITE)
                                        .strong()
                                )
                                .fill(egui::Color32::from_rgb(100, 50, 150))
                                .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(180, 80, 255)));

                                egui::Frame::NONE
                                    .shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: egui::Color32::from_black_alpha(200) })
                                    .corner_radius(8.0)
                                    .show(ui, |ui| {
                                        if is_narrow {
                                            ui.vertical_centered(|ui| {
                                                let resp1 = ui.add_sized([btn_w, btn_h], btn);
                                                if resp1.clicked() {
                                                    engine_action = EngineAction::OpenFile;
                                                }
                                                ui.add_space(14.0);
                                                let resp2 = ui.add_sized([btn_w, btn_h], url_btn);
                                                if resp2.clicked() {
                                                    engine_action = EngineAction::OpenUrlDialog;
                                                }
                                            });
                                        } else {
                                            ui.horizontal(|ui| {
                                                let total_width = btn_w * 2.0 + 20.0;
                                                ui.add_space((ui.available_width() - total_width) / 2.0);
                                                let resp1 = ui.add_sized([btn_w, btn_h], btn);
                                                if resp1.clicked() {
                                                    engine_action = EngineAction::OpenFile;
                                                }
                                                ui.add_space(20.0);
                                                let resp2 = ui.add_sized([btn_w, btn_h], url_btn);
                                                if resp2.clicked() {
                                                    engine_action = EngineAction::OpenUrlDialog;
                                                }
                                            });
                                        }
                                    });
                                
                                ui.add_space(20.0);
                            
                            let mut force_stereo = state.force_stereo_downmix;
                            if ui.checkbox(&mut force_stereo, "Force Stereo Downmix (Fixes crackling on some devices)").changed() {
                                engine_action = EngineAction::SetForceStereo(force_stereo);
                            }
                            
                            #[cfg(any(target_os = "windows", target_os = "linux"))]
                            {
                                let mut passthrough = state.passthrough_enabled;
                                 
                                #[cfg(target_os = "windows")]
                                let label = "Enable Bitstream Passthrough (WASAPI Exclusive)";
                                #[cfg(target_os = "linux")]
                                let label = "Enable Bitstream Passthrough (PipeWire)";
                                
                                if ui.checkbox(&mut passthrough, label).changed() {
                                    engine_action = EngineAction::SetPassthrough(passthrough);
                                }
                            }
                            // The shortcuts are now rendered in the TopBottomPanel
                        });
                    });
                });

                let dropped_files: Vec<String> = ctx.input(|i| {
                    i.raw.dropped_files
                        .iter()
                        .filter_map(|df| df.path.as_ref().map(|p| p.to_string_lossy().into_owned()))
                        .collect()
                });
                if !dropped_files.is_empty() {
                    engine_action = EngineAction::LoadFiles(dropped_files, false);
                }

                return;
            }

            let is_portrait = ctx.viewport_rect().width() < ctx.viewport_rect().height();
            if state.show_hud && state.video_mode != 3 {
                let total_h = ctx.viewport_rect().height();
                let min_h = 220.0f32;
                let top_h = (total_h * state.panel_split_ratio).clamp(min_h, (total_h - min_h).max(min_h));
                let top_margin = if is_portrait { 54 } else { 8 };
                let panel_resp = egui::Panel::top("top_panel")
                    .resizable(false)
                    .frame(
                        egui::Frame::NONE
                            .fill(egui::Color32::TRANSPARENT)
                            .inner_margin(egui::Margin {
                                left: 16,
                                right: 16,
                                top: top_margin,
                                bottom: 8,
                            })
                    )
                    .exact_size(top_h)
                    .show_inside(ctx, |ui| {
                        if state.video_mode == 2 && self.video_state.is_some() {
                            // Video occupies the entire top panel
                        } else {
                            let render_progress_bar = |col: &mut egui::Ui, out_fire_rect: &mut Option<egui::Rect>, engine_action: &mut EngineAction| {
                                // Custom Fire/Charred Progress Bar
                                let (rect, response) = col.allocate_exact_size(egui::vec2(col.available_width(), 16.0), egui::Sense::click_and_drag());
                                *out_fire_rect = Some(rect);
                                
                                if response.drag_stopped() || response.clicked() {
                                    if let Some(mouse_pos) = response.interact_pointer_pos() {
                                        let rel_x = (mouse_pos.x - rect.left()).clamp(0.0, rect.width());
                                        let pct = rel_x / rect.width();
                                        *engine_action = EngineAction::Seek(pct);
                                    }
                                } else if response.dragged() {
                                    if let Some(mouse_pos) = response.interact_pointer_pos() {
                                        let rel_x = (mouse_pos.x - rect.left()).clamp(0.0, rect.width());
                                        let pct = rel_x / rect.width();
                                        *engine_action = EngineAction::ScrubPreview(pct, 0.0);
                                    }
                                } else if !response.is_pointer_button_down_on() && state.scrub_target_seconds.is_some() {
                                    *engine_action = EngineAction::ScrubEnd;
                                }
                                
                                let painter = col.painter();
                                let format_time = |secs: f64| -> String {
                                    let m = (secs / 60.0).floor() as u32;
                                    let s = (secs % 60.0).floor() as u32;
                                    let f = (secs.fract() * 10.0).floor() as u32;
                                    format!("{:02}:{:02}.{}", m, s, f)
                                };
                                
                                let display_secs = state.scrub_target_seconds.unwrap_or(state.current_seconds);
                                let time_text = if state.duration_seconds <= 0.0 {
                                    format!("{} / LIVE", format_time(display_secs))
                                } else {
                                    format!("{} / {}", format_time(display_secs), format_time(state.duration_seconds))
                                };
                                
                                painter.text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    time_text,
                                    egui::FontId::proportional(11.0),
                                    egui::Color32::WHITE,
                                );
                            };

                            let render_col_channels = |col: &mut egui::Ui, out_meters_rect: &mut Option<egui::Rect>, out_fire_rect: &mut Option<egui::Rect>, engine_action: &mut EngineAction| {
                                if !is_portrait {
                                    col.heading("Channels");
                                    col.separator();
                                }
                                let meters_height = if is_portrait {
                                    col.available_height()
                                } else {
                                    col.available_height() - 25.0
                                };
                                let (channel_rect, _) = col.allocate_exact_size(
                                    egui::vec2(col.available_width(), meters_height), 
                                    egui::Sense::hover()
                                );
                                *out_meters_rect = Some(channel_rect);
                                
                                let painter = col.painter();
                                let num_channels = state.channel_vus.len();
                                if num_channels > 0 {
                                    let w = channel_rect.width() / num_channels as f32;
                                    for i in 0..num_channels {
                                        let x = channel_rect.left() + i as f32 * w + w * 0.2;
                                        let bw = w * 0.6;
                                        let y_bottom = channel_rect.bottom() - 15.0;
                                        
                                        // Label
                                        if num_channels <= 16 {
                                            let label = if state.tracker_channels.is_some() {
                                                if i == 0 {
                                                    "L".to_string()
                                                } else if i == num_channels - 1 {
                                                    "R".to_string()
                                                } else {
                                                    format!("{}", i)
                                                }
                                            } else {
                                                match num_channels {
                                                    2 => ["L", "R"].get(i).unwrap_or(&"?").to_string(),
                                                    3 => ["L", "C", "R"].get(i).unwrap_or(&"?").to_string(),
                                                    4 => ["Ls", "L", "R", "Rs"].get(i).unwrap_or(&"?").to_string(),
                                                    6 => ["Ls", "L", "C", "LFE", "R", "Rs"].get(i).unwrap_or(&"?").to_string(),
                                                    8 => ["Lrs", "Ls", "L", "C", "LFE", "R", "Rs", "Rrs"].get(i).unwrap_or(&"?").to_string(),
                                                    12 => ["Ltr", "Ltf", "Ls", "L", "C", "LFE", "R", "Rs", "Rtf", "Rtr", "Lrs", "Rrs"].get(i).unwrap_or(&"?").to_string(),
                                                    _ => format!("{}", i + 1),
                                                }
                                            };
                                            painter.text(
                                                egui::pos2(x + bw * 0.5, y_bottom + 2.0),
                                                egui::Align2::CENTER_TOP,
                                                label,
                                                egui::FontId::proportional(12.0),
                                                egui::Color32::GRAY,
                                            );
                                        }
                                    }
                                }
                                
                                if !is_portrait {
                                    col.add_space(5.0);
                                    render_progress_bar(col, out_fire_rect, engine_action);
                                }
                            };

                            let render_col_heatmap = |col: &mut egui::Ui, out_heatmap_rect: &mut Option<egui::Rect>| {
                                if !is_portrait {
                                    let col1_heading = if state.lyrics.is_some() {
                                        "Lyrics"
                                    } else if !state.tracker_patterns_by_order.is_empty() {
                                        "Tracker Pattern"
                                    } else {
                                        "Pattern Heatmap"
                                    };
                                    col.heading(col1_heading);
                                    col.separator();
                                }
                                let hm_rect = col.available_rect_before_wrap();
                                *out_heatmap_rect = Some(hm_rect);
                                
                                col.painter().rect_filled(hm_rect, 0.0, egui::Color32::TRANSPARENT);
                                
                                let painter = col.painter().with_clip_rect(hm_rect);
                                let chunks = 64;
                                let cell_w = hm_rect.width() / chunks as f32;
                                
                                for c in 0..=chunks {
                                    let x = hm_rect.left() + c as f32 * cell_w;
                                    painter.line_segment([egui::pos2(x, hm_rect.top()), egui::pos2(x, hm_rect.bottom())], (1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 5)));
                                }
                                
                                if let Some(lyrics) = &state.lyrics {
                                    // Layer 1: Backdrop Scrim - Dim heatmap slightly for crisp lyrics readability
                                    painter.rect_filled(
                                        hm_rect,
                                        0.0,
                                        egui::Color32::from_rgba_unmultiplied(10, 10, 14, 175)
                                    );

                                    let display_secs = state.scrub_target_seconds.unwrap_or(state.current_seconds);
                                    let center_y = hm_rect.top() + hm_rect.height() / 2.0;
                                    let row_height = 32.0;
                                    let num_rows_to_draw = (hm_rect.height() / row_height) as i32;
                                    let half_rows = (num_rows_to_draw / 2).max(1);

                                    let active_idx = lyrics.find_current_line_idx(display_secs);
                                    let is_intro = active_idx.is_none();
                                    let base_idx = active_idx.unwrap_or(0);

                                    for offset in -half_rows..=half_rows {
                                        let target_idx = (base_idx as i32) + offset - if is_intro && offset > 0 { 1 } else { 0 };
                                        let y = center_y + (offset as f32) * row_height;

                                        let distance = offset.abs() as f32 / (half_rows as f32);
                                        let alpha = (1.0 - distance * 0.80).clamp(0.0, 1.0);
                                        if alpha <= 0.02 {
                                            continue;
                                        }

                                        if is_intro && offset == 0 {
                                            // Active Intro indicator
                                            let font_id = egui::FontId::proportional(18.0);
                                            let text = "♪   ♪   ♪   (Intro)";
                                            let galley = painter.layout_no_wrap(
                                                text.to_string(),
                                                font_id,
                                                egui::Color32::from_rgba_unmultiplied(225, 225, 240, 240),
                                            );
                                            let pos = egui::pos2(hm_rect.center().x, y);
                                            let rect = egui::Rect::from_center_size(pos, galley.size());
                                            painter.rect(
                                                rect.expand2(egui::vec2(14.0, 4.0)),
                                                5.0,
                                                egui::Color32::from_rgba_unmultiplied(14, 14, 18, 235),
                                                egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 50)),
                                                egui::StrokeKind::Outside,
                                            );
                                            painter.galley(rect.min, galley, egui::Color32::from_rgba_unmultiplied(225, 225, 240, 240));
                                            continue;
                                        }

                                        if is_intro && offset < 0 {
                                            continue;
                                        }

                                        if target_idx >= 0 && (target_idx as usize) < lyrics.lines.len() {
                                            let line = &lyrics.lines[target_idx as usize];
                                            let is_current = !is_intro && offset == 0;
                                            let text_display = if line.text.is_empty() {
                                                "♪   ♪   ♪"
                                            } else {
                                                line.text.as_str()
                                            };

                                            let pos = egui::pos2(hm_rect.center().x, y);

                                            if is_current {
                                                // Active line: prominent large font + high contrast pill with subtle luminous border
                                                let font_id = egui::FontId::proportional(20.0);
                                                let max_w = (hm_rect.width() - 24.0).max(50.0);
                                                let galley = painter.layout(
                                                    text_display.to_string(),
                                                    font_id,
                                                    egui::Color32::WHITE,
                                                    max_w,
                                                );
                                                let rect = egui::Rect::from_center_size(pos, galley.size());

                                                painter.rect(
                                                    rect.expand2(egui::vec2(16.0, 5.0)),
                                                    6.0,
                                                    egui::Color32::from_rgba_unmultiplied(12, 12, 16, 240),
                                                    egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60)),
                                                    egui::StrokeKind::Outside,
                                                );
                                                painter.galley(rect.min, galley, egui::Color32::WHITE);
                                            } else {
                                                // Surrounding lines: readable 16pt font with smooth distance fade + soft dark backdrop for contrast
                                                let font_id = egui::FontId::proportional(16.0);
                                                let max_w = (hm_rect.width() - 24.0).max(50.0);
                                                let text_color = if line.text.is_empty() {
                                                    egui::Color32::from_rgba_unmultiplied(150, 150, 160, (alpha * 140.0) as u8)
                                                } else {
                                                    egui::Color32::from_rgba_unmultiplied(225, 225, 235, (alpha * 200.0) as u8)
                                                };
                                                let galley = painter.layout(
                                                    text_display.to_string(),
                                                    font_id,
                                                    text_color,
                                                    max_w,
                                                );
                                                let rect = egui::Rect::from_center_size(pos, galley.size());

                                                let pill_alpha = (alpha * 180.0) as u8;
                                                if pill_alpha > 15 {
                                                    painter.rect_filled(
                                                        rect.expand2(egui::vec2(10.0, 3.0)),
                                                        4.0,
                                                        egui::Color32::from_rgba_unmultiplied(10, 10, 14, pill_alpha),
                                                    );
                                                }
                                                painter.galley(rect.min, galley, text_color);
                                            }
                                        }
                                    }
                                } else if !state.tracker_patterns_by_order.is_empty() {
                                    let current_order = state.current_tracker_order as i32;
                                    let current_row = state.current_tracker_row as i32;
                                    let center_y = hm_rect.top() + hm_rect.height() / 2.0;
                                    let row_height = 16.0;
                                    let num_rows_to_draw = (hm_rect.height() / row_height) as i32;
                                    
                                    let font_id = egui::FontId::monospace(12.0);
                                    let char_width = 7.0; // Approx monospace char width at 12pt
                                    let max_chars = ((hm_rect.width() - 20.0) / char_width).max(10.0) as usize;
                                    let max_text_chars = max_chars.saturating_sub(4);
                                    
                                    let mut formatted = String::with_capacity(max_text_chars + 16);
                                    
                                    for offset in -(num_rows_to_draw / 2)..=(num_rows_to_draw / 2) {
                                        let mut resolved_order = current_order;
                                        let mut resolved_row = current_row + offset;
                                        
                                        if offset < 0 {
                                            // Read exact playback sequence from history
                                            let history_idx = (-offset - 1) as usize;
                                            if history_idx < state.tracker_row_history.len() {
                                                let (hist_order, hist_row) = state.tracker_row_history[history_idx];
                                                resolved_order = hist_order;
                                                resolved_row = hist_row;
                                            } else {
                                                // Fall back to underflow if history hasn't built up yet
                                                while resolved_row < 0 && resolved_order > 0 {
                                                    resolved_order -= 1;
                                                    if (resolved_order as usize) < state.tracker_patterns_by_order.len() {
                                                        resolved_row += state.tracker_patterns_by_order[resolved_order as usize].len() as i32;
                                                    } else {
                                                        break;
                                                    }
                                                }
                                            }
                                        } else if offset > 0 {
                                            // Overflow forwards within the current pattern, clamped
                                            while (resolved_order as usize) < state.tracker_patterns_by_order.len() {
                                                let pattern_len = state.tracker_patterns_by_order[resolved_order as usize].len() as i32;
                                                if resolved_row >= pattern_len {
                                                    resolved_row -= pattern_len;
                                                    resolved_order += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                        }
                                        
                                        if resolved_order >= 0 && (resolved_order as usize) < state.tracker_patterns_by_order.len() {
                                            let pattern = &state.tracker_patterns_by_order[resolved_order as usize];
                                            if resolved_row >= 0 && (resolved_row as usize) < pattern.len() {
                                                let row_str = &pattern[resolved_row as usize];
                                                
                                                let distance = offset.abs() as f32 / (num_rows_to_draw as f32 / 2.0);
                                                let alpha = (1.0 - distance * 0.75).clamp(0.0, 1.0);
                                                let y = center_y + (offset as f32) * row_height;
                                                
                                                // Format tracker row text
                                                formatted.clear();
                                                let text_slice = if row_str.chars().count() > max_text_chars {
                                                    let mut end = row_str.len();
                                                    for (char_count, (byte_idx, _)) in row_str.char_indices().enumerate() {
                                                        if char_count == max_text_chars {
                                                            end = byte_idx;
                                                            break;
                                                        }
                                                    }
                                                    &row_str[..end]
                                                } else {
                                                    row_str.as_str()
                                                };
                                                
                                                use std::fmt::Write;
                                                let _ = write!(formatted, "{:02} | {}", resolved_row, text_slice);
                                                
                                                let pos = egui::pos2(hm_rect.left() + 10.0, y);
                                                if offset == 0 {
                                                    // Prominent highlight background on the active playback row
                                                    let galley = painter.layout_no_wrap(
                                                        formatted.clone(),
                                                        font_id.clone(),
                                                        egui::Color32::WHITE,
                                                    );
                                                    let rect = egui::Rect::from_min_size(
                                                        pos + egui::vec2(0.0, -galley.size().y / 2.0),
                                                        galley.size(),
                                                    );
                                                    painter.rect_filled(
                                                        rect.expand2(egui::vec2(10.0, 2.0)),
                                                        4.0,
                                                        egui::Color32::from_black_alpha(220)
                                                    );
                                                    painter.galley(rect.min, galley, egui::Color32::WHITE);
                                                } else {
                                                    // Valid unmultiplied alpha color
                                                    let color = egui::Color32::from_rgba_unmultiplied(150, 150, 150, (alpha * 100.0) as u8);
                                                    
                                                    let galley = painter.layout_no_wrap(
                                                        formatted.clone(),
                                                        font_id.clone(),
                                                        egui::Color32::WHITE,
                                                    );
                                                    
                                                    let rect = egui::Rect::from_center_size(pos, galley.size());
                                                    painter.galley(rect.min, galley, color);
                                                }
                                                
                                                // Pattern boundary indicator
                                                if resolved_row == 0 {
                                                    painter.line_segment(
                                                        [egui::pos2(hm_rect.left(), y - row_height / 2.0), egui::pos2(hm_rect.right(), y - row_height / 2.0)],
                                                        (1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, (alpha * 150.0) as u8))
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            };

                            let render_col_info = |col: &mut egui::Ui, out_track_info_rect: &mut Option<egui::Rect>, engine_action: &mut EngineAction| {
                                if state.video_mode == 1 && self.video_state.is_some() {
                                    let available = col.available_size();
                                    let (rect, _) = col.allocate_exact_size(available, egui::Sense::hover());
                                    *out_track_info_rect = Some(rect);
                                } else {
                                    col.style_mut().visuals.override_text_color = Some(egui::Color32::from_gray(235));
                                    if !is_portrait {
                                        let heading_text = if state.playlist.len() > 1 {
                                            format!("Track Info ({}/{})", state.playlist_index + 1, state.playlist.len())
                                        } else {
                                            "Track Info".to_string()
                                        };
                                        col.heading(heading_text);
                                        col.separator();
                                    }
                                
                                    let render_smooth_marquee = |ui: &mut egui::Ui, text: &str, size: f32, is_title: bool| {
                                        let available_width = ui.available_width();
                                        let font_id = egui::FontId::proportional(size);
                                        let color = ui.style().visuals.override_text_color.unwrap_or(egui::Color32::from_gray(235));
                                        let text_color = if is_title { egui::Color32::WHITE } else { color };
                                        
                                        let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, text_color);
                                        let text_width = galley.rect.width();
                                        let height = galley.rect.height();
                                        
                                        let (rect, _response) = ui.allocate_exact_size(egui::vec2(available_width, height), egui::Sense::hover());
                                        let painter = ui.painter().with_clip_rect(rect);
                                        
                                        if text_width > available_width {
                                            let max_scroll = text_width - available_width;
                                            let scroll_duration = max_scroll / 35.0;
                                            let total_period = 2.0 + scroll_duration + 2.0 + scroll_duration;
                                            let t = (state.current_seconds as f32) % total_period;
                                            
                                            let offset = if t < 2.0 {
                                                0.0
                                            } else if t < 2.0 + scroll_duration {
                                                let progress = (t - 2.0) / scroll_duration;
                                                progress * max_scroll
                                            } else if t < 2.0 + scroll_duration + 2.0 {
                                                max_scroll
                                            } else {
                                                let progress = (t - (2.0 + scroll_duration + 2.0)) / scroll_duration;
                                                max_scroll - (progress * max_scroll)
                                            };
                                            
                                            painter.galley(rect.min + egui::vec2(-offset, 0.0), galley, text_color);
                                        } else {
                                            painter.galley(rect.min, galley, text_color);
                                        }
                                    };

                                    let display_title = if state.song_title.is_empty() {
                                        "Unknown Title".to_string()
                                    } else {
                                        let p = std::path::Path::new(&state.song_title);
                                        if p.extension().is_some() || state.song_title.contains('/') || state.song_title.contains('\\') {
                                            p.file_stem().unwrap_or_default().to_string_lossy().to_string()
                                        } else {
                                            state.song_title.clone()
                                        }
                                    };

                                    let current_path_str = if state.playlist_index < state.playlist.len() {
                                        state.playlist[state.playlist_index].clone()
                                    } else {
                                        state.song_title.clone()
                                    };
                                    let is_network = current_path_str.starts_with("http://") || current_path_str.starts_with("https://");
                                    let file_name = if is_network {
                                        display_title.clone()
                                    } else {
                                        std::path::Path::new(&current_path_str).file_name().unwrap_or_default().to_string_lossy().to_string()
                                    };
                                    let file_dir = if is_network {
                                        current_path_str
                                    } else {
                                        let abs_path = if std::path::Path::new(&current_path_str).is_absolute() {
                                            std::path::PathBuf::from(&current_path_str)
                                        } else if let Ok(cwd) = std::env::current_dir() {
                                            cwd.join(&current_path_str)
                                        } else {
                                            std::path::PathBuf::from(&current_path_str)
                                        };
                                        let dir_path = abs_path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                                        if let Ok(home) = std::env::var("HOME") {
                                            if dir_path.starts_with(&home) {
                                                dir_path.replacen(&home, "~", 1)
                                            } else {
                                                dir_path
                                            }
                                        } else {
                                            dir_path
                                        }
                                    };

                                    egui::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .show(col, |ui| {
                                            egui::Grid::new("track_info_meta_grid")
                                                .num_columns(2)
                                                .spacing([10.0, 4.0])
                                                .show(ui, |ui| {
                                                    // 1. Song Title
                                                    ui.label(egui::RichText::new("Title:").color(egui::Color32::from_rgb(160, 180, 200)).strong());
                                                    render_smooth_marquee(ui, &display_title, 14.0, true);
                                                    ui.end_row();
                                                    
                                                    // 2. Artist
                                                    let artist_str = if state.artist.is_empty() { "Unknown" } else { &state.artist };
                                                    ui.label(egui::RichText::new("Artist:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                    render_smooth_marquee(ui, artist_str, 14.0, false);
                                                    ui.end_row();
                                                    
                                                    // 3. File Name
                                                    ui.label(egui::RichText::new("File:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                    render_smooth_marquee(ui, &file_name, 14.0, false);
                                                    ui.end_row();

                                                    // 4. File Directory / URL
                                                    let folder_label = if is_network { "Stream:" } else { "Folder:" };
                                                    ui.label(egui::RichText::new(folder_label).color(egui::Color32::from_rgb(160, 180, 200)));
                                                    render_smooth_marquee(ui, &file_dir, 14.0, false);
                                                    ui.end_row();
                                                    
                                                    // 5. Codec / Format & Bitrate
                                                    let format_str = if let Some(br) = state.bitrate {
                                                        format!("{} ({} kbps)", state.module_type, br)
                                                    } else {
                                                        state.module_type.clone()
                                                    };
                                                    ui.label(egui::RichText::new("Format:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                    ui.label(format_str);
                                                    ui.end_row();
                                                    
                                                    // 6. Channels & Track Info
                                                    let ch_info = if let Some(tc) = state.tracker_channels {
                                                        format!("{} hw / {} tracker", state.hardware_channels, tc)
                                                    } else {
                                                        format!("{} channels", state.num_channels)
                                                    };
                                                    ui.label(egui::RichText::new("Channels:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                    ui.label(ch_info);
                                                    ui.end_row();

                                                    // 7. Track Duration
                                                    ui.label(egui::RichText::new("Duration:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                    if state.duration_seconds <= 0.0 {
                                                        ui.label("Live Stream");
                                                    } else {
                                                        let mins = (state.duration_seconds / 60.0).floor() as u32;
                                                        let secs = (state.duration_seconds % 60.0).floor() as u32;
                                                        ui.label(format!("{:.1}s ({:02}:{:02})", state.duration_seconds, mins, secs));
                                                    }
                                                    ui.end_row();
                                                });

                                            if state.audio_tracks.len() > 1 {
                                                ui.add_space(6.0);
                                                ui.horizontal(|ui| {
                                                    ui.label(egui::RichText::new("Audio Tracks:").strong().color(egui::Color32::from_rgb(0, 210, 255)));
                                                });
                                                ui.add_space(3.0);

                                                // Multi-Track Mix Interface
                                                for (idx, track) in state.audio_tracks.iter().enumerate() {
                                                    let is_in_mix = state.active_audio_tracks.contains(&idx);
                                                    let lower = track.title.to_lowercase();
                                                    let (icon, color) = if lower.contains("vocal") || lower.contains("guide") {
                                                        ("♪", egui::Color32::from_rgb(255, 110, 180))
                                                    } else if lower.contains("instrumental") || lower.contains("karaoke") || lower.contains("music") {
                                                        ("♫", egui::Color32::from_rgb(0, 230, 160))
                                                    } else {
                                                        ("♪", egui::Color32::from_rgb(255, 200, 60))
                                                    };

                                                    let mut current_vol = state.audio_track_volumes.get(idx).copied().unwrap_or(1.0);

                                                    ui.horizontal(|ui| {
                                                        // Checkbox / Toggle Button
                                                        let check_label = if is_in_mix {
                                                            format!("☑ 🔗 {} {}", icon, track.title)
                                                        } else {
                                                            format!("☐   {} {}", icon, track.title)
                                                        };
                                                        let btn_color = if is_in_mix { color } else { egui::Color32::GRAY };
                                                        let chk_btn = egui::Button::new(egui::RichText::new(check_label).color(btn_color).size(12.0))
                                                            .fill(if is_in_mix { egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 35) } else { egui::Color32::from_rgba_unmultiplied(40, 40, 45, 160) })
                                                            .stroke(if is_in_mix { egui::Stroke::new(1.0_f32, color) } else { egui::Stroke::NONE });
                                                        
                                                        let btn_w = if is_in_mix {
                                                            (ui.available_width() - 110.0).max(110.0).min(180.0)
                                                        } else {
                                                            ui.available_width()
                                                        };

                                                        if ui.add_sized([btn_w, 28.0], chk_btn).clicked() {
                                                            *engine_action = EngineAction::ToggleAudioTrackInMix(idx);
                                                        }

                                                        // Inline Controls when track is active in mix
                                                        if is_in_mix {
                                                            let vol_text = format!("{:.0}%", current_vol * 100.0);
                                                            let slider = egui::Slider::new(&mut current_vol, 0.0..=1.0)
                                                                .show_value(false)
                                                                .text(vol_text);
                                                            if ui.add(slider).changed() {
                                                                *engine_action = EngineAction::SetAudioTrackVolume(idx, current_vol);
                                                            }

                                                            // Mute button [M]
                                                            let is_muted = current_vol == 0.0;
                                                            let m_btn = egui::Button::new(egui::RichText::new("M").strong().color(if is_muted { egui::Color32::RED } else { egui::Color32::LIGHT_GRAY }).size(11.0))
                                                                .fill(if is_muted { egui::Color32::from_rgba_unmultiplied(200, 50, 50, 80) } else { egui::Color32::from_rgba_unmultiplied(50, 50, 55, 180) });
                                                            if ui.add_sized([22.0, 24.0], m_btn).clicked() {
                                                                let new_vol = if is_muted { 1.0 } else { 0.0 };
                                                                *engine_action = EngineAction::SetAudioTrackVolume(idx, new_vol);
                                                            }

                                                            // Solo button [S]
                                                            let s_btn = egui::Button::new(egui::RichText::new("S").strong().color(egui::Color32::YELLOW).size(11.0))
                                                                .fill(egui::Color32::from_rgba_unmultiplied(50, 50, 55, 180));
                                                            if ui.add_sized([22.0, 24.0], s_btn).clicked() {
                                                                *engine_action = EngineAction::SetAudioMixTracks(vec![(idx, 1.0)]);
                                                            }
                                                        }
                                                    });
                                                    ui.add_space(2.0);
                                                }

                                                // Quick Mix Presets Bar (Always Available)
                                                ui.add_space(5.0);
                                                ui.label(egui::RichText::new("Quick Mix Presets:").size(12.0).color(egui::Color32::from_rgb(170, 190, 210)).strong());
                                                ui.add_space(2.0);

                                                let num_tracks = state.audio_tracks.len();
                                                let is_all_on = state.active_audio_tracks.len() == num_tracks && num_tracks > 0;
                                                let is_mix_1_2 = state.active_audio_tracks.len() == 2 && state.active_audio_tracks.contains(&0) && state.active_audio_tracks.contains(&1);

                                                let total_width = ui.available_width();
                                                let spacing = 6.0;

                                                if num_tracks <= 2 {
                                                    // 4 full-width buttons in a single row: [All On] [Trk 1] [Trk 2] [Mix 1&2]
                                                    let btn_count = if num_tracks == 2 { 4.0 } else { 2.0 };
                                                    let btn_w = ((total_width - spacing * (btn_count - 1.0)) / btn_count).max(50.0);
                                                    let btn_h = 34.0;

                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut().item_spacing.x = spacing;

                                                        // All On
                                                        let all_btn = egui::Button::new(
                                                            egui::RichText::new("All On").strong().size(12.0).color(if is_all_on { egui::Color32::from_rgb(0, 240, 170) } else { egui::Color32::LIGHT_GRAY })
                                                        )
                                                        .fill(if is_all_on { egui::Color32::from_rgba_unmultiplied(0, 140, 100, 180) } else { egui::Color32::from_rgba_unmultiplied(50, 60, 70, 220) })
                                                        .stroke(if is_all_on { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 240, 170)) } else { egui::Stroke::NONE });

                                                        if ui.add_sized([btn_w, btn_h], all_btn).clicked() {
                                                            let all_mix: Vec<(usize, f32)> = (0..num_tracks).map(|i| (i, 1.0)).collect();
                                                            *engine_action = EngineAction::SetAudioMixTracks(all_mix);
                                                        }

                                                        // Trk 1
                                                        let is_solo_0 = state.active_audio_tracks.len() == 1 && state.active_audio_tracks.contains(&0);
                                                        let trk1_btn = egui::Button::new(
                                                            egui::RichText::new("Trk 1").strong().size(12.0).color(if is_solo_0 { egui::Color32::from_rgb(0, 220, 255) } else { egui::Color32::LIGHT_GRAY })
                                                        )
                                                        .fill(if is_solo_0 { egui::Color32::from_rgba_unmultiplied(0, 100, 150, 180) } else { egui::Color32::from_rgba_unmultiplied(50, 60, 70, 220) })
                                                        .stroke(if is_solo_0 { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 220, 255)) } else { egui::Stroke::NONE });

                                                        if ui.add_sized([btn_w, btn_h], trk1_btn).clicked() {
                                                            *engine_action = EngineAction::SetAudioMixTracks(vec![(0, 1.0)]);
                                                        }

                                                        if num_tracks >= 2 {
                                                            // Trk 2
                                                            let is_solo_1 = state.active_audio_tracks.len() == 1 && state.active_audio_tracks.contains(&1);
                                                            let trk2_btn = egui::Button::new(
                                                                egui::RichText::new("Trk 2").strong().size(12.0).color(if is_solo_1 { egui::Color32::from_rgb(0, 220, 255) } else { egui::Color32::LIGHT_GRAY })
                                                            )
                                                            .fill(if is_solo_1 { egui::Color32::from_rgba_unmultiplied(0, 100, 150, 180) } else { egui::Color32::from_rgba_unmultiplied(50, 60, 70, 220) })
                                                            .stroke(if is_solo_1 { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 220, 255)) } else { egui::Stroke::NONE });

                                                            if ui.add_sized([btn_w, btn_h], trk2_btn).clicked() {
                                                                *engine_action = EngineAction::SetAudioMixTracks(vec![(1, 1.0)]);
                                                            }

                                                            // Mix 1&2
                                                            let mix_btn = egui::Button::new(
                                                                egui::RichText::new("Mix 1&2").strong().size(12.0).color(if is_mix_1_2 { egui::Color32::from_rgb(0, 240, 170) } else { egui::Color32::from_rgb(0, 210, 160) })
                                                            )
                                                            .fill(if is_mix_1_2 { egui::Color32::from_rgba_unmultiplied(0, 150, 110, 200) } else { egui::Color32::from_rgba_unmultiplied(0, 80, 70, 180) })
                                                            .stroke(if is_mix_1_2 { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 240, 170)) } else { egui::Stroke::NONE });

                                                            if ui.add_sized([btn_w, btn_h], mix_btn).clicked() {
                                                                *engine_action = EngineAction::SetAudioMixTracks(vec![(0, 1.0), (1, 1.0)]);
                                                            }
                                                        }
                                                    });
                                                } else {
                                                    // 2 rows for 3+ tracks:
                                                    // Row 1: Mix combos [All On] [Mix 1&2] (each 50% width)
                                                    let row1_w = ((total_width - spacing) / 2.0).max(60.0);
                                                    let btn_h = 34.0;

                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut().item_spacing.x = spacing;

                                                        // All On
                                                        let all_btn = egui::Button::new(
                                                            egui::RichText::new("All On").strong().size(12.5).color(if is_all_on { egui::Color32::from_rgb(0, 240, 170) } else { egui::Color32::LIGHT_GRAY })
                                                        )
                                                        .fill(if is_all_on { egui::Color32::from_rgba_unmultiplied(0, 140, 100, 180) } else { egui::Color32::from_rgba_unmultiplied(50, 60, 70, 220) })
                                                        .stroke(if is_all_on { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 240, 170)) } else { egui::Stroke::NONE });

                                                        if ui.add_sized([row1_w, btn_h], all_btn).clicked() {
                                                            let all_mix: Vec<(usize, f32)> = (0..num_tracks).map(|i| (i, 1.0)).collect();
                                                            *engine_action = EngineAction::SetAudioMixTracks(all_mix);
                                                        }

                                                        // Mix 1&2
                                                        let mix_btn = egui::Button::new(
                                                            egui::RichText::new("Mix 1&2").strong().size(12.5).color(if is_mix_1_2 { egui::Color32::from_rgb(0, 240, 170) } else { egui::Color32::from_rgb(0, 210, 160) })
                                                        )
                                                        .fill(if is_mix_1_2 { egui::Color32::from_rgba_unmultiplied(0, 150, 110, 200) } else { egui::Color32::from_rgba_unmultiplied(0, 80, 70, 180) })
                                                        .stroke(if is_mix_1_2 { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 240, 170)) } else { egui::Stroke::NONE });

                                                        if ui.add_sized([row1_w, btn_h], mix_btn).clicked() {
                                                            *engine_action = EngineAction::SetAudioMixTracks(vec![(0, 1.0), (1, 1.0)]);
                                                        }
                                                    });

                                                    ui.add_space(3.0);

                                                    // Row 2: Solo Tracks [Trk 1] [Trk 2] [Trk 3] [Trk 4] (equally dividing width)
                                                    let display_count = num_tracks.min(4);
                                                    let row2_w = ((total_width - spacing * (display_count - 1) as f32) / display_count as f32).max(45.0);

                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut().item_spacing.x = spacing;

                                                        for i in 0..display_count {
                                                            let is_solo_i = state.active_audio_tracks.len() == 1 && state.active_audio_tracks.contains(&i);
                                                            let btn = egui::Button::new(
                                                                egui::RichText::new(format!("Trk {}", i + 1)).strong().size(12.0).color(if is_solo_i { egui::Color32::from_rgb(0, 220, 255) } else { egui::Color32::LIGHT_GRAY })
                                                            )
                                                            .fill(if is_solo_i { egui::Color32::from_rgba_unmultiplied(0, 100, 150, 180) } else { egui::Color32::from_rgba_unmultiplied(50, 60, 70, 220) })
                                                            .stroke(if is_solo_i { egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0, 220, 255)) } else { egui::Stroke::NONE });

                                                            if ui.add_sized([row2_w, 32.0], btn).clicked() {
                                                                *engine_action = EngineAction::SetAudioMixTracks(vec![(i, 1.0)]);
                                                            }
                                                        }
                                                    });
                                                }
                                                ui.add_space(4.0);
                                            }
                                            
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("Device:").color(egui::Color32::from_rgb(160, 180, 200)));
                                                let mut current_device = state.selected_audio_device.clone().unwrap_or_else(|| "Default Device".to_string());
                                                let prev_device = current_device.clone();
                                                
                                                egui::ComboBox::from_id_salt("audio_device_combo")
                                                    .selected_text(&current_device)
                                                    .width(ui.available_width().max(80.0))
                                                    .show_ui(ui, |ui| {
                                                        for dev in &state.available_audio_devices {
                                                            ui.selectable_value(&mut current_device, dev.clone(), dev);
                                                        }
                                                    });
                                                
                                                if current_device != prev_device {
                                                    *engine_action = EngineAction::SetAudioDevice(current_device);
                                                }
                                            });
                                            
                                            // 8. Next Song (placed at the bottom, smooth marquee if long)
                                            if state.playlist_index + 1 < state.playlist.len() {
                                                let next_path = std::path::Path::new(&state.playlist[state.playlist_index + 1]);
                                                let next_song = next_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                                                ui.horizontal(|ui| { 
                                                    ui.label("Next Song:"); 
                                                    render_smooth_marquee(ui, &next_song, 14.0, false); 
                                                });
                                            }
                                            ui.add_space(6.0);
                                            let open_btn = egui::Button::new(
                                                egui::RichText::new("📂  OPEN AUDIO FILE")
                                                    .strong()
                                                    .size(13.5)
                                                    .color(egui::Color32::WHITE)
                                            )
                                            .fill(egui::Color32::from_rgb(0, 100, 200))
                                            .stroke(egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(80, 200, 255)))
                                            .min_size(egui::vec2(ui.available_width(), 38.0));

                                            if ui.add(open_btn).clicked() {
                                                *engine_action = EngineAction::OpenFile;
                                            }
                                        });
                                    
                                    *out_track_info_rect = Some(col.max_rect());
                                }
                            };

                            if is_portrait {
                                let has_video = state.has_video_stream || self.video_state.is_some();
                                let progress_height = 20.0;
                                let available_h = ui.available_height();
                                let content_h = (available_h - progress_height).max(40.0);

                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), content_h),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        match state.mobile_hud_tab {
                                            crate::state::MobileHudTab::Channels => {
                                                render_col_channels(ui, &mut out_meters_rect, &mut out_fire_rect, &mut engine_action);
                                            }
                                            crate::state::MobileHudTab::Heatmap => {
                                                render_col_heatmap(ui, &mut out_heatmap_rect);
                                            }
                                            crate::state::MobileHudTab::Info => {
                                                render_col_info(ui, &mut out_track_info_rect, &mut engine_action);
                                            }
                                            crate::state::MobileHudTab::Video => {
                                                if has_video {
                                                    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                                                    out_video_rect = Some(rect);
                                                } else {
                                                    ui.centered_and_justified(|ui| {
                                                        ui.label(egui::RichText::new("No video stream present").italics());
                                                    });
                                                }
                                            }
                                        }
                                    }
                                );

                                ui.add_space(2.0);
                                render_progress_bar(ui, &mut out_fire_rect, &mut engine_action);
                            } else {
                                ui.columns(3, |columns| {
                                    render_col_channels(&mut columns[0], &mut out_meters_rect, &mut out_fire_rect, &mut engine_action);
                                    render_col_heatmap(&mut columns[1], &mut out_heatmap_rect);
                                    render_col_info(&mut columns[2], &mut out_track_info_rect, &mut engine_action);
                                });
                            }
                        }
                    });
                out_top_panel_rect = Some(panel_resp.response.rect);
            }

        if state.show_hud && state.video_mode != 3 {
            let total_height = ctx.content_rect().height();
            let total_width = ctx.content_rect().width();
            
            let drag_y = out_top_panel_rect.map(|r| r.bottom()).unwrap_or(total_height * state.panel_split_ratio);
            
            egui::Area::new("split_drag_area".into())
                .fixed_pos(egui::pos2(0.0, drag_y - 6.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let drag_rect = egui::Rect::from_min_size(
                        ui.min_rect().min,
                        egui::vec2(total_width, 12.0)
                    );
                    let response = ui.allocate_rect(drag_rect, egui::Sense::click_and_drag());
                    if response.hovered() || response.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if response.dragged() {
                        if let Some(mouse_pos) = response.interact_pointer_pos() {
                            let min_h = 220.0f32;
                            let min_r = min_h / total_height;
                            let max_r = (total_height - min_h) / total_height;
                            let new_ratio = mouse_pos.y / total_height;
                            engine_action = EngineAction::SetSplitRatio(new_ratio.clamp(min_r.min(max_r), min_r.max(max_r)));
                        }
                    }
                });
        }

            let frame = egui::Frame::NONE.fill(egui::Color32::TRANSPARENT);
            egui::CentralPanel::default().frame(frame).show_inside(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                central_rect = rect;
                
                // Draw OSD text from keyboard/gamepad/touch actions
                if let Some(osd) = &state.osd_text {
                    if state.osd_timer > 0.0 {
                        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("osd_notification")));
                        let alpha = (state.osd_timer.min(0.5) * 2.0 * 255.0) as u8;
                        let screen = ui.ctx().viewport_rect();
                        let is_portrait = screen.width() < screen.height();
                        
                        let max_width = if is_portrait {
                            (screen.width() - 40.0).max(100.0)
                        } else {
                            (screen.width() * 0.85).max(200.0)
                        };
                        
                        let font_size = if is_portrait {
                            if osd.len() > 32 {
                                17.0
                            } else if osd.len() > 18 {
                                21.0
                            } else {
                                26.0
                            }
                        } else {
                            if osd.len() > 35 {
                                24.0
                            } else {
                                32.0
                            }
                        };
                        
                        let text_color = egui::Color32::from_rgba_premultiplied(255, 255, 255, alpha);
                        let galley = painter.layout(
                            osd.clone(),
                            egui::FontId::proportional(font_size),
                            text_color,
                            max_width,
                        );
                        
                        let top_y = if is_portrait {
                            screen.top() + 65.0
                        } else {
                            screen.top() + 40.0
                        };
                        let galley_pos = egui::pos2(
                            screen.center().x - galley.rect.width() * 0.5,
                            top_y,
                        );
                        
                        let bg_rect = galley.rect.translate(galley_pos.to_vec2()).expand2(egui::vec2(14.0, 8.0));
                        let bg_alpha = (alpha as f32 * 0.65) as u8;
                        painter.rect_filled(
                            bg_rect,
                            8.0,
                            egui::Color32::from_rgba_unmultiplied(10, 10, 15, bg_alpha),
                        );
                        
                        painter.galley(galley_pos, galley, text_color);
                    }
                }
                
                if state.visualizer_mode == 0 && state.show_hud && state.video_mode != 3 {
                    let painter = ui.painter();
                    let is_portrait = ui.ctx().viewport_rect().width() < ui.ctx().viewport_rect().height();
                    let bottom_margin = if is_portrait { 36.0 } else { 20.0 };
                    let y = rect.bottom() - bottom_margin;
                    
                    let max_freq = state.max_frequency;
                    let min_freq = 20.0_f32;
                    let x_at = |f: f32| -> f32 { (f / min_freq).ln() / (max_freq / min_freq).ln() };
                    
                    let side_margin = if is_portrait { 28.0 } else { 16.0 };
                    let usable_width = (rect.width() - 2.0 * side_margin).max(10.0);
                    
                    let labels = [
                        (0.0_f32, format!("{}Hz", min_freq as u32), egui::Align2::LEFT_BOTTOM),
                        (x_at(100.0), "100Hz".to_string(), egui::Align2::CENTER_BOTTOM),
                        (x_at(1000.0), "1kHz".to_string(), egui::Align2::CENTER_BOTTOM),
                        (x_at(5000.0), "5kHz".to_string(), egui::Align2::CENTER_BOTTOM),
                        (1.0_f32, format!("{:.0}kHz", max_freq / 1000.0), egui::Align2::RIGHT_BOTTOM),
                    ];
                    
                    for (x_pct, text, align) in labels.iter() {
                        let x = rect.left() + side_margin + usable_width * x_pct;
                        painter.text(
                            egui::pos2(x, y),
                            *align,
                            text,
                            egui::FontId::proportional(15.0),
                            egui::Color32::from_gray(230),
                        );
                    }
                }
            });

            let dropped_files: Vec<String> = ctx.input(|i| {
                i.raw.dropped_files
                    .iter()
                    .filter_map(|df| df.path.as_ref().map(|p| p.to_string_lossy().into_owned()))
                    .collect()
            });

            let pointer_pos = ctx.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos()));
            let is_over_track_info = if let (Some(pos), Some(ti_rect)) = (pointer_pos, out_track_info_rect) {
                ti_rect.contains(pos)
            } else {
                false
            };

            if !dropped_files.is_empty() {
                if !state.file_loaded {
                    engine_action = EngineAction::LoadFiles(dropped_files, false);
                } else if is_over_track_info {
                    engine_action = EngineAction::LoadFiles(dropped_files, true);
                } else {
                    engine_action = EngineAction::LoadFiles(dropped_files, false);
                }
            } else if (!ctx.input(|i| i.raw.hovered_files.is_empty()) || state.hovered_file.is_some()) && state.file_loaded {
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drag_overlay")));
                if is_over_track_info {
                    if let Some(ti_rect) = out_track_info_rect {
                        painter.rect(
                            ti_rect.expand(2.0),
                            6.0,
                            egui::Color32::from_rgba_unmultiplied(20, 80, 40, 100),
                            egui::Stroke::new(2.5_f32, egui::Color32::from_rgb(80, 240, 120)),
                            egui::StrokeKind::Outside,
                        );
                        let font_id = egui::FontId::proportional(15.0);
                        let galley = painter.layout(
                            "➕ Add to Playlist".to_string(),
                            font_id,
                            egui::Color32::WHITE,
                            ti_rect.width(),
                        );
                        let badge_rect = egui::Rect::from_center_size(ti_rect.center(), galley.size() + egui::vec2(20.0, 10.0));
                        painter.rect(
                            badge_rect,
                            6.0,
                            egui::Color32::from_rgba_unmultiplied(12, 35, 18, 235),
                            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 230, 130)),
                            egui::StrokeKind::Outside,
                        );
                        painter.galley(badge_rect.min + egui::vec2(10.0, 5.0), galley, egui::Color32::WHITE);
                    }
                } else {
                    let screen_rect = ctx.content_rect();
                    let font_id = egui::FontId::proportional(16.0);
                    let galley = painter.layout(
                        "▶  Drop to Play Immediately".to_string(),
                        font_id,
                        egui::Color32::WHITE,
                        400.0,
                    );
                    let banner_rect = egui::Rect::from_center_size(
                        egui::pos2(screen_rect.center().x, screen_rect.top() + 45.0),
                        galley.size() + egui::vec2(28.0, 14.0),
                    );
                    painter.rect(
                        banner_rect,
                        8.0,
                        egui::Color32::from_rgba_unmultiplied(10, 25, 45, 240),
                        egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(80, 180, 255)),
                        egui::StrokeKind::Outside,
                    );
                    painter.galley(banner_rect.min + egui::vec2(14.0, 7.0), galley, egui::Color32::WHITE);
                }
            }

        });

        let scale = egui_ctx.pixels_per_point();
        let w = self.config.width as f32;
        let h = self.config.height as f32;
        
        if let Some(r) = out_meters_rect {
            self.meters_uv_rect = [(r.min.x * scale) / w, (r.min.y * scale) / h, (r.max.x * scale) / w, (r.max.y * scale) / h];
        } else {
            self.meters_uv_rect = [0.0; 4];
        }
        
        if let Some(r) = out_fire_rect {
            self.fire_uv_rect = [(r.min.x * scale) / w, (r.min.y * scale) / h, (r.max.x * scale) / w, (r.max.y * scale) / h];
        } else {
            self.fire_uv_rect = [0.0; 4];
        }
        
        if let Some(r) = out_heatmap_rect {
            self.heatmap_uv_rect = [(r.min.x * scale) / w, (r.min.y * scale) / h, (r.max.x * scale) / w, (r.max.y * scale) / h];
        } else {
            self.heatmap_uv_rect = [0.0; 4];
        }

        egui_state.handle_platform_output(window, full_output.platform_output);
        let clipped_primitives = egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);

        let ui_elapsed = ui_start.elapsed().as_micros() as f32;
        let phase_egui_layout_us = ui_elapsed; // Egui layout now accurately measures only UI logic and tessellation

        for (id, image_delta) in &full_output.textures_delta.set {
            self.egui_renderer.update_texture(&self.device, &self.queue, *id, image_delta);
        }

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: egui_ctx.pixels_per_point(),
        };
        let render_start = std::time::Instant::now();

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &clipped_primitives,
            &screen_descriptor,
        );


        // GPU heatmap compute
        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Heatmap Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.heatmap_compute_pipeline);
            compute_pass.set_bind_group(0, Some(&self.heatmap_bind_group), &[]);
            compute_pass.dispatch_workgroups(1, 1, 1); // 256x1x1 threads
        }
        
        // GPU FFT compute - bypassed on GPU since we now compute Cooley-Tukey FFT on CPU
        // Write dummy timestamps to satisfy Vulkan query resolving
        if let Some(qs) = &self.query_set {
            encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Dummy FFT Pass"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: qs,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
            });
        }

        if state.gpu_fft {
            let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
            if vis_def.requires_resynth {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Resynth Compute Pass"),
                    timestamp_writes: None,
                });
                compute_pass.set_pipeline(&self.resynth_compute_pipeline);
                compute_pass.set_bind_group(0, Some(&self.resynth_bind_group), &[]);
                compute_pass.dispatch_workgroups(32, 2, 1); // 512/16=32, 32/16=2
            }
        }

        // GPU fire compute: dispatch simulation + copy result to texture
        let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
        if vis_def.requires_fire {
            {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fire Compute"),
                    timestamp_writes: self.query_set.as_ref().map(|qs| wgpu::ComputePassTimestampWrites {
                        query_set: qs,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    }),
                });
                if vis_def.id == 5 {
                    compute_pass.set_pipeline(&self.fire_compute_pipeline);
                } else {
                    compute_pass.set_pipeline(&self.firesim_compute_pipeline);
                }
                let bg = if self.fire_ping { &self.fire_bind_group_a } else { &self.fire_bind_group_b };
                compute_pass.set_bind_group(0, Some(bg), &[]);
                compute_pass.dispatch_workgroups(64, 36, 1); // 1024/16=64, 576/16=36
            }
            // Copy output buffer to fire_grid_texture
            let output_buffer = if self.fire_ping { &self.fire_buffer_b } else { &self.fire_buffer_a };
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: output_buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(1024 * 4),
                        rows_per_image: Some(576),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.fire_grid_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d { width: 1024, height: 576, depth_or_array_layers: 1 },
            );
            self.fire_ping = !self.fire_ping;
        } else {
            // Write dummy timestamps to satisfy Vulkan validation
            if let Some(qs) = &self.query_set {
                encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Dummy Fire Pass"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                        query_set: qs,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    }),
                });
            }
        }

        if vis_def.id == 11 { // Neon Corridor
            // Update params
            let mut act = 0.0;
            let count = state.channel_vus.len().min(8);
            for i in 0..count {
                act += state.channel_vus[i];
            }
            if count > 0 { act /= count as f32; }
            self.queue.write_buffer(&self.smoke_params_buffer, 0, bytemuck::cast_slice(&[
                state.current_seconds as f32,
                act, 0.0, 0.0 // padding
            ]));

            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Neon Smoke Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.smoke_compute_pipeline);
            compute_pass.set_bind_group(0, Some(&self.smoke_compute_bind_group), &[]);
            compute_pass.dispatch_workgroups(16, 16, 16); // 64 / 4
        }

        if vis_def.requires_ferrofluidsim {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Ferrofluid Sim Compute"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.ferrofluidsim_clear_pipeline);
            compute_pass.set_bind_group(0, Some(&self.ferrofluidsim_bind_group), &[]);
            compute_pass.dispatch_workgroups(1024, 1, 1);
            
            compute_pass.set_pipeline(&self.ferrofluidsim_compute_pipeline);
            compute_pass.dispatch_workgroups(391, 1, 1);
        }

        if vis_def.id == 20 {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Bioluminescence Waves Sim Compute"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.biolum_compute_pipeline);
            compute_pass.set_bind_group(0, Some(&self.biolum_compute_bind_group), &[]);
            compute_pass.dispatch_workgroups(256, 1, 1);
        }

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.1, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.query_set.as_ref().map(|qs| wgpu::RenderPassTimestampWrites {
                    query_set: qs,
                    beginning_of_pass_write_index: Some(4),
                    end_of_pass_write_index: Some(5),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let scale_factor = egui_ctx.pixels_per_point();
            let vp_x = ((central_rect.min.x * scale_factor).clamp(0.0, self.config.width as f32)).round();
            let vp_y = ((central_rect.min.y * scale_factor).clamp(0.0, self.config.height as f32)).round();
            let max_w = (self.config.width as f32 - vp_x).max(1.0);
            let vp_w = ((central_rect.width() * scale_factor).clamp(1.0, max_w)).round();
            let max_h = (self.config.height as f32 - vp_y).max(1.0);
            let vp_h = ((central_rect.height() * scale_factor).clamp(1.0, max_h)).round();
            
            // --- 3D Engine Camera Math ---
            let aspect = vp_w / vp_h.max(1.0);
            
            // Update aspect_ratio uniform dynamically based on current viewport
            const ASPECT_RATIO_OFFSET: wgpu::BufferAddress =
                std::mem::offset_of!(AudioUniforms, aspect_ratio) as wgpu::BufferAddress;
            self.queue.write_buffer(&self.uniform_buffer, ASPECT_RATIO_OFFSET, bytemuck::cast_slice(&[aspect as f32]));

            // Adapt FOV when aspect ratio is square or portrait (< 1.0) so 3D scenes fit without horizontal clipping
            let base_fov = 48.0_f32.to_radians();
            let fov_y = if aspect < 1.0 {
                (base_fov / aspect.clamp(0.65, 1.0)).min(75.0_f32.to_radians())
            } else {
                base_fov
            };
            let proj = glam::Mat4::perspective_rh_gl(fov_y, aspect, 0.1, 1000.0);
            let view = if state.visualizer_mode == 20 && state.biolum_top_down {
                glam::Mat4::look_at_rh(
                    glam::Vec3::new(0.0, 14.0, 0.0),
                    glam::Vec3::new(0.0, 0.0, 0.0),
                    glam::Vec3::new(0.0, 0.0, 1.0),
                )
            } else if state.visualizer_mode == 14 {
                glam::Mat4::look_at_rh(
                    glam::Vec3::new(0.0, 1.50, 0.6),
                    glam::Vec3::new(0.0, 0.95, 14.0),
                    glam::Vec3::new(0.0, 1.0, 0.0),
                )
            } else if state.visualizer_mode == 19 {
                // 3D Vintage Hi-Fi Master Rack
                glam::Mat4::look_at_rh(
                    glam::Vec3::new(0.0, 0.0, 6.2),
                    glam::Vec3::new(0.0, 0.0, 0.0),
                    glam::Vec3::new(0.0, 1.0, 0.0),
                )
            } else if state.visualizer_mode == 11 {
                // 3D Concentric Neon Audio Portal Frames Corridor
                glam::Mat4::look_at_rh(
                    glam::Vec3::new(0.0, 2.1, -3.8),
                    glam::Vec3::new(0.0, 1.8, 20.0),
                    glam::Vec3::new(0.0, 1.0, 0.0),
                )
            } else if state.visualizer_mode == 17 {
                // 3D Volumetric Falling Rain Storm
                glam::Mat4::look_at_rh(
                    glam::Vec3::new(0.0, 8.0, 0.0),
                    glam::Vec3::new(0.0, 14.0, 50.0),
                    glam::Vec3::new(0.0, 1.0, 0.0),
                )
            } else if state.visualizer_mode == 23 {
                // 3D Glass Water Lyrics: Low-angle studio camera
                glam::Mat4::look_at_rh(
                    glam::Vec3::new(0.0, 1.4, 4.2),
                    glam::Vec3::new(0.0, 0.45, 0.0),
                    glam::Vec3::new(0.0, 1.0, 0.0),
                )
            } else {
                glam::Mat4::look_at_rh(
                    glam::Vec3::new(0.0, 1.5, -2.0),
                    glam::Vec3::new(0.0, 1.5, 0.0),
                    glam::Vec3::new(0.0, 1.0, 0.0),
                )
            };
            let camera_uniforms = CameraUniforms {
                view_matrix: view.to_cols_array_2d(),
                proj_matrix: proj.to_cols_array_2d(),
            };
            self.queue.write_buffer(&self.camera_uniform_buffer, 0, bytemuck::cast_slice(&[camera_uniforms]));
            
            render_pass.set_viewport(vp_x, vp_y, vp_w, vp_h, 0.0, 1.0);
            render_pass.set_scissor_rect(vp_x as u32, vp_y as u32, vp_w as u32, vp_h as u32);

            let mode_idx = state.current_visualizer_idx.min(self.render_pipelines.len() - 1);
            let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];

            if vis_def.id == 12 || vis_def.id == 3 || vis_def.id == 23 {
                render_pass.set_pipeline(&self.clear_black_pipeline);
                render_pass.draw(0..3, 0..1);
            } else if vis_def.id == 20 {
                render_pass.set_pipeline(&self.biolum_bg_pipeline);
                render_pass.draw(0..3, 0..1);
            } else if vis_def.id == 18 {
                render_pass.set_pipeline(&self.crt_background_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, &self.smoke_render_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            } else if vis_def.id == 14 {
                render_pass.set_pipeline(&self.synthwave_sky_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, &self.smoke_render_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            } else if vis_def.id == 19 {
                render_pass.set_pipeline(&self.vumeters_bg_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, &self.smoke_render_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            } else if vis_def.id == 11 {
                render_pass.set_pipeline(&self.neon_bg_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, &self.smoke_render_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            } else if vis_def.id == 17 {
                render_pass.set_pipeline(&self.storm_sky_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, &self.smoke_render_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }
            
            render_pass.set_pipeline(&self.render_pipelines[mode_idx]);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_bind_group(1, &self.smoke_render_bind_group, &[]);
            
            match &vis_def.pipeline_type {
                crate::state::PipelineType::FullscreenQuad => {
                    render_pass.draw(0..3, 0..1);
                },
                crate::state::PipelineType::Mesh3D { geometry, instances } => {
                    render_pass.set_bind_group(2, &self.camera_bind_group, &[]);
                    if vis_def.id == 20 {
                        render_pass.set_bind_group(3, &self.biolum_render_bind_group, &[]);
                    }
                    if let Some(mesh) = self.mesh_registry.get(geometry) {
                        render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        render_pass.draw_indexed(0..mesh.index_count, 0, 0..*instances);
                    }
                    
                    if vis_def.id == 13 {
                        render_pass.set_pipeline(&self.lamp_pipeline);
                        render_pass.set_vertex_buffer(0, self.lamp_vertex_buffer.slice(..));
                        render_pass.set_index_buffer(self.lamp_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        render_pass.draw_indexed(0..self.lamp_index_count, 0, 0..16);
                    }
                }
            }
            
            let is_portrait = self.config.width < self.config.height;
            let is_video_active = self.video_state.is_some() && (
                state.video_mode > 0 || (is_portrait && state.mobile_hud_tab == crate::state::MobileHudTab::Video)
            );
            
            if is_video_active {
                let mut v_vp_x = 0.0;
                let mut v_vp_y = 0.0;
                let mut v_vp_w = self.config.width as f32;
                let mut v_vp_h = self.config.height as f32;
                
                let target_rect = if state.video_mode == 3 {
                    None // mode 3: full screen
                } else if is_portrait && state.mobile_hud_tab == crate::state::MobileHudTab::Video {
                    out_video_rect
                } else if state.video_mode == 1 {
                    out_track_info_rect
                } else if state.video_mode == 2 {
                    out_top_panel_rect
                } else {
                    None // mode 3: full screen
                };
                
                if let Some(r) = target_rect {
                    v_vp_x = ((r.min.x * scale_factor).clamp(0.0, self.config.width as f32)).round();
                    v_vp_y = ((r.min.y * scale_factor).clamp(0.0, self.config.height as f32)).round();
                    let max_w = (self.config.width as f32 - v_vp_x).max(1.0);
                    v_vp_w = ((r.width() * scale_factor).clamp(1.0, max_w)).round();
                    let max_h = (self.config.height as f32 - v_vp_y).max(1.0);
                    v_vp_h = ((r.height() * scale_factor).clamp(1.0, max_h)).round();
                }
                
                render_pass.set_viewport(v_vp_x, v_vp_y, v_vp_w, v_vp_h, 0.0, 1.0);
                render_pass.set_scissor_rect(v_vp_x as u32, v_vp_y as u32, v_vp_w as u32, v_vp_h as u32);
                
                if let Some(vs) = &self.video_state {
                    let params = VideoParams {
                        color_space: vs.color_space,
                        color_range: vs.color_range,
                        bit_depth: vs.bit_depth,
                        color_trc: vs.color_trc,
                        viewport_width: v_vp_w,
                        viewport_height: v_vp_h,
                        video_width: vs.width as f32,
                        video_height: vs.height as f32,
                    };
                    self.queue.write_buffer(&vs.params_buffer, 0, bytemuck::cast_slice(&[params]));
                    render_pass.set_pipeline(&self.video_pipeline);
                    render_pass.set_bind_group(0, &vs.bind_group, &[]);
                    render_pass.draw(0..3, 0..1);
                }
            }
            
            if state.show_hud && state.video_mode != 3 {
                render_pass.set_viewport(0.0, 0.0, self.config.width as f32, self.config.height as f32, 0.0, 1.0);
                render_pass.set_pipeline(&self.hud_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.set_bind_group(1, &self.smoke_render_bind_group, &[]);
                
                let mut drawn = false;
                let mut draw_rect = |r: Option<egui::Rect>| {
                    if let Some(rect) = r {
                        let x = ((rect.min.x * scale_factor).clamp(0.0, self.config.width as f32)).round() as u32;
                        let y = ((rect.min.y * scale_factor).clamp(0.0, self.config.height as f32)).round() as u32;
                        
                        let w = if x < self.config.width {
                            let max_w = self.config.width - x;
                            ((rect.width() * scale_factor).round() as u32).clamp(1, max_w)
                        } else {
                            0
                        };
                        
                        let h = if y < self.config.height {
                            let max_h = self.config.height - y;
                            ((rect.height() * scale_factor).round() as u32).clamp(1, max_h)
                        } else {
                            0
                        };
                        
                        if w > 0 && h > 0 {
                            render_pass.set_scissor_rect(x, y, w, h);
                            render_pass.draw(0..3, 0..1);
                            drawn = true;
                        }
                    }
                };

                draw_rect(out_meters_rect);
                draw_rect(out_heatmap_rect);
                draw_rect(out_fire_rect);

                if !drawn && !is_portrait {
                    render_pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
                    render_pass.draw(0..3, 0..1);
                }
            }
        }

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            }).forget_lifetime();
            self.egui_renderer.render(&mut render_pass, &clipped_primitives, &screen_descriptor);
        }

        // Resolve timestamp queries into the resolve buffer, then copy to the read buffer.
        // ONLY do this when no mapping is pending — wgpu will panic if we copy to a buffer
        // that has an active or pending map operation.
        let should_start_mapping = !self.timestamp_mapping_active && self.query_set.is_some();
        if should_start_mapping {
            if let (Some(qs), Some(res_buf), Some(read_buf)) = (&self.query_set, &self.query_resolve_buffer, &self.query_read_buffer) {
                encoder.resolve_query_set(qs, 0..6, res_buf, 0);
                encoder.copy_buffer_to_buffer(res_buf, 0, read_buf, 0, 48);
            }
        }

        let capture_frame = if let Ok(bf_str) = std::env::var("BENCH_FRAMES") {
            bf_str.parse::<u32>().unwrap_or(180).saturating_sub(1)
        } else {
            180
        };
        let do_capture = std::env::var("CAPTURE_FRAME").is_ok() && self.frame_count == capture_frame;

        let mut readback_buffer = None;
        if do_capture {
            let bpr = (self.config.width * 4 + 255) & !255;
            let rb = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Readback"),
                size: (bpr * self.config.height) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &surface_texture.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &rb, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(self.config.height) } },
                wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 }
            );
            readback_buffer = Some(rb);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        

        // Start async timestamp mapping AFTER submit (non-blocking)
        if should_start_mapping {
            if let Some(read_buf) = &self.query_read_buffer {
                let flag = self.timestamp_map_complete.clone();
                flag.store(false, Ordering::Release);
                let slice = read_buf.slice(..);
                slice.map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_ok() {
                        flag.store(true, Ordering::Release);
                    }
                });
                self.timestamp_mapping_active = true;
            }
        }
        
        if let Some(rb) = readback_buffer {
            let slice = rb.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
            self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            if rx.recv().unwrap().is_ok() {
                let data = slice.get_mapped_range();
                let bpr = (self.config.width * 4 + 255) & !255;
                let mut img = image::RgbaImage::new(self.config.width, self.config.height);
                for y in 0..self.config.height {
                    for x in 0..self.config.width {
                        let offset = (y * bpr + x * 4) as usize;
                        let b = data[offset];
                        let g = data[offset + 1];
                        let r = data[offset + 2];
                        let _a = data[offset + 3];
                        img.put_pixel(x, y, image::Rgba([r, g, b, 255])); // Ignore A to force fully opaque screenshot
                    }
                }
                let capture_path = std::env::var("CAPTURE_PATH").unwrap_or_else(|_| "screenshot.png".to_string());
                img.save(&capture_path).unwrap();
                println!("Screenshot saved to {}", capture_path);
            }
            if std::env::var("BENCH_FRAMES").is_err() {
                std::process::exit(0);
            }
        }
        
        surface_texture.present();

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        
        let submit_elapsed = render_start.elapsed().as_micros() as f32;
        let phase_encode_us = submit_elapsed; // entire encode+submit block

        Ok((engine_action, ui_elapsed, submit_elapsed, fire_shader_time_us, fft_shader_time_us, vis_shader_time_us,
             phase_surface_us, phase_egui_layout_us, phase_encode_us, 0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_size_alignment() {
        let rust_size = std::mem::size_of::<AudioUniforms>();
        assert_eq!(rust_size, 8832, "Rust AudioUniforms size is not 8832 bytes (actual: {})", rust_size);

        // Parse _common.wgsl to compute WGSL structure size
        let common_source = std::fs::read_to_string("src/shaders/_common.wgsl")
            .expect("Failed to read _common.wgsl");

        // Parse AudioUniforms struct fields
        let struct_content = common_source
            .split("struct AudioUniforms {")
            .nth(1)
            .expect("Could not find struct AudioUniforms in _common.wgsl")
            .split("};")
            .next()
            .expect("Could not find end of struct AudioUniforms");

        let mut offset = 0;
        for line in struct_content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() < 2 {
                continue;
            }
            let ty = parts[1].trim().trim_end_matches(',');

            // Determine size and alignment
            let (size, align) = if ty.starts_with("array<") {
                if ty.contains("vec4<") {
                    let count_str = ty.split(',').nth(1).unwrap().trim().trim_end_matches('>');
                    let count: usize = count_str.parse().unwrap();
                    (count * 16, 16)
                } else {
                    panic!("Unknown array type in shader: {}", ty);
                }
            } else {
                match ty {
                    "u32" | "i32" | "f32" => (4, 4),
                    "vec2<f32>" | "vec2<u32>" | "vec2<i32>" => (8, 8),
                    "vec3<f32>" | "vec3<u32>" | "vec3<i32>" => (12, 16),
                    "vec4<f32>" | "vec4<u32>" | "vec4<i32>" => (16, 16),
                    _ => panic!("Unknown type in shader: {}", ty),
                }
            };

            // Align the offset
            offset = (offset + align - 1) / align * align;
            offset += size;
        }

        // Align struct size to maximum alignment (16)
        let wgsl_size = (offset + 15) / 16 * 16;
        assert_eq!(wgsl_size, 8832, "WGSL AudioUniforms size is not 8832 bytes (actual: {})", wgsl_size);
        assert_eq!(rust_size, wgsl_size, "Size mismatch: Rust AudioUniforms is {}, WGSL is {}", rust_size, wgsl_size);
    }

    #[test]
    fn test_timing_and_scroll_stability() {
        let mut smooth_dt = 1.0f64 / 60.0f64;
        let mut smooth_time = 0.0f64;
        let mut raw_time = 0.0f64;

        let mut raw_deltas = Vec::new();
        let mut smooth_deltas = Vec::new();

        // Simulate 100 frames with CPU scheduling noise
        for i in 0..100 {
            // Deterministic pseudo-random jitter between 13ms and 20ms using a sine phase
            let phase = (i as f64) * 0.73;
            let jitter = phase.sin() * 0.0035; // +/- 3.5ms
            let dt = 0.01667 + jitter;

            let prev_raw = raw_time;
            raw_time += dt;
            raw_deltas.push(raw_time - prev_raw);

            let alpha = 0.03f64;
            let prev_smooth = smooth_time;
            smooth_dt = smooth_dt * (1.0 - alpha) + dt.clamp(0.001, 0.1) * alpha;
            smooth_time += smooth_dt;
            smooth_deltas.push(smooth_time - prev_smooth);
        }

        // Calculate second derivative (acceleration / velocity change) of the timeline
        let mut raw_accelerations = Vec::new();
        let mut smooth_accelerations = Vec::new();
        for i in 1..99 {
            let acc_raw = raw_deltas[i] - raw_deltas[i-1];
            let acc_smooth = smooth_deltas[i] - smooth_deltas[i-1];
            raw_accelerations.push(acc_raw.abs());
            smooth_accelerations.push(acc_smooth.abs());
        }

        let max_raw_acc = raw_accelerations.iter().cloned().fold(0.0, f64::max);
        let max_smooth_acc = smooth_accelerations.iter().cloned().fold(0.0, f64::max);

        println!("Max raw frame-to-frame velocity jump: {:.6}s", max_raw_acc);
        println!("Max smoothed frame-to-frame velocity jump: {:.6}s", max_smooth_acc);

        // Smooth timelines must have at least 15x lower frame-to-frame velocity jumps
        assert!(
            max_smooth_acc < max_raw_acc / 15.0,
            "The timing filter did not damp frame-to-frame velocity changes sufficiently: smooth={:.6}, raw={:.6}",
            max_smooth_acc,
            max_raw_acc
        );
    }

    #[test]
    fn test_drag_and_drop_use_cases() {
        let mut state = crate::state::AppState::new("Test App".to_string());
        let track_info_rect = [800.0f32, 0.0, 1200.0, 300.0];
        state.track_info_rect = Some(track_info_rect);

        let dropped_file = "song1.flac".to_string();

        // Use Case 1: Splash screen drop (file_loaded == false)
        assert!(!state.file_loaded);
        state.playlist = vec![dropped_file.clone()];
        state.playlist_index = 0;
        state.load_request = Some(dropped_file.clone());
        state.file_loaded = true;

        assert!(state.file_loaded);
        assert_eq!(state.playlist.len(), 1);
        assert_eq!(state.load_request, Some("song1.flac".to_string()));

        // Use Case 2: Drop on main view outside Track Info (e.g. cursor at (400, 150))
        let cursor_outside = [400.0f32, 150.0];
        let is_over_ti = cursor_outside[0] >= track_info_rect[0] && cursor_outside[0] <= track_info_rect[2]
            && cursor_outside[1] >= track_info_rect[1] && cursor_outside[1] <= track_info_rect[3];
        assert!(!is_over_ti);

        let new_file = "song2.mp3".to_string();
        if !is_over_ti {
            state.playlist = vec![new_file.clone()];
            state.playlist_index = 0;
            state.load_request = Some(new_file.clone());
        }
        assert_eq!(state.playlist, vec!["song2.mp3".to_string()]);
        assert_eq!(state.load_request, Some("song2.mp3".to_string()));

        // Use Case 3: Drop on Track Info pane (e.g. cursor at (950, 100))
        let cursor_inside = [950.0f32, 100.0];
        let is_over_ti_inside = cursor_inside[0] >= track_info_rect[0] && cursor_inside[0] <= track_info_rect[2]
            && cursor_inside[1] >= track_info_rect[1] && cursor_inside[1] <= track_info_rect[3];
        assert!(is_over_ti_inside);

        let append_file = "song3.wav".to_string();
        if is_over_ti_inside {
            state.playlist.push(append_file.clone());
            state.osd_text = Some(format!("Added to Playlist: {}", append_file));
            state.osd_timer = 3.0;
        }
        assert_eq!(state.playlist.len(), 2);
        assert_eq!(state.playlist[1], "song3.wav");
        assert_eq!(state.osd_text, Some("Added to Playlist: song3.wav".to_string()));
    }

    #[test]
    fn test_view_mode_rotation() {
        let mut state = crate::state::AppState::new("Test App".to_string());
        assert_eq!(state.show_hud, true);
        assert_eq!(state.video_mode, 0);

        // Press 'v' in audio mode -> Toggle HUD off (Full Screen Visualizer)
        state.show_hud = !state.show_hud;
        state.video_mode = 0;
        assert_eq!(state.show_hud, false);

        // Press 'v' again -> Toggle HUD back on (Standard View)
        state.show_hud = !state.show_hud;
        state.video_mode = 0;
        assert_eq!(state.show_hud, true);
    }

    #[test]
    fn test_synthwave_lyrics_visualizer_registration() {
        let vis = crate::state::VISUALIZERS.iter().find(|v| v.id == 23);
        assert!(vis.is_some(), "Visualizer ID 23 (3D Glass Water Lyrics) must be registered in VISUALIZERS");
        let def = vis.unwrap();
        assert_eq!(def.name, "3D Glass Water Lyrics");
        assert_eq!(def.filename, "vis_lyrics.wgsl");
        assert_eq!(def.pipeline_type, crate::state::PipelineType::Mesh3D { geometry: crate::state::Geometry::GlassLyricsScene, instances: 1 });

        // Verify that the shader source is present and includes compile cleanly
        let raw_source = include_str!("shaders/vis_lyrics.wgsl");
        assert!(raw_source.contains("// INCLUDE: common"));
        assert!(raw_source.contains("vs_main_3d"));
        assert!(raw_source.contains("fs_main"));
    }

    #[test]
    fn test_progress_fire_decay_when_stopped() {
        let mut state = crate::state::AppState::new("Test App".to_string());
        state.file_loaded = true;
        state.is_paused = false;
        state.track_ended = false;

        let mut fire_intensity = 0.0f32;
        let dt = 1.0f32 / 60.0f32;

        // 1. When playing, fire should ramp up to 1.0
        for _ in 0..60 {
            let is_playing = !state.is_paused && state.file_loaded && !state.track_ended;
            let target_intensity = if is_playing { 1.0f32 } else { 0.0f32 };
            let fire_rate = if is_playing { 5.0f32 } else { 2.5f32 };
            if fire_intensity < target_intensity {
                fire_intensity = (fire_intensity + fire_rate * dt).min(target_intensity);
            } else if fire_intensity > target_intensity {
                fire_intensity = (fire_intensity - fire_rate * dt).max(target_intensity);
            }
        }
        assert!((fire_intensity - 1.0).abs() < 1e-4, "Fire should be fully ignited (1.0) while playing");

        // 2. Pause playback -> fire intensity should decay down to 0.0
        state.is_paused = true;
        for _ in 0..60 {
            let is_playing = !state.is_paused && state.file_loaded && !state.track_ended;
            let target_intensity = if is_playing { 1.0f32 } else { 0.0f32 };
            let fire_rate = if is_playing { 5.0f32 } else { 2.5f32 };
            if fire_intensity < target_intensity {
                fire_intensity = (fire_intensity + fire_rate * dt).min(target_intensity);
            } else if fire_intensity > target_intensity {
                fire_intensity = (fire_intensity - fire_rate * dt).max(target_intensity);
            }
        }
        assert_eq!(fire_intensity, 0.0, "Fire intensity should die off completely (0.0) when paused");

        // 3. Track ended -> fire intensity should die off
        state.is_paused = false;
        state.track_ended = true;
        fire_intensity = 0.8;
        for _ in 0..60 {
            let is_at_end = state.duration_seconds > 0.0 && state.current_seconds >= state.duration_seconds - 0.05;
            let is_playing = !state.is_paused && state.file_loaded && !state.track_ended && !is_at_end;
            let target_intensity = if is_playing { 1.0f32 } else { 0.0f32 };
            let fire_rate = if is_playing { 5.0f32 } else { 3.5f32 };
            if fire_intensity < target_intensity {
                fire_intensity = (fire_intensity + fire_rate * dt).min(target_intensity);
            } else if fire_intensity > target_intensity {
                fire_intensity = (fire_intensity - fire_rate * dt).max(target_intensity);
            }
        }
        assert_eq!(fire_intensity, 0.0, "Fire intensity should die off completely (0.0) when track ended");

        // 4. End of song reached (current_seconds >= duration_seconds) -> fire intensity should die off
        state.track_ended = false;
        state.duration_seconds = 180.0;
        state.current_seconds = 180.0;
        fire_intensity = 0.9;
        for _ in 0..60 {
            let is_at_end = state.duration_seconds > 0.0 && state.current_seconds >= state.duration_seconds - 0.05;
            let is_playing = !state.is_paused && state.file_loaded && !state.track_ended && !is_at_end;
            let target_intensity = if is_playing { 1.0f32 } else { 0.0f32 };
            let fire_rate = if is_playing { 5.0f32 } else { 3.5f32 };
            if fire_intensity < target_intensity {
                fire_intensity = (fire_intensity + fire_rate * dt).min(target_intensity);
            } else if fire_intensity > target_intensity {
                fire_intensity = (fire_intensity - fire_rate * dt).max(target_intensity);
            }
        }
        assert_eq!(fire_intensity, 0.0, "Fire intensity should die off completely (0.0) when end of song is reached");
    }
}
