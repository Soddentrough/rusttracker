#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use std::{error::Error, io, sync::{Arc, Mutex}, time::{Duration, Instant}};
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::{Backend, CrosstermBackend}, Terminal};
use winit::{
    event::{Event, WindowEvent, ElementState},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{PhysicalKey, KeyCode as WinitKeyCode},
};
#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowAttributesExtWayland;
#[cfg(target_os = "linux")]
use winit::platform::x11::WindowAttributesExtX11;

mod audio;
mod state;
mod ui;
mod engine;

use crate::state::AppState;
use crate::engine::{VulkanEngine, EngineAction};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    file: Vec<String>,

    #[arg(long, default_value_t = false)]
    tui: bool,

    #[arg(long, default_value_t = false)]
    mic: bool,

    #[arg(long, default_value_t = false)]
    fullscreen: bool,

    #[arg(long)]
    vis: Option<String>,

    #[arg(long, default_value_t = false)]
    gpu_fft: bool,
}

struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    fn new() -> Result<Self, Box<dyn Error>> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg = match info.payload().downcast_ref::<&'static str>() {
            Some(s) => *s,
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => &s[..],
                None => "Box<dyn Any>",
            },
        };
        let location = info.location().map(|l| format!("{}", l)).unwrap_or_else(|| "unknown".to_string());
        let _ = std::fs::write("rusttracker_crash.log", format!("RustTracker Panic at {}:\n{}\n\nBacktrace:\n{}", location, msg, backtrace));
    }));

    let args = Args::parse();
    let title = if args.mic {
        "Microphone Input".to_string()
    } else {
        args.file.first().cloned().unwrap_or_default()
    };
    
    let app_state = Arc::new(Mutex::new(AppState::new(title)));
    
    {
        let mut state = app_state.lock().unwrap();
        state.playlist = args.file.clone();
        state.playlist_index = 0;
        
        if let Some(vis) = &args.vis {
            let vis_lower = vis.to_lowercase();
            state.visualizer_mode = if vis_lower.contains("firesim") {
                6
            } else if vis_lower.contains("spectrum") || vis_lower.contains("freq") {
                0
            } else if vis_lower.contains("fire") || vis_lower.contains("flame") {
                1
            } else if vis_lower.contains("osc") || vis_lower.contains("crt") {
                2
            } else if vis_lower.contains("spatial") || vis_lower.contains("vector") {
                3
            } else if vis_lower.contains("ferrofluid") || vis_lower.contains("chrome") {
                4
            } else if vis_lower.contains("neon") || vis_lower.contains("corridor") {
                5
            } else {
                0
            };
        }
    }
    
    let file_path = args.file.first().cloned().unwrap_or_default();
    let initial_stream = match audio::start_audio_thread(&file_path, args.mic, Arc::clone(&app_state)) {
            Ok(stream) => {
                let mut state = app_state.lock().unwrap();
                state.file_loaded = true;
                Some(stream)
            },
            Err(e) => {
                eprintln!("AUDIO LOAD ERROR: {:?}", e);
                None
            }
        };
    if args.tui {
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture, crossterm::cursor::Show);
            original_hook(panic_info);
        }));

        let mut tui = Tui::new()?;
        if let Err(err) = run_tui(&mut tui.terminal, app_state) {
            eprintln!("App error: {:?}", err);
        }
    } else {
        pollster::block_on(run_gui(app_state, initial_stream, args.fullscreen, args.gpu_fft));
    }

    Ok(())
}

