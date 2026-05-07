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
    pub num_channels: u32,
    pub mode: u32,
    pub time: f32,
    pub duration: f32,
    pub smooth_time: f32,
    pub _pad: [u32; 3],
    pub ui_meters_rect: [f32; 4],
    pub ui_heatmap_rect: [f32; 4],
    pub ui_fire_rect: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaveformHistoryStorage {
    pub waveforms: [[f32; 1024]; 60],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VisualizerStorage {
    pub history: [[f32; 64]; 120],
    pub fire_grid: [[f32; 1024]; 144],
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
    visualizer_storage_buffer: wgpu::Buffer,
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
    fire_grid: Box<[[f32; 1024]; 144]>,
    last_fire_time: f32,
    
    pub start_time: std::time::Instant,
}

impl<'a> VulkanEngine<'a> {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        // The instance is a handle to our GPU
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN, // Request Vulkan explicitly!
            ..Default::default()
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

        let (device, queue) = adapter.request_device(
            &wgpu::DeviceDescriptor {
                required_features,
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
                label: None,
            },
            None,
        ).await.unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Let WGPU pick the best non-vsync method to ensure frame pacing doesn't tear/stutter under Wayland
        let present_mode = wgpu::PresentMode::AutoNoVsync;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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

        let waveform_storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Waveform History Storage Buffer"),
            size: std::mem::size_of::<WaveformHistoryStorage>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let visualizer_storage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Heatmap History Storage Buffer"),
            size: std::mem::size_of::<VisualizerStorage>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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
                    resource: visualizer_storage_buffer.as_entire_binding(),
                }
            ],
            label: Some("audio_bind_group"),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader_modules = vec![
            device.create_shader_module(wgpu::include_wgsl!("shaders/vis_spectrum.wgsl")),
            device.create_shader_module(wgpu::include_wgsl!("shaders/vis_flame.wgsl")),
            device.create_shader_module(wgpu::include_wgsl!("shaders/vis_oscilloscope.wgsl")),
            device.create_shader_module(wgpu::include_wgsl!("shaders/vis_spatial.wgsl")),
            device.create_shader_module(wgpu::include_wgsl!("shaders/vis_ferrofluid.wgsl")),
            device.create_shader_module(wgpu::include_wgsl!("shaders/vis_neon.wgsl")),
        ];

        let mut render_pipelines = Vec::new();
        for (i, shader) in shader_modules.iter().enumerate() {
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("Render Pipeline {}", i)),
                layout: Some(&render_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
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
                multiview: None,
                cache: None,
            });
            render_pipelines.push(pipeline);
        }

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
            multiview: None,
            cache: None,
        });

        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, None, 1, false);

        let mut query_set = None;
        let mut query_resolve_buffer = None;
        let mut query_read_buffer = None;
        let timestamp_period = queue.get_timestamp_period();

        if supports_timestamps {
            query_set = Some(device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Shader Timestamps"),
                count: 2,
                ty: wgpu::QueryType::Timestamp,
            }));

            query_resolve_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Query Resolve Buffer"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));

            query_read_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Query Read Buffer"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
        }

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
            visualizer_storage_buffer,
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
            fire_grid: Box::new([[0.0; 1024]; 144]),
            last_fire_time: 0.0,
            start_time: std::time::Instant::now(),
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

    pub fn update(&mut self, state: &AppState) {
        let mut uniforms = AudioUniforms {
            spectrum: [0.0; 1024],
            fire_heat: [0.0; 1024],
            channels: [0.0; 32],
            channel_peaks: [0.0; 32],
            num_channels: state.num_channels as u32,
            mode: state.visualizer_mode,
            time: state.current_seconds as f32,
            duration: state.duration_seconds as f32,
            smooth_time: self.start_time.elapsed().as_secs_f32(),
            _pad: [0; 3],
            ui_meters_rect: self.meters_uv_rect,
            ui_heatmap_rect: self.heatmap_uv_rect,
            ui_fire_rect: self.fire_uv_rect,
        };

        uniforms.spectrum.copy_from_slice(&state.spectrum_data);
        uniforms.fire_heat.copy_from_slice(&state.fire_heat);
        
        let mut history_storage = WaveformHistoryStorage {
            waveforms: [[0.0; 1024]; 60],
        };
        for (i, wave) in state.waveform_history.iter().enumerate().take(60) {
            history_storage.waveforms[i].copy_from_slice(wave);
        }
        
        let ch_len = state.channel_vus.len().min(32);
        
        let mut display_order: Vec<usize> = (0..ch_len).collect();
        if ch_len == 6 {
            display_order = vec![4, 0, 2, 3, 1, 5]; // Ls, L, C, LFE, R, Rs
        } else if ch_len == 8 {
            display_order = vec![6, 4, 0, 2, 3, 1, 5, 7]; // SBL, Ls, L, C, LFE, R, Rs, SBR
        }

        for (disp_idx, &src_idx) in display_order.iter().enumerate() {
            if src_idx < state.channel_vus.len() {
                uniforms.channels[disp_idx] = state.channel_vus[src_idx];
                uniforms.channel_peaks[disp_idx] = state.peak_vus[src_idx];
            }
        }

        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
        self.queue.write_buffer(&self.waveform_storage_buffer, 0, bytemuck::cast_slice(&[history_storage]));
        
        let mut visualizer_storage = VisualizerStorage {
            history: [[0.0; 64]; 120],
            fire_grid: *self.fire_grid,
        };
        let chunks = 64;
        let history_len = state.spectrum_history.len().min(120);
        if history_len > 0 {
            for (time_idx, bands) in state.spectrum_history.iter().take(120).enumerate() {
                let current_bands = bands.len();
                for x in 0..chunks {
                    let mut max_val = 0.0;
                    if current_bands >= chunks {
                        let scale_start = (x as f32 / chunks as f32).powf(2.0);
                        let scale_end = ((x + 1) as f32 / chunks as f32).powf(2.0);
                        let start_idx = (scale_start * current_bands as f32) as usize;
                        let end_idx = ((scale_end * current_bands as f32) as usize).max(start_idx + 1);
                        
                        for idx in start_idx..end_idx {
                            if idx < current_bands && bands[idx] > max_val {
                                max_val = bands[idx];
                            }
                        }
                    } else {
                        let chunk_size = (current_bands / chunks).max(1);
                        for k in 0..chunk_size {
                            let idx = x * chunk_size + k;
                            if idx < current_bands && bands[idx] > max_val {
                                max_val = bands[idx];
                            }
                        }
                    }
                    visualizer_storage.history[time_idx][x] = max_val;
                }
            }
        }
        
        // Classic Doom Fire Cellular Automaton
        if state.visualizer_mode == 1 {
            let current_time = self.start_time.elapsed().as_secs_f32();
            if self.last_fire_time == 0.0 {
                self.last_fire_time = current_time;
            }
            
            let mut seed = (current_time * 1000.0) as u32;
            let mut rng = || {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                seed
            };
            
            // Run at a fixed 60Hz timestep to prevent 800+ FPS visual blowout
            while current_time - self.last_fire_time >= 1.0 / 60.0 {
                self.last_fire_time += 1.0 / 60.0;
                
                // 1. Propagate Heat Upwards
                for y in 0usize..143 {
                    for x in 0usize..1024 {
                        // Random spread (-1, 0, +1)
                        let spread = (rng() % 3) as i32 - 1;
                        let src_x = (x as i32 + spread).clamp(0, 1023) as usize;
                        
                        let heat = self.fire_grid[y + 1][src_x];
                        
                        // Base cooling
                        let mut cooling = (rng() % 3) as f32 * 0.015;
                        
                        // Occasionally "bite" a large chunk of heat out to create detached rising sparks
                        if rng() % 100 < 3 {
                            cooling += 0.15;
                        }
                        
                        self.fire_grid[y][x] = (heat - cooling).max(0.0);
                    }
                }
                
                // 2. Inject Fuel at the bottom (y = 143)
                let n_ch = state.channel_vus.len().max(1).min(32);
                let lfe_idx = if n_ch == 6 { 3 } else if n_ch == 8 { 4 } else { 999 };
                
                // We divide the screen width by the number of SPATIAL channels (excluding LFE)
                // This guarantees the Center channel (which is exactly in the middle of the remaining channels)
                // is mapped perfectly to x = 512.
                let n_spatial_ch = if lfe_idx < n_ch { n_ch - 1 } else { n_ch };
                let channel_width = 1024.0 / n_spatial_ch as f32;
                
                let mut base_lfe_fuel = 0.0;
                if lfe_idx < n_ch {
                    let ch_vu = uniforms.channels[lfe_idx];
                    let ch_peak = uniforms.channel_peaks[lfe_idx];
                    // LFE power
                    base_lfe_fuel = (ch_vu + ch_peak * 0.5) * 0.6;
                }
                
                for x in 0usize..1024 {
                    // Apply a tighter normal distribution (Bell Curve) to the LFE bed
                    // Centered at 512, fades sharply towards the edges
                    let lfe_dist = (x as f32 - 512.0).abs();
                    let lfe_sigma = 70.0;
                    let lfe_influence = (- (lfe_dist * lfe_dist) / (2.0 * lfe_sigma * lfe_sigma)).exp();
                    
                    let mut fuel = base_lfe_fuel * lfe_influence;
                    
                    let mut spatial_idx = 0;
                    for i in 0..n_ch {
                        if i == lfe_idx { continue; } // Skip LFE as a spatial pillar
                        
                        let center_x = (spatial_idx as f32 + 0.5) * channel_width;
                        let dist = (x as f32 - center_x).abs();
                        
                        let ch_vu = uniforms.channels[i];
                        let ch_peak = uniforms.channel_peaks[i];
                        
                        // Narrow the Gaussian so channels don't merge into a giant blob
                        let sigma = channel_width * 0.15;
                        let influence = (- (dist * dist) / (2.0 * sigma * sigma)).exp();
                        
                        // Base fuel + sudden spikes from peaks
                        fuel += (ch_vu + ch_peak * 0.5) * influence;
                        
                        spatial_idx += 1;
                    }
                    
                    // Aggressive spatial jitter to break up the smooth Gaussian curve
                    let jitter = (rng() % 100) as f32 / 100.0;
                    fuel *= 0.4 + 0.6 * jitter; // Huge variance
                    
                    self.fire_grid[143][x] = fuel.max(0.0).min(1.0);
                }
            }
            
            visualizer_storage.fire_grid = *self.fire_grid;
        }

        self.queue.write_buffer(&self.visualizer_storage_buffer, 0, bytemuck::cast_slice(&[visualizer_storage]));
    }

    pub fn render(
        &mut self,
        window: &winit::window::Window,
        egui_ctx: &egui::Context,
        egui_state: &mut egui_winit::State,
        state: &AppState,
    ) -> Result<(EngineAction, f32, f32, Option<f32>, f32), wgpu::SurfaceError> {
        let ui_start = std::time::Instant::now();
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut shader_time_us = None;
        if self.query_in_flight {
            if let Some(read_buffer) = &self.query_read_buffer {
                let slice = read_buffer.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
                self.device.poll(wgpu::Maintain::Wait);
                if rx.recv().unwrap().is_ok() {
                    let data = slice.get_mapped_range();
                    let start: u64 = u64::from_le_bytes(data[0..8].try_into().unwrap());
                    let end: u64 = u64::from_le_bytes(data[8..16].try_into().unwrap());
                    drop(data);
                    read_buffer.unmap();
                    self.query_in_flight = false;
                    
                    if end > start {
                        let elapsed_ns = (end - start) as f32 * self.timestamp_period;
                        shader_time_us = Some(elapsed_ns / 1_000.0);
                    }
                }
            }
        }

        // Process egui UI
        let raw_input = egui_state.take_egui_input(window);
        let mut central_rect = egui::Rect::from_min_max(Default::default(), egui::pos2(self.config.width as f32, self.config.height as f32));
        let mut engine_action = EngineAction::None;
        let mut fire_time_us = 0.0;
        
        let mut out_meters_rect = None;
        let mut out_fire_rect = None;
        let mut out_heatmap_rect = None;
        
        let full_output = egui_ctx.run(raw_input, |ctx| {
            if state.show_stats {
                egui::Window::new("Stats")
                    .anchor(egui::Align2::RIGHT_TOP, [-10.0, 10.0])
                    .title_bar(false)
                    .resizable(false)
                    .collapsible(false)
                    .frame(egui::Frame::window(&ctx.style()).fill(egui::Color32::from_black_alpha(200)))
                    .show(ctx, |ui| {
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
                        ui.label(
                            egui::RichText::new(format!("FFT: {:.2} ms | Decode: {:.2} ms", state.stats.fft_us / 1000.0, state.stats.decode_us / 1000.0))
                                .color(egui::Color32::GRAY)
                        );
                        ui.label(
                            egui::RichText::new(format!("Shader: {:.2} ms", state.stats.shader_us / 1000.0))
                                .color(egui::Color32::GRAY)
                        );
                        ui.label(
                            egui::RichText::new("Fire FX: 0.00 ms (GPU Offloaded)")
                                .color(egui::Color32::GRAY)
                        );
                    });
            }

            if !state.file_loaded {
                central_rect = ctx.screen_rect();
                
                let frame = egui::Frame::NONE
                    .fill(egui::Color32::from_rgba_unmultiplied(10, 10, 15, 200)) // dark tint to pop text over the visualizer
                    .inner_margin(40.0);
                    
                egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
                    let avail_height = ui.available_height();
                    let content_height = 450.0;
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
                            .fill(egui::Color32::from_rgb(0, 120, 215))
                            .corner_radius(8.0);
                            
                            if ui.add_sized([350.0, 60.0], btn).clicked() {
                                engine_action = EngineAction::OpenFile;
                            }
                            
                            ui.add_space(60.0);
                            ui.label(egui::RichText::new("Keyboard Shortcuts").color(egui::Color32::LIGHT_GRAY).strong().size(18.0));
                            ui.add_space(15.0);
                            
                            egui::Grid::new("shortcuts_grid")
                                .num_columns(2)
                                .spacing([30.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("O").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Open File").color(egui::Color32::GRAY));
                                    ui.end_row();
                                    
                                    ui.label(egui::RichText::new("Tab").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Toggle HUD").color(egui::Color32::GRAY));
                                    ui.end_row();
                                    
                                    ui.label(egui::RichText::new("F").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Toggle Fullscreen").color(egui::Color32::GRAY));
                                    ui.end_row();
                                    
                                    ui.label(egui::RichText::new("Space").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Play / Pause").color(egui::Color32::GRAY));
                                    ui.end_row();

                                    ui.label(egui::RichText::new("Arrows").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Seek Timeline").color(egui::Color32::GRAY));
                                    ui.end_row();
                                    
                                    ui.label(egui::RichText::new("Q / Esc").color(egui::Color32::WHITE).strong());
                                    ui.label(egui::RichText::new("Quit").color(egui::Color32::GRAY));
                                    ui.end_row();
                                });
                        }
                    );
                });
                return;
            }

            if state.show_hud {
                egui::TopBottomPanel::top("top_panel")
                    .resizable(false)
                    .frame(egui::Frame::NONE.fill(egui::Color32::TRANSPARENT))
                    .exact_height(ctx.screen_rect().height() / 2.0)
                    .show(ctx, |ui| {
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
                                    // The VU meters are now drawn natively in WGPU behind the egui layer.
                                    // We keep the layout space and just draw the labels below.

                                    
                                    // Label
                                    if num_channels <= 16 {
                                        let label = match num_channels {
                                            2 => ["L", "R"].get(i).unwrap_or(&"?").to_string(), // Stereo
                                            3 => ["L", "R", "LFE"].get(i).unwrap_or(&"?").to_string(), // 2.1
                                            4 => ["L", "R", "Ls", "Rs"].get(i).unwrap_or(&"?").to_string(), // Quad
                                            6 => ["Ls", "L", "C", "LFE", "R", "Rs"].get(i).unwrap_or(&"?").to_string(), // 5.1 mapped
                                            8 => ["SBL", "Ls", "L", "C", "LFE", "R", "Rs", "SBR"].get(i).unwrap_or(&"?").to_string(), // 7.1 mapped
                                            12 => ["L", "R", "C", "LFE", "Ls", "Rs", "Lrs", "Rrs", "Ltf", "Rtf", "Ltr", "Rtr"].get(i).unwrap_or(&"?").to_string(), // 7.1.4 Dolby Atmos standard
                                            _ => format!("{}", i + 1),
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
                            let progress = if state.duration_seconds > 0.0 {
                                (state.current_seconds / state.duration_seconds) as f32
                            } else {
                                0.0
                            }.clamp(0.0, 1.0);
                            
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
                            // Backgrounds and embers are now drawn natively by WGPU!
                            
                            // Fire indicator (Demoscene Fire Array) is now drawn by WGPU!
                            if progress > 0.0 && progress < 1.0 {
                                let fire_start_time = std::time::Instant::now();
                                fire_time_us = fire_start_time.elapsed().as_micros() as f32;
                            }
                            
                            // Text Overlay
                            painter.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("{:.1}s / {:.1}s", state.current_seconds, state.duration_seconds),
                                egui::FontId::proportional(11.0),
                                egui::Color32::WHITE,
                            );
                            
                            // Column 1: Heatmap History & Tracker Pattern
                            columns[1].heading("Pattern Heatmap");
                            columns[1].separator();
                            let hm_rect = columns[1].available_rect_before_wrap();
                            out_heatmap_rect = Some(hm_rect);
                            
                            let painter = columns[1].painter().with_clip_rect(hm_rect);
                            // Pattern Heatmap is now drawn natively in WGPU!
                            let history_len = state.spectrum_history.len();
                            if history_len > 0 && state.spectrum_history[0].len() > 0 {
                                let chunks = 64;
                                let cell_w = hm_rect.width() / chunks as f32;
                                
                                // Draw faint background grid over the texture
                                for c in 0..=chunks {
                                    let x = hm_rect.left() + c as f32 * cell_w;
                                    painter.line_segment([egui::pos2(x, hm_rect.top()), egui::pos2(x, hm_rect.bottom())], (1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 5)));
                                }
                                
                                // Draw Tracker Text Overlay
                                if state.tracker_patterns_by_order.len() > 0 && state.current_tracker_order >= 0 && (state.current_tracker_order as usize) < state.tracker_patterns_by_order.len() {
                                    let current_row = state.current_tracker_row as i32;
                                    
                                    let row_height = 14.0;
                                    let num_rows_to_draw = (hm_rect.height() / row_height) as i32;
                                    let center_y = hm_rect.top() + hm_rect.height() / 2.0;
                                    
                                    for offset in (-num_rows_to_draw/2)..(num_rows_to_draw/2) {
                                        let mut resolved_order = state.current_tracker_order as i32;
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
                                                    resolved_row += state.tracker_patterns_by_order[resolved_order as usize].len() as i32;
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
                            
                            // Column 2: Track Info
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
                                columns[2].horizontal(|ui| { ui.label("Video"); ui.label(video); });
                            }
                            if state.bpm > 0 { columns[2].horizontal(|ui| { ui.label("BPM"); ui.label(format!("{}", state.bpm)); }); }
                            if state.speed > 0 { columns[2].horizontal(|ui| { ui.label("Speed"); ui.label(format!("{}", state.speed)); }); }
                            if state.num_patterns > 0 { columns[2].horizontal(|ui| { ui.label("Patterns"); ui.label(format!("{}", state.num_patterns)); }); }
                            if state.num_instruments > 0 { columns[2].horizontal(|ui| { ui.label("Instruments"); ui.label(format!("{}", state.num_instruments)); }); }
                            if state.num_samples > 0 { columns[2].horizontal(|ui| { ui.label("Samples"); ui.label(format!("{}", state.num_samples)); }); }
                            columns[2].horizontal(|ui| { 
                                ui.label("Channels"); 
                                if state.num_channels > state.hardware_channels && state.hardware_channels > 0 {
                                    ui.label(format!("{} (Downmixed to {})", state.num_channels, state.hardware_channels));
                                } else {
                                    ui.label(format!("{}", state.num_channels));
                                }
                            });
                            
                            // Visualizer Mode
                            let vis_name = match state.visualizer_mode {
                                0 => "Frequency Spectrum",
                                1 => "Fire",
                                2 => "CRT Oscilloscope",
                                3 => "Spatial Vectors",
                                4 => "Chrome Ferrofluid",
                                5 => "Neon Corridor",
                                _ => "Unknown",
                            };
                            columns[2].horizontal(|ui| { ui.label("Visualizer"); ui.label(vis_name); });
                            
                            columns[2].horizontal(|ui| { ui.label("Length"); ui.label(format!("{:.1}s", state.duration_seconds)); });
                        });
                    });
            }

            // Central Panel (Transparent background) for Frequency Labels
            let frame = egui::Frame::NONE.fill(egui::Color32::TRANSPARENT);
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                central_rect = rect;
                
                if state.visualizer_mode == 0 {
                    // Draw frequency labels at the bottom
                    let painter = ui.painter();
                    let y = rect.bottom() - 20.0;
                    
                    let max_freq = state.max_frequency;
                    let labels = [
                        (0.0, "0Hz".to_string()),
                        (0.25, format!("{:.1}kHz", max_freq * 0.25 / 1000.0)),
                        (0.50, format!("{:.1}kHz", max_freq * 0.50 / 1000.0)),
                        (0.75, format!("{:.1}kHz", max_freq * 0.75 / 1000.0)),
                        (0.95, format!("{:.1}kHz", max_freq / 1000.0)),
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
        }
        if let Some(r) = out_fire_rect {
            self.fire_uv_rect = [(r.min.x * scale) / w, (r.min.y * scale) / h, (r.max.x * scale) / w, (r.max.y * scale) / h];
        }
        if let Some(r) = out_heatmap_rect {
            self.heatmap_uv_rect = [(r.min.x * scale) / w, (r.min.y * scale) / h, (r.max.x * scale) / w, (r.max.y * scale) / h];
        }

        // Handle egui output
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

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.06,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: if let Some(qs) = &self.query_set {
                    Some(wgpu::RenderPassTimestampWrites {
                        query_set: qs,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    })
                } else { None },
                occlusion_query_set: None,
            });

            // Set viewport to the CentralPanel rect
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
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            }).forget_lifetime();
            self.egui_renderer.render(&mut render_pass, &clipped_primitives, &screen_descriptor);
        }

        if let (Some(qs), Some(res_buf), Some(read_buf)) = (&self.query_set, &self.query_resolve_buffer, &self.query_read_buffer) {
            encoder.resolve_query_set(qs, 0..2, res_buf, 0);
            encoder.copy_buffer_to_buffer(res_buf, 0, read_buf, 0, 16);
            self.query_in_flight = true;
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        
        let render_elapsed = render_start.elapsed().as_micros() as f32;

        Ok((engine_action, ui_elapsed, render_elapsed, shader_time_us, 0.0))
    }
}
