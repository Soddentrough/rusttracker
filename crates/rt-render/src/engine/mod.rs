use crate::state::AppState;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use winit::window::Window;

mod fire;
mod flip_disc;
mod meshes;
mod splash;
mod types;
mod ui_overlay;

pub use flip_disc::*;
pub use meshes::{
    generate_biolum_ocean_mesh, generate_glass_lyrics_mesh, generate_lamp_mesh,
    generate_neon_room_mesh, generate_synthwave_racer_mesh,
};
pub(crate) use meshes::generate_vumeter_rack_mesh;
pub(crate) use splash::{SplashCache, StatsDisplayCache};
pub use types::*;

pub struct VulkanEngine {
    instance: wgpu::Instance,
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    pub negotiated_features: NegotiatedGpuFeatures,
    pub config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    render_pipelines: Vec<Option<wgpu::RenderPipeline>>,
    shader_sources: Vec<String>,
    fallback_pipeline: wgpu::RenderPipeline,
    fallback_pipeline_3d: wgpu::RenderPipeline,
    render_pipeline_layout: wgpu::PipelineLayout,
    render_pipeline_layout_3d: wgpu::PipelineLayout,
    lyrics_render_pipeline_layout_3d: wgpu::PipelineLayout,
    pipeline_cache: Option<wgpu::PipelineCache>,
    pipeline_cache_path: Option<std::path::PathBuf>,
    hud_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    waveform_storage_buffer: wgpu::Buffer,
    _history_texture: wgpu::Texture,
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
    pub track_info_rect: Option<[f32; 4]>,

    // GPU compute fire simulation
    firesim_compute_pipeline: wgpu::ComputePipeline,
    fire_buffer_a: wgpu::Buffer,
    fire_buffer_b: wgpu::Buffer,
    _fire_coal_buffer: wgpu::Buffer,
    fire_params_buffer: wgpu::Buffer,
    fire_bind_group_a: wgpu::BindGroup, // reads A, writes B
    fire_bind_group_b: wgpu::BindGroup, // reads B, writes A
    fire_ping: bool,

    // Authentic DOOM Fire simulation state (visualizer ID 5: Retro Fire)
    retro_fire_cells: Box<[u8; 320 * 180]>,
    retro_fire_upload: Vec<f32>,
    retro_fire_rng: u32,
    retro_fire_accum: f32,
    retro_fire_cold_uploaded: bool,

    pub heatmap_row: u32,
    heatmap_compute_pipeline: wgpu::ComputePipeline,
    heatmap_bind_group: wgpu::BindGroup,

    // GPU FLIP Acoustic Water simulation (shared FLIP buffers & water compute/render pipelines)
    flip_compute_bind_group: wgpu::BindGroup,
    flip_render_bind_group: wgpu::BindGroup,
    flip_water_clear_pipeline: wgpu::ComputePipeline,
    flip_water_clear_render_pipeline: wgpu::ComputePipeline,
    flip_water_p2g_pipeline: wgpu::ComputePipeline,
    flip_water_forces_pipeline: wgpu::ComputePipeline,
    flip_water_pressure_0_to_1_pipeline: wgpu::ComputePipeline,
    flip_water_pressure_1_to_0_pipeline: wgpu::ComputePipeline,
    flip_water_project_pipeline: wgpu::ComputePipeline,
    flip_water_g2p_advect_pipeline: wgpu::ComputePipeline,
    water_splash_pipeline: wgpu::RenderPipeline,
    splash_quad_vertex_buffer: wgpu::Buffer,
    splash_quad_index_buffer: wgpu::Buffer,
    splash_compact_buffer: wgpu::Buffer,
    splash_indirect_args_buffer: wgpu::Buffer,
    lyrics_indirect_args_buffer: wgpu::Buffer,

    // GPU compute bioluminescent waves simulation
    _biolum_particles_buffer: wgpu::Buffer,
    biolum_compute_pipeline: wgpu::ComputePipeline,
    biolum_compute_bind_group: wgpu::BindGroup,
    biolum_render_bind_group: wgpu::BindGroup,
    _biolum_render_pipeline_layout: wgpu::PipelineLayout,
    biolum_ocean_vertex_buffer: wgpu::Buffer,
    biolum_ocean_index_buffer: wgpu::Buffer,
    biolum_ocean_index_count: u32,
    biolum_initialized: bool,

    // GPU FFT was removed (FFT is computed on the CPU); the dummy timestamp
    // pass in render() keeps the query-slot layout intact.
    resynth_compute_pipeline: wgpu::ComputePipeline,
    resynth_bind_group: wgpu::BindGroup,
    _gpu_spectrum_buffer: wgpu::Buffer,

    acoustic_wave_clear_pipeline: wgpu::ComputePipeline,
    acoustic_wave_step_ab_pipeline: wgpu::ComputePipeline,
    acoustic_wave_step_ba_pipeline: wgpu::ComputePipeline,
    acoustic_wave_compute_bind_group: wgpu::BindGroup,
    _acoustic_wave_a_buffer: wgpu::Buffer,
    _acoustic_wave_b_buffer: wgpu::Buffer,
    acoustic_packed_buffer: wgpu::Buffer,
    acoustic_volume_texture: wgpu::Texture,
    neon_acoustic_render_pipeline_layout_3d: wgpu::PipelineLayout,
    acoustic_render_bind_group: wgpu::BindGroup,
    acoustic_wave_initialized: bool,

    // Mechanical Flip-Disc compute & render state (visualizer ID 32)
    flip_disc_params_buffer: wgpu::Buffer,
    _flip_disc_wave_a_buffer: wgpu::Buffer,
    _flip_disc_wave_b_buffer: wgpu::Buffer,
    _flip_disc_state_buffer: wgpu::Buffer,
    flip_disc_compute_bg_a: wgpu::BindGroup,
    flip_disc_compute_bg_b: wgpu::BindGroup,
    flip_disc_init_pipeline: wgpu::ComputePipeline,
    flip_disc_wave_pipeline: wgpu::ComputePipeline,
    flip_disc_step_pipeline: wgpu::ComputePipeline,
    flip_disc_render_pipeline_layout: wgpu::PipelineLayout,
    flip_disc_render_bind_group: wgpu::BindGroup,
    flip_disc_ping: bool,
    flip_disc_initialized: bool,
    flip_disc_sim: FlipDiscSimState,
    flip_disc_pending_substeps: u32,

    depth_texture_view: wgpu::TextureView,

    // Pre-allocated buffers to avoid per-frame heap allocations
    waveform_history_flat: Vec<f32>,
    waveform_history_packed: Vec<u32>,
    tape_timeline_scratch: Vec<f32>,

    // Video
    video_blur_bind_group_layout: wgpu::BindGroupLayout,
    video_bind_group_layout: wgpu::BindGroupLayout,
    video_border_blur_pipeline: wgpu::RenderPipeline,
    video_pipeline: wgpu::RenderPipeline,
    video_state: Option<VideoState>,
    video_repack_scratch: Vec<u8>,
    clear_black_pipeline: wgpu::RenderPipeline,
    biolum_bg_pipeline: Option<wgpu::RenderPipeline>,
    biolum_ocean_pipeline: Option<wgpu::RenderPipeline>,
    synthwave_sky_pipeline: wgpu::RenderPipeline,
    vumeters_bg_pipeline: wgpu::RenderPipeline,

    pending_uniforms: Option<AudioUniforms>,
    cached_meta_path: String,
    cached_file_dir: String,
    cached_file_len: Option<u64>,
    last_acquire_instant: Option<std::time::Instant>,

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
    smooth_audio_seconds: f64,
    last_raw_state_seconds: f64,
    pub smooth_dt: f64,
    last_history_push_count: u64,
    time_since_last_push: f64,
    measured_push_interval: f64,
    vu_needle_angles: Vec<f32>,
    vu_needle_velocities: Vec<f32>,
    channel_phases: [f32; 32],
    neon_spatial_env: [f32; 16],
    vu_prev_active_segments: [u32; 32],
    vu_segment_glows: [[f32; 30]; 32],
    vu_prev_num_channels: u32,
    // Change detection & GPU ring-buffer state for waveform history uploads
    last_uploaded_push_count: u64,
    last_uploaded_vis_width: u32,
    last_uploaded_vis_id: u32,
    last_uploaded_hist_len: usize,
    waveform_ring_head: usize,
    waveform_ring_base: u32,
    pub lyric_slam_timer: f64,
    pub last_lyric_line_idx: Option<usize>,
    pub current_lyric_mesh_text: String,
    pub fire_intensity: f32,
    pub(crate) splash_cache: SplashCache,
    pub(crate) stats_cache: StatsDisplayCache,
    pub(crate) is_game_mode: bool,
    pub(crate) last_steps_to_fill: u32,
    pub(crate) last_aspect_ratio: f32,
}

impl VulkanEngine {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        // On Windows 11, default to the native DX12 backend (DXGI_SWAP_EFFECT_FLIP_DISCARD +
        // DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT + SetMaximumFrameLatency(2)) over
        // Vulkan's VK_KHR_win32_surface unless WGPU_BACKEND is explicitly overridden.
        // AMDVLK's windowed Vulkan surface does not pace to DWM's waitable VBlank/VRR flip clock
        // when a background game is driving VRR, causing unpaced >144 FPS presentation and flicker.
        let explicit_backend = wgpu::Backends::from_env();
        let preferred_backends = explicit_backend.unwrap_or(if cfg!(target_os = "windows") {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::PRIMARY
        });

