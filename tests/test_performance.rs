use rusttracker::{engine, state};

use std::time::Instant;
use wgpu::*;

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

    let adapter = instance
        .request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
        .unwrap();

    println!("Selected Adapter: {:?}", adapter.get_info());

    let mut required_features = Features::empty();
    let supports_timestamps = adapter.features().contains(Features::TIMESTAMP_QUERY);
    if supports_timestamps {
        required_features |= Features::TIMESTAMP_QUERY;
        println!("Timestamp queries are supported!");
    } else {
        println!(
            "WARNING: Timestamp queries NOT supported on this GPU/driver. Falling back to CPU-side measurements."
        );
    }

    let (device, queue) = adapter
        .request_device(&DeviceDescriptor {
            label: None,
            required_features,
            required_limits: Limits::default(),
            memory_hints: MemoryHints::default(),
            ..Default::default()
        })
        .await
        .unwrap();

    // Create 1920x1080 textures
    let width = 1920;
    let height = 1080;
    let color_format = TextureFormat::Rgba8UnormSrgb;

    let render_target = device.create_texture(&TextureDescriptor {
        label: Some("RenderTarget"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
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
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
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
        size: (1024 * 144 * 4) as u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    {
        let mut wave_row = [0.0f32; 2048];
        for (i, sample) in wave_row.iter_mut().enumerate() {
            let phase = (i as f32 / 2048.0) * std::f32::consts::TAU * 4.0;
            *sample = phase.sin() * 0.65;
        }
        let mut packed_row = [0u32; 1024];
        engine::pack_wave_row_f16(&wave_row, &mut packed_row);
        queue.write_buffer(
            &waveform_storage_buffer,
            0,
            bytemuck::cast_slice(&packed_row),
        );
    }

    let history_texture = device.create_texture(&TextureDescriptor {
        label: Some("Heatmap History Texture"),
        size: Extent3d {
            width: 256,
            height: 1024,
            depth_or_array_layers: 1,
        },
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
        size: Extent3d {
            width: 1024,
            height: 576,
            depth_or_array_layers: 1,
        },
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
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
        label: Some("audio_bind_group_layout"),
    });

    let uniform_bind_group = device.create_bind_group(&BindGroupDescriptor {
        layout: &bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: waveform_storage_buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::TextureView(&history_view),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(&fire_grid_view),
            },
            BindGroupEntry {
                binding: 4,
                resource: gpu_spectrum_buffer.as_entire_binding(),
            },
        ],
        label: Some("audio_bind_group"),
    });

    let render_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
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

    let biolum_render_bind_group_layout =
        device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Biolum Render Layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    let biolum_particles_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Bioluminescent Particles"),
        size: (crate::engine::BIOLUM_PARTICLE_COUNT as u64) * 32,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let (biolum_ocean_verts, biolum_ocean_inds) = crate::engine::generate_biolum_ocean_mesh();
    let biolum_ocean_vertex_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Biolum Ocean Vertex Buffer"),
        size: (biolum_ocean_verts.len() * std::mem::size_of::<crate::engine::Vertex>()) as u64,
        usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &biolum_ocean_vertex_buffer,
        0,
        bytemuck::cast_slice(&biolum_ocean_verts),
    );
    let biolum_ocean_index_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Biolum Ocean Index Buffer"),
        size: (biolum_ocean_inds.len() * std::mem::size_of::<u32>()) as u64,
        usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &biolum_ocean_index_buffer,
        0,
        bytemuck::cast_slice(&biolum_ocean_inds),
    );
    let biolum_ocean_index_count = biolum_ocean_inds.len() as u32;

    let biolum_render_bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: Some("Biolum Render Bind Group"),
        layout: &biolum_render_bind_group_layout,
        entries: &[BindGroupEntry {
            binding: 0,
            resource: biolum_particles_buffer.as_entire_binding(),
        }],
    });

    let render_pipeline_layout_3d = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("3D Render Pipeline Layout"),
        bind_group_layouts: &[Some(&bind_group_layout), Some(&camera_bind_group_layout)],
        immediate_size: 0,
    });

    let biolum_render_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Biolum Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(&bind_group_layout),
            Some(&camera_bind_group_layout),
            Some(&biolum_render_bind_group_layout),
        ],
        immediate_size: 0,
    });

    let flip_particles_buf = device.create_buffer(&BufferDescriptor {
        label: Some("FLIP Particles Buffer"),
        size: (65536 * 32) as u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let splash_compact_buf = device.create_buffer(&BufferDescriptor {
        label: Some("Splash Compact Buffer"),
        size: ((8 + 65536) * 4) as u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let flip_render_bind_group_layout =
        device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("FLIP Render Layout"),
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
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

    let flip_render_grid = device.create_buffer(&BufferDescriptor {
        label: Some("FLIP Render Grid"),
        size: (512 * 512 * 4) as u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let flip_render_bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: Some("FLIP Render Bind Group"),
        layout: &flip_render_bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: flip_render_grid.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: flip_particles_buf.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: splash_compact_buf.as_entire_binding(),
            },
        ],
    });

    let lyrics_render_pipeline_layout_3d =
        device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Lyrics 3D Render Pipeline Layout"),
            bind_group_layouts: &[
                Some(&bind_group_layout),
                Some(&flip_render_bind_group_layout),
                Some(&camera_bind_group_layout),
            ],
            immediate_size: 0,
        });

    let acoustic_render_bind_group_layout =
        device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Acoustic Wave Volume Render Layout"),
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

    let neon_acoustic_render_pipeline_layout_3d =
        device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Neon Acoustic Studio 3D Render Pipeline Layout"),
            bind_group_layouts: &[
                Some(&bind_group_layout),
                Some(&camera_bind_group_layout),
                Some(&acoustic_render_bind_group_layout),
            ],
            immediate_size: 0,
        });

    let flip_disc_state_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Flip-Disc State Buffer"),
        size: (crate::engine::FLIP_DISC_COUNT as u64) * 16,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let flip_disc_render_bind_group_layout =
        device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Flip-Disc Render Layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
    let flip_disc_render_bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: Some("Flip-Disc Render Bind Group"),
        layout: &flip_disc_render_bind_group_layout,
        entries: &[BindGroupEntry {
            binding: 0,
            resource: flip_disc_state_buffer.as_entire_binding(),
        }],
    });
    let flip_disc_render_pipeline_layout =
        device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Flip-Disc Render Pipeline Layout"),
            bind_group_layouts: &[
                Some(&bind_group_layout),
                Some(&flip_disc_render_bind_group_layout),
            ],
            immediate_size: 0,
        });

    let acoustic_volume_texture = device.create_texture(&TextureDescriptor {
        label: Some("Acoustic Wave 3D Volume Texture"),
        size: Extent3d {
            width: 64,
            height: 32,
            depth_or_array_layers: 64,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D3,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let acoustic_seed: Vec<u8> = (0..(64 * 32 * 64))
        .flat_map(|i| {
            let x = (i % 64) as f32 * 0.25;
            let s = (x.sin().abs() * 200.0) as u8;
            let g = (x.cos().abs() * 180.0) as u8;
            let p = ((x.sin() * 0.45 + 0.5) * 255.0) as u8;
            [s, g, p, 140]
        })
        .collect();
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &acoustic_volume_texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &acoustic_seed,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64 * 4),
            rows_per_image: Some(32),
        },
        Extent3d {
            width: 64,
            height: 32,
            depth_or_array_layers: 64,
        },
    );
    let acoustic_volume_view =
        acoustic_volume_texture.create_view(&TextureViewDescriptor::default());
    let acoustic_volume_sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("Acoustic Wave Volume Sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..Default::default()
    });
    let acoustic_render_bind_group = device.create_bind_group(&BindGroupDescriptor {
        label: Some("Acoustic Wave Volume Render Bind Group"),
        layout: &acoustic_render_bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&acoustic_volume_view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&acoustic_volume_sampler),
            },
        ],
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
        crate::engine::Vertex {
            position: [-0.5, -0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            tex_coords: [0.0, 0.0],
        },
        crate::engine::Vertex {
            position: [0.5, -0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            tex_coords: [1.0, 0.0],
        },
        crate::engine::Vertex {
            position: [0.5, 0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            tex_coords: [1.0, 1.0],
        },
        crate::engine::Vertex {
            position: [-0.5, 0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            tex_coords: [0.0, 1.0],
        },
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
        vu_segment_glows: [0; 256],
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
        waveform_history_ring_base: 0,
        _pad2: 0.0,
        _pad3: 0.0,
    };

    let camera_uniforms = crate::engine::CameraUniforms {
        view_matrix: glam::Mat4::IDENTITY.to_cols_array_2d(),
        proj_matrix: glam::Mat4::IDENTITY.to_cols_array_2d(),
    };
    queue.write_buffer(
        &camera_uniform_buffer,
        0,
        bytemuck::cast_slice(&[camera_uniforms]),
    );
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
            3 => include_str!("../src/shaders/vis_3doscilloscope_raster.wgsl"),
            4 => include_str!("../src/shaders/vis_3doscilloscope_freq.wgsl"),
            5 => include_str!("../src/shaders/vis_flame.wgsl"),
            6 => include_str!("../src/shaders/vis_firesim.wgsl"),
            7 => include_str!("../src/shaders/vis_solar.wgsl"),
            13 => include_str!("../src/shaders/vis_synthwave.wgsl"),
            14 => include_str!("../src/shaders/vis_synthwave_racer_3d.wgsl"),
            16 => include_str!("../src/shaders/vis_rain.wgsl"),
            19 => include_str!("../src/shaders/vis_vumeters_3d.wgsl"),
            20 => include_str!("../src/shaders/vis_bioluminescence.wgsl"),
            23 => include_str!("../src/shaders/vis_lyrics.wgsl"),
            24 => include_str!("../src/shaders/vis_tape_head.wgsl"),
            25 => include_str!("../src/shaders/vis_spectrum_led.wgsl"),
            27 => include_str!("../src/shaders/vis_flip_ferrofluid.wgsl"),
            28 => include_str!("../src/shaders/vis_glass_water.wgsl"),
            29 => include_str!("../src/shaders/vis_spectrum_canyon.wgsl"),
            31 => include_str!("../src/shaders/vis_neon_acoustic_room.wgsl"),
            32 => include_str!("../src/shaders/vis_flip_disc.wgsl"),
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

        let is_3d = matches!(
            vis_def.pipeline_type,
            crate::state::PipelineType::Mesh3D { .. }
        );

        let layout = if vis_def.id == 20 {
            &biolum_render_pipeline_layout
        } else if vis_def.id == 31 {
            &neon_acoustic_render_pipeline_layout_3d
        } else if vis_def.id == 23 {
            &lyrics_render_pipeline_layout_3d
        } else if vis_def.id == 32 {
            &flip_disc_render_pipeline_layout
        } else if is_3d {
            &render_pipeline_layout_3d
        } else {
            &render_pipeline_layout
        };

        let vs_entry = if is_3d { "vs_main_3d" } else { "vs_main" };

        let vertex_desc = crate::engine::Vertex::desc();
        let buffers = if is_3d {
            vec![Some(vertex_desc)]
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
                    blend: Some(if vis_def.id == 20 {
                        BlendState {
                            color: BlendComponent {
                                src_factor: BlendFactor::One,
                                dst_factor: BlendFactor::One,
                                operation: BlendOperation::Add,
                            },
                            alpha: BlendComponent::OVER,
                        }
                    } else {
                        BlendState::REPLACE
                    }),
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
                depth_write_enabled: Some(vis_def.id != 20),
                depth_compare: Some(CompareFunction::LessEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let (biolum_bg_pipeline, biolum_ocean_pipeline) = if vis_def.id == 20 {
            let bg_p = device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("Biolum BG Pipeline"),
                layout: Some(&render_pipeline_layout_3d),
                vertex: VertexState {
                    module: &shader_module,
                    entry_point: Some("vs_bg"),
                    buffers: &[],
                    compilation_options: PipelineCompilationOptions::default(),
                },
                fragment: Some(FragmentState {
                    module: &shader_module,
                    entry_point: Some("fs_bg"),
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
                    depth_compare: Some(CompareFunction::Always),
                    stencil: StencilState::default(),
                    bias: DepthBiasState::default(),
                }),
                multisample: MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            let ocean_buffers = [Some(crate::engine::Vertex::desc())];
            let ocean_p = device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("Biolum Ocean Pipeline"),
                layout: Some(&render_pipeline_layout_3d),
                vertex: VertexState {
                    module: &shader_module,
                    entry_point: Some("vs_ocean"),
                    buffers: &ocean_buffers,
                    compilation_options: PipelineCompilationOptions::default(),
                },
                fragment: Some(FragmentState {
                    module: &shader_module,
                    entry_point: Some("fs_ocean"),
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
            (Some(bg_p), Some(ocean_p))
        } else {
            (None, None)
        };

        // Also build lamp pipeline if ID is 13
        let lamp_buffers = vec![Some(crate::engine::Vertex::desc())];
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
                if is_3d {
                    if vis_def.id == 23 {
                        rp.set_bind_group(1, &flip_render_bind_group, &[]);
                        rp.set_bind_group(2, &camera_bind_group, &[]);
                    } else {
                        rp.set_bind_group(1, &camera_bind_group, &[]);
                    }
                    if vis_def.id == 20 {
                        if let Some(bg_p) = &biolum_bg_pipeline {
                            rp.set_pipeline(bg_p);
                            rp.set_bind_group(0, &uniform_bind_group, &[]);
                            rp.set_bind_group(1, &camera_bind_group, &[]);
                            rp.draw(0..3, 0..1);
                        }
                        if let Some(ocean_p) = &biolum_ocean_pipeline {
                            rp.set_pipeline(ocean_p);
                            rp.set_bind_group(0, &uniform_bind_group, &[]);
                            rp.set_bind_group(1, &camera_bind_group, &[]);
                            rp.set_vertex_buffer(0, biolum_ocean_vertex_buffer.slice(..));
                            rp.set_index_buffer(
                                biolum_ocean_index_buffer.slice(..),
                                IndexFormat::Uint32,
                            );
                            rp.draw_indexed(0..biolum_ocean_index_count, 0, 0..1);
                        }
                        rp.set_pipeline(&pipeline);
                        rp.set_bind_group(0, &uniform_bind_group, &[]);
                        rp.set_bind_group(1, &camera_bind_group, &[]);
                        rp.set_bind_group(2, &biolum_render_bind_group, &[]);
                        rp.set_vertex_buffer(0, quad_vertex_buffer.slice(..));
                        rp.set_index_buffer(quad_index_buffer.slice(..), IndexFormat::Uint32);
                        rp.draw_indexed(
                            0..quad_index_count,
                            0,
                            0..crate::engine::BIOLUM_PARTICLE_COUNT,
                        );
                    } else {
                        if vis_def.id == 31 {
                            rp.set_bind_group(2, &acoustic_render_bind_group, &[]);
                        }
                        rp.set_vertex_buffer(0, grid_vertex_buffer.slice(..));
                        rp.set_index_buffer(grid_index_buffer.slice(..), IndexFormat::Uint32);
                        let (draw_index_count, instances) = match vis_def.pipeline_type {
                            crate::state::PipelineType::Mesh3D {
                                geometry: crate::state::Geometry::Grid { width, depth },
                                instances,
                            } => ((width * depth * 6).min(grid_index_count), instances),
                            crate::state::PipelineType::Mesh3D {
                                geometry: crate::state::Geometry::UnitBox,
                                instances,
                            } => (36, instances),
                            crate::state::PipelineType::Mesh3D { instances, .. } => {
                                (grid_index_count, instances)
                            }
                            _ => (grid_index_count, 1),
                        };
                        rp.draw_indexed(0..draw_index_count, 0, 0..instances);
                    }
                    if let Some(lp) = &lamp_pipeline {
                        rp.set_pipeline(lp);
                        rp.set_vertex_buffer(0, lamp_vertex_buffer.slice(..));
                        rp.set_index_buffer(lamp_index_buffer.slice(..), IndexFormat::Uint32);
                        rp.draw_indexed(0..lamp_index_count, 0, 0..16);
                    }
                } else {
                    if vis_def.id == 32 {
                        rp.set_bind_group(1, &flip_disc_render_bind_group, &[]);
                    }
                    rp.draw(0..3, 0..1);
                }
            }
            queue.submit(Some(encoder.finish()));
        }
        let _ = device.poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        });

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
                if is_3d {
                    if vis_def.id == 23 {
                        rp.set_bind_group(1, &flip_render_bind_group, &[]);
                        rp.set_bind_group(2, &camera_bind_group, &[]);
                    } else {
                        rp.set_bind_group(1, &camera_bind_group, &[]);
                    }
                    if vis_def.id == 20 {
                        if let Some(bg_p) = &biolum_bg_pipeline {
                            rp.set_pipeline(bg_p);
                            rp.set_bind_group(0, &uniform_bind_group, &[]);
                            rp.set_bind_group(1, &camera_bind_group, &[]);
                            rp.draw(0..3, 0..1);
                        }
                        if let Some(ocean_p) = &biolum_ocean_pipeline {
                            rp.set_pipeline(ocean_p);
                            rp.set_bind_group(0, &uniform_bind_group, &[]);
                            rp.set_bind_group(1, &camera_bind_group, &[]);
                            rp.set_vertex_buffer(0, biolum_ocean_vertex_buffer.slice(..));
                            rp.set_index_buffer(
                                biolum_ocean_index_buffer.slice(..),
                                IndexFormat::Uint32,
                            );
                            rp.draw_indexed(0..biolum_ocean_index_count, 0, 0..1);
                        }
                        rp.set_pipeline(&pipeline);
                        rp.set_bind_group(0, &uniform_bind_group, &[]);
                        rp.set_bind_group(1, &camera_bind_group, &[]);
                        rp.set_bind_group(2, &biolum_render_bind_group, &[]);
                        rp.set_vertex_buffer(0, quad_vertex_buffer.slice(..));
                        rp.set_index_buffer(quad_index_buffer.slice(..), IndexFormat::Uint32);
                        rp.draw_indexed(
                            0..quad_index_count,
                            0,
                            0..crate::engine::BIOLUM_PARTICLE_COUNT,
                        );
                    } else {
                        if vis_def.id == 31 {
                            rp.set_bind_group(2, &acoustic_render_bind_group, &[]);
                        }
                        rp.set_vertex_buffer(0, grid_vertex_buffer.slice(..));
                        rp.set_index_buffer(grid_index_buffer.slice(..), IndexFormat::Uint32);
                        let (draw_index_count, instances) = match vis_def.pipeline_type {
                            crate::state::PipelineType::Mesh3D {
                                geometry: crate::state::Geometry::Grid { width, depth },
                                instances,
                            } => ((width * depth * 6).min(grid_index_count), instances),
                            crate::state::PipelineType::Mesh3D {
                                geometry: crate::state::Geometry::UnitBox,
                                instances,
                            } => (36, instances),
                            crate::state::PipelineType::Mesh3D { instances, .. } => {
                                (grid_index_count, instances)
                            }
                            _ => (grid_index_count, 1),
                        };
                        rp.draw_indexed(0..draw_index_count, 0, 0..instances);
                    }
                    if let Some(lp) = &lamp_pipeline {
                        rp.set_pipeline(lp);
                        rp.set_vertex_buffer(0, lamp_vertex_buffer.slice(..));
                        rp.set_index_buffer(lamp_index_buffer.slice(..), IndexFormat::Uint32);
                        rp.draw_indexed(0..lamp_index_count, 0, 0..16);
                    }
                } else {
                    if vis_def.id == 32 {
                        rp.set_bind_group(1, &flip_disc_render_bind_group, &[]);
                    }
                    rp.draw(0..3, 0..1);
                }
            }

            if let (Some(qs), Some(res_buf), Some(read_buf)) =
                (&query_set, &query_resolve_buffer, &query_read_buffer)
            {
                encoder.resolve_query_set(qs, 0..2, res_buf, 0);
                encoder.copy_buffer_to_buffer(res_buf, 0, read_buf, 0, 16);
            }

            queue.submit(Some(encoder.finish()));
            let _ = device.poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });

            if let Some(read_buf) = &query_read_buffer {
                let slice = read_buf.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(MapMode::Read, move |res| {
                    tx.send(res).unwrap();
                });
                let _ = device.poll(PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });
                rx.recv().unwrap().unwrap();

                let data = slice.get_mapped_range().unwrap();
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
        let status = if avg_time_ms <= 0.7 {
            "PASS"
        } else {
            "FAIL (SLUGGISH)"
        };
        println!(
            "| {} | {} | {:.3} ms | 0.700 ms | {} |",
            vis_def.id, vis_def.name, avg_time_ms, status
        );
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
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .unwrap();
        let mut required_features = Features::empty();
        if adapter.features().contains(Features::PIPELINE_CACHE) {
            required_features |= Features::PIPELINE_CACHE;
        }
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                required_features,
                ..Default::default()
            })
            .await
            .unwrap();
        let error_caught = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error_caught_clone = error_caught.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |e: Error| {
            eprintln!("TEST WGPU ERROR: {:?}", e);
            error_caught_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        }));

        let heatmap_compute_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("heatmap_compute_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::StorageTexture {
                        access: StorageTextureAccess::WriteOnly,
                        format: TextureFormat::R32Float,
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 4,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let common = include_str!("../src/shaders/_common.wgsl");
        let heatmap_raw = include_str!("../src/shaders/heatmap_compute.wgsl");
        let heatmap_source = heatmap_raw.replace("// INCLUDE: common", common);
        let heatmap_compute_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Heatmap Compute Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&heatmap_source)),
        });
        let heatmap_compute_pipeline_layout =
            device.create_pipeline_layout(&PipelineLayoutDescriptor {
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
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let biolum_raw = include_str!("../src/shaders/biolum_compute.wgsl");
        let biolum_source = biolum_raw.replace("// INCLUDE: common", common);
        let biolum_compute_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Biolum Compute Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&biolum_source)),
        });
        let biolum_compute_pipeline_layout =
            device.create_pipeline_layout(&PipelineLayoutDescriptor {
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

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        {
            let mut compute_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("Heatmap Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&heatmap_compute_pipeline);
            compute_pass.set_pipeline(&biolum_compute_pipeline);
        }
        queue.submit(Some(encoder.finish()));
        let _ = device.poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        });

        assert!(
            !error_caught.load(std::sync::atomic::Ordering::SeqCst),
            "Validation error was triggered!"
        );
    });
}

