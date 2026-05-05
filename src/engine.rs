use std::sync::Arc;
use winit::window::Window;
use crate::state::AppState;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AudioUniforms {
    pub spectrum: [f32; 512],
    pub channels: [f32; 32],
    pub num_channels: u32,
    pub _padding: [f32; 3],
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

        // Use Mailbox or Immediate for 240Hz un-capped rendering if available
        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if surface_caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        };

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
            channels: [0.0; 32],
            num_channels: state.num_channels as u32,
            _padding: [0.0; 3],
        };

        uniforms.spectrum.copy_from_slice(&state.spectrum_data);
        
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
    ) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Process egui UI
        let raw_input = egui_state.take_egui_input(window);
        let mut central_rect = egui::Rect::NOTHING;
        let full_output = egui_ctx.run(raw_input, |ctx| {
            if state.show_hud {
                egui::TopBottomPanel::top("top_panel")
                    .resizable(false)
                    .exact_height(ctx.screen_rect().height() / 2.0)
                    .show(ctx, |ui| {
                        ui.columns(3, |columns| {
                            // Column 0: Channels
                            columns[0].heading("Channels");
                            columns[0].separator();
                            let channel_rect = columns[0].available_rect_before_wrap();
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
                                    painter.text(
                                        egui::pos2(x + bw * 0.5, y_bottom + 2.0),
                                        egui::Align2::CENTER_TOP,
                                        format!("{}", i + 1),
                                        egui::FontId::proportional(12.0),
                                        egui::Color32::GRAY,
                                    );
                                }
                            }
                            
                            // Column 1: Heatmap History
                            columns[1].heading("Heatmap History");
                            columns[1].separator();
                            let hm_rect = columns[1].available_rect_before_wrap();
                            let painter = columns[1].painter();
                            let history_len = state.spectrum_history.len();
                            if history_len > 0 && state.spectrum_history[0].len() > 0 {
                                let cell_w = hm_rect.width() / history_len as f32;
                                let raw_bands = state.spectrum_history[0].len();
                                let chunks = 64; // Downsample 512 bands to 64 for visual clarity & performance
                                let chunk_size = raw_bands / chunks;
                                let cell_h = hm_rect.height() / chunks as f32;
                                
                                for (time_idx, bands) in state.spectrum_history.iter().enumerate() {
                                    let x = hm_rect.left() + time_idx as f32 * cell_w;
                                    for c in 0..chunks {
                                        let mut max_val = 0.0;
                                        for k in 0..chunk_size {
                                            let idx = c * chunk_size + k;
                                            if idx < bands.len() && bands[idx] > max_val {
                                                max_val = bands[idx];
                                            }
                                        }
                                        
                                        if max_val > 5.0 {
                                            let y = hm_rect.bottom() - c as f32 * cell_h;
                                            let color = if max_val > 60.0 {
                                                egui::Color32::from_rgb(255, 255, 255)
                                            } else if max_val > 30.0 {
                                                egui::Color32::from_rgb(255, 140, 0)
                                            } else {
                                                egui::Color32::from_rgb(180, 20, 20)
                                            };
                                            painter.rect_filled(
                                                egui::Rect::from_min_max(
                                                    egui::pos2(x, y - cell_h),
                                                    egui::pos2(x + cell_w + 1.0, y + 1.0)
                                                ),
                                                0.0,
                                                color
                                            );
                                        }
                                    }
                                }
                            }
                            
                            // Column 2: Track Info
                            columns[2].heading("Track Info");
                            columns[2].separator();
                            columns[2].horizontal(|ui| { ui.label("Title"); ui.label(&state.song_title); });
                            columns[2].horizontal(|ui| { ui.label("Artist"); ui.label(&state.artist); });
                            columns[2].horizontal(|ui| { ui.label("Type"); ui.label(&state.module_type); });
                            columns[2].horizontal(|ui| { ui.label("BPM"); ui.label(format!("{}", state.bpm)); });
                            columns[2].horizontal(|ui| { ui.label("Speed"); ui.label(format!("{}", state.speed)); });
                            columns[2].horizontal(|ui| { ui.label("Channels"); ui.label(format!("{}", state.num_channels)); });
                            columns[2].horizontal(|ui| { ui.label("Length"); ui.label(format!("{:.1}s", state.duration_seconds)); });
                        });
                    });
            }

            // Central Panel (Transparent background) for Frequency Labels
            let frame = egui::Frame::NONE.fill(egui::Color32::TRANSPARENT);
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
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
            let vp_w = (central_rect.width() * scale_factor).clamp(1.0, self.config.width as f32 - vp_x);
            let vp_h = (central_rect.height() * scale_factor).clamp(1.0, self.config.height as f32 - vp_y);
            
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

        Ok(())
    }
}