        let mut instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: preferred_backends,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            display: None,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });

        let mut surface = instance.create_surface(window.clone()).unwrap();

        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
        {
            Ok(a) => a,
            Err(_) if explicit_backend.is_none() && cfg!(target_os = "windows") => {
                instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::PRIMARY,
                    flags: wgpu::InstanceFlags::default(),
                    backend_options: wgpu::BackendOptions::from_env_or_default(),
                    display: None,
                    memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                });
                surface = instance.create_surface(window.clone()).unwrap();
                instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: Some(&surface),
                        force_fallback_adapter: false,
                        apply_limit_buckets: false,
                    })
                    .await
                    .unwrap()
            }
            Err(e) => panic!("Failed to request GPU adapter: {:?}", e),
        };
        let adapter_info = adapter.get_info();

        let is_bench =
            crate::state::BENCH_FRAMES_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) > 0;
        let want_timestamps = if cfg!(target_os = "android") {
            false
        } else {
            is_bench
                || std::env::var("RUSTTRACKER_PROFILE")
                    .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                    .unwrap_or(true)
        };
        let pipeline_cache_enabled = std::env::var("RUSTTRACKER_PIPELINE_CACHE")
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let negotiated_features = negotiate_optional_gpu_features(
            adapter.features(),
            want_timestamps,
            pipeline_cache_enabled,
        );
        let supports_timestamps = negotiated_features.timestamp_query;
        let required_features = negotiated_features.required_features;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            })
            .await
            .unwrap();

        device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| {
            eprintln!("WGPU VALIDATION ERROR: {:?}", e);
        }));

        // --- Pipeline Cache (persist compiled GPU pipelines across launches) ---
        // Keyed by the adapter so caches from different devices don't collide.
        let pipeline_cache_path: Option<std::path::PathBuf> = (|| {
            let key = wgpu::util::pipeline_cache_key(&adapter.get_info())?;
            let dir = crate::state::CACHE_DIR_OVERRIDE
                .get()
                .cloned()
                .or_else(|| {
                    std::env::var("RUSTTRACKER_CACHE_DIR")
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .map(std::path::PathBuf::from)
                })
                .or_else(|| {
                    directories::ProjectDirs::from("com", "RustTracker", "RustTracker")
                        .map(|p| p.cache_dir().to_path_buf())
                })?;
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
        // Pipeline caching is enabled by default to eliminate shader recompilation hitches,
        // with opt-out via RUSTTRACKER_PIPELINE_CACHE=0.
        let pipeline_cache = if pipeline_cache_enabled
            && device.features().contains(wgpu::Features::PIPELINE_CACHE)
        {
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
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Default to strict hardware VSync (PresentMode::Fifo / VK_PRESENT_MODE_FIFO_KHR)
        // across all platforms unless uncapped mode or an explicit override is requested.
        // Avoid defaulting to FifoRelaxed on desktop: in Windows windowed mode under DWM/MPO,
        // FifoRelaxed tears/flickers whenever a frame arrives slightly after VBlank.
        let present_mode = if crate::state::UNCAPPED_PRESENT_MODE.load(Ordering::Relaxed) {
            wgpu::PresentMode::AutoNoVsync
        } else if let Ok(mode_str) = std::env::var("RUSTTRACKER_PRESENT_MODE") {
            match mode_str.trim().to_ascii_lowercase().as_str() {
                "novsync" | "immediate" => wgpu::PresentMode::AutoNoVsync,
                "mailbox"
                    if surface_caps
                        .present_modes
                        .contains(&wgpu::PresentMode::Mailbox) =>
                {
                    wgpu::PresentMode::Mailbox
                }
                "fiforelaxed" | "adaptive"
                    if surface_caps
                        .present_modes
                        .contains(&wgpu::PresentMode::FifoRelaxed) =>
                {
                    wgpu::PresentMode::FifoRelaxed
                }
                "fifo" => wgpu::PresentMode::Fifo,
                _ => wgpu::PresentMode::Fifo,
            }
        } else {
            wgpu::PresentMode::Fifo
        };

        let alpha_mode = surface_caps
            .alpha_modes
            .iter()
            .copied()
            .find(|&m| m == wgpu::CompositeAlphaMode::Opaque)
            .unwrap_or(surface_caps.alpha_modes[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
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
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let waveform_storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Waveform History Storage (f16 Packed)"),
            size: (1024 * 144 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let history_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Heatmap History Texture"),
            size: wgpu::Extent3d {
                width: 256,
                height: 1024,
                depth_or_array_layers: 1,
            },
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
            size: wgpu::Extent3d {
                width: 1024,
                height: 576,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let fire_grid_view = fire_grid_texture.create_view(&wgpu::TextureViewDescriptor::default());

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
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
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
            ],
            label: Some("audio_bind_group"),
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let render_pipeline_layout_3d =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("3D Render Pipeline Layout"),
                bind_group_layouts: &[Some(&bind_group_layout), Some(&camera_bind_group_layout)],
                immediate_size: 0,
            });

        let biolum_render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Biolum Render Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let biolum_render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Biolum Render Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&bind_group_layout),
                    Some(&camera_bind_group_layout),
                    Some(&biolum_render_bind_group_layout),
                ],
                immediate_size: 0,
            });

        let flip_render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("FLIP Render Layout"),
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
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let lyrics_render_pipeline_layout_3d =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Lyrics 3D Render Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&bind_group_layout),
                    Some(&flip_render_bind_group_layout),
                    Some(&camera_bind_group_layout),
                ],
                immediate_size: 0,
            });

        let acoustic_render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Acoustic Wave Volume Render Layout"),
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

        let neon_acoustic_render_pipeline_layout_3d =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Neon Acoustic Studio 3D Render Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&bind_group_layout),
                    Some(&camera_bind_group_layout),
                    Some(&acoustic_render_bind_group_layout),
                ],
                immediate_size: 0,
            });

        let flip_disc_render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flip-Disc Render Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let flip_disc_render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Flip-Disc Render Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&bind_group_layout),
                    Some(&flip_disc_render_bind_group_layout),
                ],
                immediate_size: 0,
            });

        // Shared shader headers — included at compile time, resolved via simple string replacement.
        // This is the single source of truth for AudioUniforms layout and glyph font.
        const SHADER_COMMON: &str = include_str!("../../../../src/shaders/_common.wgsl");
        const SHADER_GLYPH_FONT: &str = include_str!("../../../../src/shaders/_glyph_font.wgsl");

        let resolve_shader_includes = |source: &str| -> String {
            source
                .replace("// INCLUDE: common", SHADER_COMMON)
                .replace("// INCLUDE: glyph_font", SHADER_GLYPH_FONT)
        };

        let get_shader_source = |id: u32| -> &'static str {
            match id {
                0 => include_str!("../../../../src/shaders/vis_spectrum.wgsl"),
                1 => include_str!("../../../../src/shaders/vis_oscilloscope.wgsl"),
                3 => include_str!("../../../../src/shaders/vis_3doscilloscope_raster.wgsl"),
                4 => include_str!("../../../../src/shaders/vis_3doscilloscope_freq.wgsl"),
                5 => include_str!("../../../../src/shaders/vis_flame.wgsl"),
                6 => include_str!("../../../../src/shaders/vis_firesim.wgsl"),
                7 => include_str!("../../../../src/shaders/vis_solar.wgsl"),
                13 => include_str!("../../../../src/shaders/vis_synthwave.wgsl"),
                14 => include_str!("../../../../src/shaders/vis_synthwave_racer_3d.wgsl"),
                16 => include_str!("../../../../src/shaders/vis_rain.wgsl"),
                19 => include_str!("../../../../src/shaders/vis_vumeters_3d.wgsl"),
                20 => include_str!("../../../../src/shaders/vis_bioluminescence.wgsl"),
                23 => include_str!("../../../../src/shaders/vis_lyrics.wgsl"),
                24 => include_str!("../../../../src/shaders/vis_tape_head.wgsl"),
                25 => include_str!("../../../../src/shaders/vis_spectrum_led.wgsl"),
                27 => include_str!("../../../../src/shaders/vis_flip_ferrofluid.wgsl"),
                28 => include_str!("../../../../src/shaders/vis_glass_water.wgsl"),
                29 => include_str!("../../../../src/shaders/vis_spectrum_canyon.wgsl"),
                31 => include_str!("../../../../src/shaders/vis_neon_acoustic_room.wgsl"),
                32 => include_str!("../../../../src/shaders/vis_flip_disc.wgsl"),
                _ => include_str!("../../../../src/shaders/vis_spectrum.wgsl"),
            }
        };

        let shader_sources: Vec<String> = crate::state::VISUALIZERS
            .iter()
            .map(|v| resolve_shader_includes(get_shader_source(v.id)))
            .collect();

        let render_pipelines: Vec<Option<wgpu::RenderPipeline>> =
            (0..shader_sources.len()).map(|_| None).collect();

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
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let fallback_pipeline_3d = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Fallback 3D Render Pipeline"),
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

        let lamp_pipeline = fallback_pipeline_3d.clone();

        // --- Video Pipeline (Quarter-Res Border Blur + Main Display Pass) ---
        let video_blur_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Video Border Blur Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        // Y
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // U (or UV interleaved Rg8Unorm / Rg16Unorm)
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // V (or 1x1 placeholder for SemiPlanar420)
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // Sampler
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // Params
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

        let video_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Video Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        // Pre-blurred quarter-res ambient border texture
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });

        let video_blur_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Video Border Blur Pipeline Layout"),
                bind_group_layouts: &[Some(&video_blur_bind_group_layout)],
                immediate_size: 0,
            });

        let video_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Video Pipeline Layout"),
                bind_group_layouts: &[Some(&video_bind_group_layout)],
                immediate_size: 0,
            });

        let video_shader = device.create_shader_module(wgpu::include_wgsl!(
            "../../../../src/shaders/vis_video.wgsl"
        ));
        let video_border_blur_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Video Quarter-Res Border Blur Pipeline"),
                layout: Some(&video_blur_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &video_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &video_shader,
                    entry_point: Some("fs_border_blur"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: pipeline_cache_ref,
            });

        let video_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Video Render Pipeline (No Depth)"),
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: pipeline_cache_ref,
        });

        let hud_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HUD Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let hud_source = resolve_shader_includes(include_str!("../../../../src/shaders/hud.wgsl"));
        let hud_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("HUD Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&hud_source)),
        });
        let hud_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HUD Pipeline"),
            layout: Some(&hud_pipeline_layout),
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
            depth_stencil: None,
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
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
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
            "#,
            )),
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

        let synthwave_sky_shader_src = resolve_shader_includes(include_str!(
            "../../../../src/shaders/vis_synthwave_racer_sky.wgsl"
        ));
        let synthwave_sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Synthwave Sky Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(synthwave_sky_shader_src)),
        });

        let synthwave_sky_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Synthwave Sky Pipeline"),
                layout: Some(&render_pipeline_layout_3d),
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

        let vumeters_bg_shader_src =
            resolve_shader_includes(include_str!("../../../../src/shaders/vis_vumeters_bg.wgsl"));
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

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d {
                width: config.width.max(1),
                height: config.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_texture_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions {
                depth_stencil_format: None,
                ..Default::default()
            },
        );

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

        let heatmap_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("heatmap_compute_layout"),
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
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::R32Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let heatmap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("heatmap_bind_group"),
            layout: &heatmap_compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&history_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: gpu_spectrum_buffer.as_entire_binding(),
                },
            ],
        });
        let heatmap_source =
            resolve_shader_includes(include_str!("../../../../src/shaders/heatmap_compute.wgsl"));
        let heatmap_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Heatmap Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&heatmap_source)),
        });
        let heatmap_compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("heatmap_compute_layout"),
                bind_group_layouts: &[Some(&heatmap_compute_layout)],
                immediate_size: 0,
            });
        let heatmap_compute_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Heatmap Compute Pipeline"),
                layout: Some(&heatmap_compute_pipeline_layout),
                module: &heatmap_compute_shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: pipeline_cache_ref,
            });
        let fire_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fire_compute_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let fire_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fire_bg_a"),
            layout: &fire_compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: fire_buffer_a.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: fire_buffer_b.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: fire_coal_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: fire_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: gpu_spectrum_buffer.as_entire_binding(),
                },
            ],
        });
        let fire_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fire_bg_b"),
            layout: &fire_compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: fire_buffer_b.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: fire_buffer_a.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: fire_coal_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: fire_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: gpu_spectrum_buffer.as_entire_binding(),
                },
            ],
        });

        let firesim_compute_source =
            resolve_shader_includes(include_str!("../../../../src/shaders/firesim_compute.wgsl"));
        let firesim_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FireSim Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&firesim_compute_source)),
        });
        let fire_compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fire_compute_layout"),
                bind_group_layouts: &[Some(&fire_compute_layout)],
                immediate_size: 0,
            });
        let firesim_compute_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FireSim Compute Pipeline"),
                layout: Some(&fire_compute_pipeline_layout),
                module: &firesim_compute_shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: pipeline_cache_ref,
            });

        // --- Quantum FLIP Ferrofluid Compute & Render Setup ---
        let flip_particles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FLIP Particles"),
            size: (65536 * 32) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let flip_grid_accum = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FLIP Grid Accum"),
            size: (131072 * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flip_grid_vel_new = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FLIP Grid Vel New"),
            size: (131072 * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flip_grid_vel_old = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FLIP Grid Vel Old"),
            size: (131072 * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flip_pressure_0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FLIP Pressure 0"),
            size: (131072 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flip_pressure_1 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FLIP Pressure 1"),
            size: (131072 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flip_render_grid = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FLIP Render Grid"),
            size: (512 * 512 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // GPU-compacted active airborne splash droplet header (8 u32 words, starting with
        // DrawIndexedIndirectArgs: [index_count=6, instance_count=0, 0, 0, 0]) + 65,536 active indices.
        let splash_compact_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Splash Compact Active Indices"),
            size: ((8 + 65536) * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let initial_splash_header: [u32; 8] = [6, 0, 0, 0, 0, 0, 0, 0];
        queue.write_buffer(
            &splash_compact_buffer,
            0,
            bytemuck::cast_slice(&initial_splash_header),
        );

        let splash_indirect_args_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Splash DrawIndexedIndirect Args"),
            size: 20,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &splash_indirect_args_buffer,
            0,
            bytemuck::cast_slice(&initial_splash_header[..5]),
        );

        let flip_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("flip_compute_layout"),
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 8,
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

        let flip_compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flip_compute_bg"),
            layout: &flip_compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: flip_particles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: flip_grid_accum.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: flip_grid_vel_new.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: flip_grid_vel_old.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: flip_pressure_0.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: flip_pressure_1.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: flip_render_grid.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: splash_compact_buffer.as_entire_binding(),
                },
            ],
        });

        let flip_compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flip_compute_layout"),
                bind_group_layouts: &[Some(&flip_compute_layout)],
                immediate_size: 0,
            });

        let flip_render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FLIP Render BG"),
            layout: &flip_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: flip_render_grid.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: flip_particles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: splash_compact_buffer.as_entire_binding(),
                },
            ],
        });

        // --- Acoustic FLIP Water Compute Pipelines ---
        let flip_water_compute_source = resolve_shader_includes(include_str!(
            "../../../../src/shaders/flip_water_compute.wgsl"
        ));
        let flip_water_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FLIP Water Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                &flip_water_compute_source,
            )),
        });

        let flip_water_clear_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FLIP Water Clear"),
                layout: Some(&flip_compute_pipeline_layout),
                module: &flip_water_compute_shader,
                entry_point: Some("cs_clear"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        let flip_water_clear_render_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FLIP Water Clear Render"),
                layout: Some(&flip_compute_pipeline_layout),
                module: &flip_water_compute_shader,
                entry_point: Some("cs_clear_render"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        let flip_water_p2g_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FLIP Water P2G"),
                layout: Some(&flip_compute_pipeline_layout),
                module: &flip_water_compute_shader,
                entry_point: Some("cs_p2g"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        let flip_water_forces_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FLIP Water Forces"),
                layout: Some(&flip_compute_pipeline_layout),
                module: &flip_water_compute_shader,
                entry_point: Some("cs_grid_forces"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        let flip_water_pressure_0_to_1_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FLIP Water Pressure 0->1"),
                layout: Some(&flip_compute_pipeline_layout),
                module: &flip_water_compute_shader,
                entry_point: Some("cs_pressure_0_to_1"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        let flip_water_pressure_1_to_0_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FLIP Water Pressure 1->0"),
                layout: Some(&flip_compute_pipeline_layout),
                module: &flip_water_compute_shader,
                entry_point: Some("cs_pressure_1_to_0"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        let flip_water_project_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FLIP Water Project"),
                layout: Some(&flip_compute_pipeline_layout),
                module: &flip_water_compute_shader,
                entry_point: Some("cs_grid_project"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        let flip_water_g2p_advect_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("FLIP Water G2P Advect"),
                layout: Some(&flip_compute_pipeline_layout),
                module: &flip_water_compute_shader,
                entry_point: Some("cs_g2p_advect_splat"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        // --- 3D Airborne Water Splash Droplet Pipeline ---
        let splash_quad_verts = [
            Vertex {
                position: [-0.5, -0.5, 0.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 0.0],
            },
            Vertex {
                position: [0.5, -0.5, 0.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 0.0],
            },
            Vertex {
                position: [0.5, 0.5, 0.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 1.0],
            },
            Vertex {
                position: [-0.5, 0.5, 0.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 1.0],
            },
        ];
        let splash_quad_inds: [u32; 6] = [0, 1, 2, 0, 2, 3];

        let splash_quad_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Splash Quad Vertex Buffer"),
            size: (splash_quad_verts.len() * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &splash_quad_vertex_buffer,
            0,
            bytemuck::cast_slice(&splash_quad_verts),
        );

        let splash_quad_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Splash Quad Index Buffer"),
            size: (splash_quad_inds.len() * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &splash_quad_index_buffer,
            0,
            bytemuck::cast_slice(&splash_quad_inds),
        );

        let water_splash_source = resolve_shader_includes(include_str!(
            "../../../../src/shaders/vis_water_splash.wgsl"
        ));
        let water_splash_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Water Splash Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&water_splash_source)),
        });

        let water_splash_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Water Splash Droplet Pipeline"),
                layout: Some(&lyrics_render_pipeline_layout_3d),
                vertex: wgpu::VertexState {
                    module: &water_splash_shader,
                    entry_point: Some("vs_splash"),
                    buffers: &[Some(Vertex::desc())],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &water_splash_shader,
                    entry_point: Some("fs_splash"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
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
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: pipeline_cache_ref,
            });

        // --- Bioluminescent Waves Compute & Render Setup ---
        let biolum_particles_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Bioluminescent Particles"),
            size: (BIOLUM_PARTICLE_COUNT as u64) * 32, // 36,864 particles * 32 bytes/particle
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let biolum_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let biolum_compute_source =
            resolve_shader_includes(include_str!("../../../../src/shaders/biolum_compute.wgsl"));
        let biolum_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Biolum Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&biolum_compute_source)),
        });

        let biolum_compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Biolum Compute Pipeline Layout"),
                bind_group_layouts: &[Some(&biolum_compute_layout)],
                immediate_size: 0,
            });

        let biolum_compute_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
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
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: biolum_particles_buffer.as_entire_binding(),
            }],
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

        // --- Resynth compute (consumes the CPU-filled GPU spectrum buffer + AudioUniforms) ---
        let resynth_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("resynth_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
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
        let resynth_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resynth_bind_group"),
            layout: &resynth_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: gpu_spectrum_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });
        let resynth_compute_source =
            resolve_shader_includes(include_str!("../../../../src/shaders/resynth_compute.wgsl"));
        let resynth_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Resynth Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&resynth_compute_source)),
        });
        let resynth_compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("resynth_compute_layout"),
                bind_group_layouts: &[Some(&resynth_bind_group_layout)],
                immediate_size: 0,
            });
        let resynth_compute_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Resynth Compute Pipeline"),
                layout: Some(&resynth_compute_pipeline_layout),
                module: &resynth_compute_shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: pipeline_cache_ref,
            });

        // --- Acoustic Wave Chamber 3D FDTD Compute Pipeline ---
        const ACOUSTIC_GRID_VOXELS: u64 = 64 * 32 * 64;
        let acoustic_wave_a_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Acoustic Wave Field A"),
            size: ACOUSTIC_GRID_VOXELS * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let acoustic_wave_b_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Acoustic Wave Field B"),
            size: ACOUSTIC_GRID_VOXELS * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let acoustic_packed_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Acoustic Wave Packed Volume Buffer"),
            size: ACOUSTIC_GRID_VOXELS * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let acoustic_volume_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Acoustic Wave 3D Volume Texture"),
            size: wgpu::Extent3d {
                width: 64,
                height: 32,
                depth_or_array_layers: 64,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let acoustic_volume_view =
            acoustic_volume_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let acoustic_volume_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Acoustic Wave Volume Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let acoustic_render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Acoustic Wave Volume Render Bind Group"),
            layout: &acoustic_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&acoustic_volume_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&acoustic_volume_sampler),
                },
            ],
        });
        let acoustic_wave_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Acoustic Wave Compute Bind Group Layout"),
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
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
        let acoustic_wave_compute_bind_group =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Acoustic Wave Compute Bind Group"),
                layout: &acoustic_wave_compute_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: acoustic_wave_a_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: acoustic_wave_b_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: acoustic_packed_buffer.as_entire_binding(),
                    },
                ],
            });
        let acoustic_wave_compute_source = resolve_shader_includes(include_str!(
            "../../../../src/shaders/acoustic_wave_compute.wgsl"
        ));
        let acoustic_wave_compute_shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Acoustic Wave Compute Shader"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                    &acoustic_wave_compute_source,
                )),
            });
        let acoustic_wave_compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Acoustic Wave Compute Pipeline Layout"),
                bind_group_layouts: &[Some(&acoustic_wave_compute_layout)],
                immediate_size: 0,
            });
        let acoustic_wave_clear_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Acoustic Wave Clear Pipeline"),
                layout: Some(&acoustic_wave_compute_pipeline_layout),
                module: &acoustic_wave_compute_shader,
                entry_point: Some("cs_clear_waves"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });
        let acoustic_wave_step_ab_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Acoustic Wave Step A->B Pipeline"),
                layout: Some(&acoustic_wave_compute_pipeline_layout),
                module: &acoustic_wave_compute_shader,
                entry_point: Some("cs_step_ab"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });
        let acoustic_wave_step_ba_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Acoustic Wave Step B->A Pipeline"),
                layout: Some(&acoustic_wave_compute_pipeline_layout),
                module: &acoustic_wave_compute_shader,
                entry_point: Some("cs_step_ba"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });

        // --- Mechanical Flip-Disc Compute & Render Setup (Visualizer ID 32) ---
        let flip_disc_buf_bytes = (FLIP_DISC_COUNT as u64) * 16;
        let flip_disc_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flip-Disc Params Uniform"),
            size: std::mem::size_of::<FlipDiscParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let flip_disc_wave_a_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flip-Disc Wave A"),
            size: flip_disc_buf_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let flip_disc_wave_b_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flip-Disc Wave B"),
            size: flip_disc_buf_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let flip_disc_state_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flip-Disc State Buffer"),
            size: flip_disc_buf_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flip_disc_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flip-Disc Compute Bind Group Layout"),
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
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
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

        let flip_disc_compute_bg_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flip-Disc Compute BG A"),
            layout: &flip_disc_compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: flip_disc_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: flip_disc_wave_a_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: flip_disc_wave_b_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: flip_disc_state_buffer.as_entire_binding(),
                },
            ],
        });

        let flip_disc_compute_bg_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flip-Disc Compute BG B"),
            layout: &flip_disc_compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: flip_disc_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: flip_disc_wave_b_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: flip_disc_wave_a_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: flip_disc_state_buffer.as_entire_binding(),
                },
            ],
        });

        let flip_disc_render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flip-Disc Render Bind Group"),
            layout: &flip_disc_render_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: flip_disc_state_buffer.as_entire_binding(),
            }],
        });

        let flip_disc_compute_source = resolve_shader_includes(include_str!(
            "../../../../src/shaders/flip_disc_compute.wgsl"
        ));
        let flip_disc_compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Flip-Disc Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&flip_disc_compute_source)),
        });
        let flip_disc_compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Flip-Disc Compute Pipeline Layout"),
                bind_group_layouts: &[Some(&flip_disc_compute_layout)],
                immediate_size: 0,
            });
        let flip_disc_init_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Flip-Disc Init Pipeline"),
                layout: Some(&flip_disc_compute_pipeline_layout),
                module: &flip_disc_compute_shader,
                entry_point: Some("cs_init"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });
        let flip_disc_wave_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Flip-Disc Wave Step Pipeline"),
                layout: Some(&flip_disc_compute_pipeline_layout),
                module: &flip_disc_compute_shader,
                entry_point: Some("cs_wave_step"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
            });
        let flip_disc_step_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Flip-Disc Disc Step Pipeline"),
                layout: Some(&flip_disc_compute_pipeline_layout),
                module: &flip_disc_compute_shader,
                entry_point: Some("cs_disc_step"),
                compilation_options: Default::default(),
                cache: pipeline_cache_ref,
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
                                tex_coords: [
                                    x as f32 / grid_width as f32,
                                    z as f32 / grid_depth as f32,
                                ],
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
                crate::state::Geometry::UnitQuad => {
                    let mut vertices = Vec::new();
                    let mut indices = Vec::new();
                    let start_idx = 0;
                    vertices.push(Vertex {
                        position: [-0.5, -0.5, 0.0],
                        normal: [0.0, 0.0, 1.0],
                        tex_coords: [0.0, 0.0],
                    });
                    vertices.push(Vertex {
                        position: [0.5, -0.5, 0.0],
                        normal: [0.0, 0.0, 1.0],
                        tex_coords: [1.0, 0.0],
                    });
                    vertices.push(Vertex {
                        position: [0.5, 0.5, 0.0],
                        normal: [0.0, 0.0, 1.0],
                        tex_coords: [1.0, 1.0],
                    });
                    vertices.push(Vertex {
                        position: [-0.5, 0.5, 0.0],
                        normal: [0.0, 0.0, 1.0],
                        tex_coords: [0.0, 1.0],
                    });
                    indices.extend_from_slice(&[
                        start_idx,
                        start_idx + 1,
                        start_idx + 2,
                        start_idx,
                        start_idx + 2,
                        start_idx + 3,
                    ]);
                    (vertices, indices)
                }
                crate::state::Geometry::UnitBox => {
                    let mut vertices = Vec::with_capacity(4);
                    let mut indices = Vec::with_capacity(6);
                    // Front-facing (-Z) quad for instanced 3D bars (eliminates 83% culled back/side faces)
                    let normal = [0.0, 0.0, -1.0];
                    let quad = [
                        [0.5, -0.5, -0.5],
                        [-0.5, -0.5, -0.5],
                        [-0.5, 0.5, -0.5],
                        [0.5, 0.5, -0.5],
                    ];
                    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
                    for (i, pos) in quad.into_iter().enumerate() {
                        vertices.push(Vertex {
                            position: pos,
                            normal,
                            tex_coords: uvs[i],
                        });
                    }
                    indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                    (vertices, indices)
                }
                crate::state::Geometry::NeonRoom => generate_neon_room_mesh(),
                crate::state::Geometry::SynthwaveRacerScene => generate_synthwave_racer_mesh(),
                crate::state::Geometry::VuMeterRack => generate_vumeter_rack_mesh(),
                crate::state::Geometry::GlassLyricsScene => {
                    generate_glass_lyrics_mesh("RUSTTRACKER")
                }
            };

            let vertex_capacity = (vertices.len() * std::mem::size_of::<Vertex>()).max(64) as u64;
            let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("Mesh Vertex Buffer {:?}", geom)),
                size: vertex_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));

            let index_capacity = (indices.len() * std::mem::size_of::<u32>()).max(64) as u64;
            let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("Mesh Index Buffer {:?}", geom)),
                size: index_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&index_buffer, 0, bytemuck::cast_slice(&indices));
            let index_count = indices.len() as u32;

            mesh_registry.insert(
                geom,
                MeshBuffers {
                    vertex_buffer,
                    index_buffer,
                    vertex_capacity,
                    index_capacity,
                    index_count,
                },
            );
        }

        let initial_lyrics_idx_count = mesh_registry
            .get(&crate::state::Geometry::GlassLyricsScene)
            .map(|m| m.index_count)
            .unwrap_or(0);
        let lyrics_indirect_args_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lyrics DrawIndexedIndirect Args"),
            size: 20,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let initial_lyrics_indirect: [u32; 5] = [initial_lyrics_idx_count, 1, 0, 0, 0];
        queue.write_buffer(
            &lyrics_indirect_args_buffer,
            0,
            bytemuck::cast_slice(&initial_lyrics_indirect),
        );

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

        let (biolum_ocean_verts, biolum_ocean_inds) = generate_biolum_ocean_mesh();
        let biolum_ocean_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Biolum Ocean Vertex Buffer"),
            size: (biolum_ocean_verts.len() * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &biolum_ocean_vertex_buffer,
            0,
            bytemuck::cast_slice(&biolum_ocean_verts),
        );

        let biolum_ocean_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Biolum Ocean Index Buffer"),
            size: (biolum_ocean_inds.len() * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &biolum_ocean_index_buffer,
            0,
            bytemuck::cast_slice(&biolum_ocean_inds),
        );
        let biolum_ocean_index_count = biolum_ocean_inds.len() as u32;
        // --- END 3D Engine Extensions Init ---

        let pipeline_cache_path = if pipeline_cache_enabled {
            pipeline_cache_path
        } else {
            None
        };

        let mut engine = Self {
            instance,
            surface: Some(surface),
            device,
            queue,
            adapter_info,
            negotiated_features,
            config,
            size,
            render_pipelines,
            shader_sources,
            fallback_pipeline,
            fallback_pipeline_3d,
            render_pipeline_layout,
            render_pipeline_layout_3d,
            lyrics_render_pipeline_layout_3d,
            pipeline_cache,
            pipeline_cache_path,
            hud_pipeline,
            uniform_buffer,
            waveform_storage_buffer,
            _history_texture: history_texture,
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
            track_info_rect: None,
            firesim_compute_pipeline,
            fire_buffer_a,
            fire_buffer_b,
            _fire_coal_buffer: fire_coal_buffer,
            fire_params_buffer,
            fire_bind_group_a,
            fire_bind_group_b,
            fire_ping: true,
            retro_fire_cells: Box::new([0u8; 320 * 180]),
            retro_fire_upload: vec![0.0f32; 1024 * 576],
            retro_fire_rng: 0x12345678,
            retro_fire_accum: 0.0,
            retro_fire_cold_uploaded: false,
            heatmap_row: 0,
            heatmap_compute_pipeline,
            heatmap_bind_group,

            flip_compute_bind_group,
            flip_render_bind_group,

            flip_water_clear_pipeline,
            flip_water_clear_render_pipeline,
            flip_water_p2g_pipeline,
            flip_water_forces_pipeline,
            flip_water_pressure_0_to_1_pipeline,
            flip_water_pressure_1_to_0_pipeline,
            flip_water_project_pipeline,
            flip_water_g2p_advect_pipeline,
            water_splash_pipeline,
            splash_quad_vertex_buffer,
            splash_quad_index_buffer,
            splash_compact_buffer,
            splash_indirect_args_buffer,
            lyrics_indirect_args_buffer,

            _biolum_particles_buffer: biolum_particles_buffer,
            biolum_compute_pipeline,
            biolum_compute_bind_group,
            biolum_render_bind_group,
            _biolum_render_pipeline_layout: biolum_render_pipeline_layout,
            biolum_ocean_vertex_buffer,
            biolum_ocean_index_buffer,
            biolum_ocean_index_count,
            biolum_initialized: false,

            resynth_compute_pipeline,
            resynth_bind_group,
            _gpu_spectrum_buffer: gpu_spectrum_buffer,

            acoustic_wave_clear_pipeline,
            acoustic_wave_step_ab_pipeline,
            acoustic_wave_step_ba_pipeline,
            acoustic_wave_compute_bind_group,
            _acoustic_wave_a_buffer: acoustic_wave_a_buffer,
            _acoustic_wave_b_buffer: acoustic_wave_b_buffer,
            acoustic_packed_buffer,
            acoustic_volume_texture,
            neon_acoustic_render_pipeline_layout_3d,
            acoustic_render_bind_group,
            acoustic_wave_initialized: false,

            flip_disc_params_buffer,
            _flip_disc_wave_a_buffer: flip_disc_wave_a_buffer,
            _flip_disc_wave_b_buffer: flip_disc_wave_b_buffer,
            _flip_disc_state_buffer: flip_disc_state_buffer,
            flip_disc_compute_bg_a,
            flip_disc_compute_bg_b,
            flip_disc_init_pipeline,
            flip_disc_wave_pipeline,
            flip_disc_step_pipeline,
            flip_disc_render_pipeline_layout,
            flip_disc_render_bind_group,
            flip_disc_ping: true,
            flip_disc_initialized: false,
            flip_disc_sim: FlipDiscSimState::default(),
            flip_disc_pending_substeps: 0,

            depth_texture_view,

            waveform_history_flat: vec![0.0; 2048 * WAVEFORM_HISTORY_RING_CAP],
            waveform_history_packed: vec![0u32; 1024 * WAVEFORM_HISTORY_RING_CAP],
            tape_timeline_scratch: vec![0.0; 600 * 8],
            video_blur_bind_group_layout,
            video_bind_group_layout,
            video_border_blur_pipeline,
            video_pipeline,
            video_state: None,
            video_repack_scratch: Vec::new(),
            clear_black_pipeline,
            biolum_bg_pipeline: None,
            biolum_ocean_pipeline: None,
            synthwave_sky_pipeline,
            vumeters_bg_pipeline,

            pending_uniforms: None,
            cached_meta_path: String::new(),
            cached_file_dir: String::new(),
            cached_file_len: None,
            last_acquire_instant: None,

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
            smooth_audio_seconds: 0.0f64,
            last_raw_state_seconds: 0.0f64,
            smooth_dt: 1.0f64 / 60.0f64,
            last_history_push_count: 0,
            time_since_last_push: 0.0,
            measured_push_interval: 1.0f64 / 144.0f64,
            vu_needle_angles: vec![0.0; 32],
            vu_needle_velocities: vec![0.0; 32],
            channel_phases: [0.0; 32],
            neon_spatial_env: [0.0; 16],
            vu_prev_active_segments: [0; 32],
            vu_segment_glows: [[0.0; 30]; 32],
            vu_prev_num_channels: 0,
            last_uploaded_push_count: u64::MAX,
            last_uploaded_vis_width: 0,
            last_uploaded_vis_id: u32::MAX,
            last_uploaded_hist_len: 0,
            waveform_ring_head: 0,
            waveform_ring_base: 0,
            lyric_slam_timer: 0.0,
            last_lyric_line_idx: None,
            current_lyric_mesh_text: "RUSTTRACKER".to_string(),
            fire_intensity: 0.0,
            splash_cache: SplashCache::default(),
            stats_cache: StatsDisplayCache::default(),
            is_game_mode: std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .eq_ignore_ascii_case("gamescope")
                || std::env::var("XDG_SESSION_DESKTOP")
                    .unwrap_or_default()
                    .eq_ignore_ascii_case("gamescope")
                || std::env::var("STEAM_DECK").is_ok()
                || std::env::var("SteamDeck").is_ok()
                || std::env::var("SteamOS").is_ok()
                || std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
                    .map(|v| v.trim().eq_ignore_ascii_case("valve"))
                    .unwrap_or(false),
            last_steps_to_fill: 0,
            last_aspect_ratio: size.width as f32 / size.height.max(1) as f32,
        };
        engine.ensure_visualizer_pipeline(0);
        engine
    }

    fn save_pipeline_cache(&self) {
        // Persist the pipeline cache back to disk atomically so subsequent launches
        // can skip recompiling compute, UI, and newly compiled visualizer pipelines.
        if let (Some(path), Some(cache)) = (&self.pipeline_cache_path, &self.pipeline_cache)
            && let Some(data) = cache.get_data()
        {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, &data).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    pub fn ensure_visualizer_pipeline(&mut self, i: usize) {
        if i >= self.render_pipelines.len() || self.render_pipelines[i].is_some() {
            return;
        }
        let vis_def = &crate::state::VISUALIZERS[i];
        let is_mesh_3d = matches!(
            vis_def.pipeline_type,
            crate::state::PipelineType::Mesh3D { .. }
        );
        let source = &self.shader_sources[i];
        let scope_main = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("Shader {}", i)),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(source.as_str())),
            });

        let (layout, vertex_buffers, primitive, vs_entry) = if vis_def.id == 20 {
            (
                &self._biolum_render_pipeline_layout,
                vec![Some(Vertex::desc())],
                wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                "vs_main_3d",
            )
        } else if vis_def.id == 23 {
            (
                &self.lyrics_render_pipeline_layout_3d,
                vec![Some(Vertex::desc())],
                wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                "vs_main_3d",
            )
        } else if vis_def.id == 31 {
            (
                &self.neon_acoustic_render_pipeline_layout_3d,
                vec![Some(Vertex::desc())],
                wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                "vs_main_3d",
            )
        } else if vis_def.id == 32 {
            (
                &self.flip_disc_render_pipeline_layout,
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
                "vs_main",
            )
        } else {
            match vis_def.pipeline_type {
                crate::state::PipelineType::FullscreenTriangle => (
                    &self.render_pipeline_layout,
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
                    "vs_main",
                ),
                crate::state::PipelineType::Mesh3D { .. } => (
                    &self.render_pipeline_layout_3d,
                    vec![Some(Vertex::desc())],
                    wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        unclipped_depth: false,
                        conservative: false,
                    },
                    "vs_main_3d",
                ),
            }
        };

        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                        format: self.config.format,
                        blend: Some(if vis_def.id == 4 || vis_def.id == 20 {
                            wgpu::BlendState {
                                color: wgpu::BlendComponent {
                                    src_factor: wgpu::BlendFactor::One,
                                    dst_factor: wgpu::BlendFactor::One,
                                    operation: wgpu::BlendOperation::Add,
                                },
                                alpha: wgpu::BlendComponent::OVER,
                            }
                        } else if vis_def.id == 3 {
                            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
                        } else {
                            wgpu::BlendState::REPLACE
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive,
                depth_stencil: if is_mesh_3d {
                    Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(vis_def.id != 4 && vis_def.id != 20),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    })
                } else {
                    None
                },
                multisample: wgpu::MultisampleState {
                    count: 1,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview_mask: None,
                cache: self.pipeline_cache.as_ref(),
            });

        let error_future = scope_main.pop();
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });

        if let Some(error) = pollster::block_on(error_future) {
            eprintln!("WGSL compilation error in visualizer {}: {:?}", i, error);
            self.render_pipelines[i] = Some(if is_mesh_3d {
                self.fallback_pipeline_3d.clone()
            } else {
                self.fallback_pipeline.clone()
            });
        } else {
            self.render_pipelines[i] = Some(pipeline);
            if vis_def.id == 13 {
                let lp = self
                    .device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("Lamp Render Pipeline"),
                        layout: Some(&self.render_pipeline_layout_3d),
                        vertex: wgpu::VertexState {
                            module: &shader,
                            entry_point: Some("vs_lamp"),
                            buffers: &[Some(Vertex::desc())],
                            compilation_options: wgpu::PipelineCompilationOptions::default(),
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &shader,
                            entry_point: Some("fs_lamp"),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: self.config.format,
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
                        cache: self.pipeline_cache.as_ref(),
                    });
                self.lamp_pipeline = lp;
            } else if vis_def.id == 20 {
                let scope_biolum = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
                let bg_pipe = self
                    .device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("Biolum Sky/Horizon BG Pipeline"),
                        layout: Some(&self.render_pipeline_layout_3d),
                        vertex: wgpu::VertexState {
                            module: &shader,
                            entry_point: Some("vs_bg"),
                            buffers: &[],
                            compilation_options: wgpu::PipelineCompilationOptions::default(),
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &shader,
                            entry_point: Some("fs_bg"),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: self.config.format,
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
                        cache: self.pipeline_cache.as_ref(),
                    });
                let ocean_pipe =
                    self.device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                            label: Some("Biolum 3D Ocean Surface Pipeline"),
                            layout: Some(&self.render_pipeline_layout_3d),
                            vertex: wgpu::VertexState {
                                module: &shader,
                                entry_point: Some("vs_ocean"),
                                buffers: &[Some(Vertex::desc())],
                                compilation_options: wgpu::PipelineCompilationOptions::default(),
                            },
                            fragment: Some(wgpu::FragmentState {
                                module: &shader,
                                entry_point: Some("fs_ocean"),
                                targets: &[Some(wgpu::ColorTargetState {
                                    format: self.config.format,
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
                            multisample: wgpu::MultisampleState::default(),
                            multiview_mask: None,
                            cache: self.pipeline_cache.as_ref(),
                        });
                let biolum_err = scope_biolum.pop();
                let _ = self.device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });
                if let Some(error) = pollster::block_on(biolum_err) {
                    eprintln!("Biolum auxiliary pipeline validation error: {:?}", error);
                    self.biolum_bg_pipeline = None;
                    self.biolum_ocean_pipeline = None;
                } else {
                    self.biolum_bg_pipeline = Some(bg_pipe);
                    self.biolum_ocean_pipeline = Some(ocean_pipe);
                }
            }
            self.save_pipeline_cache();
        }
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width.max(1);
            self.config.height = new_size.height.max(1);
            if let Some(surface) = &self.surface {
                surface.configure(&self.device, &self.config);
            }

            let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Depth Texture"),
                size: wgpu::Extent3d {
                    width: self.config.width.max(1),
                    height: self.config.height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth_texture_view =
                depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        }
    }

    pub fn suspend_surface(&mut self) {
        eprintln!("[RustTracker Engine] Suspending surface...");
        self.surface = None;
    }

    pub fn resume_surface(&mut self, window: Arc<Window>) {
        let size = window.inner_size();
        eprintln!(
            "[RustTracker Engine] Resuming surface with size: {:?}",
            size
        );
        match self.instance.create_surface(window) {
            Ok(surface) => {
                self.surface = Some(surface);
                self.resize(size);
                eprintln!("[RustTracker Engine] Surface successfully resumed and configured!");
            }
            Err(e) => {
                eprintln!(
                    "[RustTracker Engine] Failed to create resumed surface: {:?}",
                    e
                );
            }
        }
    }

    /// Extracts inter-channel phase relationships, spectral centroid oscillator phase, and
    /// 3 orbiting 3D object stem coordinates into the 32-float `channel_phases` uniform block
    /// used by Acoustic Wave Chamber (`id == 31` Neon Acoustic Studio).
    #[allow(clippy::too_many_arguments)]
    pub fn extract_acoustic_wave_phases(
        phases_out: &mut [f32; 32],
        raw_audio_channels: &[Vec<f32>],
        spectrum_data: &[f32],
        spatial_channels: &[f32; 16],
        num_spatial_channels: u32,
        frame_dt: f32,
        is_playing: bool,
        play_time_s: f32,
    ) {
        use std::f32::consts::{PI, TAU};
        let dt = frame_dt.clamp(0.0005, 0.05);
        let phi_alpha = (dt * 24.0).clamp(0.15, 0.65);

        // 1. Choose active reference channel (prefer Ch 0 / Left if active)
        let ch_limit = raw_audio_channels.len().min(16);
        let tail_rms_sq = |samples: &[f32], n: usize| -> f32 {
            let start = samples.len().saturating_sub(n);
            let slice = &samples[start..];
            if slice.is_empty() {
                return 0.0;
            }
            let mean = slice.iter().sum::<f32>() / slice.len() as f32;
            slice.iter().map(|&s| (s - mean) * (s - mean)).sum::<f32>()
        };

        let mut ref_idx = 0usize;
        if ch_limit > 0 {
            if tail_rms_sq(&raw_audio_channels[0], 256) < 1e-7 {
                let mut best_e = 1e-7;
                for (i, buf) in raw_audio_channels.iter().take(ch_limit).enumerate() {
                    let e = tail_rms_sq(buf, 256);
                    if e > best_e {
                        best_e = e;
                        ref_idx = i;
                    }
                }
            }

            let ref_buf = &raw_audio_channels[ref_idx];
            let n = ref_buf.len().min(256);
            if n >= 4 {
                let ref_slice = &ref_buf[ref_buf.len() - n..];
                let ref_mean = ref_slice.iter().sum::<f32>() / n as f32;
                let mut ref_c = [0.0f32; 256];
                let mut d_ref = [0.0f32; 256];
                let mut e_ref = 0.0f32;
                let mut e_dref = 0.0f32;
                for k in 0..n {
                    let v = ref_slice[k] - ref_mean;
                    ref_c[k] = v;
                    e_ref += v * v;
                    let dv = if k == 0 {
                        ref_slice[1] - ref_slice[0]
                    } else if k + 1 == n {
                        ref_slice[n - 1] - ref_slice[n - 2]
                    } else {
                        0.5 * (ref_slice[k + 1] - ref_slice[k - 1])
                    };
                    d_ref[k] = dv;
                    e_dref += dv * dv;
                }

                for (c, buf) in raw_audio_channels.iter().take(ch_limit).enumerate() {
                    if c == ref_idx {
                        phases_out[c] = 0.0;
                        continue;
                    }
                    if buf.len() >= n && e_ref > 1e-7 {
                        let ch_slice = &buf[buf.len() - n..];
                        let ch_mean = ch_slice.iter().sum::<f32>() / n as f32;
                        let mut e_c = 0.0f32;
                        let mut dot_i = 0.0f32;
                        let mut dot_q = 0.0f32;
                        for k in 0..n {
                            let cv = ch_slice[k] - ch_mean;
                            e_c += cv * cv;
                            dot_i += cv * ref_c[k];
                            dot_q += cv * d_ref[k];
                        }
                        if e_c > 1e-7 {
                            let i_norm = dot_i / (e_c * e_ref).sqrt();
                            let q_norm = if e_dref > 1e-10 {
                                dot_q / (e_c * e_dref).sqrt()
                            } else {
                                0.0
                            };
                            let target_phi = q_norm.atan2(i_norm);
                            let delta = (target_phi - phases_out[c] + PI).rem_euclid(TAU) - PI;
                            let next_phi = phases_out[c] + delta * phi_alpha;
                            phases_out[c] = (next_phi + PI).rem_euclid(TAU) - PI;
                        } else {
                            phases_out[c] *= 1.0 - (dt * 4.0).clamp(0.0, 0.5);
                        }
                    } else {
                        phases_out[c] *= 1.0 - (dt * 4.0).clamp(0.0, 0.5);
                    }
                }
            }
        }
        for slot in &mut phases_out[ch_limit..16] {
            *slot *= 1.0 - (dt * 4.0).clamp(0.0, 0.5);
        }

        // 2. Spectral centroid -> f_vis [16] and wrap-safe continuous base_osc [17]
        let mut weighted_bin = 0.0f32;
        let mut total_mag = 0.0f32;
        let max_scan = spectrum_data.len().min(96);
        for (bin, &mag) in spectrum_data.iter().enumerate().take(max_scan).skip(1) {
            let m = mag.max(0.0);
            weighted_bin += (bin as f32) * m;
            total_mag += m;
        }
        let target_f_vis = if total_mag > 1e-3 {
            let centroid = weighted_bin / total_mag;
            (centroid.ln_1p() / 96.0f32.ln_1p()).clamp(0.0, 1.0)
        } else {
            0.35
        };
        let f_alpha = (dt * 8.0).clamp(0.05, 0.40);
        phases_out[16] += (target_f_vis - phases_out[16]) * f_alpha;
        if is_playing {
            let omega = 28.0 + 14.0 * phases_out[16].clamp(0.0, 1.0);
            phases_out[17] = (phases_out[17] + dt * omega).rem_euclid(TAU);
        }

        // 3. Three dynamic 3D object / stem monopoles [18..30] with <=2-decimal time factors
        let slice_avg = |start: usize, end: usize, norm: f32, max_cap: f32| -> f32 {
            let lo = start.min(spectrum_data.len());
            let hi = end.min(spectrum_data.len());
            if hi <= lo {
                return 0.0;
            }
            let sum: f32 = spectrum_data[lo..hi].iter().copied().sum();
            (sum / ((hi - lo) as f32 * norm)).clamp(0.0, max_cap)
        };
        let obj0_e_raw = slice_avg(26, 80, 35.0, 0.85);
        let obj1_e_raw = slice_avg(8, 26, 45.0, 0.85);
        let obj2_e_raw = slice_avg(2, 8, 55.0, 0.85);
        let bass_kick_raw = slice_avg(1, 6, 65.0, 1.0);

        let e_alpha = (dt * 14.0).clamp(0.08, 0.55);
        phases_out[18] = (play_time_s * 0.45).sin() * 4.8;
        phases_out[19] = 2.2 + (play_time_s * 0.28).cos() * 1.3;
        phases_out[20] = (play_time_s * 0.36).cos() * 4.2;
        phases_out[21] += (obj0_e_raw - phases_out[21]) * e_alpha;

        phases_out[22] = -(play_time_s * 0.32).cos() * 3.6;
        phases_out[23] = 1.0 + (play_time_s * 0.52).sin() * 1.1;
        phases_out[24] = (play_time_s * 0.41).sin() * 3.8;
        phases_out[25] += (obj1_e_raw - phases_out[25]) * e_alpha;

        phases_out[26] = (play_time_s * 0.24).sin() * 3.2;
        phases_out[27] = 0.1 + (play_time_s * 0.38).sin().abs() * 1.2;
        phases_out[28] = -(play_time_s * 0.29).cos() * 3.4;
        phases_out[29] += (obj2_e_raw - phases_out[29]) * e_alpha;

        // 4. Bass transient follower [30] and master simulation intensity [31]
        let kick_alpha = if bass_kick_raw > phases_out[30] {
            (dt * 28.0).clamp(0.2, 0.85)
        } else {
            (dt * 7.0).clamp(0.04, 0.25)
        };
        phases_out[30] += (bass_kick_raw - phases_out[30]) * kick_alpha;

        let active_sp = (num_spatial_channels as usize).clamp(1, 16);
        let sp_peak = spatial_channels[..active_sp]
            .iter()
            .copied()
            .fold(0.0f32, f32::max);
        let sim_target =
            (sp_peak * 0.85 + (obj0_e_raw + obj1_e_raw + bass_kick_raw) * 0.20).clamp(0.0, 1.0);
        let sim_alpha = (dt * 10.0).clamp(0.05, 0.45);
        phases_out[31] += (sim_target - phases_out[31]) * sim_alpha;
    }

    /// Asymmetric attack/release envelope smoothing for Neon Acoustic Studio speaker motion.
    pub fn step_neon_speaker_envelopes(
        envelopes: &mut [f32; 16],
        targets: &[f32; 16],
        frame_dt: f32,
    ) {
        let dt = frame_dt.clamp(0.0005, 0.05);
        let alpha_up = 1.0 - (-dt / 0.032).exp();
        let alpha_down = 1.0 - (-dt / 0.140).exp();
        for (env, &raw_target) in envelopes.iter_mut().zip(targets.iter()) {
            let target = raw_target.clamp(0.0, 1.0);
            let alpha = if target > *env { alpha_up } else { alpha_down };
            let next = *env + (target - *env) * alpha;
            *env = if next < 1e-4 && target <= 1e-4 {
                0.0
            } else {
                next
            };
        }
    }

    pub fn clear_video_state(&mut self) {
        self.video_state = None;
        self.vu_prev_active_segments = [0; 32];
        self.vu_segment_glows = [[0.0; 30]; 32];
        self.vu_prev_num_channels = 0;
        self.channel_phases = [0.0; 32];
        self.neon_spatial_env = [0.0; 16];
        self.acoustic_wave_initialized = false;
        self.biolum_initialized = false;
        self.cached_meta_path.clear();
    }

    pub fn has_video_stream(&self) -> bool {
        self.video_state.is_some()
    }

    /// Advances the EMA-smoothed frame delta and modular host clock `smooth_time`,
    /// returning the clamped frame delta for animation integrators.
    pub(crate) fn step_smooth_frame_time(
        smooth_dt: &mut f64,
        smooth_time: &mut f64,
        dt: f32,
    ) -> f32 {
        let alpha = 0.03f64;
        *smooth_dt = *smooth_dt * (1.0 - alpha) + (dt as f64).clamp(0.001, 0.1) * alpha;
        *smooth_time = (*smooth_time + *smooth_dt).rem_euclid(HOST_TIME_WRAP_PERIOD_SECS);
        (*smooth_dt as f32).clamp(0.0005, 0.1)
    }

    pub fn update(&mut self, state: &AppState, dt: f32) {
        self.frame_count = self.frame_count.wrapping_add(1);

        let frame_dt = Self::step_smooth_frame_time(&mut self.smooth_dt, &mut self.smooth_time, dt);

        if !state.file_loaded {
            self.smooth_audio_seconds = 0.0;
            self.last_raw_state_seconds = 0.0;
            self.fire_intensity = 0.0;
            self.vu_prev_active_segments = [0; 32];
            self.vu_segment_glows = [[0.0; 30]; 32];
            self.vu_prev_num_channels = 0;
            self.channel_phases = [0.0; 32];
            self.acoustic_wave_initialized = false;
            self.biolum_initialized = false;
            return;
        }

        // Track pushes to synchronize scrolling phase with physical buffer updates
        if state.waveform_history_push_count < self.last_history_push_count {
            self.last_history_push_count = state.waveform_history_push_count;
            self.time_since_last_push = 0.0;
        }

        let mut steps = 0;
        if state.waveform_history_push_count != self.last_history_push_count {
            let diff = state
                .waveform_history_push_count
                .saturating_sub(self.last_history_push_count);
            steps = diff as u32;
            self.heatmap_row = (self.heatmap_row + steps) % 1024;

            // Measure the actual elapsed time between buffer pushes to stay in perfect
            // phase with the audio callback cadence (e.g. PipeWire/ALSA vs WASAPI)
            if self.time_since_last_push > 0.002 && self.time_since_last_push < 0.25 {
                let interval_per_step = self.time_since_last_push / (steps as f64).max(1.0);
                self.measured_push_interval =
                    self.measured_push_interval * 0.8 + interval_per_step * 0.2;
            }

            self.time_since_last_push = 0.0;
            self.last_history_push_count = state.waveform_history_push_count;
        } else if !state.is_paused && state.file_loaded && !state.track_ended {
            // Use the real frame dt (not the heavily-smoothed EMA) so the
            // sub-frame scroll interpolation stays in phase with the actual
            // DSP push cadence and does not judder.
            self.time_since_last_push += (dt as f64).clamp(0.0, 0.1);
        }

        // Target interval between pushes: use measured cadence with fallback to target_fps
        let push_interval =
            if self.measured_push_interval > 0.002 && self.measured_push_interval < 0.25 {
                self.measured_push_interval
            } else if state.target_fps > 0 {
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

        // Smooth display-rate audio playhead clock for visualizers (such as Retro Tape Head)
        if let Some(scrub_t) = state.scrub_target_seconds {
            self.smooth_audio_seconds = scrub_t;
            self.last_raw_state_seconds = state.current_seconds;
        } else if !state.file_loaded {
            self.smooth_audio_seconds = 0.0;
            self.last_raw_state_seconds = 0.0;
        } else if state.is_paused
            || state.track_ended
            || state.seek_request.is_some()
            || (state.current_seconds - self.last_raw_state_seconds).abs() > 0.20
        {
            self.smooth_audio_seconds = state.current_seconds;
            self.last_raw_state_seconds = state.current_seconds;
        } else {
            self.smooth_audio_seconds += (dt as f64).clamp(0.0, 0.1);
            let drift = state.current_seconds - self.smooth_audio_seconds;
            if drift.abs() > 0.12 {
                self.smooth_audio_seconds = state.current_seconds;
            } else if (state.current_seconds - self.last_raw_state_seconds).abs() > 1e-6 {
                self.smooth_audio_seconds += drift * 0.25;
            }
            if state.duration_seconds > 0.0 {
                self.smooth_audio_seconds =
                    self.smooth_audio_seconds.clamp(0.0, state.duration_seconds);
            }
            self.last_raw_state_seconds = state.current_seconds;
        }

        // Progress bar fire dies off when playback stops / pauses / ends / reaches end of song
        self.fire_intensity = Self::step_fire_intensity(self.fire_intensity, state, frame_dt);

        // World-Z camera position locked to history rows (0.5 units/row), used by
        // visualizers that scroll with the waveform/spectrum history ring buffer.
        // Wrapped modulo 6000.0 (12,000 rows) in integer math first to maintain precision.
        self.last_history_cam_z =
            (((self.last_history_push_count % 12_000) as f64 + step_fraction as f64) * 0.5)
                % 6000.0;
        let upmix_active = state
            .surround_upmix
            .is_active_for_hardware(state.hardware_channels);
        let mut uniforms = AudioUniforms {
            spectrum: [0.0; 1024],
            fire_heat: [0.0; 1024],
            channels: [0.0; 32],
            channel_peaks: [0.0; 32],
            spatial_channels: [0.0; 16],
            display_order: [0; 16],
            channel_phases: self.channel_phases,
            vu_segment_glows: [0; 256],
            num_channels: state.channel_vus.len().min(32) as u32,
            mode: state.visualizer_mode,
            time: if state.duration_seconds > 0.0 {
                self.smooth_audio_seconds as f32
            } else {
                wrap_host_phase_time(self.smooth_audio_seconds)
            },
            duration: state.duration_seconds as f32,
            smooth_time: wrap_host_phase_time(self.play_time),
            heatmap_row: self.heatmap_row,
            fft_channels: state.raw_audio_channels.len() as u32,
            num_spatial_channels: if upmix_active {
                state.channel_vus.len().min(16) as u32
            } else if let Some(tc) = state.tracker_channels.filter(|&n| n > 0) {
                (tc as u32).min(16)
            } else {
                state.channel_vus.len().min(16) as u32
            },
            ui_meters_rect: self.meters_uv_rect,
            ui_heatmap_rect: self.heatmap_uv_rect,
            ui_fire_rect: self.fire_uv_rect,
            waveform_resolution: 1024,
            waveform_history_size: 144,
            frame_count: self.frame_count,
            step_fraction,
            steps_to_fill: steps,
            aspect_ratio: self.last_aspect_ratio,
            frame_dt,
            history_cam_z: self.last_history_cam_z as f32,
            fire_intensity: self.fire_intensity,
            waveform_history_ring_base: self.waveform_ring_base,
            _pad2: if state.room_volume_smoke { 0.0 } else { 1.0 },
            _pad3: if state.ferrofluid_pool_mode { 1.0 } else { 0.0 },
        };
        self.last_steps_to_fill = steps;

        uniforms.spectrum.copy_from_slice(&state.spectrum_data);
        uniforms.fire_heat.copy_from_slice(&state.fire_heat);

        // Update needle physics for 3D retro VU meters
        let ch_count = state.channel_vus.len().clamp(2, 32);
        if self.vu_needle_angles.len() < ch_count {
            self.vu_needle_angles.resize(ch_count, 0.0);
            self.vu_needle_velocities.resize(ch_count, 0.0);
        }

        let rise_time_secs = 0.30;
        let damping_ratio = 0.85;
        let omega_n = 4.8 / rise_time_secs;
        let spring = omega_n * omega_n;
        let damping = 2.0 * damping_ratio * omega_n;

        let dt_clamped = frame_dt.min(0.05);
        for i in 0..ch_count {
            let target = if i < state.raw_channel_vus.len() {
                state.raw_channel_vus[i]
            } else if !state.file_loaded {
                // Sway gently if no file is loaded
                let t = self.smooth_time as f32;
                let offset = if i == 0 { 0.0 } else { 1.5 };
                (0.12 * (t * 2.0 + offset).sin() + 0.15 * (t * 3.7 - offset).cos() + 0.2)
                    .clamp(0.0, 1.0)
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
        if state.tracker_channels.is_none() || upmix_active {
            if ch_len == 6 {
                display_order = vec![4, 0, 2, 3, 1, 5]; // Ls, L, C, LFE, R, Rs
            } else if ch_len == 8 {
                // SMPTE 7.1: ch4=Ls(side), ch5=Rs(side), ch6=Lrs(rear), ch7=Rrs(rear)
                display_order = vec![6, 4, 0, 2, 3, 1, 5, 7]; // Lrs, Ls, L, C, LFE, R, Rs, Rrs
            } else if ch_len == 12 {
                // 7.1.4 HUD display order matching ["Ltr", "Ltf", "Ls", "L", "C", "LFE", "R", "Rs", "Rtf", "Rtr", "Lrs", "Rrs"]
                display_order = vec![10, 8, 4, 0, 2, 3, 1, 5, 9, 11, 6, 7];
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
                uniforms.channel_peaks[disp_idx] =
                    state.peak_vus.get(src_idx).copied().unwrap_or(0.0);
            }
        }

        // 2. Populate Raw Spatial Channels (strict spatial mapping without UI reordering,
        // blending fast raw_channel_vus for crisp per-note speaker cone & light transients;
        // Neon Acoustic Studio uses a calmer raw blend + asymmetric envelope smoothing).
        let raw_vu_weight = if state.visualizer_mode == 31 {
            0.25
        } else {
            0.72
        };
        let get_fast_vu = |idx: usize| -> f32 {
            let smoothed = state.channel_vus.get(idx).copied().unwrap_or(0.0);
            if let Some(&raw) = state.raw_channel_vus.get(idx) {
                raw * raw_vu_weight + smoothed * (1.0 - raw_vu_weight)
            } else {
                smoothed
            }
        };
        if let Some(num_tracks) = state.tracker_channels.filter(|&n| n > 0 && !upmix_active) {
            // For tracker files, channel_vus is [L_Peak, Track1..N, R_Peak].
            // Map individual tracker tracks into spatial channels so surround/room visualizers
            // light up each speaker with its actual instrument track!
            let tracks_count = (num_tracks as usize).min(state.channel_vus.len().saturating_sub(2));
            if tracks_count > 0 {
                let count = tracks_count.min(16);
                for (i, dst) in uniforms.spatial_channels.iter_mut().enumerate().take(count) {
                    *dst = get_fast_vu(i + 1);
                }
            } else if state.channel_vus.len() >= 2 {
                uniforms.spatial_channels[0] = get_fast_vu(0);
                uniforms.spatial_channels[1] = get_fast_vu(state.channel_vus.len() - 1);
            }
        } else {
            let spatial_count = state.channel_vus.len().min(16);
            for (i, dst) in uniforms
                .spatial_channels
                .iter_mut()
                .enumerate()
                .take(spatial_count)
            {
                *dst = get_fast_vu(i);
            }
        }
        if state.visualizer_mode == 31 {
            Self::step_neon_speaker_envelopes(
                &mut self.neon_spatial_env,
                &uniforms.spatial_channels,
                frame_dt,
            );
            uniforms.spatial_channels = self.neon_spatial_env;
        }

        if state.visualizer_mode == 19 {
            let actual_ch = state.channel_vus.len().clamp(2, 32);
            uniforms.num_channels = actual_ch as u32;
            for (disp_idx, &src_idx) in display_order.iter().enumerate() {
                if disp_idx < 32 {
                    uniforms.channels[disp_idx] = if src_idx < self.vu_needle_angles.len() {
                        self.vu_needle_angles[src_idx]
                    } else {
                        0.0
                    };
                }
            }
            let (needle_l, needle_r) = if let Some(num_tracks) =
                state.tracker_channels.filter(|&n| n > 0 && !upmix_active)
                && self.vu_needle_angles.len() > num_tracks as usize + 1
            {
                (
                    self.vu_needle_angles[0],
                    self.vu_needle_angles[self.vu_needle_angles.len() - 1],
                )
            } else {
                (
                    self.vu_needle_angles.first().copied().unwrap_or(0.0),
                    self.vu_needle_angles.get(1).copied().unwrap_or(0.0),
                )
            };
            uniforms.spatial_channels[0] = needle_l;
            uniforms.spatial_channels[1] = needle_r;
        }

        if uniforms.num_channels != self.vu_prev_num_channels {
            self.vu_prev_active_segments = [0; 32];
            self.vu_segment_glows = [[0.0; 30]; 32];
            self.vu_prev_num_channels = uniforms.num_channels;
        }
        uniforms.vu_segment_glows = Self::step_vu_phosphor_ghosts(
            &mut self.vu_prev_active_segments,
            &mut self.vu_segment_glows,
            &uniforms.channels,
            uniforms.num_channels,
            frame_dt,
        );

        if state.visualizer_mode == 23 {
            let display_secs = state.scrub_target_seconds.unwrap_or(state.current_seconds);
            let active_idx = state
                .lyrics
                .as_ref()
                .and_then(|l| l.find_current_line_idx(display_secs));

            // Smooth 500+ FPS frame-interpolated slam timer accumulated in f64
            if active_idx != self.last_lyric_line_idx {
                self.last_lyric_line_idx = active_idx;
                self.lyric_slam_timer = 0.0;
            } else if !state.is_paused && state.file_loaded && !state.track_ended {
                self.lyric_slam_timer += self.smooth_dt.clamp(0.0005, 0.1);
            }

            let (char_bytes, line_progress, is_instrumental, line_duration) =
                if let Some(lyrics) = &state.lyrics {
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
                            let is_inst =
                                if (next_time - cur_time) > 4.5 && elapsed > 2.5 && gap > 1.5 {
                                    1.0
                                } else {
                                    0.0
                                };
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

            let slam_t_f64 = self.lyric_slam_timer;
            let bass = if state.spectrum_data.len() > 4 {
                (state.spectrum_data[1..5]
                    .iter()
                    .copied()
                    .fold(0.0f32, f32::max)
                    / 65.0)
                    .clamp(0.0, 1.5)
            } else {
                0.0
            };

            // Calculate exact slam vertical drop, water plunge, and continuous fluid buoyancy in f64
            let slam_y = if slam_t_f64 < 0.28 {
                // Gravity drop from y = 3.5 down to 0.0
                let t_norm = (slam_t_f64 / 0.28).clamp(0.0, 1.0) as f32;
                3.5 * (1.0 - t_norm * t_norm)
            } else {
                // Water impact, plunge and damped spring bobbing
                let dt_impact = slam_t_f64 - 0.28;
                let impact_spring =
                    (-0.32 * (-3.6 * dt_impact).exp() * (9.5 * dt_impact).cos()) as f32;
                // Gentle continuous idle buoyancy on the water surface after impact settles
                let settle = (1.0 - (-2.0 * dt_impact).exp()) as f32;
                let idle_bob = (((slam_t_f64 * 1.6).sin() * 0.022
                    + (slam_t_f64 * 0.85).cos() * 0.014) as f32)
                    * settle;
                let bass_swell = bass * 0.03 * settle;
                impact_spring + idle_bob + bass_swell
            };
            let slam_t = if slam_t_f64 > 10.0 {
                10.0 + ((slam_t_f64 - 10.0).rem_euclid(HOST_TIME_WRAP_PERIOD_SECS)) as f32
            } else {
                slam_t_f64 as f32
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
            uniforms.fire_heat[7] = ((char_count as f32) * 0.28).clamp(1.5, 5.0); // Estimated text half-width

            for (i, &b) in char_bytes.iter().take(64).enumerate() {
                uniforms.fire_heat[16 + i] = b as f32;
            }

            if self.current_lyric_mesh_text.as_bytes() != char_bytes.as_slice() {
                let active_text_str = String::from_utf8_lossy(&char_bytes).into_owned();
                let (vertices, indices) = generate_glass_lyrics_mesh(&active_text_str);
                let req_v_bytes = (vertices.len() * std::mem::size_of::<Vertex>()).max(64) as u64;
                let req_i_bytes = (indices.len() * std::mem::size_of::<u32>()).max(64) as u64;
                let index_count = indices.len() as u32;

                if let Some(existing) = self
                    .mesh_registry
                    .get_mut(&crate::state::Geometry::GlassLyricsScene)
                {
                    if req_v_bytes > existing.vertex_capacity {
                        let new_cap = (req_v_bytes * 3 / 2).max(4096);
                        existing.vertex_buffer =
                            self.device.create_buffer(&wgpu::BufferDescriptor {
                                label: Some("Mesh Vertex Buffer GlassLyricsScene"),
                                size: new_cap,
                                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                                mapped_at_creation: false,
                            });
                        existing.vertex_capacity = new_cap;
                    }
                    if !vertices.is_empty() {
                        self.queue.write_buffer(
                            &existing.vertex_buffer,
                            0,
                            bytemuck::cast_slice(&vertices),
                        );
                    }

                    if req_i_bytes > existing.index_capacity {
                        let new_cap = (req_i_bytes * 3 / 2).max(4096);
                        existing.index_buffer =
                            self.device.create_buffer(&wgpu::BufferDescriptor {
                                label: Some("Mesh Index Buffer GlassLyricsScene"),
                                size: new_cap,
                                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                                mapped_at_creation: false,
                            });
                        existing.index_capacity = new_cap;
                    }
                    if !indices.is_empty() {
                        self.queue.write_buffer(
                            &existing.index_buffer,
                            0,
                            bytemuck::cast_slice(&indices),
                        );
                    }
                    existing.index_count = index_count;
                } else {
                    let vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Mesh Vertex Buffer GlassLyricsScene"),
                        size: req_v_bytes,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    self.queue
                        .write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));
                    let index_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Mesh Index Buffer GlassLyricsScene"),
                        size: req_i_bytes,
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    self.queue
                        .write_buffer(&index_buffer, 0, bytemuck::cast_slice(&indices));
                    self.mesh_registry.insert(
                        crate::state::Geometry::GlassLyricsScene,
                        MeshBuffers {
                            vertex_buffer,
                            index_buffer,
                            vertex_capacity: req_v_bytes,
                            index_capacity: req_i_bytes,
                            index_count,
                        },
                    );
                }

                let lyrics_indirect: [u32; 5] = [index_count, 1, 0, 0, 0];
                self.queue.write_buffer(
                    &self.lyrics_indirect_args_buffer,
                    0,
                    bytemuck::cast_slice(&lyrics_indirect),
                );
                self.current_lyric_mesh_text = active_text_str;
            }
        }

        let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
        if vis_def.id == 32 {
            let lr_pair = if state.tracker_channels.is_some() && state.channel_vus.len() >= 2 {
                [
                    state.channel_vus[0],
                    state.channel_vus[state.channel_vus.len() - 1],
                ]
            } else {
                let l = state.channel_vus.first().copied().unwrap_or(0.0);
                let r = state.channel_vus.get(1).copied().unwrap_or(l);
                [l, r]
            };
            let (fd_params, substeps) = self.flip_disc_sim.step(
                &state.spectrum_data,
                &lr_pair,
                frame_dt,
                self.last_aspect_ratio,
                !state.is_paused && !state.track_ended,
                uniforms.smooth_time,
                self.flip_disc_initialized,
                state.flip_disc_mode,
            );
            self.flip_disc_pending_substeps = substeps;
            self.queue.write_buffer(
                &self.flip_disc_params_buffer,
                0,
                bytemuck::cast_slice(&[fd_params]),
            );
        }
        if vis_def.requires_acoustic_waves {
            Self::extract_acoustic_wave_phases(
                &mut self.channel_phases,
                &state.raw_audio_channels,
                &state.spectrum_data,
                &uniforms.spatial_channels,
                uniforms.num_spatial_channels,
                frame_dt,
                !state.is_paused && !state.track_ended,
                uniforms.smooth_time,
            );
            uniforms.channel_phases = self.channel_phases;
        }
        let vis_width = state.visual_width.clamp(128, 2048);
        uniforms.waveform_resolution = vis_width;
        let max_hist_rows = if vis_def.id == 3 { 96usize } else { 144usize };
        let hist_len = state.waveform_history.len();
        let logical_rows = if vis_def.id == 3 {
            96usize
        } else {
            hist_len.min(WAVEFORM_HISTORY_RING_CAP)
        };
        uniforms.waveform_history_size = logical_rows as u32;
        uniforms.step_fraction = step_fraction;

        // Only upload waveform history when the active visualizer requires it,
        // and only when new rows were pushed or the width/mode changed (uploads
        // only newly pushed 8KB rows into the 144-slot GPU ring buffer in steady state).
        let history_dirty = is_waveform_history_dirty(
            state.waveform_history_push_count,
            self.last_uploaded_push_count,
            hist_len,
            self.last_uploaded_hist_len,
            vis_width,
            self.last_uploaded_vis_width,
            vis_def.id,
            self.last_uploaded_vis_id,
        );
        if vis_def.id == 24 && (!state.is_paused || history_dirty) {
            crate::audio::update_lookahead_timeline_from_queue(
                &state.lookahead_queue,
                &state.raw_waveform,
                &state.raw_channel_vus,
                self.smooth_audio_seconds,
                &mut self.tape_timeline_scratch,
            );
            self.last_uploaded_push_count = state.waveform_history_push_count;
            self.last_uploaded_vis_width = vis_width;
            self.last_uploaded_vis_id = vis_def.id;
            self.last_uploaded_hist_len = usize::MAX;
            self.waveform_ring_head = 0;
            self.waveform_ring_base = 0;
            self.queue.write_buffer(
                &self.waveform_storage_buffer,
                0,
                bytemuck::cast_slice(&self.tape_timeline_scratch),
            );
        } else if vis_def.requires_history && history_dirty {
            let new_rows = state
                .waveform_history_push_count
                .wrapping_sub(self.last_uploaded_push_count);
            let smooth_rows = vis_def.id != 3;
            let can_append_ring = vis_def.id == self.last_uploaded_vis_id
                && vis_width == self.last_uploaded_vis_width
                && self.last_uploaded_hist_len > 0
                && (1..=16).contains(&new_rows)
                && hist_len
                    == (self.last_uploaded_hist_len + new_rows as usize)
                        .min(WAVEFORM_HISTORY_RING_CAP);

            self.last_uploaded_push_count = state.waveform_history_push_count;
            self.last_uploaded_vis_width = vis_width;
            self.last_uploaded_vis_id = vis_def.id;
            self.last_uploaded_hist_len = hist_len;

            let visual_width_usize = vis_width as usize;
            let format_wave_row = |dst: &mut [f32], wave: &[f32]| {
                let wave_len = wave.len().min(visual_width_usize);
                if wave_len > 0 {
                    dst[..wave_len].copy_from_slice(&wave[..wave_len]);
                    if wave_len < visual_width_usize {
                        dst[wave_len..visual_width_usize].fill(0.0);
                    }
                    // Simple pre-smoothing inline for 2D CRT trace (skip for 3D Oscilloscope
                    // so 3D ridges retain crisp transient height)
                    if smooth_rows && wave_len > 2 {
                        let mut prev = dst[0];
                        for j in 1..wave_len - 1 {
                            let curr = dst[j];
                            let next = dst[j + 1];
                            dst[j] = (prev + curr * 2.0 + next) / 4.0;
                            prev = curr;
                        }
                    }
                } else {
                    dst[..visual_width_usize].fill(0.0);
                }
            };

            if can_append_ring {
                let append_count = new_rows as usize;
                for i in 0..append_count {
                    let wave = &state.waveform_history[hist_len - append_count + i];
                    let offset = i * 2048;
                    format_wave_row(&mut self.waveform_history_flat[offset..offset + 2048], wave);
                    if visual_width_usize < 2048 {
                        self.waveform_history_flat[offset + visual_width_usize..offset + 2048]
                            .fill(0.0);
                    }
                    pack_wave_row_f16(
                        &self.waveform_history_flat[offset..offset + 2048],
                        &mut self.waveform_history_packed[i * 1024..(i + 1) * 1024],
                    );
                }
                let first_rows =
                    append_count.min(WAVEFORM_HISTORY_RING_CAP - self.waveform_ring_head);
                self.queue.write_buffer(
                    &self.waveform_storage_buffer,
                    (self.waveform_ring_head * 1024 * 4) as u64,
                    bytemuck::cast_slice(&self.waveform_history_packed[..first_rows * 1024]),
                );
                if first_rows < append_count {
                    self.queue.write_buffer(
                        &self.waveform_storage_buffer,
                        0,
                        bytemuck::cast_slice(
                            &self.waveform_history_packed[first_rows * 1024..append_count * 1024],
                        ),
                    );
                }
                self.waveform_ring_head =
                    (self.waveform_ring_head + append_count) % WAVEFORM_HISTORY_RING_CAP;
                self.waveform_ring_base =
                    compute_waveform_ring_base(self.waveform_ring_head, logical_rows);
            } else {
                // Full linear resync: upload up to max_hist_rows most recent frames
                // (right-aligned with leading zero-padding for 3D Oscilloscope), packed 2× as f16.
                let start = hist_len.saturating_sub(max_hist_rows);
                let count = hist_len.min(max_hist_rows);
                let pad_slots = if vis_def.id == 3 {
                    max_hist_rows.saturating_sub(count)
                } else {
                    0
                };
                if pad_slots > 0 {
                    self.waveform_history_flat[..pad_slots * 2048].fill(0.0);
                    self.waveform_history_packed[..pad_slots * 1024].fill(0);
                }

                for (i, wave) in state
                    .waveform_history
                    .iter()
                    .skip(start)
                    .enumerate()
                    .take(max_hist_rows)
                {
                    let slot = pad_slots + i;
                    let offset = slot * 2048;
                    format_wave_row(&mut self.waveform_history_flat[offset..offset + 2048], wave);
                    if visual_width_usize < 2048 {
                        self.waveform_history_flat[offset + visual_width_usize..offset + 2048]
                            .fill(0.0);
                    }
                    pack_wave_row_f16(
                        &self.waveform_history_flat[offset..offset + 2048],
                        &mut self.waveform_history_packed[slot * 1024..(slot + 1) * 1024],
                    );
                }
                let total_slots = pad_slots + count;
                if total_slots > 0 {
                    self.queue.write_buffer(
                        &self.waveform_storage_buffer,
                        0,
                        bytemuck::cast_slice(&self.waveform_history_packed[..total_slots * 1024]),
                    );
                }
                self.waveform_ring_head = total_slots % WAVEFORM_HISTORY_RING_CAP;
                self.waveform_ring_base = 0;
            }
        }

        uniforms.waveform_history_ring_base = self.waveform_ring_base;
        self.pending_uniforms = Some(uniforms);

        if vis_def.requires_fire && (!state.is_paused || self.fire_intensity > 0.001) {
            if vis_def.id == 5 {
                let start_sim = std::time::Instant::now();
                self.update_retro_fire(state, &uniforms);
                let elapsed_us = start_sim.elapsed().as_micros() as f32;
                self.cached_fire_us = Some(elapsed_us);
            } else {
                let bass_sum: f32 = uniforms.fire_heat[..64].iter().sum();
                let bass = (bass_sum / 64.0 / 100.0).min(1.0);
                let mids_sum: f32 = uniforms.fire_heat[64..512].iter().sum();
                let mids = (mids_sum / 448.0 / 100.0).min(1.0);
                let highs_sum: f32 = uniforms.fire_heat[512..1024].iter().sum();
                let highs = (highs_sum / 512.0 / 100.0).min(1.0);

                let n_ch = state.channel_vus.len().clamp(1, 32);
                let lfe_idx =
                    if (n_ch == 6 || n_ch == 8 || n_ch == 16) && state.tracker_channels.is_none() {
                        3
                    } else {
                        999
                    };

                let mut fire_params = FireParams {
                    bass,
                    mids,
                    highs,
                    time: wrap_host_phase_time(self.play_time),
                    cooling_factor: 1.0 - mids * 0.5,
                    turb_spread_f: self.fire_intensity,
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

                self.queue.write_buffer(
                    &self.fire_params_buffer,
                    0,
                    bytemuck::cast_slice(&[fire_params]),
                );
            }
        }

        // GPU spectrum is consumed only by the firesim compute (id 6) and the
        // resynth compute (requires_resynth, id 4) — skip the 256KB upload otherwise,
        // and only upload when new audio FFT data arrived (history_dirty)
        if (vis_def.id == 6 || vis_def.requires_resynth)
            && !state.gpu_spectrum_data.is_empty()
            && history_dirty
        {
            self.last_uploaded_push_count = state.waveform_history_push_count;
            self.last_uploaded_vis_width = vis_width;
            self.last_uploaded_vis_id = vis_def.id;
            self.queue.write_buffer(
                &self._gpu_spectrum_buffer,
                0,
                bytemuck::cast_slice(&state.gpu_spectrum_data),
            );
        }

        if let Some(rx) = &state.video_frame_rx {
            let mut latest_frame = None;
            while let Ok(frame) = rx.try_recv() {
                if let Some(old_frame) = latest_frame.take()
                    && let Some(tx) = &state.free_video_frame_tx
                {
                    let _ = tx.try_send(old_frame);
                }
                latest_frame = Some(frame);
            }

            if let Some(frame) = latest_frame {
                if frame.width >= 2 && frame.height >= 2 {
                    let use_16bit =
                        frame.bit_depth > 8 && self.negotiated_features.texture_format_16bit_norm;
                    let effective_bit_depth = if frame.bit_depth > 8 && !use_16bit {
                        8u32
                    } else {
                        frame.bit_depth as u32
                    };
                    let is_semi_planar = frame.chroma_layout.is_semi_planar();
                    let needs_init = !self.video_state.as_ref().is_some_and(|vs| {
                        vs.width == frame.width
                            && vs.height == frame.height
                            && vs.use_16bit_textures == use_16bit
                            && vs.chroma_layout.is_semi_planar() == is_semi_planar
                    });
                    if needs_init {
                        let y_format = if use_16bit {
                            wgpu::TextureFormat::R16Unorm
                        } else {
                            wgpu::TextureFormat::R8Unorm
                        };
                        let u_format = match (is_semi_planar, use_16bit) {
                            (true, true) => wgpu::TextureFormat::Rg16Unorm,
                            (true, false) => wgpu::TextureFormat::Rg8Unorm,
                            (false, true) => wgpu::TextureFormat::R16Unorm,
                            (false, false) => wgpu::TextureFormat::R8Unorm,
                        };
                        let v_format = if is_semi_planar {
                            wgpu::TextureFormat::R8Unorm
                        } else {
                            y_format
                        };
                        let y_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("Video Y Texture"),
                            size: wgpu::Extent3d {
                                width: frame.width,
                                height: frame.height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: y_format,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });
                        let u_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some(if is_semi_planar {
                                "Video UV Texture (NV12/NV21/P010)"
                            } else {
                                "Video U Texture"
                            }),
                            size: wgpu::Extent3d {
                                width: frame.width / 2,
                                height: frame.height / 2,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: u_format,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });
                        let v_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some(if is_semi_planar {
                                "Video V Placeholder 1x1"
                            } else {
                                "Video V Texture"
                            }),
                            size: wgpu::Extent3d {
                                width: if is_semi_planar { 1 } else { frame.width / 2 },
                                height: if is_semi_planar { 1 } else { frame.height / 2 },
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: v_format,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });
                        let border_blur_texture =
                            self.device.create_texture(&wgpu::TextureDescriptor {
                                label: Some("Video Quarter-Res Border Blur Texture"),
                                size: wgpu::Extent3d {
                                    width: (frame.width / 4).max(1),
                                    height: (frame.height / 4).max(1),
                                    depth_or_array_layers: 1,
                                },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                    | wgpu::TextureUsages::TEXTURE_BINDING,
                                view_formats: &[],
                            });
                        let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let u_view = u_texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let v_view = v_texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let border_blur_view = border_blur_texture
                            .create_view(&wgpu::TextureViewDescriptor::default());

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

                        let blur_bind_group =
                            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("Video Border Blur Bind Group"),
                                layout: &self.video_blur_bind_group_layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::TextureView(&y_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(&u_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::TextureView(&v_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 3,
                                        resource: wgpu::BindingResource::Sampler(&sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 4,
                                        resource: params_buffer.as_entire_binding(),
                                    },
                                ],
                            });

                        let bind_group =
                            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("Video Bind Group"),
                                layout: &self.video_bind_group_layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::TextureView(&y_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(&u_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::TextureView(&v_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 3,
                                        resource: wgpu::BindingResource::Sampler(&sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 4,
                                        resource: params_buffer.as_entire_binding(),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 5,
                                        resource: wgpu::BindingResource::TextureView(
                                            &border_blur_view,
                                        ),
                                    },
                                ],
                            });

                        self.video_state = Some(VideoState {
                            y_texture,
                            u_texture,
                            v_texture,
                            border_blur_texture,
                            border_blur_view,
                            blur_bind_group,
                            bind_group,
                            params_buffer,
                            last_uploaded_params: None,
                            width: frame.width,
                            height: frame.height,
                            rotation: frame.rotation,
                            color_space: frame.color_space,
                            color_range: frame.color_range,
                            bit_depth: effective_bit_depth,
                            color_trc: frame.color_trc,
                            chroma_layout: frame.chroma_layout,
                            use_16bit_textures: use_16bit,
                            frame_generation: 0,
                            blurred_generation: u64::MAX,
                            blurred_color_key: (u32::MAX, 0, 0, 0, 0),
                        });
                    } else if let Some(vs) = &mut self.video_state {
                        vs.rotation = frame.rotation;
                        vs.color_space = frame.color_space;
                        vs.color_range = frame.color_range;
                        vs.bit_depth = effective_bit_depth;
                        vs.color_trc = frame.color_trc;
                        vs.chroma_layout = frame.chroma_layout;
                    }

                    if let Some(vs) = &mut self.video_state {
                        vs.frame_generation = if frame.generation != 0 {
                            frame.generation
                        } else {
                            vs.frame_generation.wrapping_add(1).max(1)
                        };

                        let upload_plane =
                            |queue: &wgpu::Queue,
                             scratch: &mut Vec<u8>,
                             tex: &wgpu::Texture,
                             plane: &[u8],
                             src_stride: usize,
                             tex_w: u32,
                             tex_h: u32,
                             channels: usize,
                             src_16bit: bool,
                             dst_16bit: bool,
                             src_bit_depth: u8| {
                                let dst_bps = if dst_16bit { 2 } else { 1 };
                                let dst_row_bytes = tex_w as usize * channels * dst_bps;
                                let aligned_stride =
                                    crate::state::align_wgpu_row_bytes(dst_row_bytes);
                                let rows = tex_h as usize;

                                if src_16bit == dst_16bit
                                    && src_stride == aligned_stride
                                    && plane.len() >= aligned_stride * rows
                                {
                                    // Zero-copy / zero-repack fast path (FFmpeg & Android decoders pre-align to 256B)
                                    queue.write_texture(
                                        wgpu::TexelCopyTextureInfo {
                                            texture: tex,
                                            mip_level: 0,
                                            origin: wgpu::Origin3d::ZERO,
                                            aspect: wgpu::TextureAspect::All,
                                        },
                                        &plane[..aligned_stride * rows],
                                        wgpu::TexelCopyBufferLayout {
                                            offset: 0,
                                            bytes_per_row: Some(aligned_stride as u32),
                                            rows_per_image: Some(tex_h),
                                        },
                                        wgpu::Extent3d {
                                            width: tex_w,
                                            height: tex_h,
                                            depth_or_array_layers: 1,
                                        },
                                    );
                                    return;
                                }

                                let needed = aligned_stride * rows;
                                if scratch.len() < needed {
                                    scratch.resize(needed, 0);
                                }
                                if src_16bit && !dst_16bit {
                                    // Graceful 16-bit -> 8-bit fallback for GPUs without TEXTURE_FORMAT_16BIT_NORM
                                    let shift = match src_bit_depth {
                                        10 => 2u32,
                                        12 => 4u32,
                                        _ => 8u32,
                                    };
                                    let samples_per_row = tex_w as usize * channels;
                                    for r in 0..rows {
                                        let src_row = r * src_stride;
                                        let dst_row = r * aligned_stride;
                                        for s in 0..samples_per_row {
                                            let idx = src_row + s * 2;
                                            if idx + 1 < plane.len() {
                                                let v16 = u16::from_le_bytes([
                                                    plane[idx],
                                                    plane[idx + 1],
                                                ]);
                                                scratch[dst_row + s] =
                                                    ((v16 >> shift).min(255)) as u8;
                                            }
                                        }
                                    }
                                } else {
                                    for r in 0..rows {
                                        let src_start = r * src_stride;
                                        let dst_start = r * aligned_stride;
                                        if src_start < plane.len() {
                                            let copy_len =
                                                dst_row_bytes.min(plane.len() - src_start);
                                            scratch[dst_start..dst_start + copy_len]
                                                .copy_from_slice(
                                                    &plane[src_start..src_start + copy_len],
                                                );
                                        }
                                    }
                                }
                                queue.write_texture(
                                    wgpu::TexelCopyTextureInfo {
                                        texture: tex,
                                        mip_level: 0,
                                        origin: wgpu::Origin3d::ZERO,
                                        aspect: wgpu::TextureAspect::All,
                                    },
                                    &scratch[..needed],
                                    wgpu::TexelCopyBufferLayout {
                                        offset: 0,
                                        bytes_per_row: Some(aligned_stride as u32),
                                        rows_per_image: Some(tex_h),
                                    },
                                    wgpu::Extent3d {
                                        width: tex_w,
                                        height: tex_h,
                                        depth_or_array_layers: 1,
                                    },
                                );
                            };

                        let src_16bit = frame.bit_depth > 8;
                        upload_plane(
                            &self.queue,
                            &mut self.video_repack_scratch,
                            &vs.y_texture,
                            &frame.y_plane,
                            frame.y_stride,
                            frame.width,
                            frame.height,
                            1,
                            src_16bit,
                            use_16bit,
                            frame.bit_depth,
                        );
                        upload_plane(
                            &self.queue,
                            &mut self.video_repack_scratch,
                            &vs.u_texture,
                            &frame.u_plane,
                            frame.u_stride,
                            frame.width / 2,
                            frame.height / 2,
                            if is_semi_planar { 2 } else { 1 },
                            src_16bit,
                            use_16bit,
                            frame.bit_depth,
                        );
                        if !is_semi_planar {
                            upload_plane(
                                &self.queue,
                                &mut self.video_repack_scratch,
                                &vs.v_texture,
                                &frame.v_plane,
                                frame.v_stride,
                                frame.width / 2,
                                frame.height / 2,
                                1,
                                src_16bit,
                                use_16bit,
                                frame.bit_depth,
                            );
                        }
                    }
                }

                if let Some(tx) = &state.free_video_frame_tx {
                    let _ = tx.try_send(frame);
                }
            }
        }
    }

    #[allow(clippy::type_complexity)]
    pub fn render(
        &mut self,
        window: &winit::window::Window,
        egui_ctx: &egui::Context,
        egui_state: &mut egui_winit::State,
        state: &AppState,
        file_dialog: &mut egui_file_dialog::FileDialog,
        gamepad_events: Vec<egui::Event>,
    ) -> Result<
        (
            Vec<EngineAction>,
            f32,
            f32,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            f32,
            f32,
            f32,
            f32,
        ),
        wgpu::SurfaceStatus,
    > {
        let physical_size = window.inner_size();
        if physical_size.width > 0 && physical_size.height > 0 && physical_size != self.size {
            self.resize(physical_size);
        }
        if self.surface.is_none() {
            return Err(wgpu::SurfaceStatus::Lost);
        }

        let mut fft_shader_time_us = self.cached_fft_us;
        let mut fire_shader_time_us = self.cached_fire_us;
        let mut vis_shader_time_us = self.cached_vis_us;

        // NON-BLOCKING timestamp readback: poll without waiting, check if mapping completed
        if self.timestamp_mapping_active {
            // Non-blocking poll to process any completed GPU work
            let _ = self.device.poll(wgpu::PollType::Poll);

            if self.timestamp_map_complete.load(Ordering::Acquire)
                && let Some(read_buffer) = &self.query_read_buffer
            {
                let slice = read_buffer.slice(..);
                if let Ok(data) = slice.get_mapped_range() {
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
                }
                read_buffer.unmap();
                self.timestamp_mapping_active = false;
            }
            // If mapping not yet complete, we use cached values from last successful read
        }

        // Process egui UI
        let ui_start = std::time::Instant::now();
        let mut raw_input = egui_state.take_egui_input(window);
        if (state.hovered_file.is_some() || !raw_input.hovered_files.is_empty())
            && let Some([x, y]) = state.drag_cursor_pos
        {
            raw_input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(x, y)));
        }
        raw_input.events.extend(gamepad_events);
        let mut central_rect = egui::Rect::from_min_max(
            Default::default(),
            egui::pos2(self.config.width as f32, self.config.height as f32),
        );
        let mut engine_actions: Vec<EngineAction> = Vec::new();

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
            let (disp_w, disp_h) = if vs.rotation == 90 || vs.rotation == 270 {
                (vs.height, vs.width)
            } else {
                (vs.width, vs.height)
            };
            let disp_bits = if vs.bit_depth == 16 { 10 } else { vs.bit_depth };
            let fps_suffix = if state.stats.video_fps > 0.0 {
                if (state.stats.video_fps - state.stats.video_fps.round()).abs() < 0.05 {
                    format!(" @ {:.0} fps", state.stats.video_fps.round())
                } else {
                    format!(" @ {:.2} fps", state.stats.video_fps)
                }
            } else {
                String::new()
            };
            let hw_suffix = if !state.stats.video_hw_decoder.is_empty() {
                format!(" | {}", state.stats.video_hw_decoder)
            } else {
                String::new()
            };
            video_info_str = Some(format!(
                "{}x{}{} | {} {}-bit {}{}",
                disp_w, disp_h, fps_suffix, cs, disp_bits, cr, hw_suffix
            ));
        }

        let mut full_output = egui_ctx.run_ui(raw_input, |ctx| {
            self.build_ui_overlay(
                ctx,
                state,
                file_dialog,
                UiOverlayContext {
                    central_rect: &mut central_rect,
                    engine_actions: &mut engine_actions,
                    out_meters_rect: &mut out_meters_rect,
                    out_fire_rect: &mut out_fire_rect,
                    out_heatmap_rect: &mut out_heatmap_rect,
                    out_track_info_rect: &mut out_track_info_rect,
                    out_top_panel_rect: &mut out_top_panel_rect,
                    out_video_rect: &mut out_video_rect,
                    vis_name,
                    video_info_str: &video_info_str,
                },
            );
        });

        self.track_info_rect = out_track_info_rect.map(|r| [r.min.x, r.min.y, r.max.x, r.max.y]);

        let scale = egui_ctx.pixels_per_point();
        let w = self.config.width as f32;
        let h = self.config.height as f32;

        if let Some(r) = out_meters_rect {
            self.meters_uv_rect = [
                (r.min.x * scale) / w,
                (r.min.y * scale) / h,
                (r.max.x * scale) / w,
                (r.max.y * scale) / h,
            ];
        } else {
            self.meters_uv_rect = [0.0; 4];
        }

        if let Some(r) = out_fire_rect {
            self.fire_uv_rect = [
                (r.min.x * scale) / w,
                (r.min.y * scale) / h,
                (r.max.x * scale) / w,
                (r.max.y * scale) / h,
            ];
        } else {
            self.fire_uv_rect = [0.0; 4];
        }

        if let Some(r) = out_heatmap_rect {
            self.heatmap_uv_rect = [
                (r.min.x * scale) / w,
                (r.min.y * scale) / h,
                (r.max.x * scale) / w,
                (r.max.y * scale) / h,
            ];
        } else {
            self.heatmap_uv_rect = [0.0; 4];
        }

        egui_state.handle_platform_output(window, full_output.platform_output);
        let mut clipped_primitives =
            egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        if !state.file_loaded {
            clipped_primitives.insert(
                0,
                self.splash_cache.build_combined_bg_primitive(
                    egui_ctx,
                    central_rect,
                    self.smooth_time as f32,
                ),
            );
        }

        let ui_elapsed = ui_start.elapsed().as_micros() as f32;
        let phase_egui_layout_us = ui_elapsed; // Egui layout now accurately measures only UI logic and tessellation

        for (id, image_deltas) in full_output.textures_delta.set.drain() {
            for image_delta in &image_deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, id, image_delta);
            }
        }
        let textures_to_free = std::mem::take(&mut full_output.textures_delta.free);

        // Acquire swapchain image AFTER CPU UI layout & tessellation ("Late Swapchain Acquire")
        // so Egui layout overlaps with the previous frame's GPU execution and VSync fence wait,
        // and GPU command submission happens immediately after VSync unblocks.
        let surface_start = std::time::Instant::now();
        let surface = self.surface.as_ref().ok_or(wgpu::SurfaceStatus::Lost)?;
        let output = surface.get_current_texture();
        let surface_texture = match output {
            wgpu::CurrentSurfaceTexture::Success(tex)
            | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            wgpu::CurrentSurfaceTexture::Lost => return Err(wgpu::SurfaceStatus::Lost),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(wgpu::SurfaceStatus::Outdated),
            wgpu::CurrentSurfaceTexture::Timeout => return Err(wgpu::SurfaceStatus::Timeout),
            _ => return Err(wgpu::SurfaceStatus::Lost),
        };
        if cfg!(windows)
            && is_vsync_present_mode(self.config.present_mode)
            && crate::state::BENCH_FRAMES_OVERRIDE.load(Ordering::Relaxed) == 0
            && std::env::var_os("BENCH_FRAMES").is_none()
        {
            // In windowed mode over a lower-FPS VRR game, DWM FLIP_DISCARD can occasionally
            // recycle a queued backbuffer early and signal the waitable swapchain semaphore
            // before the display's physical minimum refresh period (1 / target_fps) has elapsed.
            // Clamp early wakeups to the monitor's max refresh rate ceiling (e.g. 144 Hz = 6.944 ms).
            let min_frame_time =
                std::time::Duration::from_secs_f32(1.0 / (state.target_fps.clamp(30, 500) as f32));
            if let Some(last_acq) = self.last_acquire_instant {
                let target_instant = last_acq + min_frame_time;
                let now = std::time::Instant::now();
                if target_instant > now {
                    let rem = target_instant - now;
                    let spin_margin = std::time::Duration::from_micros(1500);
                    if rem > spin_margin {
                        std::thread::sleep(rem - spin_margin);
                    }
                    while std::time::Instant::now() < target_instant {
                        std::hint::spin_loop();
                    }
                }
            }
            self.last_acquire_instant = Some(std::time::Instant::now());
        } else {
            self.last_acquire_instant = None;
        }
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let phase_surface_us = surface_start.elapsed().as_micros() as f32;

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: egui_ctx.pixels_per_point(),
        };
        let render_start = std::time::Instant::now();

        let scale_factor = egui_ctx.pixels_per_point();
        let vp_x =
            ((central_rect.min.x * scale_factor).clamp(0.0, self.config.width as f32)).round();
        let vp_y =
            ((central_rect.min.y * scale_factor).clamp(0.0, self.config.height as f32)).round();
        let max_w = (self.config.width as f32 - vp_x).max(1.0);
        let vp_w = ((central_rect.width() * scale_factor).clamp(1.0, max_w)).round();
        let max_h = (self.config.height as f32 - vp_y).max(1.0);
        let vp_h = ((central_rect.height() * scale_factor).clamp(1.0, max_h)).round();

        let is_portrait = self.config.width < self.config.height;
        let is_video_active = state.file_loaded
            && self.video_state.is_some()
            && (state.video_mode > 0
                || (is_portrait && state.mobile_hud_tab == crate::state::MobileHudTab::Video));

        let mut v_vp_x = 0.0;
        let mut v_vp_y = 0.0;
        let mut v_vp_w = self.config.width as f32;
        let mut v_vp_h = self.config.height as f32;

        if state.file_loaded && is_video_active {
            let target_rect = if state.video_mode == 3 {
                None // mode 3: full screen
            } else if is_portrait && state.mobile_hud_tab == crate::state::MobileHudTab::Video {
                out_video_rect
            } else if state.video_mode == 1 {
                out_track_info_rect
            } else if state.video_mode == 2 {
                out_video_rect.or(out_top_panel_rect)
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
        }

        if state.file_loaded {
            let mode_idx = state
                .current_visualizer_idx
                .min(self.render_pipelines.len() - 1);
            self.ensure_visualizer_pipeline(mode_idx);

            let aspect = vp_w / vp_h.max(1.0);
            self.last_aspect_ratio = aspect;

            if let Some(mut uniforms) = self.pending_uniforms.take() {
                uniforms.ui_meters_rect = self.meters_uv_rect;
                uniforms.ui_heatmap_rect = self.heatmap_uv_rect;
                uniforms.ui_fire_rect = self.fire_uv_rect;
                uniforms.aspect_ratio = aspect;
                self.queue
                    .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
            }

            let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
            if matches!(
                vis_def.pipeline_type,
                crate::state::PipelineType::Mesh3D { .. }
            ) {
                // Adapt FOV when aspect ratio is square or portrait (< 1.0) so 3D scenes fit without horizontal clipping
                let is_room_vis = state.visualizer_mode == 31;
                let racer_cam = if state.visualizer_mode == 14 {
                    Some(types::compute_synthwave_racer_camera(
                        self.play_time as f32,
                        aspect,
                        state.racer_camera,
                    ))
                } else {
                    None
                };
                let base_fov = if is_room_vis {
                    64.0_f32.to_radians()
                } else if let Some((_, _, _, fov_deg)) = racer_cam {
                    fov_deg.to_radians()
                } else {
                    48.0_f32.to_radians()
                };
                let fov_y = if state.visualizer_mode == 19
                    || state.visualizer_mode == 23
                    || state.visualizer_mode == 29
                {
                    base_fov
                } else if state.visualizer_mode == 13 && aspect < 1.35 {
                    (48.0 * (1.35 / aspect.clamp(0.70, 1.35)))
                        .min(76.0)
                        .to_radians()
                } else if aspect < 1.0 {
                    (base_fov / aspect.clamp(0.65, 1.0)).min(if is_room_vis {
                        85.0_f32.to_radians()
                    } else {
                        75.0_f32.to_radians()
                    })
                } else {
                    base_fov
                };
                let proj = glam::Mat4::perspective_rh_gl(fov_y, aspect, 0.1, 1000.0);
                let cam_view = if state.visualizer_mode == 20 {
                    let (eye, target, up) =
                        compute_biolum_camera(self.play_time, state.biolum_top_down);
                    glam::Mat4::look_at_rh(eye, target, up)
                } else if let Some((eye, target, up, _)) = racer_cam {
                    glam::Mat4::look_at_rh(eye, target, up)
                } else if state.visualizer_mode == 19 {
                    let vu_t = ((aspect - 0.85) / 0.40).clamp(0.0, 1.0);
                    let vu_smooth = vu_t * vu_t * (3.0 - 2.0 * vu_t);
                    let cam_z = ((12.0 + 1.5 * vu_smooth) / aspect).clamp(6.2, 22.0);
                    glam::Mat4::look_at_rh(
                        glam::Vec3::new(0.0, 0.0, cam_z),
                        glam::Vec3::new(0.0, 0.0, 0.0),
                        glam::Vec3::new(0.0, 1.0, 0.0),
                    )
                } else if state.visualizer_mode == 23 {
                    let lyrics_scale = (2.2 / aspect).max(1.0);
                    let eye_y = 0.45 + 0.95 * lyrics_scale;
                    let eye_z = 4.2 * lyrics_scale;
                    glam::Mat4::look_at_rh(
                        glam::Vec3::new(0.0, eye_y, eye_z),
                        glam::Vec3::new(0.0, 0.45, 0.0),
                        glam::Vec3::new(0.0, 1.0, 0.0),
                    )
                } else if state.visualizer_mode == 31 {
                    let (cam_eye, cam_target) = compute_neon_acoustic_studio_camera(self.play_time);
                    glam::Mat4::look_at_rh(cam_eye, cam_target, glam::Vec3::Y)
                } else if state.visualizer_mode == 29 {
                    glam::Mat4::look_at_rh(
                        glam::Vec3::new(0.0, 0.0, -6.5),
                        glam::Vec3::new(0.0, 0.0, 20.0),
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
                    view_matrix: cam_view.to_cols_array_2d(),
                    proj_matrix: proj.to_cols_array_2d(),
                };
                self.queue.write_buffer(
                    &self.camera_uniform_buffer,
                    0,
                    bytemuck::cast_slice(&[camera_uniforms]),
                );
            }

            if is_video_active && let Some(vs) = &mut self.video_state {
                let chroma_layout_u32 = vs.chroma_layout.shader_code();
                let params = VideoParams {
                    color_space: vs.color_space,
                    color_range: vs.color_range,
                    bit_depth: vs.bit_depth,
                    color_trc: vs.color_trc,
                    viewport_width: v_vp_w,
                    viewport_height: v_vp_h,
                    video_width: vs.width as f32,
                    video_height: vs.height as f32,
                    rotation: vs.rotation,
                    chroma_layout: chroma_layout_u32,
                    _pad2: 0.0,
                    _pad3: 0.0,
                };
                let params_bytes: [u8; std::mem::size_of::<VideoParams>()] = bytemuck::cast(params);
                if vs.last_uploaded_params != Some(params_bytes) {
                    self.queue.write_buffer(&vs.params_buffer, 0, &params_bytes);
                    vs.last_uploaded_params = Some(params_bytes);
                }
            }
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        // Pre-blur video borders at quarter resolution ONLY when a new decoded video frame
        // (generation) or color metadata arrives, eliminating 93 per-pixel fetches every refresh.
        if state.file_loaded
            && is_video_active
            && let Some(vs) = &mut self.video_state
        {
            let chroma_layout_u32 = vs.chroma_layout.shader_code();
            let color_key = (
                vs.color_space,
                vs.color_range,
                vs.bit_depth,
                vs.color_trc,
                chroma_layout_u32,
            );
            if vs.blurred_generation != vs.frame_generation || vs.blurred_color_key != color_key {
                const VIDEO_BLUR_CLEAR_COLOR: wgpu::Color = wgpu::Color::BLACK;
                let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Video Quarter-Res Border Blur Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &vs.border_blur_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(VIDEO_BLUR_CLEAR_COLOR),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                blur_pass.set_pipeline(&self.video_border_blur_pipeline);
                blur_pass.set_bind_group(0, &vs.blur_bind_group, &[]);
                blur_pass.draw(0..3, 0..1);
                drop(blur_pass);
                vs.blurred_generation = vs.frame_generation;
                vs.blurred_color_key = color_key;
            }
        }

        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &clipped_primitives,
            &screen_descriptor,
        );

        let is_bench =
            crate::state::BENCH_FRAMES_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) > 0;
        let should_start_mapping = state.file_loaded
            && (state.show_stats || is_bench)
            && !self.timestamp_mapping_active
            && self.query_set.is_some();
        let active_query_set = if should_start_mapping {
            self.query_set.as_ref()
        } else {
            None
        };

        // GPU compute passes (skipped when paused and idle so paused frames do minimal GPU work)
        if state.file_loaded {
            let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
            let need_heatmap_or_resynth =
                !state.is_paused || self.last_steps_to_fill > 0 || active_query_set.is_some();
            if need_heatmap_or_resynth {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Heatmap & Resynth Compute Pass"),
                    timestamp_writes: active_query_set.map(|qs| wgpu::ComputePassTimestampWrites {
                        query_set: qs,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }),
                });
                compute_pass.set_pipeline(&self.heatmap_compute_pipeline);
                compute_pass.set_bind_group(0, Some(&self.heatmap_bind_group), &[]);
                compute_pass.dispatch_workgroups(1, 1, 1); // 256x1x1 threads

                if vis_def.requires_resynth && !state.is_paused {
                    compute_pass.set_pipeline(&self.resynth_compute_pipeline);
                    compute_pass.set_bind_group(0, Some(&self.resynth_bind_group), &[]);
                    compute_pass.dispatch_workgroups(32, 2, 1); // 512/16=32, 32/16=2
                }
            }

            let mut sim_timestamp_written = false;
            let mut take_sim_timestamps = || {
                if sim_timestamp_written {
                    None
                } else {
                    active_query_set.map(|qs| {
                        sim_timestamp_written = true;
                        wgpu::ComputePassTimestampWrites {
                            query_set: qs,
                            beginning_of_pass_write_index: Some(2),
                            end_of_pass_write_index: Some(3),
                        }
                    })
                }
            };

            // GPU fire compute: dispatch simulation + copy result to texture
            if vis_def.requires_fire
                && (!state.is_paused || self.fire_intensity > 0.001)
                && vis_def.id == 6
            {
                {
                    let mut compute_pass =
                        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("Fire Compute"),
                            timestamp_writes: take_sim_timestamps(),
                        });
                    compute_pass.set_pipeline(&self.firesim_compute_pipeline);
                    let bg = if self.fire_ping {
                        &self.fire_bind_group_a
                    } else {
                        &self.fire_bind_group_b
                    };
                    compute_pass.set_bind_group(0, Some(bg), &[]);
                    compute_pass.dispatch_workgroups(64, 36, 1); // 1024/16=64, 576/16=36
                }
                let output_buffer = if self.fire_ping {
                    &self.fire_buffer_b
                } else {
                    &self.fire_buffer_a
                };
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
                    wgpu::Extent3d {
                        width: 1024,
                        height: 576,
                        depth_or_array_layers: 1,
                    },
                );
                self.fire_ping = !self.fire_ping;
            }

            if !state.is_paused && vis_def.requires_flip_water {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Acoustic FLIP Water Compute"),
                    timestamp_writes: take_sim_timestamps(),
                });
                compute_pass.set_pipeline(&self.flip_water_clear_pipeline);
                compute_pass.set_bind_group(0, Some(&self.flip_compute_bind_group), &[]);
                compute_pass.dispatch_workgroups(512, 1, 1);

                compute_pass.set_pipeline(&self.flip_water_clear_render_pipeline);
                compute_pass.set_bind_group(0, Some(&self.flip_compute_bind_group), &[]);
                compute_pass.dispatch_workgroups(1024, 1, 1);

                compute_pass.set_pipeline(&self.flip_water_p2g_pipeline);
                compute_pass.set_bind_group(0, Some(&self.flip_compute_bind_group), &[]);
                compute_pass.dispatch_workgroups(256, 1, 1);

                compute_pass.set_pipeline(&self.flip_water_forces_pipeline);
                compute_pass.set_bind_group(0, Some(&self.flip_compute_bind_group), &[]);
                compute_pass.dispatch_workgroups(512, 1, 1);

                for _ in 0..8 {
                    compute_pass.set_pipeline(&self.flip_water_pressure_0_to_1_pipeline);
                    compute_pass.set_bind_group(0, Some(&self.flip_compute_bind_group), &[]);
                    compute_pass.dispatch_workgroups(512, 1, 1);

                    compute_pass.set_pipeline(&self.flip_water_pressure_1_to_0_pipeline);
                    compute_pass.set_bind_group(0, Some(&self.flip_compute_bind_group), &[]);
                    compute_pass.dispatch_workgroups(512, 1, 1);
                }

                compute_pass.set_pipeline(&self.flip_water_project_pipeline);
                compute_pass.set_bind_group(0, Some(&self.flip_compute_bind_group), &[]);
                compute_pass.dispatch_workgroups(512, 1, 1);

                compute_pass.set_pipeline(&self.flip_water_g2p_advect_pipeline);
                compute_pass.set_bind_group(0, Some(&self.flip_compute_bind_group), &[]);
                compute_pass.dispatch_workgroups(256, 1, 1);
                drop(compute_pass);

                if vis_def.id == 23 {
                    encoder.copy_buffer_to_buffer(
                        &self.splash_compact_buffer,
                        0,
                        &self.splash_indirect_args_buffer,
                        0,
                        20,
                    );
                }
            }

            if vis_def.id == 20 && (!state.is_paused || !self.biolum_initialized) {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Bioluminescence Waves Sim Compute"),
                    timestamp_writes: take_sim_timestamps(),
                });
                compute_pass.set_pipeline(&self.biolum_compute_pipeline);
                compute_pass.set_bind_group(0, Some(&self.biolum_compute_bind_group), &[]);
                compute_pass.dispatch_workgroups(BIOLUM_WORKGROUPS, 1, 1);
                self.biolum_initialized = true;
            }

            if vis_def.id == 32 && (!state.is_paused || !self.flip_disc_initialized) {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Mechanical Flip-Disc Compute"),
                    timestamp_writes: take_sim_timestamps(),
                });
                if !self.flip_disc_initialized {
                    compute_pass.set_pipeline(&self.flip_disc_init_pipeline);
                    compute_pass.set_bind_group(0, Some(&self.flip_disc_compute_bg_a), &[]);
                    compute_pass.dispatch_workgroups(
                        FLIP_DISC_WORKGROUPS_X,
                        FLIP_DISC_WORKGROUPS_Y,
                        1,
                    );
                    compute_pass.set_bind_group(0, Some(&self.flip_disc_compute_bg_b), &[]);
                    compute_pass.dispatch_workgroups(
                        FLIP_DISC_WORKGROUPS_X,
                        FLIP_DISC_WORKGROUPS_Y,
                        1,
                    );
                    self.flip_disc_ping = true;
                    self.flip_disc_initialized = true;
                }
                if !state.is_paused {
                    let substeps = self.flip_disc_pending_substeps.min(2);
                    for _ in 0..substeps {
                        compute_pass.set_pipeline(&self.flip_disc_wave_pipeline);
                        let bg = if self.flip_disc_ping {
                            &self.flip_disc_compute_bg_a
                        } else {
                            &self.flip_disc_compute_bg_b
                        };
                        compute_pass.set_bind_group(0, Some(bg), &[]);
                        compute_pass.dispatch_workgroups(
                            FLIP_DISC_WORKGROUPS_X,
                            FLIP_DISC_WORKGROUPS_Y,
                            1,
                        );
                        self.flip_disc_ping = !self.flip_disc_ping;
                    }
                    let disc_bg = if self.flip_disc_ping {
                        &self.flip_disc_compute_bg_a
                    } else {
                        &self.flip_disc_compute_bg_b
                    };
                    compute_pass.set_pipeline(&self.flip_disc_step_pipeline);
                    compute_pass.set_bind_group(0, Some(disc_bg), &[]);
                    compute_pass.dispatch_workgroups(
                        FLIP_DISC_WORKGROUPS_X,
                        FLIP_DISC_WORKGROUPS_Y,
                        1,
                    );
                }
            }

            if vis_def.requires_acoustic_waves && state.room_volume_smoke {
                let needs_clear = !self.acoustic_wave_initialized;
                if needs_clear || !state.is_paused {
                    {
                        let mut compute_pass =
                            encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                label: Some("Acoustic Wave Chamber Compute"),
                                timestamp_writes: take_sim_timestamps(),
                            });
                        compute_pass.set_bind_group(
                            0,
                            Some(&self.acoustic_wave_compute_bind_group),
                            &[],
                        );
                        if needs_clear {
                            compute_pass.set_pipeline(&self.acoustic_wave_clear_pipeline);
                            compute_pass.dispatch_workgroups(8, 8, 16);
                            self.acoustic_wave_initialized = true;
                        }
                        if !state.is_paused {
                            compute_pass.set_pipeline(&self.acoustic_wave_step_ab_pipeline);
                            compute_pass.dispatch_workgroups(8, 8, 16);
                            compute_pass.set_pipeline(&self.acoustic_wave_step_ba_pipeline);
                            compute_pass.dispatch_workgroups(8, 8, 16);
                        }
                    }
                    encoder.copy_buffer_to_texture(
                        wgpu::TexelCopyBufferInfo {
                            buffer: &self.acoustic_packed_buffer,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(64 * 4),
                                rows_per_image: Some(32),
                            },
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &self.acoustic_volume_texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width: 64,
                            height: 32,
                            depth_or_array_layers: 64,
                        },
                    );
                }
            }

            if !sim_timestamp_written && let Some(qs) = active_query_set {
                encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Dummy Sim Pass"),
                    timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                        query_set: qs,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    }),
                });
            }
        }

        let is_mesh_3d = state.file_loaded
            && matches!(
                crate::state::VISUALIZERS[state.current_visualizer_idx].pipeline_type,
                crate::state::PipelineType::Mesh3D { .. }
            );

        // If a 3D mesh visualizer is active, run its depth-buffered pass first, then overlay 2D HUD + Egui
        // without depth attachment. Otherwise (2D visualizers or Splash), execute a SINGLE swapchain render pass
        // with no depth attachment for minimal GPU overhead.
        if is_mesh_3d {
            let vis_def = &crate::state::VISUALIZERS[state.current_visualizer_idx];
            let mode_idx = state
                .current_visualizer_idx
                .min(self.render_pipelines.len() - 1);
            let mut render_pass_3d = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3D Visualizer Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(APP_DEFAULT_BG_CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: active_query_set.map(|qs| wgpu::RenderPassTimestampWrites {
                    query_set: qs,
                    beginning_of_pass_write_index: Some(4),
                    end_of_pass_write_index: Some(5),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });

            render_pass_3d.set_viewport(vp_x, vp_y, vp_w, vp_h, 0.0, 1.0);
            render_pass_3d.set_scissor_rect(vp_x as u32, vp_y as u32, vp_w as u32, vp_h as u32);

            if vis_def.id == 20 {
                if let Some(bg_pipe) = &self.biolum_bg_pipeline {
                    render_pass_3d.set_pipeline(bg_pipe);
                    render_pass_3d.set_bind_group(0, &self.uniform_bind_group, &[]);
                    render_pass_3d.set_bind_group(1, &self.camera_bind_group, &[]);
                    render_pass_3d.draw(0..3, 0..1);
                }
                if let Some(ocean_pipe) = &self.biolum_ocean_pipeline {
                    render_pass_3d.set_pipeline(ocean_pipe);
                    render_pass_3d.set_bind_group(0, &self.uniform_bind_group, &[]);
                    render_pass_3d.set_bind_group(1, &self.camera_bind_group, &[]);
                    render_pass_3d.set_vertex_buffer(0, self.biolum_ocean_vertex_buffer.slice(..));
                    render_pass_3d.set_index_buffer(
                        self.biolum_ocean_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    render_pass_3d.draw_indexed(0..self.biolum_ocean_index_count, 0, 0..1);
                }
            } else if vis_def.id == 14 {
                render_pass_3d.set_pipeline(&self.synthwave_sky_pipeline);
                render_pass_3d.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass_3d.set_bind_group(1, &self.camera_bind_group, &[]);
                render_pass_3d.draw(0..3, 0..1);
            } else if vis_def.id == 19 {
                render_pass_3d.set_pipeline(&self.vumeters_bg_pipeline);
                render_pass_3d.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass_3d.draw(0..3, 0..1);
            } else if vis_def.id != 13 && vis_def.id != 29 && vis_def.id != 31 {
                render_pass_3d.set_pipeline(&self.clear_black_pipeline);
                render_pass_3d.draw(0..3, 0..1);
            }

            if let Some(vis_pipeline) = self.render_pipelines[mode_idx].as_ref() {
                render_pass_3d.set_pipeline(vis_pipeline);
            }
            render_pass_3d.set_bind_group(0, &self.uniform_bind_group, &[]);

            if let crate::state::PipelineType::Mesh3D {
                geometry,
                instances,
            } = &vis_def.pipeline_type
            {
                if vis_def.id == 23 {
                    render_pass_3d.set_bind_group(1, &self.flip_render_bind_group, &[]);
                    render_pass_3d.set_bind_group(2, &self.camera_bind_group, &[]);
                } else {
                    render_pass_3d.set_bind_group(1, &self.camera_bind_group, &[]);
                    if vis_def.id == 20 {
                        render_pass_3d.set_bind_group(2, &self.biolum_render_bind_group, &[]);
                    } else if vis_def.id == 31 {
                        render_pass_3d.set_bind_group(2, &self.acoustic_render_bind_group, &[]);
                    }
                }
                if let Some(mesh) = self.mesh_registry.get(geometry) {
                    render_pass_3d.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                    render_pass_3d
                        .set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    if vis_def.id == 23 {
                        render_pass_3d.draw_indexed_indirect(&self.lyrics_indirect_args_buffer, 0);
                    } else {
                        render_pass_3d.draw_indexed(0..mesh.index_count, 0, 0..*instances);
                    }
                }

                if vis_def.id == 23 {
                    render_pass_3d.set_pipeline(&self.water_splash_pipeline);
                    render_pass_3d.set_vertex_buffer(0, self.splash_quad_vertex_buffer.slice(..));
                    render_pass_3d.set_index_buffer(
                        self.splash_quad_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    render_pass_3d.draw_indexed_indirect(&self.splash_indirect_args_buffer, 0);
                }

                if vis_def.id == 13 {
                    render_pass_3d.set_pipeline(&self.lamp_pipeline);
                    render_pass_3d.set_vertex_buffer(0, self.lamp_vertex_buffer.slice(..));
                    render_pass_3d.set_index_buffer(
                        self.lamp_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    render_pass_3d.draw_indexed(0..self.lamp_index_count, 0, 0..16);
                }
            }
        }

        {
            let mut render_pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Main Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: if is_mesh_3d {
                                wgpu::LoadOp::Load
                            } else {
                                wgpu::LoadOp::Clear(APP_DEFAULT_BG_CLEAR_COLOR)
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: if is_mesh_3d {
                        None
                    } else {
                        active_query_set.map(|qs| wgpu::RenderPassTimestampWrites {
                            query_set: qs,
                            beginning_of_pass_write_index: Some(4),
                            end_of_pass_write_index: Some(5),
                        })
                    },
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();

            if state.file_loaded {
                if !is_mesh_3d {
                    let mode_idx = state
                        .current_visualizer_idx
                        .min(self.render_pipelines.len() - 1);

                    render_pass.set_viewport(vp_x, vp_y, vp_w, vp_h, 0.0, 1.0);
                    render_pass.set_scissor_rect(
                        vp_x as u32,
                        vp_y as u32,
                        vp_w as u32,
                        vp_h as u32,
                    );

                    if let Some(vis_pipeline) = self.render_pipelines[mode_idx].as_ref() {
                        render_pass.set_pipeline(vis_pipeline);
                    }
                    render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                    if crate::state::VISUALIZERS[mode_idx].id == 32 {
                        render_pass.set_bind_group(1, &self.flip_disc_render_bind_group, &[]);
                    }
                    render_pass.draw(0..3, 0..1);
                }

                if is_video_active {
                    render_pass.set_viewport(v_vp_x, v_vp_y, v_vp_w, v_vp_h, 0.0, 1.0);
                    render_pass.set_scissor_rect(
                        v_vp_x as u32,
                        v_vp_y as u32,
                        v_vp_w as u32,
                        v_vp_h as u32,
                    );

                    if let Some(vs) = &self.video_state {
                        render_pass.set_pipeline(&self.video_pipeline);
                        render_pass.set_bind_group(0, &vs.bind_group, &[]);
                        render_pass.draw(0..3, 0..1);
                    }
                }

                if state.show_hud
                    && state.video_mode != 3
                    && (out_meters_rect.is_some()
                        || out_heatmap_rect.is_some()
                        || out_fire_rect.is_some())
                {
                    render_pass.set_viewport(
                        0.0,
                        0.0,
                        self.config.width as f32,
                        self.config.height as f32,
                        0.0,
                        1.0,
                    );
                    render_pass.set_pipeline(&self.hud_pipeline);
                    render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);

                    let mut draw_rect = |r: Option<egui::Rect>| {
                        if let Some(rect) = r {
                            let x = ((rect.min.x * scale_factor)
                                .clamp(0.0, self.config.width as f32))
                            .round() as u32;
                            let y = ((rect.min.y * scale_factor)
                                .clamp(0.0, self.config.height as f32))
                            .round() as u32;

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
                            }
                        }
                    };

                    draw_rect(out_meters_rect);
                    draw_rect(out_heatmap_rect);
                    draw_rect(out_fire_rect);
                }

                render_pass.set_viewport(
                    0.0,
                    0.0,
                    self.config.width as f32,
                    self.config.height as f32,
                    0.0,
                    1.0,
                );
                render_pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
            }

            if !clipped_primitives.is_empty() {
                self.egui_renderer.render(
                    &mut render_pass,
                    &clipped_primitives,
                    &screen_descriptor,
                );
            }
        }

        // Resolve timestamp queries into the resolve buffer, then copy to the read buffer.
        // ONLY do this when stats are visible and no mapping is pending.
        if should_start_mapping
            && let (Some(qs), Some(res_buf), Some(read_buf)) = (
                &self.query_set,
                &self.query_resolve_buffer,
                &self.query_read_buffer,
            )
        {
            encoder.resolve_query_set(qs, 0..6, res_buf, 0);
            encoder.copy_buffer_to_buffer(res_buf, 0, read_buf, 0, 48);
        }

        static CAPTURE_ENV: std::sync::OnceLock<(Option<u32>, u32, bool)> =
            std::sync::OnceLock::new();
        let &(env_bench_frames, capture_count, env_capture_enabled) =
            CAPTURE_ENV.get_or_init(|| {
                let bf = std::env::var("BENCH_FRAMES")
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok());
                let cc = std::env::var("CAPTURE_FRAMES_COUNT")
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(1);
                let cap = std::env::var("CAPTURE_FRAME").is_ok();
                (bf, cc, cap)
            });

        let bench_override = crate::state::BENCH_FRAMES_OVERRIDE.load(Ordering::Relaxed);
        let capture_frame = if bench_override > 0 {
            bench_override.saturating_sub(1)
        } else if let Some(bf) = env_bench_frames {
            bf.saturating_sub(1)
        } else {
            180
        };
        let capture_start = if capture_count > 1 {
            capture_frame.saturating_sub(capture_count.saturating_sub(1))
        } else {
            capture_frame
        };
        let do_capture = env_capture_enabled
            && self.frame_count >= capture_start
            && self.frame_count <= capture_frame;

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
                wgpu::TexelCopyTextureInfo {
                    texture: &surface_texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &rb,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bpr),
                        rows_per_image: Some(self.config.height),
                    },
                },
                wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
            );
            readback_buffer = Some(rb);
        }

        let command_buffer = encoder.finish();
        let phase_encode_us = render_start.elapsed().as_micros() as f32;
        let submit_start = std::time::Instant::now();

        self.queue.submit(std::iter::once(command_buffer));

        // Start async timestamp mapping AFTER submit (non-blocking)
        if should_start_mapping && let Some(read_buf) = &self.query_read_buffer {
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

        if let Some(rb) = readback_buffer {
            let slice = rb.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |v| {
                let _ = tx.send(v);
            });
            if self
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .is_ok()
                && rx.recv().is_ok_and(|res| res.is_ok())
                && let Ok(data) = slice.get_mapped_range()
            {
                let bpr = (self.config.width * 4 + 255) & !255;
                let is_bgra = matches!(
                    self.config.format,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
                );
                let is_rgb10a2 = matches!(
                    self.config.format,
                    wgpu::TextureFormat::Rgb10a2Unorm | wgpu::TextureFormat::Rgb10a2Uint
                );
                let mut img = image::RgbaImage::new(self.config.width, self.config.height);
                for y in 0..self.config.height {
                    for x in 0..self.config.width {
                        let offset = (y * bpr + x * 4) as usize;
                        let (r, g, b) = if is_rgb10a2 {
                            let packed = u32::from_le_bytes([
                                data[offset],
                                data[offset + 1],
                                data[offset + 2],
                                data[offset + 3],
                            ]);
                            let r10 = packed & 0x3FF;
                            let g10 = (packed >> 10) & 0x3FF;
                            let b10 = (packed >> 20) & 0x3FF;
                            ((r10 >> 2) as u8, (g10 >> 2) as u8, (b10 >> 2) as u8)
                        } else if is_bgra {
                            (data[offset + 2], data[offset + 1], data[offset])
                        } else {
                            (data[offset], data[offset + 1], data[offset + 2])
                        };
                        img.put_pixel(x, y, image::Rgba([r, g, b, 255])); // Ignore A to force fully opaque screenshot
                    }
                }
                let base_capture_path =
                    std::env::var("CAPTURE_PATH").unwrap_or_else(|_| "screenshot.png".to_string());
                let capture_path = if capture_count > 1 {
                    if let Some(pos) = base_capture_path.rfind('.') {
                        format!(
                            "{}_{:03}{}",
                            &base_capture_path[..pos],
                            self.frame_count,
                            &base_capture_path[pos..]
                        )
                    } else {
                        format!("{}_{:03}.png", base_capture_path, self.frame_count)
                    }
                } else {
                    base_capture_path
                };
                if let Err(err) = img.save(&capture_path) {
                    eprintln!("[capture] Failed to save screenshot to {capture_path}: {err}");
                } else {
                    println!("Screenshot saved to {}", capture_path);
                }
            }
            if bench_override == 0
                && env_bench_frames.is_none()
                && self.frame_count == capture_frame
            {
                engine_actions.push(EngineAction::Quit);
            }
        }

        window.pre_present_notify();
        self.queue.present(surface_texture);

        for id in &textures_to_free {
            self.egui_renderer.free_texture(id);
        }

        let phase_submit_us = submit_start.elapsed().as_micros() as f32;
        let submit_elapsed = phase_encode_us + phase_submit_us;

        Ok((
            engine_actions,
            ui_elapsed,
            submit_elapsed,
            fire_shader_time_us,
            fft_shader_time_us,
            vis_shader_time_us,
            phase_surface_us,
            phase_egui_layout_us,
            phase_encode_us,
            phase_submit_us,
        ))
    }
}

#[cfg(test)]
mod tests;