#[test]
fn test_gpu_video_two_plane_nv12_p010_and_quarter_res_border_blur() {
    pollster::block_on(async {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            backend_options: BackendOptions::default(),
            display: None,
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
        });
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("GPU adapter required for video test");

        let has_16bit_norm = adapter
            .features()
            .contains(Features::TEXTURE_FORMAT_16BIT_NORM);
        let required_features = if has_16bit_norm {
            Features::TEXTURE_FORMAT_16BIT_NORM
        } else {
            Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("Video 2-Plane GPU Test Device"),
                required_features,
                required_limits: Limits::default(),
                memory_hints: MemoryHints::default(),
                ..Default::default()
            })
            .await
            .expect("request_device");

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct TestVideoParams {
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

        let video_src = include_str!("../src/shaders/vis_video.wgsl");
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("vis_video.wgsl"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(video_src)),
        });

        // Bind group layout for quarter-res blur pass (bindings 0..=4)
        let blur_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Blur BGL"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 4,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Bind group layout for main video pass (bindings 0..=5)
        let main_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Main Video BGL"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 4,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 5,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let blur_pl = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Blur PL"),
            bind_group_layouts: &[Some(&blur_bgl)],
            immediate_size: 0,
        });
        let main_pl = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Main PL"),
            bind_group_layouts: &[Some(&main_bgl)],
            immediate_size: 0,
        });

        let blur_pipe = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Quarter-Res Border Blur Pipe"),
            layout: Some(&blur_pl),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_border_blur"),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8UnormSrgb,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let main_pipe = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Main Video Pipe"),
            layout: Some(&main_pl),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8UnormSrgb,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            ..Default::default()
        });

        // Viewport 320x180 (16:9), video 160x160 (1:1 pillarboxed inside 16:9!)
        let vp_w = 320u32;
        let vp_h = 180u32;
        let vid_w = 160u32;
        let vid_h = 160u32;

        let out_tex = device.create_texture(&TextureDescriptor {
            label: Some("Video Out"),
            size: Extent3d {
                width: vp_w,
                height: vp_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out_view = out_tex.create_view(&TextureViewDescriptor::default());

        let blur_tex = device.create_texture(&TextureDescriptor {
            label: Some("Quarter Blur Tex"),
            size: Extent3d {
                width: 40,
                height: 40,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let blur_view = blur_tex.create_view(&TextureViewDescriptor::default());

        let dummy_v = device.create_texture(&TextureDescriptor {
            label: Some("Dummy V"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let dummy_v_view = dummy_v.create_view(&TextureViewDescriptor::default());

        // Helper closure to read back a single RGBA pixel from out_tex
        let readback_pixel = |px_x: u32, px_y: u32| -> [u8; 4] {
            let bpr = 320u32.div_ceil(64) * 256; // 1280 (multiple of 256)
            let rb = device.create_buffer(&BufferDescriptor {
                label: Some("Video RB"),
                size: (bpr * vp_h) as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            enc.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &out_tex,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &rb,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bpr),
                        rows_per_image: Some(vp_h),
                    },
                },
                Extent3d {
                    width: vp_w,
                    height: vp_h,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(enc.finish()));
            let sl = rb.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            sl.map_async(MapMode::Read, move |r| tx.send(r).unwrap());
            let _ = device.poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.recv().unwrap().unwrap();
            let mapped = sl.get_mapped_range().unwrap();
            let off = (px_y * bpr + px_x * 4) as usize;
            let rgba = [
                mapped[off],
                mapped[off + 1],
                mapped[off + 2],
                mapped[off + 3],
            ];
            drop(mapped);
            rb.unmap();
            rgba
        };

        // --- CASE A: 2-Plane 8-bit NV12 (R8Unorm + Rg8Unorm, chroma_layout = 1) ---
        let nv12_y = device.create_texture(&TextureDescriptor {
            label: Some("NV12 Y"),
            size: Extent3d {
                width: vid_w,
                height: vid_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let nv12_uv = device.create_texture(&TextureDescriptor {
            label: Some("NV12 UV"),
            size: Extent3d {
                width: vid_w / 2,
                height: vid_h / 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Bright saturated cyan-ish video: Y=180, U=170, V=80
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &nv12_y,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &vec![180u8; (256 * vid_h) as usize],
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(vid_h),
            },
            Extent3d {
                width: vid_w,
                height: vid_h,
                depth_or_array_layers: 1,
            },
        );
        let mut uv_data = vec![0u8; (256 * (vid_h / 2)) as usize];
        for pair in uv_data.as_chunks_mut::<2>().0 {
            *pair = [170, 80];
        }
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &nv12_uv,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &uv_data,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(vid_h / 2),
            },
            Extent3d {
                width: vid_w / 2,
                height: vid_h / 2,
                depth_or_array_layers: 1,
            },
        );

        let params_nv12 = TestVideoParams {
            color_space: 0, // BT.709
            color_range: 0, // Limited
            bit_depth: 8,
            color_trc: 0,
            viewport_width: vp_w as f32,
            viewport_height: vp_h as f32,
            video_width: vid_w as f32,
            video_height: vid_h as f32,
            rotation: 0,
            chroma_layout: 1, // SemiPlanar420
            _pad2: 0.0,
            _pad3: 0.0,
        };
        let params_buf = device.create_buffer(&BufferDescriptor {
            label: Some("VideoParams"),
            size: std::mem::size_of::<TestVideoParams>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params_nv12));

        let nv12_y_view = nv12_y.create_view(&TextureViewDescriptor::default());
        let nv12_uv_view = nv12_uv.create_view(&TextureViewDescriptor::default());

        let blur_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("NV12 Blur BG"),
            layout: &blur_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&nv12_y_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&nv12_uv_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&dummy_v_view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });
        let main_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("NV12 Main BG"),
            layout: &main_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&nv12_y_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&nv12_uv_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&dummy_v_view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: params_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::TextureView(&blur_view),
                },
            ],
        });

        let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut bp = enc.begin_render_pass(&RenderPassDescriptor {
                label: Some("Blur Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &blur_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            bp.set_pipeline(&blur_pipe);
            bp.set_bind_group(0, &blur_bg, &[]);
            bp.draw(0..3, 0..1);
        }
        {
            let mut mp = enc.begin_render_pass(&RenderPassDescriptor {
                label: Some("Main Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &out_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            mp.set_pipeline(&main_pipe);
            mp.set_bind_group(0, &main_bg, &[]);
            mp.draw(0..3, 0..1);
        }
        queue.submit(Some(enc.finish()));

        let center_nv12 = readback_pixel(160, 90);
        let pillarbox_nv12 = readback_pixel(20, 90);
        // Active cyan video center: B > G > R, bright; pillarbox border: dimmed blurred version (>0, <center)!
        assert!(
            center_nv12[2] > center_nv12[0] + 40 && center_nv12[1] > 150,
            "2-Plane NV12 center pixel must decode cyan-ish Rec.709 color, got {center_nv12:?}"
        );
        assert!(
            pillarbox_nv12[1] > 20 && pillarbox_nv12[1] < center_nv12[1],
            "Quarter-res pre-blurred pillarbox border must be non-black and dimmed relative to active video, got border={pillarbox_nv12:?} vs center={center_nv12:?}"
        );

        // --- CASE A2: 2-Plane 8-bit NV21 (Rg8Unorm VU, chroma_layout = 2) with 90° rotation ---
        // Swapping chroma_layout to 2u on the exact same [170, 80] texture interprets V=170, U=80 (warm orange/red: R > B)!
        let params_nv21_rot90 = TestVideoParams {
            rotation: 90,
            chroma_layout: 2, // SemiPlanarVu420 (NV21)
            ..params_nv12
        };
        queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params_nv21_rot90));
        let mut enc_nv21 = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut bp = enc_nv21.begin_render_pass(&RenderPassDescriptor {
                label: Some("NV21 Blur Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &blur_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            bp.set_pipeline(&blur_pipe);
            bp.set_bind_group(0, &blur_bg, &[]);
            bp.draw(0..3, 0..1);
        }
        {
            let mut mp = enc_nv21.begin_render_pass(&RenderPassDescriptor {
                label: Some("NV21 Main Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &out_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            mp.set_pipeline(&main_pipe);
            mp.set_bind_group(0, &main_bg, &[]);
            mp.draw(0..3, 0..1);
        }
        queue.submit(Some(enc_nv21.finish()));
        let center_nv21 = readback_pixel(160, 90);
        let pillarbox_nv21 = readback_pixel(60, 90);
        assert!(
            center_nv21[0] > center_nv21[2] + 40,
            "2-Plane NV21 (chroma_layout=2) must decode VU pair (R > B), got {center_nv21:?}"
        );
        assert!(
            pillarbox_nv21[0] > pillarbox_nv21[2] && pillarbox_nv21[0] < center_nv21[0],
            "90°-rotated NV21 pre-blurred pillarbox border must preserve warm hue and dim cleanly, got border={pillarbox_nv21:?} vs center={center_nv21:?}"
        );

        // --- CASE B: 2-Plane 10-bit P010 (R16Unorm + Rg16Unorm, bit_depth = 16, neutral 512<<6 chroma) ---
        if has_16bit_norm {
            let p010_y = device.create_texture(&TextureDescriptor {
                label: Some("P010 Y"),
                size: Extent3d {
                    width: vid_w,
                    height: vid_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::R16Unorm,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let p010_uv = device.create_texture(&TextureDescriptor {
                label: Some("P010 UV"),
                size: Extent3d {
                    width: vid_w / 2,
                    height: vid_h / 2,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rg16Unorm,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let mut y16 = vec![0u8; (512 * vid_h) as usize];
            for w in y16.as_chunks_mut::<2>().0 {
                *w = (512u16 << 6).to_le_bytes();
            }
            let mut uv16 = vec![0u8; (512 * (vid_h / 2)) as usize];
            for uv in uv16.as_chunks_mut::<4>().0 {
                let neutral = (512u16 << 6).to_le_bytes();
                uv[0..2].copy_from_slice(&neutral);
                uv[2..4].copy_from_slice(&neutral);
            }
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &p010_y,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                &y16,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(512),
                    rows_per_image: Some(vid_h),
                },
                Extent3d {
                    width: vid_w,
                    height: vid_h,
                    depth_or_array_layers: 1,
                },
            );
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &p010_uv,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                &uv16,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(512),
                    rows_per_image: Some(vid_h / 2),
                },
                Extent3d {
                    width: vid_w / 2,
                    height: vid_h / 2,
                    depth_or_array_layers: 1,
                },
            );

            let params_p010 = TestVideoParams {
                bit_depth: 16,
                ..params_nv12
            };
            queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params_p010));

            let p010_y_view = p010_y.create_view(&TextureViewDescriptor::default());
            let p010_uv_view = p010_uv.create_view(&TextureViewDescriptor::default());
            let p010_main_bg = device.create_bind_group(&BindGroupDescriptor {
                label: Some("P010 Main BG"),
                layout: &main_bgl,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&p010_y_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&p010_uv_view),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&dummy_v_view),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Sampler(&sampler),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: params_buf.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: BindingResource::TextureView(&blur_view),
                    },
                ],
            });
            let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut mp = enc.begin_render_pass(&RenderPassDescriptor {
                    label: Some("P010 Main Pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &out_view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(Color::BLACK),
                            store: StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                mp.set_pipeline(&main_pipe);
                mp.set_bind_group(0, &p010_main_bg, &[]);
                mp.draw(0..3, 0..1);
            }
            queue.submit(Some(enc.finish()));

            let center_p010 = readback_pixel(160, 90);
            // Neutral 10-bit (512<<6) chroma must render neutral grey (|R-G| <= 2, |G-B| <= 2) with zero green/magenta bias!
            assert!(
                center_p010[0].abs_diff(center_p010[1]) <= 2
                    && center_p010[1].abs_diff(center_p010[2]) <= 2
                    && center_p010[0] > 100,
                "2-Plane 10-bit P010 neutral (512<<6) chroma must render neutral grey without tint, got {center_p010:?}"
            );
        }
    });
}

#[test]
fn test_acoustic_wave_chamber_compute_and_nodal_interference() {
    pollster::block_on(async {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            backend_options: BackendOptions::default(),
            display: None,
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
        });
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("GPU adapter required for acoustic wave compute test");
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("Acoustic Wave Test Device"),
                required_features: Features::empty(),
                required_limits: Limits::default(),
                memory_hints: MemoryHints::default(),
                ..Default::default()
            })
            .await
            .expect("GPU device required");

        const VOXELS: usize = 64 * 32 * 64;
        let uniform_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Acoustic Test Uniforms"),
            size: std::mem::size_of::<engine::AudioUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wave_a_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Acoustic Test Wave A"),
            size: (VOXELS * 16) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wave_b_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Acoustic Test Wave B"),
            size: (VOXELS * 16) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let packed_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Acoustic Test Packed"),
            size: (VOXELS * 4) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Acoustic Test Readback"),
            size: (VOXELS * 4) as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Acoustic Test Compute Layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Acoustic Test Compute Bind Group"),
            layout: &layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: wave_a_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: wave_b_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: packed_buf.as_entire_binding(),
                },
            ],
        });

        let compute_src =
            resolve_shader_includes(include_str!("../src/shaders/acoustic_wave_compute.wgsl"));
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Acoustic Test Compute Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&compute_src)),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Acoustic Test Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let clear_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Acoustic Test Clear"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_clear_waves"),
            compilation_options: Default::default(),
            cache: None,
        });
        let step_ab_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Acoustic Test Step AB"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_step_ab"),
            compilation_options: Default::default(),
            cache: None,
        });
        let step_ba_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Acoustic Test Step BA"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_step_ba"),
            compilation_options: Default::default(),
            cache: None,
        });

        let run_case = |phase_fr: f32| -> Vec<u32> {
            let mut uniforms: engine::AudioUniforms = bytemuck::Zeroable::zeroed();
            uniforms.mode = 30;
            uniforms.num_channels = 12;
            uniforms.num_spatial_channels = 12;
            uniforms.spatial_channels[0] = 1.0; // FL (-3.8, -0.25, 5.5)
            uniforms.spatial_channels[1] = 1.0; // FR (+3.8, -0.25, 5.5)
            uniforms.channel_phases[0] = 0.0;
            uniforms.channel_phases[1] = phase_fr;
            uniforms.channel_phases[16] = 0.25;
            uniforms.channel_phases[31] = 1.0;
            uniforms.frame_dt = 1.0 / 60.0;
            queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[uniforms]));

            let mut clear_enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut cp = clear_enc.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("Acoustic Test Clear Pass"),
                    timestamp_writes: None,
                });
                cp.set_pipeline(&clear_pipe);
                cp.set_bind_group(0, Some(&bind_group), &[]);
                cp.dispatch_workgroups(8, 8, 16);
            }
            queue.submit(Some(clear_enc.finish()));

            for frame in 0..60u32 {
                uniforms.frame_count = frame;
                uniforms.channel_phases[17] =
                    (frame as f32 * 0.40).rem_euclid(std::f32::consts::TAU);
                queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[uniforms]));

                let mut step_enc =
                    device.create_command_encoder(&CommandEncoderDescriptor::default());
                {
                    let mut cp = step_enc.begin_compute_pass(&ComputePassDescriptor {
                        label: Some("Acoustic Test Step Pass"),
                        timestamp_writes: None,
                    });
                    cp.set_bind_group(0, Some(&bind_group), &[]);
                    cp.set_pipeline(&step_ab_pipe);
                    cp.dispatch_workgroups(8, 8, 16);
                    cp.set_pipeline(&step_ba_pipe);
                    cp.dispatch_workgroups(8, 8, 16);
                }
                if frame == 59 {
                    step_enc.copy_buffer_to_buffer(
                        &packed_buf,
                        0,
                        &staging_buf,
                        0,
                        (VOXELS * 4) as u64,
                    );
                }
                queue.submit(Some(step_enc.finish()));
            }

            let slice = staging_buf.slice(..);
            slice.map_async(MapMode::Read, |r| r.unwrap());
            let _ = device.poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            let mapped = slice.get_mapped_range().unwrap();
            let voxels: Vec<u32> = bytemuck::cast_slice(&mapped).to_vec();
            drop(mapped);
            staging_buf.unmap();
            voxels
        };

        let in_phase = run_case(0.0);
        let out_phase = run_case(std::f32::consts::PI);

        let vox_at = |buf: &[u32], ix: usize, iy: usize, iz: usize| -> [u8; 4] {
            buf[ix + iy * 64 + iz * 2048].to_le_bytes()
        };

        let mut in_mid_r_sum = 0.0f32;
        let mut out_mid_r_sum = 0.0f32;
        let mut out_mid_g_sum = 0.0f32;
        let mut mid_pair_diff_max = 0.0f32;
        let mut count = 0usize;

        for iz in 38..=56 {
            for iy in 5..=12 {
                let in_l = vox_at(&in_phase, 31, iy, iz);
                let in_r = vox_at(&in_phase, 32, iy, iz);
                let out_l = vox_at(&out_phase, 31, iy, iz);
                let out_r = vox_at(&out_phase, 32, iy, iz);

                in_mid_r_sum += (in_l[0] as f32 + in_r[0] as f32) * 0.5;
                out_mid_r_sum += (out_l[0] as f32 + out_r[0] as f32) * 0.5;
                out_mid_g_sum += (out_l[1] as f32 + out_r[1] as f32) * 0.5;

                // Trilinear pressure average right at the X=0 median plane (midpoint of ix=31 and ix=32)
                let p_mid_signed = ((out_l[2] as f32 - 127.5) + (out_r[2] as f32 - 127.5)) * 0.5;
                mid_pair_diff_max = mid_pair_diff_max.max(p_mid_signed.abs());
                count += 1;
            }
        }

        let in_mid_r_avg = in_mid_r_sum / count as f32;
        let out_mid_r_avg = out_mid_r_sum / count as f32;
        let out_mid_g_avg = out_mid_g_sum / count as f32;

        // Also check off-axis standing energy near FL in the out-of-phase run
        let mut off_axis_r_sum = 0.0f32;
        let mut off_count = 0usize;
        for iz in 44..=56 {
            for iy in 5..=10 {
                for ix in 18..=24 {
                    off_axis_r_sum += vox_at(&out_phase, ix, iy, iz)[0] as f32;
                    off_count += 1;
                }
            }
        }
        let off_axis_r_avg = off_axis_r_sum / off_count as f32;

        assert!(
            in_mid_r_avg > 20.0,
            "Constructive in-phase FL+FR must create strong X=0 antinode pressure, got avg R={in_mid_r_avg}"
        );
        assert!(
            in_mid_r_avg > out_mid_r_avg * 2.5,
            "Constructive X=0 pressure ({in_mid_r_avg}) must strongly exceed destructive cancellation ({out_mid_r_avg})"
        );
        assert!(
            mid_pair_diff_max <= 3.0,
            "Destructive out-of-phase median X=0 pressure interpolation must cancel near zero, got max |p_mid|={mid_pair_diff_max} bytes"
        );
        assert!(
            off_axis_r_avg > 15.0 && out_mid_g_avg > 10.0,
            "Out-of-phase run must maintain strong off-axis lobes ({off_axis_r_avg}) and high X=0 gradient velocity ({out_mid_g_avg})"
        );
    });
}