#[allow(unused_variables, unused_assignments)]
async fn run_gui(app_state: Arc<Mutex<AppState>>, mut active_stream: Option<cpal::Stream>, is_fullscreen: bool, use_gpu_fft: bool) {

    if use_gpu_fft {
        let mut state = app_state.lock().unwrap();
        state.gpu_fft = true;
    }

    let event_loop = EventLoop::new().unwrap();
    
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::load_from_memory(include_bytes!("../icon.png"))
            .expect("Failed to load icon")
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };
    let window_icon = winit::window::Icon::from_rgba(icon_rgba, icon_width, icon_height).unwrap();

    #[allow(unused_mut)]
    let mut attrs = winit::window::Window::default_attributes()
        .with_title("RustTracker Vulkan Visualizer")
        .with_inner_size(winit::dpi::PhysicalSize::new(1920, 1080))
        .with_window_icon(Some(window_icon));
        
    if is_fullscreen {
        attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
    }
        
    #[cfg(target_os = "linux")]
    {
        attrs = WindowAttributesExtWayland::with_name(attrs, "rusttracker", "rusttracker");
        attrs = WindowAttributesExtX11::with_name(attrs, "rusttracker", "rusttracker");
    }

    #[allow(deprecated)]
    let window = Arc::new(event_loop.create_window(attrs).unwrap());

    let mut engine = VulkanEngine::new(window.clone()).await;
    let mut last_update = Instant::now();
    event_loop.set_control_flow(ControlFlow::Poll);
    
    let egui_ctx = egui::Context::default();
    let mut egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &window, None, None, None);

    let mut last_mouse_move = Instant::now();
    let mut is_cursor_visible = true;

    #[allow(deprecated)]
    let _ = event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { ref event, window_id } if window_id == window.id() => {
                let response = egui_state.on_window_event(&window, event);
                
                if let WindowEvent::CursorMoved { .. } = event {
                    last_mouse_move = Instant::now();
                    if !is_cursor_visible {
                        window.set_cursor_visible(true);
                        is_cursor_visible = true;
                    }
                }
                
                // Process global hotkeys regardless of egui consuming them
                if let WindowEvent::KeyboardInput { event: kb_event, .. } = event {
                    if kb_event.state == ElementState::Pressed && !kb_event.repeat {
                        if let PhysicalKey::Code(keycode) = kb_event.physical_key {
                            match keycode {
                                WinitKeyCode::Escape | WinitKeyCode::KeyQ => elwt.exit(),
                                WinitKeyCode::Tab => {
                                    let mut state = app_state.lock().unwrap();
                                    state.show_hud = !state.show_hud;
                                },
                                WinitKeyCode::KeyF => {
                                    if window.fullscreen().is_some() {
                                        window.set_fullscreen(None);
                                    } else {
                                        window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
                                    }
                                },
                                WinitKeyCode::KeyG => {
                                    let mut state = app_state.lock().unwrap();
                                    state.gpu_fft = !state.gpu_fft;
                                },
                                WinitKeyCode::KeyS => {
                                    let mut state = app_state.lock().unwrap();
                                    state.show_stats = !state.show_stats;
                                },
                                WinitKeyCode::KeyO => {
                                    let app_state_clone = Arc::clone(&app_state);
                                    std::thread::spawn(move || {
                                        if let Some(paths) = rfd::FileDialog::new()
                                            .add_filter("Audio/Video Files", &["flac", "wav", "mp3", "ogg", "aac", "m4a", "mp4", "mkv", "avi", "webm", "opus", "mod", "s3m", "xm", "it", "stm", "669", "mtm", "med", "okt", "psm"])
                                            .add_filter("All Files", &["*"])
                                            .pick_files() {
                                            if !paths.is_empty() {
                                                let mut state = app_state_clone.lock().unwrap();
                                                state.playlist = paths.into_iter().map(|p| p.display().to_string()).collect();
                                                state.playlist_index = 0;
                                                state.load_request = Some(state.playlist[0].clone());
                                                state.file_loaded = true;
                                            }
                                        }
                                    });
                                },
                                WinitKeyCode::Space => {
                                    let mut state = app_state.lock().unwrap();
                                    if state.current_seconds >= state.duration_seconds - 0.1 && state.duration_seconds > 0.0 {
                                        state.seek_request = Some(0.0);
                                        state.spectrum_history.clear();
                                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                        state.is_paused = false;
                                    } else {
                                        state.is_paused = !state.is_paused;
                                    }
                                },
                                WinitKeyCode::ArrowRight => {
                                    let mut state = app_state.lock().unwrap();
                                    let target = state.current_seconds + 5.0;
                                    state.seek_request = Some(target);
                                    state.spectrum_history.clear();
                                    for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                },
                                WinitKeyCode::ArrowLeft => {
                                    let mut state = app_state.lock().unwrap();
                                    let target = (state.current_seconds - 5.0).max(0.0);
                                    state.seek_request = Some(target);
                                    state.spectrum_history.clear();
                                    for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                },
                                WinitKeyCode::ArrowUp => {
                                    let mut state = app_state.lock().unwrap();
                                    if !state.available_visualizers.is_empty() {
                                        state.current_visualizer_idx = (state.current_visualizer_idx + 1) % state.available_visualizers.len();
                                        state.visualizer_mode = state.available_visualizers[state.current_visualizer_idx];
                                    }
                                },
                                WinitKeyCode::ArrowDown => {
                                    let mut state = app_state.lock().unwrap();
                                    if !state.available_visualizers.is_empty() {
                                        if state.current_visualizer_idx == 0 {
                                            state.current_visualizer_idx = state.available_visualizers.len() - 1;
                                        } else {
                                            state.current_visualizer_idx -= 1;
                                        }
                                        state.visualizer_mode = state.available_visualizers[state.current_visualizer_idx];
                                    }
                                },
                                _ => {}
                            }
                        }
                    }
                }

                if response.consumed {
                    return;
                }

                match event {
                WindowEvent::CloseRequested => {
                    elwt.exit();
                }
                WindowEvent::Resized(physical_size) => {
                    engine.resize(*physical_size);
                }
                WindowEvent::RedrawRequested => {
                    let load_path = {
                        let mut state = app_state.lock().unwrap();
                        
                        if state.track_ended {
                            state.track_ended = false;
                            state.playlist_index += 1;
                            if state.playlist_index < state.playlist.len() {
                                state.load_request = Some(state.playlist[state.playlist_index].clone());
                            }
                        }
                        
                        state.load_request.take()
                    };
                    
                    if let Some(path) = load_path {
                        active_stream = None; // DROP OLD STREAM FIRST to release WASAPI lock!
                        // We rely entirely on DSP thread messages to update tracker string state
                        if let Ok(stream) = audio::start_audio_thread(&path, false, Arc::clone(&app_state)) {
                            let mut state = app_state.lock().unwrap();
                            state.file_loaded = true;
                            state.song_title = path.clone();
                            active_stream = Some(stream);
                        } else {
                            app_state.lock().unwrap().file_loaded = false;
                            let mut state = app_state.lock().unwrap();
                            state.artist = "Load Failed".to_string();
                        }
                    }

                    let now = Instant::now();
                    let raw_dt = now.duration_since(last_update).as_secs_f32();
                    let dt = raw_dt.min(0.1);
                    last_update = now;
                    let time_scale = dt * 60.0; // Decay logic built for 60fps
                    let fps = if raw_dt > 0.0 { 1.0 / raw_dt } else { 0.0 };

                    {
                        let mut state = app_state.lock().unwrap();
                        state.current_fps = state.current_fps * 0.9 + fps * 0.1;
                        
                        if !state.file_loaded {
                            let t = now.elapsed().as_secs_f32();
                            for i in 0..1024 {
                                let pct = i as f32 / 1024.0;
                                let wave1 = (t * 2.0 + pct * 10.0).sin();
                                let wave2 = (t * 1.5 - pct * 15.0).cos();
                                let wave3 = (t * 0.5 + pct * 5.0).sin();
                                let combined = (wave1 + wave2 + wave3) / 3.0; // -1 to 1
                                let val = (combined * 0.5 + 0.5).powf(2.0) * 0.5; // 0 to 0.5, biased low
                                state.raw_spectrum_data[i] = val;
                            }
                        }

                        if state.channel_vus.len() != state.raw_channel_vus.len() {
                            state.channel_vus = vec![0.0; state.raw_channel_vus.len()];
                        }
                        for i in 0..state.raw_channel_vus.len() {
                            if state.raw_channel_vus[i] > state.channel_vus[i] {
                                state.channel_vus[i] = state.raw_channel_vus[i];
                            } else {
                                state.channel_vus[i] = (state.channel_vus[i] - (0.015 * time_scale)).max(state.raw_channel_vus[i]);
                            }
                        }

                        if state.peak_vus.len() != state.channel_vus.len() {
                            state.peak_vus = vec![0.0; state.channel_vus.len()];
                        }
                        for i in 0..state.channel_vus.len() {
                            state.peak_vus[i] = (state.peak_vus[i] - (0.005 * time_scale)).max(0.0);
                            if state.channel_vus[i] > state.peak_vus[i] {
                                state.peak_vus[i] = state.channel_vus[i];
                            }
                        }

                        if state.spectrum_data.len() != state.raw_spectrum_data.len() {
                            state.spectrum_data = vec![0.0; state.raw_spectrum_data.len()];
                        }
                        for i in 0..state.raw_spectrum_data.len() {
                            if state.raw_spectrum_data[i] > state.spectrum_data[i] {
                                state.spectrum_data[i] = state.raw_spectrum_data[i];
                            } else {
                                state.spectrum_data[i] = (state.spectrum_data[i] - (1.5 * time_scale)).max(state.raw_spectrum_data[i]);
                            }
                            
                            // Smoothly decay fire heat using display refresh rate
                            if state.raw_spectrum_data[i] > state.fire_heat[i] {
                                state.fire_heat[i] = state.raw_spectrum_data[i];
                            } else {
                                state.fire_heat[i] = (state.fire_heat[i] - (1.5 * time_scale)).max(0.0);
                            }
                        }

                        if state.spectrum_peaks.len() != state.spectrum_data.len() {
                            state.spectrum_peaks = vec![0.0; state.spectrum_data.len()];
                        }
                        for i in 0..state.spectrum_data.len() {
                            state.spectrum_peaks[i] = (state.spectrum_peaks[i] - (0.5 * time_scale)).max(0.0);
                            if state.spectrum_data[i] > state.spectrum_peaks[i] {
                                state.spectrum_peaks[i] = state.spectrum_data[i];
                            }
                        }
                        // Scroll spectrum history
                        if state.spectrum_history.len() > 120 {
                            state.spectrum_history.pop_front();
                        }
                        let cloned_data = state.spectrum_data.clone();
                        state.spectrum_history.push_back(cloned_data);

                        // Temporal smoothing for waveform history to prevent jerky oscilloscope motion.
                        // Lerp the display waveform toward the raw DSP data each frame,
                        // decoupling visual smoothness from the DSP callback rate.
                        let raw_wave = state.raw_waveform.clone();
                        if let Some(newest) = state.waveform_history.back_mut() {
                            let lerp_speed = (12.0 * dt).min(1.0); // fast attack, smooth motion
                            for i in 0..newest.len().min(1024) {
                                newest[i] += (raw_wave[i] - newest[i]) * lerp_speed;
                            }
                        }

                        engine.update(&state);
                    }

                    let state_copy = {
                        app_state.lock().unwrap().clone()
                    };

                    let mut action = EngineAction::None;
                    let mut ui_time = 0.0;
                    let mut render_time = 0.0;
                    let mut fire_time = None;
                    let mut fft_time = None;
                    
                    match engine.render(&window, &egui_ctx, &mut egui_state, &state_copy) {
                            Ok((res, ui_el, ren_el, fire_el, fft_el)) => {
                                action = res;
                                ui_time = ui_el;
                                render_time = ren_el;
                                fire_time = fire_el;
                                fft_time = fft_el;
                            },
                            Err(wgpu::SurfaceStatus::Lost) => engine.resize(engine.size),
                            Err(wgpu::SurfaceStatus::Outdated) => engine.resize(engine.size),
                            Err(wgpu::SurfaceStatus::Timeout) => eprintln!("Surface timeout"),
                            Err(e) => eprintln!("Surface error: {:?}", e),
                        }
                        
                    if ui_time > 0.0 || render_time > 0.0 {
                        let mut state = app_state.lock().unwrap();
                        state.stats.ui_us = state.stats.ui_us * 0.9 + ui_time * 0.1;
                        state.stats.render_us = state.stats.render_us * 0.9 + render_time * 0.1;
                        if let Some(sh) = fire_time {
                            state.stats.fire_us = state.stats.fire_us * 0.9 + sh * 0.1;
                            state.stats.shader_us = state.stats.fire_us; // Keep alias updated for existing code
                        }
                        if let Some(ft) = fft_time {
                            state.stats.gpu_fft_us = state.stats.gpu_fft_us * 0.9 + ft * 0.1;
                        }
                    }
                    
                    if let EngineAction::Seek(pct) = action {
                        let mut state = app_state.lock().unwrap();
                        let target = (state.duration_seconds * pct as f64).clamp(0.0, state.duration_seconds);
                        state.seek_request = Some(target);
                        state.spectrum_history.clear();
                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                    } else if action == EngineAction::OpenFile {
                        let app_state_clone = Arc::clone(&app_state);
                        std::thread::spawn(move || {
                            if let Some(paths) = rfd::FileDialog::new()
                                .add_filter("Audio/Video Files", &["flac", "wav", "mp3", "ogg", "aac", "m4a", "mp4", "mkv", "avi", "webm", "opus", "mod", "s3m", "xm", "it", "stm", "669", "mtm", "med", "okt", "psm"])
                                .add_filter("All Files", &["*"])
                                .pick_files() {
                                if !paths.is_empty() {
                                    let mut state = app_state_clone.lock().unwrap();
                                    state.playlist = paths.into_iter().map(|p| p.display().to_string()).collect();
                                    state.playlist_index = 0;
                                    state.load_request = Some(state.playlist[0].clone());
                                    state.file_loaded = true;
                                }
                            }
                        });
                    }
                    
                    // Fallback for Wayland/Mesa broken FIFO vsync:
                    // Only manually throttle if hardware VSYNC completely failed (e.g. running > 200 FPS).
                    // Unconditional std::thread::sleep overshoots by ~1ms, causing 120Hz monitors to drop to ~116 FPS.
                    if raw_dt < 0.005 {
                        let target_fps = window.current_monitor()
                            .and_then(|m| m.refresh_rate_millihertz())
                            .map(|mhz| mhz as f32 / 1000.0)
                            .unwrap_or(60.0);
                            
                        let target_frame_time = Duration::from_secs_f32(1.0 / target_fps);
                        let elapsed = now.elapsed();
                        if elapsed < target_frame_time {
                            // Sleep up to the last millisecond, then spin-lock for exact precision
                            let sleep_time = target_frame_time.saturating_sub(elapsed);
                            if sleep_time > Duration::from_millis(1) {
                                std::thread::sleep(sleep_time - Duration::from_millis(1));
                            }
                            while now.elapsed() < target_frame_time {
                                std::hint::spin_loop();
                            }
                        }
                    }
                    
                    window.request_redraw();
                }
                _ => {}
                }
            },
            Event::AboutToWait => {
                if window.fullscreen().is_some() {
                    if is_cursor_visible && last_mouse_move.elapsed().as_secs_f32() > 2.0 {
                        window.set_cursor_visible(false);
                        is_cursor_visible = false;
                    }
                } else if !is_cursor_visible {
                    window.set_cursor_visible(true);
                    is_cursor_visible = true;
                }
                
                window.request_redraw();
            }
            _ => {}
        }
    });
}

