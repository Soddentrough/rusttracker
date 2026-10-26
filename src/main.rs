#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use std::{
    error::Error,
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowAttributesExtWayland;
#[cfg(target_os = "linux")]
use winit::platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11};
use winit::{
    event::{ElementState, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode as WinitKeyCode, PhysicalKey},
};

use rusttracker::{audio, engine, ipc, lyrics, playlist, state, ui};

use crate::engine::{EngineAction, VulkanEngine};
use crate::state::AppState;

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

    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    vis: Option<String>,

    #[arg(long, default_value_t = false)]
    list_vis: bool,

    #[arg(long, default_value_t = false)]
    gpu_fft: bool,

    #[arg(long)]
    bench: Option<u32>,

    #[arg(
        long,
        default_value_t = false,
        help = "Disable VSync / display sync and unlock uncapped frame rates"
    )]
    uncapped: bool,
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
    env_logger::init();
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg = match info.payload().downcast_ref::<&'static str>() {
            Some(s) => *s,
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => &s[..],
                None => "Box<dyn Any>",
            },
        };
        let location = info
            .location()
            .map(|l| format!("{}", l))
            .unwrap_or_else(|| "unknown".to_string());
        let crash_report = format!(
            "RustTracker Panic at {}:\n{}\n\nBacktrace:\n{}",
            location, msg, backtrace
        );
        if std::fs::write("rusttracker_crash.log", &crash_report).is_err() {
            let fallback_path = directories::ProjectDirs::from("com", "RustTracker", "RustTracker")
                .map(|proj| {
                    let dir = proj.data_local_dir();
                    let _ = std::fs::create_dir_all(dir);
                    dir.join("rusttracker_crash.log")
                })
                .unwrap_or_else(|| std::env::temp_dir().join("rusttracker_crash.log"));
            let _ = std::fs::write(fallback_path, &crash_report);
        }
    }));

    let filtered_args =
        std::env::args_os().filter(|arg| !arg.to_str().is_some_and(|s| s.starts_with("-psn_")));
    let mut args = Args::parse_from(filtered_args);
    if args.uncapped {
        crate::state::UNCAPPED_PRESENT_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let should_list_vis =
        args.list_vis || matches!(args.vis.as_deref(), Some("") | Some("list") | Some("help"));
    if should_list_vis {
        println!(
            "{:<4} {:<24} {:<28} Description",
            "ID", "Short Name", "Name"
        );
        println!("{}", "-".repeat(90));
        let mut visualizers: Vec<_> = crate::state::VISUALIZERS.iter().collect();
        visualizers.sort_by_key(|v| v.id);
        for v in visualizers {
            let short_name = v
                .filename
                .strip_prefix("vis_")
                .unwrap_or(v.filename)
                .strip_suffix(".wgsl")
                .unwrap_or(v.filename);
            println!(
                "{:<4} {:<24} {:<28} {}",
                v.id, short_name, v.name, v.description
            );
        }
        return Ok(());
    }

    // Single-instance forwarding: if not in TUI mode and files were specified, attempt to forward to running instance
    if !args.tui
        && !args.file.is_empty()
        && crate::ipc::try_forward_to_existing_instance(&args.file)
    {
        println!("[ipc] Forwarded file(s) to existing RustTracker instance.");
        return Ok(());
    }

    // Expand directories and playlists (.m3u, .pls) into individual track paths
    args.file = crate::playlist::expand_input_paths(&args.file);

    let (ipc_tx, ipc_rx) = crossbeam_channel::unbounded::<Vec<String>>();
    if !args.tui {
        crate::ipc::start_ipc_server(ipc_tx.clone());
        #[cfg(target_os = "macos")]
        let _ = MACOS_OPEN_FILES_TX.set(ipc_tx.clone());
    }

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

        let mut history_changed = false;
        for path in &args.file {
            if path.starts_with("http") && !state.url_history.iter().any(|(u, _)| u == path) {
                state
                    .url_history
                    .push((path.clone(), "Network Stream".to_string()));
                history_changed = true;
            }
        }
        if history_changed {
            let mut out = String::new();
            for (u, t) in &state.url_history {
                out.push_str(&format!("{}|{}\n", u, t));
            }
            let _ = std::fs::write(crate::state::get_history_file_path(), out);
        }

        if let Some(vis) = &args.vis {
            let vis_lower = vis.to_lowercase();
            let normalize = |s: &str| -> String {
                s.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
                    .to_lowercase()
            };
            let vis_norm = normalize(&vis_lower);
            let matched_idx = crate::state::VISUALIZERS
                .iter()
                .enumerate()
                .position(|(i, v)| {
                    let short_name = v
                        .filename
                        .strip_prefix("vis_")
                        .unwrap_or(v.filename)
                        .strip_suffix(".wgsl")
                        .unwrap_or(v.filename)
                        .to_lowercase();
                    let name_norm = normalize(v.name);
                    let short_norm = normalize(&short_name);

                    v.id.to_string() == vis_lower
                        || i.to_string() == vis_lower
                        || v.name.to_lowercase() == vis_lower
                        || short_name == vis_lower
                        || name_norm == vis_norm
                        || short_norm == vis_norm
                        || (v.id == 5
                            && (vis_norm == "retrofire"
                                || vis_norm == "retro_fire"
                                || vis_norm == "flame"
                                || vis_norm == "fire"
                                || vis_norm == "doom"
                                || vis_norm == "doomfire"
                                || vis_norm == "doom_fire"
                                || vis_norm == "psxfire"))
                        || (v.id == 6 && (vis_norm == "firesim" || vis_norm == "firesimulation"))
                });

            if let Some(idx) = matched_idx {
                state.current_visualizer_idx = idx;
                state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                state.vis_enabled[idx] = true;
            } else {
                eprintln!(
                    "Warning: Visualizer '{}' not found. Launching with default visualizer.",
                    vis
                );
            }
        }
    }

    let file_path = args.file.first().cloned().unwrap_or_default();
    let initial_stream = if !file_path.is_empty() || args.mic {
        let lyrics = if args.mic || file_path.is_empty() {
            None
        } else {
            crate::lyrics::load_lyrics_for_file(&file_path).map(std::sync::Arc::new)
        };
        match audio::start_audio_thread(&file_path, args.mic, Arc::clone(&app_state)) {
            Ok(stream) => {
                let mut state = app_state.lock().unwrap();
                state.file_loaded = true;
                state.lyrics = lyrics;
                Some(stream)
            }
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
            let _ = execute!(
                io::stdout(),
                LeaveAlternateScreen,
                DisableMouseCapture,
                crossterm::cursor::Show
            );
            original_hook(panic_info);
        }));

        let mut tui = Tui::new()?;
        if let Err(err) = run_tui(&mut tui.terminal, app_state, initial_stream, args.mic) {
            eprintln!("App error: {:?}", err);
        }
    } else {
        pollster::block_on(run_gui(args, app_state, initial_stream, ipc_rx))?;
    }

    Ok(())
}

struct DesktopRuntime {
    egui_state: egui_winit::State,
    engine: VulkanEngine,
    window: Arc<winit::window::Window>,
}

#[cfg(target_os = "macos")]
static MACOS_OPEN_FILES_TX: std::sync::OnceLock<crossbeam_channel::Sender<Vec<String>>> =
    std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn macos_application_open_files(
    _this: *mut objc2::runtime::AnyObject,
    _cmd: objc2::runtime::Sel,
    sender: *mut objc2::runtime::AnyObject,
    filenames: *mut objc2::runtime::AnyObject,
) {
    use objc2::msg_send;
    unsafe {
        if !filenames.is_null() {
            let count: usize = msg_send![filenames, count];
            let mut paths = Vec::with_capacity(count);
            for i in 0..count {
                let ns_str: *mut objc2::runtime::AnyObject = msg_send![filenames, objectAtIndex: i];
                if !ns_str.is_null() {
                    let utf8: *const std::ffi::c_char = msg_send![ns_str, UTF8String];
                    if !utf8.is_null() {
                        let path = std::ffi::CStr::from_ptr(utf8)
                            .to_string_lossy()
                            .into_owned();
                        if !path.is_empty() {
                            paths.push(path);
                        }
                    }
                }
            }
            if !paths.is_empty()
                && let Some(tx) = MACOS_OPEN_FILES_TX.get()
            {
                let _ = tx.send(paths);
            }
        }
        if !sender.is_null() {
            // NSApplicationDelegateReplySuccess = 0
            let _: () = msg_send![sender, replyToOpenOrPrint: 0usize];
        }
    }
}

#[cfg(target_os = "macos")]
fn register_macos_open_files_handler() {
    use objc2::runtime::{AnyClass, Imp};
    use objc2::sel;
    if let Some(cls) = AnyClass::get(c"WinitApplicationDelegate") {
        unsafe {
            let imp: Imp = std::mem::transmute(
                macos_application_open_files
                    as unsafe extern "C-unwind" fn(
                        *mut objc2::runtime::AnyObject,
                        objc2::runtime::Sel,
                        *mut objc2::runtime::AnyObject,
                        *mut objc2::runtime::AnyObject,
                    ),
            );
            let _ = objc2::ffi::class_addMethod(
                cls as *const AnyClass as *mut AnyClass,
                sel!(application:openFiles:),
                imp,
                c"v@:@@".as_ptr(),
            );
        }
    }
}

struct DesktopApp<InitFn, EventFn> {
    runtime: Option<DesktopRuntime>,
    init_error: Option<anyhow::Error>,
    init_fn: Option<InitFn>,
    event_fn: EventFn,
}