#[test]
fn test_neon_acoustic_studio_smoke_blasts_and_toggle() {
    pollster::block_on(async {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            backend_options: BackendOptions::default(),
            display: None,
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
        });
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("GPU adapter required for Neon Acoustic Studio test");
        let has_timestamp_query = adapter.features().contains(Features::TIMESTAMP_QUERY);
        let required_features = if has_timestamp_query {
            Features::TIMESTAMP_QUERY
        } else {
            Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("Neon Acoustic Studio Test Device"),
                required_features,
                required_limits: Limits::default(),
                memory_hints: MemoryHints::default(),
                ..Default::default()
            })
            .await
            .expect("GPU device required");

        const VOXELS: usize = 64 * 32 * 64;
        let uniform_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Studio Test Uniforms"),
            size: std::mem::size_of::<engine::AudioUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wave_a_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Studio Test Wave A"),
            size: (VOXELS * 16) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wave_b_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Studio Test Wave B"),
            size: (VOXELS * 16) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let packed_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Studio Test Packed"),
            size: (VOXELS * 4) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let comp_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Studio Test Compute BGL"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let comp_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Studio Test Compute BG"),
            layout: &comp_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: wave_a_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: wave_b_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: packed_buf.as_entire_binding(),
                },
            ],
        });

        let compute_src =
            resolve_shader_includes(include_str!("../src/shaders/acoustic_wave_compute.wgsl"));
        let comp_mod = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Studio Test Compute Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&compute_src)),
        });
        let comp_pipe_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Studio Test Compute Pipeline Layout"),
            bind_group_layouts: &[Some(&comp_bgl)],
            immediate_size: 0,
        });
        let clear_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Studio Test Clear"),
            layout: Some(&comp_pipe_layout),
            module: &comp_mod,
            entry_point: Some("cs_clear_waves"),
            compilation_options: Default::default(),
            cache: None,
        });
        let step_ab_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Studio Test Step AB"),
            layout: Some(&comp_pipe_layout),
            module: &comp_mod,
            entry_point: Some("cs_step_ab"),
            compilation_options: Default::default(),
            cache: None,
        });
        let step_ba_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Studio Test Step BA"),
            layout: Some(&comp_pipe_layout),
            module: &comp_mod,
            entry_point: Some("cs_step_ba"),
            compilation_options: Default::default(),
            cache: None,
        });

        let r_w = 640u32;
        let r_h = 360u32;
        let mut uniforms: engine::AudioUniforms = bytemuck::Zeroable::zeroed();
        uniforms.mode = 31;
        uniforms.num_channels = 12;
        uniforms.num_spatial_channels = 12;
        uniforms.aspect_ratio = r_w as f32 / r_h as f32;
        uniforms.spatial_channels = [1.0; 16];
        uniforms.spectrum[1..20].fill(65.0);
        uniforms.spectrum[20..80].fill(20.0);
        uniforms.channel_phases[16] = 0.08;
        uniforms.channel_phases[18] = -2.8;
        uniforms.channel_phases[19] = 2.0;
        uniforms.channel_phases[20] = 2.6;
        uniforms.channel_phases[21] = 0.85;
        uniforms.channel_phases[22] = 2.6;
        uniforms.channel_phases[23] = 1.1;
        uniforms.channel_phases[24] = -1.8;
        uniforms.channel_phases[25] = 0.85;
        uniforms.channel_phases[26] = 0.4;
        uniforms.channel_phases[27] = 0.6;
        uniforms.channel_phases[28] = 1.9;
        uniforms.channel_phases[29] = 0.85;
        uniforms.channel_phases[30] = 1.0;
        uniforms.channel_phases[31] = 1.0;
        uniforms.frame_dt = 1.0 / 60.0;

        let vol_tex = device.create_texture(&TextureDescriptor {
            label: Some("Studio Test Volume 3D"),
            size: Extent3d {
                width: 64,
                height: 32,
                depth_or_array_layers: 64,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D3,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let vol_view = vol_tex.create_view(&TextureViewDescriptor::default());
        let vol_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("Studio Test Volume Sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });

        let mut clear_enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut cp = clear_enc.begin_compute_pass(&ComputePassDescriptor {
                label: Some("Studio Test Clear Pass"),
                timestamp_writes: None,
            });
            cp.set_pipeline(&clear_pipe);
            cp.set_bind_group(0, Some(&comp_bg), &[]);
            cp.dispatch_workgroups(8, 8, 16);
        }
        queue.submit(Some(clear_enc.finish()));

        for frame in 0..90u32 {
            uniforms.frame_count = frame;
            uniforms.smooth_time = frame as f32 * (1.0 / 60.0);
            uniforms.channel_phases[17] = (uniforms.channel_phases[17]
                + (1.0 / 60.0) * (28.0 + 14.0 * uniforms.channel_phases[16]))
                .rem_euclid(std::f32::consts::TAU);
            queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[uniforms]));

            let mut step_enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut cp = step_enc.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("Studio Test Step Pass"),
                    timestamp_writes: None,
                });
                cp.set_bind_group(0, Some(&comp_bg), &[]);
                cp.set_pipeline(&step_ab_pipe);
                cp.dispatch_workgroups(8, 8, 16);
                cp.set_pipeline(&step_ba_pipe);
                cp.dispatch_workgroups(8, 8, 16);
            }
            if frame == 89 {
                step_enc.copy_buffer_to_texture(
                    TexelCopyBufferInfo {
                        buffer: &packed_buf,
                        layout: TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(64 * 4),
                            rows_per_image: Some(32),
                        },
                    },
                    TexelCopyTextureInfo {
                        texture: &vol_tex,
                        mip_level: 0,
                        origin: Origin3d::ZERO,
                        aspect: TextureAspect::All,
                    },
                    Extent3d {
                        width: 64,
                        height: 32,
                        depth_or_array_layers: 64,
                    },
                );
            }
            queue.submit(Some(step_enc.finish()));
        }

        // Synchronize smooth_time and CameraUniforms look_at_rh eye position (Shot 0 Master View)
        let render_smooth_time = 1.50f32;
        uniforms.smooth_time = render_smooth_time;
        let (cam_eye, cam_target) =
            engine::compute_neon_acoustic_studio_camera(render_smooth_time as f64);
        let cam_view = glam::Mat4::look_at_rh(cam_eye, cam_target, glam::Vec3::Y);
        let cam_proj = glam::Mat4::perspective_rh_gl(
            64.0_f32.to_radians(),
            r_w as f32 / r_h as f32,
            0.1,
            1000.0,
        );
        let camera_uniforms = engine::CameraUniforms {
            view_matrix: cam_view.to_cols_array_2d(),
            proj_matrix: cam_proj.to_cols_array_2d(),
        };
        let camera_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Studio Test Camera Uniforms"),
            size: std::mem::size_of::<engine::CameraUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&camera_buf, 0, bytemuck::cast_slice(&[camera_uniforms]));

        let (room_verts, room_inds) = engine::generate_neon_room_mesh();
        let room_vb = device.create_buffer(&BufferDescriptor {
            label: Some("Studio Test Room VB"),
            size: (room_verts.len() * std::mem::size_of::<engine::Vertex>()) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&room_vb, 0, bytemuck::cast_slice(&room_verts));
        let room_ib = device.create_buffer(&BufferDescriptor {
            label: Some("Studio Test Room IB"),
            size: (room_inds.len() * std::mem::size_of::<u32>()) as u64,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&room_ib, 0, bytemuck::cast_slice(&room_inds));
        let room_index_count = room_inds.len() as u32;

        let bgl0 = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Studio Test BGL0"),
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
        let bgl1 = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Studio Test BGL1"),
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
        let bgl2 = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Studio Test BGL2"),
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
        let bg0 = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Studio Test BG0"),
            layout: &bgl0,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let bg1 = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Studio Test BG1"),
            layout: &bgl1,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });
        let bg2 = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Studio Test BG2"),
            layout: &bgl2,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&vol_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&vol_sampler),
                },
            ],
        });

        let vis_src =
            resolve_shader_includes(include_str!("../src/shaders/vis_neon_acoustic_room.wgsl"));
        let vis_mod = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Studio Test Render Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&vis_src)),
        });
        let render_pipe_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Studio Test Render Pipeline Layout"),
            bind_group_layouts: &[Some(&bgl0), Some(&bgl1), Some(&bgl2)],
            immediate_size: 0,
        });
        let render_pipe = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Studio Test Render Pipeline"),
            layout: Some(&render_pipe_layout),
            vertex: VertexState {
                module: &vis_mod,
                entry_point: Some("vs_main_3d"),
                buffers: &[Some(engine::Vertex::desc())],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &vis_mod,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8UnormSrgb,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
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

        let render_target = device.create_texture(&TextureDescriptor {
            label: Some("Studio Test Color Target"),
            size: Extent3d {
                width: r_w,
                height: r_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let render_view = render_target.create_view(&TextureViewDescriptor::default());
        let depth_tex = device.create_texture(&TextureDescriptor {
            label: Some("Studio Test Depth Target"),
            size: Extent3d {
                width: r_w,
                height: r_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_tex.create_view(&TextureViewDescriptor::default());
        let img_readback_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Studio Test Readback"),
            size: (r_w * r_h * 4) as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut perf_uniforms = uniforms;
        perf_uniforms._pad2 = 0.0;

        let render_with_smoke = |smoke_pad: f32, out_path: &str| -> Vec<u8> {
            let mut u = uniforms;
            u._pad2 = smoke_pad;
            queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[u]));

            let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                    label: Some("Studio Test Render Pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &render_view,
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
                            store: StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&render_pipe);
                rp.set_bind_group(0, &bg0, &[]);
                rp.set_bind_group(1, &bg1, &[]);
                rp.set_bind_group(2, &bg2, &[]);
                rp.set_vertex_buffer(0, room_vb.slice(..));
                rp.set_index_buffer(room_ib.slice(..), IndexFormat::Uint32);
                rp.draw_indexed(0..room_index_count, 0, 0..1);
            }
            enc.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &render_target,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &img_readback_buf,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(r_w * 4),
                        rows_per_image: Some(r_h),
                    },
                },
                Extent3d {
                    width: r_w,
                    height: r_h,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(enc.finish()));

            let img_slice = img_readback_buf.slice(..);
            img_slice.map_async(MapMode::Read, |r| r.unwrap());
            let _ = device.poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            let mapped = img_slice.get_mapped_range().unwrap();
            let bytes = mapped.to_vec();
            drop(mapped);
            img_readback_buf.unmap();

            let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(r_w, r_h, bytes.clone())
                .expect("640x360 RGBA buffer must build image");
            img.save(out_path).expect("Save PNG image");
            bytes
        };

        let rgba_on = render_with_smoke(0.0, "target/vis_neon_acoustic_studio_smoke_on.png");
        let rgba_off = render_with_smoke(1.0, "target/vis_neon_acoustic_studio_smoke_off.png");

        queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[perf_uniforms]));
        for _ in 0..10 {
            let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                    label: Some("Studio Perf Warmup Pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &render_view,
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
                            store: StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&render_pipe);
                rp.set_bind_group(0, &bg0, &[]);
                rp.set_bind_group(1, &bg1, &[]);
                rp.set_bind_group(2, &bg2, &[]);
                rp.set_vertex_buffer(0, room_vb.slice(..));
                rp.set_index_buffer(room_ib.slice(..), IndexFormat::Uint32);
                rp.draw_indexed(0..room_index_count, 0, 0..1);
            }
            queue.submit(Some(enc.finish()));
            let _ = device.poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        }

        if has_timestamp_query {
            let qs = device.create_query_set(&QuerySetDescriptor {
                label: Some("Studio Perf Timestamp Query Set"),
                ty: QueryType::Timestamp,
                count: 2,
            });
            let res_buf = device.create_buffer(&BufferDescriptor {
                label: Some("Studio Perf Timestamp Resolve"),
                size: 16,
                usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read_buf = device.create_buffer(&BufferDescriptor {
                label: Some("Studio Perf Timestamp Read"),
                size: 16,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let timestamp_period = queue.get_timestamp_period();
            let perf_iters = 40u32;
            let mut total_ms = 0.0f32;
            for _ in 0..perf_iters {
                let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
                {
                    let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                        label: Some("Studio Perf Timed Pass"),
                        color_attachments: &[Some(RenderPassColorAttachment {
                            view: &render_view,
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
                                store: StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: Some(RenderPassTimestampWrites {
                            query_set: &qs,
                            beginning_of_pass_write_index: Some(0),
                            end_of_pass_write_index: Some(1),
                        }),
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    rp.set_pipeline(&render_pipe);
                    rp.set_bind_group(0, &bg0, &[]);
                    rp.set_bind_group(1, &bg1, &[]);
                    rp.set_bind_group(2, &bg2, &[]);
                    rp.set_vertex_buffer(0, room_vb.slice(..));
                    rp.set_index_buffer(room_ib.slice(..), IndexFormat::Uint32);
                    rp.draw_indexed(0..room_index_count, 0, 0..1);
                }
                enc.resolve_query_set(&qs, 0..2, &res_buf, 0);
                enc.copy_buffer_to_buffer(&res_buf, 0, &read_buf, 0, 16);
                queue.submit(Some(enc.finish()));
                let _ = device.poll(PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });
                let slice = read_buf.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(MapMode::Read, move |res| {
                    tx.send(res).unwrap();
                });
                let _ = device.poll(PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });
                rx.recv().unwrap().unwrap();
                let data = slice.get_mapped_range().unwrap();
                let start = u64::from_le_bytes(data[0..8].try_into().unwrap());
                let end = u64::from_le_bytes(data[8..16].try_into().unwrap());
                drop(data);
                read_buf.unmap();
                if end > start {
                    total_ms += ((end - start) as f32 * timestamp_period) / 1_000_000.0;
                }
            }
            let full_room_render_ms = total_ms / perf_iters as f32;
            println!(
                "Neon Acoustic Studio 640x360 full-mesh GPU render: {full_room_render_ms:.3} ms (budget < 0.700 ms)"
            );
            assert!(
                full_room_render_ms < 0.700,
                "Full Neon Acoustic Studio 640x360 render exceeded budget: {full_room_render_ms:.3} ms"
            );
        }

        let measure = |bytes: &[u8]| -> (f64, usize, usize) {
            let mut luma_sum = 0.0f64;
            let mut dark = 0usize;
            let mut bright_white = 0usize;
            for &[r, g, b, _] in bytes.as_chunks::<4>().0 {
                let luma = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0;
                luma_sum += luma;
                if luma < 0.25 {
                    dark += 1;
                }
                if r > 235 && g > 235 && b > 235 {
                    bright_white += 1;
                }
            }
            (luma_sum / (r_w * r_h) as f64, dark, bright_white)
        };

        let (avg_luma_on, dark_pixels_on, bright_white_pixels_on) = measure(&rgba_on);
        let (avg_luma_off, dark_pixels_off, bright_white_pixels_off) = measure(&rgba_off);
        let luma_delta = avg_luma_on - avg_luma_off;

        let mut abs_diff_sum = 0u64;
        for (a, b) in rgba_on
            .as_chunks::<4>()
            .0
            .iter()
            .zip(rgba_off.as_chunks::<4>().0)
        {
            abs_diff_sum += a[0].abs_diff(b[0]) as u64
                + a[1].abs_diff(b[1]) as u64
                + a[2].abs_diff(b[2]) as u64;
        }
        let mean_abs_channel_diff = abs_diff_sum as f64 / ((r_w * r_h * 3) as f64);

        println!(
            "Neon Acoustic Studio render: on(luma={avg_luma_on:.4}, dark={dark_pixels_on}, white={bright_white_pixels_on}), off(luma={avg_luma_off:.4}, dark={dark_pixels_off}, white={bright_white_pixels_off}), luma_delta={luma_delta:.4}, mean_abs_channel_diff={mean_abs_channel_diff:.3}"
        );

        assert!(
            0.006 < luma_delta && luma_delta < 0.055 && mean_abs_channel_diff > 1.2,
            "Smoke blasts must be subtly visible by default without washing out the studio (luma_delta={luma_delta:.4}, mean_abs_channel_diff={mean_abs_channel_diff:.3})"
        );
        assert!(
            avg_luma_on < avg_luma_off + 0.055 && avg_luma_on < 0.315 && dark_pixels_on > 120_000,
            "Heavy bass smoke blasts must preserve baseline studio dark contrast (avg_luma_on={avg_luma_on:.4}, avg_luma_off={avg_luma_off:.4}, dark_pixels_on={dark_pixels_on})"
        );
        assert!(
            bright_white_pixels_on <= bright_white_pixels_off + 30,
            "Smoke blasts must not cause white clipping beyond clean room highlights (on={bright_white_pixels_on}, off={bright_white_pixels_off})"
        );

        // Render and verify additional choreographed angles (Shot 1: Sub+Center Stack Hero, Shot 2: Control Room EQ Window Reveal, Shot 3: High Catwalk Panorama)
        for &(shot_play_time, out_path, label, max_luma, min_dark) in &[
            (
                12.5 * 2.88,
                "target/vis_neon_acoustic_studio_stack_hero.png",
                "Shot 1 Sub+Center Stack Hero",
                0.44_f64,
                65_000_usize,
            ),
            (
                21.0 * 2.88,
                "target/vis_neon_acoustic_studio_eq_window.png",
                "Shot 2 EQ Window",
                0.36_f64,
                90_000_usize,
            ),
            (
                30.0 * 2.88,
                "target/vis_neon_acoustic_studio_catwalk.png",
                "Shot 3 High Catwalk",
                0.36_f64,
                90_000_usize,
            ),
        ] {
            let (shot_eye, shot_target) =
                engine::compute_neon_acoustic_studio_camera(shot_play_time);
            let shot_view = glam::Mat4::look_at_rh(shot_eye, shot_target, glam::Vec3::Y);
            let shot_cam = engine::CameraUniforms {
                view_matrix: shot_view.to_cols_array_2d(),
                proj_matrix: cam_proj.to_cols_array_2d(),
            };
            queue.write_buffer(&camera_buf, 0, bytemuck::cast_slice(&[shot_cam]));
            let bytes = render_with_smoke(0.0, out_path);
            let (avg_luma, dark_pixels, bright_white) = measure(&bytes);
            println!(
                "Neon Acoustic Studio {label}: luma={avg_luma:.4}, dark={dark_pixels}, white={bright_white}"
            );
            assert!(
                0.05 < avg_luma
                    && avg_luma < max_luma
                    && dark_pixels > min_dark
                    && bright_white < 12_000,
                "{label} must stay cleanly framed and dark-contrasted (avg_luma={avg_luma:.4}, max_luma={max_luma:.2}, dark={dark_pixels}, min_dark={min_dark}, white={bright_white})"
            );
        }
    });
}

#[test]
fn test_gpu_bioluminescent_waves_render_and_compute() {
    pollster::block_on(async {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            backend_options: BackendOptions::default(),
            display: None,
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
        });
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("Failed to find an appropriate adapter");

        let timestamp_supported = adapter.features().contains(Features::TIMESTAMP_QUERY);
        let mut required_features = Features::empty();
        if timestamp_supported {
            required_features |= Features::TIMESTAMP_QUERY;
        }
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("Biolum Test Device"),
                required_features,
                ..Default::default()
            })
            .await
            .expect("Failed to create device");

        let error_caught = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let err_flag = error_caught.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |err| {
            eprintln!("WGPU Validation/Device Error in Biolum test: {:?}", err);
            err_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));

        let common = include_str!("../src/shaders/_common.wgsl");
        let biolum_compute_src = include_str!("../src/shaders/biolum_compute.wgsl")
            .replace("// INCLUDE: common", common);
        let biolum_render_src = include_str!("../src/shaders/vis_bioluminescence.wgsl")
            .replace("// INCLUDE: common", common);

        let uniform_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Biolum Test Uniforms"),
            size: std::mem::size_of::<engine::AudioUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Biolum Test Camera"),
            size: std::mem::size_of::<engine::CameraUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let particles_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Biolum Test Particles"),
            size: (engine::BIOLUM_PARTICLE_COUNT as u64) * 32,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (ocean_verts, ocean_inds) = engine::generate_biolum_ocean_mesh();
        let ocean_vb = device.create_buffer(&BufferDescriptor {
            label: Some("Biolum Test Ocean VB"),
            size: (ocean_verts.len() * std::mem::size_of::<engine::Vertex>()) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ocean_vb, 0, bytemuck::cast_slice(&ocean_verts));
        let ocean_ib = device.create_buffer(&BufferDescriptor {
            label: Some("Biolum Test Ocean IB"),
            size: (ocean_inds.len() * std::mem::size_of::<u32>()) as u64,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ocean_ib, 0, bytemuck::cast_slice(&ocean_inds));
        let ocean_index_count = ocean_inds.len() as u32;

        let quad_verts = [
            engine::Vertex {
                position: [-0.5, -0.5, 0.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 0.0],
            },
            engine::Vertex {
                position: [0.5, -0.5, 0.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 0.0],
            },
            engine::Vertex {
                position: [0.5, 0.5, 0.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [1.0, 1.0],
            },
            engine::Vertex {
                position: [-0.5, 0.5, 0.0],
                normal: [0.0, 0.0, 1.0],
                tex_coords: [0.0, 1.0],
            },
        ];
        let quad_inds: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let quad_vb = device.create_buffer(&BufferDescriptor {
            label: Some("Biolum Test Quad VB"),
            size: std::mem::size_of_val(&quad_verts) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&quad_vb, 0, bytemuck::cast_slice(&quad_verts));
        let quad_ib = device.create_buffer(&BufferDescriptor {
            label: Some("Biolum Test Quad IB"),
            size: std::mem::size_of_val(&quad_inds) as u64,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&quad_ib, 0, bytemuck::cast_slice(&quad_inds));

        let compute_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Biolum Test Compute BGL"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let compute_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Biolum Test Compute BG"),
            layout: &compute_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: particles_buf.as_entire_binding(),
                },
            ],
        });
        let compute_mod = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Biolum Test Compute Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&biolum_compute_src)),
        });
        let compute_pl = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Biolum Test Compute PL"),
            bind_group_layouts: &[Some(&compute_bgl)],
            immediate_size: 0,
        });
        let compute_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Biolum Test Compute Pipeline"),
            layout: Some(&compute_pl),
            module: &compute_mod,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let audio_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Biolum Test Audio BGL"),
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
        let cam_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Biolum Test Camera BGL"),
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
        let part_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Biolum Test Particles BGL"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let audio_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Biolum Test Audio BG"),
            layout: &audio_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let cam_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Biolum Test Camera BG"),
            layout: &cam_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });
        let part_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Biolum Test Particles BG"),
            layout: &part_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: particles_buf.as_entire_binding(),
            }],
        });

        let layout_3d = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Biolum Test 3D PL"),
            bind_group_layouts: &[Some(&audio_bgl), Some(&cam_bgl)],
            immediate_size: 0,
        });
        let layout_particles = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Biolum Test Particles PL"),
            bind_group_layouts: &[Some(&audio_bgl), Some(&cam_bgl), Some(&part_bgl)],
            immediate_size: 0,
        });

        let render_mod = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Biolum Test Render Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&biolum_render_src)),
        });
        let color_format = TextureFormat::Rgba8UnormSrgb;
        let v_desc = [Some(engine::Vertex::desc())];

        let bg_pipe = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Biolum Test BG Pipeline"),
            layout: Some(&layout_3d),
            vertex: VertexState {
                module: &render_mod,
                entry_point: Some("vs_bg"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &render_mod,
                entry_point: Some("fs_bg"),
                targets: &[Some(ColorTargetState {
                    format: color_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Always),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let ocean_pipe = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Biolum Test Ocean Pipeline"),
            layout: Some(&layout_3d),
            vertex: VertexState {
                module: &render_mod,
                entry_point: Some("vs_ocean"),
                buffers: &v_desc,
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &render_mod,
                entry_point: Some("fs_ocean"),
                targets: &[Some(ColorTargetState {
                    format: color_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
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

        let particle_pipe = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Biolum Test Particle Pipeline"),
            layout: Some(&layout_particles),
            vertex: VertexState {
                module: &render_mod,
                entry_point: Some("vs_main_3d"),
                buffers: &v_desc,
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &render_mod,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: color_format,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Add,
                        },
                        alpha: BlendComponent::OVER,
                    }),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::LessEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Drive 20 compute simulation ticks with realistic multi-band musical energy
        let mut uniforms: engine::AudioUniforms = bytemuck::Zeroable::zeroed();
        uniforms.mode = 20;
        uniforms.num_channels = 2;
        uniforms.num_spatial_channels = 2;
        uniforms.aspect_ratio = 16.0 / 9.0;
        uniforms.frame_dt = 0.0166;
        uniforms.channels[0] = 0.85;
        uniforms.channels[1] = 0.72;
        uniforms.spatial_channels[0] = 0.85;
        uniforms.spatial_channels[1] = 0.72;
        uniforms.spectrum[0..12].fill(68.0);
        uniforms.spectrum[32..88].fill(42.0);
        uniforms.spectrum[160..320].fill(36.0);

        let mut play_time = 6.5_f64;
        for step in 0..20 {
            play_time += 0.0166 * 2.88;
            uniforms.time = (play_time / 2.88) as f32;
            uniforms.smooth_time = play_time as f32;
            uniforms.frame_count = step;
            queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[uniforms]));

            let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut cp = enc.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("Biolum Test Compute"),
                    timestamp_writes: None,
                });
                cp.set_pipeline(&compute_pipe);
                cp.set_bind_group(0, Some(&compute_bg), &[]);
                cp.dispatch_workgroups(engine::BIOLUM_WORKGROUPS, 1, 1);
            }
            queue.submit(Some(enc.finish()));
        }
        let _ = device.poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        });

        let snapshot_dir = std::env::var("RT_SAVE_BIOLUM_SNAPSHOT_DIR").ok();
        let cases = [
            (1280u32, 720u32, false, "biolum_perspective_16x9.png"),
            (1400u32, 400u32, false, "biolum_perspective_ultrawide.png"),
            (1400u32, 400u32, true, "biolum_topdown_ultrawide.png"),
        ];

        for (w, h, top_down, filename) in cases {
            let aspect = w as f32 / h as f32;
            let fov_y = 48.0_f32.to_radians();
            let proj = glam::Mat4::perspective_rh_gl(fov_y, aspect, 0.1, 1000.0);
            let (eye, target, up) = engine::compute_biolum_camera(play_time, top_down);
            let view = glam::Mat4::look_at_rh(eye, target, up);

            let cam_u = engine::CameraUniforms {
                view_matrix: view.to_cols_array_2d(),
                proj_matrix: proj.to_cols_array_2d(),
            };
            queue.write_buffer(&camera_buf, 0, bytemuck::cast_slice(&[cam_u]));

            let mut u = uniforms;
            u.aspect_ratio = aspect;
            queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[u]));

            let color_tex = device.create_texture(&TextureDescriptor {
                label: Some("Biolum Test Color"),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: color_format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let color_view = color_tex.create_view(&TextureViewDescriptor::default());
            let depth_tex = device.create_texture(&TextureDescriptor {
                label: Some("Biolum Test Depth"),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                usage: TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth_view = depth_tex.create_view(&TextureViewDescriptor::default());

            let padded_row_bytes = (w * 4).div_ceil(256) * 256;
            let readback_buf = device.create_buffer(&BufferDescriptor {
                label: Some("Biolum Test Readback"),
                size: (padded_row_bytes * h) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            // Sentinel magenta background clear proves fs_bg + fs_ocean cover 100% of every pixel without border gaps
            let sentinel_clear = Color {
                r: 1.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            };

            let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                    label: Some("Biolum Test Render"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(sentinel_clear),
                            store: StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(1.0),
                            store: StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&bg_pipe);
                rp.set_bind_group(0, &audio_bg, &[]);
                rp.set_bind_group(1, &cam_bg, &[]);
                rp.draw(0..3, 0..1);

                rp.set_pipeline(&ocean_pipe);
                rp.set_bind_group(0, &audio_bg, &[]);
                rp.set_bind_group(1, &cam_bg, &[]);
                rp.set_vertex_buffer(0, ocean_vb.slice(..));
                rp.set_index_buffer(ocean_ib.slice(..), IndexFormat::Uint32);
                rp.draw_indexed(0..ocean_index_count, 0, 0..1);

                rp.set_pipeline(&particle_pipe);
                rp.set_bind_group(0, &audio_bg, &[]);
                rp.set_bind_group(1, &cam_bg, &[]);
                rp.set_bind_group(2, &part_bg, &[]);
                rp.set_vertex_buffer(0, quad_vb.slice(..));
                rp.set_index_buffer(quad_ib.slice(..), IndexFormat::Uint32);
                rp.draw_indexed(0..6, 0, 0..engine::BIOLUM_PARTICLE_COUNT);
            }
            enc.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &color_tex,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &readback_buf,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_row_bytes),
                        rows_per_image: Some(h),
                    },
                },
                Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(enc.finish()));

            let slice = readback_buf.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(MapMode::Read, move |r| tx.send(r).unwrap());
            let _ = device.poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.recv().unwrap().unwrap();
            let mapped = slice.get_mapped_range().unwrap();
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row_start = y * padded_row_bytes as usize;
                rgba.extend_from_slice(&mapped[row_start..row_start + (w * 4) as usize]);
            }
            drop(mapped);
            readback_buf.unmap();

            if let Some(dir) = &snapshot_dir {
                let out_path = std::path::Path::new(dir).join(filename);
                image::save_buffer(&out_path, &rgba, w, h, image::ColorType::Rgba8)
                    .expect("Failed to save biolum snapshot");
            }

            let total_px = (w * h) as usize;
            let mut dark_px = 0usize;
            let mut cyan_glow_px = 0usize;
            let mut white_clip_px = 0usize;
            let mut border_grey_leaks = 0usize;
            let mut luma_sum = 0.0f64;

            for y in 0..h as usize {
                for x in 0..w as usize {
                    let i = (y * w as usize + x) * 4;
                    let [r, g, b, _] = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
                    let luma = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0;
                    luma_sum += luma;
                    if luma < 0.22 {
                        dark_px += 1;
                    }
                    if b > r.saturating_add(25) && g >= 95 && b >= 130 {
                        cyan_glow_px += 1;
                    }
                    if r > 240 && g > 240 && b > 240 {
                        white_clip_px += 1;
                    }
                    let is_edge = x < 4 || x >= w as usize - 4 || y < 4 || y >= h as usize - 4;
                    if is_edge && r > 180 && g < 50 && b > 180 {
                        border_grey_leaks += 1;
                    }
                }
            }

            let avg_luma = luma_sum / total_px as f64;
            println!(
                "Biolum {filename} ({w}x{h}, top_down={top_down}): avg_luma={avg_luma:.4}, dark={dark_px}/{total_px}, cyan_glow={cyan_glow_px}, white_clip={white_clip_px}, border_leaks={border_grey_leaks}"
            );

            assert_eq!(
                border_grey_leaks, 0,
                "{filename} must fill 100% edge-to-edge with zero grey border leaks"
            );
            assert!(
                dark_px > total_px / 4,
                "{filename} must preserve deep dark midnight troughs/sky ({dark_px} / {total_px})"
            );
            assert!(
                cyan_glow_px > 500,
                "{filename} must display vibrant bioluminescent cyan waves/particles (got {cyan_glow_px})"
            );
            assert!(
                white_clip_px < total_px / 20,
                "{filename} must not blow out into flat white ({white_clip_px} / {total_px})"
            );
        }

        // Benchmark 3-pass render time at 1280x720 when timestamp queries are supported
        if timestamp_supported {
            let w = 1280u32;
            let h = 720u32;
            let aspect = w as f32 / h as f32;
            let proj = glam::Mat4::perspective_rh_gl(48.0_f32.to_radians(), aspect, 0.1, 1000.0);
            let (eye, target, up) = engine::compute_biolum_camera(play_time, false);
            let view = glam::Mat4::look_at_rh(eye, target, up);
            let cam_u = engine::CameraUniforms {
                view_matrix: view.to_cols_array_2d(),
                proj_matrix: proj.to_cols_array_2d(),
            };
            queue.write_buffer(&camera_buf, 0, bytemuck::cast_slice(&[cam_u]));

            let color_tex = device.create_texture(&TextureDescriptor {
                label: Some("Biolum Perf Color"),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: color_format,
                usage: TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let color_view = color_tex.create_view(&TextureViewDescriptor::default());
            let depth_tex = device.create_texture(&TextureDescriptor {
                label: Some("Biolum Perf Depth"),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                usage: TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth_view = depth_tex.create_view(&TextureViewDescriptor::default());

            let qs = device.create_query_set(&QuerySetDescriptor {
                label: Some("Biolum Perf QS"),
                ty: QueryType::Timestamp,
                count: 2,
            });
            let res_buf = device.create_buffer(&BufferDescriptor {
                label: Some("Biolum Perf Res"),
                size: 16,
                usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let ts_read_buf = device.create_buffer(&BufferDescriptor {
                label: Some("Biolum Perf Read"),
                size: 16,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let period = queue.get_timestamp_period();
            let iters = 30u32;
            let mut sum_ms = 0.0f32;
            for _ in 0..iters {
                let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
                {
                    let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                        label: Some("Biolum Perf Pass"),
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
                                store: StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: Some(RenderPassTimestampWrites {
                            query_set: &qs,
                            beginning_of_pass_write_index: Some(0),
                            end_of_pass_write_index: Some(1),
                        }),
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    rp.set_pipeline(&bg_pipe);
                    rp.set_bind_group(0, &audio_bg, &[]);
                    rp.set_bind_group(1, &cam_bg, &[]);
                    rp.draw(0..3, 0..1);

                    rp.set_pipeline(&ocean_pipe);
                    rp.set_bind_group(0, &audio_bg, &[]);
                    rp.set_bind_group(1, &cam_bg, &[]);
                    rp.set_vertex_buffer(0, ocean_vb.slice(..));
                    rp.set_index_buffer(ocean_ib.slice(..), IndexFormat::Uint32);
                    rp.draw_indexed(0..ocean_index_count, 0, 0..1);

                    rp.set_pipeline(&particle_pipe);
                    rp.set_bind_group(0, &audio_bg, &[]);
                    rp.set_bind_group(1, &cam_bg, &[]);
                    rp.set_bind_group(2, &part_bg, &[]);
                    rp.set_vertex_buffer(0, quad_vb.slice(..));
                    rp.set_index_buffer(quad_ib.slice(..), IndexFormat::Uint32);
                    rp.draw_indexed(0..6, 0, 0..engine::BIOLUM_PARTICLE_COUNT);
                }
                enc.resolve_query_set(&qs, 0..2, &res_buf, 0);
                enc.copy_buffer_to_buffer(&res_buf, 0, &ts_read_buf, 0, 16);
                queue.submit(Some(enc.finish()));
                let _ = device.poll(PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });
                let slice = ts_read_buf.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(MapMode::Read, move |r| tx.send(r).unwrap());
                let _ = device.poll(PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });
                rx.recv().unwrap().unwrap();
                let data = slice.get_mapped_range().unwrap();
                let s = u64::from_le_bytes(data[0..8].try_into().unwrap());
                let e = u64::from_le_bytes(data[8..16].try_into().unwrap());
                drop(data);
                ts_read_buf.unmap();
                if e > s {
                    sum_ms += ((e - s) as f32 * period) / 1_000_000.0;
                }
            }
            let avg_ms = sum_ms / iters as f32;
            println!(
                "Bioluminescent Waves 1280x720 3-pass GPU render: {avg_ms:.3} ms (budget < 0.700 ms)"
            );
            assert!(
                avg_ms < 0.700,
                "Bioluminescent Waves 3-pass render exceeded 0.7ms budget: {avg_ms:.3} ms"
            );
        }

        assert!(
            !error_caught.load(std::sync::atomic::Ordering::SeqCst),
            "WGPU validation error was triggered during Bioluminescent Waves test!"
        );
    });
}

#[test]
fn test_gpu_ferrofluid_render_and_perf() {
    pollster::block_on(async {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            backend_options: BackendOptions::default(),
            display: None,
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
        });

        let adapter = match instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
        {
            Ok(a) => a,
            Err(_) => {
                println!("Skipping ferrofluid GPU test: No GPU adapter available.");
                return;
            }
        };

        let timestamp_supported = adapter.features().contains(Features::TIMESTAMP_QUERY);
        let req_features = if timestamp_supported {
            Features::TIMESTAMP_QUERY
        } else {
            Features::empty()
        };

        let (device, queue) = match adapter
            .request_device(&DeviceDescriptor {
                label: Some("Ferrofluid Test Device"),
                required_features: req_features,
                required_limits: Limits::default(),
                memory_hints: MemoryHints::default(),
                ..Default::default()
            })
            .await
        {
            Ok(dq) => dq,
            Err(e) => {
                println!("Skipping ferrofluid GPU test: Failed to request device: {e:?}");
                return;
            }
        };

        let error_caught = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let err_flag = error_caught.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |err| {
            eprintln!("WGPU uncaptured error in ferrofluid test: {err:?}");
            err_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));

        const SHADER_COMMON: &str = include_str!("../src/shaders/_common.wgsl");
        let ferro_src = include_str!("../src/shaders/vis_flip_ferrofluid.wgsl")
            .replace("// INCLUDE: common", SHADER_COMMON);

        let uniform_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Ferrofluid Test Uniforms"),
            size: std::mem::size_of::<engine::AudioUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let audio_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Ferrofluid Test Audio BGL"),
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

        let audio_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Ferrofluid Test Audio BG"),
            layout: &audio_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Ferrofluid Test PL"),
            bind_group_layouts: &[Some(&audio_bgl)],
            immediate_size: 0,
        });

        let shader_mod = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Ferrofluid Test Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&ferro_src)),
        });

        let color_format = TextureFormat::Rgba8UnormSrgb;
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Ferrofluid Test Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader_mod,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader_mod,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: color_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let make_uniforms =
            |w: u32, h: u32, pool_mode: bool, loud: bool| -> engine::AudioUniforms {
                let mut u: engine::AudioUniforms = bytemuck::Zeroable::zeroed();
                u.mode = 27;
                u.num_channels = 2;
                u.num_spatial_channels = 2;
                u.time = 6.2;
                u.smooth_time = 6.2;
                u.duration = 200.0;
                u.frame_dt = 0.0166;
                u.aspect_ratio = w as f32 / h as f32;
                u._pad3 = if pool_mode { 1.0 } else { 0.0 };
                if loud {
                    u.channels[0] = 0.88;
                    u.channels[1] = 0.72;
                    u.spatial_channels[0] = 0.88;
                    u.spatial_channels[1] = 0.72;
                    for (offset, dst) in u.spectrum[56..=260].iter_mut().enumerate() {
                        let i = 56 + offset;
                        *dst = 72.0 - (i as f32 - 140.0).abs() * 0.15;
                    }
                    for (offset, dst) in u.spectrum[261..=680].iter_mut().enumerate() {
                        let i = 261 + offset;
                        *dst = 48.0 + 18.0 * ((i as f32) * 0.07).sin().abs();
                    }
                    for (offset, dst) in u.spectrum[681..=990].iter_mut().enumerate() {
                        let i = 681 + offset;
                        *dst = 36.0 + 16.0 * ((i as f32) * 0.11).cos().abs();
                    }
                }
                u
            };

        let snapshot_dir = std::env::var("RT_SAVE_FERROFLUID_SNAPSHOT_DIR").ok();
        if let Some(dir) = &snapshot_dir {
            std::fs::create_dir_all(dir).expect("Failed to create snapshot output dir");
        }

        let cases = [
            (1280u32, 720u32, false, true, "core_16x9.png"),
            (1400u32, 400u32, false, true, "core_ultrawide.png"),
            (540u32, 960u32, false, true, "core_portrait.png"),
            (1280u32, 720u32, true, true, "pool_16x9.png"),
            (1280u32, 720u32, false, false, "core_quiet_16x9.png"),
        ];

        let mut core_loud_rgba: Option<Vec<u8>> = None;
        let mut core_quiet_rgba: Option<Vec<u8>> = None;

        for (w, h, pool_mode, loud, filename) in cases {
            let u = make_uniforms(w, h, pool_mode, loud);
            queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[u]));

            let color_tex = device.create_texture(&TextureDescriptor {
                label: Some("Ferrofluid Test Color"),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: color_format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let color_view = color_tex.create_view(&TextureViewDescriptor::default());

            let padded_row_bytes = (w * 4).div_ceil(256) * 256;
            let readback_buf = device.create_buffer(&BufferDescriptor {
                label: Some("Ferrofluid Test Readback"),
                size: (padded_row_bytes * h) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            // Sentinel magenta proves 100% viewport coverage without border leaks
            let sentinel_clear = Color {
                r: 1.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            };

            let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                    label: Some("Ferrofluid Test Pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(sentinel_clear),
                            store: StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&pipeline);
                rp.set_bind_group(0, &audio_bg, &[]);
                rp.draw(0..3, 0..1);
            }
            enc.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &color_tex,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &readback_buf,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_row_bytes),
                        rows_per_image: Some(h),
                    },
                },
                Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(enc.finish()));

            let slice = readback_buf.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(MapMode::Read, move |r| tx.send(r).unwrap());
            let _ = device.poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.recv().unwrap().unwrap();
            let mapped = slice.get_mapped_range().unwrap();
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row_start = y * padded_row_bytes as usize;
                rgba.extend_from_slice(&mapped[row_start..row_start + (w * 4) as usize]);
            }
            drop(mapped);
            readback_buf.unmap();

            if let Some(dir) = &snapshot_dir {
                let out_path = std::path::Path::new(dir).join(filename);
                image::save_buffer(&out_path, &rgba, w, h, image::ColorType::Rgba8)
                    .expect("Failed to save ferrofluid snapshot");
            }

            let total_px = (w * h) as usize;
            let mut dark_px = 0usize;
            let mut bright_px = 0usize;
            let mut white_clip_px = 0usize;
            let mut border_leaks = 0usize;
            let mut luma_sum = 0.0f64;

            for y in 0..h as usize {
                for x in 0..w as usize {
                    let i = (y * w as usize + x) * 4;
                    let [r, g, b, _] = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
                    let luma = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0;
                    luma_sum += luma;
                    if luma < 0.18 {
                        dark_px += 1;
                    }
                    if r.max(g).max(b) >= 95 {
                        bright_px += 1;
                    }
                    if r > 245 && g > 245 && b > 245 {
                        white_clip_px += 1;
                    }
                    let is_edge = x < 4 || x >= w as usize - 4 || y < 4 || y >= h as usize - 4;
                    if is_edge && r > 180 && g < 50 && b > 180 {
                        border_leaks += 1;
                    }
                }
            }

            let avg_luma = luma_sum / total_px as f64;
            println!(
                "Ferrofluid {filename} ({w}x{h}, pool={pool_mode}, loud={loud}): avg_luma={avg_luma:.4}, dark={dark_px}/{total_px}, bright={bright_px}, white_clip={white_clip_px}, border_leaks={border_leaks}"
            );

            assert_eq!(
                border_leaks, 0,
                "{filename} must fill 100% edge-to-edge with zero border leaks"
            );
            assert!(
                dark_px > total_px * 35 / 100,
                "{filename} must preserve deep obsidian contrast ({dark_px} / {total_px})"
            );
            let min_bright = if loud { 400 } else { 150 };
            assert!(
                bright_px > min_bright,
                "{filename} must display crisp specular/rim highlights (got {bright_px})"
            );
            assert!(
                white_clip_px < total_px / 20,
                "{filename} must not blow out into flat white ({white_clip_px} / {total_px})"
            );

            if filename == "core_16x9.png" {
                core_loud_rgba = Some(rgba);
            } else if filename == "core_quiet_16x9.png" {
                core_quiet_rgba = Some(rgba);
            }
        }

        let loud_px = core_loud_rgba.expect("loud frame captured");
        let quiet_px = core_quiet_rgba.expect("quiet frame captured");
        let mean_diff: f64 = loud_px
            .as_chunks::<4>()
            .0
            .iter()
            .zip(quiet_px.as_chunks::<4>().0.iter())
            .map(|(a, b)| {
                ((a[0] as f64 - b[0] as f64).abs()
                    + (a[1] as f64 - b[1] as f64).abs()
                    + (a[2] as f64 - b[2] as f64).abs())
                    / 3.0
            })
            .sum::<f64>()
            / (loud_px.len() / 4) as f64;
        println!("Ferrofluid audio reactivity mean channel diff (loud vs quiet): {mean_diff:.3}");
        assert!(
            mean_diff > 1.8,
            "Ferrofluid shader must respond strongly to audio energy (mean_diff={mean_diff:.3})"
        );

        if timestamp_supported {
            for (bw, bh, budget_ms, label) in [
                (1280u32, 720u32, Some(0.700_f32), "1280x720"),
                (1920u32, 1080u32, None, "1920x1080"),
            ] {
                let u = make_uniforms(bw, bh, false, true);
                queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[u]));

                let color_tex = device.create_texture(&TextureDescriptor {
                    label: Some("Ferrofluid Perf Color"),
                    size: Extent3d {
                        width: bw,
                        height: bh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: color_format,
                    usage: TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let color_view = color_tex.create_view(&TextureViewDescriptor::default());

                let qs = device.create_query_set(&QuerySetDescriptor {
                    label: Some("Ferrofluid Perf QS"),
                    ty: QueryType::Timestamp,
                    count: 2,
                });
                let res_buf = device.create_buffer(&BufferDescriptor {
                    label: Some("Ferrofluid Perf Res"),
                    size: 16,
                    usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let ts_read_buf = device.create_buffer(&BufferDescriptor {
                    label: Some("Ferrofluid Perf Read"),
                    size: 16,
                    usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let period = queue.get_timestamp_period();

                // Warmup 5 iterations
                for _ in 0..5 {
                    let mut enc =
                        device.create_command_encoder(&CommandEncoderDescriptor::default());
                    {
                        let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                            label: Some("Ferrofluid Warmup"),
                            color_attachments: &[Some(RenderPassColorAttachment {
                                view: &color_view,
                                resolve_target: None,
                                ops: Operations {
                                    load: LoadOp::Clear(Color::BLACK),
                                    store: StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        rp.set_pipeline(&pipeline);
                        rp.set_bind_group(0, &audio_bg, &[]);
                        rp.draw(0..3, 0..1);
                    }
                    queue.submit(Some(enc.finish()));
                }
                let _ = device.poll(PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });

                let iters = 30u32;
                let mut sum_ms = 0.0f32;
                for _ in 0..iters {
                    let mut enc =
                        device.create_command_encoder(&CommandEncoderDescriptor::default());
                    {
                        let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                            label: Some("Ferrofluid Perf Pass"),
                            color_attachments: &[Some(RenderPassColorAttachment {
                                view: &color_view,
                                resolve_target: None,
                                ops: Operations {
                                    load: LoadOp::Clear(Color::BLACK),
                                    store: StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: Some(RenderPassTimestampWrites {
                                query_set: &qs,
                                beginning_of_pass_write_index: Some(0),
                                end_of_pass_write_index: Some(1),
                            }),
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        rp.set_pipeline(&pipeline);
                        rp.set_bind_group(0, &audio_bg, &[]);
                        rp.draw(0..3, 0..1);
                    }
                    enc.resolve_query_set(&qs, 0..2, &res_buf, 0);
                    enc.copy_buffer_to_buffer(&res_buf, 0, &ts_read_buf, 0, 16);
                    queue.submit(Some(enc.finish()));
                    let _ = device.poll(PollType::Wait {
                        submission_index: None,
                        timeout: None,
                    });
                    let slice = ts_read_buf.slice(..);
                    let (tx, rx) = std::sync::mpsc::channel();
                    slice.map_async(MapMode::Read, move |r| tx.send(r).unwrap());
                    let _ = device.poll(PollType::Wait {
                        submission_index: None,
                        timeout: None,
                    });
                    rx.recv().unwrap().unwrap();
                    let data = slice.get_mapped_range().unwrap();
                    let s = u64::from_le_bytes(data[0..8].try_into().unwrap());
                    let e = u64::from_le_bytes(data[8..16].try_into().unwrap());
                    drop(data);
                    ts_read_buf.unmap();
                    if e > s {
                        sum_ms += ((e - s) as f32 * period) / 1_000_000.0;
                    }
                }
                let avg_ms = sum_ms / iters as f32;
                println!("Ferrofluid {label} GPU render: {avg_ms:.3} ms");
                if let Some(limit) = budget_ms {
                    assert!(
                        avg_ms < limit,
                        "Ferrofluid {label} render exceeded {limit:.3}ms budget: {avg_ms:.3} ms"
                    );
                }
            }
        }

        assert!(
            !error_caught.load(std::sync::atomic::Ordering::SeqCst),
            "WGPU validation error was triggered during Ferrofluid test!"
        );
    });
}

#[test]
fn test_flip_disc_compute_and_render_verification() {
    pollster::block_on(async {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::PRIMARY,
            flags: InstanceFlags::default(),
            backend_options: BackendOptions::default(),
            display: None,
            memory_budget_thresholds: MemoryBudgetThresholds::default(),
        });

        let adapter = match instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
        {
            Ok(a) => a,
            Err(_) => {
                println!("Skipping flip-disc GPU test: No GPU adapter available.");
                return;
            }
        };

        let timestamp_supported = adapter.features().contains(Features::TIMESTAMP_QUERY);
        let req_features = if timestamp_supported {
            Features::TIMESTAMP_QUERY
        } else {
            Features::empty()
        };

        let (device, queue) = match adapter
            .request_device(&DeviceDescriptor {
                label: Some("Flip-Disc Test Device"),
                required_features: req_features,
                required_limits: Limits::default(),
                memory_hints: MemoryHints::default(),
                ..Default::default()
            })
            .await
        {
            Ok(dq) => dq,
            Err(e) => {
                println!("Skipping flip-disc GPU test: Failed to request device: {e:?}");
                return;
            }
        };

        let error_caught = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let err_flag = error_caught.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |err| {
            eprintln!("WGPU uncaptured error in flip-disc test: {err:?}");
            err_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));

        const SHADER_COMMON: &str = include_str!("../src/shaders/_common.wgsl");
        let compute_src = include_str!("../src/shaders/flip_disc_compute.wgsl")
            .replace("// INCLUDE: common", SHADER_COMMON);
        let render_src = include_str!("../src/shaders/vis_flip_disc.wgsl")
            .replace("// INCLUDE: common", SHADER_COMMON);

        let uniform_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Flip-Disc Test Audio Uniforms"),
            size: std::mem::size_of::<engine::AudioUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let buf_bytes = (engine::FLIP_DISC_COUNT as u64) * 16;
        let params_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Flip-Disc Test Params"),
            size: std::mem::size_of::<engine::FlipDiscParams>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wave_a_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Flip-Disc Test Wave A"),
            size: buf_bytes,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wave_b_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Flip-Disc Test Wave B"),
            size: buf_bytes,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let discs_buf = device.create_buffer(&BufferDescriptor {
            label: Some("Flip-Disc Test State"),
            size: buf_bytes,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let compute_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Flip-Disc Test Compute BGL"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::COMPUTE,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let compute_bg_a = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Flip-Disc Test Compute BG A"),
            layout: &compute_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: wave_a_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: wave_b_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: discs_buf.as_entire_binding(),
                },
            ],
        });
        let compute_bg_b = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Flip-Disc Test Compute BG B"),
            layout: &compute_bgl,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: wave_b_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: wave_a_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: discs_buf.as_entire_binding(),
                },
            ],
        });

        let compute_pl = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Flip-Disc Test Compute PL"),
            bind_group_layouts: &[Some(&compute_bgl)],
            immediate_size: 0,
        });
        let compute_mod = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Flip-Disc Test Compute Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&compute_src)),
        });
        let init_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Flip-Disc Test Init"),
            layout: Some(&compute_pl),
            module: &compute_mod,
            entry_point: Some("cs_init"),
            compilation_options: Default::default(),
            cache: None,
        });
        let wave_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Flip-Disc Test Wave"),
            layout: Some(&compute_pl),
            module: &compute_mod,
            entry_point: Some("cs_wave_step"),
            compilation_options: Default::default(),
            cache: None,
        });
        let step_pipe = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("Flip-Disc Test Step"),
            layout: Some(&compute_pl),
            module: &compute_mod,
            entry_point: Some("cs_disc_step"),
            compilation_options: Default::default(),
            cache: None,
        });

        let audio_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Flip-Disc Test Audio BGL"),
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
        let audio_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Flip-Disc Test Audio BG"),
            layout: &audio_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let discs_render_bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Flip-Disc Test Render BGL"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let discs_render_bg = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Flip-Disc Test Render BG"),
            layout: &discs_render_bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: discs_buf.as_entire_binding(),
            }],
        });

        let render_pl = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Flip-Disc Test Render PL"),
            bind_group_layouts: &[Some(&audio_bgl), Some(&discs_render_bgl)],
            immediate_size: 0,
        });
        let render_mod = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Flip-Disc Test Render Shader"),
            source: ShaderSource::Wgsl(std::borrow::Cow::Borrowed(&render_src)),
        });
        let color_format = TextureFormat::Rgba8UnormSrgb;
        let render_pipe = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Flip-Disc Test Render Pipeline"),
            layout: Some(&render_pl),
            vertex: VertexState {
                module: &render_mod,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &render_mod,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: color_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let snapshot_dir = std::env::var("RT_SAVE_FLIP_DISC_SNAPSHOT_DIR").ok();
        if let Some(dir) = &snapshot_dir {
            std::fs::create_dir_all(dir).expect("Failed to create flip-disc snapshot dir");
        }

        let cases = [
            (1280u32, 720u32, true, "flip_disc_16x9.png"),
            (1400u32, 400u32, true, "flip_disc_ultrawide.png"),
            (540u32, 960u32, true, "flip_disc_portrait.png"),
            (1280u32, 720u32, false, "flip_disc_quiet_16x9.png"),
        ];

        let mut loud_16x9_rgba: Option<Vec<u8>> = None;
        let mut quiet_16x9_rgba: Option<Vec<u8>> = None;

        for (w, h, loud, filename) in cases {
            let aspect = w as f32 / h as f32;
            let mut sim = engine::FlipDiscSimState::default();
            let mut ping = true;
            let mut initialized = false;
            let steps = if loud { 36 } else { 48 };

            for frame in 0..steps {
                let t = (frame as f32) * (1.0 / 60.0);
                let mut spec = [0.0f32; 1024];
                let mut vus = [0.0f32, 0.0f32];
                if loud {
                    let kick_on = frame % 16 == 0 || frame % 16 == 1;
                    let snare_on = frame % 12 == 4;
                    let hat_on = frame % 5 == 0;
                    if kick_on {
                        spec[1..16].fill(82.0);
                    } else {
                        spec[1..16].fill(22.0);
                    }
                    if snare_on {
                        spec[30..110].fill(72.0);
                    } else {
                        spec[30..110].fill(24.0);
                    }
                    if hat_on {
                        spec[200..420].fill(58.0);
                    } else {
                        spec[200..420].fill(16.0);
                    }
                    let pan = (frame as f32 * 0.35).sin() * 0.38;
                    vus[0] = (0.65 - pan).clamp(0.1, 1.0);
                    vus[1] = (0.65 + pan).clamp(0.1, 1.0);
                }

                let (fd_params, substeps) =
                    sim.step(&spec, &vus, 1.0 / 60.0, aspect, true, t, initialized);
                queue.write_buffer(&params_buf, 0, bytemuck::cast_slice(&[fd_params]));

                let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
                {
                    let mut cp = enc.begin_compute_pass(&ComputePassDescriptor::default());
                    if !initialized {
                        cp.set_pipeline(&init_pipe);
                        cp.set_bind_group(0, Some(&compute_bg_a), &[]);
                        cp.dispatch_workgroups(
                            engine::FLIP_DISC_WORKGROUPS_X,
                            engine::FLIP_DISC_WORKGROUPS_Y,
                            1,
                        );
                        cp.set_bind_group(0, Some(&compute_bg_b), &[]);
                        cp.dispatch_workgroups(
                            engine::FLIP_DISC_WORKGROUPS_X,
                            engine::FLIP_DISC_WORKGROUPS_Y,
                            1,
                        );
                        initialized = true;
                        ping = true;
                    }
                    for _ in 0..substeps.min(2) {
                        cp.set_pipeline(&wave_pipe);
                        let bg = if ping { &compute_bg_a } else { &compute_bg_b };
                        cp.set_bind_group(0, Some(bg), &[]);
                        cp.dispatch_workgroups(
                            engine::FLIP_DISC_WORKGROUPS_X,
                            engine::FLIP_DISC_WORKGROUPS_Y,
                            1,
                        );
                        ping = !ping;
                    }
                    let disc_bg = if ping { &compute_bg_a } else { &compute_bg_b };
                    cp.set_pipeline(&step_pipe);
                    cp.set_bind_group(0, Some(disc_bg), &[]);
                    cp.dispatch_workgroups(
                        engine::FLIP_DISC_WORKGROUPS_X,
                        engine::FLIP_DISC_WORKGROUPS_Y,
                        1,
                    );
                }
                queue.submit(Some(enc.finish()));
            }

            let mut u: engine::AudioUniforms = bytemuck::Zeroable::zeroed();
            u.mode = 32;
            u.num_channels = 2;
            u.num_spatial_channels = 2;
            u.time = 3.2;
            u.smooth_time = 3.2;
            u.frame_dt = 1.0 / 60.0;
            u.aspect_ratio = aspect;
            queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[u]));

            let color_tex = device.create_texture(&TextureDescriptor {
                label: Some("Flip-Disc Test Color"),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: color_format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let color_view = color_tex.create_view(&TextureViewDescriptor::default());

            let padded_row_bytes = (w * 4).div_ceil(256) * 256;
            let readback_buf = device.create_buffer(&BufferDescriptor {
                label: Some("Flip-Disc Test Readback"),
                size: (padded_row_bytes * h) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let sentinel_clear = Color {
                r: 1.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            };

            let mut enc = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                    label: Some("Flip-Disc Test Render"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(sentinel_clear),
                            store: StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&render_pipe);
                rp.set_bind_group(0, &audio_bg, &[]);
                rp.set_bind_group(1, &discs_render_bg, &[]);
                rp.draw(0..3, 0..1);
            }
            enc.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &color_tex,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &readback_buf,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_row_bytes),
                        rows_per_image: Some(h),
                    },
                },
                Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(enc.finish()));

            let slice = readback_buf.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(MapMode::Read, move |r| tx.send(r).unwrap());
            let _ = device.poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.recv().unwrap().unwrap();
            let mapped = slice.get_mapped_range().unwrap();
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row_start = y * padded_row_bytes as usize;
                rgba.extend_from_slice(&mapped[row_start..row_start + (w * 4) as usize]);
            }
            drop(mapped);
            readback_buf.unmap();

            if let Some(dir) = &snapshot_dir {
                let out_path = std::path::Path::new(dir).join(filename);
                image::save_buffer(&out_path, &rgba, w, h, image::ColorType::Rgba8)
                    .expect("Failed to save flip-disc snapshot");
            }

            let total_px = (w * h) as usize;
            let mut dark_px = 0usize;
            let mut bright_px = 0usize;
            let mut border_leaks = 0usize;
            let mut chroma_outliers = 0usize;
            let mut luma_sum = 0.0f64;

            for y in 0..h as usize {
                for x in 0..w as usize {
                    let i = (y * w as usize + x) * 4;
                    let [r, g, b, _] = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
                    let luma = (0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64) / 255.0;
                    luma_sum += luma;
                    if luma < 0.28 {
                        dark_px += 1;
                    }
                    if r.max(g).max(b) >= 160 {
                        bright_px += 1;
                    }
                    let spread = r.max(g).max(b) - r.min(g).min(b);
                    if spread > 32 {
                        chroma_outliers += 1;
                    }
                    let is_edge = x < 4 || x >= w as usize - 4 || y < 4 || y >= h as usize - 4;
                    if is_edge && r > 180 && g < 50 && b > 180 {
                        border_leaks += 1;
                    }
                }
            }

            let avg_luma = luma_sum / total_px as f64;
            println!(
                "Flip-Disc {filename} ({w}x{h}, loud={loud}): avg_luma={avg_luma:.4}, dark={dark_px}/{total_px}, bright={bright_px}, chroma_outliers={chroma_outliers}, border_leaks={border_leaks}"
            );

            assert_eq!(
                border_leaks, 0,
                "{filename} must fill 100% edge-to-edge with zero border leaks"
            );
            assert_eq!(
                chroma_outliers, 0,
                "{filename} must preserve charcoal-and-white monochrome neutrality"
            );
            assert!(
                dark_px > total_px * 35 / 100,
                "{filename} must preserve dark charcoal discs & pocket contrast ({dark_px}/{total_px})"
            );
            if loud {
                assert!(
                    bright_px > total_px / 30,
                    "{filename} must show bright white flipped ripple discs (got {bright_px}/{total_px})"
                );
            } else {
                assert!(
                    bright_px < total_px / 100,
                    "{filename} quiet wall should rest on charcoal discs (got bright={bright_px}/{total_px})"
                );
            }

            if filename == "flip_disc_16x9.png" {
                loud_16x9_rgba = Some(rgba);
            } else if filename == "flip_disc_quiet_16x9.png" {
                quiet_16x9_rgba = Some(rgba);
            }
        }

        let loud_px = loud_16x9_rgba.expect("loud frame captured");
        let quiet_px = quiet_16x9_rgba.expect("quiet frame captured");
        let mean_diff: f64 = loud_px
            .as_chunks::<4>()
            .0
            .iter()
            .zip(quiet_px.as_chunks::<4>().0.iter())
            .map(|(a, b)| {
                ((a[0] as f64 - b[0] as f64).abs()
                    + (a[1] as f64 - b[1] as f64).abs()
                    + (a[2] as f64 - b[2] as f64).abs())
                    / 3.0
            })
            .sum::<f64>()
            / (loud_px.len() / 4) as f64;
        println!("Flip-Disc audio reactivity mean channel diff (loud vs quiet): {mean_diff:.3}");
        assert!(
            mean_diff > 5.0,
            "Flip-Disc shader must respond strongly to audio ripples (mean_diff={mean_diff:.3})"
        );

        if timestamp_supported {
            for (bw, bh, budget_ms, label) in [
                (1280u32, 720u32, Some(0.700_f32), "1280x720"),
                (1920u32, 1080u32, None, "1920x1080"),
            ] {
                let mut u: engine::AudioUniforms = bytemuck::Zeroable::zeroed();
                u.mode = 32;
                u.time = 3.2;
                u.smooth_time = 3.2;
                u.aspect_ratio = bw as f32 / bh as f32;
                queue.write_buffer(&uniform_buf, 0, bytemuck::cast_slice(&[u]));

                let color_tex = device.create_texture(&TextureDescriptor {
                    label: Some("Flip-Disc Perf Color"),
                    size: Extent3d {
                        width: bw,
                        height: bh,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: color_format,
                    usage: TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let color_view = color_tex.create_view(&TextureViewDescriptor::default());

                let qs = device.create_query_set(&QuerySetDescriptor {
                    label: Some("Flip-Disc Perf QS"),
                    ty: QueryType::Timestamp,
                    count: 2,
                });
                let res_buf = device.create_buffer(&BufferDescriptor {
                    label: Some("Flip-Disc Perf Res"),
                    size: 16,
                    usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let ts_read_buf = device.create_buffer(&BufferDescriptor {
                    label: Some("Flip-Disc Perf Read"),
                    size: 16,
                    usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let period = queue.get_timestamp_period();

                for _ in 0..5 {
                    let mut enc =
                        device.create_command_encoder(&CommandEncoderDescriptor::default());
                    {
                        let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                            label: Some("Flip-Disc Warmup"),
                            color_attachments: &[Some(RenderPassColorAttachment {
                                view: &color_view,
                                resolve_target: None,
                                ops: Operations {
                                    load: LoadOp::Clear(Color::BLACK),
                                    store: StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        rp.set_pipeline(&render_pipe);
                        rp.set_bind_group(0, &audio_bg, &[]);
                        rp.set_bind_group(1, &discs_render_bg, &[]);
                        rp.draw(0..3, 0..1);
                    }
                    queue.submit(Some(enc.finish()));
                }
                let _ = device.poll(PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });

                let iters = 30u32;
                let mut sum_ms = 0.0f32;
                for _ in 0..iters {
                    let mut enc =
                        device.create_command_encoder(&CommandEncoderDescriptor::default());
                    {
                        let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
                            label: Some("Flip-Disc Perf Pass"),
                            color_attachments: &[Some(RenderPassColorAttachment {
                                view: &color_view,
                                resolve_target: None,
                                ops: Operations {
                                    load: LoadOp::Clear(Color::BLACK),
                                    store: StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: Some(RenderPassTimestampWrites {
                                query_set: &qs,
                                beginning_of_pass_write_index: Some(0),
                                end_of_pass_write_index: Some(1),
                            }),
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        rp.set_pipeline(&render_pipe);
                        rp.set_bind_group(0, &audio_bg, &[]);
                        rp.set_bind_group(1, &discs_render_bg, &[]);
                        rp.draw(0..3, 0..1);
                    }
                    enc.resolve_query_set(&qs, 0..2, &res_buf, 0);
                    enc.copy_buffer_to_buffer(&res_buf, 0, &ts_read_buf, 0, 16);
                    queue.submit(Some(enc.finish()));
                    let _ = device.poll(PollType::Wait {
                        submission_index: None,
                        timeout: None,
                    });
                    let slice = ts_read_buf.slice(..);
                    let (tx, rx) = std::sync::mpsc::channel();
                    slice.map_async(MapMode::Read, move |r| tx.send(r).unwrap());
                    let _ = device.poll(PollType::Wait {
                        submission_index: None,
                        timeout: None,
                    });
                    rx.recv().unwrap().unwrap();
                    let data = slice.get_mapped_range().unwrap();
                    let s = u64::from_le_bytes(data[0..8].try_into().unwrap());
                    let e = u64::from_le_bytes(data[8..16].try_into().unwrap());
                    drop(data);
                    ts_read_buf.unmap();
                    if e > s {
                        sum_ms += ((e - s) as f32 * period) / 1_000_000.0;
                    }
                }
                let avg_ms = sum_ms / iters as f32;
                println!("Flip-Disc {label} GPU render: {avg_ms:.3} ms");
                if let Some(limit) = budget_ms {
                    assert!(
                        avg_ms < limit,
                        "Flip-Disc {label} render exceeded {limit:.3}ms budget: {avg_ms:.3} ms"
                    );
                }
            }
        }

        assert!(
            !error_caught.load(std::sync::atomic::Ordering::SeqCst),
            "WGPU validation error was triggered during Flip-Disc test!"
        );
    });
}
