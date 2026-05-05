use std::sync::Arc;
use winit::window::Window;
use crate::state::AppState;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EngineAction {
    None,
    OpenFile,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AudioUniforms {
    pub spectrum: [f32; 512],
    pub waveform: [[f32; 512]; 4],
    pub fire_heat: [f32; 512],
    pub channels: [f32; 32],
    pub num_channels: u32,
    pub mode: u32,
    pub time: f32,
    pub _padding: u32,
}

pub struct VulkanEngine<'a> {
    surface: wgpu::Surface<'a>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    render_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    pub egui_renderer: egui_wgpu::Renderer,
    pub heatmap_texture: Option<egui::TextureHandle>,
    pub fire_buffer: Vec<u8>,
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

        let (device, queue) = adapter.request_device(
            &wgpu::DeviceDescriptor {
                required_features: wgpu::Features::empty(),
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

        // Lock to VSYNC strictly
        let present_mode = wgpu::PresentMode::Fifo;

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

        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/spectrum.wgsl"));

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Audio Uniform Buffer"),
            size: std::mem::size_of::<AudioUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
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
                }
            ],
            label: Some("audio_bind_group"),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
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
            multiview: None,
            cache: None,
        });

        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, None, 1, false);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            render_pipeline,
            uniform_buffer,
            uniform_bind_group,
            egui_renderer,
            heatmap_texture: None,
            fire_buffer: vec![0; 80 * 20],
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
            spectrum: [0.0; 512],
            waveform: [[0.0; 512]; 4],
            fire_heat: [0.0; 512],
            channels: [0.0; 32],
            num_channels: state.num_channels as u32,
            mode: state.visualizer_mode,
            time: state.current_seconds as f32,
            _padding: 0,
        };

        uniforms.spectrum.copy_from_slice(&state.spectrum_data);
        for (i, wave) in state.waveform_history.iter().enumerate().take(4) {
            uniforms.waveform[i].copy_from_slice(wave);
        }
        uniforms.fire_heat.copy_from_slice(&state.fire_heat);
        
        let ch_len = state.channel_vus.len().min(32);
        uniforms.channels[..ch_len].copy_from_slice(&state.channel_vus[..ch_len]);

        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    pub fn render(
        &mut self,
        window: &winit::window::Window,
        egui_ctx: &egui::Context,
        egui_state: &mut egui_winit::State,
        state: &AppState,
    ) -> Result<(EngineAction, f32, f32), wgpu::SurfaceError> {
        let ui_start = std::time::Instant::now();
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Process egui UI
        let raw_input = egui_state.take_egui_input(window);
        let mut central_rect = egui::Rect::from_min_max(Default::default(), egui::pos2(self.config.width as f32, self.config.height as f32));
        let mut engine_action = EngineAction::None;
        
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
                                egui::RichText::new("  OPEN TRACKER MODULE  ")
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
                            let painter = columns[0].painter();
                            let num_channels = state.channel_vus.len();
                            if num_channels > 0 {
                                let w = channel_rect.width() / num_channels as f32;
                                let max_h = channel_rect.height() - 20.0;
                                for i in 0..num_channels {
                                    let x = channel_rect.left() + i as f32 * w + w * 0.2;
                                    let bw = w * 0.6;
                                    let y_bottom = channel_rect.bottom() - 15.0;
                                    
                                    // Draw fast VU
                                    let vu = state.channel_vus[i].clamp(0.0, 1.0);
                                    let vh = vu * max_h;
                                    let color = if vu > 0.8 {
                                        egui::Color32::from_rgb(255, 255, 255)
                                    } else if vu > 0.5 {
                                        egui::Color32::from_rgb(255, 200, 50)
                                    } else {
                                        egui::Color32::from_rgb(200, 50, 50)
                                    };
                                    painter.rect_filled(
                                        egui::Rect::from_min_max(
                                            egui::pos2(x, y_bottom - vh),
                                            egui::pos2(x + bw, y_bottom)
                                        ),
                                        0.0,
                                        color
                                    );

                                    // Draw peak
                                    let peak = state.peak_vus[i].clamp(0.0, 1.0);
                                    let ph = peak * max_h;
                                    painter.rect_filled(
                                        egui::Rect::from_min_max(
                                            egui::pos2(x, y_bottom - ph - 4.0),
                                            egui::pos2(x + bw, y_bottom - ph)
                                        ),
                                        0.0,
                                        egui::Color32::WHITE
                                    );
                                    
                                    // Label
                                    if num_channels <= 16 {
                                        painter.text(
                                            egui::pos2(x + bw * 0.5, y_bottom + 2.0),
                                            egui::Align2::CENTER_TOP,
                                            format!("{}", i + 1),
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
                            let (rect, _) = columns[0].allocate_exact_size(egui::vec2(columns[0].available_width(), 16.0), egui::Sense::hover());
                            let painter = columns[0].painter();
                            
                            // Unplayed background (Grey)
                            painter.rect_filled(rect, 2.0, egui::Color32::from_gray(50));
                            
                            // Played section (Charred/Burned look)
                            let played_width = rect.width() * progress;
                            let played_rect = egui::Rect::from_min_size(rect.min, egui::vec2(played_width, rect.height()));
                            painter.rect_filled(played_rect, 2.0, egui::Color32::from_rgb(15, 15, 15)); // Black charred
                            
                            // Glowing embers at base of played section
                            let base_y = rect.max.y - 2.0;
                            let ember_count = (played_width / 4.0) as usize;
                            for i in 0..ember_count {
                                let ember_x = rect.min.x + (i * 4) as f32;
                                let static_seed = ((ember_x as f64 * 12.34).sin() * 43758.5453).fract().abs();
                                if static_seed > 0.85 {
                                    let phase = ember_x as f64 * 0.5;
                                    let pulse = ((state.current_seconds as f64 * 2.0 + phase).sin() * 0.5 + 0.5) as f32;
                                    let max_brightness = ((static_seed - 0.85) / 0.15 * 255.0) as f32;
                                    let brightness = (max_brightness * (0.3 + 0.7 * pulse)) as u8;
                                    
                                    painter.rect_filled(
                                        egui::Rect::from_min_size(egui::pos2(ember_x, base_y - 2.0), egui::vec2(2.0, 2.0)),
                                        0.0,
                                        egui::Color32::from_rgba_unmultiplied(255, brightness / 4, 0, brightness)
                                    );
                                }
                            }
                            
                            // Fire indicator (Demoscene Fire Array)
                            if progress > 0.0 && progress < 1.0 {
                                let fire_x = rect.min.x + played_width;
                                let fire_w = 40;
                                let fire_h = 10;
                                let pixel_size = 2.0;
                                let t = state.current_seconds * 1000.0;
                                
                                // Randomize bottom row, hot on the right, tapering left
                                for x in 0..fire_w {
                                    let intensity = (x as f64 / fire_w as f64).powf(2.0);
                                    let seed = (x as f64 * 31.415 + t as f64).sin() * 43758.5453;
                                    let mut r = (seed.fract().abs() * 255.0) * intensity;
                                    if x >= fire_w - 2 { r = 255.0; } // Playhead core
                                    self.fire_buffer[(fire_h - 1) * fire_w + x] = r as u8;
                                }
                                
                                // Propagate up and drift left (wind)
                                for y in 0..(fire_h - 1) {
                                    for x in 0..fire_w {
                                        let src_idx = (y + 1) * fire_w + x;
                                        let mut sum = self.fire_buffer[src_idx] as u32 * 2; // directly below
                                        
                                        if x > 0 { sum += self.fire_buffer[src_idx - 1] as u32; }
                                        if x < fire_w - 1 { sum += self.fire_buffer[src_idx + 1] as u32 * 2; } // bottom-right pulls left
                                        sum += self.fire_buffer[(y + 2).min(fire_h - 1) * fire_w + x] as u32; // two below
                                        
                                        let avg = sum / 6;
                                        let seed2 = (y as f64 * 12.34 + x as f64 * 45.67 + t as f64).sin() * 43758.5453;
                                        let cool = (seed2.fract().abs() * 5.0) as u32; // Random cooling 0-4
                                        
                                        self.fire_buffer[y * fire_w + x] = avg.saturating_sub(cool) as u8;
                                    }
                                }

                                let start_x = fire_x - (fire_w as f32 * pixel_size) + pixel_size; // Anchor right side to playhead
                                let start_y = rect.max.y - 1.0 - (fire_h as f32 * pixel_size);
                                
                                for y in 0..fire_h {
                                    for x in 0..fire_w {
                                        let mut temp = self.fire_buffer[y * fire_w + x];
                                        
                                        // Soft fade at the top to prevent hard edges and blend into smoke
                                        if y < 4 {
                                            temp = (temp as f32 * (y as f32 / 4.0)).max(0.0) as u8;
                                        }
                                        
                                        let (r, g, b, a) = if temp > 230 {
                                            (255, 255, 255, 255) // White hot
                                        } else if temp > 150 {
                                            let n = (temp - 150) as f32 / 80.0;
                                            (255, (n * 255.0) as u8, (n * 100.0) as u8, 255) // Orange/Yellow
                                        } else if temp > 80 {
                                            let n = (temp - 80) as f32 / 70.0;
                                            (255, (n * 100.0) as u8, 0, 240) // Bright Red to Deep Orange
                                        } else if temp > 30 {
                                            let n = (temp - 30) as f32 / 50.0;
                                            ((100.0 + n * 155.0) as u8, 0, 0, 200) // Deep Red
                                        } else if temp > 10 {
                                            let n = (temp - 10) as f32 / 20.0;
                                            let grey = 30 + (n * 40.0) as u8;
                                            (grey, grey, grey, (n * 150.0) as u8) // Grey Smoke
                                        } else {
                                            (0, 0, 0, 0)
                                        };
                                        
                                        if a > 0 {
                                            let px_x = start_x + x as f32 * pixel_size;
                                            let px_y = start_y + y as f32 * pixel_size;
                                            
                                            // Clip fire to the start of the progress bar
                                            if px_x >= rect.min.x {
                                                painter.rect_filled(
                                                    egui::Rect::from_min_size(
                                                        egui::pos2(px_x, px_y),
                                                        egui::vec2(pixel_size, pixel_size)
                                                    ),
                                                    0.0,
                                                    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
                                                );
                                            }
                                        }
                                    }
                                }
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
                            let painter = columns[1].painter().with_clip_rect(hm_rect);
                            let history_len = state.spectrum_history.len();
                            if history_len > 0 && state.spectrum_history[0].len() > 0 {
                                let raw_bands = state.spectrum_history[0].len();
                                let chunks = 64; // Downsample 512 bands to 64 for visual clarity & performance
                                let chunk_size = raw_bands / chunks;
                                
                                let cell_w = hm_rect.width() / chunks as f32; // Horizontal frequency scale
                                
                                let center_y = hm_rect.top() + hm_rect.height() / 2.0;
                                
                                let mut image = egui::ColorImage::new([chunks, history_len], egui::Color32::from_rgb(20, 20, 22));
                                
                                for (time_idx, bands) in state.spectrum_history.iter().enumerate() {
                                    // time_idx 119 (newest) at bottom. time_idx 0 (oldest) at top.
                                    let y = history_len - 1 - time_idx;
                                    
                                    for x in 0..chunks {
                                        let mut max_val = 0.0;
                                        for k in 0..chunk_size {
                                            let idx = x * chunk_size + k;
                                            if idx < bands.len() && bands[idx] > max_val {
                                                max_val = bands[idx];
                                            }
                                        }
                                        
                                        if max_val > 5.0 {
                                            let color = if max_val > 60.0 {
                                                egui::Color32::from_rgb(180, 180, 180)
                                            } else if max_val > 30.0 {
                                                egui::Color32::from_rgb(255, 140, 0)
                                            } else {
                                                egui::Color32::from_rgb(180, 20, 20)
                                            };
                                            image[(x, y)] = color;
                                        }
                                    }
                                }
                                
                                if self.heatmap_texture.is_none() {
                                    self.heatmap_texture = Some(ctx.load_texture(
                                        "heatmap",
                                        image,
                                        egui::TextureOptions::NEAREST
                                    ));
                                } else {
                                    self.heatmap_texture.as_mut().unwrap().set(image, egui::TextureOptions::NEAREST);
                                }
                                
                                painter.image(
                                    self.heatmap_texture.as_ref().unwrap().id(),
                                    hm_rect,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE
                                );
                                
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
                            columns[2].horizontal(|ui| { ui.label("Title"); ui.label(&state.song_title); });
                            columns[2].horizontal(|ui| { ui.label("Artist"); ui.label(&state.artist); });
                            columns[2].horizontal(|ui| { ui.label("Type"); ui.label(&state.module_type); });
                            if state.bpm > 0 { columns[2].horizontal(|ui| { ui.label("BPM"); ui.label(format!("{}", state.bpm)); }); }
                            if state.speed > 0 { columns[2].horizontal(|ui| { ui.label("Speed"); ui.label(format!("{}", state.speed)); }); }
                            if state.num_patterns > 0 { columns[2].horizontal(|ui| { ui.label("Patterns"); ui.label(format!("{}", state.num_patterns)); }); }
                            if state.num_instruments > 0 { columns[2].horizontal(|ui| { ui.label("Instruments"); ui.label(format!("{}", state.num_instruments)); }); }
                            if state.num_samples > 0 { columns[2].horizontal(|ui| { ui.label("Samples"); ui.label(format!("{}", state.num_samples)); }); }
                            columns[2].horizontal(|ui| { ui.label("Channels"); ui.label(format!("{}", state.num_channels)); });
                            columns[2].horizontal(|ui| { ui.label("Length"); ui.label(format!("{:.1}s", state.duration_seconds)); });
                        });
                    });
            }

            // Central Panel (Transparent background) for Frequency Labels
            let frame = egui::Frame::NONE.fill(egui::Color32::TRANSPARENT);
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
                if state.visualizer_mode != 1 {
                    // Draw frequency labels at the bottom
                    let rect = ui.available_rect_before_wrap();
                    central_rect = rect;
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
                timestamp_writes: None,
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

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.draw(0..3, 0..1);
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

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        
        let render_elapsed = render_start.elapsed().as_micros() as f32;

        Ok((engine_action, ui_elapsed, render_elapsed))
    }
}