impl<InitFn, EventFn> winit::application::ApplicationHandler for DesktopApp<InitFn, EventFn>
where
    InitFn: FnOnce(&winit::event_loop::ActiveEventLoop) -> anyhow::Result<DesktopRuntime>,
    EventFn: FnMut(&mut DesktopRuntime, Event<()>, &winit::event_loop::ActiveEventLoop),
{
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.runtime.is_none()
            && let Some(init) = self.init_fn.take()
        {
            match init(event_loop) {
                Ok(rt) => {
                    event_loop.set_control_flow(ControlFlow::Poll);
                    self.runtime = Some(rt);
                }
                Err(e) => {
                    self.init_error = Some(e);
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Some(ref mut rt) = self.runtime {
            (self.event_fn)(rt, Event::WindowEvent { window_id, event }, event_loop);
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(ref mut rt) = self.runtime {
            (self.event_fn)(rt, Event::AboutToWait, event_loop);
        }
    }

    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        let _ = self.runtime.take();
    }
}

/// Computes clamped initial inner logical size, minimum inner logical size, and centered outer
/// physical position so that the window (including titlebar/borders and OS taskbar reserve)
/// never exceeds the monitor's display area or draws outside the display area on load.
fn clamp_window_geometry_to_monitor(
    mon_phys_w: u32,
    mon_phys_h: u32,
    mon_pos_x: i32,
    mon_pos_y: i32,
    scale: f64,
    desired_logical_w: f64,
    desired_logical_h: f64,
) -> ((f64, f64), (f64, f64), (i32, i32)) {
    let scale = scale.max(1.0);
    let mon_logical_w = mon_phys_w as f64 / scale;
    let mon_logical_h = mon_phys_h as f64 / scale;

    // Reserve logical margin for window decorations (borders + titlebar) and OS taskbar/dock
    let max_inner_w = (mon_logical_w - 48.0).min(mon_logical_w * 0.94).max(320.0);
    let max_inner_h = (mon_logical_h - 96.0).min(mon_logical_h * 0.88).max(240.0);

    let target_w = desired_logical_w.min(max_inner_w).floor();
    let target_h = desired_logical_h.min(max_inner_h).floor();
    let min_w = 1024.0_f64.min(target_w);
    let min_h = 600.0_f64.min(target_h);

    let est_outer_w = ((target_w + 16.0) * scale).round() as i32;
    let est_outer_h = ((target_h + 38.0) * scale).round() as i32;
    let usable_phys_w = mon_phys_w as i32;
    let taskbar_phys = (48.0 * scale).round() as i32;
    let usable_phys_h = (mon_phys_h as i32 - taskbar_phys).max(est_outer_h);

    let pos_x = mon_pos_x + ((usable_phys_w - est_outer_w).max(0) / 2);
    let pos_y = mon_pos_y + ((usable_phys_h - est_outer_h).max(0) / 2);

    ((target_w, target_h), (min_w, min_h), (pos_x, pos_y))
}

fn compute_initial_window_geometry(
    monitor: Option<winit::monitor::MonitorHandle>,
    desired_logical_w: f64,
    desired_logical_h: f64,
) -> (
    winit::dpi::LogicalSize<f64>,
    winit::dpi::LogicalSize<f64>,
    Option<winit::dpi::PhysicalPosition<i32>>,
) {
    if let Some(mon) = monitor {
        let mon_phys = mon.size();
        if mon_phys.width > 0 && mon_phys.height > 0 {
            let mon_pos = mon.position();
            let scale = mon.scale_factor();
            let ((target_w, target_h), (min_w, min_h), (pos_x, pos_y)) =
                clamp_window_geometry_to_monitor(
                    mon_phys.width,
                    mon_phys.height,
                    mon_pos.x,
                    mon_pos.y,
                    scale,
                    desired_logical_w,
                    desired_logical_h,
                );
            return (
                winit::dpi::LogicalSize::new(target_w, target_h),
                winit::dpi::LogicalSize::new(min_w, min_h),
                Some(winit::dpi::PhysicalPosition::new(pos_x, pos_y)),
            );
        }
    }
    (
        winit::dpi::LogicalSize::new(desired_logical_w, desired_logical_h),
        winit::dpi::LogicalSize::new(
            1024.0_f64.min(desired_logical_w),
            600.0_f64.min(desired_logical_h),
        ),
        None,
    )
}

fn constrain_window_to_monitor(
    window: &winit::window::Window,
    desired_logical_w: Option<f64>,
    desired_logical_h: Option<f64>,
    center: bool,
) {
    let Some(mon) = window
        .current_monitor()
        .or_else(|| window.primary_monitor())
        .or_else(|| window.available_monitors().next())
    else {
        return;
    };

    let scale = window.scale_factor().max(1.0);
    let mon_phys = mon.size();
    let mon_pos = mon.position();
    if mon_phys.width == 0 || mon_phys.height == 0 {
        return;
    }

    let taskbar_margin_phys = (48.0 * scale).round() as u32;
    let avail_w = mon_phys.width;
    let avail_h = mon_phys.height.saturating_sub(taskbar_margin_phys).max(240);

    let mon_logical_w = mon_phys.width as f64 / scale;
    let mon_logical_h = mon_phys.height as f64 / scale;
    let safe_max_logical_w = (mon_logical_w - 48.0).min(mon_logical_w * 0.94).max(320.0);
    let safe_max_logical_h = (mon_logical_h - 96.0).min(mon_logical_h * 0.88).max(240.0);

    let outer = window.outer_size();
    let inner = window.inner_size();
    let frame_w = outer.width.saturating_sub(inner.width);
    let frame_h = outer.height.saturating_sub(inner.height);

    let max_inner_phys_w = avail_w
        .saturating_sub(frame_w)
        .min((safe_max_logical_w.floor() * scale).round() as u32)
        .max(320);
    let max_inner_phys_h = avail_h
        .saturating_sub(frame_h)
        .min((safe_max_logical_h.floor() * scale).round() as u32)
        .max(240);

    let want_phys_w = desired_logical_w
        .map(|w| (w * scale).round() as u32)
        .unwrap_or(inner.width);
    let want_phys_h = desired_logical_h
        .map(|h| (h * scale).round() as u32)
        .unwrap_or(inner.height);

    let new_inner_w = want_phys_w.min(max_inner_phys_w);
    let new_inner_h = want_phys_h.min(max_inner_phys_h);

    let min_logical_w = 1024.0_f64.min(new_inner_w as f64 / scale);
    let min_logical_h = 600.0_f64.min(new_inner_h as f64 / scale);
    window.set_min_inner_size(Some(winit::dpi::LogicalSize::new(
        min_logical_w,
        min_logical_h,
    )));

    if new_inner_w != inner.width || new_inner_h != inner.height {
        let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(new_inner_w, new_inner_h));
    }

    let final_outer_w = (new_inner_w + frame_w) as i32;
    let final_outer_h = (new_inner_h + frame_h) as i32;
    let min_x = mon_pos.x;
    let min_y = mon_pos.y;
    let max_x = mon_pos.x + avail_w as i32;
    let max_y = mon_pos.y + avail_h as i32;

    if center {
        let pos_x = min_x + ((avail_w as i32 - final_outer_w).max(0) / 2);
        let pos_y = min_y + ((avail_h as i32 - final_outer_h).max(0) / 2);
        window.set_outer_position(winit::dpi::PhysicalPosition::new(pos_x, pos_y));
    } else if let Ok(pos) = window.outer_position() {
        let clamped_x = pos.x.min(max_x - final_outer_w).max(min_x);
        let clamped_y = pos.y.min(max_y - final_outer_h).max(min_y);
        if clamped_x != pos.x || clamped_y != pos.y {
            window.set_outer_position(winit::dpi::PhysicalPosition::new(clamped_x, clamped_y));
        }
    }
}

async fn run_gui(
    args: Args,
    app_state: Arc<Mutex<AppState>>,
    mut active_stream: Option<audio::PlaybackHandle>,
    ipc_rx: crossbeam_channel::Receiver<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let is_fullscreen = args.fullscreen;
    let use_gpu_fft = args.gpu_fft;
    let bench = args.bench;
    let is_mic_launch = args.mic;
    let is_uncapped = args.uncapped;

    if use_gpu_fft {
        let mut state = app_state.lock().unwrap();
        state.gpu_fft = true;
    }
    if let Some(bf) = bench {
        crate::state::BENCH_FRAMES_OVERRIDE.store(bf, std::sync::atomic::Ordering::Relaxed);
    }

    let mut event_loop_builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        if std::env::var("RUSTTRACKER_X11").is_ok() {
            event_loop_builder.with_x11();
        }
    }
    let event_loop = match event_loop_builder.build() {
        Ok(el) => el,
        Err(err) => {
            #[cfg(target_os = "linux")]
            {
                eprintln!(
                    "[warn] Failed to initialize display backend ({:?}). Retrying default display backend...",
                    err
                );
                EventLoop::builder().build().map_err(|e| {
                    anyhow::anyhow!("Failed to initialize GUI display/window event loop: {:?}. Make sure a graphical display (Wayland or X11) is available, or use a virtual display (e.g. xvfb-run).", e)
                })?
            }
            #[cfg(not(target_os = "linux"))]
            {
                return Err(
                    anyhow::anyhow!("Failed to initialize window event loop: {:?}", err).into(),
                );
            }
        }
    };

    #[cfg(target_os = "macos")]
    register_macos_open_files_handler();

    let (icon_rgba, icon_width, icon_height) = {
        let image = image::load_from_memory(include_bytes!("../icon.png"))
            .expect("Failed to load icon")
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };
    let window_icon = winit::window::Icon::from_rgba(icon_rgba, icon_width, icon_height).unwrap();

    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        use windows::core::w;
        let _ = SetCurrentProcessExplicitAppUserModelID(w!("Soddentrough.RustTracker"));
    }

    let is_steam_deck = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
        .map(|s| s.trim().to_lowercase().contains("valve"))
        .unwrap_or(false);
    let is_game_mode = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase()
        == "gamescope"
        || std::env::var("XDG_SESSION_DESKTOP")
            .unwrap_or_default()
            .to_lowercase()
            == "gamescope"
        || std::env::var("STEAM_DECK").is_ok()
        || is_steam_deck;

    let (desired_logical_w, desired_logical_h) = if is_game_mode {
        (1280.0, 800.0)
    } else {
        (1440.0, 840.0)
    };
    let initial_size = winit::dpi::LogicalSize::new(desired_logical_w, desired_logical_h);

    let mut attrs = winit::window::Window::default_attributes()
        .with_title("RustTracker Vulkan Visualizer")
        .with_theme(Some(winit::window::Theme::Dark))
        .with_visible(false)
        .with_inner_size(initial_size)
        .with_min_inner_size(winit::dpi::LogicalSize::new(1024.0, 600.0))
        .with_window_icon(Some(window_icon));

    if is_fullscreen {
        attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
    }

    #[cfg(target_os = "linux")]
    {
        attrs = WindowAttributesExtWayland::with_name(attrs, "rusttracker", "rusttracker");
        attrs = WindowAttributesExtX11::with_name(attrs, "rusttracker", "rusttracker");
    }

    let start_time = Instant::now();
    let mut last_update = Instant::now();

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
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .push("kenney_icons".to_owned());
    egui_ctx.set_fonts(fonts);

    let egui_ctx_for_init = egui_ctx.clone();
    let app_state_for_init = Arc::clone(&app_state);
    let init_fn =
        move |elwt: &winit::event_loop::ActiveEventLoop| -> anyhow::Result<DesktopRuntime> {
            let mut attrs = attrs;
            let mut had_monitor_geometry = false;
            if !is_fullscreen {
                let monitor = elwt
                    .primary_monitor()
                    .or_else(|| elwt.available_monitors().next());
                let (init_size, min_size, init_pos) =
                    compute_initial_window_geometry(monitor, desired_logical_w, desired_logical_h);
                attrs = attrs
                    .with_inner_size(init_size)
                    .with_min_inner_size(min_size);
                if let Some(pos) = init_pos {
                    attrs = attrs.with_position(pos);
                    had_monitor_geometry = true;
                }
            }
            let window = elwt.create_window(attrs).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create application window: {:?}. Please verify that a graphical session (Wayland or X11) is active.",
                e
            )
        })?;
            if !is_fullscreen && !had_monitor_geometry {
                constrain_window_to_monitor(
                    &window,
                    Some(desired_logical_w),
                    Some(desired_logical_h),
                    true,
                );
            }
            let window = Arc::new(window);
            let mut engine = pollster::block_on(VulkanEngine::new(window.clone()));
            let mut egui_state = egui_winit::State::new(
                egui_ctx_for_init.clone(),
                egui::ViewportId::ROOT,
                &window,
                None,
                None,
                None,
            );
            // Pre-render and present the initial frame while the window is still hidden
            // so the OS compositor never flashes an unpainted white window on startup.
            if std::env::var("CAPTURE_FRAME").is_err() {
                let state_snapshot = {
                    let mut state = app_state_for_init.lock().unwrap();
                    state.visual_width = engine.config.width / 2;
                    engine.update(&state, 1.0 / 60.0);
                    state.render_snapshot()
                };
                let mut init_dialog = egui_file_dialog::FileDialog::new();
                let _ = engine.render(
                    &window,
                    &egui_ctx_for_init,
                    &mut egui_state,
                    &state_snapshot,
                    &mut init_dialog,
                    Vec::new(),
                );
            }
            window.set_visible(true);
            window.request_redraw();
            Ok(DesktopRuntime {
                window,
                engine,
                egui_state,
            })
        };

    let mut last_mouse_move = Instant::now();
    let mut is_cursor_visible = true;
    let mut is_fullscreen = is_fullscreen; // Use the argument value instead of hardcoding false
    let mut keep_awake: Option<keepawake::KeepAwake> = None;
    if is_fullscreen {
        keep_awake = keepawake::Builder::default()
            .display(true)
            .idle(true)
            .create()
            .ok();
    }
    let mut is_first_frame = true;
    let mut last_mouse_scrub_seek = Instant::now();

    let mut gilrs =
        gilrs::Gilrs::new().unwrap_or_else(|_| gilrs::GilrsBuilder::new().build().unwrap());

    // is_game_mode already detected above

    #[cfg(windows)]
    let mut initial_dir = std::path::PathBuf::from(
        std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string()),
    );
    #[cfg(not(windows))]
    let mut initial_dir =
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));

    let mut file_dialog = egui_file_dialog::FileDialog::new()
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .initial_directory(initial_dir.clone())
        .add_file_filter_extensions(
            "Audio/Video Files",
            crate::playlist::FILE_DIALOG_EXTENSIONS.to_vec(),
        )
        .show_all_files_filter(true)
        .default_file_filter("Audio/Video Files");

    // Native file picker channel (used on non-SteamDeck systems to bypass
    // egui-file-dialog's synchronous sysinfo disk enumeration)
    let (rfd_tx, rfd_rx) = crossbeam_channel::unbounded::<Vec<String>>();
    let mut rfd_pending = false;

    let mut modifiers = winit::keyboard::ModifiersState::empty();
    let mut last_cursor_pos: Option<egui::Pos2> = None;
    let mut bench_frame_count = 0u32;

    let mut app = DesktopApp {
        runtime: None,
        init_error: None,
        init_fn: Some(init_fn),
        event_fn: move |rt: &mut DesktopRuntime,
                        event: Event<()>,
                        elwt: &winit::event_loop::ActiveEventLoop| {
            let DesktopRuntime {
                ref window,
                ref mut engine,
                ref mut egui_state,
            } = *rt;
            match event {
                Event::WindowEvent {
                    ref event,
                    window_id,
                } if window_id == window.id() => {
                    let response = egui_state.on_window_event(window, event);

                    if let WindowEvent::CursorMoved { position, .. } = event {
                        last_mouse_move = Instant::now();
                        let sf = window.scale_factor() as f32;
                        last_cursor_pos =
                            Some(egui::pos2(position.x as f32 / sf, position.y as f32 / sf));
                        if !is_cursor_visible {
                            window.set_cursor_visible(true);
                            is_cursor_visible = true;
                        }
                    }

                    if let WindowEvent::HoveredFile(path) = event {
                        let mut state = app_state.lock().unwrap();
                        state.hovered_file = Some(path.to_string_lossy().into_owned());
                        window.request_redraw();
                    } else if let WindowEvent::HoveredFileCancelled = event {
                        let mut state = app_state.lock().unwrap();
                        state.hovered_file = None;
                        window.request_redraw();
                    }

                    if let WindowEvent::ModifiersChanged(m) = &event {
                        modifiers = m.state();
                    }

                    let is_typing = {
                        let state = app_state.lock().unwrap();
                        state.is_url_dialog_open || state.is_file_picker_open
                    };

                    // Process global hotkeys unless user is actively typing in a text dialog
                    if !is_typing
                        && let WindowEvent::KeyboardInput {
                            event: kb_event, ..
                        } = &event
                        && kb_event.state == ElementState::Pressed
                        && let PhysicalKey::Code(keycode) = kb_event.physical_key
                    {
                        match keycode {
                            WinitKeyCode::ArrowRight
                            | WinitKeyCode::ArrowLeft
                            | WinitKeyCode::Comma
                            | WinitKeyCode::Period => {
                                let mut state = app_state.lock().unwrap();
                                let duration = state.duration_seconds.max(0.1);
                                let mut delta = 0.0;
                                let step = if modifiers.control_key() || modifiers.super_key() {
                                    60.0f64.min(duration).max(5.0)
                                } else if modifiers.shift_key() {
                                    30.0f64.min(duration).max(1.0)
                                } else {
                                    5.0f64.min(duration).max(0.5)
                                };

                                match keycode {
                                    WinitKeyCode::ArrowRight => {
                                        delta = step;
                                    }
                                    WinitKeyCode::ArrowLeft => {
                                        delta = -step;
                                    }
                                    WinitKeyCode::Comma => delta = -1.0 / 60.0, // ~1 frame at 60fps
                                    WinitKeyCode::Period => delta = 1.0 / 60.0,
                                    _ => {}
                                }

                                let target = (state.current_seconds + delta)
                                    .clamp(0.0, state.duration_seconds);
                                state.seek_request = Some(target);
                                state.current_seconds = target;
                                state.track_ended = false;
                                state.spectrum_history.clear();
                                for _ in 0..120 {
                                    state.spectrum_history.push_back(vec![0.0; 1024]);
                                }

                                let sign = if delta >= 0.0 { "+" } else { "" };
                                if delta.abs() < 0.1 {
                                    state.osd_text = Some(format!(
                                        "{}1 Frame",
                                        if delta > 0.0 { "+" } else { "-" }
                                    ));
                                } else {
                                    state.osd_text = Some(format!("{}{:.1}s", sign, delta));
                                }
                                state.osd_timer = 2.0;
                            }
                            _ => {
                                if !kb_event.repeat {
                                    match keycode {
                                        WinitKeyCode::Escape => {
                                            let picker_open =
                                                app_state.lock().unwrap().show_vis_picker;
                                            if picker_open {
                                                app_state.lock().unwrap().show_vis_picker = false;
                                            } else {
                                                elwt.exit();
                                            }
                                        }
                                        WinitKeyCode::KeyQ => {
                                            // Only quit on KeyQ if Ctrl or Cmd (Super) is held to prevent accidental quits
                                            if modifiers.control_key() || modifiers.super_key() {
                                                elwt.exit();
                                            }
                                        }
                                        WinitKeyCode::MediaPlayPause => {
                                            let mut state = app_state.lock().unwrap();
                                            state.is_paused = !state.is_paused;
                                            state.osd_text = Some(if state.is_paused {
                                                "Paused".to_string()
                                            } else {
                                                "Playing".to_string()
                                            });
                                            state.osd_timer = 2.0;
                                        }
                                        WinitKeyCode::MediaTrackNext => {
                                            let mut state = app_state.lock().unwrap();
                                            if !state.playlist.is_empty()
                                                && state.playlist_index + 1 < state.playlist.len()
                                            {
                                                state.playlist_index += 1;
                                                state.load_request = Some(
                                                    state.playlist[state.playlist_index].clone(),
                                                );
                                                state.is_paused = false;
                                                state.osd_text = Some("Next Track".to_string());
                                                state.osd_timer = 2.0;
                                            }
                                        }
                                        WinitKeyCode::MediaTrackPrevious => {
                                            let mut state = app_state.lock().unwrap();
                                            if !state.playlist.is_empty()
                                                && state.playlist_index > 0
                                            {
                                                state.playlist_index -= 1;
                                                state.load_request = Some(
                                                    state.playlist[state.playlist_index].clone(),
                                                );
                                                state.is_paused = false;
                                                state.osd_text = Some("Previous Track".to_string());
                                                state.osd_timer = 2.0;
                                            }
                                        }
                                        WinitKeyCode::MediaStop => {
                                            let mut state = app_state.lock().unwrap();
                                            state.is_paused = true;
                                            state.seek_request = Some(0.0);
                                            state.osd_text = Some("Stopped".to_string());
                                            state.osd_timer = 2.0;
                                        }
                                        WinitKeyCode::AudioVolumeUp => {
                                            let mut state = app_state.lock().unwrap();
                                            let num_tracks = state.audio_tracks.len().max(1);
                                            if state.audio_track_volumes.len() != num_tracks {
                                                state.audio_track_volumes = vec![1.0; num_tracks];
                                            }
                                            for vol in &mut state.audio_track_volumes {
                                                *vol = (*vol + 0.05).clamp(0.0, 1.0);
                                            }
                                            let current_vol = state
                                                .audio_track_volumes
                                                .first()
                                                .copied()
                                                .unwrap_or(1.0);
                                            let mix: Vec<(usize, f32)> = state
                                                .active_audio_tracks
                                                .iter()
                                                .map(|&idx| {
                                                    (
                                                        idx,
                                                        state
                                                            .audio_track_volumes
                                                            .get(idx)
                                                            .copied()
                                                            .unwrap_or(1.0),
                                                    )
                                                })
                                                .collect();
                                            state.audio_mix_request = Some(mix);
                                            state.osd_text = Some(format!(
                                                "Volume: {}%",
                                                (current_vol * 100.0).round() as u32
                                            ));
                                            state.osd_timer = 2.0;
                                        }
                                        WinitKeyCode::AudioVolumeDown => {
                                            let mut state = app_state.lock().unwrap();
                                            let num_tracks = state.audio_tracks.len().max(1);
                                            if state.audio_track_volumes.len() != num_tracks {
                                                state.audio_track_volumes = vec![1.0; num_tracks];
                                            }
                                            for vol in &mut state.audio_track_volumes {
                                                *vol = (*vol - 0.05).clamp(0.0, 1.0);
                                            }
                                            let current_vol = state
                                                .audio_track_volumes
                                                .first()
                                                .copied()
                                                .unwrap_or(1.0);
                                            let mix: Vec<(usize, f32)> = state
                                                .active_audio_tracks
                                                .iter()
                                                .map(|&idx| {
                                                    (
                                                        idx,
                                                        state
                                                            .audio_track_volumes
                                                            .get(idx)
                                                            .copied()
                                                            .unwrap_or(1.0),
                                                    )
                                                })
                                                .collect();
                                            state.audio_mix_request = Some(mix);
                                            state.osd_text = Some(format!(
                                                "Volume: {}%",
                                                (current_vol * 100.0).round() as u32
                                            ));
                                            state.osd_timer = 2.0;
                                        }
                                        WinitKeyCode::AudioVolumeMute => {
                                            let mut state = app_state.lock().unwrap();
                                            let num_tracks = state.audio_tracks.len().max(1);
                                            if state.audio_track_volumes.len() != num_tracks {
                                                state.audio_track_volumes = vec![1.0; num_tracks];
                                            }
                                            let is_currently_zero =
                                                state.audio_track_volumes.iter().all(|&v| v == 0.0);
                                            let new_vol = if is_currently_zero { 1.0 } else { 0.0 };
                                            state.audio_track_volumes.fill(new_vol);
                                            let mix: Vec<(usize, f32)> = state
                                                .active_audio_tracks
                                                .iter()
                                                .map(|&idx| {
                                                    (
                                                        idx,
                                                        state
                                                            .audio_track_volumes
                                                            .get(idx)
                                                            .copied()
                                                            .unwrap_or(1.0),
                                                    )
                                                })
                                                .collect();
                                            state.audio_mix_request = Some(mix);
                                            state.osd_text = Some(if new_vol == 0.0 {
                                                "Muted".to_string()
                                            } else {
                                                "Unmuted".to_string()
                                            });
                                            state.osd_timer = 2.0;
                                        }
                                        WinitKeyCode::BracketLeft => {
                                            let mut state = app_state.lock().unwrap();
                                            state.panel_split_ratio =
                                                (state.panel_split_ratio - 0.05).clamp(0.15, 0.85);
                                        }
                                        WinitKeyCode::BracketRight => {
                                            let mut state = app_state.lock().unwrap();
                                            state.panel_split_ratio =
                                                (state.panel_split_ratio + 0.05).clamp(0.15, 0.85);
                                        }
                                        WinitKeyCode::Tab => {
                                            let mut state = app_state.lock().unwrap();
                                            state.show_hud = !state.show_hud;
                                            state.osd_text = Some(format!(
                                                "HUD: {}",
                                                if state.show_hud { "Visible" } else { "Hidden" }
                                            ));
                                            state.osd_timer = 2.0;
                                        }
                                        WinitKeyCode::KeyF => {
                                            let currently_fullscreen =
                                                window.fullscreen().is_some();
                                            is_fullscreen = !currently_fullscreen;
                                            if is_fullscreen {
                                                window.set_fullscreen(Some(
                                                    winit::window::Fullscreen::Borderless(None),
                                                ));
                                                keep_awake = keepawake::Builder::default()
                                                    .display(true)
                                                    .idle(true)
                                                    .create()
                                                    .ok();
                                            } else {
                                                window.set_fullscreen(None);
                                                constrain_window_to_monitor(
                                                    window,
                                                    Some(desired_logical_w),
                                                    Some(desired_logical_h),
                                                    true,
                                                );
                                                keep_awake = None;
                                            }
                                        }
                                        WinitKeyCode::KeyG => {
                                            let mut state = app_state.lock().unwrap();
                                            state.gpu_fft = !state.gpu_fft;
                                        }
                                        WinitKeyCode::KeyC => {
                                            let mut state = app_state.lock().unwrap();
                                            if state.visualizer_mode == 20 {
                                                state.biolum_top_down = !state.biolum_top_down;
                                                state.osd_text = Some(format!(
                                                    "Camera: {}",
                                                    if state.biolum_top_down {
                                                        "Top-down"
                                                    } else {
                                                        "Perspective"
                                                    }
                                                ));
                                                state.osd_timer = 2.0;
                                            }
                                        }
                                        WinitKeyCode::KeyS => {
                                            let mut state = app_state.lock().unwrap();
                                            state.show_stats = !state.show_stats;
                                        }
                                        WinitKeyCode::KeyH => {
                                            let mut state = app_state.lock().unwrap();
                                            state.show_help = !state.show_help;
                                        }
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
                                                        .add_filter(
                                                            "Audio/Video Files",
                                                            crate::playlist::FILE_DIALOG_EXTENSIONS,
                                                        )
                                                        .add_filter("All Files", &["*"])
                                                        .pick_files();
                                                    if let Some(paths) = result {
                                                        let strings: Vec<String> = paths
                                                            .iter()
                                                            .map(|p| p.display().to_string())
                                                            .collect();
                                                        let _ = tx.send(strings);
                                                    } else {
                                                        let _ = tx.send(vec![]);
                                                    }
                                                });
                                            }
                                        }
                                        WinitKeyCode::KeyM => {
                                            let mut state = app_state.lock().unwrap();
                                            state.show_vis_picker = !state.show_vis_picker;
                                            if state.show_vis_picker {
                                                state.vis_picker_cursor =
                                                    state.current_visualizer_idx;
                                                state.vis_picker_scroll_to_cursor = true;
                                            }
                                        }
                                        WinitKeyCode::KeyV => {
                                            let mut state = app_state.lock().unwrap();
                                            if state.video_frame_rx.is_some()
                                                || state.has_video_stream
                                            {
                                                state.video_mode = (state.video_mode + 1) % 4;
                                                let mode_name = match state.video_mode {
                                                    0 => "Standard View",
                                                    1 => "Video in Track Info",
                                                    2 => "Video in Top Panel",
                                                    3 => "Full Screen Video",
                                                    _ => "Video",
                                                };
                                                state.osd_text =
                                                    Some(format!("View: {}", mode_name));
                                            } else {
                                                state.show_hud = !state.show_hud;
                                                state.video_mode = 0;
                                                let mode_name = if state.show_hud {
                                                    "Standard View"
                                                } else {
                                                    "Full Screen Visualizer"
                                                };
                                                state.osd_text =
                                                    Some(format!("View: {}", mode_name));
                                            }
                                            state.osd_timer = 2.0;
                                        }
                                        WinitKeyCode::Space => {
                                            let mut state = app_state.lock().unwrap();
                                            if state.show_vis_picker {
                                                // Toggle enable/disable for highlighted visualizer (idx 0 always enabled)
                                                let idx = state.vis_picker_cursor;
                                                if idx != 0 {
                                                    state.vis_enabled[idx] =
                                                        !state.vis_enabled[idx];
                                                }
                                            } else if state.track_ended
                                                || (state.current_seconds
                                                    >= state.duration_seconds - 0.1
                                                    && state.duration_seconds > 0.0)
                                            {
                                                state.track_ended = false;
                                                state.seek_request = Some(0.0);
                                                state.spectrum_history.clear();
                                                for _ in 0..120 {
                                                    state
                                                        .spectrum_history
                                                        .push_back(vec![0.0; 1024]);
                                                }
                                                state.is_paused = false;
                                            } else if state.playlist_index >= state.playlist.len()
                                                && !state.playlist.is_empty()
                                            {
                                                if state.duration_seconds == 0.0 {
                                                    state.playlist_index = state.playlist.len() - 1;
                                                } else {
                                                    state.playlist_index = 0;
                                                }
                                                state.load_request = Some(
                                                    state.playlist[state.playlist_index].clone(),
                                                );
                                                state.is_paused = false;
                                            } else {
                                                state.is_paused = !state.is_paused;
                                            }
                                        }
                                        WinitKeyCode::KeyN => {
                                            let mut state = app_state.lock().unwrap();
                                            if !state.playlist.is_empty()
                                                && state.playlist_index + 1 < state.playlist.len()
                                            {
                                                state.playlist_index += 1;
                                                state.load_request = Some(
                                                    state.playlist[state.playlist_index].clone(),
                                                );
                                                state.is_paused = false;
                                                state.osd_text = Some("Next Track".to_string());
                                                state.osd_timer = 2.0;
                                            }
                                        }
                                        WinitKeyCode::KeyP => {
                                            let mut state = app_state.lock().unwrap();
                                            if !state.playlist.is_empty()
                                                && state.playlist_index > 0
                                            {
                                                state.playlist_index -= 1;
                                                state.load_request = Some(
                                                    state.playlist[state.playlist_index].clone(),
                                                );
                                                state.is_paused = false;
                                                state.osd_text = Some("Previous Track".to_string());
                                                state.osd_timer = 2.0;
                                            }
                                        }
                                        WinitKeyCode::Enter => {
                                            let mut state = app_state.lock().unwrap();
                                            if state.show_vis_picker {
                                                state.current_visualizer_idx =
                                                    state.vis_picker_cursor;
                                                state.visualizer_mode = crate::state::VISUALIZERS
                                                    [state.vis_picker_cursor]
                                                    .id;
                                                state.osd_text = Some(
                                                    crate::state::VISUALIZERS
                                                        [state.vis_picker_cursor]
                                                        .name
                                                        .to_string(),
                                                );
                                                state.osd_timer = 2.0;
                                                state.show_vis_picker = false;
                                            }
                                        }
                                        WinitKeyCode::ArrowUp => {
                                            let mut state = app_state.lock().unwrap();
                                            if state.show_vis_picker {
                                                if state.vis_picker_cursor == 0 {
                                                    state.vis_picker_cursor =
                                                        crate::state::VISUALIZERS.len() - 1;
                                                } else {
                                                    state.vis_picker_cursor -= 1;
                                                }
                                                state.vis_picker_scroll_to_cursor = true;
                                            } else {
                                                // Cycle to next enabled visualizer
                                                let len = crate::state::VISUALIZERS.len();
                                                let mut idx = state.current_visualizer_idx;
                                                for _ in 0..len {
                                                    idx = (idx + 1) % len;
                                                    if state.vis_enabled[idx] {
                                                        break;
                                                    }
                                                }
                                                state.current_visualizer_idx = idx;
                                                state.visualizer_mode =
                                                    crate::state::VISUALIZERS[idx].id;
                                                state.osd_text = Some(
                                                    crate::state::VISUALIZERS[idx].name.to_string(),
                                                );
                                                state.osd_timer = 2.0;
                                            }
                                        }
                                        WinitKeyCode::ArrowDown => {
                                            let mut state = app_state.lock().unwrap();
                                            if state.show_vis_picker {
                                                state.vis_picker_cursor = (state.vis_picker_cursor
                                                    + 1)
                                                    % crate::state::VISUALIZERS.len();
                                                state.vis_picker_scroll_to_cursor = true;
                                            } else {
                                                // Cycle to previous enabled visualizer
                                                let len = crate::state::VISUALIZERS.len();
                                                let mut idx = state.current_visualizer_idx;
                                                for _ in 0..len {
                                                    idx = if idx == 0 { len - 1 } else { idx - 1 };
                                                    if state.vis_enabled[idx] {
                                                        break;
                                                    }
                                                }
                                                state.current_visualizer_idx = idx;
                                                state.visualizer_mode =
                                                    crate::state::VISUALIZERS[idx].id;
                                                state.osd_text = Some(
                                                    crate::state::VISUALIZERS[idx].name.to_string(),
                                                );
                                                state.osd_timer = 2.0;
                                            }
                                        }
                                        WinitKeyCode::Digit1
                                        | WinitKeyCode::Numpad1
                                        | WinitKeyCode::Digit2
                                        | WinitKeyCode::Numpad2
                                        | WinitKeyCode::Digit3
                                        | WinitKeyCode::Numpad3
                                        | WinitKeyCode::Digit4
                                        | WinitKeyCode::Numpad4
                                        | WinitKeyCode::Digit5
                                        | WinitKeyCode::Numpad5
                                        | WinitKeyCode::Digit6
                                        | WinitKeyCode::Numpad6
                                        | WinitKeyCode::Digit7
                                        | WinitKeyCode::Numpad7
                                        | WinitKeyCode::Digit8
                                        | WinitKeyCode::Numpad8
                                        | WinitKeyCode::Digit9
                                        | WinitKeyCode::Numpad9 => {
                                            let track_num = match keycode {
                                                WinitKeyCode::Digit1 | WinitKeyCode::Numpad1 => 1,
                                                WinitKeyCode::Digit2 | WinitKeyCode::Numpad2 => 2,
                                                WinitKeyCode::Digit3 | WinitKeyCode::Numpad3 => 3,
                                                WinitKeyCode::Digit4 | WinitKeyCode::Numpad4 => 4,
                                                WinitKeyCode::Digit5 | WinitKeyCode::Numpad5 => 5,
                                                WinitKeyCode::Digit6 | WinitKeyCode::Numpad6 => 6,
                                                WinitKeyCode::Digit7 | WinitKeyCode::Numpad7 => 7,
                                                WinitKeyCode::Digit8 | WinitKeyCode::Numpad8 => 8,
                                                WinitKeyCode::Digit9 | WinitKeyCode::Numpad9 => 9,
                                                _ => 0,
                                            };
                                            if track_num > 0 {
                                                let mut state = app_state.lock().unwrap();
                                                let num_tracks = state.audio_tracks.len();
                                                if num_tracks > 1 {
                                                    if track_num <= num_tracks {
                                                        let track_idx = track_num - 1;
                                                        if state.active_audio_tracks
                                                            != vec![track_idx]
                                                            || state.selected_audio_track
                                                                != track_idx
                                                        {
                                                            state.selected_audio_track = track_idx;
                                                            state.active_audio_tracks =
                                                                vec![track_idx];
                                                            if state.audio_track_volumes.len()
                                                                != num_tracks
                                                            {
                                                                state.audio_track_volumes =
                                                                    vec![1.0; num_tracks];
                                                            }
                                                            state.audio_track_request =
                                                                Some(track_idx);
                                                            state.audio_mix_request =
                                                                Some(vec![(track_idx, 1.0)]);
                                                        }
                                                    } else if track_num == num_tracks + 1 {
                                                        // Shortcut for mixing tracks: <number of tracks + 1>
                                                        // E.g. for a 2-track file, "3" plays the mix of all tracks!
                                                        if state.audio_track_volumes.len()
                                                            != num_tracks
                                                        {
                                                            state.audio_track_volumes =
                                                                vec![1.0; num_tracks];
                                                        }
                                                        let mix: Vec<(usize, f32)> = (0
                                                            ..num_tracks)
                                                            .map(|idx| {
                                                                (
                                                                    idx,
                                                                    state
                                                                        .audio_track_volumes
                                                                        .get(idx)
                                                                        .copied()
                                                                        .unwrap_or(1.0),
                                                                )
                                                            })
                                                            .collect();
                                                        state.active_audio_tracks =
                                                            (0..num_tracks).collect();
                                                        state.audio_mix_request = Some(mix);
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }

                    if let WindowEvent::DroppedFile(path) = &event {
                        let mut state = app_state.lock().unwrap();
                        state.hovered_file = None;
                        let path_str = crate::lyrics::normalize_path_str(&path.to_string_lossy());
                        let expanded = crate::playlist::expand_input_paths(&[path_str]);
                        if !expanded.is_empty() {
                            let is_over_ti = if let (Some(c_pos), Some(ti_rect)) =
                                (last_cursor_pos, state.track_info_rect)
                            {
                                c_pos.x >= ti_rect[0]
                                    && c_pos.x <= ti_rect[2]
                                    && c_pos.y >= ti_rect[1]
                                    && c_pos.y <= ti_rect[3]
                            } else {
                                false
                            };

                            if !state.file_loaded || state.playlist.is_empty() {
                                // Use case 1: Splash screen -> Play immediately
                                state.playlist = expanded.clone();
                                state.playlist_index = 0;
                                state.load_request = Some(state.playlist[0].clone());
                                state.file_loaded = true;
                            } else if is_over_ti {
                                // Use case 3: Track Info pane -> Add to playlist
                                state.playlist.extend(expanded.clone());
                                let file_name = std::path::Path::new(&expanded[0])
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                if expanded.len() == 1 {
                                    state.osd_text =
                                        Some(format!("Added to Playlist: {}", file_name));
                                } else {
                                    state.osd_text = Some(format!(
                                        "Added {} tracks to Playlist",
                                        expanded.len()
                                    ));
                                }
                                state.osd_timer = 3.0;
                            } else {
                                // Use case 2: Main player view (outside Track Info) -> Play immediately
                                state.playlist = expanded.clone();
                                state.playlist_index = 0;
                                state.load_request = Some(state.playlist[0].clone());
                                state.file_loaded = true;
                            }
                        }
                        window.request_redraw();
                    }

                    if response.consumed {
                        return;
                    }

                    match event {
                        WindowEvent::CloseRequested => {
                            elwt.exit();
                        }
                        WindowEvent::ScaleFactorChanged { .. } => {
                            engine.resize(window.inner_size());
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

                                if state.audio_device_lost {
                                    state.audio_device_lost = false;
                                    let reload_path = if is_mic_launch {
                                        "".to_string()
                                    } else if state.playlist_index < state.playlist.len() {
                                        state.playlist[state.playlist_index].clone()
                                    } else {
                                        state.song_title.clone()
                                    };

                                    if !reload_path.is_empty() || is_mic_launch {
                                        state.load_request = Some(reload_path);
                                        state.osd_text =
                                            Some("Audio Device Reconnected".to_string());
                                        state.osd_timer = 3.0;
                                    }
                                }

                                let device_change = state.audio_device_change_request.take();
                                if let Some(new_device) = device_change {
                                    let reload_path = if is_mic_launch {
                                        "".to_string()
                                    } else if state.playlist_index < state.playlist.len() {
                                        state.playlist[state.playlist_index].clone()
                                    } else {
                                        state.song_title.clone()
                                    };

                                    if !reload_path.is_empty() || is_mic_launch {
                                        state.load_request = Some(reload_path);
                                        state.osd_text =
                                            Some(format!("Audio Output: {}", new_device));
                                        state.osd_timer = 3.0;
                                    }
                                }

                                if state.track_ended && state.load_request.is_none() {
                                    if state.playlist_index + 1 < state.playlist.len() {
                                        state.track_ended = false;
                                        state.playlist_index += 1;
                                        state.load_request =
                                            Some(state.playlist[state.playlist_index].clone());
                                    } else {
                                        state.is_paused = true;
                                    }
                                }

                                state.load_request.take()
                            };

                            if let Some(path) = load_path {
                                let is_mic = is_mic_launch && path.is_empty();
                                // Check if the path exists first!
                                let path_exists = if path.starts_with("http") || is_mic {
                                    true
                                } else {
                                    std::path::Path::new(&path).exists()
                                };

                                if !path_exists {
                                    let mut state = app_state.lock().unwrap();
                                    state.osd_text = Some(format!(
                                        "File Not Found: {}",
                                        std::path::Path::new(&path)
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                    ));
                                    state.osd_timer = 3.0;

                                    state.playlist_index += 1;
                                    if state.playlist_index < state.playlist.len() {
                                        state.load_request =
                                            Some(state.playlist[state.playlist_index].clone());
                                    } else {
                                        if state.file_loaded {
                                            if let Some(idx) = state
                                                .playlist
                                                .iter()
                                                .position(|p| p == &state.song_title)
                                            {
                                                state.playlist_index = idx;
                                            } else {
                                                state.playlist_index =
                                                    state.playlist_index.saturating_sub(1);
                                            }
                                        } else {
                                            state.file_loaded = false;
                                            state.artist = "Load Failed".to_string();
                                        }
                                    }
                                } else {
                                    let _ = active_stream.take(); // DROP OLD STREAM FIRST to release WASAPI lock!

                                    // Cleanup old video state completely before loading the next file
                                    engine.clear_video_state();
                                    {
                                        let mut state = app_state.lock().unwrap();
                                        state.video_frame_rx = None;
                                        state.free_video_frame_tx = None;
                                        state.has_video_stream = false;
                                        state.video_info = None;
                                        state.video_mode = 0;
                                        state.stats.bitstream_active = false;
                                        state.stats.audio_buffer_fill_pct = 0.0;
                                        state.stats.video_buffer_fill_pct = 0.0;
                                        state.stats.video_hw_decoder.clear();
                                        state.stats.video_fps = 0.0;
                                        state.stats.video_bitrate_kbps = None;
                                        state.stats.audio_bitrate_kbps = None;
                                        state.track_ended = false;
                                    }

                                    // We rely entirely on DSP thread messages to update tracker string state
                                    let mut loaded_stream = None;
                                    let mut last_err = None;
                                    for _ in 0..5 {
                                        match audio::start_audio_thread(
                                            &path,
                                            is_mic,
                                            Arc::clone(&app_state),
                                        ) {
                                            Ok(stream) => {
                                                loaded_stream = Some(stream);
                                                break;
                                            }
                                            Err(e) => {
                                                last_err = Some(e);
                                            }
                                        }
                                        std::thread::sleep(std::time::Duration::from_millis(50));
                                    }

                                    if let Some(stream) = loaded_stream {
                                        let mut state = app_state.lock().unwrap();
                                        state.file_loaded = true;
                                        state.song_title = if is_mic {
                                            "Microphone Input".to_string()
                                        } else {
                                            path.clone()
                                        };
                                        state.lyrics = if is_mic {
                                            None
                                        } else {
                                            crate::lyrics::load_lyrics_for_file(&path)
                                                .map(std::sync::Arc::new)
                                        };
                                        state.track_ended = false;
                                        active_stream = Some(stream);
                                    } else {
                                        let mut state = app_state.lock().unwrap();
                                        state.lyrics = None;
                                        let err_msg = last_err
                                            .map(|e| format!("{:?}", e))
                                            .unwrap_or_else(|| "Unknown error".to_string());
                                        let file_name = if is_mic {
                                            "Microphone".to_string()
                                        } else {
                                            std::path::Path::new(&path)
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .into_owned()
                                        };
                                        state.osd_text = Some(format!(
                                            "Load Failed: {}\n{}",
                                            file_name, err_msg
                                        ));
                                        state.osd_timer = 5.0;

                                        if !is_mic {
                                            state.playlist_index += 1;
                                            if state.playlist_index < state.playlist.len() {
                                                state.load_request = Some(
                                                    state.playlist[state.playlist_index].clone(),
                                                );
                                            } else {
                                                state.file_loaded = false;
                                                state.artist = "Load Failed".to_string();
                                            }
                                        } else {
                                            state.file_loaded = false;
                                            state.artist = "Mic Failed".to_string();
                                        }
                                    }
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
                                    let t = start_time.elapsed().as_secs_f32();
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
                                        state.channel_vus[i] = (state.channel_vus[i]
                                            - (0.015 * time_scale))
                                            .max(state.raw_channel_vus[i]);
                                    }
                                }

                                if state.peak_vus.len() != state.channel_vus.len() {
                                    state.peak_vus = vec![0.0; state.channel_vus.len()];
                                }
                                for i in 0..state.channel_vus.len() {
                                    state.peak_vus[i] =
                                        (state.peak_vus[i] - (0.005 * time_scale)).max(0.0);
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
                                        state.spectrum_data[i] = (state.spectrum_data[i]
                                            - (1.5 * time_scale))
                                            .max(state.raw_spectrum_data[i]);
                                    }

                                    // Smoothly decay fire heat using display refresh rate
                                    if state.raw_spectrum_data[i] > state.fire_heat[i] {
                                        state.fire_heat[i] = state.raw_spectrum_data[i];
                                    } else {
                                        state.fire_heat[i] =
                                            (state.fire_heat[i] - (1.5 * time_scale)).max(0.0);
                                    }
                                }

                                if state.spectrum_peaks.len() != state.spectrum_data.len() {
                                    state.spectrum_peaks = vec![0.0; state.spectrum_data.len()];
                                }
                                for i in 0..state.spectrum_data.len() {
                                    state.spectrum_peaks[i] =
                                        (state.spectrum_peaks[i] - (0.5 * time_scale)).max(0.0);
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

                                // Gamepad analog panel scaling and scrubbing
                                for (_id, gamepad) in gilrs.gamepads() {
                                    let right_y = gamepad.value(gilrs::Axis::RightStickY);
                                    if right_y.abs() > 0.1 {
                                        let scaled_delta = right_y * dt * 1.0; // Subtract moving UP (positive Y stick)
                                        state.panel_split_ratio = (state.panel_split_ratio
                                            - scaled_delta)
                                            .clamp(0.15, 0.85);
                                    }

                                    // Analog trigger scrubbing
                                    let lt = gamepad
                                        .button_data(gilrs::Button::LeftTrigger2)
                                        .map(|d| d.value())
                                        .unwrap_or(0.0);
                                    let rt = gamepad
                                        .button_data(gilrs::Button::RightTrigger2)
                                        .map(|d| d.value())
                                        .unwrap_or(0.0);
                                    let scrub_delta = rt - lt;
                                    if scrub_delta.abs() > 0.05 {
                                        let scrub_amount = (scrub_delta * dt) as f64
                                            * (state.duration_seconds * 0.1).clamp(2.0, 30.0);
                                        let target = (state.current_seconds + scrub_amount)
                                            .clamp(0.0, state.duration_seconds);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }

                                        let sign = if scrub_delta > 0.0 { "+" } else { "" };
                                        state.osd_text = Some(format!(
                                            "Scrubbing {}{:.1}x",
                                            sign,
                                            scrub_delta * 10.0
                                        ));
                                        state.osd_timer = 0.5;
                                    }
                                }

                                if state.osd_timer > 0.0 {
                                    state.osd_timer = (state.osd_timer - dt).max(0.0);
                                    if state.osd_timer == 0.0 {
                                        state.osd_text = None;
                                    }
                                }

                                // Clamp panel split ratio to ensure both top and bottom panels have at least 220px height
                                let total_h =
                                    engine.config.height as f32 / window.scale_factor() as f32;
                                if total_h > 0.0 {
                                    let min_h = 220.0f32;
                                    let min_r = (min_h / total_h).clamp(0.0, 1.0);
                                    let max_r = ((total_h - min_h) / total_h).clamp(0.0, 1.0);
                                    state.panel_split_ratio = state
                                        .panel_split_ratio
                                        .clamp(min_r.min(max_r), min_r.max(max_r));
                                }

                                engine.update(&state, dt);
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

                            // Update power management / idle sleep inhibition
                            let is_playing = !state_copy.is_paused
                                && state_copy.file_loaded
                                && !state_copy.track_ended;
                            let needs_keepawake = is_playing || is_fullscreen;
                            if needs_keepawake && keep_awake.is_none() {
                                keep_awake = keepawake::Builder::default()
                                    .display(
                                        is_fullscreen
                                            || state_copy.visualizer_mode != 0
                                            || state_copy.video_frame_rx.is_some(),
                                    )
                                    .idle(true)
                                    .create()
                                    .ok();
                            } else if !needs_keepawake && keep_awake.is_some() {
                                keep_awake = None;
                            }

                            let mut action = EngineAction::None;
                            let mut ui_time = 0.0;
                            let mut render_time = 0.0;
                            let mut fire_time = None;
                            let mut fft_time = None;
                            let mut vis_shader_time = None;
                            let mut phase_surface_us = 0.0f32;
                            let mut phase_egui_us = 0.0f32;
                            let mut phase_encode_us = 0.0f32;
                            let mut phase_submit_us = 0.0f32;

                            match engine.render(
                                window,
                                &egui_ctx,
                                egui_state,
                                &state_copy,
                                &mut file_dialog,
                                gamepad_events,
                            ) {
                                Ok((
                                    res,
                                    ui_el,
                                    ren_el,
                                    fire_el,
                                    fft_el,
                                    vis_el,
                                    surf,
                                    egui_l,
                                    enc,
                                    sub,
                                )) => {
                                    action = res.clone();
                                    ui_time = ui_el;
                                    render_time = ren_el;
                                    fire_time = fire_el;
                                    fft_time = fft_el;
                                    vis_shader_time = vis_el;
                                    phase_surface_us = surf;
                                    phase_egui_us = egui_l;
                                    phase_encode_us = enc;
                                    phase_submit_us = sub;
                                }
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
                                state.vis_picker_scroll_to_cursor = false;

                                // Write back timing stats
                                if ui_time > 0.0 || render_time > 0.0 {
                                    state.stats.ui_us = state.stats.ui_us * 0.9 + ui_time * 0.1;
                                    state.stats.render_us =
                                        state.stats.render_us * 0.9 + render_time * 0.1;
                                    if let Some(sh) = fire_time {
                                        state.stats.fire_us = state.stats.fire_us * 0.9 + sh * 0.1;
                                    }
                                    if let Some(vis) = vis_shader_time {
                                        state.stats.shader_us =
                                            state.stats.shader_us * 0.9 + vis * 0.1;
                                    }
                                    if let Some(ft) = fft_time {
                                        state.stats.gpu_fft_us =
                                            state.stats.gpu_fft_us * 0.9 + ft * 0.1;
                                    }
                                }

                                if let Some(bench_frames) = bench {
                                    bench_frame_count += 1;
                                    if bench_frame_count >= bench_frames {
                                        let vis_def = &crate::state::VISUALIZERS
                                            [state.current_visualizer_idx];
                                        println!("BENCHMARK_RESULT_VISUALIZER: {}", vis_def.name);
                                        println!("BENCHMARK_RESULT_FPS: {:.2}", state.current_fps);
                                        println!(
                                            "BENCHMARK_RESULT_SHADER_US: {:.2}",
                                            state.stats.shader_us
                                        );
                                        println!(
                                            "BENCHMARK_RESULT_RENDER_US: {:.2}",
                                            state.stats.render_us
                                        );
                                        println!(
                                            "BENCHMARK_RESULT_UI_US: {:.2}",
                                            state.stats.ui_us
                                        );
                                        println!(
                                            "BENCHMARK_RESULT_DECODE_US: {:.2}",
                                            state.stats.decode_us
                                        );
                                        println!(
                                            "BENCHMARK_RESULT_FFT_US: {:.2}",
                                            state.stats.fft_us
                                        );
                                        println!(
                                            "BENCHMARK_RESULT_GPU_FFT_US: {:.2}",
                                            state.stats.gpu_fft_us
                                        );
                                        println!(
                                            "BENCHMARK_RESULT_FIRE_US: {:.2}",
                                            state.stats.fire_us
                                        );
                                        elwt.exit();
                                        return;
                                    }
                                }

                                // Process engine actions
                                match action {
                                    EngineAction::ScrubPreview(pct, delta) => {
                                        let target = (state.duration_seconds * pct as f64)
                                            .clamp(0.0, state.duration_seconds);
                                        state.scrub_target_seconds = Some(target);

                                        if delta == 0.0
                                            && state.duration_seconds > 0.0
                                            && last_mouse_scrub_seek.elapsed()
                                                >= Duration::from_millis(80)
                                            && (target - state.current_seconds).abs() > 0.25
                                        {
                                            state.seek_request = Some(target);
                                            state.current_seconds = target;
                                            state.track_ended = false;
                                            last_mouse_scrub_seek = Instant::now();
                                        }

                                        if delta != 0.0 {
                                            if state.osd_timer > 0.0
                                                && state
                                                    .osd_text
                                                    .as_ref()
                                                    .is_some_and(|s| s.starts_with("Scrubbing"))
                                            {
                                                state.cumulative_scrub += delta;
                                            } else {
                                                state.cumulative_scrub = delta;
                                            }
                                            state.osd_timer = 0.5;
                                            let sign = if state.cumulative_scrub > 0.0 {
                                                "+"
                                            } else {
                                                "-"
                                            };
                                            state.osd_text = Some(format!(
                                                "Scrubbing {}{:.1}s",
                                                sign,
                                                state.cumulative_scrub.abs()
                                            ));
                                        }
                                    }
                                    EngineAction::ScrubEnd => {
                                        if let Some(target) = state.scrub_target_seconds.take() {
                                            state.seek_request = Some(target);
                                            state.current_seconds = target;
                                            state.track_ended = false;
                                            state.spectrum_history.clear();
                                            for _ in 0..120 {
                                                state.spectrum_history.push_back(vec![0.0; 1024]);
                                            }
                                        }
                                    }
                                    EngineAction::Seek(pct) => {
                                        let target = (state.duration_seconds * pct as f64)
                                            .clamp(0.0, state.duration_seconds);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.scrub_target_seconds = None;
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }
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
                                                    .add_filter(
                                                        "Audio/Video Files",
                                                        crate::playlist::FILE_DIALOG_EXTENSIONS,
                                                    )
                                                    .add_filter("All Files", &["*"])
                                                    .pick_files();
                                                if let Some(paths) = result {
                                                    let strings: Vec<String> = paths
                                                        .iter()
                                                        .map(|p| p.display().to_string())
                                                        .collect();
                                                    let _ = tx.send(strings);
                                                } else {
                                                    let _ = tx.send(vec![]);
                                                }
                                            });
                                        }
                                    }
                                    EngineAction::LoadFiles(paths, append) => {
                                        if append && !state.playlist.is_empty() && state.file_loaded
                                        {
                                            let count = paths.len();
                                            let first_name = std::path::Path::new(&paths[0])
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .into_owned();
                                            if count == 1 {
                                                state.osd_text = Some(format!(
                                                    "Added to Playlist: {}",
                                                    first_name
                                                ));
                                            } else {
                                                state.osd_text = Some(format!(
                                                    "Added {} tracks to Playlist",
                                                    count
                                                ));
                                            }
                                            state.osd_timer = 3.0;
                                            state.playlist.extend(paths);
                                        } else if !paths.is_empty() {
                                            state.playlist = paths;
                                            state.playlist_index = 0;
                                            state.load_request = Some(state.playlist[0].clone());
                                            state.file_loaded = true;
                                        }
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
                                    EngineAction::SetAudioDevice(device_name) => {
                                        state.selected_audio_device = Some(device_name.clone());
                                        state.audio_device_change_request = Some(device_name);
                                    }
                                    EngineAction::ToggleAudioTrackInMix(track_idx) => {
                                        if state.audio_tracks.len() > 1
                                            && track_idx < state.audio_tracks.len()
                                        {
                                            if state.active_audio_tracks.is_empty() {
                                                state.active_audio_tracks =
                                                    vec![state.selected_audio_track];
                                            }
                                            if let Some(pos) = state
                                                .active_audio_tracks
                                                .iter()
                                                .position(|&t| t == track_idx)
                                            {
                                                if state.active_audio_tracks.len() > 1 {
                                                    state.active_audio_tracks.remove(pos);
                                                }
                                            } else {
                                                state.active_audio_tracks.push(track_idx);
                                                state.active_audio_tracks.sort();
                                            }
                                            state.multi_track_mix_mode =
                                                state.active_audio_tracks.len() > 1;
                                            if state.audio_track_volumes.len()
                                                != state.audio_tracks.len()
                                            {
                                                state.audio_track_volumes =
                                                    vec![1.0; state.audio_tracks.len()];
                                            }
                                            let mix: Vec<(usize, f32)> = state
                                                .active_audio_tracks
                                                .iter()
                                                .map(|&idx| {
                                                    (
                                                        idx,
                                                        state
                                                            .audio_track_volumes
                                                            .get(idx)
                                                            .copied()
                                                            .unwrap_or(1.0),
                                                    )
                                                })
                                                .collect();
                                            state.audio_mix_request = Some(mix);
                                        }
                                    }
                                    EngineAction::SetAudioTrackVolume(track_idx, volume) => {
                                        if track_idx < state.audio_tracks.len() {
                                            if state.audio_track_volumes.len()
                                                != state.audio_tracks.len()
                                            {
                                                state.audio_track_volumes =
                                                    vec![1.0; state.audio_tracks.len()];
                                            }
                                            state.audio_track_volumes[track_idx] =
                                                volume.clamp(0.0, 1.0);
                                            let mix: Vec<(usize, f32)> = state
                                                .active_audio_tracks
                                                .iter()
                                                .map(|&idx| {
                                                    (
                                                        idx,
                                                        state
                                                            .audio_track_volumes
                                                            .get(idx)
                                                            .copied()
                                                            .unwrap_or(1.0),
                                                    )
                                                })
                                                .collect();
                                            state.audio_mix_request = Some(mix);
                                        }
                                    }
                                    EngineAction::SetAudioMixTracks(tracks) => {
                                        state.multi_track_mix_mode = tracks.len() > 1;
                                        state.active_audio_tracks =
                                            tracks.iter().map(|(idx, _)| *idx).collect();
                                        if state.audio_track_volumes.len()
                                            != state.audio_tracks.len()
                                        {
                                            state.audio_track_volumes =
                                                vec![1.0; state.audio_tracks.len()];
                                        }
                                        for &(idx, vol) in &tracks {
                                            if idx < state.audio_track_volumes.len() {
                                                state.audio_track_volumes[idx] = vol;
                                            }
                                        }
                                        if let Some(&(first_idx, _)) = tracks.first() {
                                            state.selected_audio_track = first_idx;
                                            if tracks.len() == 1 {
                                                state.audio_track_request = Some(first_idx);
                                            }
                                        }
                                        let mix_desc = if tracks.len() > 1 {
                                            let track_nums: Vec<String> = tracks
                                                .iter()
                                                .map(|(idx, _)| (idx + 1).to_string())
                                                .collect();
                                            format!("🎛 Mix: Tracks {}", track_nums.join("+"))
                                        } else if let Some(&(idx, _)) = tracks.first() {
                                            let track_title = state
                                                .audio_tracks
                                                .get(idx)
                                                .map(|t| t.title.clone())
                                                .unwrap_or_else(|| format!("Track {}", idx + 1));
                                            format!("🎛 Audio: {}", track_title)
                                        } else {
                                            "🎛 Audio Mix".to_string()
                                        };
                                        state.osd_text = Some(mix_desc);
                                        state.osd_timer = 2.0;
                                        state.audio_mix_request = Some(tracks);
                                    }
                                    EngineAction::VisPickerSelect(idx) => {
                                        state.current_visualizer_idx = idx;
                                        state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                                        state.osd_text =
                                            Some(crate::state::VISUALIZERS[idx].name.to_string());
                                        state.osd_timer = 2.0;
                                        state.show_vis_picker = false;
                                    }
                                    EngineAction::ToggleVisPicker => {
                                        state.show_vis_picker = !state.show_vis_picker;
                                        if state.show_vis_picker {
                                            state.vis_picker_cursor = state.current_visualizer_idx;
                                            state.vis_picker_scroll_to_cursor = true;
                                        }
                                    }
                                    EngineAction::VisPickerToggleEnabled(idx) => {
                                        if idx != 0 {
                                            // Frequency Spectrum always enabled
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
                                    EngineAction::OpenUrlDialog => {
                                        state.is_url_dialog_open = true;
                                        state.focus_url_input = true;
                                        window.set_ime_allowed(true); // Hint to Wayland/SteamOS to show the virtual keyboard
                                    }
                                    EngineAction::ClearFocusUrlInput => {
                                        state.focus_url_input = false;
                                    }
                                    EngineAction::CloseUrlDialog => {
                                        state.is_url_dialog_open = false;
                                        window.set_ime_allowed(false);
                                    }
                                    EngineAction::SetUrlInput(url) => {
                                        state.url_input_text = url;
                                    }
                                    EngineAction::EditUrl(url) => {
                                        state.url_input_text = url;
                                        state.focus_url_input = true;
                                        window.set_ime_allowed(true);
                                    }
                                    EngineAction::LoadUrl(url) => {
                                        let mut final_url = url.clone();
                                        if !final_url.starts_with("http://")
                                            && !final_url.starts_with("https://")
                                        {
                                            final_url = format!("https://{}", final_url);
                                        }
                                        state.is_url_dialog_open = false;
                                        window.set_ime_allowed(false);
                                        state.url_input_text.clear();
                                        state.playlist = vec![final_url.clone()];
                                        state.playlist_index = 0;
                                        state.load_request = Some(final_url.clone());
                                        state.file_loaded = true;

                                        if let Some(idx) = state
                                            .url_history
                                            .iter()
                                            .position(|(u, _)| u == &final_url)
                                        {
                                            let item = state.url_history.remove(idx);
                                            state.url_history.push(item);
                                        } else {
                                            state.url_history.push((
                                                final_url.clone(),
                                                "Network Stream".to_string(),
                                            ));
                                        }

                                        let mut out = String::new();
                                        for (u, t) in &state.url_history {
                                            out.push_str(&format!("{}|{}\n", u, t));
                                        }
                                        let _ = std::fs::write(
                                            crate::state::get_history_file_path(),
                                            out,
                                        );
                                    }
                                    EngineAction::None => {}
                                }

                                // File picker state
                                let fd_state = file_dialog.state();
                                state.is_file_picker_open =
                                    *fd_state == egui_file_dialog::DialogState::Open;
                                if *fd_state == egui_file_dialog::DialogState::Cancelled {
                                    file_dialog = egui_file_dialog::FileDialog::new()
                                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                                        .initial_directory(initial_dir.clone())
                                        .add_file_filter_extensions(
                                            "Audio/Video Files",
                                            crate::playlist::FILE_DIALOG_EXTENSIONS.to_vec(),
                                        )
                                        .show_all_files_filter(true)
                                        .default_file_filter("Audio/Video Files");
                                }
                                if state.open_file_request && !state.is_file_picker_open {
                                    state.open_file_request = false;
                                    state.is_file_picker_open = true;
                                    trigger_picker = true;
                                }

                                // Write phase timings
                                state.stats.phase_lock_update_us = state.stats.phase_lock_update_us
                                    * 0.9
                                    + phase_lock_update_us * 0.1;
                                state.stats.phase_snapshot_us =
                                    state.stats.phase_snapshot_us * 0.9 + phase_snapshot_us * 0.1;
                                state.stats.phase_surface_us =
                                    state.stats.phase_surface_us * 0.9 + phase_surface_us * 0.1;
                                state.stats.phase_egui_layout_us =
                                    state.stats.phase_egui_layout_us * 0.9 + phase_egui_us * 0.1;
                                state.stats.phase_encode_us =
                                    state.stats.phase_encode_us * 0.9 + phase_encode_us * 0.1;
                                state.stats.phase_submit_us =
                                    state.stats.phase_submit_us * 0.9 + phase_submit_us * 0.1;
                                state.stats.phase_post_us = state.stats.phase_post_us * 0.9
                                    + post_timer.elapsed().as_micros() as f32 * 0.1;
                                // Poll native file picker results
                                if let Ok(paths) = rfd_rx.try_recv() {
                                    rfd_pending = false;
                                    if !paths.is_empty() {
                                        if let Some(parent) =
                                            std::path::Path::new(&paths[0]).parent()
                                        {
                                            initial_dir = parent.to_path_buf();
                                        }
                                        let expanded = crate::playlist::expand_input_paths(&paths);
                                        let append = state.append_to_playlist;
                                        if append && !state.playlist.is_empty() {
                                            state.playlist.extend(expanded);
                                        } else if !expanded.is_empty() {
                                            state.playlist = expanded;
                                            state.playlist_index = 0;
                                            state.load_request = Some(state.playlist[0].clone());
                                            state.file_loaded = true;
                                        }
                                        state.is_file_picker_open = false;
                                        window.request_redraw();
                                    }
                                }

                                // Poll IPC incoming paths (from secondary instances)
                                if let Ok(raw_paths) = ipc_rx.try_recv() {
                                    if !raw_paths.is_empty() {
                                        let expanded =
                                            crate::playlist::expand_input_paths(&raw_paths);
                                        if !expanded.is_empty() {
                                            if !state.file_loaded || state.playlist.is_empty() {
                                                state.playlist = expanded.clone();
                                                state.playlist_index = 0;
                                                state.load_request =
                                                    Some(state.playlist[0].clone());
                                                state.file_loaded = true;
                                            } else {
                                                let first_new_idx = state.playlist.len();
                                                state.playlist.extend(expanded.clone());
                                                state.playlist_index = first_new_idx;
                                                state.load_request =
                                                    Some(state.playlist[first_new_idx].clone());
                                                state.is_paused = false;
                                                let file_name = std::path::Path::new(&expanded[0])
                                                    .file_name()
                                                    .unwrap_or_default()
                                                    .to_string_lossy()
                                                    .into_owned();
                                                if expanded.len() == 1 {
                                                    state.osd_text =
                                                        Some(format!("Now Playing: {}", file_name));
                                                } else {
                                                    state.osd_text = Some(format!(
                                                        "Added {} tracks (Playing {})",
                                                        expanded.len(),
                                                        file_name
                                                    ));
                                                }
                                                state.osd_timer = 3.0;
                                            }
                                            window.set_minimized(false);
                                            window.focus_window();
                                            window.request_redraw();
                                        }
                                    } else {
                                        window.set_minimized(false);
                                        window.focus_window();
                                        window.request_redraw();
                                    }
                                }
                            }

                            if trigger_picker {
                                file_dialog.pick_multiple();
                            }

                            // VSync and Frame Rate pacing:
                            // When --uncapped or --bench is specified, allow maximum unconstrained framerates.
                            // Otherwise, pace to display refresh rate to eliminate runaway GPU power draw.
                            let fps_limit =
                                if let Ok(limit_str) = std::env::var("RUSTTRACKER_FPS_LIMIT") {
                                    limit_str.parse::<f32>().ok().filter(|&fps| fps > 0.0)
                                } else if !is_uncapped && bench.is_none() {
                                    let target_fps = window
                                        .current_monitor()
                                        .and_then(|m| m.refresh_rate_millihertz())
                                        .map(|mhz| mhz as f32 / 1000.0)
                                        .unwrap_or(60.0);
                                    Some(target_fps)
                                } else {
                                    None
                                };

                            if let Some(target_fps) = fps_limit {
                                let target_frame_time = Duration::from_secs_f32(1.0 / target_fps);
                                let elapsed = now.elapsed();
                                if elapsed < target_frame_time {
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
                }
                Event::AboutToWait => {
                    // Poll native file picker results immediately to eliminate UI latency
                    if let Ok(paths) = rfd_rx.try_recv() {
                        rfd_pending = false;
                        if !paths.is_empty() {
                            if let Some(parent) = std::path::Path::new(&paths[0]).parent() {
                                initial_dir = parent.to_path_buf();
                            }
                            let expanded = crate::playlist::expand_input_paths(&paths);
                            let mut state = app_state.lock().unwrap();
                            let append = state.append_to_playlist;
                            if append && !state.playlist.is_empty() {
                                state.playlist.extend(expanded);
                            } else if !expanded.is_empty() {
                                state.playlist = expanded;
                                state.playlist_index = 0;
                                state.load_request = Some(state.playlist[0].clone());
                                state.file_loaded = true;
                            }
                            state.is_file_picker_open = false;
                            window.request_redraw();
                        }
                    }

                    // Poll IPC / macOS Finder openFiles results immediately
                    if let Ok(raw_paths) = ipc_rx.try_recv() {
                        if !raw_paths.is_empty() {
                            let expanded = crate::playlist::expand_input_paths(&raw_paths);
                            if !expanded.is_empty() {
                                let mut state = app_state.lock().unwrap();
                                if !state.file_loaded || state.playlist.is_empty() {
                                    state.playlist = expanded.clone();
                                    state.playlist_index = 0;
                                    state.load_request = Some(state.playlist[0].clone());
                                    state.file_loaded = true;
                                } else {
                                    let first_new_idx = state.playlist.len();
                                    state.playlist.extend(expanded.clone());
                                    state.playlist_index = first_new_idx;
                                    state.load_request =
                                        Some(state.playlist[first_new_idx].clone());
                                    state.is_paused = false;
                                    let file_name = std::path::Path::new(&expanded[0])
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned();
                                    if expanded.len() == 1 {
                                        state.osd_text =
                                            Some(format!("Now Playing: {}", file_name));
                                    } else {
                                        state.osd_text = Some(format!(
                                            "Added {} tracks (Playing {})",
                                            expanded.len(),
                                            file_name
                                        ));
                                    }
                                    state.osd_timer = 3.0;
                                }
                                window.set_minimized(false);
                                window.focus_window();
                                window.request_redraw();
                            }
                        } else {
                            window.set_minimized(false);
                            window.focus_window();
                            window.request_redraw();
                        }
                    }

                    let is_dialog_open =
                        *file_dialog.state() == egui_file_dialog::DialogState::Open || {
                            let state = app_state.lock().unwrap();
                            state.is_url_dialog_open
                        };

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
                                if name.contains("sony")
                                    || name.contains("dualshock")
                                    || name.contains("dualsense")
                                    || name.contains("ps4")
                                    || name.contains("ps5")
                                    || name.contains("wireless controller")
                                    || vendor == 0x054C
                                {
                                    g_type = crate::state::GamepadType::PlayStation;
                                    break;
                                } else if name.contains("nintendo")
                                    || name.contains("pro controller")
                                    || name.contains("joy-con")
                                    || vendor == 0x057E
                                {
                                    g_type = crate::state::GamepadType::Nintendo;
                                    break;
                                }
                            }
                            state.gamepad_type = g_type;
                        }
                    }

                    while let Some(gilrs::Event { event: g_event, .. }) = gilrs.next_event() {
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
                                        let target = (state.current_seconds + 10.0)
                                            .clamp(0.0, state.duration_seconds);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }
                                        state.osd_text = Some("+10.0s".to_string());
                                        state.osd_timer = 2.0;
                                    }
                                    gilrs::Button::DPadLeft => {
                                        let mut state = app_state.lock().unwrap();
                                        let target = (state.current_seconds - 10.0).max(0.0);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }
                                        state.osd_text = Some("-10.0s".to_string());
                                        state.osd_timer = 2.0;
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
                                    let currently_fullscreen = window.fullscreen().is_some();
                                    is_fullscreen = !currently_fullscreen;
                                    if is_fullscreen {
                                        window.set_fullscreen(Some(
                                            winit::window::Fullscreen::Borderless(None),
                                        ));
                                        keep_awake = keepawake::Builder::default()
                                            .display(true)
                                            .idle(true)
                                            .create()
                                            .ok();
                                    } else {
                                        window.set_fullscreen(None);
                                        constrain_window_to_monitor(
                                            window,
                                            Some(desired_logical_w),
                                            Some(desired_logical_h),
                                            true,
                                        );
                                        keep_awake = None;
                                    }
                                    continue;
                                }
                                _ => {}
                            }

                            if is_dialog_open {
                                let mut state = app_state.lock().unwrap();
                                let is_url_open = state.is_url_dialog_open;
                                let mut push_key = |key: egui::Key| {
                                    state.egui_gamepad_events.push(egui::Event::Key {
                                        key,
                                        physical_key: None,
                                        pressed: true,
                                        repeat: false,
                                        modifiers: egui::Modifiers::NONE,
                                    });
                                    state.egui_gamepad_events.push(egui::Event::Key {
                                        key,
                                        physical_key: None,
                                        pressed: false,
                                        repeat: false,
                                        modifiers: egui::Modifiers::NONE,
                                    });
                                };
                                match button {
                                    gilrs::Button::DPadUp => {
                                        if is_url_open {
                                            state.egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Tab,
                                                physical_key: None,
                                                pressed: true,
                                                repeat: false,
                                                modifiers: egui::Modifiers::SHIFT,
                                            });
                                            state.egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Tab,
                                                physical_key: None,
                                                pressed: false,
                                                repeat: false,
                                                modifiers: egui::Modifiers::SHIFT,
                                            });
                                        } else {
                                            push_key(egui::Key::ArrowUp);
                                        }
                                        continue;
                                    }
                                    gilrs::Button::DPadDown => {
                                        if is_url_open {
                                            state.egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Tab,
                                                physical_key: None,
                                                pressed: true,
                                                repeat: false,
                                                modifiers: egui::Modifiers::NONE,
                                            });
                                            state.egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Tab,
                                                physical_key: None,
                                                pressed: false,
                                                repeat: false,
                                                modifiers: egui::Modifiers::NONE,
                                            });
                                        } else {
                                            push_key(egui::Key::ArrowDown);
                                        }
                                        continue;
                                    }
                                    gilrs::Button::DPadLeft => {
                                        push_key(egui::Key::ArrowLeft);
                                        continue;
                                    }
                                    gilrs::Button::DPadRight => {
                                        push_key(egui::Key::ArrowRight);
                                        continue;
                                    }
                                    gilrs::Button::South => {
                                        push_key(egui::Key::Enter);
                                        continue;
                                    }
                                    gilrs::Button::East => {
                                        push_key(egui::Key::Escape);
                                        continue;
                                    }
                                    gilrs::Button::RightTrigger => {
                                        state.egui_gamepad_events.push(egui::Event::Key {
                                            key: egui::Key::Tab,
                                            physical_key: None,
                                            pressed: true,
                                            repeat: false,
                                            modifiers: egui::Modifiers::NONE,
                                        });
                                        state.egui_gamepad_events.push(egui::Event::Key {
                                            key: egui::Key::Tab,
                                            physical_key: None,
                                            pressed: false,
                                            repeat: false,
                                            modifiers: egui::Modifiers::NONE,
                                        });
                                        continue;
                                    }
                                    gilrs::Button::LeftTrigger => {
                                        state.egui_gamepad_events.push(egui::Event::Key {
                                            key: egui::Key::Tab,
                                            physical_key: None,
                                            pressed: true,
                                            repeat: false,
                                            modifiers: egui::Modifiers::SHIFT,
                                        });
                                        state.egui_gamepad_events.push(egui::Event::Key {
                                            key: egui::Key::Tab,
                                            physical_key: None,
                                            pressed: false,
                                            repeat: false,
                                            modifiers: egui::Modifiers::SHIFT,
                                        });
                                        continue;
                                    }
                                    _ => {}
                                }
                            } else {
                                let is_splash = {
                                    let state = app_state.lock().unwrap();
                                    !state.file_loaded
                                };
                                if is_splash {
                                    let mut state = app_state.lock().unwrap();
                                    let mut push_tab = |shift: bool| {
                                        let modifiers = if shift {
                                            egui::Modifiers::SHIFT
                                        } else {
                                            egui::Modifiers::NONE
                                        };
                                        state.egui_gamepad_events.push(egui::Event::Key {
                                            key: egui::Key::Tab,
                                            physical_key: None,
                                            pressed: true,
                                            repeat: false,
                                            modifiers,
                                        });
                                        state.egui_gamepad_events.push(egui::Event::Key {
                                            key: egui::Key::Tab,
                                            physical_key: None,
                                            pressed: false,
                                            repeat: false,
                                            modifiers,
                                        });
                                    };
                                    match button {
                                        gilrs::Button::DPadLeft => {
                                            push_tab(true);
                                            continue;
                                        }
                                        gilrs::Button::DPadRight => {
                                            push_tab(false);
                                            continue;
                                        }
                                        gilrs::Button::South => {
                                            state.egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Enter,
                                                physical_key: None,
                                                pressed: true,
                                                repeat: false,
                                                modifiers: egui::Modifiers::NONE,
                                            });
                                            state.egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Enter,
                                                physical_key: None,
                                                pressed: false,
                                                repeat: false,
                                                modifiers: egui::Modifiers::NONE,
                                            });
                                            continue;
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            match button {
                                gilrs::Button::LeftTrigger => {
                                    // L1 Bumper
                                    let mut state = app_state.lock().unwrap();
                                    if !state.playlist.is_empty()
                                        && state.playlist.len() > 1
                                        && state.playlist_index > 0
                                    {
                                        state.playlist_index -= 1;
                                        state.load_request =
                                            Some(state.playlist[state.playlist_index].clone());
                                        state.is_paused = false;
                                        state.osd_text = Some("Previous Track".to_string());
                                        state.osd_timer = 2.0;
                                    } else {
                                        let target = (state.current_seconds - 60.0).max(0.0);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }
                                        state.osd_text = Some("-60.0s".to_string());
                                        state.osd_timer = 2.0;
                                    }
                                }
                                gilrs::Button::RightTrigger => {
                                    // R1 Bumper
                                    let mut state = app_state.lock().unwrap();
                                    if !state.playlist.is_empty()
                                        && state.playlist.len() > 1
                                        && state.playlist_index + 1 < state.playlist.len()
                                    {
                                        state.playlist_index += 1;
                                        state.load_request =
                                            Some(state.playlist[state.playlist_index].clone());
                                        state.is_paused = false;
                                        state.osd_text = Some("Next Track".to_string());
                                        state.osd_timer = 2.0;
                                    } else {
                                        let target = (state.current_seconds + 60.0)
                                            .clamp(0.0, state.duration_seconds);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }
                                        state.osd_text = Some("+60.0s".to_string());
                                        state.osd_timer = 2.0;
                                    }
                                }
                                gilrs::Button::North => {
                                    // 'Y' or Triangle
                                    let mut state = app_state.lock().unwrap();
                                    state.open_file_request = true;
                                }
                                gilrs::Button::West => {
                                    // 'X' or Square
                                    let mut state = app_state.lock().unwrap();
                                    if state.video_frame_rx.is_some() || state.has_video_stream {
                                        state.video_mode = (state.video_mode + 1) % 4;
                                        let mode_name = match state.video_mode {
                                            0 => "Standard View",
                                            1 => "Video in Track Info",
                                            2 => "Video in Top Panel",
                                            3 => "Full Screen Video",
                                            _ => "Video",
                                        };
                                        state.osd_text = Some(format!("View: {}", mode_name));
                                    } else {
                                        state.show_hud = !state.show_hud;
                                        state.video_mode = 0;
                                        let mode_name = if state.show_hud {
                                            "Standard View"
                                        } else {
                                            "Full Screen Visualizer"
                                        };
                                        state.osd_text = Some(format!("View: {}", mode_name));
                                    }
                                    state.osd_timer = 2.0;
                                }
                                gilrs::Button::East => {
                                    // 'B' or Circle
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
                                        let target = (state.current_seconds + 10.0)
                                            .clamp(0.0, state.duration_seconds);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }
                                        state.osd_text = Some("+10.0s".to_string());
                                        state.osd_timer = 2.0;
                                    }
                                }
                                gilrs::Button::DPadLeft => {
                                    let mut state = app_state.lock().unwrap();
                                    if !state.show_vis_picker {
                                        let target = (state.current_seconds - 10.0).max(0.0);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }
                                        state.osd_text = Some("-10.0s".to_string());
                                        state.osd_timer = 2.0;
                                    }
                                }
                                gilrs::Button::DPadUp => {
                                    let mut state = app_state.lock().unwrap();
                                    if state.show_vis_picker {
                                        if state.vis_picker_cursor == 0 {
                                            state.vis_picker_cursor =
                                                crate::state::VISUALIZERS.len() - 1;
                                        } else {
                                            state.vis_picker_cursor -= 1;
                                        }
                                        state.vis_picker_scroll_to_cursor = true;
                                    } else {
                                        let len = crate::state::VISUALIZERS.len();
                                        let mut idx = state.current_visualizer_idx;
                                        for _ in 0..len {
                                            idx = (idx + 1) % len;
                                            if state.vis_enabled[idx] {
                                                break;
                                            }
                                        }
                                        state.current_visualizer_idx = idx;
                                        state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                                        state.osd_text =
                                            Some(crate::state::VISUALIZERS[idx].name.to_string());
                                        state.osd_timer = 2.0;
                                    }
                                }
                                gilrs::Button::DPadDown => {
                                    let mut state = app_state.lock().unwrap();
                                    if state.show_vis_picker {
                                        state.vis_picker_cursor = (state.vis_picker_cursor + 1)
                                            % crate::state::VISUALIZERS.len();
                                        state.vis_picker_scroll_to_cursor = true;
                                    } else {
                                        let len = crate::state::VISUALIZERS.len();
                                        let mut idx = state.current_visualizer_idx;
                                        for _ in 0..len {
                                            idx = if idx == 0 { len - 1 } else { idx - 1 };
                                            if state.vis_enabled[idx] {
                                                break;
                                            }
                                        }
                                        state.current_visualizer_idx = idx;
                                        state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                                        state.osd_text =
                                            Some(crate::state::VISUALIZERS[idx].name.to_string());
                                        state.osd_timer = 2.0;
                                    }
                                }
                                gilrs::Button::South => {
                                    let mut state = app_state.lock().unwrap();
                                    if state.show_vis_picker {
                                        state.current_visualizer_idx = state.vis_picker_cursor;
                                        state.visualizer_mode =
                                            crate::state::VISUALIZERS[state.vis_picker_cursor].id;
                                        state.osd_text = Some(
                                            crate::state::VISUALIZERS[state.vis_picker_cursor]
                                                .name
                                                .to_string(),
                                        );
                                        state.osd_timer = 2.0;
                                        state.show_vis_picker = false;
                                    } else if state.track_ended
                                        || (state.current_seconds >= state.duration_seconds - 0.1
                                            && state.duration_seconds > 0.0)
                                    {
                                        state.track_ended = false;
                                        state.seek_request = Some(0.0);
                                        state.spectrum_history.clear();
                                        for _ in 0..120 {
                                            state.spectrum_history.push_back(vec![0.0; 1024]);
                                        }
                                        state.is_paused = false;
                                    } else if state.playlist_index >= state.playlist.len()
                                        && !state.playlist.is_empty()
                                    {
                                        if state.duration_seconds == 0.0 {
                                            state.playlist_index = state.playlist.len() - 1;
                                        } else {
                                            state.playlist_index = 0;
                                        }
                                        state.load_request =
                                            Some(state.playlist[state.playlist_index].clone());
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
        },
    };
    event_loop.run_app(&mut app)?;
    if let Some(err) = app.init_error {
        return Err(err.into());
    }
    Ok(())
}

