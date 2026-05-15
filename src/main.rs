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
mod engine;
mod state;
mod ui;
pub mod bitstream;

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
            if let Some(idx) = crate::state::VISUALIZERS.iter().position(|v| v.filename.to_lowercase().contains(&vis_lower) || v.name.to_lowercase().contains(&vis_lower)) {
                state.current_visualizer_idx = idx;
                state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
            }
        }
    }
    
    let file_path = args.file.first().cloned().unwrap_or_default();
    let initial_stream = if !file_path.is_empty() || args.mic {
        match audio::start_audio_thread(&file_path, args.mic, Arc::clone(&app_state)) {
            Ok(stream) => {
                let mut state = app_state.lock().unwrap();
                state.file_loaded = true;
                Some(stream)
            },
            Err(e) => {
                eprintln!("AUDIO LOAD ERROR: {:?}", e);
                None
            }
        }
    } else {
        None
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
async fn run_gui(app_state: Arc<Mutex<AppState>>, mut active_stream: Option<audio::PlaybackHandle>, is_fullscreen: bool, use_gpu_fft: bool) {
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
        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
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
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "kenney_icons".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/kenney-icon-font.ttf")).into(),
    );
    fonts.font_data.insert(
        "orbitron".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/Orbitron-Black.ttf")).into(),
    );
    fonts.families.insert(
        egui::FontFamily::Name("Orbitron".into()),
        vec!["orbitron".to_owned()],
    );
    fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().push("kenney_icons".to_owned());
    egui_ctx.set_fonts(fonts);
    
    let mut egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &window, None, None, None);

    let mut last_mouse_move = Instant::now();
    let mut is_cursor_visible = true;
    let mut is_fullscreen = false;
    let mut is_first_frame = true;

    let mut gilrs = gilrs::Gilrs::new().unwrap_or_else(|_| gilrs::GilrsBuilder::new().build().unwrap());

    let is_game_mode = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase() == "gamescope" || 
                       std::env::var("XDG_SESSION_DESKTOP").unwrap_or_default().to_lowercase() == "gamescope" ||
                       std::env::var("STEAM_DECK").is_ok();

    #[cfg(windows)]
    let initial_dir = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string()));
    #[cfg(not(windows))]
    let initial_dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));

    let mut file_dialog = egui_file_dialog::FileDialog::new()
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .initial_directory(initial_dir.clone())
        .add_file_filter_extensions("Audio/Video Files", vec!["flac", "wav", "mp3", "ogg", "aac", "m4a", "mp4", "mkv", "avi", "webm", "opus", "mod", "s3m", "xm", "it", "stm", "669", "mtm", "med", "okt", "psm", "dawproject", "aaf"])
        .default_file_filter("Audio/Video Files");

    // Native file picker channel (used on non-SteamDeck systems to bypass
    // egui-file-dialog's synchronous sysinfo disk enumeration)
    let (rfd_tx, rfd_rx) = crossbeam_channel::unbounded::<Vec<String>>();
    let mut rfd_pending = false;

    let mut modifiers = winit::keyboard::ModifiersState::empty();

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
                
                if let WindowEvent::ModifiersChanged(m) = &event {
                    modifiers = m.state();
                }

                // Process global hotkeys regardless of egui consuming them
                if let WindowEvent::KeyboardInput { event: kb_event, .. } = &event {
                    if kb_event.state == ElementState::Pressed {
                        if let PhysicalKey::Code(keycode) = kb_event.physical_key {
                            match keycode {
                                WinitKeyCode::ArrowRight => {
                                    let mut state = app_state.lock().unwrap();
                                    if modifiers.control_key() {
                                        if !kb_event.repeat {
                                            state.track_ended = true;
                                        }
                                    } else {
                                        let target = state.current_seconds + 5.0;
                                        state.seek_request = Some(target);
                                        state.spectrum_history.clear();
                                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                    }
                                },
                                WinitKeyCode::ArrowLeft => {
                                    let mut state = app_state.lock().unwrap();
                                    if modifiers.control_key() {
                                        if !kb_event.repeat {
                                            if state.playlist_index > 0 {
                                                state.playlist_index -= 1;
                                                state.load_request = Some(state.playlist[state.playlist_index].clone());
                                                state.track_ended = false;
                                            }
                                        }
                                    } else {
                                        let target = (state.current_seconds - 5.0).max(0.0);
                                        state.seek_request = Some(target);
                                        state.spectrum_history.clear();
                                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                    }
                                },
                                _ => {
                                    if !kb_event.repeat {
                                        match keycode {
                                            WinitKeyCode::Escape | WinitKeyCode::KeyQ => {
                                                let picker_open = app_state.lock().unwrap().show_vis_picker;
                                                if picker_open {
                                                    app_state.lock().unwrap().show_vis_picker = false;
                                                } else {
                                                    elwt.exit();
                                                }
                                            },
                                            WinitKeyCode::BracketLeft => {
                                                let mut state = app_state.lock().unwrap();
                                                state.panel_split_ratio = (state.panel_split_ratio - 0.05).clamp(0.15, 0.85);
                                            },
                                            WinitKeyCode::BracketRight => {
                                                let mut state = app_state.lock().unwrap();
                                                state.panel_split_ratio = (state.panel_split_ratio + 0.05).clamp(0.15, 0.85);
                                            },
                                            WinitKeyCode::Tab => {
                                                let mut state = app_state.lock().unwrap();
                                                state.show_hud = !state.show_hud;
                                            },
                                            WinitKeyCode::KeyF => {
                                                is_fullscreen = !is_fullscreen;
                                                if is_fullscreen {
                                                    window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
                                                } else {
                                                    window.set_fullscreen(None);
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
                                            WinitKeyCode::KeyH => {
                                                let mut state = app_state.lock().unwrap();
                                                state.show_help = !state.show_help;
                                            },
                                            WinitKeyCode::KeyO => {
                                                if is_game_mode {
                                                    let mut state = app_state.lock().unwrap();
                                                    state.open_file_request = true;
                                                } else if !rfd_pending {
                                                    rfd_pending = true;
                                                    let tx = rfd_tx.clone();
                                                    let dir = initial_dir.clone();
                                                    std::thread::spawn(move || {
                                                        let result = rfd::FileDialog::new()
                                                            .set_directory(&dir)
                                                            .add_filter("Audio/Video Files", &["flac", "wav", "mp3", "ogg", "aac", "m4a", "mp4", "mkv", "avi", "webm", "opus", "mod", "s3m", "xm", "it", "stm", "669", "mtm", "med", "okt", "psm", "dawproject", "aaf"])
                                                            .pick_files();
                                                        if let Some(paths) = result {
                                                            let strings: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
                                                            let _ = tx.send(strings);
                                                        } else {
                                                            let _ = tx.send(vec![]);
                                                        }
                                                    });
                                                }
                                            },
                                            WinitKeyCode::KeyM => {
                                                let mut state = app_state.lock().unwrap();
                                                state.show_vis_picker = !state.show_vis_picker;
                                                if state.show_vis_picker {
                                                    state.vis_picker_cursor = state.current_visualizer_idx;
                                                }
                                            },
                                            WinitKeyCode::KeyV => {
                                                let mut state = app_state.lock().unwrap();
                                                if state.video_frame_rx.is_some() {
                                                    state.video_mode = (state.video_mode + 1) % 4;
                                                } else {
                                                    state.video_mode = 0;
                                                }
                                            },
                                            WinitKeyCode::Space => {
                                                let mut state = app_state.lock().unwrap();
                                                if state.show_vis_picker {
                                                    // Toggle enable/disable for highlighted visualizer (idx 0 always enabled)
                                                    let idx = state.vis_picker_cursor;
                                                    if idx != 0 {
                                                        state.vis_enabled[idx] = !state.vis_enabled[idx];
                                                    }
                                                } else if state.current_seconds >= state.duration_seconds - 0.1 && state.duration_seconds > 0.0 {
                                                    state.seek_request = Some(0.0);
                                                    state.spectrum_history.clear();
                                                    for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                                    state.is_paused = false;
                                                } else {
                                                    state.is_paused = !state.is_paused;
                                                }
                                            },
                                            WinitKeyCode::Enter => {
                                                let mut state = app_state.lock().unwrap();
                                                if state.show_vis_picker {
                                                    state.current_visualizer_idx = state.vis_picker_cursor;
                                                    state.visualizer_mode = crate::state::VISUALIZERS[state.vis_picker_cursor].id;
                                                    state.show_vis_picker = false;
                                                }
                                            },
                                            WinitKeyCode::ArrowUp => {
                                                let mut state = app_state.lock().unwrap();
                                                if state.show_vis_picker {
                                                    if state.vis_picker_cursor == 0 {
                                                        state.vis_picker_cursor = crate::state::VISUALIZERS.len() - 1;
                                                    } else {
                                                        state.vis_picker_cursor -= 1;
                                                    }
                                                } else {
                                                    // Cycle to next enabled visualizer
                                                    let len = crate::state::VISUALIZERS.len();
                                                    let mut idx = state.current_visualizer_idx;
                                                    for _ in 0..len {
                                                        idx = (idx + 1) % len;
                                                        if state.vis_enabled[idx] { break; }
                                                    }
                                                    state.current_visualizer_idx = idx;
                                                    state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                                                }
                                            },
                                            WinitKeyCode::ArrowDown => {
                                                let mut state = app_state.lock().unwrap();
                                                if state.show_vis_picker {
                                                    state.vis_picker_cursor = (state.vis_picker_cursor + 1) % crate::state::VISUALIZERS.len();
                                                } else {
                                                    // Cycle to previous enabled visualizer
                                                    let len = crate::state::VISUALIZERS.len();
                                                    let mut idx = state.current_visualizer_idx;
                                                    for _ in 0..len {
                                                        idx = if idx == 0 { len - 1 } else { idx - 1 };
                                                        if state.vis_enabled[idx] { break; }
                                                    }
                                                    state.current_visualizer_idx = idx;
                                                    state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                                                }
                                            },
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let WindowEvent::DroppedFile(path) = &event {
                    let mut state = app_state.lock().unwrap();
                    let path_str = path.to_string_lossy().into_owned();
                    if state.playlist.is_empty() {
                        state.playlist = vec![path_str.clone()];
                        state.playlist_index = 0;
                        state.load_request = Some(path_str);
                    } else {
                        state.playlist.push(path_str);
                        if !state.file_loaded {
                            state.playlist_index = state.playlist.len() - 1;
                            state.load_request = Some(state.playlist.last().unwrap().clone());
                        }
                    }
                    state.file_loaded = true;
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
                    if is_first_frame {
                        is_first_frame = false;
                        app_state.lock().unwrap().is_paused = false;
                    }
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
                        
                        // Cleanup old video state completely before loading the next file
                        engine.clear_video_state();
                        {
                            let mut state = app_state.lock().unwrap();
                            state.video_frame_rx = None;
                            state.free_video_frame_tx = None;
                            state.video_mode = 0;
                        }
                        
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

                    let phase_timer = Instant::now();
                    {
                        let mut state = app_state.lock().unwrap();
                        state.current_fps = state.current_fps * 0.9 + fps * 0.1;
                        state.visual_width = engine.config.width / 2;
                        
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

                        // Gamepad analog panel scaling (Stage 2)
                        for (_id, gamepad) in gilrs.gamepads() {
                            let right_y = gamepad.value(gilrs::Axis::RightStickY);
                            if right_y.abs() > 0.1 {
                                let scaled_delta = right_y * dt * 1.0; // Subtract moving UP (positive Y stick)
                                state.panel_split_ratio = (state.panel_split_ratio - scaled_delta).clamp(0.15, 0.85);
                            }
                        }

                        engine.update(&state);
                    }
                    let phase_lock_update_us = phase_timer.elapsed().as_micros() as f32;

                    // Create a lightweight snapshot for render (skips ~1.3 MB of audio data)
                    // and drain gamepad events in the same lock acquisition
                    let snapshot_timer = Instant::now();
                    let (state_copy, gamepad_events) = {
                        let mut state = app_state.lock().unwrap();
                        let events = state.egui_gamepad_events.clone();
                        state.egui_gamepad_events.clear();
                        (state.render_snapshot(), events)
                    };
                    let phase_snapshot_us = snapshot_timer.elapsed().as_micros() as f32;

                    let mut action = EngineAction::None;
                    let mut ui_time = 0.0;
                    let mut render_time = 0.0;
                    let mut fire_time = None;
                    let mut fft_time = None;
                    let mut vis_shader_time = None;
                    let mut phase_surface_us = 0.0f32;
                    let mut phase_egui_us = 0.0f32;
                    let mut phase_encode_us = 0.0f32;

                    match engine.render(&window, &egui_ctx, &mut egui_state, &state_copy, &mut file_dialog, gamepad_events) {
                            Ok((res, ui_el, ren_el, fire_el, fft_el, vis_el, surf, egui_l, enc, _sub)) => {
                                action = res.clone();
                                ui_time = ui_el;
                                render_time = ren_el;
                                fire_time = fire_el;
                                fft_time = fft_el;
                                vis_shader_time = vis_el;
                                phase_surface_us = surf;
                                phase_egui_us = egui_l;
                                phase_encode_us = enc;
                            },
                            Err(wgpu::SurfaceStatus::Lost) => engine.resize(engine.size),
                            Err(wgpu::SurfaceStatus::Outdated) => engine.resize(engine.size),
                            Err(wgpu::SurfaceStatus::Timeout) => eprintln!("Surface timeout"),
                            Err(e) => eprintln!("Surface error: {:?}", e),
                        }
                        
                    // Consolidate all post-render state writes into a single lock
                    let post_timer = Instant::now();
                    let mut trigger_picker = false;
                    {
                        let mut state = app_state.lock().unwrap();
                        
                        // Write back timing stats
                        if ui_time > 0.0 || render_time > 0.0 {
                            state.stats.ui_us = state.stats.ui_us * 0.9 + ui_time * 0.1;
                            state.stats.render_us = state.stats.render_us * 0.9 + render_time * 0.1;
                            if let Some(sh) = fire_time {
                                state.stats.fire_us = state.stats.fire_us * 0.9 + sh * 0.1;
                            }
                            if let Some(vis) = vis_shader_time {
                                state.stats.shader_us = state.stats.shader_us * 0.9 + vis * 0.1;
                            }
                            if let Some(ft) = fft_time {
                                state.stats.gpu_fft_us = state.stats.gpu_fft_us * 0.9 + ft * 0.1;
                            }
                        }
                        
                        // Process engine actions
                        match action {
                            EngineAction::Seek(pct) => {
                                let target = (state.duration_seconds * pct as f64).clamp(0.0, state.duration_seconds);
                                state.seek_request = Some(target);
                                state.spectrum_history.clear();
                                for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                            }
                            EngineAction::OpenFile => {
                                if is_game_mode {
                                    // SteamDeck: use egui-file-dialog (gamepad-navigable)
                                    state.open_file_request = true;
                                } else if !rfd_pending {
                                    // Desktop: use native OS file picker via rfd
                                    rfd_pending = true;
                                    let tx = rfd_tx.clone();
                                    let dir = initial_dir.clone();
                                    std::thread::spawn(move || {
                                        let result = rfd::FileDialog::new()
                                            .set_directory(&dir)
                                            .add_filter("Audio/Video Files", &["flac", "wav", "mp3", "ogg", "aac", "m4a", "mp4", "mkv", "avi", "webm", "opus", "mod", "s3m", "xm", "it", "stm", "669", "mtm", "med", "okt", "psm", "dawproject", "aaf"])
                                            .pick_files();
                                        if let Some(paths) = result {
                                            let strings: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
                                            let _ = tx.send(strings);
                                        } else {
                                            let _ = tx.send(vec![]);
                                        }
                                    });
                                }
                            }
                            EngineAction::LoadFiles(paths, append) => {
                                if append && !state.playlist.is_empty() {
                                    state.playlist.extend(paths);
                                } else if !paths.is_empty() {
                                    state.playlist = paths;
                                    state.playlist_index = 0;
                                    state.load_request = Some(state.playlist[0].clone());
                                }
                                state.file_loaded = true;
                                state.is_file_picker_open = false;
                            }
                            EngineAction::SetAppendToPlaylist(val) => {
                                state.append_to_playlist = val;
                            }
                            EngineAction::SetForceStereo(val) => {
                                state.force_stereo_downmix = val;
                            }
                            EngineAction::SetPassthrough(val) => {
                                state.passthrough_enabled = val;
                            }
                            EngineAction::SetSplitRatio(val) => {
                                state.panel_split_ratio = val;
                            }
                            EngineAction::VisPickerSelect(idx) => {
                                state.current_visualizer_idx = idx;
                                state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                                state.show_vis_picker = false;
                            }
                            EngineAction::VisPickerToggleEnabled(idx) => {
                                if idx != 0 { // Frequency Spectrum always enabled
                                    state.vis_enabled[idx] = !state.vis_enabled[idx];
                                }
                            }
                            EngineAction::VisPickerSetCursor(idx) => {
                                state.vis_picker_cursor = idx;
                            }
                            EngineAction::VisPickerEnableAll => {
                                for i in 0..state.vis_enabled.len() {
                                    state.vis_enabled[i] = true;
                                }
                            }
                            EngineAction::VisPickerEnableNone => {
                                for i in 0..state.vis_enabled.len() {
                                    // Index 0 (Frequency Spectrum) is always enabled
                                    state.vis_enabled[i] = i == 0;
                                }
                            }
                            EngineAction::None => {}
                        }
                        
                        // File picker state
                        state.is_file_picker_open = *file_dialog.state() != egui_file_dialog::DialogState::Closed;
                        if state.open_file_request && !state.is_file_picker_open {
                            state.open_file_request = false;
                            state.is_file_picker_open = true;
                            trigger_picker = true;
                        }
                        
                        // Write phase timings
                        state.stats.phase_lock_update_us = state.stats.phase_lock_update_us * 0.9 + phase_lock_update_us * 0.1;
                        state.stats.phase_snapshot_us = state.stats.phase_snapshot_us * 0.9 + phase_snapshot_us * 0.1;
                        state.stats.phase_surface_us = state.stats.phase_surface_us * 0.9 + phase_surface_us * 0.1;
                        state.stats.phase_egui_layout_us = state.stats.phase_egui_layout_us * 0.9 + phase_egui_us * 0.1;
                        state.stats.phase_encode_us = state.stats.phase_encode_us * 0.9 + phase_encode_us * 0.1;
                        state.stats.phase_post_us = state.stats.phase_post_us * 0.9 + post_timer.elapsed().as_micros() as f32 * 0.1;
                        // Poll native file picker results
                        if let Ok(paths) = rfd_rx.try_recv() {
                            rfd_pending = false;
                            if !paths.is_empty() {
                                let append = state.append_to_playlist;
                                if append && !state.playlist.is_empty() {
                                    state.playlist.extend(paths);
                                } else {
                                    state.playlist = paths;
                                    state.playlist_index = 0;
                                    state.load_request = Some(state.playlist[0].clone());
                                }
                                state.file_loaded = true;
                                state.is_file_picker_open = false;
                            }
                        }
                    }
                    
                    if trigger_picker {
                        file_dialog.pick_multiple();
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
                    let is_dialog_open = *file_dialog.state() != egui_file_dialog::DialogState::Closed;
                    
                    {
                        let mut state = app_state.lock().unwrap();
                        
                        let mut has_gp = false;
                        for _ in gilrs.gamepads() {
                            has_gp = true;
                        }
                        state.has_gamepad = is_game_mode || has_gp;

                        if !is_game_mode {
                            let mut g_type = crate::state::GamepadType::Xbox;
                            for (_id, gamepad) in gilrs.gamepads() {
                                let name = gamepad.name().to_lowercase();
                                let vendor = gamepad.vendor_id().unwrap_or(0);
                                if name.contains("sony") || name.contains("dualshock") || name.contains("dualsense") || name.contains("ps4") || name.contains("ps5") || name.contains("wireless controller") || vendor == 0x054C {
                                    g_type = crate::state::GamepadType::PlayStation;
                                    break;
                                } else if name.contains("nintendo") || name.contains("pro controller") || name.contains("joy-con") || vendor == 0x057E {
                                    g_type = crate::state::GamepadType::Nintendo;
                                    break;
                                }
                            }
                            state.gamepad_type = g_type;
                        }
                    }
                    
                    while let Some(gilrs::Event { id: _, event: g_event, time: _, .. }) = gilrs.next_event() {
                    let is_pressed = matches!(g_event, gilrs::EventType::ButtonPressed(_, _));
                    let is_repeated = matches!(g_event, gilrs::EventType::ButtonRepeated(_, _));
                    if is_pressed || is_repeated {
                        let button = match g_event {
                            gilrs::EventType::ButtonPressed(b, _) => b,
                            gilrs::EventType::ButtonRepeated(b, _) => b,
                            _ => unreachable!(),
                        };

                        if is_repeated {
                            match button {
                                gilrs::Button::DPadRight => {
                                    let mut state = app_state.lock().unwrap();
                                    let target = state.current_seconds + 5.0;
                                    state.seek_request = Some(target);
                                    state.spectrum_history.clear();
                                    for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                }
                                gilrs::Button::DPadLeft => {
                                    let mut state = app_state.lock().unwrap();
                                    let target = (state.current_seconds - 5.0).max(0.0);
                                    state.seek_request = Some(target);
                                    state.spectrum_history.clear();
                                    for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                }
                                _ => {}
                            }
                            continue;
                        }
                            // System-critical buttons always work regardless of dialog state
                            match button {
                                gilrs::Button::Select => {
                                    elwt.exit();
                                    continue;
                                }
                                gilrs::Button::Start => {
                                    is_fullscreen = !is_fullscreen;
                                    if is_fullscreen {
                                        window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
                                    } else {
                                        window.set_fullscreen(None);
                                    }
                                    continue;
                                }
                                _ => {}
                            }

                            if is_dialog_open {
                                let mut state = app_state.lock().unwrap();
                                let mut push_key = |key: egui::Key| {
                                    state.egui_gamepad_events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
                                    state.egui_gamepad_events.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE });
                                };
                                match button {
                                    gilrs::Button::DPadUp => { push_key(egui::Key::ArrowUp); continue; }
                                    gilrs::Button::DPadDown => { push_key(egui::Key::ArrowDown); continue; }
                                    gilrs::Button::DPadLeft => { push_key(egui::Key::ArrowLeft); continue; }
                                    gilrs::Button::DPadRight => { push_key(egui::Key::ArrowRight); continue; }
                                    gilrs::Button::South => { push_key(egui::Key::Enter); continue; }
                                    gilrs::Button::East => { push_key(egui::Key::Escape); continue; }
                                    _ => {}
                                }
                            }
                            
                            match button {
                                gilrs::Button::LeftTrigger => { // L1 Bumper
                                    let mut state = app_state.lock().unwrap();
                                    if state.playlist_index > 0 {
                                        state.playlist_index -= 1;
                                        state.load_request = Some(state.playlist[state.playlist_index].clone());
                                        state.track_ended = false;
                                    }
                                }
                                gilrs::Button::RightTrigger => { // R1 Bumper
                                    let mut state = app_state.lock().unwrap();
                                    state.track_ended = true;
                                }
                                gilrs::Button::LeftTrigger2 => { // L2
                                    let mut state = app_state.lock().unwrap();
                                    state.show_hud = !state.show_hud;
                                }
                                gilrs::Button::RightTrigger2 => { // R2
                                    let mut state = app_state.lock().unwrap();
                                    state.gpu_fft = !state.gpu_fft;
                                }
                                gilrs::Button::North => { // 'Y' or Triangle
                                    let mut state = app_state.lock().unwrap();
                                    state.open_file_request = true;
                                }
                                gilrs::Button::West => { // 'X' or Square
                                    let mut state = app_state.lock().unwrap();
                                    if state.video_frame_rx.is_some() {
                                        state.video_mode = (state.video_mode + 1) % 4;
                                    } else {
                                        state.video_mode = 0;
                                    }
                                }
                                gilrs::Button::East => { // 'B' or Circle
                                    let mut state = app_state.lock().unwrap();
                                    if state.show_vis_picker {
                                        state.show_vis_picker = false;
                                    } else {
                                        state.show_stats = !state.show_stats;
                                    }
                                }
                                gilrs::Button::DPadRight => {
                                    let mut state = app_state.lock().unwrap();
                                    if !state.show_vis_picker {
                                        let target = state.current_seconds + 5.0;
                                        state.seek_request = Some(target);
                                        state.spectrum_history.clear();
                                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                    }
                                }
                                gilrs::Button::DPadLeft => {
                                    let mut state = app_state.lock().unwrap();
                                    if !state.show_vis_picker {
                                        let target = (state.current_seconds - 5.0).max(0.0);
                                        state.seek_request = Some(target);
                                        state.spectrum_history.clear();
                                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                    }
                                }
                                gilrs::Button::DPadUp => {
                                    let mut state = app_state.lock().unwrap();
                                    if state.show_vis_picker {
                                        if state.vis_picker_cursor == 0 {
                                            state.vis_picker_cursor = crate::state::VISUALIZERS.len() - 1;
                                        } else {
                                            state.vis_picker_cursor -= 1;
                                        }
                                    } else {
                                        let len = crate::state::VISUALIZERS.len();
                                        let mut idx = state.current_visualizer_idx;
                                        for _ in 0..len {
                                            idx = (idx + 1) % len;
                                            if state.vis_enabled[idx] { break; }
                                        }
                                        state.current_visualizer_idx = idx;
                                        state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                                    }
                                }
                                gilrs::Button::DPadDown => {
                                    let mut state = app_state.lock().unwrap();
                                    if state.show_vis_picker {
                                        state.vis_picker_cursor = (state.vis_picker_cursor + 1) % crate::state::VISUALIZERS.len();
                                    } else {
                                        let len = crate::state::VISUALIZERS.len();
                                        let mut idx = state.current_visualizer_idx;
                                        for _ in 0..len {
                                            idx = if idx == 0 { len - 1 } else { idx - 1 };
                                            if state.vis_enabled[idx] { break; }
                                        }
                                        state.current_visualizer_idx = idx;
                                        state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                                    }
                                }
                                gilrs::Button::South => {
                                    let mut state = app_state.lock().unwrap();
                                    if state.show_vis_picker {
                                        state.current_visualizer_idx = state.vis_picker_cursor;
                                        state.visualizer_mode = crate::state::VISUALIZERS[state.vis_picker_cursor].id;
                                        state.show_vis_picker = false;
                                    } else if state.current_seconds >= state.duration_seconds - 0.1 && state.duration_seconds > 0.0 {
                                        state.seek_request = Some(0.0);
                                        state.spectrum_history.clear();
                                        for _ in 0..120 { state.spectrum_history.push_back(vec![0.0; 1024]); }
                                        state.is_paused = false;
                                    } else {
                                        state.is_paused = !state.is_paused;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                if is_fullscreen {
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
    let mut is_first_frame = true;

    loop {
        if is_first_frame {
            is_first_frame = false;
            app_state.lock().unwrap().is_paused = false;
        }
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

