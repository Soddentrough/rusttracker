#[path = "../src/lyrics.rs"]
pub mod lyrics;
#[path = "../src/state.rs"]
pub mod state;
#[path = "../src/audio.rs"]
pub mod audio;
#[path = "../src/bitstream.rs"]
pub mod bitstream;
#[path = "../src/engine.rs"]
pub mod engine;

use wgpu::*;
use std::time::Instant;

fn resolve_shader_includes(source: &str) -> String {
    const SHADER_COMMON: &str = include_str!("../src/shaders/_common.wgsl");
    const SHADER_GLYPH_FONT: &str = include_str!("../src/shaders/_glyph_font.wgsl");
    source
        .replace("// INCLUDE: common", SHADER_COMMON)
        .replace("// INCLUDE: glyph_font", SHADER_GLYPH_FONT)
}

#[test]
fn test_performance() {
    pollster::block_on(run_perf_test());
}

async fn run_perf_test() {
    let instance = Instance::new(InstanceDescriptor {
        backends: Backends::PRIMARY,
        flags: InstanceFlags::default(),
        backend_options: BackendOptions::default(),
        display: None,
        memory_budget_thresholds: MemoryBudgetThresholds::default(),
    });
    
    let adapter = instance.request_adapter(&RequestAdapterOptions {
        power_preference: PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }).await.unwrap();

    println!("Selected Adapter: {:?}", adapter.get_info());

    let mut required_features = Features::empty();
    let supports_timestamps = adapter.features().contains(Features::TIMESTAMP_QUERY);
    if supports_timestamps {
        required_features |= Features::TIMESTAMP_QUERY;
        println!("Timestamp queries are supported!");
    } else {
        println!("WARNING: Timestamp queries NOT supported on this GPU/driver. Falling back to CPU-side measurements.");
    }

    let (device, queue) = adapter.request_device(
        &DeviceDescriptor {
            label: None,
            required_features,
            required_limits: Limits::default(),
            memory_hints: MemoryHints::default(),
            ..Default::default()
        },
    ).await.unwrap();

    // Create 1920x1080 textures
    let width = 1920;
    let height = 1080;
    let color_format = TextureFormat::Rgba8UnormSrgb;

    let render_target = device.create_texture(&TextureDescriptor {
        label: Some("RenderTarget"),
        size: Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: color_format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = render_target.create_view(&TextureViewDescriptor::default());

    let depth_texture = device.create_texture(&TextureDescriptor {
        label: Some("DepthTexture"),
        size: Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_texture.create_view(&TextureViewDescriptor::default());

    // Create layouts & uniform buffers
    let uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Uniforms"),
        size: std::mem::size_of::<crate::engine::AudioUniforms>() as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let waveform_storage_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Waveform History Storage"),
        size: (2048 * 144 * 4) as u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let history_texture = device.create_texture(&TextureDescriptor {
        label: Some("Heatmap History Texture"),
        size: Extent3d { width: 256, height: 1024, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::R32Float,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let history_view = history_texture.create_view(&TextureViewDescriptor::default());

    let fire_grid_texture = device.create_texture(&TextureDescriptor {
        label: Some("Fire Grid Texture"),
        size: Extent3d { width: 1024, height: 576, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::R32Float,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let fire_grid_view = fire_grid_texture.create_view(&TextureViewDescriptor::default());

    let gpu_spectrum_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("GPU FFT Spectrum Buffer"),
        size: 32 * 1024 * 8,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let ferrofluidsim_grid = device.create_buffer(&BufferDescriptor {
        label: Some("Ferrofluid Grid"),
        size: (512 * 512 * 4) as u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: false },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: false },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 5,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }
        ],
        label: Some("audio_bind_group_layout"),
    });

    let uniform_bind_group = device.create_bind_group(&BindGroupDescriptor {
        layout: &bind_group_layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
            BindGroupEntry { binding: 1, resource: waveform_storage_buffer.as_entire_binding() },
            BindGroupEntry { binding: 2, resource: BindingResource::TextureView(&history_view) },
            BindGroupEntry { binding: 3, resource: BindingResource::TextureView(&fire_grid_view) },
            BindGroupEntry { binding: 4, resource: gpu_spectrum_buffer.as_entire_binding() },
            BindGroupEntry { binding: 5, resource: ferrofluidsim_grid.as_entire_binding() }
        ],
        label: Some("audio_bind_group"),
    });

    // Create smoke dummy bindings for the rendering layout
    let smoke_texture = device.create_texture(&TextureDescriptor {
        label: Some("Neon Smoke Texture"),
        size: Extent3d { width: 64, height: 64, depth_or_array_layers: 64 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D3,
        format: TextureFormat::Rgba16Float,
        usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let smoke_texture_view = smoke_texture.create_view(&TextureViewDescriptor::default());

    let smoke_sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("Neon Smoke Sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..Default::default()
    });

    let smoke_render_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("Smoke Render Layout"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let smoke_render_bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: Some("Smoke Render Bind Group"),
        layout: &smoke_render_layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&smoke_texture_view) },
            BindGroupEntry { binding: 1, resource: BindingResource::Sampler(&smoke_sampler) },
        ],
    });

    let render_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[Some(&bind_group_layout), Some(&smoke_render_layout)],
        immediate_size: 0,
    });

    // 3D camera / grid setup
    let camera_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("Camera Bind Group Layout"),
        entries: &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let camera_uniform_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Camera Uniforms"),
        size: std::mem::size_of::<crate::engine::CameraUniforms>() as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let camera_bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: Some("Camera Bind Group"),
        layout: &camera_bind_group_layout,
        entries: &[BindGroupEntry {
            binding: 0,
            resource: camera_uniform_buffer.as_entire_binding(),
        }],
    });

    let biolum_render_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("Biolum Render Layout"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let biolum_particles_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Bioluminescent Particles"),
        size: 65536 * 32,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&biolum_particles_buffer, 0, &vec![0u8; 65536 * 32]);

    let biolum_render_bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: Some("Biolum Render Bind Group"),
        layout: &biolum_render_bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: biolum_particles_buffer.as_entire_binding(),
            },
        ],
    });

    let render_pipeline_layout_3d = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("3D Render Pipeline Layout"),
        bind_group_layouts: &[Some(&bind_group_layout), Some(&smoke_render_layout), Some(&camera_bind_group_layout)],
        immediate_size: 0,
    });

    let biolum_render_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Biolum Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(&bind_group_layout),
            Some(&smoke_render_layout),
            Some(&camera_bind_group_layout),
            Some(&biolum_render_bind_group_layout),
        ],
        immediate_size: 0,
    });

    // Grid buffers
    let mut vertices = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let grid_width = 200;
    let grid_depth = 200;
    for z in 0..=grid_depth {
        for x in 0..=grid_width {
            let px = x as f32 - (grid_width as f32) / 2.0;
            let pz = z as f32 - (grid_depth as f32) / 2.0;
            vertices.push(crate::engine::Vertex {
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

    let grid_vertex_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Grid Vertex Buffer"),
        size: (vertices.len() * std::mem::size_of::<crate::engine::Vertex>()) as u64,
        usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&grid_vertex_buffer, 0, bytemuck::cast_slice(&vertices));

    let grid_index_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Grid Index Buffer"),
        size: (indices.len() * std::mem::size_of::<u32>()) as u64,
        usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&grid_index_buffer, 0, bytemuck::cast_slice(&indices));
    let grid_index_count = indices.len() as u32;

    // Lamp buffers
    let (lamp_verts, lamp_inds) = crate::engine::generate_lamp_mesh();
    let lamp_vertex_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Lamp Vertex Buffer"),
        size: (lamp_verts.len() * std::mem::size_of::<crate::engine::Vertex>()) as u64,
        usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&lamp_vertex_buffer, 0, bytemuck::cast_slice(&lamp_verts));

    let lamp_index_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Lamp Index Buffer"),
        size: (lamp_inds.len() * std::mem::size_of::<u32>()) as u64,
        usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&lamp_index_buffer, 0, bytemuck::cast_slice(&lamp_inds));
    let lamp_index_count = lamp_inds.len() as u32;

    // UnitQuad buffers for instanced particle visualizer (ID 20)
    let quad_verts = vec![
        crate::engine::Vertex { position: [-0.5, -0.5, 0.0], normal: [0.0, 0.0, 1.0], tex_coords: [0.0, 0.0] },
        crate::engine::Vertex { position: [ 0.5, -0.5, 0.0], normal: [0.0, 0.0, 1.0], tex_coords: [1.0, 0.0] },
        crate::engine::Vertex { position: [ 0.5,  0.5, 0.0], normal: [0.0, 0.0, 1.0], tex_coords: [1.0, 1.0] },
        crate::engine::Vertex { position: [-0.5,  0.5, 0.0], normal: [0.0, 0.0, 1.0], tex_coords: [0.0, 1.0] },
    ];
    let quad_inds: Vec<u32> = vec![0, 1, 2, 0, 2, 3];
    let quad_vertex_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Quad Vertex Buffer"),
        size: (quad_verts.len() * std::mem::size_of::<crate::engine::Vertex>()) as u64,
        usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&quad_vertex_buffer, 0, bytemuck::cast_slice(&quad_verts));

    let quad_index_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Quad Index Buffer"),
        size: (quad_inds.len() * std::mem::size_of::<u32>()) as u64,
        usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&quad_index_buffer, 0, bytemuck::cast_slice(&quad_inds));
    let quad_index_count = quad_inds.len() as u32;

    // Write uniform data
    let mut uniforms = crate::engine::AudioUniforms {
        spectrum: [0.1; 1024],
        fire_heat: [0.0; 1024],
        channels: [0.8; 32],
        channel_peaks: [0.8; 32],
        spatial_channels: [0.0; 16],
        display_order: [0; 16],
        channel_phases: [0.0; 32],
        num_channels: 16,
        mode: 0,
        time: 5.0,
        duration: 200.0,
        smooth_time: 5.12,
        heatmap_row: 0,
        fft_channels: 2,
        num_spatial_channels: 2,
        ui_meters_rect: [0.0; 4],
        ui_heatmap_rect: [0.0; 4],
        ui_fire_rect: [0.0; 4],
        waveform_resolution: 1024,
        waveform_history_size: 60,
        frame_count: 300,
        step_fraction: 0.1,
        steps_to_fill: 1,
        aspect_ratio: 1.0,
        frame_dt: 0.016,
        history_cam_z: 0.0,
        fire_intensity: 1.0,
        _pad1: 0.0,
        _pad2: 0.0,
        _pad3: 0.0,
    };

    let camera_uniforms = crate::engine::CameraUniforms {
        view_matrix: glam::Mat4::IDENTITY.to_cols_array_2d(),
        proj_matrix: glam::Mat4::IDENTITY.to_cols_array_2d(),
    };
    queue.write_buffer(&camera_uniform_buffer, 0, bytemuck::cast_slice(&[camera_uniforms]));
    queue.write_buffer(&uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

    // Query set setup
    let query_set = if supports_timestamps {
        Some(device.create_query_set(&QuerySetDescriptor {
            label: Some("TimestampQuerySet"),
            ty: QueryType::Timestamp,
            count: 2,
        }))
    } else {
        None
    };

    let query_resolve_buffer = if supports_timestamps {
        Some(device.create_buffer(&BufferDescriptor {
            label: Some("QueryResolveBuffer"),
            size: 16,
            usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }))
    } else {
        None
    };

    let query_read_buffer = if supports_timestamps {
        Some(device.create_buffer(&BufferDescriptor {
            label: Some("QueryReadBuffer"),
            size: 16,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }))
    } else {
        None
    };

    let timestamp_period = queue.get_timestamp_period();

    let get_shader_source = |id: u32| -> &'static str {
        match id {
            0 => include_str!("../src/shaders/vis_spectrum.wgsl"),
            1 => include_str!("../src/shaders/vis_oscilloscope.wgsl"),
            2 => include_str!("../src/shaders/vis_3doscilloscope.wgsl"),
            3 => include_str!("../src/shaders/vis_3doscilloscope_raster.wgsl"),
            4 => include_str!("../src/shaders/vis_3doscilloscope_freq.wgsl"),
            5 => include_str!("../src/shaders/vis_flame.wgsl"),
            6 => include_str!("../src/shaders/vis_firesim.wgsl"),
            7 => include_str!("../src/shaders/vis_solar.wgsl"),
            8 => include_str!("../src/shaders/vis_spatial.wgsl"),
            9 => include_str!("../src/shaders/vis_ferrofluid.wgsl"),
            10 => include_str!("../src/shaders/vis_ferrofluidsim.wgsl"),
            11 => include_str!("../src/shaders/vis_neon_3d.wgsl"),
            12 => include_str!("../src/shaders/vis_lissajous.wgsl"),
            13 => include_str!("../src/shaders/vis_synthwave.wgsl"),
            14 => include_str!("../src/shaders/vis_synthwave_racer_3d.wgsl"),
            15 => include_str!("../src/shaders/vis_starfield.wgsl"),
            16 => include_str!("../src/shaders/vis_rain.wgsl"),
            17 => include_str!("../src/shaders/vis_storm_3d.wgsl"),
            18 => include_str!("../src/shaders/vis_cuboids.wgsl"),
            19 => include_str!("../src/shaders/vis_vumeters_3d.wgsl"),
            20 => include_str!("../src/shaders/vis_bioluminescence.wgsl"),
            21 => include_str!("../src/shaders/vis_matrix.wgsl"),
            22 => include_str!("../src/shaders/vis_neon_room.wgsl"),
            23 => include_str!("../src/shaders/vis_lyrics.wgsl"),
            _ => include_str!("../src/shaders/vis_spectrum.wgsl"),
        }
    };

    println!("\n--- RENDERING PERFORMANCE RESULTS ---");
    println!("| ID | Visualizer Name | Avg Render Time (ms) | Target Budget (0.7ms) | Status |");
    println!("| :--- | :--- | :--- | :--- | :--- |");

    for vis_def in crate::state::VISUALIZERS {
        // Skip disabled ones or just benchmark everything? Let's benchmark everything.
        let shader_source = get_shader_source(vis_def.id);
        let full_source = resolve_shader_includes(shader_source);
        let shader_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some(vis_def.name),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&full_source)),
        });

        let is_3d = matches!(vis_def.pipeline_type, crate::state::PipelineType::Mesh3D { .. });

        let layout = if vis_def.id == 20 {
            &biolum_render_pipeline_layout
        } else if is_3d {
            &render_pipeline_layout_3d
        } else {
            &render_pipeline_layout
        };

        let vs_entry = if is_3d { "vs_main_3d" } else { "vs_main" };

        let vertex_desc = crate::engine::Vertex::desc();
        let buffers = if is_3d {
            vec![vertex_desc]
        } else {
            vec![]
        };

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(vis_def.name),
            layout: Some(layout),
            vertex: VertexState {
                module: &shader_module,
                entry_point: Some(vs_entry),
                buffers: &buffers,
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: color_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::LessEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Also build lamp pipeline if ID is 13
        let lamp_buffers = vec![crate::engine::Vertex::desc()];
        let lamp_pipeline = if vis_def.id == 13 {
            Some(device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("Lamp Render Pipeline"),
                layout: Some(&render_pipeline_layout_3d),
                vertex: VertexState {
                    module: &shader_module,
                    entry_point: Some("vs_lamp"),
                    buffers: &lamp_buffers,
                    compilation_options: PipelineCompilationOptions::default(),
                },
                fragment: Some(FragmentState {
                    module: &shader_module,
                    entry_point: Some("fs_lamp"),
                    targets: &[Some(ColorTargetState {
                        format: color_format,
                        blend: Some(BlendState::REPLACE),
                        write_mask: ColorWrites::ALL,
                    })],
                    compilation_options: PipelineCompilationOptions::default(),
                }),
                primitive: PrimitiveState {
                    topology: PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: Some(DepthStencilState {
                    format: TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(CompareFunction::LessEqual),
                    stencil: StencilState::default(),
                    bias: DepthBiasState::default(),
                }),
                multisample: MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            }))
        } else {
            None
        };

        // Warm up runs (10 frames)
        for _ in 0..10 {
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut rp = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(Color::BLACK),
                            store: StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(1.0),
                            store: StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&pipeline);
                rp.set_bind_group(0, &uniform_bind_group, &[]);
                rp.set_bind_group(1, &smoke_render_bind_group, &[]);
                if is_3d {
                    rp.set_bind_group(2, &camera_bind_group, &[]);
                    if vis_def.id == 20 {
                        rp.set_bind_group(3, &biolum_render_bind_group, &[]);
                        rp.set_vertex_buffer(0, quad_vertex_buffer.slice(..));
                        rp.set_index_buffer(quad_index_buffer.slice(..), IndexFormat::Uint32);
                        rp.draw_indexed(0..quad_index_count, 0, 0..65536);
                    } else {
                        rp.set_vertex_buffer(0, grid_vertex_buffer.slice(..));
                        rp.set_index_buffer(grid_index_buffer.slice(..), IndexFormat::Uint32);
                        let instances = match vis_def.pipeline_type {
                            crate::state::PipelineType::Mesh3D { instances, .. } => instances,
                            _ => 1,
                        };
                        rp.draw_indexed(0..grid_index_count, 0, 0..instances);
                    }
                    if let Some(lp) = &lamp_pipeline {
                        rp.set_pipeline(lp);
                        rp.set_vertex_buffer(0, lamp_vertex_buffer.slice(..));
                        rp.set_index_buffer(lamp_index_buffer.slice(..), IndexFormat::Uint32);
                        rp.draw_indexed(0..lamp_index_count, 0, 0..16);
                    }
                } else {
                    rp.draw(0..3, 0..1);
                }
            }
            queue.submit(Some(encoder.finish()));
        }
        let _ = device.poll(PollType::Wait { submission_index: None, timeout: None });

        // Benchmark runs
        let num_iterations = 60;
        let mut total_time_ms = 0.0;
        
        for frame in 0..num_iterations {
            uniforms.mode = vis_def.id;
            uniforms.time = 5.0 + (frame as f32) * 0.016;
            uniforms.smooth_time = 5.12 + (frame as f32) * 0.016;
            uniforms.channels[0] = 0.95;
            uniforms.channels[1] = 0.95;
            queue.write_buffer(&uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
            
            let tw = query_set.as_ref().map(|qs| RenderPassTimestampWrites {
                query_set: qs,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            });

            let start_cpu = Instant::now();
            {
                let mut rp = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(Color::BLACK),
                            store: StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(1.0),
                            store: StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: tw,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&pipeline);
                rp.set_bind_group(0, &uniform_bind_group, &[]);
                rp.set_bind_group(1, &smoke_render_bind_group, &[]);
                if is_3d {
                    rp.set_bind_group(2, &camera_bind_group, &[]);
                    if vis_def.id == 20 {
                        rp.set_bind_group(3, &biolum_render_bind_group, &[]);
                        rp.set_vertex_buffer(0, quad_vertex_buffer.slice(..));
                        rp.set_index_buffer(quad_index_buffer.slice(..), IndexFormat::Uint32);
                        rp.draw_indexed(0..quad_index_count, 0, 0..65536);
                    } else {
                        rp.set_vertex_buffer(0, grid_vertex_buffer.slice(..));
                        rp.set_index_buffer(grid_index_buffer.slice(..), IndexFormat::Uint32);
                        let instances = match vis_def.pipeline_type {
                            crate::state::PipelineType::Mesh3D { instances, .. } => instances,
                            _ => 1,
                        };
                        rp.draw_indexed(0..grid_index_count, 0, 0..instances);
                    }
                    if let Some(lp) = &lamp_pipeline {
                        rp.set_pipeline(lp);
                        rp.set_vertex_buffer(0, lamp_vertex_buffer.slice(..));
                        rp.set_index_buffer(lamp_index_buffer.slice(..), IndexFormat::Uint32);
                        rp.draw_indexed(0..lamp_index_count, 0, 0..16);
                    }
                } else {
                    rp.draw(0..3, 0..1);
                }
            }

            if let (Some(qs), Some(res_buf), Some(read_buf)) = (&query_set, &query_resolve_buffer, &query_read_buffer) {
                encoder.resolve_query_set(qs, 0..2, res_buf, 0);
                encoder.copy_buffer_to_buffer(res_buf, 0, read_buf, 0, 16);
            }

            queue.submit(Some(encoder.finish()));
            let _ = device.poll(PollType::Wait { submission_index: None, timeout: None });

            if let Some(read_buf) = &query_read_buffer {
                let slice = read_buf.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(MapMode::Read, move |res| {
                    tx.send(res).unwrap();
                });
                let _ = device.poll(PollType::Wait { submission_index: None, timeout: None });
                rx.recv().unwrap().unwrap();

                let data = slice.get_mapped_range();
                let start: u64 = u64::from_le_bytes(data[0..8].try_into().unwrap());
                let end: u64 = u64::from_le_bytes(data[8..16].try_into().unwrap());
                drop(data);
                read_buf.unmap();

                if end > start {
                    let elapsed_ns = (end - start) as f32 * timestamp_period;
                    total_time_ms += elapsed_ns / 1_000_000.0;
                }
            } else {
                let elapsed = start_cpu.elapsed().as_secs_f32() * 1000.0;
                total_time_ms += elapsed;
            }
        }

        let avg_time_ms = total_time_ms / (num_iterations as f32);
        let status = if avg_time_ms <= 0.7 { "PASS" } else { "FAIL (SLUGGISH)" };
        println!("| {} | {} | {:.3} ms | 0.700 ms | {} |", vis_def.id, vis_def.name, avg_time_ms, status);
    }
    println!("-------------------------------------");
}

