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

mod audio;
mod state;
mod ui;
mod engine;

use crate::state::AppState;
use crate::engine::{VulkanEngine, EngineAction};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    file: Option<String>,

    #[arg(long, default_value_t = false)]
    tui: bool,

    #[arg(long, default_value_t = false)]
    mic: bool,
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
    let args = Args::parse();
    let title = if args.mic {
        "Microphone Input".to_string()
    } else {
        args.file.clone().unwrap_or_default()
    };
    
    let app_state = Arc::new(Mutex::new(AppState::new(title)));
    
    let mut initial_stream = None;
    if args.mic || args.file.is_some() {
        let file_path = args.file.clone().unwrap_or_default();
        initial_stream = audio::start_audio_thread(&file_path, args.mic, Arc::clone(&app_state)).ok();
    }

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
        pollster::block_on(run_gui(app_state, initial_stream));
    }

    Ok(())
}

#[allow(unused_variables, unused_assignments)]
async fn run_gui(app_state: Arc<Mutex<AppState>>, mut active_stream: Option<cpal::Stream>) {
    let event_loop = EventLoop::new().unwrap();
    
    #[allow(deprecated)]
    let window = Arc::new(event_loop.create_window(
        winit::window::Window::default_attributes()
            .with_title("RustTracker Vulkan Visualizer")
            .with_inner_size(winit::dpi::PhysicalSize::new(1920, 1080))
    ).unwrap());

    let mut engine = VulkanEngine::new(window.clone()).await;
    let mut last_update = Instant::now();
    event_loop.set_control_flow(ControlFlow::Poll);
    
    let egui_ctx = egui::Context::default();
    let mut egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &window, None, None, None);

    #[allow(deprecated)]
    let _ = event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { ref event, window_id } if window_id == window.id() => {
                let response = egui_state.on_window_event(&window, event);
                
                // Process global hotkeys regardless of egui consuming them
                if let WindowEvent::KeyboardInput { event: kb_event, .. } = event {
                    if kb_event.state == ElementState::Pressed && !kb_event.repeat {
                        if let PhysicalKey::Code(keycode) = kb_event.physical_key {
                            match keycode {
                                WinitKeyCode::Escape | WinitKeyCode::KeyQ => elwt.exit(),
                                WinitKeyCode::Tab | WinitKeyCode::KeyF => {
                                    let mut state = app_state.lock().unwrap();
                                    state.show_hud = !state.show_hud;
                                },
                                WinitKeyCode::Space => {
                                    let mut state = app_state.lock().unwrap();
                                    state.is_paused = !state.is_paused;
                                },
                                WinitKeyCode::ArrowRight => {
                                    let mut state = app_state.lock().unwrap();
                                    let target = (state.current_seconds + 5.0).min(state.duration_seconds);
                                    state.seek_request = Some(target);
                                    state.spectrum_history.clear();
                                    for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 512]); }
                                },
                                WinitKeyCode::ArrowLeft => {
                                    let mut state = app_state.lock().unwrap();
                                    let target = (state.current_seconds - 5.0).max(0.0);
                                    state.seek_request = Some(target);
                                    state.spectrum_history.clear();
                                    for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 512]); }
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
                        state.load_request.take()
                    };
                    
                    if let Some(path) = load_path {
                        active_stream = audio::start_audio_thread(&path, false, Arc::clone(&app_state)).ok();
                    }

                    let now = Instant::now();
                    let dt = now.duration_since(last_update).as_secs_f32().min(0.1);
                    last_update = now;
                    let time_scale = dt * 60.0; // Decay logic built for 60fps

                    {
                        let mut state = app_state.lock().unwrap();
                        
                        if !state.file_loaded {
                            let t = now.elapsed().as_secs_f32();
                            for i in 0..512 {
                                let pct = i as f32 / 512.0;
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

                        engine.update(&state);
                    }

                    let state_copy = {
                        app_state.lock().unwrap().clone()
                    };

                    let mut action = EngineAction::None;
                    match engine.render(&window, &egui_ctx, &mut egui_state, &state_copy) {
                            Ok(res) => action = res,
                            Err(wgpu::SurfaceError::Lost) => engine.resize(engine.size),
                            Err(wgpu::SurfaceError::OutOfMemory) => elwt.exit(),
                            Err(e) => eprintln!("{:?}", e),
                        }
                    
                    if action == EngineAction::OpenFile {
                        let app_state_clone = Arc::clone(&app_state);
                        std::thread::spawn(move || {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Tracker Modules", &["mod", "s3m", "xm", "it", "stm", "669", "mtm", "med", "okt", "psm"])
                                .add_filter("All Files", &["*"])
                                .pick_file() {
                                let mut state = app_state_clone.lock().unwrap();
                                state.load_request = Some(path.display().to_string());
                                state.file_loaded = true;
                            }
                        });
                    }
                    
                    window.request_redraw();
                }
                _ => {}
                }
            },
            Event::AboutToWait => {
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
