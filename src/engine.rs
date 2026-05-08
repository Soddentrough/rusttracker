use std::sync::Arc;
use winit::window::Window;
use crate::state::AppState;

#[derive(Clone, Copy, PartialEq)]
pub enum EngineAction {
    None,
    OpenFile,
    Seek(f32),
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AudioUniforms {
    pub spectrum: [f32; 1024],
    pub fire_heat: [f32; 1024],
    pub channels: [f32; 32],
    pub channel_peaks: [f32; 32],
    pub spatial_channels: [f32; 16],
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
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaveformHistoryStorage {
    pub waveforms: [[f32; 1024]; 8],  // only the 8 most recent frames (was 60)
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
    pub _pad: u32,
    pub channels: [[f32; 4]; 8],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FFTParams {
    pub num_channels: u32,
    pub sample_rate: f32,
    pub min_freq: f32,
    pub max_freq: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VideoParams {
    pub color_space: u32,
    pub color_range: u32,
    pub bit_depth: u32,
    pub _pad: u32,
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
}

pub struct VulkanEngine<'a> {
    surface: wgpu::Surface<'a>,
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
    query_in_flight: bool,
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
    pub start_time: std::time::Instant,

    // GPU FFT
    fft_compute_pipeline: wgpu::ComputePipeline,
    fft_bind_group: wgpu::BindGroup,
    raw_audio_buffer: wgpu::Buffer,
    _gpu_spectrum_buffer: wgpu::Buffer,
    fft_params_buffer: wgpu::Buffer,
    
    // Video
    video_bind_group_layout: wgpu::BindGroupLayout,
    video_pipeline: wgpu::RenderPipeline,
    video_state: Option<VideoState>,
}

impl<'a> VulkanEngine<'a> {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        // The instance is a handle to our GPU
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN, // Request Vulkan explicitly!
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

        let mut required_features = wgpu::Features::empty();
        let supports_timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
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

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Let WGPU pick the best non-vsync method to ensure frame pacing doesn't tear/stutter under Wayland
        let present_mode = wgpu::PresentMode::AutoNoVsync;

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
            size: 32 * 1024 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let waveform_storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Waveform History Storage Buffer"),
            size: std::mem::size_of::<WaveformHistoryStorage>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let history_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Heatmap History Texture"),
            size: wgpu::Extent3d { width: 256, height: 120, depth_or_array_layers: 1 },
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

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
                }
            ],
            label: Some("audio_bind_group"),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let shader_sources = vec![
            include_str!("shaders/vis_spectrum.wgsl"),
            include_str!("shaders/vis_flame.wgsl"),
            include_str!("shaders/vis_oscilloscope.wgsl"),
            include_str!("shaders/vis_spatial.wgsl"),
            include_str!("shaders/vis_ferrofluid.wgsl"),
            include_str!("shaders/vis_neon.wgsl"),
            include_str!("shaders/vis_firesim.wgsl"),
            include_str!("shaders/vis_3doscilloscope.wgsl"),
        ];

        let mut render_pipelines = Vec::new();
        
        let scope_fallback = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let fallback_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Fallback Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(shader_sources[0])),
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
            cache: None,
        });
        let _ = scope_fallback.pop().await;

        for (i, source) in shader_sources.iter().enumerate() {
            let scope_main = device.push_error_scope(wgpu::ErrorFilter::Validation);
            
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("Shader {}", i)),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(source)),
            });
            
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("Render Pipeline {}", i)),
                layout: Some(&render_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
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
                cache: None,
            });
            
            let error_future = scope_main.pop();
            let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            
            if let Some(error) = error_future.await {
                eprintln!("WGSL compilation error in visualizer {}: {:?}", i, error);
                render_pipelines.push(fallback_pipeline.clone());
            } else {
                render_pipelines.push(pipeline);
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let hud_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/hud.wgsl"));
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, egui_wgpu::RendererOptions::default());

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
            ],
        });
        let heatmap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("heatmap_bind_group"), layout: &heatmap_compute_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&history_view) },
            ],
        });
        let heatmap_compute_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/heatmap_compute.wgsl"));
        let heatmap_compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("heatmap_compute_layout"), bind_group_layouts: &[Some(&heatmap_compute_layout)], immediate_size: 0,
        });
        let heatmap_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Heatmap Compute Pipeline"), layout: Some(&heatmap_compute_pipeline_layout), module: &heatmap_compute_shader, entry_point: Some("main"), compilation_options: wgpu::PipelineCompilationOptions::default(), cache: None,
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
            cache: None,
        });
        let firesim_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FireSim Compute Pipeline"),
            layout: Some(&fire_compute_pipeline_layout),
            module: &firesim_compute_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let mut query_set = None;
        let mut query_resolve_buffer = None;
        let mut query_read_buffer = None;
        let timestamp_period = queue.get_timestamp_period();

        if supports_timestamps {
            query_set = Some(device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Shader Timestamps"),
                count: 4, // 0-1 for FFT, 2-3 for Fire
                ty: wgpu::QueryType::Timestamp,
            }));

            query_resolve_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Query Resolve Buffer"),
                size: 32, // 4 * 8 bytes
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));

            query_read_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Query Read Buffer"),
                size: 32,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
        }

        // --- GPU FFT INIT ---
        let raw_audio_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU FFT Raw Audio Buffer"),
            size: 32 * 8192 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });


        let fft_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU FFT Params Buffer"),
            size: std::mem::size_of::<FFTParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let fft_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("fft_bind_group_layout"),
        });

        let fft_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fft_bind_group"),
            layout: &fft_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: raw_audio_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: gpu_spectrum_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: fft_params_buffer.as_entire_binding() },
            ],
        });

        let fft_compute_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/gpu_fft.wgsl"));
        let fft_compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fft_compute_layout"),
            bind_group_layouts: &[Some(&fft_bind_group_layout)],
            immediate_size: 0,
        });

        let fft_compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FFT Compute Pipeline"),
            layout: Some(&fft_compute_pipeline_layout),
            module: &fft_compute_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        // --- END GPU FFT INIT ---

        Self {
            surface,
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
            query_in_flight: false,
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
            start_time: std::time::Instant::now(),
            fft_compute_pipeline,
            fft_bind_group,
            raw_audio_buffer,
            _gpu_spectrum_buffer: gpu_spectrum_buffer,
            fft_params_buffer,
            video_bind_group_layout,
            video_pipeline,
            video_state: None,
        }
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn clear_video_state(&mut self) {
        self.video_state = None;
    }

    pub fn update(&mut self, state: &AppState) {
        self.heatmap_row = (self.heatmap_row + 1) % 120;
        let mut uniforms = AudioUniforms {
            spectrum: [0.0; 1024],
            fire_heat: [0.0; 1024],
            channels: [0.0; 32],
            channel_peaks: [0.0; 32],
            spatial_channels: [0.0; 16],
            num_channels: state.channel_vus.len().min(32) as u32,
            mode: state.visualizer_mode,
            time: state.current_seconds as f32,
            duration: state.duration_seconds as f32,
            smooth_time: self.start_time.elapsed().as_secs_f32(),
            heatmap_row: self.heatmap_row,
            fft_channels: state.raw_audio_channels.len() as u32,
            num_spatial_channels: state.channel_vus.len().saturating_sub(state.tracker_channels.unwrap_or(0) as usize) as u32,
            ui_meters_rect: self.meters_uv_rect,
            ui_heatmap_rect: self.heatmap_uv_rect,
            ui_fire_rect: self.fire_uv_rect,
        };

        uniforms.spectrum.copy_from_slice(&state.spectrum_data);
        uniforms.fire_heat.copy_from_slice(&state.fire_heat);
        
        let ch_len = state.channel_vus.len().min(32);
        
        // 1. Populate UI Display Channels (may be visually remapped)
        let mut display_order: Vec<usize> = (0..ch_len).collect();
        if state.tracker_channels.is_none() {
            if ch_len == 6 {
                display_order = vec![4, 0, 2, 3, 1, 5]; // Ls, L, C, LFE, R, Rs
            } else if ch_len == 8 {
                display_order = vec![4, 6, 0, 2, 3, 1, 7, 5]; // SBL, Ls, L, C, LFE, R, Rs, SBR
            }
        }

        for (disp_idx, &src_idx) in display_order.iter().enumerate() {
            if src_idx < state.channel_vus.len() {
                uniforms.channels[disp_idx] = state.channel_vus[src_idx];
                uniforms.channel_peaks[disp_idx] = state.peak_vus[src_idx];
            }
        }
        
        // 2. Populate Raw Spatial Channels (strict spatial mapping without UI reordering)
        let spatial_offset = state.tracker_channels.unwrap_or(0) as usize;
        let spatial_count = state.channel_vus.len().saturating_sub(spatial_offset);
        for i in 0..spatial_count {
            let src_idx = spatial_offset + i;
            if src_idx < state.channel_vus.len() && i < 16 {
                uniforms.spatial_channels[i] = state.channel_vus[src_idx];
            }
        }

        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        // Only upload waveform history when the oscilloscope visualizer is active
        if state.visualizer_mode == 2 {
            let mut history_storage = WaveformHistoryStorage {
                waveforms: [[0.0; 1024]; 8],
            };
            // Upload only the 8 most recent frames (32KB vs 240KB)
            let hist_len = state.waveform_history.len();
            let start = hist_len.saturating_sub(8);
            for (slot, wave) in state.waveform_history.iter().skip(start).enumerate().take(8) {
                // Pre-smooth with [1,2,1] triangle kernel
                history_storage.waveforms[slot][0] = (wave[0] * 3.0 + wave[1]) / 4.0;
                for j in 1..1023 {
                    history_storage.waveforms[slot][j] = (wave[j - 1] + wave[j] * 2.0 + wave[j + 1]) / 4.0;
                }
                history_storage.waveforms[slot][1023] = (wave[1022] + wave[1023] * 3.0) / 4.0;
            }
            self.queue.write_buffer(&self.waveform_storage_buffer, 0, bytemuck::cast_slice(&[history_storage]));
        }
        
        if state.visualizer_mode == 1 || state.visualizer_mode == 6 {
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
            let lfe_idx = if n_ch == 6 { 3 } else if n_ch == 8 { 4 } else { 999 };
            
            let mut fire_params = FireParams {
                bass,
                mids,
                highs,
                time: self.start_time.elapsed().as_secs_f32(),
                cooling_factor: 1.3 - mids * 0.6,
                turb_spread_f: 1.0 + highs * 3.0,
                width: 1024,
                height: 576,
                num_channels: ch_len as u32,
                lfe_idx: lfe_idx as u32,
                fft_channels: state.raw_audio_channels.len() as u32,
                _pad: 0,
                channels: [[0.0; 4]; 8],
            };
            
            for i in 0..n_ch {
                fire_params.channels[i / 4][i % 4] = uniforms.channels[i];
            }
            
            self.queue.write_buffer(&self.fire_params_buffer, 0, bytemuck::cast_slice(&[fire_params]));
        }

        if state.gpu_fft {
            let num_channels = state.raw_audio_channels.len().min(32);
            let mut flat_raw_audio = vec![0.0f32; 32 * 8192];
            for (c, channel_data) in state.raw_audio_channels.iter().take(32).enumerate() {
                let len = channel_data.len().min(8192);
                flat_raw_audio[(c * 8192)..(c * 8192 + len)].copy_from_slice(&channel_data[..len]);
            }
            self.queue.write_buffer(&self.raw_audio_buffer, 0, bytemuck::cast_slice(&flat_raw_audio));

            let fft_params = FFTParams {
                num_channels: num_channels as u32,
                sample_rate: 44100.0, // TODO: Use actual sample rate if available
                min_freq: 20.0,
                max_freq: state.max_frequency,
            };
            self.queue.write_buffer(&self.fft_params_buffer, 0, bytemuck::cast_slice(&[fft_params]));
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
                    });
                } else if let Some(vs) = &mut self.video_state {
                    vs.color_space = frame.color_space;
                    vs.color_range = frame.color_range;
                    vs.bit_depth = frame.bit_depth as u32;
                }
                
                if let Some(vs) = &self.video_state {
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &vs.y_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        &frame.y_plane,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(frame.y_stride as u32), rows_per_image: Some(frame.height) },
                        wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 }
                    );
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &vs.u_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        &frame.u_plane,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(frame.u_stride as u32), rows_per_image: Some(frame.height / 2) },
                        wgpu::Extent3d { width: frame.width / 2, height: frame.height / 2, depth_or_array_layers: 1 }
                    );
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &vs.v_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        &frame.v_plane,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(frame.v_stride as u32), rows_per_image: Some(frame.height / 2) },
                        wgpu::Extent3d { width: frame.width / 2, height: frame.height / 2, depth_or_array_layers: 1 }
                    );
                    
                    let params = VideoParams {
                        color_space: frame.color_space,
                        color_range: frame.color_range,
                        bit_depth: frame.bit_depth as u32,
                        _pad: 0,
                        viewport_width: 1920.0,
                        viewport_height: 1080.0,
                        video_width: frame.width as f32,
                        video_height: frame.height as f32,
                    };
                    self.queue.write_buffer(&vs.params_buffer, 0, bytemuck::cast_slice(&[params]));
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
    ) -> Result<(EngineAction, f32, f32, Option<f32>, Option<f32>), wgpu::SurfaceStatus> {
        let ui_start = std::time::Instant::now();
        let output = self.surface.get_current_texture();
        let surface_texture = match output {
            wgpu::CurrentSurfaceTexture::Success(tex) | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            wgpu::CurrentSurfaceTexture::Lost => return Err(wgpu::SurfaceStatus::Lost),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(wgpu::SurfaceStatus::Outdated),
            wgpu::CurrentSurfaceTexture::Timeout => return Err(wgpu::SurfaceStatus::Timeout),
            _ => return Err(wgpu::SurfaceStatus::Lost),
        };
        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut fft_shader_time_us = None;
        let mut fire_shader_time_us = None;
        if self.query_in_flight {
            if let Some(read_buffer) = &self.query_read_buffer {
                let slice = read_buffer.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
                self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                if rx.recv().unwrap().is_ok() {
                    let data = slice.get_mapped_range();
                    
                    let fft_start: u64 = u64::from_le_bytes(data[0..8].try_into().unwrap());
                    let fft_end: u64 = u64::from_le_bytes(data[8..16].try_into().unwrap());
                    if fft_end > fft_start {
                        let elapsed_ns = (fft_end - fft_start) as f32 * self.timestamp_period;
                        fft_shader_time_us = Some(elapsed_ns / 1_000.0);
                    }

                    let fire_start: u64 = u64::from_le_bytes(data[16..24].try_into().unwrap());
                    let fire_end: u64 = u64::from_le_bytes(data[24..32].try_into().unwrap());
                    if fire_end > fire_start {
                        let elapsed_ns = (fire_end - fire_start) as f32 * self.timestamp_period;
                        fire_shader_time_us = Some(elapsed_ns / 1_000.0);
                    }

                    drop(data);
                    read_buffer.unmap();
                    self.query_in_flight = false;
                }
            }
        }

        // Process egui UI
        let raw_input = egui_state.take_egui_input(window);
        let mut central_rect = egui::Rect::from_min_max(Default::default(), egui::pos2(self.config.width as f32, self.config.height as f32));
        let mut engine_action = EngineAction::None;
        
        let mut out_meters_rect = None;
        let mut out_fire_rect = None;
        let mut out_heatmap_rect = None;
        let mut out_track_info_rect = None;
        let mut out_top_panel_rect = None;
        
        let vis_name = match state.visualizer_mode {
            0 => "Frequency Spectrum",
            1 => "Classic Flame",
            2 => "CRT Oscilloscope",
            3 => "Spatial Vectors",
            4 => "Chrome Ferrofluid",
            5 => "Neon Corridor",
            6 => "Fire Simulation",
            7 => "3D CRT Oscilloscope",
            _ => "Unknown",
        };
        
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
                            egui::RichText::new(format!("UI: {:.2} ms | Render: {:.2} ms", state.stats.ui_us / 1000.0, state.stats.render_us / 1000.0))
                                .color(egui::Color32::GRAY)
                        );
                        if state.gpu_fft {
                            ui.label(
                                egui::RichText::new(format!("GPU FFT: {:.2} ms | Decode: {:.2} ms", state.stats.gpu_fft_us / 1000.0, state.stats.decode_us / 1000.0))
                                    .color(egui::Color32::LIGHT_BLUE)
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(format!("CPU FFT: {:.2} ms | Decode: {:.2} ms", state.stats.fft_us / 1000.0, state.stats.decode_us / 1000.0))
                                    .color(egui::Color32::GRAY)
                            );
                        }
                        ui.label(
                            egui::RichText::new(format!("Shader: {:.2} ms", state.stats.shader_us / 1000.0))
                                .color(egui::Color32::GRAY)
                        );
                        ui.label(
                            egui::RichText::new("Fire FX: 0.00 ms (GPU Offloaded)")
                                .color(egui::Color32::GRAY)
                        );
                        ui.label(
                            egui::RichText::new(format!("Audio Buffer: {:.1}%", state.stats.audio_buffer_fill_pct))
                                .color(if state.stats.audio_buffer_fill_pct < 5.0 { egui::Color32::RED } else if state.stats.audio_buffer_fill_pct > 95.0 { egui::Color32::YELLOW } else { egui::Color32::GREEN })
                        );
                        ui.label(
                            egui::RichText::new(format!("Video Buffer: {:.1}%", state.stats.video_buffer_fill_pct))
                                .color(if state.stats.video_buffer_fill_pct < 1.0 { egui::Color32::RED } else if state.stats.video_buffer_fill_pct > 95.0 { egui::Color32::YELLOW } else { egui::Color32::GREEN })
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

            if !state.file_loaded {
                central_rect = ctx.content_rect();
                
                let frame = egui::Frame::NONE
                    .fill(egui::Color32::from_rgba_unmultiplied(10, 10, 15, 200)) // dark tint to pop text over the visualizer
                    .inner_margin(40.0);
                    
                egui::CentralPanel::default().frame(frame).show_inside(ctx, |ui| {
                    let avail_height = ui.available_height();
                    let content_height = 380.0;
                    let space = (avail_height - content_height) / 2.0;
                    if space > 0.0 {
                        ui.add_space(space);
                    }
                    
                    ui.allocate_ui_with_layout(
                        ui.available_size(),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("RustTracker")
                                    .size(72.0)
                                    .color(egui::Color32::from_rgb(100, 200, 255))
                                    .strong()
                            );
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("A High-Performance Vulkan Module Visualizer").size(18.0).color(egui::Color32::GRAY));
                            
                            ui.add_space(40.0);
                            
                            let btn = egui::Button::new(
                                egui::RichText::new("  OPEN AUDIO FILE  ")
                                    .size(24.0)
                                    .color(egui::Color32::WHITE)
                                    .strong()
                            )
                            .fill(egui::Color32::from_rgb(0, 120, 215));
                            
                            if ui.add_sized([350.0, 60.0], btn).clicked() {
                                engine_action = EngineAction::OpenFile;
                            }
                            
                            ui.add_space(60.0);
                            ui.label(egui::RichText::new("Keyboard Shortcuts").color(egui::Color32::LIGHT_GRAY).strong().size(18.0));
                            ui.add_space(15.0);
                            
                            egui::Grid::new("shortcuts_grid")
                                .num_columns(4)
                                .spacing([30.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("O").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Open File").color(egui::Color32::GRAY));
                                    ui.label(egui::RichText::new("Space").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Play / Pause").color(egui::Color32::GRAY));
                                    ui.end_row();
                                    
                                    ui.label(egui::RichText::new("Tab").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Toggle HUD").color(egui::Color32::GRAY));
                                    ui.label(egui::RichText::new("Left / Right").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Seek Timeline").color(egui::Color32::GRAY));
                                    ui.end_row();
                                    
                                    ui.label(egui::RichText::new("F").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Toggle Fullscreen").color(egui::Color32::GRAY));
                                    ui.label(egui::RichText::new("Up / Down").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Change Visualizer").color(egui::Color32::GRAY));
                                    ui.end_row();
                                    
                                    ui.label(egui::RichText::new("S").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Toggle Stats").color(egui::Color32::GRAY));
                                    ui.label(egui::RichText::new("Q / Esc").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Quit").color(egui::Color32::GRAY));
                                    ui.end_row();
                                    
                                    ui.label(egui::RichText::new("G").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Toggle GPU FFT").color(egui::Color32::GRAY));
                                    ui.end_row();
                                });
                        }
                    );
                });
                return;
            }

            if state.show_hud {
                egui::Panel::top("top_panel")
                    .resizable(false)
                    .frame(egui::Frame::NONE.fill(egui::Color32::TRANSPARENT))
                    .exact_size(ctx.content_rect().height() / 2.0)
                    .show_inside(ctx, |ui| {
                        out_top_panel_rect = Some(ui.max_rect());
                        if state.video_mode == 2 {
                            out_top_panel_rect = Some(ui.max_rect());
                        } else {
                            ui.columns(3, |columns| {
                                // Column 0: Channels
                                columns[0].heading("Channels");
                            columns[0].separator();
                            let (channel_rect, _) = columns[0].allocate_exact_size(
                                egui::vec2(columns[0].available_width(), columns[0].available_height() - 25.0), 
                                egui::Sense::hover()
                            );
                            out_meters_rect = Some(channel_rect);
                            
                            let painter = columns[0].painter();
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
                                                8 => ["SBL", "Ls", "L", "C", "LFE", "R", "Rs", "SBR"].get(i).unwrap_or(&"?").to_string(),
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
                            
                            columns[0].add_space(5.0);
                            
                            // Custom Fire/Charred Progress Bar
                            let (rect, response) = columns[0].allocate_exact_size(egui::vec2(columns[0].available_width(), 16.0), egui::Sense::click_and_drag());
                            out_fire_rect = Some(rect);
                            
                            if response.dragged() || response.clicked() {
                                if let Some(mouse_pos) = response.interact_pointer_pos() {
                                    let rel_x = (mouse_pos.x - rect.left()).clamp(0.0, rect.width());
                                    let pct = rel_x / rect.width();
                                    engine_action = EngineAction::Seek(pct);
                                }
                            }
                            
                            let painter = columns[0].painter();
                            let format_time = |secs: f64| -> String {
                                let m = (secs / 60.0).floor() as u32;
                                let s = (secs % 60.0).floor() as u32;
                                let f = (secs.fract() * 10.0).floor() as u32;
                                format!("{:02}:{:02}.{}", m, s, f)
                            };
                            
                            painter.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("{} / {}", format_time(state.current_seconds), format_time(state.duration_seconds)),
                                egui::FontId::proportional(11.0),
                                egui::Color32::WHITE,
                            );
                            
                            // Column 1: Heatmap History & Tracker Pattern
                            columns[1].heading("Pattern Heatmap");
                            columns[1].separator();
                            let hm_rect = columns[1].available_rect_before_wrap();
                            out_heatmap_rect = Some(hm_rect);
                            
                            columns[1].painter().rect_filled(hm_rect, 0.0, egui::Color32::TRANSPARENT);
                            
                            let painter = columns[1].painter().with_clip_rect(hm_rect);
                            let history_len = state.spectrum_history.len();
                            if history_len > 0 && state.spectrum_history[0].len() > 0 {
                                let chunks = 64;
                                let cell_w = hm_rect.width() / chunks as f32;
                                
                                for c in 0..=chunks {
                                    let x = hm_rect.left() + c as f32 * cell_w;
                                    painter.line_segment([egui::pos2(x, hm_rect.top()), egui::pos2(x, hm_rect.bottom())], (1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 5)));
                                }
                                
                                if !state.tracker_patterns_by_order.is_empty() {
                                    let current_order = state.current_tracker_order as i32;
                                    let current_row = state.current_tracker_row as i32;
                                    let center_y = hm_rect.top() + hm_rect.height() / 2.0;
                                    let row_height = 16.0;
                                    let num_rows_to_draw = (hm_rect.height() / row_height) as i32;
                                    
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
                                        } else {
                                            // Handle overflow (next predicted patterns)
                                            while resolved_order >= 0 
                                                && (resolved_order as usize) < state.tracker_patterns_by_order.len() 
                                                && resolved_row >= state.tracker_patterns_by_order[resolved_order as usize].len() as i32 
                                            {
                                                resolved_row -= state.tracker_patterns_by_order[resolved_order as usize].len() as i32;
                                                resolved_order += 1;
                                            }
                                        }
                                        
                                        if resolved_order >= 0 && (resolved_order as usize) < state.tracker_patterns_by_order.len() && resolved_row >= 0 {
                                            if (resolved_row as usize) < state.tracker_patterns_by_order[resolved_order as usize].len() {
                                                let text = &state.tracker_patterns_by_order[resolved_order as usize][resolved_row as usize];
                                                let y = center_y + offset as f32 * row_height;
                                                
                                                // Fade out based on distance
                                                let distance = offset.abs() as f32 / (num_rows_to_draw as f32 / 2.0);
                                                let alpha = (1.0 - distance).max(0.0);
                                                
                                                let color = if offset == 0 {
                                                    egui::Color32::from_rgba_premultiplied(255, 255, 255, 255)
                                                } else {
                                                    egui::Color32::from_rgba_premultiplied(150, 150, 150, (alpha * 100.0) as u8)
                                                };
                                                let font_id = egui::FontId::monospace(12.0);
                                                
                                                // Draw text centered horizontally
                                                let rect = painter.text(
                                                    egui::pos2(hm_rect.center().x, y),
                                                    egui::Align2::CENTER_CENTER,
                                                    format!("{:02X}  {}", resolved_row, text),
                                                    font_id,
                                                    color
                                                );
                                                
                                                // Optional: highlight background of active row
                                                if offset == 0 {
                                                    painter.rect_filled(
                                                        rect.expand2(egui::vec2(10.0, 2.0)),
                                                        2.0,
                                                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
                                                    );
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
                            }
                            
                            // Column 2: Track Info
                            if state.video_mode == 1 {
                                let available = columns[2].available_size();
                                let (rect, _) = columns[2].allocate_exact_size(available, egui::Sense::hover());
                                out_track_info_rect = Some(rect);
                            } else {
                                columns[2].heading("Track Info");
                                columns[2].separator();
                            if state.playlist.len() > 1 {
                                columns[2].horizontal(|ui| { ui.label("Playlist"); ui.label(format!("{} / {}", state.playlist_index + 1, state.playlist.len())); });
                            }
                            let title_path = std::path::Path::new(&state.song_title);
                            let file_name = title_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                            let file_dir = title_path.parent().unwrap_or(std::path::Path::new("")).to_string_lossy().to_string();
                            columns[2].horizontal(|ui| { ui.label("File"); ui.label(&file_name); });
                            columns[2].horizontal(|ui| { ui.label("Path"); ui.label(&file_dir); });
                            columns[2].horizontal(|ui| { ui.label("Artist"); ui.label(&state.artist); });
                            columns[2].horizontal(|ui| { ui.label("Type"); ui.label(&state.module_type); });
                            if let Some(video) = &state.video_info {
                                if video == "Unsupported Codec" {
                                    columns[2].horizontal(|ui| { ui.label("Video"); ui.label(video); });
                                } else {
                                    columns[2].horizontal(|ui| { ui.label("Video"); ui.label(format!("{} (Video stream available)", video)); });
                                }
                            }
                            if state.bpm > 0 { columns[2].horizontal(|ui| { ui.label("BPM"); ui.label(format!("{}", state.bpm)); }); }
                            if state.speed > 0 { columns[2].horizontal(|ui| { ui.label("Speed"); ui.label(format!("{}", state.speed)); }); }
                            if state.num_patterns > 0 { columns[2].horizontal(|ui| { ui.label("Patterns"); ui.label(format!("{}", state.num_patterns)); }); }
                            if state.num_instruments > 0 { columns[2].horizontal(|ui| { ui.label("Instruments"); ui.label(format!("{}", state.num_instruments)); }); }
                            if state.num_samples > 0 { columns[2].horizontal(|ui| { ui.label("Samples"); ui.label(format!("{}", state.num_samples)); }); }
                            columns[2].horizontal(|ui| { 
                                if let Some(tc) = state.tracker_channels {
                                    ui.label("Tracker Channels");
                                    ui.label(format!("{}", tc));
                                } else {
                                    ui.label("Channels"); 
                                    if state.num_channels > state.hardware_channels && state.hardware_channels > 0 {
                                        ui.label(format!("{} (Downmixed to {})", state.num_channels, state.hardware_channels));
                                    } else {
                                        ui.label(format!("{}", state.num_channels));
                                    }
                                }
                            });
                            
                            columns[2].horizontal(|ui| { ui.label("Length"); ui.label(format!("{:.1}s", state.duration_seconds)); });
                            out_track_info_rect = Some(columns[2].min_rect());
                        }
                    });
                }
            });
        }

            let frame = egui::Frame::NONE.fill(egui::Color32::TRANSPARENT);
            egui::CentralPanel::default().frame(frame).show_inside(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                central_rect = rect;
                
                if state.visualizer_mode == 0 {
                    let painter = ui.painter();
                    let y = rect.bottom() - 20.0;
                    
                    let max_freq = state.max_frequency;
                    let min_freq = 20.0_f32;
                    let x_at = |f: f32| -> f32 { (f / min_freq).ln() / (max_freq / min_freq).ln() };
                    let labels = [
                        (0.0, format!("{}Hz", min_freq as u32)),
                        (x_at(100.0), "100Hz".to_string()),
                        (x_at(1000.0), "1kHz".to_string()),
                        (x_at(5000.0), "5kHz".to_string()),
                        (0.97, format!("{:.0}kHz", max_freq / 1000.0)),
                    ];
                    
                    let width = rect.width();
                    for (x_pct, text) in labels.iter() {
                        let x = rect.left() + width * x_pct;
                        painter.text(
                            egui::pos2(x, y),
                            egui::Align2::LEFT_BOTTOM,
                            text,
                            egui::FontId::proportional(16.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
            });
        });

        let scale = window.scale_factor() as f32;
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

        for (id, image_delta) in &full_output.textures_delta.set {
            self.egui_renderer.update_texture(&self.device, &self.queue, *id, image_delta);
        }

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: window.scale_factor() as f32,
        };
        
        let ui_elapsed = ui_start.elapsed().as_micros() as f32;
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
        
        // GPU FFT compute
        if state.gpu_fft {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("FFT Compute Pass"),
                timestamp_writes: self.query_set.as_ref().map(|qs| wgpu::ComputePassTimestampWrites {
                    query_set: qs,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
            });
            compute_pass.set_pipeline(&self.fft_compute_pipeline);
            compute_pass.set_bind_group(0, Some(&self.fft_bind_group), &[]);
            compute_pass.dispatch_workgroups(64, 2, 1); // 1024/16=64, 32/16=2
        }

        // GPU fire compute: dispatch simulation + copy result to texture
        if state.visualizer_mode == 1 || state.visualizer_mode == 6 {
            {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fire Compute"),
                    timestamp_writes: self.query_set.as_ref().map(|qs| wgpu::ComputePassTimestampWrites {
                        query_set: qs,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    }),
                });
                if state.visualizer_mode == 1 {
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
        }

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.1,
                            b: 0.1,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None, // Moved render timing to fire compute (2,3) to avoid overlapping/complex query sets. We don't trace render pass anymore.
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let scale_factor = window.scale_factor() as f32;
            let vp_x = (central_rect.min.x * scale_factor).clamp(0.0, self.config.width as f32);
            let vp_y = (central_rect.min.y * scale_factor).clamp(0.0, self.config.height as f32);
            let max_w = (self.config.width as f32 - vp_x).max(1.0);
            let vp_w = (central_rect.width() * scale_factor).clamp(1.0, max_w);
            let max_h = (self.config.height as f32 - vp_y).max(1.0);
            let vp_h = (central_rect.height() * scale_factor).clamp(1.0, max_h);
            
            render_pass.set_viewport(vp_x, vp_y, vp_w, vp_h, 0.0, 1.0);

            let mode_idx = (state.visualizer_mode as usize).min(self.render_pipelines.len() - 1);
            render_pass.set_pipeline(&self.render_pipelines[mode_idx]);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.draw(0..3, 0..1);
            
            if state.video_mode > 0 && self.video_state.is_some() {
                let mut v_vp_x = 0.0;
                let mut v_vp_y = 0.0;
                let mut v_vp_w = self.config.width as f32;
                let mut v_vp_h = self.config.height as f32;
                
                let target_rect = if state.video_mode == 1 {
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
                
                if let Some(vs) = &self.video_state {
                    let params = VideoParams {
                        color_space: vs.color_space,
                        color_range: vs.color_range,
                        bit_depth: vs.bit_depth,
                        _pad: 0,
                        viewport_width: v_vp_w,
                        viewport_height: v_vp_h,
                        video_width: vs.width as f32,
                        video_height: vs.height as f32,
                    };
                    self.queue.write_buffer(&vs.params_buffer, 0, bytemuck::cast_slice(&[params]));
                    render_pass.set_pipeline(&self.video_pipeline);
                    render_pass.set_bind_group(0, &vs.bind_group, &[]);
                }
                render_pass.draw(0..3, 0..1);
            }
            
            if state.show_hud {
                render_pass.set_viewport(0.0, 0.0, self.config.width as f32, self.config.height as f32, 0.0, 1.0);
                render_pass.set_pipeline(&self.hud_pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
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

        if let (Some(qs), Some(res_buf), Some(read_buf)) = (&self.query_set, &self.query_resolve_buffer, &self.query_read_buffer) {
            encoder.resolve_query_set(qs, 0..4, res_buf, 0);
            encoder.copy_buffer_to_buffer(res_buf, 0, read_buf, 0, 32);
            self.query_in_flight = true;
        }

        let do_capture = std::env::var("CAPTURE_FRAME").is_ok() && state.current_seconds >= 20.0;
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
                img.save("screenshot.png").unwrap();
            }
            std::process::exit(0);
        }
        
        surface_texture.present();

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        
        let render_elapsed = render_start.elapsed().as_micros() as f32;

        Ok((engine_action, ui_elapsed, render_elapsed, fire_shader_time_us, fft_shader_time_us))
    }
}