fn run_tui<B: Backend>(terminal: &mut Terminal<B>, app_state: Arc<Mutex<AppState>>) -> io::Result<()> 
where std::io::Error: From<<B as Backend>::Error>
{
    let tick_rate = Duration::from_millis(16); // ~60fps
    let mut last_tick = Instant::now();

    loop {
        {
            let mut state = app_state.lock().unwrap();

            // Smoothing physics for VUs (Inertia)
            if state.channel_vus.len() != state.raw_channel_vus.len() {
                state.channel_vus = vec![0.0; state.raw_channel_vus.len()];
            }
            for i in 0..state.raw_channel_vus.len() {
                if state.raw_channel_vus[i] > state.channel_vus[i] {
                    state.channel_vus[i] = state.raw_channel_vus[i];
                } else {
                    state.channel_vus[i] = (state.channel_vus[i] - 0.015).max(state.raw_channel_vus[i]);
                }
            }

            // Decay peaks and apply gravity
            if state.peak_vus.len() != state.channel_vus.len() {
                state.peak_vus = vec![0.0; state.channel_vus.len()];
            }
            for i in 0..state.channel_vus.len() {
                state.peak_vus[i] = (state.peak_vus[i] - 0.005).max(0.0);
                if state.channel_vus[i] > state.peak_vus[i] {
                    state.peak_vus[i] = state.channel_vus[i];
                }
            }

            // Smoothing physics for Spectrum
            if state.spectrum_data.len() != state.raw_spectrum_data.len() {
                state.spectrum_data = vec![0.0; state.raw_spectrum_data.len()];
            }
            for i in 0..state.raw_spectrum_data.len() {
                if state.raw_spectrum_data[i] > state.spectrum_data[i] {
                    state.spectrum_data[i] = state.raw_spectrum_data[i];
                } else {
                    state.spectrum_data[i] = (state.spectrum_data[i] - 1.5).max(state.raw_spectrum_data[i]);
                }
            }

            // Decay spectrum peaks
            if state.spectrum_peaks.len() != state.spectrum_data.len() {
                state.spectrum_peaks = vec![0.0; state.spectrum_data.len()];
            }
            for i in 0..state.spectrum_data.len() {
                state.spectrum_peaks[i] = (state.spectrum_peaks[i] - 0.5).max(0.0);
                if state.spectrum_data[i] > state.spectrum_peaks[i] {
                    state.spectrum_peaks[i] = state.spectrum_data[i];
                }
            }

            // Scroll spectrum history
            state.spectrum_history.pop_front();
            let cloned_data = state.spectrum_data.clone();
            state.spectrum_history.push_back(cloned_data);

            terminal.draw(|f| ui::draw(f, &state))?;
        }

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let CEvent::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => {
                        return Ok(());
                    }
                    KeyCode::Char(' ') => {
                        let mut state = app_state.lock().unwrap();
                        state.is_paused = !state.is_paused;
                    }
                    KeyCode::Right => {
                        let mut state = app_state.lock().unwrap();
                        let target = (state.current_seconds + 5.0).min(state.duration_seconds);
                        state.seek_request = Some(target);
                        state.spectrum_history.clear();
                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 128]); }
                    }
                    KeyCode::Left => {
                        let mut state = app_state.lock().unwrap();
                        let target = (state.current_seconds - 5.0).max(0.0);
                        state.seek_request = Some(target);
                        state.spectrum_history.clear();
                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 128]); }
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }
}