#[test]
fn test_heatmap_compute_pipeline_creation() {
    pollster::block_on(async {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            backend_options: BackendOptions::default(),
            display: None,
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
        });
        let adapter = instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }).await.unwrap();
        let mut required_features = Features::empty();
        if adapter.features().contains(Features::PIPELINE_CACHE) {
            required_features |= Features::PIPELINE_CACHE;
        }
        let (device, queue) = adapter.request_device(&DeviceDescriptor {
            required_features,
            ..Default::default()
        }).await.unwrap();
        let error_caught = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error_caught_clone = error_caught.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |e: Error| {
            eprintln!("TEST WGPU ERROR: {:?}", e);
            error_caught_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        }));

        let heatmap_compute_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("heatmap_compute_layout"),
            entries: &[
                BindGroupLayoutEntry { binding: 0, visibility: ShaderStages::COMPUTE, ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                BindGroupLayoutEntry { binding: 1, visibility: ShaderStages::COMPUTE, ty: BindingType::StorageTexture { access: StorageTextureAccess::WriteOnly, format: TextureFormat::R32Float, view_dimension: TextureViewDimension::D2 }, count: None },
                BindGroupLayoutEntry { binding: 4, visibility: ShaderStages::COMPUTE, ty: BindingType::Buffer { ty: BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let common = include_str!("../src/shaders/_common.wgsl");
        let heatmap_raw = include_str!("../src/shaders/heatmap_compute.wgsl");
        let heatmap_source = heatmap_raw.replace("// INCLUDE: common", common);
        let heatmap_compute_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Heatmap Compute Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&heatmap_source)),
        });
        let heatmap_compute_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("heatmap_compute_layout"),
            bind_group_layouts: &[Some(&heatmap_compute_layout)],
            immediate_size: 0,
        });

        // Test with pipeline cache enabled if supported
        let pipeline_cache = if device.features().contains(Features::PIPELINE_CACHE) {
            Some(unsafe {
                device.create_pipeline_cache(&PipelineCacheDescriptor {
                    label: Some("Test Pipeline Cache"),
                    data: None,
                    fallback: true,
                })
            })
        } else {
            None
        };
        let pipeline_cache_ref = pipeline_cache.as_ref();

        let heatmap_compute_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Heatmap Compute Pipeline"),
            layout: Some(&heatmap_compute_pipeline_layout),
            module: &heatmap_compute_shader,
            entry_point: Some("main"),
            compilation_options: PipelineCompilationOptions::default(),
            cache: pipeline_cache_ref,
        });

        // Biolum compute pipeline
        let biolum_compute_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Biolum Compute Layout"),
            entries: &[
                BindGroupLayoutEntry { binding: 0, visibility: ShaderStages::COMPUTE, ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                BindGroupLayoutEntry { binding: 1, visibility: ShaderStages::COMPUTE, ty: BindingType::Buffer { ty: BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let biolum_raw = include_str!("../src/shaders/biolum_compute.wgsl");
        let biolum_source = biolum_raw.replace("// INCLUDE: common", common);
        let biolum_compute_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Biolum Compute Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&biolum_source)),
        });
        let biolum_compute_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Biolum Compute Pipeline Layout"),
            bind_group_layouts: &[Some(&biolum_compute_layout)],
            immediate_size: 0,
        });
        let biolum_compute_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Biolum Compute Pipeline"),
            layout: Some(&biolum_compute_pipeline_layout),
            module: &biolum_compute_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: pipeline_cache_ref,
        });

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("Render Encoder") });
        {
            let mut compute_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("Heatmap Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&heatmap_compute_pipeline);
            compute_pass.set_pipeline(&biolum_compute_pipeline);
        }
        queue.submit(Some(encoder.finish()));
        let _ = device.poll(PollType::Wait { submission_index: None, timeout: None });

        assert!(!error_caught.load(std::sync::atomic::Ordering::SeqCst), "Validation error was triggered!");
    });
}