fn run_tui<B: Backend>(
    terminal: &mut Terminal<B>,
    app_state: Arc<Mutex<AppState>>,
    mut active_stream: Option<audio::PlaybackHandle>,
    is_mic_launch: bool,
) -> io::Result<()>
where
    std::io::Error: From<<B as Backend>::Error>,
{
    let tick_rate = Duration::from_millis(16); // ~60fps
    let mut last_tick = Instant::now();
    let mut is_first_frame = true;

    loop {
        if is_first_frame {
            is_first_frame = false;
            app_state.lock().unwrap().is_paused = false;
        }

        let load_path = {
            let mut state = app_state.lock().unwrap();

            let device_change = state.audio_device_change_request.take();
            if let Some(new_device) = device_change {
                state.selected_audio_device = Some(new_device.clone());
                let reload_path = if is_mic_launch {
                    "".to_string()
                } else if state.playlist_index < state.playlist.len() {
                    state.playlist[state.playlist_index].clone()
                } else {
                    state.song_title.clone()
                };

                if !reload_path.is_empty() || is_mic_launch {
                    state.load_request = Some(reload_path);
                    state.osd_text = Some(format!("Audio Output: {}", new_device));
                    state.osd_timer = 3.0;
                }
            }

            if state.audio_device_lost {
                state.audio_device_lost = false;
                let reload_path = if is_mic_launch {
                    "".to_string()
                } else if state.playlist_index < state.playlist.len() {
                    state.playlist[state.playlist_index].clone()
                } else {
                    state.song_title.clone()
                };

                if !reload_path.is_empty() || is_mic_launch {
                    state.load_request = Some(reload_path);
                    state.osd_text = Some("Audio Device Reconnected".to_string());
                    state.osd_timer = 3.0;
                }
            }

            if state.track_ended {
                state.track_ended = false;
                if state.load_request.is_none() {
                    state.playlist_index += 1;
                    if state.playlist_index < state.playlist.len() {
                        state.load_request = Some(state.playlist[state.playlist_index].clone());
                    }
                }
            }

            state.load_request.take()
        };

        if let Some(path) = load_path {
            let is_mic = is_mic_launch && path.is_empty();
            // Check if the path exists first!
            let path_exists = if path.starts_with("http") || is_mic {
                true
            } else {
                std::path::Path::new(&path).exists()
            };

            if !path_exists {
                let mut state = app_state.lock().unwrap();
                state.playlist_index += 1;
                if state.playlist_index < state.playlist.len() {
                    state.load_request = Some(state.playlist[state.playlist_index].clone());
                } else {
                    if state.file_loaded {
                        if let Some(idx) =
                            state.playlist.iter().position(|p| p == &state.song_title)
                        {
                            state.playlist_index = idx;
                        } else {
                            state.playlist_index = state.playlist_index.saturating_sub(1);
                        }
                    } else {
                        state.file_loaded = false;
                        state.artist = "Load Failed".to_string();
                    }
                }
            } else {
                let _ = active_stream.take(); // DROP OLD STREAM FIRST
                {
                    let mut state = app_state.lock().unwrap();
                    state.stats.bitstream_active = false;
                    state.stats.audio_buffer_fill_pct = 0.0;
                    state.stats.video_buffer_fill_pct = 0.0;
                    state.stats.video_hw_decoder.clear();
                    state.stats.video_fps = 0.0;
                    state.stats.video_bitrate_kbps = None;
                    state.stats.audio_bitrate_kbps = None;
                    state.track_ended = false;
                }

                let mut loaded_stream = None;
                let mut last_err = None;
                for _ in 0..5 {
                    match audio::start_audio_thread(&path, is_mic, Arc::clone(&app_state)) {
                        Ok(stream) => {
                            loaded_stream = Some(stream);
                            break;
                        }
                        Err(e) => {
                            last_err = Some(e);
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }

                if let Some(stream) = loaded_stream {
                    let mut state = app_state.lock().unwrap();
                    state.file_loaded = true;
                    state.song_title = if is_mic {
                        "Microphone Input".to_string()
                    } else {
                        path.clone()
                    };
                    state.lyrics = if is_mic {
                        None
                    } else {
                        crate::lyrics::load_lyrics_for_file(&path).map(std::sync::Arc::new)
                    };
                    state.track_ended = false;
                    active_stream = Some(stream);
                } else {
                    let mut state = app_state.lock().unwrap();
                    state.lyrics = None;
                    let err_msg = last_err
                        .map(|e| format!("{:?}", e))
                        .unwrap_or_else(|| "Unknown error".to_string());
                    let file_name = if is_mic {
                        "Microphone".to_string()
                    } else {
                        std::path::Path::new(&path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    };
                    state.osd_text = Some(format!("Load Failed: {}\n{}", file_name, err_msg));
                    state.osd_timer = 5.0;

                    if !is_mic {
                        state.playlist_index += 1;
                        if state.playlist_index < state.playlist.len() {
                            state.load_request = Some(state.playlist[state.playlist_index].clone());
                        } else {
                            state.file_loaded = false;
                            state.artist = format!("Load Failed: {}", err_msg);
                        }
                    } else {
                        state.file_loaded = false;
                        state.artist = format!("Mic Failed: {}", err_msg);
                    }
                }
            }
        }
        {
            let tui_dt = last_tick.elapsed().as_secs_f32().clamp(0.001, 0.1);
            let time_scale = tui_dt * 60.0;
            let mut state = app_state.lock().unwrap();

            // Smoothing physics for VUs (Inertia)
            if state.channel_vus.len() != state.raw_channel_vus.len() {
                state.channel_vus = vec![0.0; state.raw_channel_vus.len()];
            }
            for i in 0..state.raw_channel_vus.len() {
                if state.raw_channel_vus[i] > state.channel_vus[i] {
                    state.channel_vus[i] = state.raw_channel_vus[i];
                } else {
                    state.channel_vus[i] =
                        (state.channel_vus[i] - 0.015 * time_scale).max(state.raw_channel_vus[i]);
                }
            }

            // Decay peaks and apply gravity
            if state.peak_vus.len() != state.channel_vus.len() {
                state.peak_vus = vec![0.0; state.channel_vus.len()];
            }
            for i in 0..state.channel_vus.len() {
                state.peak_vus[i] = (state.peak_vus[i] - 0.005 * time_scale).max(0.0);
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
                    state.spectrum_data[i] =
                        (state.spectrum_data[i] - 1.5 * time_scale).max(state.raw_spectrum_data[i]);
                }
            }

            // Decay spectrum peaks
            if state.spectrum_peaks.len() != state.spectrum_data.len() {
                state.spectrum_peaks = vec![0.0; state.spectrum_data.len()];
            }
            for i in 0..state.spectrum_data.len() {
                state.spectrum_peaks[i] = (state.spectrum_peaks[i] - 0.5 * time_scale).max(0.0);
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

        if crossterm::event::poll(timeout)?
            && let CEvent::Key(key) = event::read()?
        {
            let show_picker = { app_state.lock().unwrap().show_tui_device_picker };
            if show_picker {
                match key.code {
                    KeyCode::Esc => {
                        let mut state = app_state.lock().unwrap();
                        state.show_tui_device_picker = false;
                    }
                    KeyCode::Up => {
                        let mut state = app_state.lock().unwrap();
                        let count = state.available_audio_devices.len();
                        if count > 0 {
                            if state.tui_device_picker_cursor == 0 {
                                state.tui_device_picker_cursor = count - 1;
                            } else {
                                state.tui_device_picker_cursor -= 1;
                            }
                        }
                    }
                    KeyCode::Down => {
                        let mut state = app_state.lock().unwrap();
                        let count = state.available_audio_devices.len();
                        if count > 0 {
                            state.tui_device_picker_cursor =
                                (state.tui_device_picker_cursor + 1) % count;
                        }
                    }
                    KeyCode::Enter => {
                        let mut state = app_state.lock().unwrap();
                        let cursor = state.tui_device_picker_cursor;
                        if cursor < state.available_audio_devices.len() {
                            let dev = state.available_audio_devices[cursor].clone();
                            state.audio_device_change_request = Some(dev);
                        }
                        state.show_tui_device_picker = false;
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') => {
                        drop(active_stream);
                        return Ok(());
                    }
                    KeyCode::Char('d') => {
                        let mut state = app_state.lock().unwrap();
                        state.show_tui_device_picker = true;
                        state.tui_device_picker_cursor = 0;
                        if let Some(ref current) = state.selected_audio_device
                            && let Some(idx) = state
                                .available_audio_devices
                                .iter()
                                .position(|d| d == current)
                        {
                            state.tui_device_picker_cursor = idx;
                        }
                    }
                    KeyCode::Char('n') => {
                        let mut state = app_state.lock().unwrap();
                        if !state.playlist.is_empty()
                            && state.playlist_index + 1 < state.playlist.len()
                        {
                            state.playlist_index += 1;
                            state.load_request = Some(state.playlist[state.playlist_index].clone());
                            state.is_paused = false;
                        }
                    }
                    KeyCode::Char('p') => {
                        let mut state = app_state.lock().unwrap();
                        if !state.playlist.is_empty() && state.playlist_index > 0 {
                            state.playlist_index -= 1;
                            state.load_request = Some(state.playlist[state.playlist_index].clone());
                            state.is_paused = false;
                        }
                    }
                    KeyCode::Char(' ') => {
                        let mut state = app_state.lock().unwrap();
                        if state.track_ended
                            || (state.current_seconds >= state.duration_seconds - 0.1
                                && state.duration_seconds > 0.0)
                        {
                            state.track_ended = false;
                            state.seek_request = Some(0.0);
                            state.spectrum_history.clear();
                            for _ in 0..120 {
                                state.spectrum_history.push_back(vec![0.0; 1024]);
                            }
                            state.is_paused = false;
                        } else if state.playlist_index >= state.playlist.len()
                            && !state.playlist.is_empty()
                        {
                            if state.duration_seconds == 0.0 {
                                state.playlist_index = state.playlist.len() - 1;
                            } else {
                                state.playlist_index = 0;
                            }
                            state.load_request = Some(state.playlist[state.playlist_index].clone());
                            state.is_paused = false;
                        } else {
                            state.is_paused = !state.is_paused;
                        }
                    }
                    KeyCode::Right => {
                        let mut state = app_state.lock().unwrap();
                        let target = (state.current_seconds + 5.0).min(state.duration_seconds);
                        state.seek_request = Some(target);
                        state.current_seconds = target;
                        state.track_ended = false;
                        state.spectrum_history.clear();
                        for _ in 0..120 {
                            state.spectrum_history.push_back(vec![0.0; 1024]);
                        }
                    }
                    KeyCode::Left => {
                        let mut state = app_state.lock().unwrap();
                        let target = (state.current_seconds - 5.0).max(0.0);
                        state.seek_request = Some(target);
                        state.current_seconds = target;
                        state.track_ended = false;
                        state.spectrum_history.clear();
                        for _ in 0..120 {
                            state.spectrum_history.push_back(vec![0.0; 1024]);
                        }
                    }
                    KeyCode::Char(c @ '1'..='9') => {
                        let track_num = (c as u8 - b'0') as usize;
                        let mut state = app_state.lock().unwrap();
                        if state.audio_tracks.len() > 1 {
                            let track_idx = track_num - 1;
                            if track_idx < state.audio_tracks.len()
                                && track_idx != state.selected_audio_track
                            {
                                state.audio_track_request = Some(track_idx);
                            }
                        }
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

#[cfg(test)]
mod tests {
    use super::clamp_window_geometry_to_monitor;

    #[test]
    fn test_window_geometry_fits_1080p_at_150_percent_scale() {
        // 1920x1080 at 150% DPI scaling -> 1280x720 logical monitor area
        let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
            clamp_window_geometry_to_monitor(1920, 1080, 0, 0, 1.5, 1440.0, 840.0);

        let est_outer_phys_w = ((w + 16.0) * 1.5).round() as i32;
        let est_outer_phys_h = ((h + 38.0) * 1.5).round() as i32;

        assert!(w < 1280.0 && h < 720.0);
        assert!(min_w <= w && min_h <= h);
        assert!(pos_x >= 0 && pos_y >= 0);
        assert!(pos_x + est_outer_phys_w <= 1920);
        assert!(pos_y + est_outer_phys_h <= 1080);
    }

    #[test]
    fn test_window_geometry_fits_1080p_at_125_percent_scale() {
        // 1920x1080 at 125% DPI scaling -> 1536x864 logical monitor area
        let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
            clamp_window_geometry_to_monitor(1920, 1080, 0, 0, 1.25, 1440.0, 840.0);

        let est_outer_phys_w = ((w + 16.0) * 1.25).round() as i32;
        let est_outer_phys_h = ((h + 38.0) * 1.25).round() as i32;

        assert!(w <= 1440.0 && h < 840.0);
        assert!(min_w <= w && min_h <= h);
        assert!(pos_x >= 0 && pos_y >= 0);
        assert!(pos_x + est_outer_phys_w <= 1920);
        assert!(pos_y + est_outer_phys_h <= 1080);
    }

    #[test]
    fn test_window_geometry_fits_1366x768_display() {
        let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
            clamp_window_geometry_to_monitor(1366, 768, 0, 0, 1.0, 1440.0, 840.0);

        let est_outer_phys_w = (w + 16.0).round() as i32;
        let est_outer_phys_h = (h + 38.0).round() as i32;

        assert!(w < 1366.0 && h < 768.0);
        assert!(min_w <= w && min_h <= h);
        assert!(pos_x >= 0 && pos_y >= 0);
        assert!(pos_x + est_outer_phys_w <= 1366);
        assert!(pos_y + est_outer_phys_h <= 768);
    }

    #[test]
    fn test_window_geometry_preserves_default_on_large_display() {
        let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
            clamp_window_geometry_to_monitor(2560, 1440, -2560, 0, 1.0, 1440.0, 840.0);

        assert_eq!((w, h), (1440.0, 840.0));
        assert_eq!((min_w, min_h), (1024.0, 600.0));
        assert!(pos_x >= -2560 && pos_x + 1456 <= 0);
        assert!(pos_y >= 0 && pos_y + 878 <= 1440);
    }
}
